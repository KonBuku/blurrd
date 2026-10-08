//! Text lines inside the chat box and the sender name of each line (port
//! spec section 6, steps 2 to 7).

use blurrd_core::RectI;

use crate::components::{Run, merge_runs, runs};
use crate::image::RgbView;

use super::ink::InkScratch;
use super::{INK_X, SEARCH_Y};

/// A stroke pixel has a top-hat above this in some channel.
const INK_MIN: u8 = 40;
/// A text row has at least this many stroke pixels.
const ROW_MIN_INK: u32 = 3;
/// Rows of a line before trimming.
const MIN_RUN: u32 = 6;
/// End rows below this fraction of the line's peak are trimmed off.
const TRIM: f32 = 0.1;
/// A line is 8 to 22 rows tall; a taller run is two touching lines.
const LINE_ROWS: (u32, u32) = (8, 22);
/// Rows kept at each end when two touching lines are split.
const SPLIT_MARGIN: (u32, u32) = (6, 5);
/// A line starting at most this far below the previous line's top is the
/// continuation of a wrapped message (messages are 29 px apart, a wrapped
/// line 18 px).
const WRAP_MAX: i32 = 23;
/// A line's centre lies in a box band, with this tolerance.
const BAND_TOLERANCE: f32 = 3.0;
/// A word column holds at least this many stroke pixels.
const COLUMN_MIN_INK: u32 = 2;
/// Ink columns separated by at most this many empty columns belong to one
/// word (gaps between glyphs; a space is wider).
const WORD_GAP: u32 = 2;
/// The prefix ("(All)") starts at most this far into the ink area.
const PREFIX_MAX_X: u32 = 16;
/// Width range of the prefix, from "(All)" to "(Broadcast)" (last column
/// minus first).
const PREFIX_WIDTH: (u32, u32) = (15, 85);
/// The colon is searched from this far after the name start ...
const COLON_FROM: u32 = 4;
/// ... to this far.
const COLON_TO: u32 = 230;
/// A name is at least this wide (last column minus first).
const MIN_NAME_WIDTH: u32 = 8;
/// A parenthesis is a glyph at least this fraction of the line height.
const PAREN_HEIGHT: f32 = 0.85;

/// A sender name found in a line, in reference pixels (half-open).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct NameLine {
    /// The line's text rows.
    pub(super) rows: (i32, i32),
    /// The name's columns.
    pub(super) cols: (i32, i32),
}

/// Reusable buffers of the line finder.
#[derive(Debug, Clone, Default)]
pub(super) struct LineScratch {
    strokes: InkScratch,
    /// Stroke pixels of the ink area: 1 or 0, row after row.
    ink: Vec<u8>,
    profile: Vec<u32>,
    columns: Vec<bool>,
}

/// The ink area in reference pixels: the box interior over the searched
/// rows.
const fn ink_rect() -> RectI {
    RectI {
        x0: INK_X.0,
        y0: SEARCH_Y.0,
        x1: INK_X.1,
        y1: SEARCH_Y.1,
    }
}

impl LineScratch {
    /// Marks the stroke pixels of the ink area: the white top-hat of each
    /// channel, as in a crop of only that area, above [`INK_MIN`] in any
    /// channel.
    fn compute_ink(&mut self, crop: &RgbView<'_>, origin: (i32, i32)) {
        self.strokes
            .stroke_mask(crop, origin, ink_rect(), INK_MIN, &mut self.ink);
    }

    /// Ink of row `y` (ink-area coordinates).
    fn ink_row(&self, y: u32) -> &[u8] {
        let w = ink_rect().width() as usize;
        &self.ink[y as usize * w..(y as usize + 1) * w]
    }
}

/// Finds the sender name of every chat line in `crop` (the left panel,
/// whose top-left pixel is reference pixel `origin`) and calls `emit` for
/// each with the band the line lies in.
pub(super) fn find_names<F>(
    crop: &RgbView<'_>,
    origin: (i32, i32),
    bands: &[super::BoxBand],
    scratch: &mut LineScratch,
    mut emit: F,
) where
    F: FnMut(NameLine, &super::BoxBand),
{
    scratch.compute_ink(crop, origin);
    let area = ink_rect();
    // Taken out of the scratch while the line test borrows the ink.
    let mut profile = std::mem::take(&mut scratch.profile);
    profile.clear();
    let width = area.width() as usize;
    profile.extend(
        scratch
            .ink
            .chunks_exact(width)
            .map(|row| row.iter().map(|&v| u32::from(v)).sum::<u32>()),
    );
    let mut prev_top = i32::MIN / 2;
    let candidates = runs(profile.iter().map(|&p| p >= ROW_MIN_INK));
    for run in candidates.into_iter().filter(|r| r.len() >= MIN_RUN) {
        let (mut a, mut b) = (run.start, run.end - 1);
        let peak = profile[a as usize..=b as usize]
            .iter()
            .copied()
            .max()
            .unwrap_or(0);
        let low = |v: u32| (v as f32) < TRIM * peak as f32;
        while a < b && low(profile[a as usize]) {
            a += 1;
        }
        while b > a && low(profile[b as usize]) {
            b -= 1;
        }
        let parts = if b - a + 1 > LINE_ROWS.1 {
            let lo = a + SPLIT_MARGIN.0;
            let hi = b - SPLIT_MARGIN.1;
            // The first weakest row, as `argmin` picks it.
            let m = (lo..hi)
                .min_by_key(|&i| (profile[i as usize], i))
                .unwrap_or(lo);
            [Some((a, m - 1)), Some((m + 1, b))]
        } else {
            [Some((a, b)), None]
        };
        for (a, b) in parts.into_iter().flatten() {
            let rows = b - a + 1;
            if !(LINE_ROWS.0..=LINE_ROWS.1).contains(&rows) {
                continue;
            }
            let top = area.y0 + a as i32;
            let wrapped = top - prev_top <= WRAP_MAX;
            prev_top = top;
            if wrapped {
                continue;
            }
            let centre = area.y0 as f32 + (a + b) as f32 / 2.0;
            let Some(band) = bands.iter().find(|band| {
                band.y0 as f32 - BAND_TOLERANCE <= centre
                    && centre <= (band.y1 - 1) as f32 + BAND_TOLERANCE
            }) else {
                continue;
            };
            if let Some(cols) = name_columns(scratch, a, b) {
                emit(
                    NameLine {
                        rows: (top, area.y0 + b as i32 + 1),
                        cols: (area.x0 + cols.0 as i32, area.x0 + cols.1 as i32 + 1),
                    },
                    band,
                );
            }
        }
    }
    scratch.profile = profile;
}

