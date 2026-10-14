//! The preview proxy: a small, frequently keyframed H.264 copy of a video
//! for players that cannot play the source (architecture plan 6.2 and 8).
//!
//! The desktop app previews videos in WebView2, which plays 8-bit H.264 in
//! MP4 or MOV but not every source a recorder writes (HEVC without the
//! system extension, 10-bit video, Matroska). For those, the analysis
//! writes `proxy.mp4` into the project cache while it decodes the video
//! anyway: each frame is area-averaged to [`PROXY_HEIGHT`] lines (see
//! [`VideoFrame::thumbnail`]) and encoded with the normal encoder chain,
//! a keyframe every half second so seeking stays fast. The proxy keeps the
//! source's timestamps, so its frames line up with the project's.

use std::path::{Path, PathBuf};

use blurrd_core::{Codec, MediaInfo, RateControl};
use ffmpeg_the_third::{Dictionary, format, frame};

use crate::frames::FramePool;
use crate::mux::Muxer;
use crate::transcode::{remove_temp, temp_path};
use crate::{
    EncodeOptions, EncoderFallback, EncoderInput, PixelLayout, VideoEncoder, VideoError, VideoFrame,
};

/// Height of the proxy (sources that are smaller keep their height).
pub const PROXY_HEIGHT: u32 = 540;
/// Constant quality of the proxy: a preview, not an export.
const PROXY_CQ: u8 = 28;
/// AVIO buffer of the proxy file.
const PROXY_BUFFER: usize = 1 << 20;

/// Whether WebView2 plays a video file at `path` with this codec and bit
/// depth: 8-bit H.264 in MP4 or MOV.
fn playable(path: &Path, codec: &str, bit_depth: u8) -> bool {
    let container = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    matches!(container.as_deref(), Some("mp4" | "mov" | "m4v")) && codec == "h264" && bit_depth == 8
}

/// Writes the preview proxy of one video, frame by frame.
pub struct ProxyWriter {
    encoder: VideoEncoder,
    muxer: Option<Muxer>,
    pool: FramePool,
    stream: usize,
    encoder_tb: ffmpeg_the_third::Rational,
    stream_tb: ffmpeg_the_third::Rational,
    size: (u32, u32),
    /// A keyframe every this many frames.
    keyframe_every: u32,
    /// Nominal frame duration in the source's time base.
    duration: i64,
    sent: u32,
    temp: PathBuf,
    output: PathBuf,
}

impl std::fmt::Debug for ProxyWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxyWriter")
            .field("output", &self.output)
            .field("encoder", &self.encoder.name())
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

impl ProxyWriter {
    /// Whether the desktop app's player needs a proxy for the video `info`
    /// describes: everything but 8-bit H.264 in MP4 or MOV.
    pub fn needed(info: &MediaInfo) -> bool {
        !playable(&info.path, &info.video_codec, info.bit_depth)
    }

    /// The proxy's picture size for a `width × height` source: at most
    /// [`PROXY_HEIGHT`] lines, the source's aspect ratio, even sizes.
    pub fn size(width: u32, height: u32) -> (u32, u32) {
        let out_h = height.clamp(2, PROXY_HEIGHT) & !1;
        let out_w = (u64::from(width) * u64::from(out_h) / u64::from(height.max(1))) as u32;
        ((out_w.max(2) + 1) & !1, out_h)
    }

    /// Starts the proxy of the video `info` describes at `output` (an
    /// `.mp4`; written to `<stem>.tmp.mp4` and renamed by
    /// [`ProxyWriter::finish`]). `options` choose the encoder (GPU use,
    /// a named encoder); codec, quality and keyframes are the proxy's.
    /// Returns the writer and the encoders that failed before the one
    /// used.
    ///
    /// # Errors
    ///
    /// [`VideoError::Unsupported`] for an output that is not a supported
    /// container or is the input; the errors of [`VideoEncoder::open`];
    /// [`VideoError::Io`] when the file cannot be created.
    pub fn create(
        info: &MediaInfo,
        output: &Path,
        options: &EncodeOptions,
    ) -> Result<(Self, Vec<EncoderFallback>), VideoError> {
        let temp = temp_path(&info.path, output)?;
        let size = Self::size(info.width, info.height);
        let fps = info.fps;
        let keyframe_every = (fps.num / (2 * fps.den.max(1))).max(1);
        let mut input = EncoderInput::from_media(info, PixelLayout::I420);
        input.width = size.0;
        input.height = size.1;
        input.bitrate = None;
        input.keyframe_interval = keyframe_every;
        let mut options = options.clone();
        options.codec = Codec::H264;
        options.rate = RateControl::Cq(PROXY_CQ);
        options.sessions = blurrd_core::NvencSessions::One;
        let (encoder, fallbacks) = VideoEncoder::open(&input, &options)?;
        let mut muxer = Muxer::create(&temp, PROXY_BUFFER)?;
        let stream = muxer.add_video(&encoder)?;
        let mut header = Dictionary::new();
        header.set("movflags", "+faststart");
        muxer.write_header(header)?;
        let (tb_num, tb_den) = (i64::from(info.time_base.num), i64::from(info.time_base.den));
        let duration = (tb_den * i64::from(fps.den))
            .checked_div(tb_num * i64::from(fps.num))
            .unwrap_or(1)
            .max(1);
        Ok((
            Self {
                encoder_tb: encoder.time_base(),
                stream_tb: muxer.time_base(stream),
                encoder,
                muxer: Some(muxer),
                pool: FramePool::default(),
                stream,
                size,
                keyframe_every,
                duration,
                sent: 0,
                temp,
                output: output.to_path_buf(),
            },
            fallbacks,
        ))
    }

