//! The names inside one kill-feed row (port spec 3.2 and 3.3).
//!
//! All coordinates are reference pixels; masks are read through the crop
//! offset of [`FEED_ROI`].

use blurrd_core::{NameKind, RectI};

use super::tones::{self, Sample, Tone};
use super::{
    BarEvidence, FEED_ROI, FEED_X0, FeedRow, INNER_MARGIN, RIGHT_ALIGN, ROW_H, Scratch,
    VICTIM_COLOUR_X0, VICTIM_TEXT_X1, VICTIM_X1,
};
use crate::classes::{Class, PixelClasses};
use crate::components::{Component, Connectivity, components};

/// Text components larger than this are portrait or frame pieces.
const MAX_COMP_H: i32 = 16;
/// See [`MAX_COMP_H`].
const MAX_COMP_W: i32 = 30;
/// A wider component is a run of glyphs merged into one when it is at
/// most this wide (a 16-character name, see [`NAME_REACH`]), at least
/// [`MIN_RUN_H`] tall and fills at most [`MAX_RUN_FILL`] of its bounds:
/// a frame scaled down horizontally (a 16:9 game saved as 1024 x 768)
/// blurs the gaps between letters shut (measured: four letters of a real
/// killer name in one component 45 px wide, 0.57 filled, so only its
/// last glyph was blurred for 300 frames). Bar and portrait pieces of
/// glyph height are solid.
const MAX_RUN_W: i32 = NAME_REACH;
/// See [`MAX_RUN_W`]: a run spans at least the x-height.
const MIN_RUN_H: i32 = 8;
/// See [`MAX_RUN_W`].
const MAX_RUN_FILL: f32 = 0.7;
/// Components with less ink are noise.
const MIN_COMP_AREA: u32 = 3;
/// Components starting this close to the row top touch the row edge.
const MIN_COMP_TOP: i32 = 8;
/// Components ending this close to the row bottom touch the row edge.
const MIN_COMP_BOTTOM_GAP: i32 = 7;
/// A glyph whose top is at most this far below the row top starts at or
/// above the x-height: the line's lowercase glyphs start at + 14 to + 15,
/// up to [`MAX_LINE_OFFSET`] rows lower where the row was found high
/// (measured: a "y" from + 16 to + 27 at the end of a real killer name).
const X_HEIGHT_TOP: i32 = 15 + MAX_LINE_OFFSET;
/// Lowest end of a descender below the row top (`_on_line`: + 28). Deep
/// descenders (measured: Cyrillic у and ф on a real row) end at + 27,
/// inside [`MIN_COMP_BOTTOM_GAP`].
const DESCENDER_BOTTOM: i32 = 28;
/// The baseline of the text line below the row top (port spec: + 24).
const BASELINE: i32 = 24;
/// Lowest bottom of a glyph on the text line below the row top: two rows
/// below [`BASELINE`] (measured: the lower part of a CJK glyph at the end
/// of a real victim name ends one row below it, and a sharpening filter
/// moves it one row further).
const BASELINE_LOW: i32 = BASELINE + 2;
/// Most rows the text line may sit below the row top the row finder
/// reports (measured: under a gamma 1.5 filter the anchor of a real row
/// landed one row high and every lowercase glyph ended at + 25).
const MAX_LINE_OFFSET: i32 = 2;
/// Glyphs with at least this much ink set the line's baseline.
const BASELINE_GLYPH: u32 = 20;
/// A component with at least this much ink is a "big" glyph.
const BIG_GLYPH: u32 = 12;
/// Clustering gap of killer glyphs and one-colour rows.
const GAP_NAME: i32 = 12;
/// Clustering gap of victim glyphs.
const GAP_VICTIM: i32 = 14;
/// Words of one victim name are at most this far apart.
const GAP_WORDS: i32 = 25;
/// Share of the non-white pixels of a column that must have a team
/// colour to classify the column.
const COLOUR_SHARE: f64 = 0.45;
/// Consecutive killer-colour columns that mark the colour boundary.
const BOUNDARY_RUN: u32 = 3;
/// Icons of one-colour rows are at least this tall.
const ICON_MIN_H: i32 = 22;
/// ... and at most this wide (the round kill icon is about 24 px). Wider
/// tall white components are background, for example a bright sky that
/// touches the row band above or below the bar; the prototype took them
/// for icons and dropped every name of the row.
const ICON_MAX_W: i32 = 40;
/// ... and at most this tall. A white component over the whole row height
/// is the bar itself: in greyscale video a light bar passes as white, and
/// the strip between the victim name and the portrait would pass as an
/// icon and drop the name's last glyph.
const ICON_MAX_H: i32 = 30;
/// Margin around such an icon in which glyphs are dropped.
const ICON_MARGIN: i32 = 3;
/// The victim text starts this far right of the colour boundary.
const VICTIM_START: i32 = 2;
/// Victim text starting at most this far right of [`VICTIM_START`] touches
/// the colour boundary (a correct boundary lies a slanted bar edge before
/// the name).
const BOUNDARY_TOUCH: i32 = 3;
/// A victim name that touches the boundary is read again from this far
/// left of it: a 16-character name is about 160 px wide.
const NAME_REACH: i32 = 160;
/// A colour boundary at least this far left of the victim name's start is
/// misplaced for the weapon search (see [`killer`]): the victim text
/// starts [`VICTIM_START`] right of a correct boundary.
const MISPLACED_BOUNDARY: i32 = 20;
/// A killer name ends at most this far left of the weapon.
const MAX_KILLER_TO_WEAPON: i32 = 48;
/// The killer text search stops this far left of the weapon.
const WEAPON_GAP: i32 = 6;
/// White pieces at most this far left of the weapon blob belong to the
/// icon (see [`weapon_x`]).
const WEAPON_JOIN: i32 = 2;
/// Without a weapon blob, the killer text search stops this far left of
/// the colour boundary (knife and ability icons sit in between).
const ICON_ZONE: i32 = 40;
/// A learned victim colour must cover at least this share of the
/// non-white pixels next to the victim name...
pub(super) const VICTIM_TONE_SHARE: f32 = 0.5;
/// ... and at least this many pixels (of about 1,200).
pub(super) const VICTIM_TONE_PIXELS: u32 = 300;
/// The killer colour is learned from this column to [`VICTIM_COLOUR_X0`]:
/// the colour boundary of a 16-character victim name lies at about x 1680,
/// and the killer bar shows between its name, weapon and icons.
const KILLER_TONE_X0: i32 = 1490;
/// A learned killer colour must cover at least this share of the coloured
/// pixels there that do not have the victim colour (the killer portrait
/// and the background left of the bar take the rest)...
const KILLER_TONE_SHARE: f32 = 0.35;
/// ... and at least this many pixels.
const KILLER_TONE_PIXELS: u32 = 120;

