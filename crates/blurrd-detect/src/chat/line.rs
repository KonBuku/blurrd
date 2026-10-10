//! Text lines inside the chat box and the sender name of each line (port
//! spec section 6, steps 2 to 7).

use blurrd_core::RectI;

use crate::components::{Run, merge_runs, runs};
use crate::image::RgbView;

use super::ink::InkScratch;
use super::{INK_X, SEARCH_Y};

/// Lines are found from pixels with a top-hat above this in some channel.
const INK_MIN: u8 = 40;
/// Inside a line, a stroke pixel has a top-hat above this in some channel:
/// coloured names lose most of their contrast in greyscale and darkened
/// video, while the white message keeps it and finds the line.
const READ_MIN: u8 = 24;
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
/// An ink column holds at least this many stroke pixels.
const COLUMN_MIN_INK: usize = 2;
/// Ink columns separated by at most this many empty columns belong to one
/// word (gaps between glyphs; a space is wider).
const WORD_GAP: u32 = 2;
/// The prefix ("(All)") starts at most this far into the ink area.
const PREFIX_MAX_X: u32 = 16;
/// The prefix is at most this wide (last column minus first):
/// "(Broadcast)" is 72 px wide, translated prefixes are up to about 90 px.
const PREFIX_MAX_WIDTH: u32 = 140;
/// The shortest prefix ("(All)", 24 px from the opening to the closing
/// parenthesis on recordings), with a margin: where the prefix runs into
/// the name, the name is covered from this far after the opening
/// parenthesis.
const MIN_PREFIX_WIDTH: u32 = 18;
/// Without an opening parenthesis the prefix is the first word, and its
/// width (last column minus first) must lie in this range. The port
/// spec's upper bound of 85 px is raised for translated prefixes.
const PREFIX_WIDTH: (u32, u32) = (15, PREFIX_MAX_WIDTH);
/// The colon is searched from this far after the name start (the port
/// spec's 4 px, but a name is at least [`MIN_NAME_WIDTH`] wide, and
/// fragments of a faint name must not end it early) ...
const COLON_FROM: u32 = MIN_NAME_WIDTH + 1;
/// ... to this far.
const COLON_TO: u32 = 230;
/// A colon dot is at most this many rows tall in the glyph test (the port
/// spec's 3, plus one for fonts and scalings that smear a dot over another
/// row).
const MAX_DOT_ROWS: u32 = 4;
/// A colon glyph is at most this many columns wide.
const COLON_COLUMNS: u32 = 3;
/// Empty rows between the two colon dots in the glyph test (the port
/// spec's 3 to 8; condensed fonts and downscaled video leave 2).
const DOT_GAP: (u32, u32) = (2, 8);
/// A name is at least this wide (last column minus first).
const MIN_NAME_WIDTH: u32 = 8;
/// The first glyph of a line is an opening parenthesis if it spans at
/// least this fraction of the line height (accents above capitals make a
/// line taller than its parentheses).
const OPENING_HEIGHT: f32 = 0.7;
/// Parenthesis shapes are measured over this many columns (a parenthesis
/// is 2 to 3 px wide).
const PAREN_COLUMNS: u32 = 3;
/// Glyph shapes are measured over the line's rows plus this many rows
/// above and below: trimming a line drops its sparse descender rows, and
/// parentheses reach into them.
const SHAPE_MARGIN: (u32, u32) = (2, 3);
/// A line without a colon whose name has no colour of its own (system
/// lines: "(System) Name has abandoned the match"; black-and-white video)
/// is covered from the name start for this many pixels, or to the end of
/// the line: the widest name (16 characters, about 140 px) plus a margin
/// for wide glyphs.
pub(super) const SAFE_NAME_WIDTH: u32 = 160;
/// A colour is chromatic if `(max - min) / max` of its channels reaches
/// this.
const MIN_SATURATION: f32 = 0.18;
/// Name glyphs have the prefix colour: a hue within this many degrees ...
const MAX_HUE_DIFFERENCE: f32 = 30.0;
/// ... and at least this fraction of its saturation.
const MIN_SATURATION_RATIO: f32 = 0.5;
/// The glyph after a colon is measured over this many columns.
const GLYPH_COLUMNS: u32 = 5;
/// A colour is measured on high-contrast stroke pixels if a glyph has at
/// least this many.
const MIN_COLOUR_PIXELS: usize = 4;
/// The line's white level is the brightness this fraction of its stroke
/// pixels stay below (the brightest strokes are the message's white).
const WHITE_PERCENTILE: f32 = 0.9;
/// White glyphs are at least this fraction of the line's white level
/// bright: faint coloured glyphs on bright backgrounds lose their
/// saturation, but they stay darker than the white message.
const WHITE_FRACTION: f32 = 0.85;