    /// The encoder used, for example `h264_nvenc`.
    pub fn encoder(&self) -> &'static str {
        self.encoder.name()
    }

    /// Adds the next frame of the video (frames in presentation order).
    ///
    /// # Errors
    ///
    /// [`VideoError::Encode`] or [`VideoError::Io`] when encoding or
    /// writing fails.
    pub fn push(&mut self, source: &VideoFrame) -> Result<(), VideoError> {
        let planes = source.downscale(self.size.0, self.size.1)?;
        let mut picture = frame::Video::new(format::Pixel::YUV420P, planes.width, planes.height);
        for (index, (data, width)) in [
            (&planes.y, planes.width),
            (&planes.u, planes.width / 2),
            (&planes.v, planes.width / 2),
        ]
        .into_iter()
        .enumerate()
        {
            let stride = picture.stride(index);
            let width = width as usize;
            for (dst, src) in picture
                .data_mut(index)
                .chunks_mut(stride)
                .zip(data.chunks_exact(width))
            {
                dst[..width].copy_from_slice(src);
            }
        }
        let mut frame = VideoFrame::wrap(source.index(), source.pts(), PixelLayout::I420, picture);
        let target = self.encoder.input_layout();
        if target != PixelLayout::I420 {
            frame = self.pool.copy(&frame, target)?;
        }
        let keyframe = self.sent.is_multiple_of(self.keyframe_every);
        self.encoder.send(&mut frame, keyframe)?;
        self.sent += 1;
        if target != PixelLayout::I420 {
            self.pool.give_back(frame);
        }
        self.drain()
    }

    /// Writes every packet the encoder has ready.
    fn drain(&mut self) -> Result<(), VideoError> {
        let Some(muxer) = self.muxer.as_mut() else {
            return Ok(());
        };
        while let Some(packet) = self.encoder.receive()? {
            let mut packet = packet.into_inner();
            packet.set_duration(self.duration);
            packet.set_stream(self.stream);
            packet.rescale_ts(self.encoder_tb, self.stream_tb);
            muxer.write(&mut packet)?;
        }
        Ok(())
    }

    /// Flushes the encoder, finishes the file and moves it to its name.
    /// Returns the proxy's path.
    ///
    /// # Errors
    ///
    /// [`VideoError::Encode`] or [`VideoError::Io`]; the temporary file is
    /// removed.
    pub fn finish(mut self) -> Result<PathBuf, VideoError> {
        self.encoder.flush()?;
        self.drain()?;
        if let Some(muxer) = self.muxer.take() {
            muxer.finish()?;
        }
        std::fs::rename(&self.temp, &self.output).map_err(|source| VideoError::Io {
            context: "renaming the proxy to",
            path: self.output.clone(),
            source,
        })?;
        Ok(self.output.clone())
    }
}

impl Drop for ProxyWriter {
    /// An unfinished proxy leaves no file behind.
    fn drop(&mut self) {
        drop(self.muxer.take());
        if self.temp.exists() {
            remove_temp(&self.temp);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{ProxyWriter, playable};

    #[test]
    fn proxies_keep_the_aspect_ratio_at_even_sizes() {
        assert_eq!(ProxyWriter::size(1920, 1080), (960, 540));
        assert_eq!(ProxyWriter::size(3840, 2160), (960, 540));
        assert_eq!(ProxyWriter::size(1440, 1080), (720, 540));
        assert_eq!(ProxyWriter::size(1920, 1200), (864, 540));
        assert_eq!(ProxyWriter::size(320, 240), (320, 240));
        assert_eq!(ProxyWriter::size(2560, 1080), (1280, 540));
    }

    #[test]
    fn only_videos_the_player_cannot_play_need_a_proxy() {
        let plays = |file: &str, codec: &str, depth: u8| playable(Path::new(file), codec, depth);
        assert!(plays("a.mp4", "h264", 8));
        assert!(plays("a.MOV", "h264", 8));
        assert!(!plays("a.mkv", "h264", 8));
        assert!(!plays("a.mp4", "hevc", 8));
        assert!(!plays("a.mp4", "h264", 10));
        assert!(!plays("a.mp4", "av1", 8));
    }
}
