//! The quality metrics of architecture plan 11.3, measured against ground
//! truth.
//!
//! The same functions serve the synthetic fixtures (truth from the
//! generator) and the real-footage goldens (truth from review). Boxes are
//! given as [`PreparedBox`]es, so the metrics see exactly the mask the blur
//! applies: a glyph pixel counts as covered only if the blur replaces it
//! with full strength.

use std::collections::BTreeMap;

use blurrd_blur::metrics::hf_residual;
use blurrd_blur::{BlurBackend, BlurError, CpuBlur, PixelParams, PreparedBox, SampleFormat};
use blurrd_core::{RectI, Region};

use crate::image::RgbFrame;
use crate::truth::{FrameTruth, ItemId, TruthItem};

/// Mask value a glyph pixel needs to count as covered (plan 11.3,
/// geometric leak).
pub const LEAK_COVERAGE: f32 = 0.999;

/// Mask value above which a pixel counts as blurred for the over-blur
/// ratio (port spec 11.3).
pub const BLURRED_COVERAGE: f32 = 0.1;

/// Margin around the text, in reference pixels `(x, y)`, that a Tight box
/// may blur without counting as over-blur (port spec 11.3).
pub const OVERBLUR_MARGIN_REF: (f32, f32) = (6.0, 4.0);

/// Presence or coverage runs shorter than this many frames count as
/// flicker (plan 11.3, temporal jitter).
pub const MIN_STABLE_RUN: u32 = 3;

/// The blur mask at pixel `(x, y)`: the largest coverage of any box.
pub fn coverage_at(boxes: &[PreparedBox], x: i32, y: i32) -> f32 {
    boxes
        .iter()
        .filter(|b| {
            let p = b.padded();
            x >= p.x0 && x < p.x1 && y >= p.y0 && y < p.y1
        })
        .map(|b| b.coverage(x, y))
        .fold(0.0, f32::max)
}

/// How well one name is covered in one frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemCoverage {
    /// The name.
    pub id: ItemId,
    /// Its region.
    pub region: Region,
    /// Pixels of its dilated glyph mask.
    pub glyph_pixels: u64,
    /// Glyph pixels whose mask is below [`LEAK_COVERAGE`].
    pub leaked_pixels: u64,
}

impl ItemCoverage {
    /// Whether any glyph pixel leaks: one leaked item-frame.
    pub const fn is_leak(&self) -> bool {
        self.leaked_pixels > 0
    }
}

/// Coverage of every name of a frame by `boxes`.
pub fn item_coverage(truth: &FrameTruth, boxes: &[PreparedBox]) -> Vec<ItemCoverage> {
    truth
        .items
        .iter()
        .map(|item| {
            let mut leaked = 0;
            for (x, y) in item.glyphs.pixels() {
                if coverage_at(boxes, x, y) < LEAK_COVERAGE {
                    leaked += 1;
                }
            }
            ItemCoverage {
                id: item.id,
                region: item.region,
                glyph_pixels: item.glyphs.count(),
                leaked_pixels: leaked,
            }
        })
        .collect()
}

/// Blurred pixels and how many of them lie away from every name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OverBlur {
    /// Pixels with a mask above [`BLURRED_COVERAGE`].
    pub blurred: u64,
    /// Blurred pixels outside every name's text bounds grown by
    /// [`OVERBLUR_MARGIN_REF`].
    pub outside: u64,
}

impl OverBlur {
    /// `outside / blurred` (0 when nothing is blurred).
    pub fn ratio(&self) -> f64 {
        if self.blurred == 0 {
            0.0
        } else {
            self.outside as f64 / self.blurred as f64
        }
    }

    fn add(&mut self, other: Self) {
        self.blurred += other.blurred;
        self.outside += other.outside;
    }
}

/// The text bounds of an item grown by the over-blur margin.
fn allowed_area(item: &TruthItem, scale: f32) -> RectI {
    let mx = (OVERBLUR_MARGIN_REF.0 * scale).ceil() as i32;
    let my = (OVERBLUR_MARGIN_REF.1 * scale).ceil() as i32;
    let g = item.glyphs.bounds().unwrap_or(item.text);
    RectI {
        x0: g.x0 - mx,
        y0: g.y0 - my,
        x1: g.x1 + mx,
        y1: g.y1 + my,
    }
}

