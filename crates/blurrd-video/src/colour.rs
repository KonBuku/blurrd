//! Colour tags of a video stream and how blurrd interprets them.
//!
//! Recordings are often tagged incompletely: `unknown` or `reserved`
//! matrices are common. [`VideoColour::from_info`] maps every FFmpeg tag
//! value to a matrix, range and transfer function, falling back to BT.709
//! for HD and BT.601 for SD sizes (never copying an unknown tag through);
//! the fallback is reported in [`VideoColour::assumed`].

use std::ffi::{CStr, c_char};

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

    use super::{ColourTransfer, VideoColour, hdr_kind, tag_name};

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
}