/// How the end of a sender name was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NameEnd {
    /// At the colon after the name.
    Colon,
    /// Where the prefix colour ends: the line has no colon that could be
    /// found, and the text after the name is white.
    Colour,
    /// Not found: the line has no colon and the name has no colour of its
    /// own (system lines, black-and-white video). The name is covered from
    /// its start for the widest name or to the end of the line, which
    /// covers names of several words (Safe geometry).
    Safe,
}

/// How the start of a sender name was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NameStart {
    /// After the prefix's closing parenthesis.
    Parenthesis,
    /// At the second word of the line (the parentheses could not be found,
    /// for example on broken low-contrast strokes), as in the prototype.
    Word,
    /// Not found: the prefix runs into the name without a gap (bright
    /// backgrounds, blurred upscaled video). The name is covered from the
    /// end of the shortest prefix, which covers it whatever the prefix
    /// (Safe geometry).
    Shortest,
}

/// A sender name found in a line, in reference pixels (half-open).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct NameLine {
    /// The line's text rows.
    pub(super) rows: (i32, i32),
    /// The name's columns.
    pub(super) cols: (i32, i32),
    /// How the name's start was found.
    pub(super) start: NameStart,
    /// How the name's end was found.
    pub(super) end: NameEnd,
    /// The columns of the text after the name (the message or the system
    /// phrase), if there is any.
    pub(super) message: Option<(i32, i32)>,
}

/// Reusable buffers of the line finder.
#[derive(Debug, Clone, Default)]
pub(super) struct LineScratch {
    strokes: InkScratch,
    /// The top-hat of every pixel of the ink area, row after row.
    levels: Vec<u8>,
    profile: Vec<u32>,
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
    /// Measures the strokes of the ink area: the white top-hat of each
    /// channel, as in a crop of only that area, the largest of the three.
    /// `crop` is the left panel, whose top-left pixel is reference pixel
    /// `origin`.
    pub(super) fn compute_ink(&mut self, crop: &RgbView<'_>, origin: (i32, i32)) {
        self.strokes
            .stroke_levels(crop, origin, ink_rect(), &mut self.levels);
    }

    /// Top-hat levels of row `y` (ink-area coordinates).
    fn levels_row(&self, y: u32) -> &[u8] {
        let w = ink_rect().width() as usize;
        &self.levels[y as usize * w..(y as usize + 1) * w]
    }

    /// The stroke pixels in `rect` (reference pixels, clipped to the ink
    /// area) as of the last [`compute_ink`](Self::compute_ink).
    pub(super) fn strokes_in(&self, rect: RectI) -> u32 {
        let Some(r) = rect.intersection(&ink_rect()) else {
            return 0;
        };
        let area = ink_rect();
        let (x0, x1) = ((r.x0 - area.x0) as usize, (r.x1 - area.x0) as usize);
        ((r.y0 - area.y0) as u32..(r.y1 - area.y0) as u32)
            .map(|y| {
                self.levels_row(y)[x0..x1]
                    .iter()
                    .map(|&v| u32::from(v > READ_MIN))
                    .sum::<u32>()
            })
            .sum()
    }
}