/// The over-blur of `boxes` in one frame.
pub fn over_blur(truth: &FrameTruth, boxes: &[PreparedBox]) -> OverBlur {
    let allowed: Vec<RectI> = truth
        .items
        .iter()
        .map(|i| allowed_area(i, truth.scale))
        .collect();
    let mut out = OverBlur::default();
    for_each_blurred_pixel(boxes, |x, y, m| {
        if m > BLURRED_COVERAGE {
            out.blurred += 1;
            if !allowed
                .iter()
                .any(|r| x >= r.x0 && x < r.x1 && y >= r.y0 && y < r.y1)
            {
                out.outside += 1;
            }
        }
    });
    out
}

/// Pixels with any blur inside a portrait.
pub fn portrait_intrusion(truth: &FrameTruth, boxes: &[PreparedBox]) -> u64 {
    let portraits: Vec<RectI> = truth.portraits().collect();
    let mut count = 0;
    for_each_blurred_pixel(boxes, |x, y, m| {
        if m > 0.0
            && portraits
                .iter()
                .any(|r| x >= r.x0 && x < r.x1 && y >= r.y0 && y < r.y1)
        {
            count += 1;
        }
    });
    count
}

/// Calls `f(x, y, mask)` once for every pixel of the union of the padded
/// boxes (a pixel belongs to the first box that contains it).
fn for_each_blurred_pixel(boxes: &[PreparedBox], mut f: impl FnMut(i32, i32, f32)) {
    let inside = |r: RectI, x: i32, y: i32| x >= r.x0 && x < r.x1 && y >= r.y0 && y < r.y1;
    for (i, b) in boxes.iter().enumerate() {
        let p = b.padded();
        for y in p.y0..p.y1 {
            for x in p.x0..p.x1 {
                if !boxes[..i].iter().any(|e| inside(e.padded(), x, y)) {
                    f(x, y, coverage_at(boxes, x, y));
                }
            }
        }
    }
}

/// The high-frequency residual of every name after blurring `frame` with
/// the real blur: the frame is converted to I420 (BT.709, limited range),
/// blurred with [`CpuBlur`], and
/// [`blurrd_blur::metrics::hf_residual`] is measured on the luma plane
/// over each name's text bounds.
///
/// `None` for a name means its source has too little detail to measure
/// (see [`blurrd_blur::metrics::MIN_DETAIL_VARIANCE`]).
///
/// # Errors
///
/// Propagates [`BlurError`] from the blur (for example a box prepared for a
/// different frame size).
pub fn hf_residuals(
    frame: &RgbFrame,
    truth: &FrameTruth,
    boxes: &[PreparedBox],
    params: &PixelParams,
) -> Result<Vec<(ItemId, Option<f64>)>, BlurError> {
    let source = frame.to_i420();
    let mut output = source.clone();
    CpuBlur.blur(&mut output.as_frame_mut(), (0, 0), boxes, params)?;
    Ok(truth
        .items
        .iter()
        .map(|item| {
            let r = hf_residual(source.luma(), output.luma(), SampleFormat::U8, item.text);
            (item.id, r)
        })
        .collect())
}

/// Per-item history inside an [`Evaluation`].
#[derive(Debug, Clone, Default)]
struct ItemHistory {
    region: Option<Region>,
    last_frame: Option<u32>,
    last_text: Option<RectI>,
    last_box: Option<RectI>,
    /// Current run of equal coverage states: (covered, length, state
    /// before the run).
    run: Option<(bool, u32, Option<bool>)>,
    leak_run: u32,
}

/// Results of an [`Evaluation`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Summary {
    /// Frames evaluated.
    pub frames: u64,
    /// Name-frames in the truth.
    pub item_frames: u64,
    /// Name-frames with at least one leaked glyph pixel.
    pub leaked_item_frames: u64,
    /// Longest run of consecutive leaked frames of one name.
    pub longest_leak_run: u32,
    /// Leaked name-frames per region.
    pub leaks_per_region: BTreeMap<Region, u64>,
    /// Over-blur summed over all frames.
    pub over_blur: OverBlur,
    /// Boxes that touch no name, summed over frames.
    pub false_positive_box_frames: u64,
    /// Blurred pixels inside portraits, summed over frames.
    pub portrait_pixels: u64,
    /// Frames in which a name kept its position but its box changed.
    pub jitter_frames: u64,
    /// Coverage runs shorter than [`MIN_STABLE_RUN`] frames between two
    /// runs of the other state.
    pub flicker_runs: u64,
}

