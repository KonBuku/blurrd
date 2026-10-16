//! Decoding the video stream in presentation order.

use std::path::Path;

use blurrd_core::Rational;
use ffmpeg_the_third::{codec, decoder, ffi, format, frame, media};

use crate::hw::{self, Backend};
use crate::probe::positive_rational;
use crate::{PtsTable, VideoError, input};

/// Where frames are decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum HwDecode {
    /// NVDEC if the stream is supported there, then D3D11VA, then
    /// software. [`crate::DecoderChoice`] picks a concrete option from a
    /// benchmark instead.
    Auto,
    /// FFmpeg's software decoders with frame threads.
    #[default]
    Software,
    /// NVIDIA NVDEC (through CUDA); frames are downloaded as NV12 or P010.
    Nvdec,
    /// Direct3D 11 video decoding; frames are downloaded as NV12 or P010.
    D3d11va,
}

/// Decoder settings.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DecodeOptions {
    /// Where to decode.
    pub hw: HwDecode,
    /// Number of decoder threads; `None` lets FFmpeg choose for software
    /// decoding (one per logical core, at most 16) and uses
    /// four frame threads for hardware decoding.
    pub threads: Option<u32>,
    /// Extra hardware surfaces beyond what the codec needs, so frames can
    /// wait in queues while the decoder keeps going. At least the depth of
    /// the frame queue plus the frames in flight.
    pub extra_hw_frames: u32,
}

impl Default for DecodeOptions {
    fn default() -> Self {
        Self {
            hw: HwDecode::Software,
            threads: None,
            extra_hw_frames: 16,
        }
    }
}

impl DecodeOptions {
    /// Options for the given decoding back end, defaults otherwise.
    pub fn with_hw(hw: HwDecode) -> Self {
        Self {
            hw,
            ..Self::default()
        }
    }
}

/// How the samples of a decoded frame are stored. All layouts are 4:2:0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PixelLayout {
    /// Planar 8-bit (`yuv420p`, also `yuvj420p`).
    I420,
    /// Planar 10-bit in 16-bit little-endian samples, value in the low bits
    /// (`yuv420p10le`, software HEVC Main10 decoding).
    I420P10,
    /// Semi-planar 8-bit (`nv12`).
    Nv12,
    /// Semi-planar 10-bit in 16-bit little-endian samples, value in the
    /// high bits (`p010le`, hardware decoding of 10-bit video).
    P010,
}

impl PixelLayout {
    fn from_pixel(pixel: format::Pixel) -> Option<Self> {
        match pixel {
            format::Pixel::YUV420P | format::Pixel::YUVJ420P => Some(Self::I420),
            format::Pixel::YUV420P10LE => Some(Self::I420P10),
            format::Pixel::NV12 => Some(Self::Nv12),
            format::Pixel::P010LE => Some(Self::P010),
            _ => None,
        }
    }
}

/// A decoded frame in system memory.
///
/// Holds a reference-counted FFmpeg frame; [`VideoFrame::share`] adds a
/// reference without copying pixels.
pub struct VideoFrame {
    index: u32,
    pts: i64,
    layout: PixelLayout,
    frame: frame::Video,
}

impl std::fmt::Debug for VideoFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VideoFrame")
            .field("index", &self.index)
            .field("pts", &self.pts)
            .field("layout", &self.layout)
            .field("width", &self.width())
            .field("height", &self.height())
            .finish_non_exhaustive()
    }
}

impl VideoFrame {
    /// Presentation frame index.
    pub fn index(&self) -> u32 {
        self.index
    }

    /// Presentation timestamp in the stream's time base.
    pub fn pts(&self) -> i64 {
        self.pts
    }

    /// Sample layout.
    pub fn layout(&self) -> PixelLayout {
        self.layout
    }

    /// Width in pixels.
    pub fn width(&self) -> u32 {
        self.frame.width()
    }

    /// Height in pixels.
    pub fn height(&self) -> u32 {
        self.frame.height()
    }

