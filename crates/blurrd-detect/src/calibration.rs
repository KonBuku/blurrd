//! Calibrating a video's HUD geometry from its frames (plan 7.3).
//!
//! The frame size alone does not say where the HUD is: a 16:9 recording
//! can hold a 16:9 game or a 4:3 game stretched to fill it, and a 4:3 game
//! can sit between black bars (a pillarbox). [`Calibrator`] decides from
//! sampled frames:
//!
//! 1. Black bars: columns at both frame edges that stay black in every
//!    row, of the same width on both sides, make a pillarbox.
//! 2. Every layout the frame size and the bars allow is a hypothesis
//!    ([`HudGeometry::for_layout`]). Each sampled frame is read through
//!    each hypothesis with the kill-feed and text-chat structure finders:
//!    kill-feed rows (victim text line, portrait, bar) and the chat box.
//!    Under a wrong mapping the right-aligned victim names, the portraits
//!    and the box edges miss their columns, so only the true layout finds
//!    them.
//! 3. The layout with the most frames with structure wins. Its confidence
//!    is its lead over the runner-up, scaled down while fewer than
//!    [`MIN_EVIDENCE`] frames showed structure; a region's confidence is
//!    the same for the frames that showed that region's structure.
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
use crate::killfeed::{BarHueLearner, BarHues, FEED_ROI, KillFeedDetector};

/// Calibration confidence below this means Safe geometry and a
/// `CalibrationLow` warning.
pub const MIN_CONFIDENCE: f32 = 0.5;

/// Frames with structure a layout needs for full confidence.
pub const MIN_EVIDENCE: u32 = 3;

/// A column is black if no pixel of it (every [`BAR_ROW_STEP`]th row)
/// is brighter than this (largest channel; video black is 16 in limited
/// range, a compressed black bar stays a few levels above it).
const BAR_MAX_LEVEL: u8 = 28;
/// Rows of a column checked for a black bar.
const BAR_ROW_STEP: usize = 4;
/// A pillarbox bar is at least this share of the frame width (a 4:3 image
/// in a 16:9 frame has bars of 12.5 %; thin black edges of a capture card
/// are not bars).
const MIN_BAR_SHARE: f32 = 0.02;
/// The bars on both sides differ by at most this many pixels.
const BAR_SYMMETRY: u32 = 4;
/// Consecutive samples with other bars that start a new segment.
const SEGMENT_SAMPLES: usize = 3;
/// Two bar widths within this many pixels are the same.
const BAR_TOLERANCE: u32 = 4;
/// The right end of every kill-feed row (reference x; the victim portrait
/// spans x 1838..1900).
const ROW_RIGHT: i32 = 1900;
/// Rows of a kill-feed row's band, from its top, measured for its end.
const ROW_EDGE_BAND: (i32, i32) = (4, 29);
/// Columns searched on each side of [`ROW_RIGHT`].
const ROW_EDGE_SEARCH: i32 = 12;
/// The row's end lies at most this far from [`ROW_RIGHT`].
const EDGE_TOLERANCE: u32 = 2;
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
    feed: KillFeedDetector,
    chat: ChatDetector,
    hues: BarHueLearner,
    /// Kill-feed rows whose right end lies where the layout puts it.
    feed_rows: u32,
    /// Chat box bands with lines whose right edge lies where the layout
    /// puts it.
    chat_bands: u32,
}

impl Hypothesis {
    fn new(geometry: HudGeometry) -> Self {
        Self {
            sampler: HudSampler::new(geometry),
            feed: KillFeedDetector::new(),
            chat: ChatDetector::new(),
            hues: BarHueLearner::new(),
            feed_rows: 0,
            chat_bands: 0,
        }
    }

    /// Aligned structure in all regions.
    const fn evidence(&self) -> u32 {
        self.feed_rows + self.chat_bands
    }

    /// Reads one frame through the hypothesis.
    fn read(&mut self, frame: &RgbView<'_>, crop: &mut HudCrop, dets: &mut Vec<Detection>) {
        let feed = self.sampler.crop(frame, FEED_ROI, HudAnchor::Right, crop);
        if self.feed.detect(&feed, dets).is_ok() {
            let rows = self.feed.rows().iter();
            self.feed_rows += rows.filter(|r| row_end_aligned(&feed, r.top)).count() as u32;
            self.hues.push(&self.feed);
        }
        let left = self
            .sampler
            .crop(frame, LEFT_PANEL_ROI, HudAnchor::Left, crop);
        if self.chat.detect(&left, dets).is_ok() {
            // Bands with chat lines: the box alone could be a dark patch of
            // the background.
            let lines = self.chat.lines();
            let bands = self.chat.bands().iter();
            self.chat_bands += bands
                .filter(|b| lines.iter().any(|l| l.band == **b) && box_edge_aligned(&left, b))
                .count() as u32;
        }
    }
}

