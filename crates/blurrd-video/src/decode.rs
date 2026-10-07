//! Decoding the video stream in presentation order.

use std::path::Path;

use blurrd_core::Rational;
use ffmpeg_the_third::{codec, decoder, ffi, format, frame, media};

use crate::probe::positive_rational;
use crate::{VideoError, input};

/// Decoder settings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct DecodeOptions {
    /// Number of decoder threads; `None` lets FFmpeg choose (one per
    /// logical core, at most 16).
    pub threads: Option<u32>,
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
    time_base: Rational,
    next_index: u32,
    last_pts: Option<i64>,
    draining: bool,
    finished: bool,
}

impl std::fmt::Debug for Decoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Decoder")
            .field("stream", &self.stream)
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
        context.set_threading(codec::threading::Config {
            kind: codec::threading::Type::Frame,
            count: options.threads.map_or(0, |n| n as usize),
        });
        // SAFETY: `pkt_timebase` is a plain field the decoder reads to
        // compute best-effort timestamps; the context is not open yet.
        unsafe {
            (*context.as_mut_ptr()).pkt_timebase = stream.time_base().into();
        }
        let decoder = context
            .decoder()
            .video()
            .map_err(VideoError::ffmpeg("opening the video decoder"))?;
        Ok(Self {
            input,
            stream: stream_index,
            codec: decoder,
            time_base,
            next_index: 0,
            last_pts: None,
            draining: false,
            finished: false,
        })
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
        let mut decoded = frame::Video::empty();
        loop {
            if self.finished {
                return Ok(None);
            }
            match self.codec.receive_frame(&mut decoded) {
                Ok(()) => return self.emit(decoded).map(Some),
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

/// `EAGAIN` as returned by `avcodec_receive_frame` (`AVERROR(EAGAIN)`).
const EAGAIN: i32 = 11;
