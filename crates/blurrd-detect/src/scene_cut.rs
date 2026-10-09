//! Hard cuts in edited videos, seen from the HUD (port spec section 10.4,
//! plan 7.3).
//!
//! The input may already be a montage. Temporal models must not carry a
//! name box from one shot into the next: pre-roll, post-roll, gap filling
//! and holds stop at a cut. A cut is a frame whose 64 × 36 luma thumbnail
//! differs strongly from the previous one, whose brightness histogram
//! changed (a fast camera turn changes the thumbnail but keeps the
//! histogram), *and* whose picture is no longer the previous one with
//! other brightness and contrast (a flashbang, a fade or an exposure
//! change keeps the thumbnails correlated).
//!
//! Not every cut is a cut for every HUD region: when a region's crop did
//! not change (the same HUD over a camera switch), the cut does not
//! concern that region. A track that keeps detecting across a cut is
//! never ended by it (P13); only the extensions stop there.

use blurrd_core::{Region, RegionSet};

use crate::image::RgbView;

/// Width of the frame thumbnail.
pub const THUMB_W: usize = 64;
/// Height of the frame thumbnail.
pub const THUMB_H: usize = 36;
/// Width of a region thumbnail.
const REGION_W: usize = 32;
/// Height of a region thumbnail.
const REGION_H: usize = 16;
/// Bins of the brightness histogram.
const BINS: usize = 16;

/// Thresholds of [`SceneCutDetector`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneCutParams {
    /// Mean absolute luma difference of the thumbnails (0–255) from which
    /// a frame can be a cut.
    pub thumb_diff: f32,
    /// L1 distance of the normalised brightness histograms (0–2) a cut
    /// needs as well.
    pub hist_dist: f32,
    /// Correlation of the two thumbnails (−1 to 1) from which the frame
    /// shows the same picture with other brightness and contrast: not a
    /// cut.
    pub same_picture: f32,
    /// Mean absolute luma difference of a region thumbnail below which the
    /// region did not change at a cut.
    pub region_diff: f32,
}

impl Default for SceneCutParams {
    fn default() -> Self {
        Self {
            thumb_diff: 30.0,
            hist_dist: 0.5,
            same_picture: 0.6,
            region_diff: 8.0,
        }
    }
}

/// What changed between the previous frame and this one.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CutMeasures {
    /// Mean absolute luma difference of the frame thumbnails (0–255).
    pub thumb_diff: f32,
    /// L1 distance of the normalised brightness histograms (0–2).
    pub hist_dist: f32,
    /// Correlation of the frame thumbnails (−1 to 1; 0 when one of them
    /// is flat).
    pub correlation: f32,
}

/// The scene-cut verdict for one frame.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SceneCut {
    /// Whether a hard cut lies between the previous frame and this one.
    pub frame: bool,
    /// The regions the cut concerns: those whose crop changed with it.
    /// Empty without a cut.
    pub regions: RegionSet,
    /// The measures behind the verdict.
    pub measures: CutMeasures,
}

impl SceneCut {
    /// Whether the cut ends the extensions of temporal models in `region`.
    pub fn is_cut_for(&self, region: Region) -> bool {
        self.regions.contains(region)
    }
}

/// Finds hard cuts frame by frame.
///
/// Feed every frame in presentation order; after a seek, call
/// [`SceneCutDetector::reset`] (the first frame after it is a cut for every
/// region, as nothing is known about the frame before it).
#[derive(Debug, Clone)]
pub struct SceneCutDetector {
    params: SceneCutParams,
    thumb: Vec<u8>,
    previous: Option<Vec<u8>>,
    histogram: [f32; BINS],
    previous_histogram: [f32; BINS],
    regions: Vec<(Region, Vec<u8>)>,
    region_thumb: Vec<u8>,
    scratch: ThumbScratch,
}

impl Default for SceneCutDetector {
    fn default() -> Self {
        Self::new(SceneCutParams::default())
    }
}

