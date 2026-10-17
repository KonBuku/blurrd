//! Calibrating a video's HUD geometry from its frames (plan 7.3).
//!
//! The frame size alone does not say where the HUD is: a 16:9 recording
//! can hold a 16:9 game or a 4:3 game stretched to fill it, and a 4:3 game
//! can sit between black bars (a pillarbox). [`Calibrator`] decides from
//! sampled frames:
//!
//! 1. Black bars: columns at both frame edges that stay black in every
//!    row, of the same width on both sides, make a pillarbox; rows at the
//!    top and the bottom that stay black in every column make a
//!    letterbox.
//! 2. Every layout the frame size and the bars allow is a hypothesis
//!    ([`HudGeometry::for_layout`]). Each sampled frame is read through
//!    each hypothesis with the kill-feed and text-chat structure finders:
//!    kill-feed rows (victim text line, portrait, bar) and the chat box.
//!    Under a wrong mapping the right-aligned victim names, the portraits
//!    and the box edges miss their columns, so only the true layout finds
//!    them.
//! 3. The layout with the most aligned structure wins. Its confidence is
//!    its lead over every other layout, counted only where the two differ
//!    (structure both align says nothing about which one is right) and
//!    scaled down while fewer than [`MIN_EVIDENCE`] structures make the
//!    difference; a region's confidence is the same for that region's
//!    structure. Layouts whose HUDs lie within the alignment tolerance in
//!    one region (a 16:10 game and a 16:9 image squeezed to 16:10 place
//!    the kill feed 2 px apart) are told apart by another region (the
//!    chat box, 50 px apart); when no region tells them apart the
//!    confidence is 0 and the analysis uses Safe boxes. The frame-size
//!    layout of an exactly 16:9 frame without bars needs only one aligned
//!    structure when no other layout aligns any: it is the common case,
//!    and a calibration from a few HUD frames would only add blur.
//!
//! The kill-feed rows of the winning layout also learn the video's bar
//! colours ([`BarHueLearner`]): [`CalibratedSegment::hues`] holds them when
//! they differ from the default teal and red (a colour-blind mode, custom
//! team colours, a GPU colour filter), and their
//! [`BarHues::params`](crate::killfeed::BarHues::params) make them the
//! detector's fast path.
//!
//! The calibration is kept per scene segment: when the bars change for good
//! (the game resolution changed, a different recording was spliced in), a
//! new segment starts. Confidence below [`MIN_CONFIDENCE`] means Safe
//! geometry and a `CalibrationLow` warning (plan 7.3); `sy` is never assumed
//! silently.

use std::collections::BTreeMap;

use blurrd_core::{HudAnchor, HudGeometry, HudLayout, Region};

use crate::chat::{BOX_X, BoxBand, ChatDetector, LEFT_PANEL_ROI, PANEL_X1};
use crate::classes::ClassParams;
use crate::detection::Detection;
use crate::geometry::{HudCrop, HudSampler};
use crate::image::RgbView;
use crate::killfeed::{BarHueLearner, BarHues, FEED_ROI, KillFeedDetector, ROW_PITCH};

/// Calibration confidence below this means Safe geometry and a
/// `CalibrationLow` warning.
pub const MIN_CONFIDENCE: f32 = 0.5;

/// Frames with structure a layout needs for full confidence.
pub const MIN_EVIDENCE: u32 = 3;

