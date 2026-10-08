//! Video encoding: the hardware fallback chain and the encoder probe.
//!
//! blurrd ships an LGPL FFmpeg without software H.264/HEVC encoders, so
//! output always goes through a hardware or operating-system encoder. For
//! every output codec the chain is NVENC, AMF, Quick Sync and finally Media
//! Foundation (which Windows provides on every machine). NVENC, AMF and
//! Quick Sync are tried only when a GPU of their vendor exists, and
//! encoders that the cached probe ([`probe_encoders`]) found unusable are
//! skipped. When an encoder that should work fails to open, the next one is
//! used and the failure is reported as an [`EncoderFallback`], never
//! silently.
//!
//! Two software encoders exist only for tests (`mpeg4` and the lossless
//! `ffv1`); they are never part of the chain and can only be named
//! explicitly.

use std::ffi::c_int;
use std::path::Path;
use std::ptr;
use std::sync::mpsc;
use std::time::Duration;

use blurrd_core::{
    Codec, ColorInfo, ColorRange, EncoderInfo, EncoderSetting, ExportSettings, KeyframePolicy,
    MediaInfo, RateControl, Rational, Toggle,
};
use ffmpeg_the_third::codec::packet::Mut as _;
use ffmpeg_the_third::{Dictionary, codec, ffi};
use serde::{Deserialize, Serialize};

use crate::colour::apply_color_tags;
use crate::sys::gpu;
use crate::{PixelLayout, VideoError, VideoFrame};

/// How to encode the video stream.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct EncodeOptions {
    /// Output codec; [`Codec::Source`] keeps H.264, HEVC and AV1 sources in
    /// their codec and encodes everything else as H.264.
    pub codec: Codec,
    /// The fallback chain or one named encoder.
    pub encoder: EncoderSetting,
    /// Result of [`probe_encoders`]: encoders it found unusable are left out
    /// of the chain. Empty means "not probed": every encoder whose GPU
    /// vendor is present is tried.
    pub probed: Vec<EncoderInfo>,
    /// Rate control.
    pub rate: RateControl,
    /// Speed/quality preset from 1 (fastest) to 7 (best quality), mapped to
    /// each encoder's own presets.
    pub preset: u8,
    /// 10-bit output. `Auto` keeps 10-bit sources in 10 bits when the
    /// output codec is HEVC or AV1 and the encoder supports it.
    pub ten_bit: Toggle,
    /// NVENC split-frame encoding across the GPU's encoder engines for HEVC
    /// and AV1. `Auto` splits into two strips below 4K and lets the driver
    /// decide at 4K and above.
    pub split_encode: Toggle,
    /// Keyframe placement. `MatchSource` makes every frame sent as a
    /// keyframe an IDR frame and lets the encoder add no keyframes of its
    /// own within the source's longest keyframe interval.
    pub keyframes: KeyframePolicy,
    /// NVENC input surfaces (frames in flight inside the encoder); `None`
    /// keeps the encoder's default.
    pub surfaces: Option<u8>,
    /// Whether GPU encoders (NVENC, AMF, Quick Sync) may be used. Without
    /// them, only Media Foundation is tried.
    pub gpu: bool,
}

impl Default for EncodeOptions {
    fn default() -> Self {
        Self {
            codec: Codec::Source,
            encoder: EncoderSetting::Auto,
            probed: Vec::new(),
            rate: RateControl::default(),
            preset: DEFAULT_PRESET,
            ten_bit: Toggle::Auto,
            split_encode: Toggle::Auto,
            keyframes: KeyframePolicy::MatchSource,
            surfaces: None,
            gpu: true,
        }
    }
}

impl EncodeOptions {
    /// The encoding part of the user's export settings.
    pub fn from_export(export: &ExportSettings) -> Self {
        Self {
            codec: export.codec,
            encoder: export.encoder.clone(),
            rate: export.rate,
            preset: export.preset,
            ten_bit: export.ten_bit,
            split_encode: export.split_encode,
            keyframes: export.keyframes,
            ..Self::default()
        }
    }
}

/// Preset used until the M18 quality benchmark picks the default.
const DEFAULT_PRESET: u8 = 4;
/// Peak bit rate factor of the default rate control (match source).
const DEFAULT_MAX_FACTOR: f32 = 1.5;

/// The video an encoder is opened for.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct EncoderInput {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Layout of the frames that will be sent.
    pub layout: PixelLayout,
    /// Time base of the frames' PTS; packets come out in the same time base.
    pub time_base: Rational,
    /// Nominal frame rate, used for rate control.
    pub frame_rate: Rational,
    /// Sample aspect ratio, if the source states one.
    pub sample_aspect: Option<Rational>,
    /// FFmpeg codec name of the source, used by [`Codec::Source`].
    pub source_codec: String,
    /// Colour tags written into the output (see [`crate::output_color`]).
    pub color: ColorInfo,
    /// Bit rate of the source's video stream, the target of
    /// [`RateControl::MatchSource`]. Without it, 0.12 bits per pixel are
    /// used.
    pub bitrate: Option<u64>,
    /// Longest interval in frames between two keyframes the encoder may
    /// leave on its own. With [`KeyframePolicy::MatchSource`] this is the
    /// source's longest keyframe interval, so every keyframe of the output
    /// is one that was asked for.
    pub keyframe_interval: u32,
}

impl EncoderInput {
    /// The input for frames of `layout` decoded from the video `info`
    /// describes: size, timing, colour tags (normalised), the video bit rate
    /// (the file's bit rate minus its audio streams) and the longest
    /// keyframe interval.
    pub fn from_media(info: &MediaInfo, layout: PixelLayout) -> Self {
        let audio: u64 = info.audio.iter().filter_map(|a| a.bitrate).sum();
        let frames = info.frame_count.unwrap_or(0);
        let keyframe_interval = info.keyframes.as_deref().map_or(0, |keys| {
            keys.windows(2)
                .map(|w| w[1] - w[0])
                .chain(keys.last().map(|&last| frames.saturating_sub(last)))
                .max()
                .unwrap_or(0)
        });
        Self {
            width: info.width,
            height: info.height,
            layout,
            time_base: info.time_base,
            frame_rate: info.fps,
            sample_aspect: None,
            source_codec: info.video_codec.clone(),
            color: crate::output_color(&info.color, info.height),
            bitrate: info.bitrate.map(|total| total.saturating_sub(audio)),
            // Twice the frame rate when the source has a single keyframe or
            // none is known; at least 1.
            keyframe_interval: if keyframe_interval > 0 {
                keyframe_interval + 1
            } else {
                (2 * info.fps.num / info.fps.den.max(1)).max(1)
            },
        }
    }

