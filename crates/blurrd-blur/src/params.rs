//! Blur parameters resolved to source pixels and code values.

use blurrd_core::{BlurParams, BlurStyle, SolidColor};

/// Gaussian sigma of [`BlurStyle::PixelBlur`], as a fraction of the block
/// size. 0.35 gives 2.8 / 3.5 / 4.9 px at the 8 / 10 / 14 px strengths,
/// in line with the prototype's sigma of 3.0 px.
pub const PIXEL_BLUR_SIGMA_PER_BLOCK: f32 = 0.35;

/// Gaussian sigma of [`BlurStyle::Gaussian`], as a fraction of the block
/// size. Wide enough that no block structure remains visible.
pub const GAUSSIAN_SIGMA_PER_BLOCK: f32 = 0.6;

/// Amplitude of the optional noise in 8-bit code values (scaled for
/// 10-bit). The noise offsets each block's mean by up to this amount, which
/// defeats de-pixelation attacks that match block colours against rendered
/// candidate names.
pub const NOISE_AMPLITUDE: f32 = 2.0;

/// Smallest supported HUD scale (`sy`): 270 rows.
pub const MIN_SCALE: f32 = 0.25;

/// Largest supported HUD scale (`sy`): 8640 rows.
pub const MAX_SCALE: f32 = 8.0;

/// Why [`PixelParams::new`] rejected its input.
#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum ParamsError {
    /// The HUD scale is not a finite number in [`MIN_SCALE`]`..=`[`MAX_SCALE`].
    #[error("HUD scale {0} is outside {MIN_SCALE}..={MAX_SCALE}")]
    Scale(f32),
    /// The feather is negative or not finite.
    #[error("feather {0} must be a finite, non-negative number")]
    Feather(f32),
}

/// YUV matrix coefficients, used to convert a fixed sRGB fill colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum YuvMatrix {
    /// ITU-R BT.601 (SD video).
    Bt601,
    /// ITU-R BT.709 (HD video, the default).
    #[default]
    Bt709,
    /// ITU-R BT.2020 non-constant luminance (UHD and HDR video).
    Bt2020,
}

impl YuvMatrix {
    /// The luma weights of red and blue, `(Kr, Kb)`.
    const fn weights(self) -> (f32, f32) {
        match self {
            Self::Bt601 => (0.299, 0.114),
            Self::Bt709 => (0.2126, 0.0722),
            Self::Bt2020 => (0.2627, 0.0593),
        }
    }
}

/// Range of the code values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum SampleRange {
    /// Limited (TV) range: 16–235 luma, 16–240 chroma at 8 bits.
    #[default]
    Limited,
    /// Full (PC) range: 0–255 at 8 bits.
    Full,
}

/// How the video encodes colour, needed only for fixed fill colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct YuvColour {
    /// Matrix coefficients.
    pub matrix: YuvMatrix,
    /// Code value range.
    pub range: SampleRange,
}

impl YuvColour {
    /// Converts a gamma-encoded sRGB colour to normalised YUV:
    /// `Y` in `0..=1`, `U` and `V` in `-0.5..=0.5`.
    fn normalised(self, [r, g, b]: [u8; 3]) -> [f32; 3] {
        let (kr, kb) = self.matrix.weights();
        let (r, g, b) = (
            f32::from(r) / 255.0,
            f32::from(g) / 255.0,
            f32::from(b) / 255.0,
        );
        let y = kr * r + (1.0 - kr - kb) * g + kb * b;
        [
            y,
            (b - y) / (2.0 * (1.0 - kb)),
            (r - y) / (2.0 * (1.0 - kr)),
        ]
    }

    /// Converts a gamma-encoded sRGB colour to code values for samples
    /// with `bits` bits (8 or 10).
    pub(crate) fn code_values(self, rgb: [u8; 3], bits: u32) -> [f32; 3] {
        let [y, u, v] = self.normalised(rgb);
        let scale = (1u32 << (bits - 8)) as f32;
        let max = ((1u32 << bits) - 1) as f32;
        let mid = (1u32 << (bits - 1)) as f32;
        match self.range {
            SampleRange::Limited => [
                16.0f32.mul_add(scale, 219.0 * scale * y),
                224.0f32.mul_add(scale * u, mid),
                224.0f32.mul_add(scale * v, mid),
            ],
            SampleRange::Full => [
                max * y,
                (max * u + mid).clamp(0.0, max),
                (max * v + mid).clamp(0.0, max),
            ],
        }
    }
}