/// A column (row) is black if no pixel of it (every [`BAR_STEP`]th) is
/// brighter than this (largest channel; video black is 16 in limited
/// range, a compressed black bar stays a few levels above it).
const BAR_MAX_LEVEL: u8 = 28;
/// Pixels of a column or row checked for a black bar.
const BAR_STEP: usize = 4;
/// A bar is at least this share of the frame width (pillarbox) or height
/// (letterbox): a 4:3 image in a 16:9 frame has bars of 12.5 %, a 16:9
/// image in a 16:10 frame 5 %; thin black edges of a capture card are not
/// bars.
const MIN_BAR_SHARE: f32 = 0.02;
/// Aspect ratios within this of 16:9 are 16:9.
const ASPECT_TOLERANCE: f32 = 0.02;
/// The bars on both sides differ by at most this many pixels.
const BAR_SYMMETRY: u32 = 4;
/// Consecutive samples with other bars that start a new segment.
const SEGMENT_SAMPLES: usize = 3;
/// Two bar widths within this many pixels are the same.
const BAR_TOLERANCE: u32 = 4;
/// The right end of every kill-feed row (reference x; the victim portrait
/// spans x 1838..1900).
const ROW_RIGHT: i32 = 1900;
/// The top of the first kill-feed row (reference y); the rows below it
/// stack one [`ROW_PITCH`] apart.
const FIRST_ROW_TOP: i32 = 96;
/// A row lies at most this far from the top of its slot.
const SLOT_TOLERANCE: i32 = 3;
/// Rows of a kill-feed row's band, from its top, measured for its end.
const ROW_EDGE_BAND: (i32, i32) = (4, 29);
/// Columns searched on each side of [`ROW_RIGHT`].
const ROW_EDGE_SEARCH: i32 = 12;
/// The row's end lies at most this far from [`ROW_RIGHT`] (reference
/// pixels, measured to a fraction of a pixel). Measured on real
/// recordings at 1050 to 1200 lines: the true layout puts the end within
/// 0.7 px, a 16:10 layout read on a 16:9 image squeezed to 16:10 (and the
/// reverse) 1.4 to 2.2 px away.
const EDGE_TOLERANCE: f32 = 1.0;
/// Columns on each side of the strongest edge whose edge strengths
/// locate the end to a fraction of a pixel.
const EDGE_SPREAD: i32 = 2;
/// The end's edge is at least this many times the mean edge strength of
/// the searched columns.
const EDGE_CONTRAST: i32 = 2;
/// Columns inside and outside a chat box edge measured, from the edge.
const BOX_EDGE_SPAN: (i32, i32) = (2, 8);
/// Inside a chat box edge at most this share of the brightness outside
/// it (the box darkens the frame to about 0.66).
const BOX_EDGE_RATIO: f32 = 0.85;
/// The frame outside a chat box edge is at least this bright, or the edge
/// cannot be seen.
const MIN_OUTSIDE_LEVEL: f32 = 24.0;

/// The calibrated geometry of one stretch of frames.
#[derive(Debug, Clone, PartialEq)]
pub struct CalibratedSegment {
    /// First frame of the segment (the first frame of the video for the
    /// first segment).
    pub first_frame: u32,
    /// The geometry, with its confidence.
    pub geometry: HudGeometry,
    /// The kill-feed bar colours learned in the segment, if they differ
    /// from the defaults.
    pub hues: Option<BarHues>,
}

/// The result of a calibration: one geometry per segment, in frame order.
#[derive(Debug, Clone, PartialEq)]
pub struct Calibration {
    /// The segments, the first starting at frame 0.
    pub segments: Vec<CalibratedSegment>,
}

impl Calibration {
    /// The geometry of frame `frame`.
    pub fn geometry_at(&self, frame: u32) -> Option<&HudGeometry> {
        self.segments
            .iter()
            .rev()
            .find(|s| s.first_frame <= frame)
            .or_else(|| self.segments.first())
            .map(|s| &s.geometry)
    }

    /// Whether any segment is calibrated with less than
    /// [`MIN_CONFIDENCE`].
    pub fn is_low(&self) -> bool {
        self.segments
            .iter()
            .any(|s| s.geometry.confidence < MIN_CONFIDENCE)
    }
}

/// One layout tried on the sampled frames.
#[derive(Debug)]
struct Hypothesis {
    sampler: HudSampler,
    feed: FeedReader,
    chat: ChatReader,
}

impl Hypothesis {
    fn new(geometry: HudGeometry) -> Self {
        Self {
            sampler: HudSampler::new(geometry),
            feed: FeedReader::default(),
            chat: ChatReader::default(),
        }
    }

    /// Aligned structure in all regions.
    const fn evidence(&self) -> u32 {
        self.feed.rows + self.chat.bands
    }
}

/// The kill-feed side of a [`Hypothesis`]. The regions keep their own
/// detectors and crop buffers, so they read a frame independently.
#[derive(Debug, Default)]
struct FeedReader {
    detector: KillFeedDetector,
    hues: BarHueLearner,
    crop: HudCrop,
    dets: Vec<Detection>,
    /// Kill-feed rows whose right end lies where the layout puts it.
    rows: u32,
}