    /// Bits per sample of the frames.
    fn bit_depth(&self) -> u8 {
        match self.layout {
            PixelLayout::I420P10 | PixelLayout::P010 => 10,
            _ => 8,
        }
    }

    /// The target bit rate when matching the source.
    fn source_bitrate(&self) -> u64 {
        self.bitrate.filter(|&b| b > 0).unwrap_or_else(|| {
            let pixels_per_second =
                f64::from(self.width) * f64::from(self.height) * f64::from(self.frame_rate.num)
                    / f64::from(self.frame_rate.den.max(1));
            (pixels_per_second * 0.12) as u64
        })
    }
}

/// An encoder that failed to open, and the one used instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncoderFallback {
    /// The encoder that failed, for example `h264_nvenc`.
    pub from: String,
    /// The encoder used instead.
    pub to: String,
    /// Why the first one failed (FFmpeg's error message).
    pub reason: String,
}

/// Encoder families, in fallback order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Family {
    /// NVIDIA NVENC.
    Nvenc,
    /// AMD Advanced Media Framework.
    Amf,
    /// Intel Quick Sync Video.
    Qsv,
    /// Windows Media Foundation transforms.
    MediaFoundation,
    /// FFmpeg's software encoders `mpeg4` and `ffv1`, for tests only.
    Test,
}

/// The chain, in order.
const CHAIN: [Family; 4] = [
    Family::Nvenc,
    Family::Amf,
    Family::Qsv,
    Family::MediaFoundation,
];

/// Codecs blurrd encodes to.
const OUTPUT_CODECS: [Codec; 3] = [Codec::H264, Codec::Hevc, Codec::Av1];

/// Software encoders that may be named explicitly in tests.
const TEST_ENCODERS: [&str; 2] = ["mpeg4", "ffv1"];

impl Family {
    /// The PCI vendor whose GPU the family needs, if any.
    fn vendor(self) -> Option<u32> {
        match self {
            Self::Nvenc => Some(gpu::VENDOR_NVIDIA),
            Self::Amf => Some(gpu::VENDOR_AMD),
            Self::Qsv => Some(gpu::VENDOR_INTEL),
            Self::MediaFoundation | Self::Test => None,
        }
    }

    fn hardware(self) -> bool {
        matches!(self, Self::Nvenc | Self::Amf | Self::Qsv)
    }
}

/// FFmpeg's encoder name for a family and a concrete output codec.
fn encoder_name(family: Family, codec: Codec) -> Option<&'static str> {
    Some(match (family, codec) {
        (Family::Nvenc, Codec::H264) => "h264_nvenc",
        (Family::Nvenc, Codec::Hevc) => "hevc_nvenc",
        (Family::Nvenc, Codec::Av1) => "av1_nvenc",
        (Family::Amf, Codec::H264) => "h264_amf",
        (Family::Amf, Codec::Hevc) => "hevc_amf",
        (Family::Amf, Codec::Av1) => "av1_amf",
        (Family::Qsv, Codec::H264) => "h264_qsv",
        (Family::Qsv, Codec::Hevc) => "hevc_qsv",
        (Family::Qsv, Codec::Av1) => "av1_qsv",
        (Family::MediaFoundation, Codec::H264) => "h264_mf",
        (Family::MediaFoundation, Codec::Hevc) => "hevc_mf",
        (Family::MediaFoundation, Codec::Av1) => "av1_mf",
        _ => return None,
    })
}

/// FFmpeg's codec name for a concrete output codec.
fn codec_name(codec: Codec) -> &'static str {
    match codec {
        Codec::Hevc => "hevc",
        Codec::Av1 => "av1",
        _ => "h264",
    }
}

/// The family, static name and codec of a named encoder blurrd may use.
fn named(name: &str) -> Option<(Family, &'static str, Option<Codec>)> {
    if let Some(&test) = TEST_ENCODERS.iter().find(|&&n| n == name) {
        return Some((Family::Test, test, None));
    }
    CHAIN.iter().find_map(|&family| {
        OUTPUT_CODECS.iter().find_map(|&codec| {
            encoder_name(family, codec)
                .filter(|&n| n == name)
                .map(|n| (family, n, Some(codec)))
        })
    })
}

/// The concrete output codec for `requested` and a source codec name.
fn output_codec(requested: Codec, source_codec: &str) -> Codec {
    match requested {
        Codec::Source => match source_codec {
            "hevc" => Codec::Hevc,
            "av1" => Codec::Av1,
            _ => Codec::H264,
        },
        other => other,
    }
}

/// An encoded video packet.
pub struct EncodedPacket {
    packet: codec::packet::Packet,
}

impl std::fmt::Debug for EncodedPacket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EncodedPacket")
            .field("pts", &self.pts())
            .field("dts", &self.dts())
            .field("key", &self.is_key())
            .field("size", &self.packet.size())
            .finish()
    }
}

impl EncodedPacket {
    /// Presentation timestamp in the encoder's time base.
    pub fn pts(&self) -> Option<i64> {
        self.packet.pts()
    }

    /// Decoding timestamp in the encoder's time base.
    pub fn dts(&self) -> Option<i64> {
        self.packet.dts()
    }

    /// Whether the packet is a keyframe (an IDR frame for H.264/HEVC).
    pub fn is_key(&self) -> bool {
        self.packet.is_key()
    }

    /// The compressed data.
    pub fn data(&self) -> &[u8] {
        self.packet.data().unwrap_or_default()
    }

    /// The FFmpeg packet, for the muxer.
    pub(crate) fn into_inner(self) -> codec::packet::Packet {
        self.packet
    }
}