/// A text component in reference pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Comp {
    x0: i32,
    x1: i32,
    top: i32,
    bottom: i32,
    area: u32,
}

/// A run of components (sorted by `x0`) with small gaps: a word or a name.
/// `start..end` indexes the sorted component list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Cluster {
    x0: i32,
    x1: i32,
    start: usize,
    end: usize,
}

/// Column profiles of a row's inner band: pixels per column with each
/// class, and with the learned victim and killer colours.
#[derive(Debug, Clone, Default)]
pub(super) struct Profile {
    teal: Vec<u32>,
    red: Vec<u32>,
    white: Vec<u32>,
    victim: Vec<u32>,
    killer: Vec<u32>,
}

/// Finds the components of `class` inside reference rectangle `r`, with
/// the mask cropped to `r`, into `s.found` (crop coordinates).
fn find_components(classes: &PixelClasses, class: Class, r: RectI, s: &mut Scratch) {
    let roi = RectI {
        x0: r.x0 - FEED_ROI.x0,
        y0: r.y0 - FEED_ROI.y0,
        x1: r.x1 - FEED_ROI.x0,
        y1: r.y1 - FEED_ROI.y0,
    };
    components(
        &classes.mask(class),
        roi,
        Connectivity::Eight,
        &mut s.components,
        &mut s.found,
    );
}

/// A component found by [`find_components`] in reference pixels.
fn to_comp(c: &Component) -> Comp {
    Comp {
        x0: c.bounds.x0 + FEED_ROI.x0,
        x1: c.bounds.x1 + FEED_ROI.x0,
        top: c.bounds.y0 + FEED_ROI.y0,
        bottom: c.bounds.y1 + FEED_ROI.y0,
        area: c.area,
    }
}