impl FeedReader {
    /// Reads the kill feed of one frame through `sampler`.
    fn read(&mut self, sampler: &HudSampler, frame: &RgbView<'_>) {
        let feed = sampler.crop(frame, FEED_ROI, HudAnchor::Right, &mut self.crop);
        if self.detector.detect(&feed, &mut self.dets).is_ok() {
            // The rows come top to bottom: the k-th fills the k-th slot.
            let rows = self.detector.rows().iter().enumerate();
            self.rows += rows
                .filter(|&(k, r)| in_slot(r.top, k) && row_end_aligned(&feed, r.top))
                .count() as u32;
            self.hues.push(&self.detector);
        }
    }
}

/// The text-chat side of a [`Hypothesis`].
#[derive(Debug, Default)]
struct ChatReader {
    detector: ChatDetector,
    crop: HudCrop,
    dets: Vec<Detection>,
    /// Chat box bands with lines whose right edge lies where the layout
    /// puts it.
    bands: u32,
}

impl ChatReader {
    /// Reads the left panel of one frame through `sampler`.
    fn read(&mut self, sampler: &HudSampler, frame: &RgbView<'_>) {
        let left = sampler.crop(frame, LEFT_PANEL_ROI, HudAnchor::Left, &mut self.crop);
        if self.detector.detect(&left, &mut self.dets).is_ok() {
            // Bands with chat lines: the box alone could be a dark patch of
            // the background.
            let lines = self.detector.lines();
            let bands = self.detector.bands().iter();
            self.bands += bands
                .filter(|b| lines.iter().any(|l| l.band == **b) && box_edge_aligned(&left, b))
                .count() as u32;
        }
    }
}

/// The black bars of a frame in source pixels, 0 for none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Bars {
    /// Width of each bar left and right (pillarbox).
    pillar: u32,
    /// Height of each bar at the top and the bottom (letterbox).
    letter: u32,
}

impl Bars {
    /// The bars of `frame`.
    fn of(frame: &RgbView<'_>) -> Self {
        Self {
            pillar: pillar_bar(frame),
            letter: letter_bar(frame),
        }
    }

    /// Whether two frames have the same bars.
    const fn same(self, other: Self) -> bool {
        self.pillar.abs_diff(other.pillar) <= BAR_TOLERANCE
            && self.letter.abs_diff(other.letter) <= BAR_TOLERANCE
    }
}

/// One segment while it is being calibrated.
#[derive(Debug)]
struct Open {
    first_frame: u32,
    bars: Bars,
    /// The index in `hypotheses` of the frame-size layout of a 16:9 frame
    /// without bars ([module docs](self)).
    default: Option<usize>,
    hypotheses: Vec<Hypothesis>,
}

/// Calibrates the HUD geometry of one video from sampled frames ([module
/// docs](self)).
#[derive(Debug)]
pub struct Calibrator {
    width: u32,
    height: u32,
    /// The layout the user chose; [`HudLayout::Auto`] detects it.
    layout: HudLayout,
    done: Vec<CalibratedSegment>,
    open: Option<Open>,
    /// Bars of the latest samples that disagree with the open segment.
    pending: Vec<(u32, Bars)>,
}

impl Calibrator {
    /// A calibrator for frames of `width` × `height` pixels. A `layout`
    /// other than [`HudLayout::Auto`] is the user's choice: it is the only
    /// hypothesis, and the frames still measure its confidence.
    pub fn new(width: u32, height: u32, layout: HudLayout) -> Self {
        Self {
            width,
            height,
            layout,
            done: Vec::new(),
            open: None,
            pending: Vec::new(),
        }
    }

    /// Reads the sampled frame `index` (the whole frame). Frames must come
    /// in order; about one per second is enough.
    pub fn push(&mut self, index: u32, frame: &RgbView<'_>) {
        let bars = Bars::of(frame);
        match self.open.as_ref() {
            None => self.open = Some(self.segment(index, bars)),
            Some(open) if open.bars.same(bars) => self.pending.clear(),
            Some(_) => {
                self.pending.push((index, bars));
                let settled = self.pending.len() >= SEGMENT_SAMPLES
                    && self.pending.iter().all(|&(_, b)| b.same(bars));
                if settled {
                    let first = self.pending[0].0;
                    self.pending.clear();
                    if let Some(open) = self.open.take() {
                        self.done.push(close(&open));
                    }
                    self.open = Some(self.segment(first, bars));
                }
            }
        }
        if let Some(open) = self.open.as_mut() {
            // The hypotheses, and the regions of each, read the frame
            // independently, so in parallel: the calibration runs before
            // the analysis can start.
            std::thread::scope(|scope| {
                let mut own = None;
                for h in &mut open.hypotheses {
                    let Hypothesis {
                        sampler,
                        feed,
                        chat,
                    } = h;
                    let sampler = &*sampler;
                    scope.spawn(move || chat.read(sampler, frame));
                    if own.is_none() {
                        own = Some((sampler, feed));
                    } else {
                        scope.spawn(move || feed.read(sampler, frame));
                    }
                }
                if let Some((sampler, feed)) = own {
                    feed.read(sampler, frame);
                }
            });
        }
    }