/// Finds the sender name of every chat line in `crop` (the left panel,
/// whose top-left pixel is reference pixel `origin`) and calls `emit` for
/// each with the band the line lies in. The ink must have been computed
/// for this crop ([`LineScratch::compute_ink`]).
pub(super) fn find_names<F>(
    crop: &RgbView<'_>,
    origin: (i32, i32),
    bands: &[super::BoxBand],
    scratch: &mut LineScratch,
    mut emit: F,
) where
    F: FnMut(NameLine, &super::BoxBand),
{
    let area = ink_rect();
    // Taken out of the scratch while the line test borrows the ink.
    let mut profile = std::mem::take(&mut scratch.profile);
    profile.clear();
    let width = area.width() as usize;
    profile.extend(
        scratch
            .levels
            .chunks_exact(width)
            .map(|row| row.iter().map(|&v| u32::from(v > INK_MIN)).sum::<u32>()),
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
            let line = LineInk::new(scratch, crop, origin, a, b);
            if let Some(read) = read_name(&line) {
                let x = |c: u32| area.x0 + c as i32;
                // The name's own strokes may reach beyond the line's rows,
                // which were trimmed where the line has little ink (faint
                // capitals at the top, descenders at the bottom).
                let (na, nb) = line
                    .extent(read.name.0, read.name.1)
                    .map_or((a, b), |(na, nb)| (na.min(a), nb.max(b)));
                emit(
                    NameLine {
                        rows: (area.y0 + na as i32, area.y0 + nb as i32 + 1),
                        cols: (x(read.name.0), x(read.name.1) + 1),
                        start: read.start,
                        end: read.end,
                        message: read.message.map(|(m0, m1)| (x(m0), x(m1) + 1)),
                    },
                    band,
                );
            }
        }
    }
    scratch.profile = profile;
}

/// The ink and colours of one line (rows `a..=b` of the ink area).
struct LineInk<'a> {
    scratch: &'a LineScratch,
    crop: &'a RgbView<'a>,
    /// Reference position of the crop's top-left pixel.
    origin: (i32, i32),
    a: u32,
    height: u32,
    width: u32,
    /// Ink-area rows over which glyph shapes are measured (inclusive).
    shape_rows: (u32, u32),
    /// Runs of ink columns (columns with at least [`COLUMN_MIN_INK`] stroke
    /// pixels): glyphs or groups of touching glyphs.
    segments: Vec<Run>,
    /// The same for high-contrast ink: faint texture and anti-aliasing do
    /// not join the white colon to its neighbours.
    strong_segments: Vec<Run>,
}

impl<'a> LineInk<'a> {
    fn new(
        scratch: &'a LineScratch,
        crop: &'a RgbView<'a>,
        origin: (i32, i32),
        a: u32,
        b: u32,
    ) -> Self {
        let mut line = Self {
            scratch,
            crop,
            origin,
            a,
            height: b - a + 1,
            width: ink_rect().width(),
            shape_rows: (
                a.saturating_sub(SHAPE_MARGIN.0),
                (b + SHAPE_MARGIN.1).min(ink_rect().height() - 1),
            ),
            segments: Vec::new(),
            strong_segments: Vec::new(),
        };
        line.segments = runs(
            (0..line.width)
                .map(|x| (0..line.height).filter(|&y| line.get(x, y)).count() >= COLUMN_MIN_INK),
        );
        line.strong_segments = runs((0..line.width).map(|x| {
            (0..line.height).filter(|&y| line.get_strong(x, y)).count() >= COLUMN_MIN_INK
        }));
        line
    }

    /// Whether pixel `(x, y)` of the line is a stroke pixel.
    fn get(&self, x: u32, y: u32) -> bool {
        self.scratch.levels_row(self.a + y)[x as usize] > READ_MIN
    }

