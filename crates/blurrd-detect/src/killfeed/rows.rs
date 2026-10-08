//! Finding the kill-feed rows of a frame (port spec 3.1).

use super::{FEED_ROI, FEED_Y_TOP, FeedRow, ROW_H, RowAnchor, SCORE_X, VICTIM_TEXT_X1};
use crate::classes::{Class, PixelClasses};
use crate::components::runs;

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

/// Finds the rows of the kill-feed stack, top to bottom.
///
/// `classes` must hold the classified score columns over the whole crop
/// height. A row is a 33 px band whose score columns are mostly bar
/// colour, with edges above and below; the right-aligned victim text line
/// anchors rows whose edges a wall in the bar colour hides. The rows must
/// form a stack from the top.
pub(super) fn find_rows(classes: &PixelClasses, s: &mut RowScratch, out: &mut Vec<FeedRow>) {
    out.clear();
    let width = classes.width() as usize;
    let n = classes.height() as i32;
    let (sx0, sx1) = (
        (SCORE_X.0 - FEED_ROI.x0) as usize,
        (SCORE_X.1 - FEED_ROI.x0) as usize,
    );
    let (ax0, ax1) = (
        (ANCHOR_X.0 - FEED_ROI.x0) as usize,
        (ANCHOR_X.1 - FEED_ROI.x0) as usize,
    );
    let columns = (sx1 - sx0) as f64;
    let bar = Class::Bar.bit();
    let covered = bar | Class::White.bit();
    let text = Class::Text.bit();
    s.sums.clear();
    s.sums.push(0.0);
    s.text_rows.clear();
    let mut acc = 0.0;
    for row in classes.flags().chunks_exact(width) {
        let score = &row[sx0..sx1];
        let fb = score.iter().filter(|&&f| f & bar != 0).count() as f64 / columns;
        let fa = score.iter().filter(|&&f| f & covered != 0).count() as f64 / columns;
        acc += if fa >= MIN_COVERED { fb } else { 0.0 };
        s.sums.push(acc);
        let strokes = row[ax0..ax1].iter().filter(|&&f| f & text != 0).count();
        s.text_rows.push(strokes >= ANCHOR_MIN_STROKES);
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
    for run in runs(s.text_rows.iter().copied()) {
        if run.len() < ANCHOR_RUN.0 || run.len() > ANCHOR_RUN.1 {
            continue;
        }
        let y = run.start as i32 - ANCHOR_TO_TOP;
        if y < FEED_Y_TOP - FEED_ROI.y0 - 2 || y + ROW_H > n {
            continue;
        }
        let y = y.max(FEED_Y_TOP - FEED_ROI.y0);
        if s.mean(y, y + ROW_H) >= MIN_ANCHOR_INNER && far_from(&s.tops, y) {
            s.tops.push(y);
            s.anchored.push(y);
        }
    }
    s.candidates
        .sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.cmp(&a.1)));
    for &(_, y) in &s.candidates {
        if far_from(&s.tops, y) {
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

/// Whether `y` is at least [`MIN_ROW_DISTANCE`] from every accepted row.
fn far_from(tops: &[i32], y: i32) -> bool {
    tops.iter().all(|&t| (y - t).abs() >= MIN_ROW_DISTANCE)
}