    /// The calibration of every segment.
    pub fn finish(mut self) -> Calibration {
        if let Some(open) = self.open.take() {
            self.done.push(close(&open));
        }
        if self.done.is_empty() {
            // No frame was sampled: the frame size alone, not confirmed.
            self.done.push(CalibratedSegment {
                first_frame: 0,
                geometry: HudGeometry::for_layout(self.width, self.height, self.layout),
                hues: None,
            });
        }
        if let Some(first) = self.done.first_mut() {
            first.first_frame = 0;
        }
        Calibration {
            segments: self.done,
        }
    }

    /// A new segment from frame `first_frame` whose frames have black bars
    /// `bars`.
    fn segment(&self, first_frame: u32, bars: Bars) -> Open {
        let (w, h) = (self.width, self.height);
        let layouts = if self.layout == HudLayout::Auto {
            hypotheses(w, h, bars)
        } else {
            vec![self.layout]
        };
        let no_bars = bars.pillar == 0 && bars.letter == 0;
        let default = if self.layout == HudLayout::Auto && no_bars && is_16x9(w, h) {
            let layout = HudGeometry::for_layout(w, h, HudLayout::Auto).layout;
            layouts.iter().position(|&l| l == layout)
        } else {
            None
        };
        Open {
            first_frame,
            bars,
            default,
            hypotheses: layouts
                .into_iter()
                .map(|l| Hypothesis::new(HudGeometry::for_layout(w, h, l)))
                .collect(),
        }
    }
}

/// The layouts a frame of `width` × `height` pixels with black bars
/// `bars` can hold, the most likely first (a tie goes to the first). The
/// layouts without bars are always tried: a dark scene can pass for bars.
/// A 16:9 image scaled to a frame (or between pillarbox bars) of another
/// aspect ratio comes before the game's own layouts, so a tie goes to it:
/// the game renders a 16:9 image and fills (or boxes) other aspects with
/// it, and at 16:10 its kill feed lies within 2 px of the frame-size
/// layout's.
fn hypotheses(width: u32, height: u32, bars: Bars) -> Vec<HudLayout> {
    let mut layouts = Vec::with_capacity(6);
    let px = |bar: u32| u16::try_from(bar).unwrap_or(u16::MAX);
    if bars.letter > 0 {
        layouts.push(HudLayout::Letterbox {
            bar_px: px(bars.letter),
        });
    }
    let image_w = width.saturating_sub(2 * bars.pillar);
    if bars.pillar > 0 && !is_16x9(image_w, height) {
        layouts.push(HudLayout::Scaled16x9 {
            bar_px: px(bars.pillar),
        });
    }
    if bars.pillar > 0 {
        layouts.push(HudLayout::Pillarbox {
            bar_px: px(bars.pillar),
        });
    }
    if !is_16x9(width, height) {
        layouts.push(HudLayout::Scaled16x9 { bar_px: 0 });
    }
    layouts.push(HudGeometry::for_layout(width, height, HudLayout::Auto).layout);
    if width as f32 / height.max(1) as f32 > 1.5 {
        // A 4:3 game may be stretched to fill a wide frame.
        layouts.push(HudLayout::Stretched4x3);
    }
    layouts
}

/// Whether an image of `width` × `height` pixels is 16:9, so that a 16:9
/// game scaled into it is the game's own layout.
fn is_16x9(width: u32, height: u32) -> bool {
    (width as f32 / height.max(1) as f32 - 16.0 / 9.0).abs() < ASPECT_TOLERANCE
}