/// An open video encoder.
pub struct VideoEncoder {
    context: codec::Context,
    name: &'static str,
    codec: Codec,
    input: PixelLayout,
    size: (u32, u32),
}

impl std::fmt::Debug for VideoEncoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VideoEncoder")
            .field("name", &self.name)
            .field("codec", &self.codec)
            .field("input", &self.input)
            .finish_non_exhaustive()
    }
}

/// `EAGAIN` as returned by `avcodec_receive_packet`.
const EAGAIN: i32 = 11;
/// `ENOSYS`, returned by NVENC when the driver is older than its API.
const ENOSYS: i32 = 40;

impl VideoEncoder {
    /// Opens the first working encoder of the chain, or the named encoder.
    ///
    /// Returns the encoder and, in order, every encoder that should have
    /// worked but failed to open.
    ///
    /// # Errors
    ///
    /// [`VideoError::Unsupported`] for an unknown encoder name or a named
    /// encoder of another codec; [`VideoError::EncoderUnavailable`] when no
    /// encoder of the chain opens.
    pub fn open(
        input: &EncoderInput,
        options: &EncodeOptions,
    ) -> Result<(Self, Vec<EncoderFallback>), VideoError> {
        crate::init()?;
        open_first(
            input,
            options,
            candidates(input, options, &gpu::adapters())?,
        )
    }

    /// FFmpeg's name of the encoder, for example `h264_nvenc`.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// The output codec ([`Codec::Source`] for the test encoders).
    pub fn codec(&self) -> Codec {
        self.codec
    }

    /// Layout the frames must have.
    pub fn input_layout(&self) -> PixelLayout {
        self.input
    }

    /// Codec headers (SPS/PPS and the like) for the container.
    pub fn extradata(&self) -> &[u8] {
        // SAFETY: `extradata` and `extradata_size` of an open encoder
        // describe a buffer owned by the context, which lives as long as
        // `self`.
        unsafe {
            let ctx = self.context.as_ptr();
            let size = usize::try_from((*ctx).extradata_size).unwrap_or(0);
            if (*ctx).extradata.is_null() || size == 0 {
                &[]
            } else {
                std::slice::from_raw_parts((*ctx).extradata, size)
            }
        }
    }

    /// Time base of the packets' timestamps.
    pub(crate) fn time_base(&self) -> ffmpeg_the_third::Rational {
        // SAFETY: reading a field of the open context.
        unsafe { (*self.context.as_ptr()).time_base.into() }
    }

    /// Fills a stream's codec parameters for the muxer.
    ///
    /// # Safety
    ///
    /// `parameters` must point to valid codec parameters.
    pub(crate) unsafe fn fill_parameters(
        &self,
        parameters: *mut ffi::AVCodecParameters,
    ) -> Result<(), VideoError> {
        // SAFETY: the context is open; `parameters` is valid by the
        // caller's contract.
        let ret =
            unsafe { ffi::avcodec_parameters_from_context(parameters, self.context.as_ptr()) };
        if ret < 0 {
            return Err(VideoError::encode("copying encoder parameters")(
                ffmpeg_the_third::Error::from(ret),
            ));
        }
        Ok(())
    }

    /// Sends a frame. With `keyframe`, the encoder makes it a keyframe (an
    /// IDR frame with [`KeyframePolicy::MatchSource`]). The frame keeps its
    /// PTS, so the output has the source's timing.
    ///
    /// # Errors
    ///
    /// [`VideoError::Unsupported`] when the frame's layout or size differs
    /// from what the encoder was opened for; [`VideoError::Encode`] when the
    /// encoder fails.
    pub fn send(&mut self, frame: &mut VideoFrame, keyframe: bool) -> Result<(), VideoError> {
        if frame.layout() != self.input || (frame.width(), frame.height()) != self.size {
            return Err(VideoError::Unsupported(format!(
                "{} needs {:?} frames of {}x{}, got {:?} {}x{}",
                self.name,
                self.input,
                self.size.0,
                self.size.1,
                frame.layout(),
                frame.width(),
                frame.height()
            )));
        }
        let pts = frame.pts();
        let raw = frame.raw_mut();
        // SAFETY: plain fields of the frame owned by `frame`; the pixel
        // buffers are not touched. The frame is valid and only read by
        // `avcodec_send_frame`.
        let ret = unsafe {
            let ptr = raw.as_mut_ptr();
            (*ptr).pts = pts;
            (*ptr).pict_type = if keyframe {
                ffi::AVPictureType::I
            } else {
                ffi::AVPictureType::NONE
            };
            ffi::avcodec_send_frame(self.context.as_mut_ptr(), ptr)
        };
        if ret < 0 {
            return Err(VideoError::encode("encoding a frame")(
                ffmpeg_the_third::Error::from(ret),
            ));
        }
        Ok(())
    }

    /// Tells the encoder that no more frames follow; [`VideoEncoder::receive`]
    /// then returns the remaining packets.
    ///
    /// # Errors
    ///
    /// [`VideoError::Encode`] when the encoder fails.
    pub fn flush(&mut self) -> Result<(), VideoError> {
        // SAFETY: a null frame signals the end of the stream.
        let ret = unsafe { ffi::avcodec_send_frame(self.context.as_mut_ptr(), ptr::null()) };
        if ret < 0 && ret != ffi::AVERROR_EOF {
            return Err(VideoError::encode("flushing the encoder")(
                ffmpeg_the_third::Error::from(ret),
            ));
        }
        Ok(())
    }

    /// The next encoded packet, or `None` when the encoder needs more
    /// frames or is drained.
    ///
    /// # Errors
    ///
    /// [`VideoError::Encode`] when the encoder fails.
    pub fn receive(&mut self) -> Result<Option<EncodedPacket>, VideoError> {
        let mut packet = codec::packet::Packet::empty();
        // SAFETY: the context is open; the packet is valid and owned here.
        let ret =
            unsafe { ffi::avcodec_receive_packet(self.context.as_mut_ptr(), packet.as_mut_ptr()) };
        match ret {
            0 => Ok(Some(EncodedPacket { packet })),
            ffi::AVERROR_EOF => Ok(None),
            e if e == -EAGAIN => Ok(None),
            e => Err(VideoError::encode("encoding a frame")(
                ffmpeg_the_third::Error::from(e),
            )),
        }
    }
}