    /// Whether pixel `(x, y)` of the line is a stroke pixel of high
    /// contrast, as the white colon and message always are. Faint
    /// background texture next to the colon's dots is left out.
    fn get_strong(&self, x: u32, y: u32) -> bool {
        self.scratch.levels_row(self.a + y)[x as usize] > INK_MIN
    }

    /// Whether any of rows `rows` has ink in columns `x0..x1` (clipped).
    fn any(&self, x0: u32, x1: u32, rows: &(impl Iterator<Item = u32> + Clone)) -> bool {
        (x0..x1.min(self.width)).any(|x| rows.clone().any(|y| self.get(x, y)))
    }

    /// Whether any of rows `rows` has high-contrast ink in columns
    /// `x0..x1` (clipped).
    fn any_strong(&self, x0: u32, x1: u32, rows: &(impl Iterator<Item = u32> + Clone)) -> bool {
        (x0..x1.min(self.width)).any(|x| rows.clone().any(|y| self.get_strong(x, y)))
    }

    /// The runs of high-contrast ink in column `x`.
    fn column_runs(&self, x: u32) -> Vec<Run> {
        runs((0..self.height).map(|y| self.get_strong(x, y)))
    }

    /// First and last ink-area row with ink in columns `c0..=c1`, over
    /// the line's rows widened by [`SHAPE_MARGIN`].
    fn extent(&self, c0: u32, c1: u32) -> Option<(u32, u32)> {
        let c1 = c1.min(self.width - 1);
        let mut rows = (self.shape_rows.0..=self.shape_rows.1)
            .filter(|&y| (c0..=c1).any(|x| self.scratch.levels_row(y)[x as usize] > READ_MIN));
        let first = rows.next()?;
        Some((first, rows.next_back().unwrap_or(first)))
    }

    /// The colour of the strokes in columns `c0..c1`: the channel medians
    /// of the brighter half of their high-contrast stroke pixels, or of all
    /// their stroke pixels where too few have high contrast (faint coloured
    /// glyphs). Anti-aliased edges and faint texture mix in the background
    /// behind the box, so they and the darker half are left out.
    fn colour(&self, c0: u32, c1: u32) -> Option<[u8; 3]> {
        let area = ink_rect();
        let collect = |stroke: &dyn Fn(u32, u32) -> bool| {
            let mut pixels: Vec<[u8; 3]> = Vec::new();
            for y in 0..self.height {
                for x in c0..c1.min(self.width) {
                    if stroke(x, y) {
                        pixels.push(self.crop.pixel(
                            (area.x0 + x as i32 - self.origin.0) as u32,
                            (area.y0 + (self.a + y) as i32 - self.origin.1) as u32,
                        ));
                    }
                }
            }
            pixels
        };
        let mut pixels = collect(&|x, y| self.get_strong(x, y));
        if pixels.len() < MIN_COLOUR_PIXELS {
            pixels = collect(&|x, y| self.get(x, y));
        }
        if pixels.is_empty() {
            return None;
        }
        let half = pixels.len() / 2;
        pixels.select_nth_unstable_by_key(half, |p| std::cmp::Reverse(p.iter().max().copied()));
        let bright = &mut pixels[..=half];
        let mut median = [0; 3];
        let mid = bright.len() / 2;
        for (c, m) in median.iter_mut().enumerate() {
            *m = bright.select_nth_unstable_by_key(mid, |p| p[c]).1[c];
        }
        Some(median)
    }

    /// The brightness (largest channel) that [`WHITE_PERCENTILE`] of the
    /// line's stroke pixels stay below: the white of the message where the
    /// line has one.
    fn white_level(&self) -> u8 {
        let area = ink_rect();
        let mut levels: Vec<u8> = Vec::new();
        for y in 0..self.height {
            for x in 0..self.width {
                if self.get(x, y) {
                    let [r, g, b] = self.crop.pixel(
                        (area.x0 + x as i32 - self.origin.0) as u32,
                        (area.y0 + (self.a + y) as i32 - self.origin.1) as u32,
                    );
                    levels.push(r.max(g).max(b));
                }
            }
        }
        if levels.is_empty() {
            return 0;
        }
        let k = ((levels.len() - 1) as f32 * WHITE_PERCENTILE) as usize;
        *levels.select_nth_unstable(k).1
    }
}

