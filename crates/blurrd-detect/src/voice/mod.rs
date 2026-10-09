//! Voice-chat speaker names (port spec section 5).
//!
//! While a teammate speaks, the HUD shows an entry on the left of the
//! screen: the agent portrait, the speaker's name in bold white and a grey
//! location line below it ("in B Main - Team"). Entries have no plate;
//! they are drawn straight over the game. Up to four entries stack upwards
//! from the bottom slot on a 40 px pitch, the newest at the bottom, and
//! they appear and disappear from one frame to the next.
//!
//! ```text
//! y + 2  ┌────────┐  SPEAKER NAME          name rows y + 5 .. y + 18
//!        │portrait│
//!        │        │  in B Main - Team      location rows y + 23 .. y + 35
//! y + 38 └────────┘
//!        x 16..52    x 58 ->
//! ```
//!
//! [`VoiceDetector`] scores every slot of one frame from text ink (how far
//! each pixel stands out from its blurred surroundings, maximum over the
//! colour channels), which does not depend on the hue of the text or the
//! background, so colour-blind modes and the colour filters of GPU drivers
//! leave it working. A slot shows an entry ([`SlotHit::Name`]) when it has
//! a textured portrait, ink in the name rows and the location rows, a name
//! brighter than its background, and both text lines starting at the same
//! left edge. On a bright background a white name may not stand out; the
//! portrait and the aligned location line then still show the entry, and
//! the whole name slot is boxed ([`SlotHit::Structure`]).
//!
//! The name box ends at the last ink of the name, words included (two-word
//! names), and grows up and down with glyphs of other scripts. It never
//! reaches into the location line, which is not a name (only deep
//! descenders that overlap its top rows pull the box down that far).
//! Slots without a portrait are not measured further, which keeps the
//! detector cheap in the frames without voice chat.

mod ink;

use blurrd_core::{NameKind, RectI, Region};

use crate::detection::{DetectError, Detection};
use crate::image::RgbView;

use self::ink::InkPlanes;

/// The left-panel crop of plan 5.4 in reference pixels (`HudRoi::LeftPanel`):
/// the voice-chat entries and the text chat.
pub const LEFT_PANEL_ROI: RectI = RectI {
    x0: 0,
    y0: 550,
    x1: 780,
    y1: 1040,
};

/// Number of entry slots (port spec section 5: two seen in the survey,
/// three in the test media, four supported).
pub const SLOTS: usize = 4;
/// Top of the bottom slot (slot 0).
pub const SLOT0_TOP: i32 = 678;
/// Distance between slot tops; the stack grows upwards.
pub const SLOT_PITCH: i32 = 40;
/// Left edge of a speaker name and of the location line below it.
pub const TEXT_X: i32 = 58;
/// Right end of the area a name may reach: 16 wide capitals of the name
/// font fit (a 15-character name measured 118 px, so 16 capital Ws stay
/// below 180 px).
pub const SLOT_X1: i32 = 250;

/// The portrait column of all four slots, in reference pixels: measured
/// in every frame.
const PORTRAIT_AREA: RectI = RectI {
    x0: PORTRAIT_COLS.0,
    y0: SLOT0_TOP - (SLOTS as i32 - 1) * SLOT_PITCH + PORTRAIT_ROWS.0,
    x1: PORTRAIT_COLS.1,
    y1: SLOT0_TOP + PORTRAIT_ROWS.1,
};
/// Columns of the text area of a slot (name and location line), measured
/// only when the slot shows a portrait.
const TEXT_COLS: (i32, i32) = (EDGE_SEARCH.0, SLOT_X1 + 4);
/// Rows of the text area relative to the slot top: from the slot's top
/// edge (glyphs above the name rows) to below the location line.
const TEXT_ROWS: (i32, i32) = (0, 38);
/// Portrait texture below which a slot shows no entry and its text area
/// is not measured: an entry needs a full portrait (1.0); half a portrait
/// is kept as evidence that an entry may still be shown.
pub const MEASURE_PORTRAIT: f32 = 0.5;