    /// Whether the decoder marked the frame as a keyframe.
    pub fn is_key(&self) -> bool {
        // SAFETY: reading the flags of the valid frame owned by `self`.
        unsafe { (*self.frame.as_ptr()).flags & ffi::AV_FRAME_FLAG_KEY != 0 }
    }

    /// A frame from an FFmpeg frame whose pixel format matches `layout`.
    pub(crate) fn wrap(index: u32, pts: i64, layout: PixelLayout, frame: frame::Video) -> Self {
        Self {
            index,
            pts,
            layout,
            frame,
        }
    }

    /// The underlying FFmpeg frame.
    pub(crate) fn raw(&self) -> &frame::Video {
        &self.frame
    }

    /// The underlying FFmpeg frame, for changing its properties (never its
    /// pixels, which may be shared with the decoder).
    pub(crate) fn raw_mut(&mut self) -> &mut frame::Video {
        &mut self.frame
    }

    /// The underlying FFmpeg frame, consuming the handle.
    pub(crate) fn into_raw(self) -> frame::Video {
        self.frame
    }

    /// Another handle to the same pixels (an FFmpeg frame reference, no
    /// copy).
    ///
    /// # Errors
    ///
    /// [`VideoError::Ffmpeg`] when FFmpeg cannot allocate the reference.
    pub fn share(&self) -> Result<Self, VideoError> {
        // SAFETY: `av_frame_clone` allocates a new frame that references the
        // same buffers as the valid source frame; null means out of memory.
        // The new frame is owned by the returned `frame::Video`.
        let frame = unsafe {
            let ptr = ffi::av_frame_clone(self.frame.as_ptr());
            if ptr.is_null() {
                return Err(VideoError::Ffmpeg {
                    context: "referencing a frame",
                    source: ffmpeg_the_third::Error::Other { errno: 12 },
                });
            }
            frame::Video::wrap(ptr)
        };
        Ok(Self {
            index: self.index,
            pts: self.pts,
            layout: self.layout,
            frame,
        })
    }
}

/// Decodes the video stream of a file in presentation order.
///
/// Frames come out with consecutive presentation indices starting at 0 and
/// strictly increasing PTS (the decoder's best-effort timestamp). Frames
/// the container cuts away with an edit list are dropped by FFmpeg and
/// never get an index.
pub struct Decoder {
    input: format::context::Input,
    stream: usize,
    codec: decoder::Video,
    /// Hardware surface format, when a hardware device is attached.
    surface: Option<ffi::AVPixelFormat>,
    active: HwDecode,
    hardware_frames: u32,
    time_base: Rational,
    next_index: u32,
    last_pts: Option<i64>,
    draining: bool,
    finished: bool,
    /// A frame decoded while seeking, returned by the next `next_frame`.
    pending: Option<VideoFrame>,
}

impl std::fmt::Debug for Decoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Decoder")
            .field("stream", &self.stream)
            .field("active", &self.active)
            .field("time_base", &self.time_base)
            .field("next_index", &self.next_index)
            .finish_non_exhaustive()
    }
}

