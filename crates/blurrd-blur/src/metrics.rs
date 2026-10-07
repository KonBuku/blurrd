//! Quality metrics of the blur.
//!
//! The high-frequency residual measures how much fine detail (the strokes
//! of a name) survives inside a blurred box. It is one of the release gates
//! of the architecture plan (section 11.3): every ground-truth box must stay
//! at or below [`HF_RESIDUAL_MAX`].

use blurrd_core::RectI;

use crate::frame::{Plane, SampleFormat, sample};

/// Largest allowed high-frequency residual of a box blurred with a smooth
/// style ([`blurrd_core::BlurStyle::PixelBlur`] or
/// [`blurrd_core::BlurStyle::Gaussian`]).
///
/// Calibrated once in milestone M5 and frozen. On 2,280 kill-feed name boxes
/// with detail (see [`MIN_DETAIL_VARIANCE`]) from 525 frames of 21 real
/// 1080p recordings, the residual at every strength, with and without
/// noise, was at most 0.0046 (99th percentile 0.0017, median 0.0003). That
/// calibration covered name boxes only; the M5 review measured at most
/// 0.006 on about 6,900 kill-feed and random 160×30 boxes of 16 real 1080p
/// frames (every strength, with and without noise, 8 and 10 bit). The gate
/// keeps a margin of about 1.7× above everything measured so far and stays
/// well below the 5 % ceiling of the plan. What remains is mostly the
/// rounding of the output to whole code values.
///
/// [`blurrd_core::BlurStyle::Pixelate`] is not gated: its hard block edges
/// are high-frequency by design (residuals up to 0.8 on the same boxes)
/// without carrying any detail of the text.
pub const HF_RESIDUAL_MAX: f64 = 0.01;

/// Smallest Laplacian variance of the source (in 8-bit code values
/// squared) for which a box counts as containing a readable name. Below it,
/// the rounding of the output alone (a variance of about 1.7) would dominate
/// the ratio. On the calibration boxes, 95 % of the names were above 1,500.
pub const MIN_DETAIL_VARIANCE: f64 = 400.0;

/// Variance of the 4-neighbour Laplacian over the interior of `rect`
/// (the outermost pixel ring is skipped so that every neighbour lies inside
/// `rect`).
///
/// Values are in 8-bit code values squared; 16-bit samples are scaled down
/// accordingly. Returns `None` if `rect` is smaller than 3×3 or not inside
/// the plane.
pub fn laplacian_variance(plane: Plane<'_>, format: SampleFormat, rect: RectI) -> Option<f64> {
    let inside = rect.x0 >= 0
        && rect.y0 >= 0
        && i64::from(rect.x1) <= i64::from(plane.width)
        && i64::from(rect.y1) <= i64::from(plane.height);
    if !inside || rect.width() < 3 || rect.height() < 3 {
        return None;
    }
    let scale = match format {
        SampleFormat::U8 => 1.0,
        SampleFormat::U16Le => 1.0 / 256.0,
    };
    let at = |x: i32, y: i32| sample(&plane, format, x as usize, y as usize) * scale;
    let (mut sum, mut sum_sq, mut n) = (0.0f64, 0.0f64, 0.0f64);
    for y in rect.y0 + 1..rect.y1 - 1 {
        for x in rect.x0 + 1..rect.x1 - 1 {
            let lap = at(x - 1, y) + at(x + 1, y) + at(x, y - 1) + at(x, y + 1) - 4.0 * at(x, y);
            sum += lap;
            sum_sq += lap * lap;
            n += 1.0;
        }
    }
    let mean = sum / n;
    Some((sum_sq / n - mean * mean).max(0.0))
}

/// High-frequency residual of a blurred box: the Laplacian variance of the
/// output divided by that of the source, over `rect`.
///
/// Returns `None` if the source has less detail than
/// [`MIN_DETAIL_VARIANCE`] (nothing to hide) or `rect` is not measurable
/// (see [`laplacian_variance`]).
pub fn hf_residual(
    source: Plane<'_>,
    output: Plane<'_>,
    format: SampleFormat,
    rect: RectI,
) -> Option<f64> {
    let before = laplacian_variance(source, format, rect)?;
    if before < MIN_DETAIL_VARIANCE {
        return None;
    }
    Some(laplacian_variance(output, format, rect)? / before)
}

#[cfg(test)]
mod tests {
    use blurrd_core::RectI;

    use super::{hf_residual, laplacian_variance};
    use crate::{Plane, SampleFormat};

    fn plane(data: &[u8], width: u32) -> Plane<'_> {
        Plane {
            data,
            stride: width as usize,
            width,
            height: data.len() as u32 / width,
        }
    }

    #[test]
    fn flat_planes_have_no_detail() {
        let data = vec![90u8; 64];
        let rect = RectI::new(0, 0, 8, 8).unwrap();
        assert_eq!(
            laplacian_variance(plane(&data, 8), SampleFormat::U8, rect),
            Some(0.0)
        );
        assert_eq!(
            hf_residual(plane(&data, 8), plane(&data, 8), SampleFormat::U8, rect),
            None
        );
    }

    #[test]
    fn checkerboard_has_detail_and_full_residual_against_itself() {
        let data: Vec<u8> = (0..64)
            .map(|i| if (i % 8 + i / 8) % 2 == 0 { 0 } else { 200 })
            .collect();
        let rect = RectI::new(0, 0, 8, 8).unwrap();
        let p = plane(&data, 8);
        assert!(laplacian_variance(p, SampleFormat::U8, rect).unwrap() > 1000.0);
        let r = hf_residual(p, p, SampleFormat::U8, rect).unwrap();
        assert!((r - 1.0).abs() < 1e-12);
        assert_eq!(
            laplacian_variance(p, SampleFormat::U8, RectI::new(0, 0, 9, 8).unwrap()),
            None
        );
    }
}