/// The confidence of a layout that aligned `wins` structures more than
/// another one where it aligned more, and `losses` fewer where the other
/// aligned more: its net lead, scaled down below `min_evidence`
/// structures.
fn confidence(wins: u32, losses: u32, min_evidence: u32) -> f32 {
    if wins == 0 {
        return 0.0;
    }
    let lead = wins.saturating_sub(losses) as f32 / wins as f32;
    lead * (wins as f32 / min_evidence.max(1) as f32).min(1.0)
}

/// Aligned structure per region: kill-feed rows and chat bands.
type Counts = [u32; 2];

/// The confidence of the counts `best` against the counts `other`.
fn lead_over(best: Counts, other: Counts, min_evidence: u32) -> f32 {
    let pairs = || best.iter().zip(&other);
    let wins = pairs().map(|(b, o)| b.saturating_sub(*o)).sum();
    let losses = pairs().map(|(b, o)| o.saturating_sub(*b)).sum();
    confidence(wins, losses, min_evidence)
}

/// The finished calibration of a segment: the hypothesis that aligned the
/// most structure, with its lead over every other one.
fn close(open: &Open) -> CalibratedSegment {
    // On a tie the more likely layout (listed first) wins.
    let best_index = open
        .hypotheses
        .iter()
        .enumerate()
        .min_by_key(|&(k, h)| (std::cmp::Reverse(h.evidence()), k))
        .map(|(k, _)| k);
    let Some(best_index) = best_index else {
        return CalibratedSegment {
            first_frame: open.first_frame,
            geometry: HudGeometry::default(),
            hues: None,
        };
    };
    let best = &open.hypotheses[best_index];
    let others = || {
        open.hypotheses
            .iter()
            .enumerate()
            .filter(move |&(k, _)| k != best_index)
            .map(|(_, h)| h)
    };
    let unopposed = others().all(|h| h.evidence() == 0);
    let min_evidence = if open.default == Some(best_index) && unopposed {
        1
    } else {
        MIN_EVIDENCE
    };
    let lead = |mask: Counts| -> f32 {
        let counts = |h: &Hypothesis| [h.feed.rows * mask[0], h.chat.bands * mask[1]];
        others()
            .map(|other| lead_over(counts(best), counts(other), min_evidence))
            .fold(lead_over(counts(best), [0, 0], min_evidence), f32::min)
    };
    let mut geometry = best.sampler.geometry().clone();
    geometry.confidence = lead([1, 1]);
    let mut regions = BTreeMap::new();
    regions.insert(Region::KillFeed, lead([1, 0]));
    regions.insert(Region::TextChat, lead([0, 1]));
    geometry.per_region.0 = regions;
    CalibratedSegment {
        first_frame: open.first_frame,
        geometry,
        hues: best.feed.hues.hues(&ClassParams::default()),
    }
}

/// Whether a kill-feed row with reference top `top`, the `k`-th row from
/// the top of its frame, lies in the `k`-th slot of the feed. The rows of
/// a wrong vertical mapping lie elsewhere (a 16:9 image between bars of
/// 60 px in a 1920 × 1200 frame, read as a 16:10 game, puts the first row
/// 44 px low), although they are found: the row finder searches the whole
/// feed crop.
fn in_slot(top: i32, k: usize) -> bool {
    let slot = i32::try_from(k).map_or(i32::MAX, |k| {
        FIRST_ROW_TOP.saturating_add(k.saturating_mul(ROW_PITCH))
    });
    (top - slot).abs() <= SLOT_TOLERANCE
}

