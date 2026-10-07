//! Colour tags of a video stream.

use std::ffi::{CStr, c_char};

use blurrd_core::{ColorInfo, ColorRange, HdrKind};
use ffmpeg_the_third::ffi;

/// FFmpeg's name of a tag value, or `unknown` when FFmpeg has none (an
/// out-of-range value read from a broken file).
fn tag_name(name: *const c_char) -> String {
    if name.is_null() {
        return String::from("unknown");
    }
    // SAFETY: the `av_color_*_name` functions return null or a pointer to a
    // static NUL-terminated string.
    unsafe { CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned()
}

/// The colour tags of a stream's codec parameters, with FFmpeg's names.
pub(crate) fn color_info(par: &ffi::AVCodecParameters) -> ColorInfo {
    // SAFETY: the name functions accept any enum value and return null for
    // values without a name.
    let (primaries, transfer, matrix) = unsafe {
        (
            tag_name(ffi::av_color_primaries_name(par.color_primaries)),
            tag_name(ffi::av_color_transfer_name(par.color_trc)),
            tag_name(ffi::av_color_space_name(par.color_space)),
        )
    };
    let range = match par.color_range {
        ffi::AVColorRange::MPEG => ColorRange::Limited,
        ffi::AVColorRange::JPEG => ColorRange::Full,
        _ => ColorRange::Unspecified,
    };
    ColorInfo {
        primaries,
        transfer,
        matrix,
        range,
    }
}

/// The HDR transfer function named by a transfer tag, if any.
pub(crate) fn hdr_kind(transfer: &str) -> Option<HdrKind> {
    match transfer {
        "smpte2084" => Some(HdrKind::Pq),
        "arib-std-b67" => Some(HdrKind::Hlg),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use blurrd_core::HdrKind;

    use super::hdr_kind;

    #[test]
    fn hdr_transfers_are_recognised() {
        assert_eq!(hdr_kind("smpte2084"), Some(HdrKind::Pq));
        assert_eq!(hdr_kind("arib-std-b67"), Some(HdrKind::Hlg));
        for sdr in [
            "bt709",
            "unknown",
            "reserved",
            "iec61966-2-1",
            "smpte170m",
            "bt2020-10",
        ] {
            assert_eq!(hdr_kind(sdr), None, "{sdr}");
        }
    }
}