/// Ink above which a pixel is part of the name (port spec: 40).
const NAME_INK: u8 = 40;
/// Ink above which a pixel is part of the grey location line (25).
const LOCATION_INK: u8 = 25;
/// Ink above which a pixel is portrait texture (30).
const PORTRAIT_INK: u8 = 30;
/// Share of name pixels that counts as a full name (0.10).
const NAME_NORM: f32 = 0.10;
/// Share of location pixels that counts as a full location line (0.07).
const LOCATION_NORM: f32 = 0.07;
/// Share of portrait pixels that counts as a full portrait (0.15).
const PORTRAIT_NORM: f32 = 0.15;
/// Brightness lift of the name over its background that counts as full
/// (35, on the darkest channel).
const LIFT_NORM: f32 = 35.0;
/// Quantile of the darkest channel that stands for the name strokes.
const LIFT_QUANTILE: f32 = 0.97;
/// Share of ink in the gap rows between name and location above which
/// text runs through the slot (not an entry): the score is halved.
const GAP_MAX: f32 = 0.12;
/// Score at which a slot shows an entry.
const ON_SCORE: f32 = 1.0;
/// Name ink an entry found from its structure alone needs (a quarter of a
/// full name): a bright background can swallow most of a white name, but
/// an empty name area is not an entry.
const STRUCTURE_NAME_MIN: f32 = 0.25;

/// Name rows, relative to the slot top (`y + 5 .. y + 17`).
const NAME_ROWS: (i32, i32) = (5, 17);
/// Columns of the name measure (`x 58 .. 107`).
const NAME_COLS: (i32, i32) = (58, 107);
/// Location rows (`y + 23 .. y + 35`).
const LOCATION_ROWS: (i32, i32) = (23, 35);
/// Columns of the location measure (`x 58 .. 150`).
const LOCATION_COLS: (i32, i32) = (58, 150);
/// Portrait rows (`y + 2 .. y + 36`).
const PORTRAIT_ROWS: (i32, i32) = (2, 36);
/// Portrait columns (`x 16 .. 52`).
const PORTRAIT_COLS: (i32, i32) = (16, 52);
/// Gap rows between name and location (`y + 18 .. y + 21`).
const GAP_ROWS: (i32, i32) = (18, 21);
/// Rows searched for the name's extent (`y + 5 .. y + 19`, descenders
/// included).
const EXTENT_ROWS: (i32, i32) = (5, 19);
/// Where the search for a text line's left edge starts and ends. It
/// starts left of [`EDGE_ALIGNED`], so that a line or texture running
/// into the text column from the left is found there and fails the
/// alignment.
const EDGE_SEARCH: (i32, i32) = (54, 120);
/// A text line starts in these columns when it is aligned with
/// [`TEXT_X`]. Measured on the test recordings (7,180 entry frames): the
/// location line starts at x 56 to 58, the name at x 56 to 61 (63 for a
/// few thin first letters); the rendered fixture font starts both lines
/// up to two columns later.
const EDGE_ALIGNED: (i32, i32) = (56, 62);
/// Columns right of [`EDGE_ALIGNED`] where a name may still start when
/// its first glyph carries side bearing (thin letters, other scripts).
/// Such a name needs a clean gap below it and [`BEARING_SCORE`].
const NAME_BEARING_END: i32 = 64;
/// Name score a name starting in the side-bearing columns needs.
const BEARING_SCORE: f32 = 1.5;
/// Median of the darkest channel behind the name from which the
/// background counts as bright: a white name there may not stand out
/// (lift), so the entry can only be found from its structure.
const BRIGHT_BACKGROUND: u8 = 150;
/// Ink pixels a column of the name rows needs to count as a glyph stem
/// (capitals are 10 rows tall); horizontal edges of the background reach
/// only one to three pixels per column.
const NAME_STEM: u8 = 5;
/// Ink pixels a column of the location rows needs to count as a stem.
const LOCATION_STEM: u8 = 6;
/// Rows searched for the location line's left edge (`y + 23 .. y + 36`).
const LOCATION_EDGE_ROWS: (i32, i32) = (23, 36);
/// Largest gap between ink columns inside a name; a word gap of the name
/// font is about 4 px, a double space about 9 px.
const NAME_GAP: i32 = 12;

