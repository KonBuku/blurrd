//! Video input and output for blurrd, built on FFmpeg.
//!
//! This is the only blurrd crate with `unsafe` code: it calls FFmpeg through
//! `ffmpeg-the-third` (and its raw bindings where the safe wrapper has no
//! API). Every `unsafe` block states why it is sound. FFmpeg types never
//! appear in the public API.
//!
//! What it provides so far:
//!
//! - [`init`]: one-time setup (log bridge into `tracing`, library version
//!   check). Call it before anything else.
//! - [`probe()`]: everything blurrd needs to know about an input
//!   ([`blurrd_core::MediaInfo`]): streams, colour tags, HDR, B-frames, edit
//!   lists and the keyframes as presentation frame indices.
//! - [`Decoder`]: decodes the video stream in presentation order. Every
//!   [`VideoFrame`] carries its presentation index and PTS.
//! - [`VideoFrame::roi_yuv`] and [`RgbConverter`]: even-aligned HUD crops
//!   in a layout the blur works on, and their RGB conversion (bit-exact
//!   with FFmpeg's `bgr24` for SDR, tone-mapped for HDR) with every colour
//!   tag mapped ([`VideoColour`]).
//! - [`VideoFrame::thumbnail`]: a downscaled RGB copy of a whole frame,
//!   area-averaged in the YUV domain (thumbnails, the quarter-scale view
//!   of the screen gates).
//! - [`FrameSeeker`]: random access by presentation index with a frame
//!   cache, prefetching and a PTS check against the project's table.
//! - [`DecoderChoice`]: picks software decoding, NVDEC or D3D11VA per video
//!   from a cached micro-benchmark; hardware frames are downloaded as NV12
//!   or P010.
//! - [`VideoEncoder`]: encodes frames through the fallback chain NVENC,
//!   AMF, Quick Sync, Media Foundation (no GPL encoders) and reports every
//!   fallback; [`probe_encoders`] checks once per machine which of them
//!   work. Output colour tags follow [`output_color`].
//! - [`Transcoder`]: the render. Decoding, blurring plus encoding, and
//!   muxing run on their own threads; only frames with boxes are copied
//!   before they are blurred, keyframes and timestamps stay the source's,
//!   and the output appears under its name only when it is complete.
//!   H.264 on NVENC runs as two sessions over keyframe-aligned segments
//!   (the GPU's two encoder engines); HEVC and AV1 split each frame instead.
//!   Audio, subtitle and data packets are copied unchanged (or audio is
//!   re-encoded as AAC), recorder metadata is stripped by default, and
//!   MP4/MOV outputs get their index in space reserved at the front.
//! - [`sys::gpu`]: the machine's GPUs from DXGI, identified by LUID.
//! - [`PtsTable`]: the PTS of every presentation frame, persisted as
//!   `pts.bin` in the project cache. Rendering and seeking compare decoded
//!   frames against it, so a frame index always means the same picture.
//!
//! "Frame index" always means the presentation-order index of the frames
//! the decoder outputs; decode order never appears in the API.
//!
//! Inputs are opened with FFmpeg's protocol whitelist set to `file`, so a
//! crafted file (a playlist, a reference movie) can never make FFmpeg open a
//! network connection or another protocol.

mod choice;
mod colour;
mod decode;
mod encode;
mod error;
mod frames;
mod hw;
mod input;
mod log_bridge;
mod mp4;
mod mux;
mod probe;
mod pts;
mod rgb;
mod roi;
mod seek;
mod segments;
mod streams;
pub mod sys;
mod thumb;
mod transcode;

use std::sync::OnceLock;

pub use choice::{DecodePurpose, DecoderChoice};
pub use colour::{ColourTransfer, VideoColour, output_color};
pub use decode::{DecodeOptions, Decoder, HwDecode, PixelLayout, VideoFrame};
pub use encode::{
    EncodeOptions, EncodedPacket, EncoderFallback, EncoderInput, VideoEncoder, probe_encoders,
};
pub use error::VideoError;
pub use probe::{probe, pts_table};
pub use pts::PtsTable;
pub use rgb::{RgbConverter, RgbImage};
pub use roi::{RoiLayout, YuvRoi};
pub use seek::{FrameSeeker, SeekerOptions};
pub use streams::SkippedStream;
pub use transcode::{BoxSource, MoovLayout, MuxOptions, TranscodeReport, Transcoder};

/// Sets up FFmpeg for blurrd. Safe to call more than once; only the first
/// call does the work.
///
/// Routes FFmpeg's log output (warnings and errors only) into `tracing`
/// under the target `ffmpeg`, and checks that the FFmpeg libraries loaded at
/// run time have the major versions blurrd was built against.
///
/// # Errors
///
/// [`VideoError::LibraryVersion`] when a loaded library has a different
/// major version (a wrong DLL next to the executable).
pub fn init() -> Result<(), VideoError> {
    static RESULT: OnceLock<Result<(), (&'static str, i32, String)>> = OnceLock::new();
    RESULT
        .get_or_init(|| {
            log_bridge::install();
            check_library_versions()
        })
        .clone()
        .map_err(|(library, expected, found)| VideoError::LibraryVersion {
            library,
            expected: expected.unsigned_abs(),
            found,
        })
}

/// The version of the linked FFmpeg libraries, for example `9.0.2`
/// (`avutil`'s FFmpeg release string).
pub fn ffmpeg_version() -> String {
    // SAFETY: `av_version_info` returns a pointer to a static, NUL-terminated
    // string owned by libavutil.
    unsafe {
        let ptr = ffmpeg_the_third::ffi::av_version_info();
        if ptr.is_null() {
            return String::from("unknown");
        }
        std::ffi::CStr::from_ptr(ptr).to_string_lossy().into_owned()
    }
}

/// Compares the major version of each loaded library with the headers
/// blurrd was compiled against.
fn check_library_versions() -> Result<(), (&'static str, i32, String)> {
    use ffmpeg_the_third::ffi;

    // SAFETY: the `*_version` functions take no arguments and return a
    // packed integer.
    let loaded = unsafe {
        [
            (
                "avutil",
                ffi::avutil_version(),
                ffi::LIBAVUTIL_VERSION_MAJOR,
            ),
            (
                "avcodec",
                ffi::avcodec_version(),
                ffi::LIBAVCODEC_VERSION_MAJOR,
            ),
            (
                "avformat",
                ffi::avformat_version(),
                ffi::LIBAVFORMAT_VERSION_MAJOR,
            ),
        ]
    };
    for (library, version, expected) in loaded {
        let major = (version >> 16).cast_signed();
        if major != expected {
            return Err((
                library,
                expected,
                format!("{major}.{}.{}", (version >> 8) & 0xff, version & 0xff),
            ));
        }
    }
    Ok(())
}
