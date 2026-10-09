//! ACE and clutch banner names (port spec section 4).
//!
//! When a round ends with an ace, a clutch or a similar feat, the game
//! shows a framed box with a large title in the top centre. Under it a
//! grey plate with the player's name slides in from the right, slows down
//! and settles centred on x 960:
//!
//! ```text
//!            ┌───────────────────────────────┐
//!            │              ACE              │
//!            └▀▀────────────────────────────▀▀┘   ← bright corner ticks, y 280..285
//!                 ▓▓▓▓▓▓▓  N A M E  ▓▓▓▓▓▓▓        ← grey plate, y 298..328
//! ```
//!
//! [`BannerDetector`] reads one frame: the corner ticks of the box gate the
//! detector ([`BannerFrame::gate`]), and while they are on screen the
//! white text on the plate is the name ([`BannerName`]). Text counts as a
//! name only on a plate ([`Plate`], the plate gate): the "X ULT READY"
//! notification and bright scenery in the same place have none. Which frames get
//! a box, and how large, is decided over the whole banner episode by
//! [`BannerTracker`]: the settle model of the port spec, which needs the
//! frames after the name has stopped moving.
//!
//! The banner is white and grey, so team colours and colour-blind
//! palettes do not change it. The thresholds follow the frame instead:
//! the corner ticks are the brightest white of the banner, and their level
//! sets the brightness thresholds of the name, so darkening filters of GPU
//! drivers (brightness, contrast, gamma) lower them together. The ticks
//! themselves pass either by absolute brightness (bright backgrounds) or by
//! their contrast to the rows above and below (darkened video).

mod plate;
mod tracker;

use blurrd_core::{NameKind, RectI, Region};

pub use plate::Plate;
pub use tracker::{BannerBox, BannerTracker};

use self::plate::{PlateScratch, find_plate};

use crate::components::{Component, ComponentScratch, Connectivity, components};
use crate::detection::{DetectError, Detection};
use crate::image::{BitMask, GrayImage, RgbView};
use crate::killfeed::FEED_ROI;
use crate::morph::{MorphScratch, white_tophat};

/// The banner name is centred on this column once it has settled.
pub const CENTRE_X: i32 = 960;

/// The strip searched for the name (`BX0..BX1`, `BY0..BY1`): the plate's
/// rows, and wide on the right for the slide-in.
pub const NAME_STRIP: RectI = RectI {
    x0: 690,
    y0: 299,
    x1: 1400,
    y1: 328,
};

/// The rows a banner name's blur box covers: the plate's inner band.
pub const NAME_BAND: (i32, i32) = (300, 327);

/// Horizontal padding of a name's blur box around its text (the settle
/// model's `+ 6`).
pub const NAME_PAD_X: i32 = 6;

/// Rows searched for the corner ticks of the banner box (the ticks are
/// rows 280..285, measured; the prototype also reads the dark rows under
/// them).
const TICK_ROWS: (i32, i32) = (280, 288);
/// Columns of the left and right corner tick.
const TICK_COLUMNS: [(i32, i32); 2] = [(698, 716), (1204, 1222)];
/// The rows above and below the ticks that their contrast is measured
/// against.
const TICK_SURROUND_ROWS: (i32, i32) = (276, 291);
/// A tick pixel is at least this bright (`mn ≥ 205`)...
const TICK_MIN: u8 = 205;
/// ... or at least this bright and [`TICK_CONTRAST`] brighter than the
/// rows above and below it (a darkened video).
const TICK_MIN_RELATIVE: u8 = 150;
/// See [`TICK_MIN_RELATIVE`]. Unfiltered ticks are 60 to 230 levels above
/// the brighter of their neighbouring rows (measured on 25 banners), only
/// bright skies come closer, and those pass by [`TICK_MIN`].
const TICK_CONTRAST: u8 = 50;
/// Each corner needs at least this many tick pixels.
const TICK_COUNT: u32 = 6;
/// A column counts for the contrast rule with at least this many tick
/// pixels (the ticks are 5 rows tall)...
const TICK_MIN_HEIGHT: u32 = 3;
/// ... and a tick needs at least this many such columns.
const TICK_MIN_COLUMNS: u32 = 2;
/// The name thresholds move down once the ticks are darker than this.
/// Unfiltered ticks average 240 to 253 once the box is fully shown and
/// 215 to 230 in the first frames of its fade-in (measured on 25
/// banners), so unfiltered video keeps the tuned thresholds apart from a
/// few fade-in frames, where no name is shown yet.
const TICK_WHITE: u8 = 225;
/// The name thresholds move down by at most this much with the ticks.
const MAX_WHITE_SHIFT: u8 = 60;

