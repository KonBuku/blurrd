//! Colour tags of a video stream and how blurrd interprets them.
//!
//! Recordings are often tagged incompletely: `unknown` or `reserved`
//! matrices are common. [`VideoColour::from_info`] maps every FFmpeg tag
//! value to a matrix, range and transfer function, falling back to BT.709
//! for HD and BT.601 for SD sizes (never copying an unknown tag through);
//! the fallback is reported in [`VideoColour::assumed`].

use std::ffi::{CStr, CString, c_char, c_int};

use blurrd_blur::{SampleRange, YuvColour, YuvMatrix};
use blurrd_core::{ColorInfo, ColorRange, HdrKind};
use ffmpeg_the_third::ffi;

/// Pictures taller than this are HD and default to BT.709.
const SD_MAX_HEIGHT: u32 = 576;

/// Transfer function of the samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ColourTransfer {
    /// Standard dynamic range (BT.709 / BT.1886 / sRGB-like gamma).
    Sdr,
    /// HDR perceptual quantizer (SMPTE ST 2084).
    Pq,
    /// HDR hybrid log-gamma (ARIB STD-B67).
    Hlg,
}

/// How blurrd interprets a video's colour tags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VideoColour {
    /// YUV matrix coefficients.
    pub matrix: YuvMatrix,
    /// Code value range.
    pub range: SampleRange,
    /// Transfer function.
    pub transfer: ColourTransfer,
    /// Whether the primaries are BT.2020 (wide gamut).
    pub wide_gamut: bool,
    /// Whether the matrix was not tagged usably and was assumed from the
    /// picture height.
    pub assumed: bool,
}

impl VideoColour {
    /// Interprets the tags of a stream with picture height `height`.
    pub fn from_info(info: &ColorInfo, height: u32) -> Self {
        let default_matrix = if height > SD_MAX_HEIGHT {
            YuvMatrix::Bt709
        } else {
            YuvMatrix::Bt601
        };
        let (matrix, assumed) = match info.matrix.as_str() {
            // SMPTE 240M is within rounding of BT.709 at 8 bits.
            "bt709" | "smpte240m" => (YuvMatrix::Bt709, false),
            // FCC is within rounding of BT.601.
            "bt470bg" | "smpte170m" | "fcc" => (YuvMatrix::Bt601, false),
            "bt2020nc" | "bt2020c" => (YuvMatrix::Bt2020, false),
            // `unknown`, `reserved` and matrices blurrd does not decode
            // (RGB, YCgCo, ICtCp, chroma-derived): assume from the size.
            _ => (default_matrix, true),
        };
        let range = match info.range {
            ColorRange::Full => SampleRange::Full,
            _ => SampleRange::Limited,
        };
        let transfer = match hdr_kind(&info.transfer) {
            Some(HdrKind::Pq) => ColourTransfer::Pq,
            Some(HdrKind::Hlg) => ColourTransfer::Hlg,
            _ => ColourTransfer::Sdr,
        };
        Self {
            matrix,
            range,
            transfer,
            wide_gamut: info.primaries == "bt2020",
            assumed,
        }
    }

    /// The encoding the blur needs for fixed fill colours.
    pub fn yuv_colour(&self) -> YuvColour {
        YuvColour {
            matrix: self.matrix,
            range: self.range,
        }
    }
}

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

/// Tag values that mean "not tagged".
fn untagged(name: &str) -> bool {
    matches!(name, "unknown" | "reserved" | "unspecified")
}

/// The colour tags blurrd writes into an output whose source has the tags
/// `info` and the picture height `height`.
///
/// Tags the source states are copied. Untagged (`unknown`, `reserved`)
/// primaries and transfer functions become BT.709 above 576 lines and
/// SMPTE 170M below; a matrix blurrd had to assume (see
/// [`VideoColour::from_info`]) is written as the matrix it assumed; an
/// unspecified range becomes limited. An unknown tag is never copied
/// through, so players do not have to guess either.
pub fn output_color(info: &ColorInfo, height: u32) -> ColorInfo {
    let default_name = if height > SD_MAX_HEIGHT {
        "bt709"
    } else {
        "smpte170m"
    };
    let or_default = |name: &str| {
        if untagged(name) {
            default_name.to_owned()
        } else {
            name.to_owned()
        }
    };
    let colour = VideoColour::from_info(info, height);
    let matrix = if colour.assumed {
        match colour.matrix {
            YuvMatrix::Bt601 => String::from("smpte170m"),
            YuvMatrix::Bt2020 => String::from("bt2020nc"),
            _ => String::from("bt709"),
        }
    } else {
        info.matrix.clone()
    };
    ColorInfo {
        primaries: or_default(&info.primaries),
        transfer: or_default(&info.transfer),
        matrix,
        range: match info.range {
            ColorRange::Full => ColorRange::Full,
            _ => ColorRange::Limited,
        },
    }
}