/// Text-like components of the stroke mask in the row band at `top`
/// between columns `x0..x1` (`_text_comps`), into `s.comps`.
///
/// The port spec drops every component that ends inside the bottom gap,
/// which also drops a deep descender at the end of a name: a victim name
/// then ends short of the right alignment and the whole row reported
/// nothing (measured on a real recording: a row with a Cyrillic victim
/// name ending in ф stayed unblurred for 58 frames), and a killer name
/// loses its last "y". Components that start at or right of
/// `descenders_from`, at or above the x-height, and end by
/// [`DESCENDER_BOTTOM`] are kept: right of the colour boundary for the
/// victim, next to the portrait in a one-colour row, the whole killer
/// side (whose search ends left of the weapon icon, whose pieces reach as
/// deep).
fn text_comps(
    classes: &PixelClasses,
    top: i32,
    x0: i32,
    x1: i32,
    descenders_from: i32,
    s: &mut Scratch,
) {
    s.comps.clear();
    if x1 - x0 < 3 {
        return;
    }
    let band = RectI {
        x0,
        y0: top,
        x1,
        y1: top + ROW_H,
    };
    find_components(classes, Class::Text, band, s);
    s.comps.extend(
        s.found
            .iter()
            .map(to_comp)
            .filter(|c| is_text_comp(c, top, descenders_from)),
    );
}

/// Whether a component of the row at `top` is text-like for
/// [`text_comps`]: glyph-sized (or a run of merged glyphs, see
/// [`MAX_RUN_W`]), clear of the row's edges, or a descender
/// starting at or right of `descenders_from`.
fn is_text_comp(c: &Comp, top: i32, descenders_from: i32) -> bool {
    let (w, h) = (c.x1 - c.x0, c.bottom - c.top);
    let merged_run =
        w <= MAX_RUN_W && h >= MIN_RUN_H && c.area as f32 <= MAX_RUN_FILL * (w * h) as f32;
    c.area >= MIN_COMP_AREA
        && h <= MAX_COMP_H
        && (w <= MAX_COMP_W || merged_run)
        && c.top - top >= MIN_COMP_TOP
        && (c.bottom - top <= ROW_H - MIN_COMP_BOTTOM_GAP
            || (c.x0 >= descenders_from
                && c.top - top <= X_HEIGHT_TOP
                && c.bottom - top <= DESCENDER_BOTTOM))
}

/// Whether a glyph sits on the text line of the row at `top` (`_on_line`):
/// bottom on the baseline, a descender whose top is above the x-height, or
/// a small dot or accent above the line. The baseline window ends at
/// [`BASELINE_LOW`], two rows lower than the port spec's.
fn on_line(c: &Comp, top: i32) -> bool {
    let (t, b) = (c.top - top, c.bottom - top);
    (19..=BASELINE_LOW).contains(&b)
        || ((24..=28).contains(&b) && t <= 15)
        || (b <= 15 && t >= 7 && c.area <= 12)
}

/// The row top that puts the line's glyphs on the baseline window of
/// [`on_line`]: `top`, moved down by as many rows (at most
/// [`MAX_LINE_OFFSET`]) as the median bottom of the big glyphs in `comps`
/// lies below [`BASELINE`]. Without that, every glyph of a row
/// found a row or two high fails the window and the row reports nothing.
fn line_top(comps: &[Comp], top: i32) -> i32 {
    let mut bottoms: Vec<i32> = comps
        .iter()
        .filter(|c| c.area >= BASELINE_GLYPH)
        .map(|c| c.bottom - top)
        .collect();
    if bottoms.is_empty() {
        return top;
    }
    let mid = bottoms.len() / 2;
    let (_, median, _) = bottoms.select_nth_unstable(mid);
    top + (*median - BASELINE).clamp(0, MAX_LINE_OFFSET)
}

/// Keeps the components of the killer part that sit on the row's text
/// baseline (`_glyphs`): the median bottom of the big components. Drops
/// pieces of the portrait's slanted frame and hair that touch the name.
fn keep_glyphs(top: i32, comps: &mut Vec<Comp>) {
    let mut bottoms: Vec<i32> = comps
        .iter()
        .filter(|c| c.area >= 20)
        .map(|c| c.bottom)
        .collect();
    let base = if bottoms.is_empty() {
        top + 22
    } else {
        bottoms.sort_unstable();
        let m = bottoms.len() / 2;
        if bottoms.len() % 2 == 1 {
            bottoms[m]
        } else {
            // NumPy's median of an even count is the mean of the two middle
            // values; the prototype truncates it to an integer.
            i32::midpoint(bottoms[m - 1], bottoms[m])
        }
    };
    comps.retain(|c| {
        ((c.bottom - base).abs() <= 1 && c.area >= 8)
            || (base + 2 <= c.bottom && c.bottom <= base + 4 && c.top <= base - 7 && c.area >= 8)
            || (c.bottom <= base - 6 && c.top >= base - 15 && c.area <= 12)
    });
}