/// Side of the square top-hat kernel of the name strokes.
const TOPHAT_SIZE: u32 = 7;
/// Name strokes: top-hat of `mn` at least this...
const NAME_MIN_TOPHAT: u8 = 20;
/// ... `mn` at least this...
const NAME_MIN_MN: u8 = 175;
/// ... and chroma at most this.
const NAME_MAX_CHROMA: u8 = 60;
/// Glyph components have at least this much ink...
const GLYPH_MIN_AREA: u32 = 3;
/// ... a height in this range...
const GLYPH_H: (i32, i32) = (3, 16);
/// ... at most this width...
const GLYPH_MAX_W: i32 = 22;
/// ... their vertical centre in this range of strip rows...
const GLYPH_CENTRE: (i32, i32) = (7, 19);
/// ... and end at least two rows above the strip bottom.
const GLYPH_MAX_BOTTOM: i32 = 27;
/// Glyphs closer than this belong to one name.
const NAME_GAP: i32 = 24;
/// A name is at least this wide...
const NAME_MIN_W: i32 = 10;
/// ... has at least this much ink...
const NAME_MIN_INK: u32 = 20;
/// ... and a height in this range.
const NAME_H: (i32, i32) = (4, 17);
/// A name whose centre is at most this far from [`CENTRE_X`] is centred:
/// its span is mirrored around the centre, which covers faint letters at
/// either end.
const CENTRED_TOLERANCE: i32 = 25;

/// The name found on the banner plate in one frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BannerName {
    /// Tight bounds of the name's text strokes.
    pub text: RectI,
    /// The name's horizontal extent: the text, mirrored around
    /// [`CENTRE_X`] when the name is centred (then the span's centre is
    /// exactly [`CENTRE_X`]).
    pub span: RectI,
    /// Number of stroke pixels.
    pub ink: u32,
    /// The plate the name is written on.
    pub plate: Plate,
}

impl BannerName {
    /// Whether the name is centred (within 25 px of [`CENTRE_X`]): it has
    /// settled, or is about to.
    pub fn is_centred(&self) -> bool {
        self.span.x0 + self.span.x1 == 2 * CENTRE_X
    }

    /// The per-frame detection: the span padded by [`NAME_PAD_X`] over the
    /// plate band [`NAME_BAND`].
    pub fn detection(&self) -> Detection {
        Detection {
            region: Region::Banner,
            kind: NameKind::Banner,
            text: self.text,
            rect: RectI {
                x0: self.span.x0 - NAME_PAD_X,
                y0: NAME_BAND.0.min(self.text.y0),
                x1: self.span.x1 + NAME_PAD_X,
                y1: NAME_BAND.1.max(self.text.y1),
            },
            row_top: None,
        }
    }
}

/// What the banner detector saw in one frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct BannerFrame {
    /// Whether the corner ticks of a banner box are on screen. A
    /// detector-independent structure signal (plan 7.2).
    pub gate: bool,
    /// The plate, if the gate is on and a plate was found. It is shown
    /// with the name, and can be found where motion or effects hide the
    /// name's letters.
    pub plate: Option<Plate>,
    /// The name on the plate, if the gate is on and a name was found on
    /// the plate.
    pub name: Option<BannerName>,
}

/// Reusable buffers.
#[derive(Debug, Clone, Default)]
struct Scratch {
    mn: GrayImage,
    chroma: GrayImage,
    tophat: GrayImage,
    morph: MorphScratch,
    strokes: Vec<u8>,
    components: ComponentScratch,
    found: Vec<Component>,
    glyphs: Vec<Glyph>,
    plate: PlateScratch,
}

/// A glyph component in reference pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Glyph {
    x0: i32,
    x1: i32,
    top: i32,
    bottom: i32,
    area: u32,
}

impl Glyph {
    /// The glyph's (or cluster's) bounds.
    const fn rect(&self) -> RectI {
        RectI {
            x0: self.x0,
            y0: self.top,
            x1: self.x1,
            y1: self.bottom,
        }
    }
}

/// Finds the banner name in one frame.
///
/// Reuse one detector for all frames of a video: it keeps its buffers.
#[derive(Debug, Clone, Default)]
pub struct BannerDetector {
    scratch: Scratch,
}

impl BannerDetector {
    /// A detector with the thresholds of the port spec.
    pub fn new() -> Self {
        Self::default()
    }