/// What [`read_name`] found, in ink-area columns (inclusive).
struct NameRead {
    /// The columns to cover: the name, widened over the empty columns
    /// next to it where an end was found from its neighbours.
    name: (u32, u32),
    start: NameStart,
    end: NameEnd,
    message: Option<(u32, u32)>,
}

/// Reads the sender name of a line, or returns `None` if the line does not
/// look like a chat message.
///
/// Nothing depends on the client language: the prefix is the first word,
/// recognised by its parentheses whatever its width; the name starts at
/// the next word and ends at the colon, or where the prefix colour ends.
/// Uncertain ends take Safe geometry ([`NameStart::Shortest`],
/// [`NameEnd::Safe`]).
///
/// The colour prior: the prefix has the name's colour, and the colon is
/// followed by the white message. A colon candidate followed by more of
/// the name is a broken glyph inside the prefix or the name (low contrast
/// splits a parenthesis into two dots), not the end of the name.
fn read_name(line: &LineInk<'_>) -> Option<NameRead> {
    let first = *line.segments.first()?;
    if first.start > PREFIX_MAX_X {
        return None;
    }
    let paren = opening_paren(line);
    let words = merge_runs(&line.segments, WORD_GAP);
    let prefix = *words.first()?;
    let shortest = || {
        // The prefix runs into the name (no space between them).
        let nx0 = first.start + MIN_PREFIX_WIDTH;
        (NameStart::Shortest, nx0 - 1, nx0)
    };
    let (mut start, mut prefix_end, mut nx0) =
        match paren.map(|shape| closing_paren(line, prefix, shape)) {
            Some(true) => (NameStart::Parenthesis, prefix.end - 1, words.get(1)?.start),
            Some(false) => shortest(),
            None => {
                // Without an opening parenthesis (broken strokes on low
                // contrast) the prefix is the first word, as in the
                // prototype.
                let name = words.get(1)?;
                if !(PREFIX_WIDTH.0..=PREFIX_WIDTH.1).contains(&(prefix.end - 1 - prefix.start)) {
                    return None;
                }
                (NameStart::Word, prefix.end - 1, name.start)
            }
        };
    let last_ink = line.segments.last()?.end - 1;
    let mut colours = LineColours::new(line, (first.start, prefix_end));
    if start != NameStart::Shortest
        && colours.prefix.is_some()
        && words
            .get(1)
            .and_then(|name| line.colour(name.start, name.end))
            .is_some_and(|c| colours.is_white(c))
    {
        // The second word is white, so it is the message: the first word
        // is the prefix and the name run together, and a tall last letter
        // of the name looked like the closing parenthesis.
        (start, prefix_end, nx0) = shortest();
        colours = LineColours::new(line, (first.start, prefix_end));
    }
    let ends_name = |colon: u32| colours.ends_name(line, colon);
    // The port spec's colon test first; it is exact on recordings. The
    // glyph test finds colons it cannot (other fonts and scalings).
    let colon = colon_column(line, nx0 + COLON_FROM, nx0 + COLON_TO, &ends_name)
        .or_else(|| colon_glyph(line, nx0 + COLON_FROM, nx0 + COLON_TO, &ends_name));
    // The name's last column, the last column to cover, and how the end
    // was found.
    let (nx1, cover_end, end) = if let Some(colon) = colon {
        // Up to the colon: a role before it ("(Broadcast) name (Agent):")
        // is covered with the name. Its parentheses cannot be told from
        // tall letters such as "l" reliably, and a role is not private,
        // while a name cut short is a leak.
        (colon - 1, colon - 1, NameEnd::Colon)
    } else if let Some((nx1, next)) = (start != NameStart::Shortest)
        .then(|| colours.colour_end(line, nx0))
        .flatten()
    {
        // Up to the white glyph after the name: the name's last glyph may
        // be too faint to be seen.
        (nx1, next.map_or(nx1, |n| n - 1), NameEnd::Colour)
    } else {
        // Where the prefix runs into the name, the name may start as far
        // as the widest prefix allows, and the colour run includes the
        // rest of the prefix, so it says nothing about the name's end.
        let reach = match start {
            NameStart::Shortest => PREFIX_MAX_WIDTH - MIN_PREFIX_WIDTH + SAFE_NAME_WIDTH,
            NameStart::Parenthesis | NameStart::Word => SAFE_NAME_WIDTH,
        };
        let nx1 = (nx0 + reach - 1).min(last_ink);
        (nx1, nx1, NameEnd::Safe)
    };
    if nx1 < nx0 + MIN_NAME_WIDTH {
        return None;
    }
    let message = line
        .segments
        .iter()
        .find(|s| s.start > nx1 + 1)
        .map(|s| (s.start, last_ink));
    // From the first empty column after the prefix: the name's first glyph
    // may be too faint to be seen.
    let cover_start = match start {
        NameStart::Parenthesis | NameStart::Word => (prefix_end + 1).min(nx0),
        NameStart::Shortest => nx0,
    };
    Some(NameRead {
        name: (cover_start, cover_end),
        start,
        end,
        message,
    })
}