impl SceneCutDetector {
    /// A detector with the given thresholds.
    pub fn new(params: SceneCutParams) -> Self {
        Self {
            params,
            thumb: Vec::with_capacity(THUMB_W * THUMB_H),
            previous: None,
            histogram: [0.0; BINS],
            previous_histogram: [0.0; BINS],
            regions: Vec::new(),
            region_thumb: Vec::with_capacity(REGION_W * REGION_H),
            scratch: ThumbScratch::default(),
        }
    }

    /// Forgets the previous frame.
    pub fn reset(&mut self) {
        self.previous = None;
        self.regions.clear();
    }

    /// Judges the frame shown in `screen` (the whole frame at any scale)
    /// with the crops of the HUD regions in `hud`.
    pub fn push(&mut self, screen: &RgbView<'_>, hud: &[(Region, RgbView<'_>)]) -> SceneCut {
        self.scratch
            .thumbnail(screen, THUMB_W, THUMB_H, &mut self.thumb);
        self.histogram = [0.0; BINS];
        for &v in &self.thumb {
            self.histogram[usize::from(v) * BINS / 256] += 1.0 / (THUMB_W * THUMB_H) as f32;
        }
        let first = self.previous.is_none();
        let measures = self
            .previous
            .as_ref()
            .map_or(CutMeasures::default(), |prev| CutMeasures {
                thumb_diff: mean_abs_diff(&self.thumb, prev),
                hist_dist: self
                    .histogram
                    .iter()
                    .zip(&self.previous_histogram)
                    .map(|(a, b)| (a - b).abs())
                    .sum(),
                correlation: correlation(&self.thumb, prev),
            });
        let frame = first
            || (measures.thumb_diff >= self.params.thumb_diff
                && measures.hist_dist >= self.params.hist_dist
                && measures.correlation < self.params.same_picture);

        let mut regions = RegionSet::empty();
        for (region, crop) in hud {
            self.scratch
                .thumbnail(crop, REGION_W, REGION_H, &mut self.region_thumb);
            let slot = if let Some(i) = self.regions.iter().position(|(r, _)| r == region) {
                i
            } else {
                self.regions.push((*region, Vec::new()));
                self.regions.len() - 1
            };
            let stored = &mut self.regions[slot].1;
            let changed = stored.len() != self.region_thumb.len()
                || mean_abs_diff(stored, &self.region_thumb) >= self.params.region_diff;
            if frame && changed {
                regions.insert(*region);
            }
            stored.clone_from(&self.region_thumb);
        }

        self.previous_histogram = self.histogram;
        match &mut self.previous {
            Some(prev) => prev.clone_from(&self.thumb),
            None => self.previous = Some(self.thumb.clone()),
        }
        SceneCut {
            frame,
            regions,
            measures,
        }
    }
}

/// Pearson correlation of two equally long byte slices, 0 when one of
/// them is flat.
fn correlation(first: &[u8], second: &[u8]) -> f32 {
    let count = first.len().max(1) as f64;
    let mean = |values: &[u8]| values.iter().map(|&v| f64::from(v)).sum::<f64>() / count;
    let (mean_first, mean_second) = (mean(first), mean(second));
    let (mut cross, mut var_first, mut var_second) = (0.0, 0.0, 0.0);
    for (&p, &q) in first.iter().zip(second) {
        let (p, q) = (f64::from(p) - mean_first, f64::from(q) - mean_second);
        cross += p * q;
        var_first += p * p;
        var_second += q * q;
    }
    let denominator = (var_first * var_second).sqrt();
    if denominator > 0.0 {
        (cross / denominator) as f32
    } else {
        0.0
    }
}

/// Mean absolute difference of two equally long byte slices.
fn mean_abs_diff(a: &[u8], b: &[u8]) -> f32 {
    let sum: u64 = a
        .iter()
        .zip(b)
        .map(|(&x, &y)| u64::from(x.abs_diff(y)))
        .sum();
    sum as f32 / a.len().max(1) as f32
}

/// Buffers of the thumbnail computation, reused across frames.
#[derive(Debug, Clone, Default)]
struct ThumbScratch {
    /// Thumbnail column of every source column.
    columns: Vec<usize>,
    /// Luma sums per thumbnail cell.
    sums: Vec<u32>,
    /// Source pixels per thumbnail cell.
    counts: Vec<u32>,
}

