//! The names inside one kill-feed row (port spec 3.2 and 3.3).
//!
//! All coordinates are reference pixels; masks are read through the crop
//! offset of [`FEED_ROI`].

use blurrd_core::{NameKind, RectI};

use super::{FEED_ROI, FEED_X0, RIGHT_ALIGN, ROW_H, Scratch, VICTIM_TEXT_X1, VICTIM_X1};
use crate::classes::{Class, PixelClasses};
use crate::components::{Component, Connectivity, components};

/// Text components larger than this are portrait or frame pieces.
const MAX_COMP_H: i32 = 16;
/// See [`MAX_COMP_H`].
const MAX_COMP_W: i32 = 30;
/// Components with less ink are noise.
const MIN_COMP_AREA: u32 = 3;
/// Components starting this close to the row top touch the row edge.
const MIN_COMP_TOP: i32 = 8;
/// Components ending this close to the row bottom touch the row edge.
const MIN_COMP_BOTTOM_GAP: i32 = 7;
/// A component with at least this much ink is a "big" glyph.
const BIG_GLYPH: u32 = 12;
/// Clustering gap of killer glyphs and one-colour rows.
const GAP_NAME: i32 = 12;
/// Clustering gap of victim glyphs.
const GAP_VICTIM: i32 = 14;
/// Words of one victim name are at most this far apart.
const GAP_WORDS: i32 = 25;
/// Rows of the inner band used for the colour profiles, from each edge.
const INNER_MARGIN: i32 = 4;
/// The victim colour is measured from this column to [`VICTIM_X1`].
const VICTIM_COLOUR_X0: i32 = 1790;
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
/// Margin around such an icon in which glyphs are dropped.
const ICON_MARGIN: i32 = 3;
/// The victim text starts this far right of the colour boundary.
const VICTIM_START: i32 = 2;
/// A killer name ends at most this far left of the weapon.
const MAX_KILLER_TO_WEAPON: i32 = 48;
/// The killer text search stops this far left of the weapon.
const WEAPON_GAP: i32 = 6;
/// Without a weapon blob, the killer text search stops this far left of
/// the colour boundary (knife and ability icons sit in between).
const ICON_ZONE: i32 = 40;

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
/// class.
#[derive(Debug, Clone, Default)]
pub(super) struct Profile {
    teal: Vec<u32>,
    red: Vec<u32>,
    white: Vec<u32>,
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
fn text_comps(classes: &PixelClasses, top: i32, x0: i32, x1: i32, s: &mut Scratch) {
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
    s.comps.extend(s.found.iter().map(to_comp).filter(|c| {
        let (w, h) = (c.x1 - c.x0, c.bottom - c.top);
        c.area >= MIN_COMP_AREA
            && h <= MAX_COMP_H
            && w <= MAX_COMP_W
            && c.top - top >= MIN_COMP_TOP
            && c.bottom - top <= ROW_H - MIN_COMP_BOTTOM_GAP
    }));
}

/// Whether a glyph sits on the text line of the row at `top` (`_on_line`):
/// bottom on the baseline, a descender whose top is above the x-height, or
/// a small dot or accent above the line.
fn on_line(c: &Comp, top: i32) -> bool {
    let (t, b) = (c.top - top, c.bottom - top);
    (19..=24).contains(&b)
        || ((24..=28).contains(&b) && t <= 15)
        || (b <= 15 && t >= 7 && c.area <= 12)
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

/// Finds the names in the row at `top` and reports each as
/// `(kind, text bounds)`, victims first (`analyze_row`).
pub(super) fn analyze_row(
    classes: &PixelClasses,
    top: i32,
    s: &mut Scratch,
    mut emit: impl FnMut(NameKind, RectI),
) {
    let mut profile = std::mem::take(&mut s.profile);
    profiles(classes, top, &mut profile);
    let boundary = colour_boundary(&profile);
    s.profile = profile;
    let Some(xb) = boundary else {
        one_colour_row(classes, top, s, &mut emit);
        return;
    };
    // Victim: right of the boundary, right-aligned to the portrait.
    text_comps(classes, top, xb + VICTIM_START, VICTIM_TEXT_X1, s);
    s.comps.retain(|c| on_line(c, top));
    let mut clusters = std::mem::take(&mut s.clusters);
    cluster(&mut s.comps, GAP_VICTIM, &mut clusters);
    clusters.retain(|c| is_text(c, &s.comps));
    let aligned = clusters.last().is_some_and(|c| c.x1 >= RIGHT_ALIGN);
    if !aligned {
        s.clusters = clusters;
        return;
    }
    // Words of one name are close together; a further group in the victim
    // colour (a self-kill row with the icon between two names) is another
    // name and must have two big glyphs.
    let mut first = 0;
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
            emit(
                NameKind::Victim,
                bounds(group.x0, group.x1, &s.comps[group.start..group.end]),
            );
        }
        first = i + 1;
    }
    s.clusters = clusters;
    killer(classes, top, xb, s, &mut emit);
}