/// Left edge of a name box: two columns left of the first glyph column
/// (x 57 or 58), so the glyph halo lies inside the box, and right of the
/// portrait (which ends at x 51) by more than the blur's padding.
const BOX_X0: i32 = 56;
/// Box rows relative to the slot top: 3 rows above the name rows to 2 rows
/// above the location line (port spec: the box stays clear of the
/// location line, which is not a name).
pub const BOX_ROWS: (i32, i32) = (2, 21);
/// Box rows, relative to the slot top, of a name whose glyphs reach above
/// the name rows or below the Latin descenders (tone marks and vowel signs
/// of other scripts above, deep descenders below): one row above the
/// slot's top edge, and down into the top rows of the location line,
/// which a name of that script overlaps. The glyphs are searched in the
/// rows inside these limits.
const TALL_ROWS: (i32, i32) = (-1, 26);
/// Rows the box keeps beyond the outermost connected stroke row of a tall
/// name: the faint anti-aliased tip below the stroke threshold and the
/// glyph halo.
const TALL_MARGIN: i32 = 3;
/// Bright stroke pixels, connected to the name's strokes, a row next to
/// the name needs to count as part of it.
const ROW_INK_MIN: usize = 2;
/// Share of the name's lift over its background a stroke pixel needs in
/// the darkest channel to count as part of the white name: darker
/// background texture does not (the grey location line, at about half
/// the name's lift, is never reached: the search needs strokes connected
/// to the name from row to row).
const STROKE_LIFT: f32 = 0.6;
/// Columns the box reaches beyond the name's last ink.
const BOX_PAD_RIGHT: i32 = 6;

/// Side of the portrait signature grid.
pub const SIGNATURE_SIDE: usize = 12;
/// Smallest standard deviation (luma levels) a signature is normalised
/// by: flat signatures (no portrait) are not blown up into noise.
pub const SIGNATURE_CONTRAST: f32 = 4.0;
/// Inner portrait area the signature summarises (relative to the slot
/// top): the agent icon without its transparent corners.
const SIGNATURE_AREA: (i32, i32, i32, i32) = (22, 8, 46, 32);

/// A coarse luma image of a slot's portrait: [`SIGNATURE_SIDE`]² block
/// means. An entry keeps its portrait while it is shown, whatever happens
/// behind it, so the signature tells whether two frames show the same
/// entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortraitSignature(pub [u8; SIGNATURE_SIDE * SIGNATURE_SIDE]);

impl Default for PortraitSignature {
    fn default() -> Self {
        Self([0; SIGNATURE_SIDE * SIGNATURE_SIDE])
    }
}

impl PortraitSignature {
    /// Distance to `other`: the mean absolute difference of the two
    /// signatures after each is brought to zero mean and unit variance
    /// (with a floor of [`SIGNATURE_CONTRAST`] levels for flat ones), so a
    /// flash, a fade or a colour filter that changes brightness and
    /// contrast of the whole portrait does not change it. 0 for the same
    /// picture; two different agent portraits are about 1 apart.
    pub fn distance(&self, other: &Self) -> f32 {
        let (a, b) = (self.normalised(), other.normalised());
        let sum: f32 = a.iter().zip(&b).map(|(x, y)| (x - y).abs()).sum();
        sum / a.len() as f32
    }

    /// The signature with zero mean and unit variance.
    fn normalised(&self) -> [f32; SIGNATURE_SIDE * SIGNATURE_SIDE] {
        let n = self.0.len() as f32;
        let mean = self.0.iter().map(|&v| f32::from(v)).sum::<f32>() / n;
        let variance = self
            .0
            .iter()
            .map(|&v| (f32::from(v) - mean).powi(2))
            .sum::<f32>()
            / n;
        let scale = variance.sqrt().max(SIGNATURE_CONTRAST);
        self.0.map(|v| (f32::from(v) - mean) / scale)
    }
}