/// The colours of one line: the prefix colour (the name's colour) and the
/// white of the message.
struct LineColours {
    /// Hue and saturation of the prefix, if it is chromatic.
    prefix: Option<(f32, f32)>,
    /// The brightness of the line's whitest strokes.
    white_level: u8,
}

impl LineColours {
    /// Measures the colours of `line`, whose prefix spans columns
    /// `prefix.0..=prefix.1`.
    fn new(line: &LineInk<'_>, prefix: (u32, u32)) -> Self {
        Self {
            prefix: line
                .colour(prefix.0, prefix.1 + 1)
                .map(hue_saturation)
                .filter(|&(_, sat)| sat >= MIN_SATURATION),
            white_level: line.white_level(),
        }
    }

    /// Whether `colour` is the white of the message: unsaturated and about
    /// as bright as the line's whitest strokes. A faint coloured glyph on
    /// a bright background is unsaturated too, but darker.
    fn is_white(&self, colour: [u8; 3]) -> bool {
        let max = colour.iter().max().copied().unwrap_or(0);
        hue_saturation(colour).1 < MIN_SATURATION
            && f32::from(max) >= WHITE_FRACTION * f32::from(self.white_level)
    }

    /// Whether the colon candidate in column `colon` can end the name: the
    /// glyph after it is the white message. A broken glyph inside the
    /// prefix or the name is followed by more of the name, which has the
    /// prefix colour or, in greyscale video and on bright backgrounds, is
    /// darker than the message.
    fn ends_name(&self, line: &LineInk<'_>, colon: u32) -> bool {
        // The first ink after the colon, glyph by glyph: on bright
        // backgrounds faint glyphs run together into one segment.
        (colon + COLON_COLUMNS + 1..line.width)
            .find(|&x| (0..line.height).filter(|&y| line.get(x, y)).count() >= COLUMN_MIN_INK)
            .and_then(|x| line.colour(x, x + GLYPH_COLUMNS))
            .is_none_or(|c| self.is_white(c))
    }

