//! Finding the kill-feed rows of a frame (port spec 3.1).
//!
//! Two signals find rows. The text line of the right-aligned victim names
//! (white strokes in fixed columns) does not depend on hue; its brightness
//! thresholds follow the white level of the frame ([`white_level`]). The
//! bar score of each pixel row, the share of bar-coloured pixels in the
//! score columns, needs the bar colour: the default teal and red first,
//! and where a text line sits on a bar the defaults do not explain, the
//! colour learned from that bar ([`tones`]). A text line on a bar without
//! a learnable colour (greyscale or nearly greyscale video) is accepted
//! from luminance structure instead: a vertically flat band next to a
//! textured victim portrait ([`BarEvidence::Luminance`]).

use blurrd_core::RectI;

use super::tones::{self, Sample, Tone};
use super::{
    BarEvidence, FEED_ROI, FEED_Y_TOP, FeedRow, INNER_MARGIN, ROW_H, ROW_PITCH, RowAnchor, SCORE_X,
    VICTIM_COLOUR_X0, VICTIM_TEXT_X1,
};
use crate::classes::{Class, ClassParams, PixelClasses};
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
/// Two vertically neighbouring bar pixels differ in value by at most this
/// much ...
const FLAT_STEP: u8 = 4;
/// ... in at least this share of the neighbouring pairs of a luminance
/// anchor's inner band (text pixels left out). Measured on greyscale
/// versions of the parity excerpts: rows with names 0.80 and more (1st
/// percentile), the bright sky and walls behind the victim text columns
/// that pass as a text line 0.63 to 0.70.
const MIN_FLAT: f32 = 0.75;
/// A luminance anchor whose band is less flat, down to this share, is
/// accepted only where it stacks on another row: another accepted row, or
/// another such anchor, one [`ROW_PITCH`] away. Measured on downscaled
/// greyscale recordings (720p, scaled back up): the soft edges of the
/// weapon icon, the victim name and of a death-recap panel overlapping
/// the row lower real bands to 0.57 to 0.74 (5th percentile 0.735).
const MIN_FLAT_STACKED: f32 = 0.5;
/// Two rows stack if their tops are one [`ROW_PITCH`] apart, within this
/// tolerance (text-line anchors land up to two rows off).
const STACK_TOLERANCE: i32 = 3;
/// A luminance anchor's band needs at least this many such pairs (of
/// about 4,250).
const MIN_FLAT_PAIRS: u32 = 200;
/// Pixels with at least this top-hat response (thin bright strokes) and
/// text chroma measure the white level.
const WHITE_SAMPLE_TOPHAT: u8 = 20;
/// The white level is the value of the 40th brightest such pixel: a
/// single short victim name has a few hundred stroke pixels in the anchor
/// columns, isolated specular highlights a few.
const WHITE_RANK: u32 = 40;
/// A white level below this is no white text (a dark scene without a kill
/// feed): the thresholds stay as they are.
const MIN_WHITE_LEVEL: u8 = 150;

/// Reusable buffers of [`find_rows`].
#[derive(Debug, Clone, Default)]
pub(super) struct RowScratch {
    /// Prefix sums of the row score `q`.
    sums: Vec<f64>,
    /// The same with the default bar classes only, when colours were
    /// learned (`sums` then includes them).
    default_sums: Vec<f64>,
    /// Whether `sums` includes learned colours.
    learned: bool,
    /// Bar-edge candidates `(score, crop row)`.
    candidates: Vec<(f64, i32)>,
    /// Whether each crop row has enough text strokes in the anchor
    /// columns.
    text_rows: Vec<bool>,
    /// Accepted rows with their top in crop rows.
    tops: Vec<(i32, RowAnchor, BarEvidence)>,
    /// Text-line anchors (crop rows, clamped) before acceptance.
    lines: Vec<i32>,
    /// Luminance anchors that need a stacked row ([`MIN_FLAT_STACKED`]).
    stacked: Vec<i32>,
    /// `min(R, G, B)` of two neighbouring portrait rows (wall guard).
    portrait: [Vec<u8>; 2],
}