impl Summary {
    /// Share of name-frames without a leak (the recall of plan 11.3).
    pub fn recall(&self) -> f64 {
        if self.item_frames == 0 {
            1.0
        } else {
            1.0 - self.leaked_item_frames as f64 / self.item_frames as f64
        }
    }

    /// False-positive box-seconds at `fps` frames per second.
    pub fn false_positive_seconds(&self, fps: f64) -> f64 {
        self.false_positive_box_frames as f64 / fps
    }

    /// The region with the most leaked name-frames, if any leaked.
    pub fn worst_region(&self) -> Option<Region> {
        self.leaks_per_region
            .iter()
            .filter(|(_, n)| **n > 0)
            .max_by_key(|(_, n)| **n)
            .map(|(r, _)| *r)
    }
}

/// Accumulates every metric over a sequence of frames.
///
/// Frames must be added in increasing order; a name missing from a frame
/// ends its runs.
#[derive(Debug, Clone, Default)]
pub struct Evaluation {
    items: BTreeMap<ItemId, ItemHistory>,
    summary: Summary,
}

impl Evaluation {
    /// An empty evaluation.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds one frame: its truth and the boxes the blur applies to it.
    pub fn add_frame(&mut self, truth: &FrameTruth, boxes: &[PreparedBox]) {
        let s = &mut self.summary;
        s.frames += 1;
        s.over_blur.add(over_blur(truth, boxes));
        s.portrait_pixels += portrait_intrusion(truth, boxes);
        let touched: Vec<RectI> = truth
            .items
            .iter()
            .map(|i| i.glyphs.bounds().unwrap_or(i.text))
            .collect();
        s.false_positive_box_frames += boxes
            .iter()
            .filter(|b| !touched.iter().any(|t| b.core().intersection(t).is_some()))
            .count() as u64;
        for (item, cov) in truth.items.iter().zip(item_coverage(truth, boxes)) {
            self.add_item(truth.frame, item, cov, boxes);
        }
    }

    fn add_item(&mut self, frame: u32, item: &TruthItem, cov: ItemCoverage, boxes: &[PreparedBox]) {
        let s = &mut self.summary;
        s.item_frames += 1;
        let h = self.items.entry(item.id).or_default();
        h.region = Some(item.region);
        let continues = h.last_frame.is_some_and(|f| f + 1 == frame);
        if !continues {
            h.last_text = None;
            h.last_box = None;
            h.run = None;
            h.leak_run = 0;
        }
        if cov.is_leak() {
            s.leaked_item_frames += 1;
            *s.leaks_per_region.entry(item.region).or_default() += 1;
            h.leak_run += 1;
            s.longest_leak_run = s.longest_leak_run.max(h.leak_run);
        } else {
            h.leak_run = 0;
        }
        // Jitter: the name stayed, its box moved.
        let text = item.glyphs.bounds().unwrap_or(item.text);
        let matched = boxes
            .iter()
            .filter_map(|b| b.core().intersection(&text).map(|i| (i.area(), b.core())))
            .max_by_key(|(area, _)| *area)
            .map(|(_, r)| r);
        if h.last_text == Some(item.text)
            && h.last_box.is_some()
            && matched.is_some()
            && h.last_box != matched
        {
            s.jitter_frames += 1;
        }
        h.last_text = Some(item.text);
        h.last_box = matched;
        // Flicker: short runs of one coverage state between the other.
        let covered = !cov.is_leak();
        h.run = Some(match h.run {
            Some((state, len, before)) if state == covered => (state, len + 1, before),
            Some((state, len, before)) => {
                if before == Some(covered) && len < MIN_STABLE_RUN {
                    s.flicker_runs += 1;
                }
                (covered, 1, Some(state))
            }
            None => (covered, 1, None),
        });
        h.last_frame = Some(frame);
    }

    /// The results so far.
    pub fn summary(&self) -> &Summary {
        &self.summary
    }
}

#[cfg(test)]
mod tests {
    use blurrd_blur::{PixelParams, PreparedBox, YuvColour, prepare_boxes};
    use blurrd_core::{BlurBox, BlurParams, NameKind, RectI, Region};