/// Clusters `comps` (sorted by `x0`, stable) whose gaps are at most `gap`
/// (`_clusters`).
fn cluster(comps: &mut [Comp], gap: i32, out: &mut Vec<Cluster>) {
    out.clear();
    comps.sort_by_key(|c| c.x0);
    for (i, c) in comps.iter().enumerate() {
        match out.last_mut() {
            Some(last) if c.x0 - last.x1 <= gap => {
                last.x1 = last.x1.max(c.x1);
                last.end = i + 1;
            }
            _ => out.push(Cluster {
                x0: c.x0,
                x1: c.x1,
                start: i,
                end: i + 1,
            }),
        }
    }
}

/// Whether a cluster looks like text (`_is_text`): enough ink, a glyph
/// height for its big components and a minimum width.
fn is_text(c: &Cluster, comps: &[Comp]) -> bool {
    let items = &comps[c.start..c.end];
    let ink: u32 = items.iter().map(|p| p.area).sum();
    let has_big = items.iter().any(|p| p.area >= BIG_GLYPH);
    let big = items.iter().filter(|p| !has_big || p.area >= BIG_GLYPH);
    let (ya, yb) = big.fold((i32::MAX, i32::MIN), |(a, b), p| {
        (a.min(p.top), b.max(p.bottom))
    });
    ink >= 10 && (6..=18).contains(&(yb - ya)) && c.x1 - c.x0 >= 5
}

/// Whether a cluster has at least two big glyphs.
fn multi_glyph(c: &Cluster, comps: &[Comp]) -> bool {
    comps[c.start..c.end]
        .iter()
        .filter(|p| p.area >= BIG_GLYPH)
        .count()
        >= 2
}

/// Bounds of components `start..end` with the horizontal extent `x0..x1`
/// (`_box_of`).
fn bounds(x0: i32, x1: i32, items: &[Comp]) -> RectI {
    let (top, bottom) = items.iter().fold((i32::MAX, i32::MIN), |(a, b), p| {
        (a.min(p.top), b.max(p.bottom))
    });
    RectI {
        x0,
        y0: top,
        x1,
        y1: bottom,
    }
}

/// Column profiles of the inner band of the row at `top` over the columns
/// `FEED_X0..VICTIM_X1`.
fn profiles(classes: &PixelClasses, top: i32, p: &mut Profile) {
    let width = classes.width() as usize;
    let (x0, x1) = (
        (FEED_X0 - FEED_ROI.x0) as usize,
        (VICTIM_X1 - FEED_ROI.x0) as usize,
    );
    for v in [&mut p.teal, &mut p.red, &mut p.white] {
        v.clear();
        v.resize(x1 - x0, 0);
    }
    let rows = top + INNER_MARGIN - FEED_ROI.y0..top + ROW_H - INNER_MARGIN - FEED_ROI.y0;
    for y in rows {
        let row = &classes.flags()[y as usize * width + x0..y as usize * width + x1];
        for (i, &f) in row.iter().enumerate() {
            p.teal[i] += u32::from(f & Class::Teal.bit() != 0);
            p.red[i] += u32::from(f & Class::Red.bit() != 0);
            p.white[i] += u32::from(f & Class::White.bit() != 0);
        }
    }
}