impl Decoder {
    /// Opens the main video stream of `path`.
    ///
    /// # Errors
    ///
    /// [`VideoError::Io`] for unreadable files, [`VideoError::NoVideoStream`]
    /// or [`VideoError::Ffmpeg`] when there is no decodable video.
    /// [`VideoError::Unsupported`] when [`HwDecode::Nvdec`] or
    /// [`HwDecode::D3d11va`] was requested and cannot decode this stream
    /// (no such device, or the codec has no such hardware path).
    pub fn open(path: &Path, options: &DecodeOptions) -> Result<Self, VideoError> {
        crate::init()?;
        let mut input = input::open(path)?;
        let stream_index = input::video_stream(&input)?;
        // Only the video stream is decoded; let the demuxer drop the rest.
        for mut stream in input.streams_mut() {
            if stream.index() != stream_index {
                // SAFETY: setting the public `discard` field of a stream
                // owned by `input`, which is allowed before reading packets.
                unsafe { (*stream.as_mut_ptr()).discard = ffi::AVDiscard::ALL };
            }
        }
        let stream = input
            .stream(stream_index)
            .ok_or(VideoError::NoVideoStream)?;
        if stream.parameters().medium() != media::Type::Video {
            return Err(VideoError::NoVideoStream);
        }
        let time_base = positive_rational(stream.time_base()).ok_or_else(|| {
            VideoError::Unsupported(String::from("the video stream has no time base"))
        })?;
        let mut context = codec::Context::from_parameters(stream.parameters())
            .map_err(VideoError::ffmpeg("reading the codec parameters"))?;
        // SAFETY: `pkt_timebase` is a plain field the decoder reads to
        // compute best-effort timestamps; the context is not open yet.
        unsafe {
            (*context.as_mut_ptr()).pkt_timebase = stream.time_base().into();
        }
        let (surface, active) = Self::attach_hw(&mut context, options)?;
        context.set_threading(codec::threading::Config {
            kind: codec::threading::Type::Frame,
            count: match (options.threads, surface) {
                (Some(n), _) => n as usize,
                // Frame threads keep several surfaces in flight, so decoding
                // and downloading overlap: one thread reaches ~480 fps on
                // 1080p H.264, four reach ~830 fps; more do not help.
                (None, Some(_)) => HW_FRAME_THREADS,
                (None, None) => 0,
            },
        });
        let decoder = context
            .decoder()
            .video()
            .map_err(VideoError::ffmpeg("opening the video decoder"))?;
        Ok(Self {
            input,
            stream: stream_index,
            codec: decoder,
            surface,
            active,
            hardware_frames: 0,
            time_base,
            next_index: 0,
            last_pts: None,
            draining: false,
            finished: false,
            pending: None,
        })
    }

    /// Attaches the hardware device the options ask for. Returns the
    /// surface format and the back end actually used.
    fn attach_hw(
        context: &mut codec::Context,
        options: &DecodeOptions,
    ) -> Result<(Option<ffi::AVPixelFormat>, HwDecode), VideoError> {
        let candidates: &[(Backend, HwDecode)] = match options.hw {
            HwDecode::Software => &[],
            HwDecode::Nvdec => &[(Backend::Nvdec, HwDecode::Nvdec)],
            HwDecode::D3d11va => &[(Backend::D3d11va, HwDecode::D3d11va)],
            HwDecode::Auto => &[
                (Backend::Nvdec, HwDecode::Nvdec),
                (Backend::D3d11va, HwDecode::D3d11va),
            ],
        };
        // SAFETY: reading the codec id of the valid, unopened context.
        let codec_id = unsafe { (*context.as_ptr()).codec_id };
        // SAFETY: plain lookup in FFmpeg's static decoder table.
        let codec = unsafe { ffi::avcodec_find_decoder(codec_id) };
        if codec.is_null() {
            return Err(VideoError::Ffmpeg {
                context: "finding a decoder",
                source: ffmpeg_the_third::Error::DecoderNotFound,
            });
        }
        let mut last_error = None;
        for &(backend, choice) in candidates {
            let Some(surface) = hw::surface_format(codec, backend) else {
                last_error = Some(VideoError::Unsupported(format!(
                    "{} cannot decode this codec",
                    backend.name()
                )));
                continue;
            };
            // SAFETY: the context is valid and not open yet.
            match unsafe {
                hw::attach_device(context.as_mut_ptr(), backend, options.extra_hw_frames)
            } {
                Ok(()) => return Ok((Some(surface), choice)),
                Err(error) => {
                    tracing::debug!(backend = backend.name(), %error, "hardware decoding unavailable");
                    last_error = Some(VideoError::Unsupported(format!(
                        "{}: {error}",
                        backend.name()
                    )));
                }
            }
        }
        match (options.hw, last_error) {
            (HwDecode::Nvdec | HwDecode::D3d11va, Some(error)) => Err(error),
            _ => Ok((None, HwDecode::Software)),
        }
    }