    /// Reads the banner in `crop`, the [`FEED_ROI`] of a frame in reference
    /// scale (the kill-feed crop: the banner lies inside it).
    ///
    /// # Errors
    ///
    /// [`DetectError::CropSize`] if the crop is not the size of
    /// [`FEED_ROI`].
    pub fn detect(&mut self, crop: &RgbView<'_>) -> Result<BannerFrame, DetectError> {
        let (width, height) = (FEED_ROI.width(), FEED_ROI.height());
        if (crop.width(), crop.height()) != (width, height) {
            return Err(DetectError::CropSize {
                width,
                height,
                actual_width: crop.width(),
                actual_height: crop.height(),
            });
        }
        let Some(white) = gate(crop) else {
            return Ok(BannerFrame::default());
        };
        let shift = TICK_WHITE.saturating_sub(white).min(MAX_WHITE_SHIFT);
        // Contrasts scale with the white level, like a contrast change.
        let scaled = |level: u8| {
            u8::try_from(u16::from(level) * u16::from(TICK_WHITE - shift) / u16::from(TICK_WHITE))
                .unwrap_or(level)
        };
        let thresholds = Thresholds {
            min_mn: NAME_MIN_MN - shift,
            min_tophat: scaled(NAME_MIN_TOPHAT),
        };
        let plate = find_plate(crop, scaled(plate::EDGE_MIN), &mut self.scratch.plate);
        let name = plate.and_then(|plate| find_name(crop, thresholds, plate, &mut self.scratch));
        Ok(BannerFrame {
            gate: true,
            plate,
            name,
        })
    }
}

/// Brightness thresholds of the name strokes in one frame.
#[derive(Debug, Clone, Copy)]
struct Thresholds {
    min_mn: u8,
    min_tophat: u8,
}

/// `min(R, G, B)` of a crop pixel at reference coordinates.
fn mn_at(crop: &RgbView<'_>, x: i32, y: i32) -> u8 {
    let rgb = crop.pixel((x - FEED_ROI.x0) as u32, (y - FEED_ROI.y0) as u32);
    rgb[0].min(rgb[1]).min(rgb[2])
}

/// The banner gate (`banner_gate`): both corner ticks of the banner box.
/// Returns the mean level of the tick pixels, or `None` if a tick is
/// missing.
fn gate(crop: &RgbView<'_>) -> Option<u8> {
    let (mut sum, mut total) = (0u32, 0u32);
    for columns in TICK_COLUMNS {
        let (count, level) = tick(crop, columns)?;
        sum += level;
        total += count;
    }
    // The mean of `u8` values fits.
    u8::try_from(sum / total).ok()
}

/// One corner tick in columns `x0..x1`: the number of tick pixels and the
/// sum of their levels, or `None` if the tick is missing.
///
/// The prototype's rule (at least [`TICK_COUNT`] pixels of
/// `mn ≥ TICK_MIN`) decides first. Otherwise tick pixels are those
/// [`TICK_CONTRAST`] brighter than the rows above and below, counted only
/// in columns with at least [`TICK_MIN_HEIGHT`] of them: a tick is a band
/// several rows tall, a thin horizontal line of the game world or another
/// overlay is not.
fn tick(crop: &RgbView<'_>, (x0, x1): (i32, i32)) -> Option<(u32, u32)> {
    let (mut count, mut level) = (0, 0);
    let (mut relative, mut relative_level, mut relative_columns) = (0, 0, 0);
    for x in x0..x1 {
        let surround =
            mn_at(crop, x, TICK_SURROUND_ROWS.0).max(mn_at(crop, x, TICK_SURROUND_ROWS.1));
        let (mut column, mut column_level) = (0, 0);
        for y in TICK_ROWS.0..TICK_ROWS.1 {
            let v = mn_at(crop, x, y);
            if v >= TICK_MIN {
                count += 1;
                level += u32::from(v);
            }
            if v >= TICK_MIN_RELATIVE && v >= surround.saturating_add(TICK_CONTRAST) {
                column += 1;
                column_level += u32::from(v);
            }
        }
        if column >= TICK_MIN_HEIGHT {
            relative += column;
            relative_level += column_level;
            relative_columns += 1;
        }
    }
    if count >= TICK_COUNT {
        Some((count, level))
    } else if relative >= TICK_COUNT && relative_columns >= TICK_MIN_COLUMNS {
        Some((relative, relative_level))
    } else {
        None
    }
}

/// The name on `plate` (`detect_banner`): the text-like cluster of stroke
/// components on the plate with the most ink.
fn find_name(
    crop: &RgbView<'_>,
    t: Thresholds,
    plate: Plate,
    s: &mut Scratch,
) -> Option<BannerName> {
    stroke_mask(crop, t, s);
    find_glyphs(s);
    let name = best_cluster(&mut s.glyphs, plate)?;
    let height = name.bottom - name.top;
    let text = name.rect();
    if name.x1 - name.x0 < NAME_MIN_W
        || name.area < NAME_MIN_INK
        || !(NAME_H.0..=NAME_H.1).contains(&height)
    {
        return None;
    }
    let mut span = text;
    // `|centre − 960| ≤ 25` with the centre at `(x0 + x1) / 2`.
    if (text.x0 + text.x1 - 2 * CENTRE_X).abs() <= 2 * CENTRED_TOLERANCE {
        let half = (CENTRE_X - text.x0).max(text.x1 - CENTRE_X);
        span.x0 = CENTRE_X - half;
        span.x1 = CENTRE_X + half;
    }
    Some(BannerName {
        text,
        span,
        ink: name.area,
        plate,
    })
}