/// One segment while it is being calibrated.
#[derive(Debug)]
struct Open {
    first_frame: u32,
    bar: u32,
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
    pending: Vec<(u32, u32)>,
    crop: HudCrop,
    dets: Vec<Detection>,
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
            crop: HudCrop::default(),
            dets: Vec::new(),
        }
    }

    /// Reads the sampled frame `index` (the whole frame). Frames must come
    /// in order; about one per second is enough.
    pub fn push(&mut self, index: u32, frame: &RgbView<'_>) {
        let bar = pillar_bar(frame);
        let same = |open: &Open| open.bar.abs_diff(bar) <= BAR_TOLERANCE;
        match self.open.as_ref() {
            None => self.open = Some(self.segment(index, bar)),
            Some(open) if same(open) => self.pending.clear(),
            Some(_) => {
                self.pending.push((index, bar));
                let settled = self.pending.len() >= SEGMENT_SAMPLES
                    && self
                        .pending
                        .iter()
                        .all(|&(_, b)| b.abs_diff(bar) <= BAR_TOLERANCE);
                if settled {
                    let first = self.pending[0].0;
                    self.pending.clear();
                    if let Some(open) = self.open.take() {
                        self.done.push(close(&open));
                    }
                    self.open = Some(self.segment(first, bar));
                }
            }
        }
        if let Some(open) = self.open.as_mut() {
            for h in &mut open.hypotheses {
                h.read(frame, &mut self.crop, &mut self.dets);
            }
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

    /// A new segment from frame `first_frame` whose frames have pillarbox
    /// bars `bar` pixels wide (0 for none).
    fn segment(&self, first_frame: u32, bar: u32) -> Open {
        let (w, h) = (self.width, self.height);
        let layouts = if self.layout == HudLayout::Auto {
            hypotheses(w, h, bar)
        } else {
            vec![self.layout]
        };
        Open {
            first_frame,
            bar,
            hypotheses: layouts
                .into_iter()
                .map(|l| Hypothesis::new(HudGeometry::for_layout(w, h, l)))
                .collect(),
        }
    }
}

/// The layouts a frame of `width` × `height` pixels whose frames show
/// pillarbox bars of `bar` pixels (0 for none) can hold, the most likely
/// first. The layouts without bars are always tried: a dark scene can pass
/// for bars. A 16:9 image scaled to a frame (or between bars) narrower
/// than 3:2 is tried after the game's own layouts; at wider aspects, such
/// as 16:10, its HUD lies within the calibration's tolerance of the game's
/// own layout and the two could not be told apart.
fn hypotheses(width: u32, height: u32, bar: u32) -> Vec<HudLayout> {
    let mut layouts = Vec::with_capacity(5);
    let bar_px = u16::try_from(bar).unwrap_or(u16::MAX);
    if bar > 0 {
        layouts.push(HudLayout::Pillarbox { bar_px });
    }
    layouts.push(HudGeometry::for_layout(width, height, HudLayout::Auto).layout);
    if width as f32 / height.max(1) as f32 > 1.5 {
        // A 4:3 game may be stretched to fill a wide frame.
        layouts.push(HudLayout::Stretched4x3);
    }
    if bar > 0 && is_narrow(width.saturating_sub(2 * bar), height) {
        layouts.push(HudLayout::Scaled16x9 { bar_px });
    }
    if is_narrow(width, height) {
        layouts.push(HudLayout::Scaled16x9 { bar_px: 0 });
    }
    layouts
}

/// Whether an image of `width` × `height` pixels is narrower than 3:2, so
/// that a 16:9 game scaled into it has a HUD of its own.
fn is_narrow(width: u32, height: u32) -> bool {
    width as f32 / height.max(1) as f32 <= 1.5
}

/// The confidence of a layout that found `names` names where the
/// runner-up found `other`: its lead, scaled down below [`MIN_EVIDENCE`]
/// names.
fn confidence(names: u32, other: u32) -> f32 {
    if names == 0 {
        return 0.0;
    }
    let lead = names.saturating_sub(other) as f32 / names as f32;
    lead * (names as f32 / MIN_EVIDENCE as f32).min(1.0)
}

/// The finished calibration of a segment: the hypothesis that found the
/// most names.
fn close(open: &Open) -> CalibratedSegment {
    let mut ranked: Vec<&Hypothesis> = open.hypotheses.iter().collect();
    // Stable: on a tie the more likely layout (listed first) stays first.
    ranked.sort_by_key(|h| std::cmp::Reverse(h.evidence()));
    let Some(best) = ranked.first() else {
        return CalibratedSegment {
            first_frame: open.first_frame,
            geometry: HudGeometry::default(),
            hues: None,
        };
    };
    let second = ranked.get(1);
    let mut geometry = best.sampler.geometry().clone();
    geometry.confidence = confidence(best.evidence(), second.map_or(0, |h| h.evidence()));
    let mut regions = BTreeMap::new();
    regions.insert(
        Region::KillFeed,
        confidence(best.feed_rows, second.map_or(0, |h| h.feed_rows)),
    );
    regions.insert(
        Region::TextChat,
        confidence(best.chat_bands, second.map_or(0, |h| h.chat_bands)),
    );
    geometry.per_region.0 = regions;
    CalibratedSegment {
        first_frame: open.first_frame,
        geometry,
        hues: best.hues.hues(&ClassParams::default()),
    }
}

/// Whether the kill-feed row at reference row `top` of the feed crop
/// `feed` ends at [`ROW_RIGHT`]: the strongest vertical edge of its band in
/// the columns around the row's right end lies there. The rows of a wrong
/// horizontal scale end elsewhere (a 4:3 game stretched to 16:9 moves the
/// end by 7 px), although they are found: the row finder reads its
/// columns with some tolerance.
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
    best.abs_diff(ROW_RIGHT) <= EDGE_TOLERANCE && strength >= EDGE_CONTRAST * mean.max(1)
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
        (0..h as usize).step_by(BAR_ROW_STEP).all(|y| {
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