impl RowScratch {
    /// Mean row score over crop rows `a..b`, clipped to the crop, as the
    /// prototype computes it (`(cs[b] − cs[a]) / max(1, b − a)`).
    fn mean(&self, a: i32, b: i32) -> f64 {
        mean_of(&self.sums, a, b)
    }

    /// Which colours make the band at crop row `y` a bar, given that all
    /// known colours give it a mean score of at least `min`.
    fn colour_evidence(&self, y: i32, min: f64) -> BarEvidence {
        if !self.learned || mean_of(&self.default_sums, y, y + ROW_H) >= min {
            BarEvidence::DefaultColour
        } else {
            BarEvidence::LearnedColour
        }
    }
}

/// Mean row score over crop rows `a..b` from prefix sums, clipped to the
/// crop.
fn mean_of(sums: &[f64], a: i32, b: i32) -> f64 {
    let n = sums.len() as i32 - 1;
    let (a, b) = (a.max(0), b.min(n));
    (sums[b as usize] - sums[a as usize]) / f64::from((b - a).max(1))
}

/// The white point of the frame's HUD text, `(level, chroma)`: the value
/// of the [`WHITE_RANK`]th brightest thin bright stroke pixel in the
/// victim text columns, and the median chroma of the stroke pixels at that
/// level or brighter; `None` if there is no white text there.
///
/// `classes` must hold the classified score columns over the whole crop
/// height.
pub(super) fn white_point(classes: &PixelClasses, params: &ClassParams) -> Option<(u8, u8)> {
    let width = classes.width() as usize;
    let (x0, x1) = (
        (ANCHOR_X.0 - FEED_ROI.x0) as usize,
        (ANCHOR_X.1 - FEED_ROI.x0) as usize,
    );
    let mut hist = [0u32; 256];
    for y in 0..classes.height() as usize {
        let range = y * width + x0..y * width + x1;
        let pixels = classes.tophat().data()[range.clone()]
            .iter()
            .zip(&classes.mn().data()[range.clone()])
            .zip(&classes.chroma().data()[range]);
        for ((&tophat, &mn), &chroma) in pixels {
            if tophat >= WHITE_SAMPLE_TOPHAT && chroma <= params.text_max_chroma {
                hist[usize::from(mn)] += 1;
            }
        }
    }
    let mut seen = 0;
    let mut found = None;
    for (level, &n) in (0..=255u8).zip(&hist).rev() {
        seen += n;
        if seen >= WHITE_RANK {
            found = (level >= MIN_WHITE_LEVEL).then_some(level);
            break;
        }
    }
    let level = found?;
    // The chroma of those strokes: the median over the pixels at the white
    // level or brighter.
    let mut chromas = [0u32; 256];
    for y in 0..classes.height() as usize {
        let range = y * width + x0..y * width + x1;
        let pixels = classes.tophat().data()[range.clone()]
            .iter()
            .zip(&classes.mn().data()[range.clone()])
            .zip(&classes.chroma().data()[range]);
        for ((&tophat, &mn), &chroma) in pixels {
            if tophat >= WHITE_SAMPLE_TOPHAT && chroma <= params.text_max_chroma && mn >= level {
                chromas[usize::from(chroma)] += 1;
            }
        }
    }
    let total: u32 = chromas.iter().sum();
    let mut acc = 0;
    let chroma = (0..=255u8)
        .zip(&chromas)
        .find(|&(_, &n)| {
            acc += n;
            2 * acc >= total
        })
        .map_or(0, |(c, _)| c);
    Some((level, chroma))
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
    s.learned = !tones.is_empty();
    if s.learned {
        std::mem::swap(&mut s.sums, &mut s.default_sums);
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
    // Text-line anchors first: they do not depend on the background. A
    // band without a known bar colour still counts if its luminance looks
    // like a bar next to a portrait.
    s.tops.clear();
    s.stacked.clear();
    for i in 0..s.lines.len() {
        let y = s.lines[i];
        if !far_from(&s.tops, y) {
            continue;
        }
        let evidence = if s.mean(y, y + ROW_H) >= MIN_ANCHOR_INNER {
            s.colour_evidence(y, MIN_ANCHOR_INNER)
        } else {
            match flatness(classes, y) {
                Some(flat)
                    if flat >= MIN_FLAT_STACKED
                        && has_portrait(crop, y + FEED_ROI.y0, &mut s.portrait) =>
                {
                    if flat < MIN_FLAT {
                        s.stacked.push(y);
                        continue;
                    }
                    BarEvidence::Luminance
                }
                _ => continue,
            }
        };
        s.tops.push((y, RowAnchor::TextLine, evidence));
    }
    s.candidates
        .sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.cmp(&a.1)));
    // Wall guard: a bar-edge candidate without a text line must show the
    // victim portrait. A wall in a bar colour behind the feed has edges
    // too, but no portrait art.
    for &(_, y) in &s.candidates {
        if far_from(&s.tops, y) && has_portrait(crop, y + FEED_ROI.y0, &mut s.portrait) {
            let evidence = s.colour_evidence(y, MIN_INNER);
            s.tops.push((y, RowAnchor::BarEdge, evidence));
        }
    }
    accept_stacked(s);
    s.tops.sort_unstable_by_key(|&(y, ..)| y);
    // The feed is a stack from the top.
    for &(y, anchor, evidence) in &s.tops {
        let top = y + FEED_ROI.y0;
        let fits = match out.last() {
            None => top <= MAX_FIRST_TOP,
            Some(last) => top - last.top <= MAX_STACK_STEP,
        };
        if fits {
            out.push(FeedRow {
                top,
                anchor,
                evidence,
            });
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

/// Accepts the luminance anchors of [`RowScratch::stacked`] that stack on
/// an accepted row or on each other, until no more are accepted.
fn accept_stacked(s: &mut RowScratch) {
    let stacks = |a: i32, b: i32| ((a - b).abs() - ROW_PITCH).abs() <= STACK_TOLERANCE;
    loop {
        let found = s.stacked.iter().position(|&y| {
            far_from(&s.tops, y)
                && (s.tops.iter().any(|&(t, ..)| stacks(t, y))
                    || s.stacked.iter().any(|&z| z != y && stacks(z, y)))
        });
        let Some(i) = found else {
            return;
        };
        let y = s.stacked.swap_remove(i);
        s.tops
            .push((y, RowAnchor::TextLine, BarEvidence::Luminance));
    }
}

/// How flat the band of the row at crop row `y` is in luminance, as an
/// opaque bar is: the share of the vertically neighbouring pixel pairs of
/// its inner band in the score columns that differ in value by at most
/// [`FLAT_STEP`]; `None` with fewer than [`MIN_FLAT_PAIRS`] pairs. Text
/// pixels (the name) are left out; white pixels are not, because a light
/// bar passes as white in greyscale video.
fn flatness(classes: &PixelClasses, y: i32) -> Option<f32> {
    let inner = RectI {
        x0: SCORE_X.0,
        y0: y + FEED_ROI.y0 + INNER_MARGIN,
        x1: SCORE_X.1,
        y1: y + FEED_ROI.y0 + ROW_H - INNER_MARGIN,
    };
    let skip = Class::Text.bit();
    let (mut pairs, mut flat) = (0u32, 0u32);
    let mut rows = tones::plane_rows(classes, inner);
    let mut above = rows.next()?;
    for row in rows {
        for i in 0..row.flags.len() {
            if (above.flags[i] | row.flags[i]) & skip == 0 {
                pairs += 1;
                flat += u32::from(above.value(i).abs_diff(row.value(i)) <= FLAT_STEP);
            }
        }
        above = row;
    }
    (pairs >= MIN_FLAT_PAIRS).then(|| flat as f32 / pairs as f32)
}

/// Whether the row at reference row `top` has the textured art of a
/// victim portrait: the mean absolute difference of `min(R, G, B)` between
/// neighbouring pixels of [`PORTRAIT`] is at least
/// [`MIN_PORTRAIT_TEXTURE`]. `buffers` hold two portrait rows.
fn has_portrait(crop: &RgbView<'_>, top: i32, buffers: &mut [Vec<u8>; 2]) -> bool {
    let (x0, x1) = (
        (PORTRAIT.0 - FEED_ROI.x0) as usize,
        (PORTRAIT.1 - FEED_ROI.x0) as usize,
    );
    let rows =
        (top + PORTRAIT_ROWS.0 - FEED_ROI.y0) as u32..(top + PORTRAIT_ROWS.1 - FEED_ROI.y0) as u32;
    let [above, row] = buffers;
    above.clear();
    let (mut sum, mut count) = (0u32, 0u32);
    for y in rows {
        if y >= crop.height() {
            return false;
        }
        row.clear();
        row.extend(
            crop.row(y).as_chunks::<3>().0[x0..x1]
                .iter()
                .map(|p| p[0].min(p[1]).min(p[2])),
        );
        for pair in row.windows(2) {
            sum += u32::from(pair[0].abs_diff(pair[1]));
        }
        count += row.len().saturating_sub(1) as u32;
        if !above.is_empty() {
            for (a, b) in above.iter().zip(row.iter()) {
                sum += u32::from(a.abs_diff(*b));
            }
            count += row.len() as u32;
        }
        std::mem::swap(above, row);
    }
    count > 0 && sum as f32 >= MIN_PORTRAIT_TEXTURE * count as f32
}

/// Whether `y` is at least [`MIN_ROW_DISTANCE`] from every accepted row.
fn far_from(tops: &[(i32, RowAnchor, BarEvidence)], y: i32) -> bool {
    tops.iter()
        .all(|&(t, ..)| (y - t).abs() >= MIN_ROW_DISTANCE)
}

#[cfg(test)]
mod tests {
    use super::{RowScratch, accept_stacked};
    use crate::killfeed::{BarEvidence, ROW_PITCH, RowAnchor};

    fn tops(s: &RowScratch) -> Vec<i32> {
        let mut t: Vec<i32> = s.tops.iter().map(|&(y, ..)| y).collect();
        t.sort_unstable();
        t
    }

    #[test]
    fn weak_luminance_anchors_count_where_they_stack() {
        // A chain below an accepted row, two rows off by up to two rows.
        let mut s = RowScratch::default();
        s.tops
            .push((16, RowAnchor::TextLine, BarEvidence::Luminance));
        s.stacked = vec![16 + ROW_PITCH + 2, 16 + 2 * ROW_PITCH + 1, 200];
        accept_stacked(&mut s);
        assert_eq!(tops(&s), [16, 57, 95]);
        assert_eq!(s.stacked, [200]);
        assert!(
            s.tops
                .iter()
                .all(|&(_, a, e)| a == RowAnchor::TextLine && e == BarEvidence::Luminance)
        );
        // Two weak anchors one pitch apart support each other.
        let mut s = RowScratch {
            stacked: vec![30, 30 + ROW_PITCH],
            ..RowScratch::default()
        };
        accept_stacked(&mut s);
        assert_eq!(tops(&s), [30, 30 + ROW_PITCH]);
        // A lone weak anchor, or one off the pitch, does not count.
        let mut s = RowScratch::default();
        s.tops
            .push((16, RowAnchor::BarEdge, BarEvidence::DefaultColour));
        s.stacked = vec![16 + ROW_PITCH + 5, 150];
        accept_stacked(&mut s);
        assert_eq!(tops(&s), [16]);
    }
}