/// What a blurred box is filled with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Fill {
    /// Area downsample, bilinear upsample, separable gaussian with this
    /// sigma (in luma pixels).
    Smooth {
        /// Gaussian sigma in luma pixels.
        sigma: f32,
    },
    /// Area downsample, nearest-neighbour upsample (hard blocks).
    Blocks,
    /// The mean of the padded box.
    BoxMean,
    /// A fixed sRGB colour, converted with the video's colour encoding.
    Colour(YuvColour, [u8; 3]),
}

/// [`BlurParams`] resolved for one video: sizes in source pixels.
///
/// Built once per job from the project's parameters, the HUD scale `sy`
/// (`HudGeometry::sy`, 1.0 at 1080p) and the video's colour encoding.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PixelParams {
    pub(crate) fill: Fill,
    pub(crate) block: f32,
    pub(crate) pad: (i32, i32),
    pub(crate) radius: f32,
    pub(crate) feather: f32,
    pub(crate) noise: bool,
}

impl PixelParams {
    /// Resolves `params` for a video with HUD scale `sy`.
    ///
    /// The block size is the strength's block (never below
    /// [`blurrd_core::MIN_BLOCK_PX`]) times `sy`. Padding is rounded up to
    /// whole pixels. The feather is limited to the padding plus half a pixel
    /// and the corner radius to the value at which the rounded corners still
    /// cover the corner pixels of the unpadded box, so the soft edge always
    /// lies in the padding.
    ///
    /// # Errors
    ///
    /// [`ParamsError::Scale`] if `sy` is not in
    /// [`MIN_SCALE`]`..=`[`MAX_SCALE`]; [`ParamsError::Feather`] if the
    /// feather is negative or not finite.
    pub fn new(params: &BlurParams, sy: f32, colour: YuvColour) -> Result<Self, ParamsError> {
        if !(MIN_SCALE..=MAX_SCALE).contains(&sy) {
            return Err(ParamsError::Scale(sy));
        }
        if !params.is_valid() {
            return Err(ParamsError::Feather(params.feather));
        }
        let block = f32::from(params.strength.block_px()) * sy;
        let pad = (
            (f32::from(params.pad.0) * sy).ceil() as i32,
            (f32::from(params.pad.1) * sy).ceil() as i32,
        );
        // Distance from the edge of the padded shape to the centre of the
        // outermost unpadded pixel.
        let inset = pad.0.min(pad.1) as f32 + 0.5;
        let feather = (params.feather * sy).min(inset);
        // The corner pixel centre of the unpadded box lies `inset` inside
        // both edges of the shape. It is at least `feather` inside the
        // rounded corner while r <= (sqrt(2) * inset - feather) / (sqrt(2) - 1).
        let max_radius =
            (std::f32::consts::SQRT_2.mul_add(inset, -feather)) / (std::f32::consts::SQRT_2 - 1.0);
        let radius = (f32::from(params.radius) * sy).min(max_radius).max(0.0);
        let fill = match params.style {
            BlurStyle::Gaussian => Fill::Smooth {
                sigma: GAUSSIAN_SIGMA_PER_BLOCK * block,
            },
            BlurStyle::Pixelate => Fill::Blocks,
            BlurStyle::Solid {
                color: SolidColor::Rgb(rgb),
            } => Fill::Colour(colour, rgb),
            BlurStyle::Solid { .. } => Fill::BoxMean,
            // `PixelBlur` and styles added later.
            _ => Fill::Smooth {
                sigma: PIXEL_BLUR_SIGMA_PER_BLOCK * block,
            },
        };
        Ok(Self {
            fill,
            block,
            pad,
            radius,
            feather,
            noise: params.noise,
        })
    }