/// How a slot was found to show an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SlotHit {
    /// No entry.
    #[default]
    None,
    /// A complete entry: name, location line and portrait, both text lines
    /// aligned. The box ends at the name's last ink.
    Name,
    /// The portrait and the aligned location line of an entry whose name
    /// does not stand out from its background (white sky, flashes): the
    /// whole name slot is boxed (Safe geometry, plan 7.3).
    Structure,
}

/// What the detector measured in one slot of one frame.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SlotEvidence {
    /// Slot index (0 = bottom).
    pub slot: u8,
    /// Top of the slot in reference pixels.
    pub top: i32,
    /// Name ink, 1.0 = a full name. This and the other text measures are
    /// 0 when the portrait is below [`MEASURE_PORTRAIT`] (not measured).
    pub name: f32,
    /// Location-line ink, 1.0 = a full line.
    pub location: f32,
    /// Portrait texture, 1.0 = a full portrait: the detector-independent
    /// "voice portrait ink" structure signal of plan 7.2.
    pub portrait: f32,
    /// Brightness lift of the name over its background, 1.0 = full.
    pub lift: f32,
    /// Median of the darkest channel in the name band: the brightness of
    /// the background behind the name.
    pub background: u8,
    /// Whether text runs through the gap rows between name and location.
    pub gap_blocked: bool,
    /// Left edge of the name line, if one was found.
    pub name_edge: Option<i32>,
    /// Left edge of the location line, if one was found.
    pub location_edge: Option<i32>,
    /// The name score: the smallest of the four measures, halved when
    /// text runs through the gap rows.
    pub score: f32,
    /// Whether and how the slot shows an entry.
    pub hit: SlotHit,
    /// Box rows relative to the slot top: [`BOX_ROWS`], widened for
    /// glyphs above the name rows or below the Latin descenders.
    pub rows: (i32, i32),
    /// Right end of the name's ink (exclusive) for a [`SlotHit::Name`],
    /// the end of the name slot ([`SLOT_X1`]) for a
    /// [`SlotHit::Structure`], 0 without an entry.
    pub name_x1: i32,
    /// The portrait signature (always measured).
    pub signature: PortraitSignature,
}

impl SlotEvidence {
    /// Whether both text lines start at the speaker-name edge (the name
    /// possibly a few columns later, see [`SlotEvidence::name_starts`]).
    pub fn aligned(&self) -> bool {
        self.name_starts() && at_edge(self.location_edge)
    }

    /// Whether the name line starts at the speaker-name edge: at x 56 to
    /// 61, or at x 62 or 63 (side bearing of thin letters and other
    /// scripts) with a clean gap below it and a name score of 1.5.
    pub fn name_starts(&self) -> bool {
        match self.name_edge {
            Some(x) if x >= EDGE_ALIGNED.1 => {
                x < NAME_BEARING_END && !self.gap_blocked && self.score >= BEARING_SCORE
            }
            edge => at_edge(edge),
        }
    }

    /// Whether the slot shows the structure of an entry whose white name
    /// does not stand out from a bright background: portrait and location
    /// line in full, both lines aligned, some name ink, a clean gap.
    /// The faint name's own edge is not reliable (its first stems can
    /// vanish in the background); it only must not start left of the text
    /// column, which texture running in from the left does.
    fn shows_structure(&self) -> bool {
        let from_left = self.name_edge.is_some_and(|x| x < EDGE_ALIGNED.0);
        self.portrait.min(self.location) >= ON_SCORE
            && self.name >= STRUCTURE_NAME_MIN
            && self.background >= BRIGHT_BACKGROUND
            && !self.gap_blocked
            && !from_left
            && at_edge(self.location_edge)
    }

    /// Whether the slot shows an entry.
    pub fn is_entry(&self) -> bool {
        self.hit != SlotHit::None
    }

    /// The detection of this slot's entry (meaningful when
    /// [`SlotEvidence::is_entry`]).
    pub fn detection(&self) -> Detection {
        entry_detection(self.top, self.rows, self.name_x1)
    }
}

