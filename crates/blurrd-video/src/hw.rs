//! Hardware decoding through FFmpeg's hwaccel API (NVDEC, D3D11VA).
//!
//! The decoder gets a device context; FFmpeg's default `get_format` then
//! picks the hardware surface format when the stream is supported and falls
//! back to software decoding otherwise (unsupported profile or size).
//! Decoded surfaces are downloaded into system memory as NV12 (8-bit) or
//! P010 (10-bit), so callers always see [`crate::PixelLayout`] frames.

use std::ptr;

use ffmpeg_the_third::{ffi, frame};

use crate::VideoError;

/// Hardware decoding back ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Backend {
    /// NVIDIA NVDEC through CUDA.
    Nvdec,
    /// Direct3D 11 video decoding (any vendor).
    D3d11va,
}

impl Backend {
    fn device_type(self) -> ffi::AVHWDeviceType {
        match self {
            Self::Nvdec => ffi::AVHWDeviceType::CUDA,
            Self::D3d11va => ffi::AVHWDeviceType::D3D11VA,
        }
    }

    /// Name used in logs and the decoder cache.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Nvdec => "nvdec",
            Self::D3d11va => "d3d11va",
        }
    }
}

/// The surface format a codec produces with `backend`, if the codec
/// supports it through a device context.
pub(crate) fn surface_format(
    codec: *const ffi::AVCodec,
    backend: Backend,
) -> Option<ffi::AVPixelFormat> {
    let wanted = backend.device_type();
    for index in 0.. {
        // SAFETY: `codec` is a valid codec descriptor from FFmpeg's static
        // table; `avcodec_get_hw_config` returns null past the last entry.
        let config = unsafe { ffi::avcodec_get_hw_config(codec, index) };
        if config.is_null() {
            return None;
        }
        // SAFETY: non-null configs point to static data.
        let config = unsafe { &*config };
        if config.device_type == wanted
            && config.methods & ffi::AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX.0 != 0
        {
            return Some(config.pix_fmt);
        }
    }
    None
}

/// Creates a device of `backend` on the default adapter and attaches it to
/// a decoder context that is not open yet.
///
/// # Safety
///
/// `ctx` must be a valid, unopened decoder context.
pub(crate) unsafe fn attach_device(
    ctx: *mut ffi::AVCodecContext,
    backend: Backend,
    extra_frames: u32,
) -> Result<(), VideoError> {
    let mut device: *mut ffi::AVBufferRef = ptr::null_mut();
    // SAFETY: `device` receives a new reference on success; null device
    // name and options select the default adapter.
    let ret = unsafe {
        ffi::av_hwdevice_ctx_create(
            &raw mut device,
            backend.device_type(),
            ptr::null(),
            ptr::null_mut(),
            0,
        )
    };
    if ret < 0 {
        return Err(VideoError::ffmpeg("creating the hardware decoding device")(
            ffmpeg_the_third::Error::from(ret),
        ));
    }
    // SAFETY: the context is valid and unopened (caller contract); it takes
    // over our reference to the device, which it unrefs when freed.
    unsafe {
        (*ctx).hw_device_ctx = device;
        (*ctx).extra_hw_frames = i32::try_from(extra_frames).unwrap_or(i32::MAX);
    }
    Ok(())
}

/// Downloads a hardware surface into a new system-memory frame with the
/// same properties (PTS, flags, colour tags). Software frames are returned
/// unchanged. The flag says whether a download happened.
pub(crate) fn download(
    decoded: frame::Video,
    surface: Option<ffi::AVPixelFormat>,
) -> Result<(frame::Video, bool), VideoError> {
    // SAFETY: reading the format field of a valid frame.
    let format = unsafe { (*decoded.as_ptr()).format };
    if surface.is_none_or(|s| s.0 != format) {
        return Ok((decoded, false));
    }
    let mut sw = frame::Video::empty();
    // SAFETY: `sw` is an empty frame that `av_hwframe_transfer_data`
    // allocates in the surface's default download format; both frames are
    // valid and owned here. Properties are copied afterwards because the
    // transfer only copies pixels.
    unsafe {
        let ret = ffi::av_hwframe_transfer_data(sw.as_mut_ptr(), decoded.as_ptr(), 0);
        if ret < 0 {
            return Err(VideoError::ffmpeg("downloading a decoded frame")(
                ffmpeg_the_third::Error::from(ret),
            ));
        }
        let ret = ffi::av_frame_copy_props(sw.as_mut_ptr(), decoded.as_ptr());
        if ret < 0 {
            return Err(VideoError::ffmpeg("copying frame properties")(
                ffmpeg_the_third::Error::from(ret),
            ));
        }
    }
    Ok((sw, true))
}