impl ThumbScratch {
    /// The `w × h` luma thumbnail of `view` by area averaging (each cell
    /// is the mean of the source pixels whose top-left corners fall into
    /// it).
    fn thumbnail(&mut self, view: &RgbView<'_>, cols: usize, rows: usize, out: &mut Vec<u8>) {
        out.clear();
        out.resize(cols * rows, 0);
        let (width, height) = (view.width() as usize, view.height() as usize);
        if width == 0 || height == 0 {
            return;
        }
        self.columns.clear();
        self.columns.extend((0..width).map(|x| x * cols / width));
        self.sums.clear();
        self.sums.resize(cols * rows, 0);
        self.counts.clear();
        self.counts.resize(cols * rows, 0);
        for y in 0..height {
            let base = y * rows / height * cols;
            let row = view.row(y as u32);
            for (px, &cell) in row.as_chunks::<3>().0.iter().zip(&self.columns) {
                let [r, g, b] = px.map(u32::from);
                self.sums[base + cell] += (299 * r + 587 * g + 114 * b + 500) / 1000;
                self.counts[base + cell] += 1;
            }
        }
        for ((luma, sum), count) in out.iter_mut().zip(&self.sums).zip(&self.counts) {
            *luma = (sum + count / 2).checked_div(*count).unwrap_or(0) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use blurrd_core::Region;

    use super::{SceneCutDetector, ThumbScratch};
    use crate::image::RgbView;

    fn image(w: usize, h: usize, f: impl Fn(usize, usize) -> u8) -> Vec<u8> {
        (0..w * h).flat_map(|i| [f(i % w, i / w); 3]).collect()
    }

    #[test]
    fn thumbnails_average_areas() {
        let data = image(128, 72, |x, _| if x < 64 { 0 } else { 200 });
        let view = RgbView::new(&data, 128, 72, 384).unwrap();
        let mut out = vec![];
        ThumbScratch::default().thumbnail(&view, 64, 36, &mut out);
        assert_eq!((out[0], out[31], out[32], out[63]), (0, 0, 200, 200));
    }

    #[test]
    fn cuts_need_a_new_picture_and_a_new_histogram() {
        fn view(d: &[u8]) -> RgbView<'_> {
            RgbView::new(d, 128, 72, 384).unwrap()
        }
        let w = 128;
        let dark = image(w, 72, |x, y| ((x + y) % 40) as u8);
        let shifted = image(w, 72, |x, y| ((x + 20 + y) % 40) as u8);
        let bright = image(w, 72, |x, y| 180 + ((x * y) % 50) as u8);
        let hud = image(32, 16, |x, _| x as u8);
        let mut det = SceneCutDetector::default();
        let hud_view = RgbView::new(&hud, 32, 16, 96).unwrap();
        let first = det.push(&view(&dark), &[(Region::VoiceChat, hud_view)]);
        assert!(first.frame && first.is_cut_for(Region::VoiceChat));
        // A moving texture: the picture changes, the histogram does not.
        assert!(!det.push(&view(&shifted), &[]).frame);
        // A new shot, but the voice-chat crop is the same: a cut for the
        // frame, not for the region.
        let cut = det.push(&view(&bright), &[(Region::VoiceChat, hud_view)]);
        assert!(cut.frame && !cut.is_cut_for(Region::VoiceChat));
        let changed = image(32, 16, |_, _| 230);
        let changed = RgbView::new(&changed, 32, 16, 96).unwrap();
        let back = det.push(&view(&dark), &[(Region::VoiceChat, changed)]);
        assert!(back.frame && back.is_cut_for(Region::VoiceChat));
        // A flash: the same picture, far brighter and with less contrast.
        let flashed: Vec<u8> = dark.iter().map(|&v| 200 + v / 4).collect();
        let flash = det.push(&view(&flashed), &[(Region::VoiceChat, changed)]);
        assert!(!flash.frame, "{:?}", flash.measures);
        assert!(flash.measures.thumb_diff >= 30.0 && flash.measures.hist_dist >= 0.5);
        det.reset();
        assert!(det.push(&view(&dark), &[]).frame);
    }
}