/// Opens the first candidate that works and lists the ones that failed
/// before it.
fn open_first(
    input: &EncoderInput,
    options: &EncodeOptions,
    candidates: Vec<(Family, Codec, &'static str)>,
) -> Result<(VideoEncoder, Vec<EncoderFallback>), VideoError> {
    let mut failures: Vec<(&'static str, String, bool)> = Vec::new();
    for (family, codec, name) in candidates {
        match try_open(input, options, family, codec, name) {
            Ok(encoder) => {
                let fallbacks = failures
                    .into_iter()
                    .map(|(from, reason, _)| EncoderFallback {
                        from: from.to_owned(),
                        to: name.to_owned(),
                        reason,
                    })
                    .collect();
                return Ok((encoder, fallbacks));
            }
            Err(Failure { reason, errno }) => {
                tracing::warn!(encoder = name, %reason, "encoder failed to open");
                failures.push((name, reason, family == Family::Nvenc && errno == ENOSYS));
            }
        }
    }
    let driver_too_old = failures.iter().any(|f| f.2);
    Err(VideoError::EncoderUnavailable {
        tried: failures
            .into_iter()
            .map(|(name, reason, _)| (name.to_owned(), reason))
            .collect(),
        driver_too_old,
    })
}

/// The encoders to try, in order, with their family and output codec.
fn candidates(
    input: &EncoderInput,
    options: &EncodeOptions,
    adapters: &[gpu::Adapter],
) -> Result<Vec<(Family, Codec, &'static str)>, VideoError> {
    if let EncoderSetting::Named(name) = &options.encoder {
        let (family, static_name, own) = named(name)
            .ok_or_else(|| VideoError::Unsupported(format!("unknown encoder {name}")))?;
        let codec = match (own, options.codec) {
            (Some(own), Codec::Source) => own,
            (Some(own), requested) if own == requested => own,
            (Some(own), requested) => {
                return Err(VideoError::Unsupported(format!(
                    "{name} encodes {}, not {}",
                    codec_name(own),
                    codec_name(requested)
                )));
            }
            // Test encoders have their own codec.
            (None, _) => Codec::Source,
        };
        return Ok(vec![(family, codec, static_name)]);
    }

    let codec = output_codec(options.codec, &input.source_codec);
    let has_vendor = |vendor: u32| adapters.iter().any(|a| a.vendor_id == vendor);
    let usable = |family: Family| -> bool {
        family.vendor().is_none_or(has_vendor) && (options.gpu || !family.hardware())
    };
    let chain: Vec<_> = CHAIN
        .iter()
        .copied()
        .filter(|&family| usable(family))
        .filter_map(|family| encoder_name(family, codec).map(|name| (family, codec, name)))
        .collect();
    // Leave out what the probe found unusable, unless that leaves nothing
    // (a stale probe must not make export impossible).
    let probed_ok: Vec<_> = chain
        .iter()
        .copied()
        .filter(|&(_, _, name)| {
            options
                .probed
                .iter()
                .find(|info| info.name == name)
                .is_none_or(|info| info.available)
        })
        .collect();
    Ok(if probed_ok.is_empty() {
        chain
    } else {
        probed_ok
    })
}

/// Why one candidate could not be used.
#[derive(Debug)]
struct Failure {
    /// FFmpeg's (or blurrd's) explanation.
    reason: String,
    /// The FFmpeg error number, or 0.
    errno: i32,
}

impl Failure {
    fn new(reason: &str) -> Self {
        Self {
            reason: reason.to_owned(),
            errno: 0,
        }
    }
}

/// Whether the output is 10-bit for these options.
fn wants_ten_bit(options: &EncodeOptions, input: &EncoderInput, codec: Codec) -> bool {
    match options.ten_bit {
        Toggle::On => true,
        Toggle::Off => false,
        Toggle::Auto => input.bit_depth() > 8 && matches!(codec, Codec::Hevc | Codec::Av1),
    }
}

/// Pixel formats an encoder accepts, or `None` if it accepts any.
fn supported_formats(codec: *const ffi::AVCodec) -> Option<Vec<ffi::AVPixelFormat>> {
    let mut list: *const std::ffi::c_void = ptr::null();
    let mut count: c_int = 0;
    // SAFETY: `codec` is a valid encoder from FFmpeg's static table; the
    // returned list is static data terminated by `AV_PIX_FMT_NONE`.
    let ret = unsafe {
        ffi::avcodec_get_supported_config(
            ptr::null(),
            codec,
            ffi::AVCodecConfig::PIX_FORMAT,
            0,
            &raw mut list,
            &raw mut count,
        )
    };
    if ret < 0 || list.is_null() {
        return None;
    }
    let count = usize::try_from(count).unwrap_or(0);
    // SAFETY: FFmpeg reported `count` valid entries at `list`.
    let formats = unsafe { std::slice::from_raw_parts(list.cast::<ffi::AVPixelFormat>(), count) };
    Some(formats.to_vec())
}

/// The layout an encoder takes for frames of `source` layout, if any. The
/// blur works on I420, NV12 and P010, so these are the only candidates.
fn input_layout(
    formats: Option<&[ffi::AVPixelFormat]>,
    source: PixelLayout,
    ten_bit: bool,
) -> Option<PixelLayout> {
    let accepts =
        |layout: PixelLayout| formats.is_none_or(|list| list.contains(&pixel_format(layout)));
    if ten_bit {
        return accepts(PixelLayout::P010).then_some(PixelLayout::P010);
    }
    let preferred = match source {
        PixelLayout::I420 => [PixelLayout::I420, PixelLayout::Nv12],
        _ => [PixelLayout::Nv12, PixelLayout::I420],
    };
    preferred.into_iter().find(|&layout| accepts(layout))
}

/// FFmpeg's pixel format for a frame layout.
pub(crate) fn pixel_format(layout: PixelLayout) -> ffi::AVPixelFormat {
    match layout {
        PixelLayout::I420 => ffi::AVPixelFormat::YUV420P,
        PixelLayout::I420P10 => ffi::AVPixelFormat::YUV420P10LE,
        PixelLayout::Nv12 => ffi::AVPixelFormat::NV12,
        PixelLayout::P010 => ffi::AVPixelFormat::P010LE,
    }
}

/// Opens one encoder.
fn try_open(
    input: &EncoderInput,
    options: &EncodeOptions,
    family: Family,
    codec: Codec,
    name: &'static str,
) -> Result<VideoEncoder, Failure> {
    let encoder = ffmpeg_the_third::encoder::find_by_name(name)
        .ok_or_else(|| Failure::new("not included in this FFmpeg build"))?;
    let ten_bit = family != Family::Test && wants_ten_bit(options, input, codec);
    let formats = supported_formats(encoder.as_ptr());
    let layout = input_layout(formats.as_deref(), input.layout, ten_bit).ok_or_else(|| {
        Failure::new(if ten_bit {
            "10-bit output is not supported"
        } else {
            "no supported pixel format"
        })
    })?;

    let mut context = codec::Context::new_with_codec(encoder);
    let mut dict = Dictionary::new();
    // SAFETY: the context was just allocated for this encoder and is not
    // open; only plain fields are written.
    unsafe {
        let ctx = context.as_mut_ptr();
        (*ctx).codec_type = ffi::AVMediaType::VIDEO;
        (*ctx).width = input.width.cast_signed();
        (*ctx).height = input.height.cast_signed();
        (*ctx).pix_fmt = pixel_format(layout);
        if let Some(sar) = input.sample_aspect {
            (*ctx).sample_aspect_ratio = rational(sar);
        }
        (*ctx).time_base = rational(input.time_base);
        (*ctx).framerate = rational(input.frame_rate);
        (*ctx).gop_size = c_int::try_from(input.keyframe_interval).unwrap_or(c_int::MAX);
        // B-frames stay off until the M18 quality benchmark decides: without
        // them DTS equals PTS, which keeps segment stitching trivial.
        (*ctx).max_b_frames = 0;
        // Every supported container (MP4, MOV, Matroska) stores the codec
        // headers out of band.
        (*ctx).flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER.cast_signed();
        apply_color_tags(ctx, &input.color);
        configure(ctx, &mut dict, input, options, family, codec);
    }

    // SAFETY: the context is valid and configured; `dict` is a valid
    // dictionary that `avcodec_open2` updates with the unused options.
    let ret =
        unsafe { ffi::avcodec_open2(context.as_mut_ptr(), encoder.as_ptr(), dict.as_mut_ptr()) };
    if ret < 0 {
        let error = ffmpeg_the_third::Error::from(ret);
        let errno = match error {
            ffmpeg_the_third::Error::Other { errno } => errno,
            _ => 0,
        };
        return Err(Failure {
            reason: error.to_string(),
            errno,
        });
    }
    for (key, value) in dict.iter() {
        tracing::debug!(encoder = name, key, value, "encoder ignored an option");
    }
    Ok(VideoEncoder {
        context,
        name,
        codec,
        input: layout,
        size: (input.width, input.height),
    })
}

fn rational(r: Rational) -> ffi::AVRational {
    ffi::AVRational {
        num: c_int::try_from(r.num).unwrap_or(c_int::MAX),
        den: c_int::try_from(r.den).unwrap_or(c_int::MAX),
    }
}

/// Rate control targets: `(target, peak)` in bit/s for the bit rate modes,
/// or the quality level (0–51, lower is better) for constant quality.
fn rate_targets(input: &EncoderInput, rate: RateControl) -> (Option<(u64, u64)>, Option<u8>) {
    let match_source = |max_factor: f32| {
        let target = input.source_bitrate();
        let peak = (target as f64 * f64::from(max_factor.max(1.0))) as u64;
        (Some((target, peak)), None)
    };
    match rate {
        RateControl::MatchSource { max_factor } => match_source(max_factor),
        RateControl::Vbr {
            target_kbps,
            max_kbps,
        } => (
            Some((
                u64::from(target_kbps) * 1000,
                u64::from(max_kbps.max(target_kbps)) * 1000,
            )),
            None,
        ),
        RateControl::Cq(level) => (None, Some(level.min(51))),
        // Rate controls added later behave like the default until they are
        // mapped here.
        _ => match_source(DEFAULT_MAX_FACTOR),
    }
}

/// Sets rate control, preset and keyframe options for a family.
///
/// # Safety
///
/// `ctx` must be a valid encoder context that is not open yet.
unsafe fn configure(
    ctx: *mut ffi::AVCodecContext,
    dict: &mut Dictionary,
    input: &EncoderInput,
    options: &EncodeOptions,
    family: Family,
    codec: Codec,
) {
    let preset = options.preset.clamp(1, 7);
    let force_idr = options.keyframes == KeyframePolicy::MatchSource;
    let (bitrate, quality) = rate_targets(input, options.rate);
    if let Some((target, peak)) = bitrate {
        // SAFETY: the caller guarantees a valid, unopened context.
        unsafe {
            (*ctx).bit_rate = i64::try_from(target).unwrap_or(i64::MAX);
            (*ctx).rc_max_rate = i64::try_from(peak).unwrap_or(i64::MAX);
            (*ctx).rc_buffer_size = c_int::try_from(peak.saturating_mul(2)).unwrap_or(c_int::MAX);
        }
    }
    let mut set = |key: &str, value: &str| dict.set(key, value);
    match family {
        Family::Nvenc => {
            set("preset", &format!("p{preset}"));
            set("tune", "hq");
            set("rc", "vbr");
            if let Some(level) = quality {
                set("cq", &level.to_string());
            }
            if force_idr {
                set("forced-idr", "1");
            }
            if let Some(surfaces) = options.surfaces {
                set("surfaces", &surfaces.to_string());
            }
            if matches!(codec, Codec::Hevc | Codec::Av1) {
                let mode = match options.split_encode {
                    Toggle::Off => "disabled",
                    Toggle::Auto if input.height >= 2160 => "auto",
                    Toggle::On | Toggle::Auto => "2",
                };
                set("split_encode_mode", mode);
            }
        }
        Family::Amf => {
            set("usage", "transcoding");
            set(
                "quality",
                match preset {
                    1..=3 => "speed",
                    4..=5 => "balanced",
                    _ => "quality",
                },
            );
            if let Some(level) = quality {
                set("rc", "cqp");
                set("qp_i", &level.to_string());
                set("qp_p", &level.to_string());
            } else {
                set("rc", "vbr_peak");
            }
            if force_idr {
                set("forced_idr", "1");
            }
        }
        Family::Qsv => {
            set(
                "preset",
                match preset {
                    1 => "veryfast",
                    2 => "faster",
                    3 => "fast",
                    4 => "medium",
                    5 => "slow",
                    6 => "slower",
                    _ => "veryslow",
                },
            );
            if let Some(level) = quality {
                // Intelligent constant quality.
                // SAFETY: as above.
                unsafe { (*ctx).global_quality = c_int::from(level.max(1)) };
            }
            if force_idr {
                set("forced_idr", "1");
            }
        }
        Family::MediaFoundation => {
            if let Some(level) = quality {
                set("rate_control", "quality");
                // Media Foundation's quality is 0–100, higher is better.
                let scaled = u32::from(51 - level) * 100 / 51;
                set("quality", &scaled.to_string());
            } else {
                set("rate_control", "pc_vbr");
            }
        }
        Family::Test => {
            if let Some(level) = quality {
                // MPEG-4 Part 2 quantiser scale (2–31) in lambda units.
                // SAFETY: as above.
                unsafe {
                    (*ctx).flags |= ffi::AV_CODEC_FLAG_QSCALE.cast_signed();
                    (*ctx).global_quality =
                        c_int::from(level.clamp(2, 31)) * ffi::FF_QP2LAMBDA as c_int;
                }
            }
        }
    }
}

/// Schema name of the encoder cache file.
const CACHE_SCHEMA: &str = "blurrd-encoders/1";
/// File name of the encoder cache inside the cache folder.
const CACHE_FILE: &str = "encoders.json";
/// Picture size of the probe's test encode (above every encoder's minimum).
const PROBE_SIZE: (u32, u32) = (640, 360);
/// Entries kept in the encoder cache; the oldest are dropped first.
const MAX_CACHE_ENTRIES: usize = 16;

/// The cached probe result for one machine configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct CacheEntry {
    /// LUID of the first (high-performance) GPU, empty without one.
    gpu: String,
    driver: String,
    ffmpeg: String,
    encoders: Vec<EncoderInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CacheFile {
    schema: String,
    entries: Vec<CacheEntry>,
}

/// Which encoders of the chain work on this machine, for every output
/// codec, in chain order.
///
/// Each encoder opens with an 8-bit test picture (and, if that works, a
/// 10-bit one) and encodes one frame. That takes one to two seconds, so the
/// result is cached in `cache_dir/encoders.json`, keyed by the first GPU's
/// LUID and driver version and the FFmpeg version; later calls only read
/// the cache. Run it on a background thread at startup. Never fails: an
/// encoder that cannot be checked, or whose driver does not answer within
/// ten seconds, is reported as unavailable.
pub fn probe_encoders(cache_dir: &Path) -> Vec<EncoderInfo> {
    let adapters = gpu::adapters();
    let first = adapters.first();
    let key = CacheEntry {
        gpu: first.map(|a| a.info.luid.to_string()).unwrap_or_default(),
        driver: first.map(|a| a.info.driver.clone()).unwrap_or_default(),
        ffmpeg: crate::ffmpeg_version(),
        encoders: Vec::new(),
    };
    let path = cache_dir.join(CACHE_FILE);
    let mut cache = read_cache(&path);
    if let Some(entry) = cache.iter().find(|e| same_machine(e, &key)) {
        return entry.encoders.clone();
    }
    if crate::init().is_err() {
        return Vec::new();
    }
    let encoders: Vec<EncoderInfo> = OUTPUT_CODECS
        .iter()
        .flat_map(|&codec| {
            CHAIN.iter().filter_map(move |&family| {
                encoder_name(family, codec).map(|name| (family, codec, name))
            })
        })
        .map(|(family, codec, name)| {
            let available = bounded_test_encode(family, codec, name, false);
            EncoderInfo {
                codec: codec_name(codec).to_owned(),
                name: name.to_owned(),
                hardware: family.hardware(),
                ten_bit: available && bounded_test_encode(family, codec, name, true),
                available,
            }
        })
        .collect();
    tracing::info!(
        available = ?encoders.iter().filter(|e| e.available).map(|e| e.name.as_str()).collect::<Vec<_>>(),
        "encoder probe"
    );
    cache.push(CacheEntry {
        encoders: encoders.clone(),
        ..key
    });
    let excess = cache.len().saturating_sub(MAX_CACHE_ENTRIES);
    cache.drain(..excess);
    if let Err(error) = write_cache(&path, cache) {
        tracing::warn!(%error, "could not write the encoder cache");
    }
    encoders
}

fn same_machine(a: &CacheEntry, b: &CacheEntry) -> bool {
    (&a.gpu, &a.driver, &a.ffmpeg) == (&b.gpu, &b.driver, &b.ffmpeg)
}

/// Longest time one test encode may take. A driver that never answers (a
/// hardware encoder transform waiting for an event that does not come)
/// would otherwise block the probe forever.
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// [`test_encode`] on its own thread, given up after [`PROBE_TIMEOUT`].
/// A thread that does not finish is left behind (blocked in the driver);
/// the encoder counts as unavailable.
fn bounded_test_encode(family: Family, codec: Codec, name: &'static str, ten_bit: bool) -> bool {
    let (done, result) = mpsc::sync_channel(1);
    let spawned = std::thread::Builder::new()
        .name(format!("probe {name}"))
        .spawn(move || {
            // The receiver is gone only after a timeout.
            let _ = done.send(test_encode(family, codec, name, ten_bit));
        });
    if spawned.is_err() {
        return false;
    }
    result.recv_timeout(PROBE_TIMEOUT).unwrap_or_else(|_| {
        tracing::warn!(
            encoder = name,
            ten_bit,
            "the encoder did not answer within {} s and counts as unavailable",
            PROBE_TIMEOUT.as_secs()
        );
        false
    })
}

/// Opens an encoder for a small grey test picture and encodes one frame.
fn test_encode(family: Family, codec: Codec, name: &'static str, ten_bit: bool) -> bool {
    let (layout, pixel) = if ten_bit {
        (PixelLayout::P010, ffmpeg_the_third::format::Pixel::P010LE)
    } else {
        (PixelLayout::Nv12, ffmpeg_the_third::format::Pixel::NV12)
    };
    let unknown = String::from("unknown");
    let input = EncoderInput {
        width: PROBE_SIZE.0,
        height: PROBE_SIZE.1,
        layout,
        time_base: Rational { num: 1, den: 60 },
        frame_rate: Rational { num: 60, den: 1 },
        sample_aspect: None,
        source_codec: codec_name(codec).to_owned(),
        color: crate::output_color(
            &ColorInfo {
                primaries: unknown.clone(),
                transfer: unknown.clone(),
                matrix: unknown,
                range: ColorRange::Unspecified,
            },
            PROBE_SIZE.1,
        ),
        bitrate: Some(2_000_000),
        keyframe_interval: 60,
    };
    let options = EncodeOptions {
        ten_bit: if ten_bit { Toggle::On } else { Toggle::Off },
        ..EncodeOptions::default()
    };
    let Ok(mut encoder) = try_open(&input, &options, family, codec, name) else {
        return false;
    };
    if encoder.input_layout() != layout {
        return false;
    }
    let mut picture = ffmpeg_the_third::frame::Video::new(pixel, PROBE_SIZE.0, PROBE_SIZE.1);
    // Mid-grey: 0x80 in every byte is grey for NV12 and close to grey for
    // P010 (0x8080).
    for plane in 0..picture.planes() {
        picture.data_mut(plane).fill(0x80);
    }
    let mut frame = VideoFrame::wrap(0, 0, layout, picture);
    let produced = encoder.send(&mut frame, true).is_ok()
        && encoder.flush().is_ok()
        && matches!(encoder.receive(), Ok(Some(_)));
    if !produced {
        tracing::debug!(encoder = name, ten_bit, "probe encode produced no packet");
    }
    produced
}

fn read_cache(path: &Path) -> Vec<CacheEntry> {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<CacheFile>(&bytes).ok())
        .filter(|file| file.schema == CACHE_SCHEMA)
        .map(|file| file.entries)
        .unwrap_or_default()
}

fn write_cache(path: &Path, entries: Vec<CacheEntry>) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let file = CacheFile {
        schema: CACHE_SCHEMA.to_owned(),
        entries,
    };
    let json = serde_json::to_vec_pretty(&file).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use std::ffi::CString;
    use std::ptr;

    use blurrd_core::{
        Codec, ColorInfo, ColorRange, EncoderInfo, EncoderSetting, GpuInfo, Luid, Rational,
    };
    use ffmpeg_the_third::ffi;

    use super::{
        CacheEntry, CacheFile, EncodeOptions, EncoderInput, Family, candidates, input_layout,
        open_first, output_codec, read_cache, same_machine, write_cache,
    };
    use crate::sys::gpu::{Adapter, VENDOR_AMD, VENDOR_NVIDIA};
    use crate::{PixelLayout, VideoError};

    fn input(codec: &str) -> EncoderInput {
        EncoderInput {
            width: 320,
            height: 240,
            layout: PixelLayout::I420,
            time_base: Rational { num: 1, den: 60 },
            frame_rate: Rational { num: 60, den: 1 },
            sample_aspect: None,
            source_codec: codec.to_owned(),
            color: ColorInfo {
                primaries: String::from("bt709"),
                transfer: String::from("bt709"),
                matrix: String::from("bt709"),
                range: ColorRange::Limited,
            },
            bitrate: None,
            keyframe_interval: 30,
        }
    }

    fn adapter(vendor_id: u32) -> Adapter {
        Adapter {
            info: GpuInfo {
                luid: Luid(1),
                name: String::from("gpu"),
                vram: 0,
                driver: String::from("1.0"),
            },
            vendor_id,
        }
    }

    fn names(list: &[(Family, Codec, &'static str)]) -> Vec<&'static str> {
        list.iter().map(|c| c.2).collect()
    }

    /// Whether `encoder` has the private option `key` in this FFmpeg build.
    fn has_option(encoder: &str, key: &str) -> bool {
        let codec = ffmpeg_the_third::encoder::find_by_name(encoder).unwrap();
        // SAFETY: `priv_class` of a static codec is null or a static class.
        let mut class = unsafe { (*codec.as_ptr()).priv_class };
        assert!(!class.is_null(), "{encoder} has no options");
        let key = CString::new(key).unwrap();
        // SAFETY: `av_opt_find` searches the static option table of the
        // class through a pointer to a pointer to it (a "fake object").
        let option = unsafe {
            ffi::av_opt_find(
                (&raw mut class).cast(),
                key.as_ptr(),
                ptr::null(),
                0,
                ffi::AV_OPT_SEARCH_FAKE_OBJ,
            )
        };
        !option.is_null()
    }

    #[test]
    fn the_chain_follows_the_gpu_vendors() {
        let options = EncodeOptions::default();
        let nvidia = candidates(&input("h264"), &options, &[adapter(VENDOR_NVIDIA)]).unwrap();
        assert_eq!(names(&nvidia), ["h264_nvenc", "h264_mf"]);
        let both = candidates(
            &input("hevc"),
            &options,
            &[adapter(VENDOR_AMD), adapter(VENDOR_NVIDIA)],
        )
        .unwrap();
        assert_eq!(names(&both), ["hevc_nvenc", "hevc_amf", "hevc_mf"]);
        let none = candidates(&input("vp9"), &options, &[]).unwrap();
        assert_eq!(names(&none), ["h264_mf"], "VP9 sources become H.264");
        let no_gpu = EncodeOptions {
            gpu: false,
            ..EncodeOptions::default()
        };
        let cpu = candidates(&input("av1"), &no_gpu, &[adapter(VENDOR_NVIDIA)]).unwrap();
        assert_eq!(names(&cpu), ["av1_mf"]);
    }

    #[test]
    fn the_probe_removes_unusable_encoders_unless_none_would_remain() {
        let info = |name: &str, available| EncoderInfo {
            codec: String::from("h264"),
            name: name.to_owned(),
            hardware: true,
            ten_bit: false,
            available,
        };
        let mut options = EncodeOptions {
            probed: vec![info("h264_nvenc", false), info("h264_mf", true)],
            ..EncodeOptions::default()
        };
        let gpus = [adapter(VENDOR_NVIDIA)];
        let chain = candidates(&input("h264"), &options, &gpus).unwrap();
        assert_eq!(names(&chain), ["h264_mf"]);
        options.probed = vec![info("h264_nvenc", false), info("h264_mf", false)];
        let chain = candidates(&input("h264"), &options, &gpus).unwrap();
        assert_eq!(names(&chain), ["h264_nvenc", "h264_mf"]);
    }

    #[test]
    fn named_encoders_are_checked() {
        let named = |name: &str, codec| EncodeOptions {
            encoder: EncoderSetting::Named(name.to_owned()),
            codec,
            ..EncodeOptions::default()
        };
        let gpus = [adapter(VENDOR_NVIDIA)];
        let one = candidates(&input("hevc"), &named("h264_mf", Codec::Source), &gpus).unwrap();
        assert_eq!(names(&one), ["h264_mf"]);
        assert_eq!(one[0].1, Codec::H264, "the encoder decides the codec");
        assert!(candidates(&input("h264"), &named("h264_mf", Codec::Hevc), &gpus).is_err());
        assert!(candidates(&input("h264"), &named("libx264", Codec::Source), &gpus).is_err());
        let test = candidates(&input("h264"), &named("ffv1", Codec::Source), &gpus).unwrap();
        assert_eq!(test[0].0, Family::Test);
    }

    #[test]
    fn source_codecs_map_to_output_codecs() {
        assert_eq!(output_codec(Codec::Source, "h264"), Codec::H264);
        assert_eq!(output_codec(Codec::Source, "hevc"), Codec::Hevc);
        assert_eq!(output_codec(Codec::Source, "av1"), Codec::Av1);
        assert_eq!(output_codec(Codec::Source, "mpeg4"), Codec::H264);
        assert_eq!(output_codec(Codec::Av1, "h264"), Codec::Av1);
    }

    #[test]
    fn input_layouts_follow_the_source_and_the_bit_depth() {
        use ffi::AVPixelFormat as F;
        let nvenc = [F::YUV420P, F::NV12, F::P010LE];
        let qsv = [F::NV12];
        assert_eq!(
            input_layout(Some(&nvenc), PixelLayout::I420, false),
            Some(PixelLayout::I420)
        );
        assert_eq!(
            input_layout(Some(&qsv), PixelLayout::I420, false),
            Some(PixelLayout::Nv12)
        );
        assert_eq!(
            input_layout(Some(&nvenc), PixelLayout::I420P10, true),
            Some(PixelLayout::P010)
        );
        assert_eq!(
            input_layout(Some(&nvenc), PixelLayout::P010, false),
            Some(PixelLayout::Nv12)
        );
        assert_eq!(input_layout(Some(&qsv), PixelLayout::P010, true), None);
    }

    #[test]
    fn family_options_exist_in_this_ffmpeg_build() {
        for (encoder, keys) in [
            (
                "h264_nvenc",
                &["preset", "tune", "rc", "cq", "forced-idr", "surfaces"][..],
            ),
            ("hevc_nvenc", &["split_encode_mode"][..]),
            ("av1_nvenc", &["split_encode_mode"][..]),
            (
                "h264_amf",
                &["usage", "quality", "rc", "qp_i", "qp_p", "forced_idr"][..],
            ),
            ("h264_qsv", &["preset", "forced_idr"][..]),
            ("h264_mf", &["rate_control", "quality"][..]),
        ] {
            for key in keys {
                assert!(has_option(encoder, key), "{encoder} has no option {key}");
            }
        }
    }

    #[test]
    fn a_failing_encoder_falls_back_and_is_reported() {
        let options = EncodeOptions::default();
        // VA-API does not exist on Windows builds, so it always fails.
        let chain = vec![
            (Family::Nvenc, Codec::H264, "h264_vaapi"),
            (Family::Test, Codec::Source, "mpeg4"),
        ];
        let (encoder, fallbacks) = open_first(&input("h264"), &options, chain).unwrap();
        assert_eq!(encoder.name(), "mpeg4");
        assert_eq!(fallbacks.len(), 1);
        assert_eq!(
            (fallbacks[0].from.as_str(), fallbacks[0].to.as_str()),
            ("h264_vaapi", "mpeg4")
        );
        let error = open_first(
            &input("h264"),
            &options,
            vec![(Family::Nvenc, Codec::H264, "h264_vaapi")],
        )
        .unwrap_err();
        assert!(
            matches!(&error, VideoError::EncoderUnavailable { tried, driver_too_old: false }
                if tried.len() == 1),
            "{error}"
        );
        assert_eq!(error.kind(), blurrd_core::ErrorKind::EncoderUnavailable);
    }

    #[test]
    fn the_cache_is_keyed_by_machine() {
        let dir = std::env::temp_dir().join(format!("blurrd-encoders-{}", std::process::id()));
        let path = dir.join("encoders.json");
        let entry = CacheEntry {
            gpu: String::from("0000000000000001"),
            driver: String::from("32.0.15.7242"),
            ffmpeg: String::from("9.0.2"),
            encoders: Vec::new(),
        };
        write_cache(&path, vec![entry.clone()]).unwrap();
        assert_eq!(read_cache(&path), vec![entry.clone()]);
        let mut other = entry.clone();
        other.driver = String::from("other");
        assert!(!same_machine(&entry, &other));
        // A file with another schema is ignored.
        let foreign = CacheFile {
            schema: String::from("blurrd-encoders/2"),
            entries: vec![entry],
        };
        std::fs::write(&path, serde_json::to_vec(&foreign).unwrap()).unwrap();
        assert_eq!(read_cache(&path), Vec::new());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
