//! Skipping crops that did not change since the last OCR.

use std::collections::BTreeMap;

use crate::engine::ZoneId;
use crate::image::RgbView;

/// Remembers, per zone, a quarter-scale copy of the last crop that was
/// OCR'd and says whether a new crop differs from it.
///
/// A crop is skipped only when its mean absolute difference to the **last
/// OCR'd** crop (not the previous frame) is below [`Self::MAD_THRESHOLD`],
/// so a slow fade or slide can never creep past OCR one small step at a
/// time (architecture plan 5.5, review point D17). The threshold and the
/// quarter-scale sampling are the prototype's (`ocr_leak_scan.py`).
#[derive(Debug, Clone, Default)]
pub struct CropDedup {
    last: BTreeMap<ZoneId, Thumb>,
}

/// Every fourth pixel of every fourth row, all three channels.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Thumb {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

impl Thumb {
    fn of(image: RgbView<'_>) -> Self {
        let width = image.width().div_ceil(4);
        let height = image.height().div_ceil(4);
        let mut data = Vec::with_capacity(3 * (width * height) as usize);
        for y in (0..image.height()).step_by(4) {
            let row = image.row(y);
            for px in row.as_chunks::<3>().0.iter().step_by(4) {
                data.extend_from_slice(px);
            }
        }
        Self {
            width,
            height,
            data,
        }
    }

    /// Mean absolute difference per sample, or `None` for other sizes.
    fn mad(&self, other: &Self) -> Option<f32> {
        if (self.width, self.height) != (other.width, other.height) {
            return None;
        }
        let sum: u64 = self
            .data
            .iter()
            .zip(&other.data)
            .map(|(&a, &b)| u64::from(a.abs_diff(b)))
            .sum();
        Some(sum as f32 / self.data.len().max(1) as f32)
    }
}

impl CropDedup {
    /// Crops closer than this (mean absolute difference per sample, in code
    /// values) to the last OCR'd crop of their zone are skipped.
    pub const MAD_THRESHOLD: f32 = 0.6;

    /// No crop seen yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether `image` must be OCR'd: `true` for the first crop of a zone,
    /// a crop of another size, or one that differs from the last OCR'd crop
    /// by at least [`Self::MAD_THRESHOLD`]. When it returns `true`, the crop
    /// becomes the zone's last OCR'd crop, so the caller must OCR it.
    pub fn should_ocr(&mut self, zone: ZoneId, image: RgbView<'_>) -> bool {
        let thumb = Thumb::of(image);
        let changed = self
            .last
            .get(&zone)
            .and_then(|last| last.mad(&thumb))
            .is_none_or(|mad| mad >= Self::MAD_THRESHOLD);
        if changed {
            self.last.insert(zone, thumb);
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::CropDedup;
    use crate::engine::ZoneId;
    use crate::image::RgbView;

    #[test]
    fn skips_only_crops_close_to_the_last_ocrd_one() {
        fn view(data: &[u8]) -> RgbView<'_> {
            RgbView::new(16, 16, data).unwrap()
        }
        let mut dedup = CropDedup::new();
        let zone = ZoneId(1);
        let base = vec![100u8; 16 * 16 * 3];
        assert!(dedup.should_ocr(zone, view(&base)));
        assert!(!dedup.should_ocr(zone, view(&base)));
        // Creeping changes below the threshold per step are measured
        // against the last OCR'd crop, so they add up and trigger OCR.
        let mut creeping = base.clone();
        let mut triggered = false;
        for _ in 0..3 {
            for v in creeping.iter_mut().step_by(2) {
                *v += 1; // half the samples +1: mean 0.5 per step
            }
            triggered |= dedup.should_ocr(zone, view(&creeping));
        }
        assert!(triggered);
        // Other zones and other sizes are independent.
        assert!(dedup.should_ocr(ZoneId(2), view(&base)));
        let small = vec![100u8; 8 * 8 * 3];
        assert!(dedup.should_ocr(zone, RgbView::new(8, 8, &small).unwrap()));
    }
}