/// The detection of a speaker name in the slot at `top`: box rows `rows`
/// relative to the slot top ([`SlotEvidence::rows`]) and the name's ink
/// ending at `name_x1` (exclusive). The box runs from the portrait's edge
/// to a few columns past the name, at most to the end of the name slot.
pub fn entry_detection(top: i32, rows: (i32, i32), name_x1: i32) -> Detection {
    Detection {
        region: Region::VoiceChat,
        kind: NameKind::Speaker,
        text: RectI {
            x0: TEXT_X,
            y0: top + EXTENT_ROWS.0.min(rows.0 + 1),
            x1: name_x1.max(TEXT_X + 1),
            y1: top + EXTENT_ROWS.1.max(rows.1 - 1),
        },
        rect: RectI {
            x0: BOX_X0,
            y0: top + rows.0,
            x1: (name_x1 + BOX_PAD_RIGHT).min(SLOT_X1 + BOX_PAD_RIGHT),
            y1: top + rows.1,
        },
        row_top: Some(top),
    }
}

/// Whether a text line starting at `edge` is aligned with [`TEXT_X`].
fn at_edge(edge: Option<i32>) -> bool {
    edge.is_some_and(|x| (EDGE_ALIGNED.0..EDGE_ALIGNED.1).contains(&x))
}

/// Top of slot `k` in reference pixels.
pub const fn slot_top(k: usize) -> i32 {
    SLOT0_TOP - SLOT_PITCH * k as i32
}

/// Finds the speaker names of the voice-chat entries in one frame.
///
/// Reuse one detector for all frames of a video: it keeps its buffers.
#[derive(Debug, Clone, Default)]
pub struct VoiceDetector {
    portraits: InkPlanes,
    text: InkPlanes,
    slots: [SlotEvidence; SLOTS],
    columns: Vec<u8>,
    /// Scratch: bright strokes of the last connected row and of the row
    /// under test.
    strokes: [Vec<bool>; 2],
}

impl VoiceDetector {
    /// A detector with the thresholds of the port spec.
    pub fn new() -> Self {
        Self::default()
    }

    /// What the detector measured in every slot of the last frame, bottom
    /// slot first.
    pub fn slots(&self) -> &[SlotEvidence; SLOTS] {
        &self.slots
    }

    /// Detects the speaker names in `crop`, the [`LEFT_PANEL_ROI`] of a
    /// frame in reference scale, and replaces the content of `out` with
    /// them, bottom slot first.
    ///
    /// # Errors
    ///
    /// [`DetectError::CropSize`] if the crop is not the size of
    /// [`LEFT_PANEL_ROI`].
    pub fn detect(
        &mut self,
        crop: &RgbView<'_>,
        out: &mut Vec<Detection>,
    ) -> Result<(), DetectError> {
        out.clear();
        let (width, height) = (LEFT_PANEL_ROI.width(), LEFT_PANEL_ROI.height());
        if (crop.width(), crop.height()) != (width, height) {
            return Err(DetectError::CropSize {
                width,
                height,
                actual_width: crop.width(),
                actual_height: crop.height(),
            });
        }
        self.portraits.compute(crop, to_crop(PORTRAIT_AREA));
        for k in 0..SLOTS {
            let evidence = self.measure(crop, k);
            if evidence.is_entry() {
                out.push(evidence.detection());
            }
            self.slots[k] = evidence;
        }
        Ok(())
    }

