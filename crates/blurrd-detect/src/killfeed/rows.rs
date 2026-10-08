//! Finding the kill-feed rows of a frame (port spec 3.1).
//!
//! Two signals find rows. The text line of the right-aligned victim names
//! (white strokes in fixed columns) does not depend on colour at all. The
//! bar score of each pixel row, the share of bar-coloured pixels in the
//! score columns, needs the bar colour: the default teal and red first,
//! and where a text line sits on a bar the defaults do not explain, the
//! colour learned from that bar ([`tones`]).

use blurrd_core::RectI;

use super::tones::{self, Sample, Tone};
use super::{
    FEED_ROI, FEED_Y_TOP, FeedRow, INNER_MARGIN, ROW_H, RowAnchor, SCORE_X, VICTIM_COLOUR_X0,
    VICTIM_TEXT_X1,
};
use crate::classes::{Class, PixelClasses};
use crate::components::runs;
use crate::image::RgbView;

/// Minimum mean row score inside a bar-edge candidate.
const MIN_INNER: f64 = 0.5;
/// Minimum mean score of every 4-row window inside a candidate: the 6 px
/// gap between two stacked rows and a faded row fall below it.
const MIN_HOLE: f64 = 0.15;
/// Minimum contrast between a candidate's inside and the 4 rows just
/// outside it.
const MIN_EDGE: f64 = 0.22;
/// Minimum mean row score of a text-line anchor's band.
const MIN_ANCHOR_INNER: f64 = 0.45;
/// A row counts only if at least this share of its score columns is bar or
/// white.
const MIN_COVERED: f64 = 0.6;
/// Rows closer than this are the same row.
const MIN_ROW_DISTANCE: i32 = 30;
/// The topmost row of the stack starts at or above this line.
const MAX_FIRST_TOP: i32 = 182;
/// Rows of the stack follow each other at most this far apart (pitch 39).
const MAX_STACK_STEP: i32 = 46;
/// Columns of the victim text line used as an anchor.
const ANCHOR_X: (i32, i32) = (1790, VICTIM_TEXT_X1);
/// Minimum text-stroke pixels per crop row in the anchor columns.
const ANCHOR_MIN_STROKES: usize = 2;
/// Length range of a run of text rows that anchors a row.
const ANCHOR_RUN: (u32, u32) = (6, 17);
/// The row top lies this far above the start of its text line.
const ANCHOR_TO_TOP: i32 = 11;
/// Columns of the victim portrait (x 1838..1900, without its slanted
/// edges) checked by the wall guard.
const PORTRAIT: (i32, i32) = (1842, 1896);
/// Rows of the portrait below the row top checked by the wall guard.
const PORTRAIT_ROWS: (i32, i32) = (3, 30);
/// Minimum mean absolute difference of `min(R, G, B)` between neighbouring
/// portrait pixels. Measured on the parity excerpts: rows with names
/// 7.3 to 25 (1st percentile 14.8), bar-edge rows without names (empty
/// cover bars, walls) 0 to 4.1 (95th percentile).
const MIN_PORTRAIT_TEXTURE: f32 = 6.0;
/// A bar colour is learned from a text line's band if its tone covers at
/// least this share of the non-white pixels next to the name...
const TONE_SHARE: f32 = 0.6;
/// ... and at least this many pixels (of about 1,100).
const TONE_PIXELS: u32 = 300;
/// The killer colour must cover at least this share of the coloured
/// pixels in the score columns left of the victim area that do not have
/// the victim colour...
const KILLER_TONE_SHARE: f32 = 0.35;
/// ... and at least this many pixels (of about 3,250).
const KILLER_TONE_PIXELS: u32 = 100;
/// At most this many bar colours are learned per frame (two teams, plus
/// room for a wall that passes as a bar).
const MAX_TONES: usize = 4;

/// Reusable buffers of [`find_rows`].
#[derive(Debug, Clone, Default)]
pub(super) struct RowScratch {
    /// Prefix sums of the row score `q`.
    sums: Vec<f64>,
    /// Bar-edge candidates `(score, crop row)`.
    candidates: Vec<(f64, i32)>,
    /// Whether each crop row has enough text strokes in the anchor
    /// columns.
    text_rows: Vec<bool>,
    /// Accepted row tops in crop rows.
    tops: Vec<i32>,
    /// The tops that came from text-line anchors.
    anchored: Vec<i32>,
    /// Text-line anchors (crop rows, clamped) before acceptance.
    lines: Vec<i32>,
}

impl RowScratch {
    /// Mean row score over crop rows `a..b`, clipped to the crop, as the
    /// prototype computes it (`(cs[b] − cs[a]) / max(1, b − a)`).
    fn mean(&self, a: i32, b: i32) -> f64 {
        let n = self.sums.len() as i32 - 1;
        let (a, b) = (a.max(0), b.min(n));
        (self.sums[b as usize] - self.sums[a as usize]) / f64::from((b - a).max(1))
    }
}