    /// The back end in use. Hardware decoders can still fall back to
    /// software per stream (unsupported profile); frames are then decoded
    /// in software and [`Decoder::hardware_frames`] stays zero.
    pub fn active(&self) -> HwDecode {
        self.active
    }

    /// Number of frames so far that were decoded on the GPU.
    pub fn hardware_frames(&self) -> u32 {
        self.hardware_frames
    }

    /// Time base of the PTS values.
    pub fn time_base(&self) -> Rational {
        self.time_base
    }

    /// Decodes the next frame in presentation order, or `None` at the end.
    ///
    /// # Errors
    ///
    /// [`VideoError::Ffmpeg`] when demuxing or decoding fails,
    /// [`VideoError::Unsupported`] for a pixel format blurrd does not
    /// process, [`VideoError::Timestamps`] when a frame has no PTS or the PTS
    /// does not increase.
    pub fn next_frame(&mut self) -> Result<Option<VideoFrame>, VideoError> {
        if let Some(frame) = self.pending.take() {
            return Ok(Some(frame));
        }
        match self.receive()? {
            Some(decoded) => self.emit(decoded).map(Some),
            None => Ok(None),
        }
    }

    /// Positions the decoder so that the next [`Decoder::next_frame`]
    /// returns frame `index` of `table` (the PTS table of this file).
    ///
    /// Seeks the demuxer to the last keyframe at or before the frame's PTS
    /// and decodes forward, dropping earlier frames. When the demuxer lands
    /// after the frame (its index is not exact for files with B-frames or
    /// edit lists), it seeks further back and tries again.
    ///
    /// # Errors
    ///
    /// [`VideoError::FrameOutOfRange`] for an index past the table,
    /// [`VideoError::FrameNotFound`] when no decoded frame has the expected
    /// PTS (the file does not match the table), [`VideoError::Ffmpeg`] when
    /// seeking or decoding fails.
    pub fn seek(&mut self, index: u32, table: &PtsTable) -> Result<(), VideoError> {
        let target = table.pts(index).ok_or(VideoError::FrameOutOfRange {
            index,
            count: table.len(),
        })?;
        let mut back = 0u32;
        loop {
            let from = index.saturating_sub(back);
            let timestamp = table.pts(from).unwrap_or(target);
            self.seek_demuxer(timestamp)?;
            let mut found = None;
            while let Some(decoded) = self.receive()? {
                let pts = decoded.timestamp();
                match pts {
                    Some(pts) if pts < target => {}
                    Some(pts) if pts == target => {
                        found = Some(decoded);
                        break;
                    }
                    // Landed after the target (or a frame without PTS).
                    _ => break,
                }
            }
            if let Some(decoded) = found {
                self.next_index = index;
                self.last_pts = None;
                let frame = self.emit(decoded)?;
                self.pending = Some(frame);
                return Ok(());
            }
            if from == 0 {
                return Err(VideoError::FrameNotFound { index, pts: target });
            }
            back = (back * 4).max(32);
        }
    }

    /// Positions the decoder at the keyframe at or before `timestamp`
    /// (stream time base), for sampling a video whose PTS table is not
    /// known yet. The next [`Decoder::next_frame`] returns that keyframe
    /// (or the first frame for a `timestamp` before it). The frames that
    /// follow carry their PTS, but their indices count from 0 at the
    /// keyframe: they are not presentation indices of the file.
    ///
    /// # Errors
    ///
    /// [`VideoError::Ffmpeg`] when seeking fails.
    pub fn seek_keyframe(&mut self, timestamp: i64) -> Result<(), VideoError> {
        self.seek_demuxer(timestamp)?;
        self.next_index = 0;
        self.last_pts = None;
        Ok(())
    }