    /// Measures slot `k` of `crop`: the portrait on the portrait planes,
    /// then, if the slot shows one, its text area.
    fn measure(&mut self, crop: &RgbView<'_>, k: usize) -> SlotEvidence {
        let top = slot_top(k);
        let share = |planes: &InkPlanes, rows: (i32, i32), cols: (i32, i32), ink: u8| {
            let (n, total) = planes.count_above(to_crop(band(top, rows, cols)), ink);
            if total == 0 {
                0.0
            } else {
                n as f32 / total as f32
            }
        };
        let portrait =
            share(&self.portraits, PORTRAIT_ROWS, PORTRAIT_COLS, PORTRAIT_INK) / PORTRAIT_NORM;
        let signature = self.signature(top);
        if portrait < MEASURE_PORTRAIT {
            return SlotEvidence {
                slot: k as u8,
                top,
                portrait,
                rows: BOX_ROWS,
                signature,
                ..SlotEvidence::default()
            };
        }
        self.text
            .compute(crop, to_crop(band(top, TEXT_ROWS, TEXT_COLS)));
        let p = &self.text;
        let name = share(p, NAME_ROWS, NAME_COLS, NAME_INK) / NAME_NORM;
        let location = share(p, LOCATION_ROWS, LOCATION_COLS, LOCATION_INK) / LOCATION_NORM;
        let gap_blocked = share(p, GAP_ROWS, NAME_COLS, NAME_INK) > GAP_MAX;
        let (high, background) = p
            .lmin_quantile_and_median(to_crop(band(top, NAME_ROWS, NAME_COLS)), LIFT_QUANTILE)
            .unwrap_or((0, 0));
        let lift = f32::from(high.saturating_sub(background)) / LIFT_NORM;
        let mut score = name.min(location).min(portrait).min(lift);
        if gap_blocked {
            score *= 0.5;
        }
        let name_edge = self.edge(top, EXTENT_ROWS, NAME_INK, NAME_STEM);
        let location_edge = self.edge(top, LOCATION_EDGE_ROWS, LOCATION_INK, LOCATION_STEM);
        let mut evidence = SlotEvidence {
            slot: k as u8,
            top,
            name,
            location,
            portrait,
            lift,
            background,
            gap_blocked,
            name_edge,
            location_edge,
            score,
            hit: SlotHit::None,
            rows: BOX_ROWS,
            name_x1: 0,
            signature,
        };
        if score >= ON_SCORE && evidence.aligned() {
            evidence.hit = SlotHit::Name;
            evidence.name_x1 = self.name_end(top, name_edge.unwrap_or(TEXT_X));
            let white = background
                .saturating_add((STROKE_LIFT * f32::from(high.saturating_sub(background))) as u8);
            evidence.rows = self.name_rows(top, evidence.name_x1, white);
        } else if evidence.shows_structure() {
            evidence.hit = SlotHit::Structure;
            evidence.name_x1 = SLOT_X1;
        }
        evidence
    }

    /// The first column of `rows` of the slot at `top` with at least
    /// `stem` pixels of ink above `ink`.
    ///
    /// A run of stem columns that starts at the search start and ends
    /// before [`EDGE_ALIGNED`] with an empty column after it is the halo of
    /// a hard portrait edge (the ink measure spreads an edge over about
    /// three columns) and is skipped; texture running into the text column
    /// from the left has no such gap and is reported where it starts.
    fn edge(&mut self, top: i32, rows: (i32, i32), ink: u8, stem: u8) -> Option<i32> {
        let rect = to_crop(band(top, rows, EDGE_SEARCH));
        self.text.column_counts(rect, ink, &mut self.columns);
        let is_stem = |n: &u8| *n >= stem;
        let first = self.columns.iter().position(is_stem)?;
        let spill = (EDGE_ALIGNED.0 - EDGE_SEARCH.0) as usize;
        let run_end = first
            + self.columns[first..]
                .iter()
                .position(|n| !is_stem(n))
                .unwrap_or(self.columns.len() - first);
        let start = if first == 0 && run_end <= spill {
            run_end + self.columns[run_end..].iter().position(is_stem)?
        } else {
            first
        };
        Some(EDGE_SEARCH.0 + start as i32)
    }

    /// The box rows of a name ending at `x1` whose strokes are at least
    /// `white` bright in the darkest channel: [`BOX_ROWS`], widened up to
    /// [`TALL_ROWS`] while the rows next to the name rows hold bright
    /// strokes connected to the name's strokes (one empty row is crossed:
    /// tone marks float above their letter).
    fn name_rows(&mut self, top: i32, x1: i32, white: u8) -> (i32, i32) {
        let up = self
            .stroke_extent(top, x1, white, EXTENT_ROWS.0, TALL_ROWS.0 + 1, -1)
            .map_or(BOX_ROWS.0, |r| (r - TALL_MARGIN).max(TALL_ROWS.0));
        let down = self
            .stroke_extent(top, x1, white, EXTENT_ROWS.1 - 1, TALL_ROWS.1 - 1, 1)
            .map_or(BOX_ROWS.1, |r| (r + 1 + TALL_MARGIN).min(TALL_ROWS.1));
        (up, down)
    }