/// Finds the rows of the kill-feed stack, top to bottom, and the bar
/// colours learned on the way.
///
/// `classes` must hold the classified score columns over the whole crop
/// height. A row is a 33 px band whose score columns are mostly bar
/// colour, with edges above and below; the right-aligned victim text line
/// anchors rows whose edges a wall in the bar colour hides. The rows must
/// form a stack from the top.
pub(super) fn find_rows(
    crop: &RgbView<'_>,
    classes: &PixelClasses,
    s: &mut RowScratch,
    tones: &mut Vec<Tone>,
    out: &mut Vec<FeedRow>,
) {
    out.clear();
    tones.clear();
    let n = classes.height() as i32;
    score_rows(classes, tones, s);
    // Text-line anchors, before any colour check.
    s.lines.clear();
    for run in runs(s.text_rows.iter().copied()) {
        if run.len() < ANCHOR_RUN.0 || run.len() > ANCHOR_RUN.1 {
            continue;
        }
        let y = run.start as i32 - ANCHOR_TO_TOP;
        if y < FEED_Y_TOP - FEED_ROI.y0 - 2 || y + ROW_H > n {
            continue;
        }
        s.lines.push(y.max(FEED_Y_TOP - FEED_ROI.y0));
    }
    // A text line on a bar the default colours do not explain: learn the
    // bar's colour from the pixels around the name and score again.
    for &y in &s.lines {
        if s.mean(y, y + ROW_H) < MIN_ANCHOR_INNER {
            learn_row_tones(classes, y + FEED_ROI.y0, tones);
        }
    }
    if !tones.is_empty() {
        score_rows(classes, tones, s);
    }
    // Bar-edge candidates.
    s.candidates.clear();
    for y in FEED_Y_TOP - FEED_ROI.y0..n - ROW_H {
        let inner = s.mean(y, y + ROW_H);
        if inner < MIN_INNER {
            continue;
        }
        let holed = (y + 2..y + ROW_H - 5).any(|k| s.mean(k, k + 4) < MIN_HOLE);
        if holed {
            continue;
        }
        let outer = f64::midpoint(s.mean(y - 4, y), s.mean(y + ROW_H, y + ROW_H + 4));
        let score = inner - outer;
        if score >= MIN_EDGE {
            s.candidates.push((score, y));
        }
    }
    // Text-line anchors first: they do not depend on the background.
    s.tops.clear();
    s.anchored.clear();
    for i in 0..s.lines.len() {
        let y = s.lines[i];
        if s.mean(y, y + ROW_H) >= MIN_ANCHOR_INNER && far_from(&s.tops, y) {
            s.tops.push(y);
            s.anchored.push(y);
        }
    }
    s.candidates
        .sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.cmp(&a.1)));
    // Wall guard: a bar-edge candidate without a text line must show the
    // victim portrait. A wall in a bar colour behind the feed has edges
    // too, but no portrait art.
    for &(_, y) in &s.candidates {
        if far_from(&s.tops, y) && has_portrait(crop, y + FEED_ROI.y0) {
            s.tops.push(y);
        }
    }
    s.tops.sort_unstable();
    // The feed is a stack from the top.
    for &y in &s.tops {
        let top = y + FEED_ROI.y0;
        let fits = match out.last() {
            None => top <= MAX_FIRST_TOP,
            Some(last) => top - last.top <= MAX_STACK_STEP,
        };
        if fits {
            let anchor = if s.anchored.contains(&y) {
                RowAnchor::TextLine
            } else {
                RowAnchor::BarEdge
            };
            out.push(FeedRow { top, anchor });
        }
    }
}

/// Learns the colours of the row at reference row `top`: the victim's
/// from the bar next to the name, then the killer's from the score
/// columns left of it (they show the killer side when the victim name is
/// short). A colour already known widens the known tone.
fn learn_row_tones(classes: &PixelClasses, top: i32, tones: &mut Vec<Tone>) {
    let band = |x0: i32, x1: i32| RectI {
        x0,
        y0: top + INNER_MARGIN,
        x1,
        y1: top + ROW_H - INNER_MARGIN,
    };
    let victim = Sample {
        rect: band(VICTIM_COLOUR_X0, SCORE_X.1),
        exclude: None,
        min_share: TONE_SHARE,
        min_pixels: TONE_PIXELS,
        count_grey: true,
    };
    let Some(victim) = tones::learn(classes, &victim) else {
        return;
    };
    add_tone(tones, victim);
    let killer = Sample {
        rect: band(SCORE_X.0, VICTIM_COLOUR_X0),
        exclude: Some(victim),
        min_share: KILLER_TONE_SHARE,
        min_pixels: KILLER_TONE_PIXELS,
        count_grey: false,
    };
    if let Some(killer) = tones::learn(classes, &killer)
        && killer.hue_gap(victim) >= tones::MIN_TEAM_HUE_GAP
    {
        add_tone(tones, killer);
    }
}