/// Whether the kill-feed row at reference row `top` of the feed crop
/// `feed` ends at [`ROW_RIGHT`]: the strongest vertical edge of its band in
/// the columns around the row's right end lies there, located to a
/// fraction of a pixel. The rows of a wrong horizontal scale end
/// elsewhere (a 4:3 game stretched to 16:9 moves the end by 7 px, a 16:10
/// game read as a squeezed 16:9 image by 2 px), although they are found:
/// the row finder reads its columns with some tolerance.
fn row_end_aligned(feed: &RgbView<'_>, top: i32) -> bool {
    let rows = (top + ROW_EDGE_BAND.0 - FEED_ROI.y0).max(0) as u32
        ..((top + ROW_EDGE_BAND.1 - FEED_ROI.y0) as u32).min(feed.height());
    let level = |x: i32, y: u32| {
        let p = feed.pixel((x - FEED_ROI.x0) as u32, y);
        i32::from(p[0].max(p[1]).max(p[2]))
    };
    let edge = |x: i32| -> i32 {
        rows.clone()
            .map(|y| (level(x - 1, y) - level(x, y)).abs())
            .sum()
    };
    let window = ROW_RIGHT - ROW_EDGE_SEARCH..=ROW_RIGHT + ROW_EDGE_SEARCH;
    let (best, strength) = window
        .clone()
        .map(|x| (x, edge(x)))
        .max_by_key(|&(_, e)| e)
        .unwrap_or((0, 0));
    let mean = window.map(edge).sum::<i32>() / (2 * ROW_EDGE_SEARCH + 1);
    // The centroid of the edge strengths around the strongest edge.
    let around = || (best - EDGE_SPREAD..=best + EDGE_SPREAD).map(|x| (x, edge(x)));
    let total: i32 = around().map(|(_, e)| e).sum();
    let centroid = around().map(|(x, e)| x as f32 * e as f32).sum::<f32>() / total.max(1) as f32;
    (centroid - ROW_RIGHT as f32).abs() <= EDGE_TOLERANCE && strength >= EDGE_CONTRAST * mean.max(1)
}

/// Whether the chat box band `band` of the left-panel crop `left` has its
/// right edge where the layout puts it: inside the box (darkened) at most
/// [`BOX_EDGE_RATIO`] as bright as just outside it, at the closed box's or
/// the open panel's right edge.
fn box_edge_aligned(left: &RgbView<'_>, band: &BoxBand) -> bool {
    let rows = (band.y0 - LEFT_PANEL_ROI.y0).max(0) as u32
        ..((band.y1 - LEFT_PANEL_ROI.y0) as u32).min(left.height());
    let mean = |x0: i32, x1: i32| -> f32 {
        let mut sum = 0u32;
        let mut n = 0u32;
        for y in rows.clone() {
            for x in x0..x1 {
                let p = left.pixel((x - LEFT_PANEL_ROI.x0) as u32, y);
                sum += u32::from(p[0].max(p[1]).max(p[2]));
                n += 1;
            }
        }
        sum as f32 / n.max(1) as f32
    };
    [BOX_X.1, PANEL_X1].into_iter().any(|x| {
        let inside = mean(x - BOX_EDGE_SPAN.1, x - BOX_EDGE_SPAN.0);
        let outside = mean(x + BOX_EDGE_SPAN.0, x + BOX_EDGE_SPAN.1);
        outside >= MIN_OUTSIDE_LEVEL && inside <= BOX_EDGE_RATIO * outside
    })
}

/// The width of the black pillarbox bars of `frame`, or 0 if it has none.
fn pillar_bar(frame: &RgbView<'_>) -> u32 {
    let (w, h) = (frame.width(), frame.height());
    let black = |x: u32| {
        (0..h as usize).step_by(BAR_STEP).all(|y| {
            let p = frame.pixel(x, y as u32);
            p[0].max(p[1]).max(p[2]) <= BAR_MAX_LEVEL
        })
    };
    let half = w / 2;
    let left = (0..half).take_while(|&x| black(x)).count() as u32;
    let right = (0..half).take_while(|&x| black(w - 1 - x)).count() as u32;
    let bar = left.min(right);
    let wide_enough = bar as f32 >= MIN_BAR_SHARE * w as f32;
    if wide_enough && left.abs_diff(right) <= BAR_SYMMETRY && bar < half {
        bar
    } else {
        0
    }
}

/// The height of the black letterbox bars of `frame`, or 0 if it has
/// none.
fn letter_bar(frame: &RgbView<'_>) -> u32 {
    let (w, h) = (frame.width(), frame.height());
    let black = |y: u32| {
        (0..w as usize).step_by(BAR_STEP).all(|x| {
            let p = frame.pixel(x as u32, y);
            p[0].max(p[1]).max(p[2]) <= BAR_MAX_LEVEL
        })
    };
    let half = h / 2;
    let top = (0..half).take_while(|&y| black(y)).count() as u32;
    let bottom = (0..half).take_while(|&y| black(h - 1 - y)).count() as u32;
    let bar = top.min(bottom);
    let tall_enough = bar as f32 >= MIN_BAR_SHARE * h as f32;
    if tall_enough && top.abs_diff(bottom) <= BAR_SYMMETRY && bar < half {
        bar
    } else {
        0
    }
}