/// Fills `s.strokes` with the name-stroke mask of [`NAME_STRIP`] (one byte
/// per pixel, 1 = stroke).
fn stroke_mask(crop: &RgbView<'_>, t: Thresholds, s: &mut Scratch) {
    let strip = NAME_STRIP;
    let (w, h) = (strip.width(), strip.height());
    s.mn.resize(w, h);
    s.chroma.resize(w, h);
    let start = 3 * (strip.x0 - FEED_ROI.x0) as usize;
    for y in 0..h {
        let row = crop.row((strip.y0 - FEED_ROI.y0) as u32 + y);
        let pixels = row[start..start + 3 * w as usize].as_chunks::<3>().0;
        let out = y as usize * w as usize..(y as usize + 1) * w as usize;
        for ((mn, chroma), rgb) in s.mn.data_mut()[out.clone()]
            .iter_mut()
            .zip(&mut s.chroma.data_mut()[out])
            .zip(pixels)
        {
            *mn = rgb[0].min(rgb[1]).min(rgb[2]);
            *chroma = rgb[0].max(rgb[1]).max(rgb[2]) - *mn;
        }
    }
    // The top-hat runs on the strip alone, borders included, as in the
    // prototype.
    white_tophat(&s.mn, TOPHAT_SIZE, TOPHAT_SIZE, &mut s.tophat, &mut s.morph);
    s.strokes.clear();
    s.strokes.extend(
        s.tophat
            .data()
            .iter()
            .zip(s.mn.data())
            .zip(s.chroma.data())
            .map(|((&th, &mn), &chroma)| {
                u8::from(th >= t.min_tophat && mn >= t.min_mn && chroma <= NAME_MAX_CHROMA)
            }),
    );
}

/// Collects the glyph-like components of the stroke mask into
/// `s.glyphs` (reference pixels).
fn find_glyphs(s: &mut Scratch) {
    let strip = NAME_STRIP;
    let (w, h) = (strip.width(), strip.height());
    let mask = BitMask::from_plane(&s.strokes, 1, w, h);
    let roi = RectI {
        x0: 0,
        y0: 0,
        x1: w as i32,
        y1: h as i32,
    };
    components(
        &mask,
        roi,
        Connectivity::Eight,
        &mut s.components,
        &mut s.found,
    );
    s.glyphs.clear();
    s.glyphs.extend(s.found.iter().filter_map(|c| {
        let b = c.bounds;
        let (width, height) = (b.x1 - b.x0, b.y1 - b.y0);
        let centre2 = 2 * b.y0 + height;
        (c.area >= GLYPH_MIN_AREA
            && (GLYPH_H.0..=GLYPH_H.1).contains(&height)
            && width <= GLYPH_MAX_W
            && (2 * GLYPH_CENTRE.0..=2 * GLYPH_CENTRE.1).contains(&centre2)
            && b.y1 <= GLYPH_MAX_BOTTOM)
            .then_some(Glyph {
                x0: b.x0 + strip.x0,
                x1: b.x1 + strip.x0,
                top: b.y0 + strip.y0,
                bottom: b.y1 + strip.y0,
                area: c.area,
            })
    }));
}

/// Clusters the glyphs (sorted by `x0`, stable like the prototype's sort)
/// over gaps of at most [`NAME_GAP`] and returns the first cluster with the
/// most ink among those `plate` carries.
fn best_cluster(glyphs: &mut [Glyph], plate: Plate) -> Option<Glyph> {
    glyphs.sort_by_key(|g| g.x0);
    let mut best: Option<Glyph> = None;
    let mut consider = |c: Glyph| {
        if plate.carries(&c.rect()) {
            best = Some(more_ink(best, c));
        }
    };
    let mut current: Option<Glyph> = None;
    for g in glyphs.iter() {
        current = match current {
            Some(c) if g.x0 - c.x1 <= NAME_GAP => Some(Glyph {
                x0: c.x0,
                x1: c.x1.max(g.x1),
                top: c.top.min(g.top),
                bottom: c.bottom.max(g.bottom),
                area: c.area + g.area,
            }),
            Some(c) => {
                consider(c);
                Some(*g)
            }
            None => Some(*g),
        };
    }
    if let Some(c) = current {
        consider(c);
    }
    best
}

/// The cluster with more ink; the earlier one on a tie.
fn more_ink(best: Option<Glyph>, c: Glyph) -> Glyph {
    match best {
        Some(b) if b.area >= c.area => b,
        _ => c,
    }
}