/// Adds `tone` to `tones`, or widens the known tone of its colour.
fn add_tone(tones: &mut Vec<Tone>, tone: Tone) {
    if let Some(known) = tones.iter_mut().find(|t| t.same_colour(tone)) {
        *known = known.widened(tone);
    } else if tones.len() < MAX_TONES {
        tones.push(tone);
    }
}

/// Scores every crop row: the prefix sums of `q` (the share of bar pixels
/// in the score columns, 0 unless at least [`MIN_COVERED`] of them are bar
/// or white) and whether the row has text strokes in the anchor columns.
/// A pixel is a bar pixel if it has the default bar class or one of
/// `tones`.
fn score_rows(classes: &PixelClasses, tones: &[Tone], s: &mut RowScratch) {
    let width = classes.width() as usize;
    let (sx0, sx1) = (
        (SCORE_X.0 - FEED_ROI.x0) as usize,
        (SCORE_X.1 - FEED_ROI.x0) as usize,
    );
    let (ax0, ax1) = (
        (ANCHOR_X.0 - FEED_ROI.x0) as usize,
        (ANCHOR_X.1 - FEED_ROI.x0) as usize,
    );
    let columns = (sx1 - sx0) as f64;
    let (bar, white, text) = (Class::Bar.bit(), Class::White.bit(), Class::Text.bit());
    s.sums.clear();
    s.sums.push(0.0);
    s.text_rows.clear();
    let mut acc = 0.0;
    let score = RectI {
        x0: SCORE_X.0,
        y0: FEED_ROI.y0,
        x1: SCORE_X.1,
        y1: FEED_ROI.y1,
    };
    let rows = classes.flags().chunks_exact(width);
    for (row, planes) in rows.zip(tones::plane_rows(classes, score)) {
        let (mut bars, mut covered) = (0u32, 0u32);
        for (i, &f) in planes.flags.iter().enumerate() {
            let is_bar = f & bar != 0
                || (!tones.is_empty() && {
                    let (h, s, v) = (planes.hue[i], planes.sat[i], planes.value(i));
                    tones.iter().any(|t| t.matches(h, s, v))
                });
            bars += u32::from(is_bar);
            covered += u32::from(is_bar || f & white != 0);
        }
        let (fb, fa) = (f64::from(bars) / columns, f64::from(covered) / columns);
        acc += if fa >= MIN_COVERED { fb } else { 0.0 };
        s.sums.push(acc);
        let strokes = row[ax0..ax1].iter().filter(|&&f| f & text != 0).count();
        s.text_rows.push(strokes >= ANCHOR_MIN_STROKES);
    }
}

/// Whether the row at reference row `top` has the textured art of a
/// victim portrait: the mean absolute difference of `min(R, G, B)` between
/// neighbouring pixels of [`PORTRAIT`] is at least
/// [`MIN_PORTRAIT_TEXTURE`].
fn has_portrait(crop: &RgbView<'_>, top: i32) -> bool {
    let (x0, x1) = (
        (PORTRAIT.0 - FEED_ROI.x0) as usize,
        (PORTRAIT.1 - FEED_ROI.x0) as usize,
    );
    let rows =
        (top + PORTRAIT_ROWS.0 - FEED_ROI.y0) as u32..(top + PORTRAIT_ROWS.1 - FEED_ROI.y0) as u32;
    let mn = |row: &[u8]| -> Vec<i16> {
        row.as_chunks::<3>().0[x0..x1]
            .iter()
            .map(|p| i16::from(p[0].min(p[1]).min(p[2])))
            .collect()
    };
    let (mut sum, mut count) = (0u32, 0u32);
    let mut above: Option<Vec<i16>> = None;
    for y in rows {
        if y >= crop.height() {
            return false;
        }
        let row = mn(crop.row(y));
        for pair in row.windows(2) {
            sum += u32::from(pair[0].abs_diff(pair[1]));
        }
        count += row.len().saturating_sub(1) as u32;
        if let Some(prev) = &above {
            for (a, b) in prev.iter().zip(&row) {
                sum += u32::from(a.abs_diff(*b));
            }
            count += row.len() as u32;
        }
        above = Some(row);
    }
    count > 0 && sum as f32 >= MIN_PORTRAIT_TEXTURE * count as f32
}

/// Whether `y` is at least [`MIN_ROW_DISTANCE`] from every accepted row.
fn far_from(tops: &[i32], y: i32) -> bool {
    tops.iter().all(|&t| (y - t).abs() >= MIN_ROW_DISTANCE)
}