/// Finds the names in `row` and reports each as `(kind, text bounds)`,
/// victims first (`analyze_row`).
///
/// The killer and victim sides are told apart by the default team colours
/// if they explain the bar next to the victim name, otherwise by colours
/// learned from the row ([`learned_sides`]). A row found from luminance
/// alone has no colours to tell the sides apart and is read as a
/// one-colour row.
pub(super) fn analyze_row(
    classes: &PixelClasses,
    row: FeedRow,
    s: &mut Scratch,
    mut emit: impl FnMut(NameKind, RectI),
) {
    let top = row.top;
    if row.evidence == BarEvidence::Luminance {
        one_colour_row(classes, top, s, &mut emit);
        return;
    }
    let mut p = std::mem::take(&mut s.profile);
    profiles(classes, top, &mut p);
    // The default colours decide wherever they explain the victim side;
    // otherwise the learned colours (a colour-blind palette, custom team
    // colours, a GPU filter), and the defaults again if those find no
    // boundary. A row without a boundary is read as a one-colour row,
    // which still reports every name.
    let explained = explains_victim(default_sides(&p).0, &p.white);
    let mut boundary = None;
    if !explained && learned_sides(classes, top, &s.tones, &mut p) {
        boundary = colour_boundary(&p.victim, &p.killer, &p.white);
    }
    if boundary.is_none() {
        let (own, other) = default_sides(&p);
        boundary = colour_boundary(own, other, &p.white);
    }
    s.profile = p;
    let Some(xb) = boundary else {
        one_colour_row(classes, top, s, &mut emit);
        return;
    };
    // Victim: right of the boundary, right-aligned to the portrait.
    text_comps(
        classes,
        top,
        xb + VICTIM_START,
        VICTIM_TEXT_X1,
        xb + VICTIM_START,
        s,
    );
    let line = line_top(&s.comps, top);
    s.comps.retain(|c| on_line(c, line));
    let mut clusters = std::mem::take(&mut s.clusters);
    cluster(&mut s.comps, GAP_VICTIM, &mut clusters);
    clusters.retain(|c| is_text(c, &s.comps));
    let aligned = clusters.last().is_some_and(|c| c.x1 >= RIGHT_ALIGN);
    if !aligned {
        s.clusters = clusters;
        return;
    }
    if clusters
        .first()
        .is_some_and(|c| c.x0 <= xb + VICTIM_START + BOUNDARY_TOUCH)
    {
        s.clusters = clusters;
        let victim_x0 = touching_victim(classes, top, xb, s, &mut emit);
        killer(classes, top, xb, victim_x0, s, &mut emit);
        return;
    }
    // Words of one name are close together; a further group in the victim
    // colour (a self-kill row with the icon between two names) is another
    // name and must have two big glyphs.
    let mut first = 0;
    let mut victim_x0 = None;
    for i in 0..clusters.len() {
        let last_of_group =
            i + 1 == clusters.len() || clusters[i + 1].x0 - clusters[i].x1 > GAP_WORDS;
        if !last_of_group {
            continue;
        }
        let group = Cluster {
            x0: clusters[first].x0,
            x1: clusters[i].x1,
            start: clusters[first].start,
            end: clusters[i].end,
        };
        if i + 1 == clusters.len() || multi_glyph(&group, &s.comps) {
            victim_x0.get_or_insert(group.x0);
            emit(
                NameKind::Victim,
                bounds(group.x0, group.x1, &s.comps[group.start..group.end]),
            );
        }
        first = i + 1;
    }
    s.clusters = clusters;
    killer(classes, top, xb, victim_x0, s, &mut emit);
}

/// The victim of a row whose victim text starts right at the colour
/// boundary `xb`: the boundary may lie inside the name. Under a filter
/// that pales the victim side's gradient (gamma 2.0), the walk from the
/// portrait meets killer-coloured columns in the name's first word
/// (measured: the first word of a two-word victim unblurred for 29
/// frames). The name is read again from up to [`NAME_REACH`] left of the
/// boundary and reported as the words chained to the right-aligned end
/// with gaps of at most [`GAP_WORDS`]; the killer side's weapon and icons
/// end the chain. Returns the name's left end.
fn touching_victim(
    classes: &PixelClasses,
    top: i32,
    xb: i32,
    s: &mut Scratch,
    emit: &mut impl FnMut(NameKind, RectI),
) -> Option<i32> {
    text_comps(
        classes,
        top,
        (xb - NAME_REACH).max(FEED_X0),
        VICTIM_TEXT_X1,
        xb + VICTIM_START,
        s,
    );
    let line = line_top(&s.comps, top);
    s.comps.retain(|c| on_line(c, line));
    let mut clusters = std::mem::take(&mut s.clusters);
    cluster(&mut s.comps, GAP_VICTIM, &mut clusters);
    clusters.retain(|c| is_text(c, &s.comps));
    let mut x0 = None;
    if let Some(last) = clusters.last().filter(|c| c.x1 >= RIGHT_ALIGN) {
        let mut first = clusters.len() - 1;
        while first > 0 && clusters[first].x0 - clusters[first - 1].x1 <= GAP_WORDS {
            first -= 1;
        }
        let chain = &clusters[first];
        x0 = Some(chain.x0);
        emit(
            NameKind::Victim,
            bounds(chain.x0, last.x1, &s.comps[chain.start..last.end]),
        );
    }
    s.clusters = clusters;
    x0
}

/// Profile column of reference column `x`.
const fn col(x: i32) -> usize {
    (x - FEED_X0) as usize
}