    /// Where the prefix colour ends after the name start `nx0`: the last
    /// column of the prefix-coloured glyphs, and the first column of the
    /// white glyph after them if there is one. One glyph of neither colour
    /// between them (a colon or a last letter whose few pixels mix with the
    /// background) is covered with the name. `None` if the prefix has no
    /// colour of its own, the name does not have it, or what follows the
    /// coloured glyphs is not white (the colon and the message are white;
    /// anything else means that part of the name was too faint to show its
    /// colour).
    fn colour_end(&self, line: &LineInk<'_>, nx0: u32) -> Option<(u32, Option<u32>)> {
        let (ref_hue, ref_sat) = self.prefix?;
        let mut last = None;
        let mut unclear = false;
        for segment in line.segments.iter().filter(|s| s.start >= nx0) {
            let colour = line.colour(segment.start, segment.end)?;
            let (hue, sat) = hue_saturation(colour);
            let d = (hue - ref_hue).abs();
            if d.min(360.0 - d) <= MAX_HUE_DIFFERENCE && sat >= MIN_SATURATION_RATIO * ref_sat {
                last = Some(segment.end - 1);
                unclear = false;
            } else if self.is_white(colour) {
                return last.map(|l| (l, Some(segment.start)));
            } else if unclear || last.is_none() {
                return None;
            } else {
                unclear = true;
            }
        }
        last.map(|l| (l, None))
    }
}

/// The rows of the line's first glyph if it is an opening parenthesis.
fn opening_paren(line: &LineInk<'_>) -> Option<(u32, u32)> {
    let first = line.segments.first()?;
    let rows = line.extent(first.start, first.start + PAREN_COLUMNS - 1)?;
    ((rows.1 - rows.0 + 1) as f32 >= OPENING_HEIGHT * line.height as f32).then_some(rows)
}

/// Whether the line's first word, the prefix, ends with a closing
/// parenthesis.
///
/// Only parentheses reach from the ascender line to below the baseline, so
/// the closing one has the rows of the opening one (`shape`) in its last
/// [`PAREN_COLUMNS`] columns. The prefix is one word in every client
/// language, of whatever width; when the space after it closes up (bright
/// backgrounds, blurred video), the first word runs into the name, does
/// not end with a parenthesis, and the name start is not trusted.
fn closing_paren(line: &LineInk<'_>, prefix: Run, shape: (u32, u32)) -> bool {
    prefix.end - prefix.start > PAREN_COLUMNS
        && prefix.end - 1 - prefix.start <= PREFIX_MAX_WIDTH
        && line
            .extent(prefix.end - PAREN_COLUMNS, prefix.end - 1)
            .is_some_and(|rows| same_rows(rows, shape))
}

/// Whether ink spanning `rows` has the shape of the parenthesis spanning
/// `shape`. Parentheses reach from the ascender line to below the
/// baseline; the opening and the closing one are mirror images, so their
/// rows agree, up to one row at one end for sub-pixel positions (measured
/// on recordings: 89 % exact, 97 % within this rule). A glyph that ends a
/// row short at both ends (capitals, digits, descenders) is not one.
fn same_rows(rows: (u32, u32), shape: (u32, u32)) -> bool {
    let (top, bottom) = (rows.0.abs_diff(shape.0), rows.1.abs_diff(shape.1));
    top.max(bottom) <= 1 && top.min(bottom) == 0
}

/// Hue in degrees and saturation (`(max - min) / max`) of a colour.
fn hue_saturation([r, g, b]: [u8; 3]) -> (f32, f32) {
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    if max == min {
        return (0.0, 0.0);
    }
    let chroma = f32::from(max - min);
    let channel = |c: u8| f32::from(c);
    let hue = if max == r {
        60.0 * (channel(g) - channel(b)) / chroma
    } else if max == g {
        60.0 * ((channel(b) - channel(r)) / chroma + 2.0)
    } else {
        60.0 * ((channel(r) - channel(g)) / chroma + 4.0)
    };
    (hue.rem_euclid(360.0), chroma / f32::from(max))
}