    use super::{Evaluation, item_coverage, over_blur, portrait_intrusion};
    use crate::mask::Mask;
    use crate::truth::{Content, ContentKind, FrameTruth, ItemId, TruthItem};

    fn rect(x0: i32, y0: i32, x1: i32, y1: i32) -> RectI {
        RectI::new(x0, y0, x1, y1).unwrap()
    }

    fn truth(frame: u32, text: RectI) -> FrameTruth {
        let mut glyphs = Mask::empty(text);
        for y in text.y0..text.y1 {
            for x in text.x0..text.x1 {
                glyphs.insert(x, y);
            }
        }
        FrameTruth {
            frame,
            scale: 1.0,
            items: vec![TruthItem {
                id: ItemId(1),
                region: Region::KillFeed,
                kind: NameKind::Victim,
                text,
                glyphs,
                slot: text,
                opacity: 1.0,
            }],
            content: vec![Content {
                kind: ContentKind::Portrait,
                rect: rect(300, 100, 340, 130),
            }],
        }
    }

    fn boxes(rects: &[RectI]) -> Vec<PreparedBox> {
        let params = PixelParams::new(&BlurParams::default(), 1.0, YuvColour::default()).unwrap();
        let bb: Vec<BlurBox> = rects
            .iter()
            .map(|&r| BlurBox {
                rect: r,
                region: Region::KillFeed,
                seed: 1,
                grid_origin: (r.x0, r.y0),
                block_scale: 1.0,
            })
            .collect();
        let mut out = vec![];
        prepare_boxes(&bb, 640, 360, &params, &mut out);
        out
    }

    #[test]
    fn counts_leaked_glyph_pixels() {
        let t = truth(0, rect(100, 100, 150, 120));
        let full = item_coverage(&t, &boxes(&[rect(100, 100, 150, 120)]));
        assert!(!full[0].is_leak());
        let short = item_coverage(&t, &boxes(&[rect(100, 100, 140, 120)]));
        // Columns 140..150 sit in the padding (feather) or outside.
        assert!(short[0].leaked_pixels >= 20 * 6);
        assert_eq!(item_coverage(&t, &[])[0].leaked_pixels, 50 * 20);
    }

    #[test]
    fn measures_over_blur_and_portrait_intrusion() {
        let t = truth(0, rect(100, 100, 150, 120));
        let tight = over_blur(&t, &boxes(&[rect(100, 100, 150, 120)]));
        assert!(tight.ratio() < 0.05, "{tight:?}");
        let wide = over_blur(&t, &boxes(&[rect(100, 100, 250, 120)]));
        assert!(wide.ratio() > 0.5);
        assert_eq!(
            portrait_intrusion(&t, &boxes(&[rect(100, 100, 150, 120)])),
            0
        );
        assert!(portrait_intrusion(&t, &boxes(&[rect(290, 100, 320, 120)])) > 0);
    }

    #[test]
    fn evaluates_sequences() {
        let mut e = Evaluation::new();
        let good = boxes(&[rect(100, 100, 150, 120)]);
        let stray = boxes(&[rect(100, 100, 150, 120), rect(400, 200, 440, 220)]);
        // Frames 0-1 covered, 2 leaked (flicker), 3-5 covered with a stray
        // box in frame 4 and a moved box in frame 5.
        for (f, b) in [
            (0, &good),
            (1, &good),
            (2, &vec![]),
            (3, &good),
            (4, &stray),
            (5, &boxes(&[rect(99, 100, 150, 120)])),
        ] {
            e.add_frame(&truth(f, rect(100, 100, 150, 120)), b);
        }
        let s = e.summary();
        assert_eq!((s.frames, s.item_frames, s.leaked_item_frames), (6, 6, 1));
        assert_eq!(s.longest_leak_run, 1);
        assert_eq!(s.worst_region(), Some(Region::KillFeed));
        assert_eq!(s.false_positive_box_frames, 1);
        assert_eq!(s.jitter_frames, 1);
        assert_eq!(s.flicker_runs, 1);
        assert!((s.recall() - 5.0 / 6.0).abs() < 1e-9);
        assert!((s.false_positive_seconds(60.0) - 1.0 / 60.0).abs() < 1e-12);
    }
}