/// The victim | killer colour boundary: walking left from the victim
/// portrait, the first place with three killer-colour columns since the
/// last victim-colour column. Columns of neither colour do not reset the
/// count (port spec 3.3, kept exactly).
fn colour_boundary(p: &Profile) -> Option<i32> {
    let col = |x: i32| (x - FEED_X0) as usize;
    let sum = |v: &[u32]| -> u32 { v[col(VICTIM_COLOUR_X0)..col(VICTIM_X1)].iter().sum() };
    let (own, other) = if sum(&p.teal) >= sum(&p.red) {
        (&p.teal, &p.red)
    } else {
        (&p.red, &p.teal)
    };
    let rows = f64::from(ROW_H - 2 * INNER_MARGIN);
    let mut count = 0;
    for x in (FEED_X0 + 1..VICTIM_X1).rev() {
        let i = col(x);
        let (own, other) = (f64::from(own[i]) / rows, f64::from(other[i]) / rows);
        let non_white = (1.0 - f64::from(p.white[i]) / rows).max(1e-3);
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
        .filter(|c| c.bottom - c.top >= ICON_MIN_H && c.x1 - c.x0 <= ICON_MAX_W)
        .map(|c| (c.x0 - ICON_MARGIN, c.x1 + ICON_MARGIN))
        .collect();
    text_comps(classes, top, FEED_X0, VICTIM_TEXT_X1, s);
    s.comps
        .retain(|c| on_line(c, top) && !icons.iter().any(|&(i0, i1)| c.x0 < i1 && c.x1 > i0));
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
fn killer(
    classes: &PixelClasses,
    top: i32,
    xb: i32,
    s: &mut Scratch,
    emit: &mut impl FnMut(NameKind, RectI),
) {
    let weapon = weapon_x(classes, top, xb, s);
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
    text_comps(classes, top, FEED_X0, hard.min(limit + MAX_COMP_W), s);
    s.comps.retain(|c| c.x0 < limit);
}

/// The left edge of the weapon icon: the rightmost solid pure-white blob
/// left of the colour boundary whose right end sits within 80 px of it
/// (the headshot and wallbang icons are in between).
fn weapon_x(classes: &PixelClasses, top: i32, xb: i32, s: &mut Scratch) -> Option<i32> {
    let band = RectI::new(FEED_X0, top + 2, xb, top + ROW_H - 2).ok()?;
    find_components(classes, Class::PureWhite, band, s);
    let mut best: Option<i32> = None;
    for c in s.found.iter().map(to_comp) {
        let (w, h) = (c.x1 - c.x0, c.bottom - c.top);
        // The prototype starts its "rightmost" search at offset 0 with a
        // strict comparison, so a blob at the band's left edge never wins.
        let right_of_best = c.x0 > best.unwrap_or(FEED_X0);
        if (10..=25).contains(&h)
            && (12..=140).contains(&w)
            && c.area >= 40
            && c.x0 < xb - 36
            && c.x1 >= xb - 80
            && right_of_best
        {
            best = Some(c.x0);
        }
    }
    best
}