/// The first column of the first narrow glyph in `from..=to` that is the
/// colon after a name: at most [`COLON_COLUMNS`] columns wide, two short
/// dots over its columns together ([`MAX_DOT_ROWS`] rows, [`DOT_GAP`]
/// rows apart, the lower one in the bottom half of the line), followed by a
/// space.
///
/// [`colon_column`] tests single columns as the port spec does, and
/// requires the name's last glyph right before the colon; this test takes
/// the glyph as a whole, so it also finds colons whose dots fall on
/// different rows in neighbouring columns (sub-pixel positions, scaled
/// video, other fonts) and colons after a name too faint to be seen. The
/// glyphs are taken from high-contrast ink.
/// Candidates `ends_name` rejects are skipped.
fn colon_glyph(
    line: &LineInk<'_>,
    from: u32,
    to: u32,
    ends_name: &impl Fn(u32) -> bool,
) -> Option<u32> {
    let segments = &line.strong_segments;
    (1..segments.len())
        .filter(|&k| segments[k].start >= from && segments[k].start <= to)
        .find(|&k| {
            let glyph = segments[k];
            if glyph.len() > COLON_COLUMNS {
                return false;
            }
            if segments
                .get(k + 1)
                .is_some_and(|next| next.start - glyph.end <= WORD_GAP)
            {
                return false;
            }
            let dots = runs(
                (0..line.height).map(|y| (glyph.start..glyph.end).any(|x| line.get_strong(x, y))),
            );
            let [upper, lower] = dots[..] else {
                return false;
            };
            upper.len() <= MAX_DOT_ROWS
                && lower.len() <= MAX_DOT_ROWS
                && (DOT_GAP.0..=DOT_GAP.1).contains(&(lower.start - upper.end))
                && (lower.end - 1) as f32 >= 0.5 * line.height as f32
                && ends_name(glyph.start)
        })
        .map(|k| segments[k].start)
}

/// The first column in `from..=to` that holds the colon after a name: two
/// short dots (at most 3 rows each, 3 to 8 empty rows apart, the lower one
/// in the bottom half of the line) glued to the name
/// before them (no space) and followed by a space. The dots and the space
/// are tested on high-contrast ink (the colon and the message are white),
/// the name on all ink (it may be faint). Candidates `ends_name`
/// rejects are skipped.
fn colon_column(
    line: &LineInk<'_>,
    from: u32,
    to: u32,
    ends_name: &impl Fn(u32) -> bool,
) -> Option<u32> {
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
        if c > 0 && line.any_strong(c - 1, c, &dot_rows) {
            return false;
        }
        // ... but the name ends right before them (no space) ...
        if c < 3 || !line.any(c - 3, c - 1, &(0..line.height)) {
            return false;
        }
        // ... a space follows ...
        if c + 6 <= line.width && line.any_strong(c + 3, c + 6, &(0..line.height)) {
            return false;
        }
        if line.any_strong(c + 3, c + 4, &dot_rows) {
            return false;
        }
        // ... and nothing lies between the dots.
        !line.any_strong(c, c + 3, &(upper.end..lower.start)) && ends_name(c)
    })
}

#[cfg(test)]
mod tests {
    use super::{hue_saturation, same_rows};

    #[test]
    fn parentheses_match_up_to_one_row_at_one_end() {
        let shape = (76, 90);
        assert!(same_rows((76, 90), shape));
        assert!(same_rows((75, 90), shape) && same_rows((76, 91), shape));
        // A capital or a digit ends a row short at both ends.
        assert!(!same_rows((77, 89), shape));
        // A descender starts at the x-height.
        assert!(!same_rows((81, 90), shape));
    }

    #[test]
    fn measures_hue_and_saturation() {
        let (hue, sat) = hue_saturation([230, 98, 93]);
        assert!((hue - 2.2).abs() < 0.1 && (sat - 0.596).abs() < 0.001);
        let (hue, sat) = hue_saturation([100, 186, 162]);
        assert!((hue - 163.3).abs() < 0.1 && (sat - 0.462).abs() < 0.001);
        assert_eq!(hue_saturation([245, 245, 245]), (0.0, 0.0));
    }
}