/// The ink of one line (rows `a..=b` of the ink area).
struct LineInk<'a> {
    scratch: &'a LineScratch,
    a: u32,
    height: u32,
    width: u32,
}

impl LineInk<'_> {
    fn get(&self, x: u32, y: u32) -> bool {
        self.scratch.ink_row(self.a + y)[x as usize] != 0
    }

    /// Whether any of rows `rows` has ink in columns `x0..x1` (clipped).
    fn any(&self, x0: u32, x1: u32, rows: &(impl Iterator<Item = u32> + Clone)) -> bool {
        (x0..x1.min(self.width)).any(|x| rows.clone().any(|y| self.get(x, y)))
    }

    /// The runs of ink in column `x`.
    fn column_runs(&self, x: u32) -> Vec<Run> {
        runs((0..self.height).map(|y| self.get(x, y)))
    }
}

/// Inclusive first and last ink-area column of the sender name of the line
/// in rows `a..=b`, or `None` if the line does not look like a chat
/// message.
fn name_columns(scratch: &mut LineScratch, a: u32, b: u32) -> Option<(u32, u32)> {
    let width = ink_rect().width();
    let height = b - a + 1;
    let columns = &mut scratch.columns;
    columns.clear();
    columns.extend((0..width).map(|x| {
        (a..=b)
            .map(|y| u32::from(scratch.ink[y as usize * width as usize + x as usize]))
            .sum::<u32>()
            >= COLUMN_MIN_INK
    }));
    let words = merge_runs(&runs(columns.iter().copied()), WORD_GAP);
    let line = LineInk {
        scratch,
        a,
        height,
        width,
    };
    let (prefix, first) = (words.first()?, words.get(1)?);
    if prefix.start > PREFIX_MAX_X
        || !(PREFIX_WIDTH.0..=PREFIX_WIDTH.1).contains(&(prefix.end - 1 - prefix.start))
    {
        return None;
    }
    let nx0 = first.start;
    let nx1 = match colon_column(&line, nx0 + COLON_FROM, nx0 + COLON_TO) {
        Some(colon) => {
            let mut name: Vec<&Run> = words[1..].iter().filter(|w| w.start < colon).collect();
            // "(Broadcast) name (Agent): ..." - the role in parentheses
            // before the colon is not part of the name.
            if let [.., last] = name[..]
                && name.len() > 1
                && is_paren(&line, last.start, last.start + 1)
                && is_paren(&line, colon - 3, colon - 1)
            {
                name.pop();
            }
            match name[..] {
                [_, .., last] => last.end - 1,
                _ => colon - 1,
            }
        }
        None => first.end - 1,
    };
    (nx1 >= nx0 + MIN_NAME_WIDTH).then_some((nx0, nx1))
}

/// Whether columns `c0..=c1` of the line hold a glyph as tall as a
/// parenthesis.
fn is_paren(line: &LineInk<'_>, c0: u32, c1: u32) -> bool {
    let rows: Vec<u32> = (0..line.height)
        .filter(|&y| (c0..=c1.min(line.width - 1)).any(|x| line.get(x, y)))
        .collect();
    match (rows.first(), rows.last()) {
        (Some(&first), Some(&last)) => {
            (last - first + 1) as f32 >= PAREN_HEIGHT * line.height as f32
        }
        _ => false,
    }
}

/// The first column in `from..=to` that holds the colon after a name: two
/// short dots (at most 3 rows each, 3 to 8 empty rows apart, the lower one
/// in the bottom half of the line) glued to the name before them (no
/// space) and followed by a space.
fn colon_column(line: &LineInk<'_>, from: u32, to: u32) -> Option<u32> {
    (from..=to.min(line.width - 1)).find(|&c| {
        let dots = line.column_runs(c);
        let [upper, lower] = dots[..] else {
            return false;
        };
        let gap = lower.start - upper.end;
        if upper.len() > 3 || lower.len() > 3 || !(3..=8).contains(&gap) {
            return false;
        }
        if ((lower.end - 1) as f32) < 0.5 * line.height as f32 {
            return false;
        }
        let dot_rows = (upper.start..upper.end).chain(lower.start..lower.end);
        // Isolated dots: nothing directly left of them ...
        if c > 0 && line.any(c - 1, c, &dot_rows) {
            return false;
        }
        // ... but the name ends right before them (no space) ...
        if c < 3 || !line.any(c - 3, c - 1, &(0..line.height)) {
            return false;
        }
        // ... a space follows ...
        if c + 6 <= line.width && line.any(c + 3, c + 6, &(0..line.height)) {
            return false;
        }
        if line.any(c + 3, c + 4, &dot_rows) {
            return false;
        }
        // ... and nothing lies between the dots.
        !line.any(c, c + 3, &(upper.end..lower.start))
    })
}