/// The default victim and killer colour profiles of a row: the default
/// team colour with more pixels next to the victim name is the victim's.
fn default_sides(p: &Profile) -> (&[u32], &[u32]) {
    let sum = |v: &[u32]| -> u32 { v[col(VICTIM_COLOUR_X0)..col(VICTIM_X1)].iter().sum() };
    if sum(&p.teal) >= sum(&p.red) {
        (&p.teal, &p.red)
    } else {
        (&p.red, &p.teal)
    }
}

/// Whether the victim colour profile `own` covers enough of the non-white
/// pixels next to the victim name to be the bar colour (the column test
/// of [`colour_boundary`], over the whole victim area).
fn explains_victim(own: &[u32], white: &[u32]) -> bool {
    let zone = col(VICTIM_COLOUR_X0)..col(VICTIM_X1);
    let pixels = zone.len() as u32 * (ROW_H - 2 * INNER_MARGIN) as u32;
    let own: u32 = own[zone.clone()].iter().sum();
    let white: u32 = white[zone].iter().sum();
    f64::from(own) >= COLOUR_SHARE * f64::from(pixels - white)
}

/// Learns the victim colour from the bar next to the victim name and finds
/// the killer colour: a bar colour the row finder learned in this frame
/// (`frame`) that differs from the victim's, or else the main colour of
/// the bar left of the victim area. Fills the victim and killer profiles.
/// Returns `false` if the victim or the killer colour cannot be found.
fn learned_sides(classes: &PixelClasses, top: i32, frame: &[Tone], p: &mut Profile) -> bool {
    let band = |x0: i32, x1: i32| RectI {
        x0,
        y0: top + INNER_MARGIN,
        x1,
        y1: top + ROW_H - INNER_MARGIN,
    };
    let Some(victim) = tones::learn(
        classes,
        &Sample {
            rect: band(VICTIM_COLOUR_X0, VICTIM_X1),
            exclude: None,
            min_share: VICTIM_TONE_SHARE,
            min_pixels: VICTIM_TONE_PIXELS,
            count_grey: true,
        },
    ) else {
        return false;
    };
    let other_team = |t: &Tone| t.hue_gap(victim) >= tones::MIN_TEAM_HUE_GAP;
    let killer = frame.iter().copied().find(other_team).or_else(|| {
        tones::learn(
            classes,
            &Sample {
                rect: band(KILLER_TONE_X0, VICTIM_COLOUR_X0),
                exclude: Some(victim),
                min_share: KILLER_TONE_SHARE,
                min_pixels: KILLER_TONE_PIXELS,
                count_grey: false,
            },
        )
        .filter(other_team)
    });
    let Some(killer) = killer else {
        return false;
    };
    let row = band(FEED_X0, VICTIM_X1);
    for (profile, tone) in [(&mut p.victim, victim), (&mut p.killer, killer)] {
        profile.clear();
        profile.resize(col(VICTIM_X1), 0);
        tone.count_columns(classes, row, profile);
    }
    true
}

/// The victim | killer colour boundary: walking left from the victim
/// portrait, the first place with three killer-colour columns since the
/// last victim-colour column. Columns of neither colour do not reset the
/// count (port spec 3.3, kept exactly): the victim side of some rows is a
/// gradient that the victim colour measured next to the portrait does not
/// cover everywhere.
fn colour_boundary(own: &[u32], other: &[u32], white: &[u32]) -> Option<i32> {
    let rows = f64::from(ROW_H - 2 * INNER_MARGIN);
    let mut count = 0;
    for x in (FEED_X0 + 1..VICTIM_X1).rev() {
        let i = col(x);
        let (own, other) = (f64::from(own[i]) / rows, f64::from(other[i]) / rows);
        let non_white = (1.0 - f64::from(white[i]) / rows).max(1e-3);
        if own >= COLOUR_SHARE * non_white && own > other {
            count = 0;
        } else if other >= COLOUR_SHARE * non_white && other > own {
            count += 1;
            if count >= BOUNDARY_RUN {
                return Some(x + BOUNDARY_RUN as i32);
            }
        }
    }
    None
}