    /// The outermost row (relative to the slot top) reached from the name
    /// row `from` towards `limit` (inclusive) in steps of `step` through
    /// rows with connected bright strokes, or `None`.
    fn stroke_extent(
        &mut self,
        top: i32,
        x1: i32,
        white: u8,
        from: i32,
        limit: i32,
        step: i32,
    ) -> Option<i32> {
        let crop_row = |r: i32| top + r - LEFT_PANEL_ROI.y0;
        let x0 = TEXT_X - LEFT_PANEL_ROI.x0;
        let x1 = x1 - LEFT_PANEL_ROI.x0;
        let [last, row] = &mut self.strokes;
        self.text
            .bright_strokes(crop_row(from), x0, x1, NAME_INK, white, last);
        let mut reached = None;
        let mut gap = false;
        let mut r = from;
        while r != limit {
            r += step;
            self.text
                .bright_strokes(crop_row(r), x0, x1, NAME_INK, white, row);
            let connected = (0..row.len())
                .filter(|&i| {
                    row[i] && last[i.saturating_sub(1)..(i + 2).min(last.len())].contains(&true)
                })
                .count();
            if connected >= ROW_INK_MIN {
                reached = Some(r);
                gap = false;
                std::mem::swap(last, row);
            } else if gap {
                break;
            } else {
                gap = true;
            }
        }
        reached
    }

    /// The right end (exclusive) of the name that starts at `start`: the
    /// last ink column of the chain of columns with gaps of at most
    /// [`NAME_GAP`], up to [`SLOT_X1`].
    fn name_end(&mut self, top: i32, start: i32) -> i32 {
        let rect = to_crop(band(top, EXTENT_ROWS, (start, SLOT_X1)));
        self.text.column_counts(rect, NAME_INK, &mut self.columns);
        let mut last = start;
        for (i, &n) in self.columns.iter().enumerate() {
            let x = start + i as i32;
            if x - last > NAME_GAP {
                break;
            }
            if n > 0 {
                last = x;
            }
        }
        last + 1
    }

    /// The portrait signature of the slot at `top`.
    fn signature(&self, top: i32) -> PortraitSignature {
        let (x0, y0, x1, y1) = SIGNATURE_AREA;
        let (bw, bh) = (
            (x1 - x0) / SIGNATURE_SIDE as i32,
            (y1 - y0) / SIGNATURE_SIDE as i32,
        );
        let mut out = PortraitSignature::default();
        for (i, cell) in out.0.iter_mut().enumerate() {
            let (cx, cy) = ((i % SIGNATURE_SIDE) as i32, (i / SIGNATURE_SIDE) as i32);
            let mut sum = 0u32;
            for y in 0..bh {
                for x in 0..bw {
                    let rx = x0 + cx * bw + x - LEFT_PANEL_ROI.x0;
                    let ry = top + y0 + cy * bh + y - LEFT_PANEL_ROI.y0;
                    sum += u32::from(self.portraits.luma(rx, ry));
                }
            }
            let n = (bw * bh) as u32;
            *cell = ((sum + n / 2) / n) as u8;
        }
        out
    }
}

/// The reference rectangle of `rows` (relative to `top`) × `cols`.
const fn band(top: i32, rows: (i32, i32), cols: (i32, i32)) -> RectI {
    RectI {
        x0: cols.0,
        y0: top + rows.0,
        x1: cols.1,
        y1: top + rows.1,
    }
}

/// A reference rectangle in crop coordinates.
const fn to_crop(r: RectI) -> RectI {
    RectI {
        x0: r.x0 - LEFT_PANEL_ROI.x0,
        y0: r.y0 - LEFT_PANEL_ROI.y0,
        x1: r.x1 - LEFT_PANEL_ROI.x0,
        y1: r.y1 - LEFT_PANEL_ROI.y0,
    }
}