    /// Seeks the demuxer to the keyframe at or before `timestamp` (stream
    /// time base) and resets the decoder.
    fn seek_demuxer(&mut self, timestamp: i64) -> Result<(), VideoError> {
        let stream = i32::try_from(self.stream).unwrap_or(i32::MAX);
        // SAFETY: the format context is valid and owned by `self.input`;
        // `avformat_seek_file` with `max_ts = timestamp` lands on a
        // keyframe at or before it.
        let ret = unsafe {
            ffi::avformat_seek_file(
                self.input.as_mut_ptr(),
                stream,
                i64::MIN,
                timestamp,
                timestamp,
                0,
            )
        };
        if ret < 0 {
            return Err(VideoError::ffmpeg("seeking")(
                ffmpeg_the_third::Error::from(ret),
            ));
        }
        self.codec.flush();
        self.draining = false;
        self.finished = false;
        self.pending = None;
        Ok(())
    }

    /// Receives the next decoded frame in system memory, feeding packets as
    /// needed; `None` at the end of the stream.
    fn receive(&mut self) -> Result<Option<frame::Video>, VideoError> {
        let mut decoded = frame::Video::empty();
        loop {
            if self.finished {
                return Ok(None);
            }
            match self.codec.receive_frame(&mut decoded) {
                Ok(()) => {
                    let (decoded, on_gpu) = hw::download(decoded, self.surface)?;
                    self.hardware_frames += u32::from(on_gpu);
                    return Ok(Some(decoded));
                }
                Err(ffmpeg_the_third::Error::Eof) => {
                    self.finished = true;
                    return Ok(None);
                }
                Err(ffmpeg_the_third::Error::Other { errno }) if errno == EAGAIN => {
                    self.feed()?;
                }
                Err(error) => return Err(VideoError::ffmpeg("decoding")(error)),
            }
        }
    }

    /// Sends the next video packet (or the end of stream) to the decoder.
    fn feed(&mut self) -> Result<(), VideoError> {
        if self.draining {
            // The decoder asked for input after EOF was sent; it has nothing
            // more to give.
            self.finished = true;
            return Ok(());
        }
        let mut packet = codec::packet::Packet::empty();
        loop {
            match packet.read(&mut self.input) {
                Ok(()) if packet.stream() == self.stream => {
                    return self
                        .codec
                        .send_packet(&packet)
                        .map_err(VideoError::ffmpeg("decoding"));
                }
                Ok(()) => {}
                Err(ffmpeg_the_third::Error::Eof) => {
                    self.draining = true;
                    return self
                        .codec
                        .send_eof()
                        .map_err(VideoError::ffmpeg("decoding"));
                }
                Err(error) => return Err(VideoError::ffmpeg("reading packets")(error)),
            }
        }
    }

    /// Turns a decoded frame into a [`VideoFrame`] with the next index.
    fn emit(&mut self, decoded: frame::Video) -> Result<VideoFrame, VideoError> {
        let index = self.next_index;
        let pts = decoded
            .timestamp()
            .ok_or_else(|| VideoError::Timestamps(format!("frame {index} has no timestamp")))?;
        if let Some(last) = self.last_pts
            && pts <= last
        {
            return Err(VideoError::Timestamps(format!(
                "frame {index} has PTS {pts}, not after the previous frame's {last}"
            )));
        }
        let layout = PixelLayout::from_pixel(decoded.format()).ok_or_else(|| {
            VideoError::Unsupported(format!(
                "pixel format {:?} (blurrd needs 4:2:0)",
                decoded.format()
            ))
        })?;
        self.last_pts = Some(pts);
        self.next_index += 1;
        Ok(VideoFrame {
            index,
            pts,
            layout,
            frame: decoded,
        })
    }
}

/// Frame threads for hardware decoding (see [`DecodeOptions::threads`]).
const HW_FRAME_THREADS: usize = 4;

/// `EAGAIN` as returned by `avcodec_receive_frame` (`AVERROR(EAGAIN)`).
const EAGAIN: i32 = 11;