/// A one-colour row (self, ability or spike kill): every multi-glyph text
/// cluster is a name, if the last one is right-aligned. Glyphs overlapping
/// a tall white icon (the round kill icon) are dropped so that it cannot
/// chain the two names together.
fn one_colour_row(
    classes: &PixelClasses,
    top: i32,
    s: &mut Scratch,
    emit: &mut impl FnMut(NameKind, RectI),
) {
    let band = RectI {
        x0: FEED_X0,
        y0: top,
        x1: VICTIM_TEXT_X1,
        y1: top + ROW_H,
    };
    find_components(classes, Class::White, band, s);
    let icons: Vec<(i32, i32)> = s
        .found
        .iter()
        .map(to_comp)
        .filter(|c| {
            let (w, h) = (c.x1 - c.x0, c.bottom - c.top);
            (ICON_MIN_H..=ICON_MAX_H).contains(&h) && w <= ICON_MAX_W
        })
        .map(|c| (c.x0 - ICON_MARGIN, c.x1 + ICON_MARGIN))
        .collect();
    text_comps(classes, top, FEED_X0, VICTIM_TEXT_X1, VICTIM_COLOUR_X0, s);
    let line = line_top(&s.comps, top);
    s.comps
        .retain(|c| on_line(c, line) && !icons.iter().any(|&(i0, i1)| c.x0 < i1 && c.x1 > i0));
    let mut clusters = std::mem::take(&mut s.clusters);
    cluster(&mut s.comps, GAP_NAME, &mut clusters);
    clusters.retain(|c| is_text(c, &s.comps) && multi_glyph(c, &s.comps));
    if clusters.last().is_some_and(|c| c.x1 >= RIGHT_ALIGN) {
        for c in &clusters {
            emit(
                NameKind::Victim,
                bounds(c.x0, c.x1, &s.comps[c.start..c.end]),
            );
        }
    }
    s.clusters = clusters;
}

/// The killer name: the text just left of the weapon, or without a weapon
/// blob (knife, small ability icon) the rightmost multi-glyph text left of
/// the icon zone.
///
/// The weapon is searched left of the colour boundary `xb`. A boundary
/// found far left of the victim name (`victim_x0`) is misplaced: under a
/// colour filter the pale end of a gradient killer bar next to the weapon
/// matches neither team colour, and the walk from the victim side stops at
/// the dark killer bar around the killer name. The weapon is then searched
/// left of the victim name instead (measured: a colour balance tint left
/// the killer name of a new row unblurred for 78 frames).
fn killer(
    classes: &PixelClasses,
    top: i32,
    xb: i32,
    victim_x0: Option<i32>,
    s: &mut Scratch,
    emit: &mut impl FnMut(NameKind, RectI),
) {
    let mut weapon = weapon_x(classes, top, xb, s);
    if weapon.is_none()
        && let Some(vx) = victim_x0
        && vx - VICTIM_START - xb >= MISPLACED_BOUNDARY
    {
        weapon = weapon_x(classes, top, vx - VICTIM_START, s);
    }
    let mut clusters = std::mem::take(&mut s.clusters);
    if let Some(xw) = weapon {
        killer_comps(classes, top, xw - WEAPON_GAP, xw, s);
        keep_glyphs(top, &mut s.comps);
        cluster(&mut s.comps, GAP_NAME, &mut clusters);
        if let Some(c) = clusters.last()
            && xw - c.x1 <= MAX_KILLER_TO_WEAPON
            && is_text(c, &s.comps)
        {
            emit(
                NameKind::Killer,
                bounds(c.x0, c.x1, &s.comps[c.start..c.end]),
            );
        }
    } else {
        killer_comps(classes, top, xb - ICON_ZONE, xb, s);
        keep_glyphs(top, &mut s.comps);
        cluster(&mut s.comps, GAP_NAME, &mut clusters);
        if let Some(c) = clusters
            .iter()
            .rev()
            .find(|c| is_text(c, &s.comps) && multi_glyph(c, &s.comps))
        {
            emit(
                NameKind::Killer,
                bounds(c.x0, c.x1, &s.comps[c.start..c.end]),
            );
        }
    }
    s.clusters = clusters;
}

/// Text components of the killer side that start left of `limit`, into
/// `s.comps`. A glyph that crosses `limit` is kept whole (up to `hard`, the
/// weapon or the colour boundary): the prototype cut it at `limit`, so the
/// end of a name that comes close to a small icon stayed readable.
fn killer_comps(classes: &PixelClasses, top: i32, limit: i32, hard: i32, s: &mut Scratch) {
    text_comps(
        classes,
        top,
        FEED_X0,
        hard.min(limit + MAX_COMP_W),
        FEED_X0,
        s,
    );
    s.comps.retain(|c| c.x0 < limit);
}