/// Writes colour tags (FFmpeg names, as produced by [`output_color`]) into
/// an encoder context that is not open yet. Names FFmpeg does not know are
/// left unset.
///
/// # Safety
///
/// `ctx` must point to a valid encoder context.
pub(crate) unsafe fn apply_color_tags(ctx: *mut ffi::AVCodecContext, info: &ColorInfo) {
    let lookup = |name: &str, find: unsafe extern "C" fn(*const c_char) -> c_int| {
        let name = CString::new(name).ok()?;
        // SAFETY: the `av_color_*_from_name` functions read a NUL-terminated
        // string and return a negative value for unknown names.
        let value = unsafe { find(name.as_ptr()) };
        (value >= 0).then_some(value)
    };
    // SAFETY: the caller guarantees a valid context; only plain enum
    // fields are written, with values FFmpeg itself returned.
    unsafe {
        if let Some(v) = lookup(&info.primaries, ffi::av_color_primaries_from_name) {
            (*ctx).color_primaries = ffi::AVColorPrimaries(v);
        }
        if let Some(v) = lookup(&info.transfer, ffi::av_color_transfer_from_name) {
            (*ctx).color_trc = ffi::AVColorTransferCharacteristic(v);
        }
        if let Some(v) = lookup(&info.matrix, ffi::av_color_space_from_name) {
            (*ctx).colorspace = ffi::AVColorSpace(v);
        }
        (*ctx).color_range = match info.range {
            ColorRange::Full => ffi::AVColorRange::JPEG,
            _ => ffi::AVColorRange::MPEG,
        };
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
    use std::ffi::c_int;

    use blurrd_blur::{SampleRange, YuvMatrix};
    use blurrd_core::{ColorInfo, ColorRange, HdrKind};
    use ffmpeg_the_third::ffi;

    use super::{ColourTransfer, VideoColour, hdr_kind, output_color, tag_name};

    fn info(matrix: &str, transfer: &str, range: ColorRange) -> ColorInfo {
        ColorInfo {
            primaries: String::from("bt709"),
            transfer: transfer.to_owned(),
            matrix: matrix.to_owned(),
            range,
        }
    }

    /// Expected matrix for every FFmpeg matrix tag at HD and SD sizes,
    /// and whether it was assumed.
    fn expected(name: &str) -> Option<(YuvMatrix, YuvMatrix, bool)> {
        use YuvMatrix::{Bt601, Bt709, Bt2020};
        Some(match name {
            "bt709" | "smpte240m" => (Bt709, Bt709, false),
            "bt470bg" | "smpte170m" | "fcc" => (Bt601, Bt601, false),
            "bt2020nc" | "bt2020c" => (Bt2020, Bt2020, false),
            "gbr" | "unknown" | "reserved" | "ycgco" | "ycgco-re" | "ycgco-ro" | "smpte2085"
            | "chroma-derived-nc" | "chroma-derived-c" | "ictcp" | "ipt-c2" => (Bt709, Bt601, true),
            _ => return None,
        })
    }

    #[test]
    fn every_matrix_tag_has_a_mapping() {
        let mut seen = 0;
        for value in 0..ffi::AVColorSpace::NB.0 {
            // SAFETY: the name lookup accepts any value.
            let name =
                tag_name(unsafe { ffi::av_color_space_name(ffi::AVColorSpace(value as c_int)) });
            let Some((hd, sd, assumed)) = expected(&name) else {
                panic!("matrix tag {value} ({name}) has no expected mapping; add it");
            };
            let colour_1080 =
                VideoColour::from_info(&info(&name, "bt709", ColorRange::Limited), 1080);
            let colour_480 =
                VideoColour::from_info(&info(&name, "bt709", ColorRange::Limited), 480);
            assert_eq!(
                (colour_1080.matrix, colour_1080.assumed),
                (hd, assumed),
                "{name} at 1080p"
            );
            assert_eq!(
                (colour_480.matrix, colour_480.assumed),
                (sd, assumed),
                "{name} at 480p"
            );
            seen += 1;
        }
        assert!(seen >= 15, "only {seen} matrix tags");
        // Out-of-range values from broken files fall back like `unknown`.
        let broken =
            VideoColour::from_info(&info("unknown", "unknown", ColorRange::Unspecified), 720);
        assert_eq!(
            (broken.matrix, broken.range, broken.assumed),
            (YuvMatrix::Bt709, SampleRange::Limited, true)
        );
    }

    #[test]
    fn ranges_and_transfers_are_mapped() {
        let full = VideoColour::from_info(&info("bt709", "bt709", ColorRange::Full), 1080);
        assert_eq!(full.range, SampleRange::Full);
        let pq = VideoColour::from_info(&info("bt2020nc", "smpte2084", ColorRange::Limited), 2160);
        assert_eq!(
            (pq.transfer, pq.matrix),
            (ColourTransfer::Pq, YuvMatrix::Bt2020)
        );
        let hlg =
            VideoColour::from_info(&info("bt2020nc", "arib-std-b67", ColorRange::Limited), 2160);
        assert_eq!(hlg.transfer, ColourTransfer::Hlg);
        assert_eq!(full.yuv_colour().range, SampleRange::Full);
    }

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

    fn tags(primaries: &str, transfer: &str, matrix: &str, range: ColorRange) -> ColorInfo {
        ColorInfo {
            primaries: primaries.to_owned(),
            transfer: transfer.to_owned(),
            matrix: matrix.to_owned(),
            range,
        }
    }

    #[test]
    fn output_tags_never_stay_unknown() {
        // A typical game recording: only the matrix is tagged.
        let recorder = tags("unknown", "unknown", "bt709", ColorRange::Limited);
        assert_eq!(
            output_color(&recorder, 1080),
            tags("bt709", "bt709", "bt709", ColorRange::Limited)
        );
        let untagged = tags("reserved", "unknown", "unknown", ColorRange::Unspecified);
        assert_eq!(
            output_color(&untagged, 1080),
            tags("bt709", "bt709", "bt709", ColorRange::Limited)
        );
        assert_eq!(
            output_color(&untagged, 480),
            tags("smpte170m", "smpte170m", "smpte170m", ColorRange::Limited)
        );
        // Stated tags are copied, including HDR and full range.
        let hdr = tags("bt2020", "smpte2084", "bt2020nc", ColorRange::Full);
        assert_eq!(output_color(&hdr, 2160), hdr);
        let pal = tags("bt470bg", "gamma28", "bt470bg", ColorRange::Limited);
        assert_eq!(output_color(&pal, 576), pal);
    }
}