    /// Block size in luma pixels (before a box's block scale).
    pub fn block(&self) -> f32 {
        self.block
    }

    /// Padding around each box in luma pixels, `(x, y)`.
    pub fn pad(&self) -> (i32, i32) {
        self.pad
    }

    /// Width of the soft edge in luma pixels, after limiting it to the
    /// padding.
    pub fn feather(&self) -> f32 {
        self.feather
    }

    /// Corner radius in luma pixels, after limiting it (see
    /// [`PixelParams::new`]).
    pub fn radius(&self) -> f32 {
        self.radius
    }
}

#[cfg(test)]
mod tests {
    use blurrd_core::{BlurParams, BlurStyle, SolidColor, Strength};

    use super::{Fill, ParamsError, PixelParams, SampleRange, YuvColour, YuvMatrix};

    #[test]
    fn scales_sizes_with_the_hud() {
        let params = BlurParams::default();
        let p = PixelParams::new(&params, 2.0, YuvColour::default()).unwrap();
        assert!((p.block() - 20.0).abs() < 1e-6);
        assert_eq!(p.pad(), (8, 6));
        assert!((p.feather() - 3.0).abs() < 1e-6);
        assert!((p.radius() - 12.0).abs() < 1e-6);
        let small = PixelParams::new(&params, 2.0 / 3.0, YuvColour::default()).unwrap();
        assert_eq!(small.pad(), (3, 2), "padding rounds up");
    }

    #[test]
    fn enforces_the_block_floor() {
        let params = BlurParams {
            strength: Strength::Custom(2),
            ..BlurParams::default()
        };
        let p = PixelParams::new(&params, 1.0, YuvColour::default()).unwrap();
        assert!((p.block() - 8.0).abs() < 1e-6);
    }

    #[test]
    fn keeps_the_soft_edge_in_the_padding() {
        let params = BlurParams {
            pad: (0, 0),
            feather: 4.0,
            radius: 20,
            ..BlurParams::default()
        };
        let p = PixelParams::new(&params, 1.0, YuvColour::default()).unwrap();
        assert!((p.feather() - 0.5).abs() < 1e-6);
        assert!(p.radius() <= 0.5 + 1e-6);
    }

    #[test]
    fn rejects_invalid_input() {
        let params = BlurParams::default();
        assert_eq!(
            PixelParams::new(&params, 0.1, YuvColour::default()),
            Err(ParamsError::Scale(0.1))
        );
        assert!(PixelParams::new(&params, f32::NAN, YuvColour::default()).is_err());
        let bad = BlurParams {
            feather: f32::INFINITY,
            ..params
        };
        assert!(matches!(
            PixelParams::new(&bad, 1.0, YuvColour::default()),
            Err(ParamsError::Feather(_))
        ));
    }

    #[test]
    fn maps_styles_to_fills() {
        let solid = BlurParams {
            style: BlurStyle::Solid {
                color: SolidColor::BoxMean,
            },
            ..BlurParams::default()
        };
        let p = PixelParams::new(&solid, 1.0, YuvColour::default()).unwrap();
        assert_eq!(p.fill, Fill::BoxMean);
        let pixel = PixelParams::new(&BlurParams::default(), 1.0, YuvColour::default()).unwrap();
        assert_eq!(pixel.fill, Fill::Smooth { sigma: 3.5 });
    }

    #[test]
    fn converts_fill_colours() {
        let limited = YuvColour::default();
        let white = limited.code_values([255, 255, 255], 8);
        assert!((white[0] - 235.0).abs() < 1e-3 && (white[1] - 128.0).abs() < 1e-3);
        let black10 = limited.code_values([0, 0, 0], 10);
        assert!((black10[0] - 64.0).abs() < 1e-3 && (black10[2] - 512.0).abs() < 1e-3);
        let full = YuvColour {
            matrix: YuvMatrix::Bt601,
            range: SampleRange::Full,
        };
        let red = full.code_values([255, 0, 0], 8);
        assert!((red[0] - 0.299 * 255.0).abs() < 1e-3);
        assert!(
            (red[2] - 255.0).abs() < 1e-3,
            "V saturates at the top of the range"
        );
    }
}