/// The left edge of the weapon icon: the rightmost solid pure-white blob
/// left of the colour boundary whose right end sits within 80 px of it
/// (the headshot and wallbang icons are in between), extended left over
/// the white pieces it continues into ([`WEAPON_JOIN`]).
fn weapon_x(classes: &PixelClasses, top: i32, xb: i32, s: &mut Scratch) -> Option<i32> {
    let band = RectI::new(FEED_X0, top + 2, xb, top + ROW_H - 2).ok()?;
    find_components(classes, Class::PureWhite, band, s);
    let mut best: Option<Comp> = None;
    for c in s.found.iter().map(to_comp) {
        let (w, h) = (c.x1 - c.x0, c.bottom - c.top);
        // The prototype starts its "rightmost" search at offset 0 with a
        // strict comparison, so a blob at the band's left edge never wins.
        let right_of_best = c.x0 > best.map_or(FEED_X0, |b| b.x0);
        if (10..=25).contains(&h)
            && (12..=140).contains(&w)
            && c.area >= 40
            && c.x0 < xb - 36
            && c.x1 >= xb - 80
            && right_of_best
        {
            best = Some(c);
        }
    }
    let mut weapon = best?;
    // A frame scaled down horizontally splits the icon (measured at
    // 1024 x 768: the stock of a rifle 11 px wide, too narrow to win, so
    // the killer name ended at the stock and was not blurred); its pieces
    // touch the blob and span at least half its height beside it.
    while let Some(piece) = s.found.iter().map(to_comp).find(|c| {
        c.x0 < weapon.x0
            && c.x1 + WEAPON_JOIN >= weapon.x0
            && c.top < weapon.bottom
            && weapon.top < c.bottom
            && 2 * (c.bottom - c.top) >= weapon.bottom - weapon.top
    }) {
        weapon.x0 = piece.x0;
    }
    Some(weapon.x0)
}

#[cfg(test)]
mod tests {
    use super::{Comp, MAX_LINE_OFFSET, is_text_comp, line_top, on_line};

    fn glyph(top: i32, bottom: i32) -> Comp {
        Comp {
            x0: 0,
            x1: 6,
            top,
            bottom,
            area: 30,
        }
    }

    #[test]
    fn a_row_found_high_is_moved_onto_the_line() {
        // Glyphs on the line of a row at 100 (baseline + 22), read with a
        // row top found two rows high.
        let comps: Vec<Comp> = (0..5).map(|_| glyph(111, 122)).collect();
        assert_eq!(line_top(&comps, 100), 100);
        let line = line_top(&comps, 98);
        assert_eq!(line, 98);
        assert!(comps.iter().all(|c| on_line(c, line)));
        // Three rows low on the window's end: moved by at most two.
        let comps: Vec<Comp> = (0..5).map(|_| glyph(116, 128)).collect();
        assert_eq!(line_top(&comps, 100), 100 + MAX_LINE_OFFSET);
        assert!(comps.iter().all(|c| on_line(c, 100 + MAX_LINE_OFFSET)));
        assert!(!on_line(&comps[0], 100));
        // No big glyph: the row top stays.
        assert_eq!(line_top(&[], 100), 100);
    }

    #[test]
    fn keeps_glyphs_merged_into_one_run() {
        // Measured at 1024 x 768 (row top 96): four letters of a killer
        // name in one component 45 px wide, 283 px of ink. A solid piece of
        // the same size is a bar or portrait part.
        let run = |area: u32| Comp {
            x0: 1635,
            x1: 1680,
            top: 107,
            bottom: 118,
            area,
        };
        assert!(is_text_comp(&run(283), 96, 1700));
        assert!(!is_text_comp(&run(460), 96, 1700));
    }

    #[test]
    fn keeps_deep_descenders_where_names_lie() {
        // Measured on real rows (row top 0): a Cyrillic "ф" from + 11 to
        // + 27 at a victim name's end, a "y" from + 16 to + 27 at a killer
        // name's end in a row found a row high.
        let comp = |x0: i32, top: i32, bottom: i32| Comp {
            x0,
            x1: x0 + 8,
            top,
            bottom,
            area: 40,
        };
        assert!(is_text_comp(&comp(1820, 11, 27), 0, 1800));
        assert!(is_text_comp(&comp(1515, 16, 27), 0, 1250));
        // Left of where the names lie (weapon icon pieces), or starting
        // below the x-height, or reaching the row's bottom edge: no text.
        assert!(!is_text_comp(&comp(1700, 11, 27), 0, 1800));
        assert!(!is_text_comp(&comp(1820, 18, 27), 0, 1800));
        assert!(!is_text_comp(&comp(1820, 11, 29), 0, 1800));
        // Glyphs on the line stay text everywhere.
        assert!(is_text_comp(&comp(1700, 14, 24), 0, 1800));
    }
}
