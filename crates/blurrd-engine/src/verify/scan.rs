//! Finding the exposed name parts and the out-of-scope names in the
//! lines pass 0 read, and grouping them into runs.

use blurrd_core::{
    BlurBox, FrameBoxTable, FrameRange, Leak, LeakKind, LeakStatus, LeakZone, RectI, Region,
};
use blurrd_detect::screens::{Anchor, ScreenMap};
use blurrd_ocr::{LeakMatcher, NameMatch};

use super::{Context, FrameLines, Presence, union};
use crate::dictionary::{ZoneKind, screen_zones};
use crate::slots::NameSlots;

/// The scoreboard's name column in reference pixels (centre-anchored):
/// the name cells of the ten player rows, with the board's vertical slide
/// (the geometry of `blurrd_detect::screens::scoreboard`: name cells at
/// columns 636 to 838, rows from 338 and from 569 every 34 rows, 16 rows
/// of slide).
const BOARD_NAMES: RectI = RectI {
    x0: 636,
    y0: 322,
    x1: 838,
    y1: 753,
};

/// Rows of dictionary names on the scoreboard's row grid that make a
/// scoreboard whose gate missed it (a name tag and a buy-phase card above
/// heads can share the name column, but not three rows of its grid).
const BOARD_MIN_ROWS: usize = 3;

/// Centre rows of the scoreboard's two blocks of five player rows
/// (reference rows 338 and 569, 34 apart, name text 17 below a row's
/// top).
const BOARD_ROW_CENTRES: [f32; 2] = [355.0, 586.0];
/// The scoreboard's row pitch, in reference rows.
const BOARD_PITCH: f32 = 34.0;
/// Rows the board slides while it opens, in reference rows.
const BOARD_SLIDE: i32 = 16;
/// Reference rows a name's centre may lie off its row's centre.
const BOARD_ROW_TOLERANCE: f32 = 5.0;

/// An exposed name part on one frame.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Exposure {
    /// The frame.
    pub(crate) frame: u32,
    /// The name part's box.
    pub(crate) part: RectI,
    /// Its line's box.
    pub(crate) line: RectI,
}

/// Exposures of one name over consecutive read frames.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Run {
    /// Its region.
    pub(crate) region: Region,
    /// The folded name (in memory only), to follow it.
    folded: String,
    /// Frames between two reads of its zone: a longer gap ends the run.
    pub(crate) spacing: u32,
    /// The exposures, in frame order (never empty).
    pub(crate) exposures: Vec<Exposure>,
}

impl Run {
    /// Its first frame.
    pub(crate) fn first(&self) -> u32 {
        self.exposures.first().map_or(0, |e| e.frame)
    }

    /// Its last frame.
    pub(crate) fn last(&self) -> u32 {
        self.exposures.last().map_or(0, |e| e.frame)
    }

    /// The union of its name parts.
    pub(crate) fn union(&self) -> RectI {
        self.fold_rects(|e| e.part)
    }

    /// The union of its lines.
    pub(crate) fn line_union(&self) -> RectI {
        self.fold_rects(|e| e.line)
    }

    /// The tallest line's height.
    pub(crate) fn height(&self) -> u32 {
        self.exposures
            .iter()
            .map(|e| e.line.height())
            .max()
            .unwrap_or(1)
    }

    fn fold_rects(&self, rect: impl Fn(&Exposure) -> RectI) -> RectI {
        let mut rects = self.exposures.iter().map(rect);
        let first = rects.next().unwrap_or(RectI {
            x0: 0,
            y0: 0,
            x1: 1,
            y1: 1,
        });
        rects.fold(first, union)
    }

    /// Whether `exposure` of `folded` on a later frame continues the run.
    fn continues(&self, folded: &str, exposure: &Exposure) -> bool {
        let Some(last) = self.exposures.last() else {
            return false;
        };
        exposure.frame > last.frame
            && exposure.frame - last.frame <= self.spacing
            && (folded == self.folded || last.part.iou(&exposure.part) >= 0.3)
    }
}

/// Runs being built, per region.
#[derive(Debug, Default)]
struct Runs {
    open: Vec<Run>,
    done: Vec<Run>,
}

impl Runs {
    /// Adds an exposure (frames in increasing order).
    fn add(&mut self, region: Region, folded: &str, spacing: u32, exposure: Exposure) {
        // Runs that can no longer continue are done.
        let frame = exposure.frame;
        let (stale, open): (Vec<Run>, Vec<Run>) = std::mem::take(&mut self.open)
            .into_iter()
            .partition(|r| frame > r.last() + r.spacing);
        self.done.extend(stale);
        self.open = open;
        match self
            .open
            .iter_mut()
            .find(|r| r.region == region && r.continues(folded, &exposure))
        {
            Some(run) => run.exposures.push(exposure),
            None => self.open.push(Run {
                region,
                folded: folded.to_owned(),
                spacing,
                exposures: vec![exposure],
            }),
        }
    }

    /// Every run, in order of its first frame.
    fn finish(mut self) -> Vec<Run> {
        self.done.append(&mut self.open);
        self.done.sort_by_key(Run::first);
        self.done
    }
}

/// What the scan found.
#[derive(Debug, Default)]
pub(crate) struct Scan {
    /// Runs of exposed name parts, in order of their first frame.
    pub(crate) runs: Vec<Run>,
    /// Out-of-scope names (status `Info`), one entry per run of a name.
    pub(crate) out_of_scope: Vec<Leak>,
}

/// Which name parts a scan lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Select {
    /// Parts the boxes do not cover (the verification).
    Exposed,
    /// Every part (the audit of an output).
    All,
}

/// Where the names of one video can be, with the plan's boxes.
pub(crate) struct Places<'a> {
    /// The plan's boxes per frame.
    table: &'a FrameBoxTable,
    /// What confirms a HUD slot.
    presence: &'a Presence,
    /// The HUD regions' name slots.
    slots: NameSlots,
    /// The screen surfaces' name zones, source pixels.
    screens: Vec<(Region, RectI)>,
    /// The scoreboard's name column, source pixels, when the scoreboard
    /// is analysed.
    board: Option<RectI>,
    /// Source row of reference row 0, and source rows per reference row.
    rows: (f32, f32),
}

impl<'a> Places<'a> {
    /// The places of `ctx`'s video with the HUD slots `slots`, the boxes
    /// of `table` and what `presence` confirms.
    pub(crate) fn new(
        ctx: &Context<'_>,
        slots: NameSlots,
        table: &'a FrameBoxTable,
        presence: &'a Presence,
    ) -> Self {
        let (w, h) = ctx.size;
        Self {
            table,
            presence,
            slots,
            screens: screen_zones(ctx.geometry, w, h, ctx.regions)
                .into_iter()
                .filter_map(|z| match z.kind {
                    ZoneKind::Screen(region) => Some((region, z.rect)),
                    _ => None,
                })
                .collect(),
            board: ctx
                .regions
                .contains(Region::Scoreboard)
                .then(|| ScreenMap::from(ctx.geometry).to_source(BOARD_NAMES, Anchor::Center)),
            rows: (ctx.geometry.oy, ctx.geometry.sy),
        }
    }
}

/// Whether the centre of `r` lies in `zone`.
fn centre_in(r: RectI, zone: RectI) -> bool {
    let (cx, cy) = (r.x0 + r.x1, r.y0 + r.y1);
    2 * zone.x0 <= cx && cx < 2 * zone.x1 && 2 * zone.y0 <= cy && cy < 2 * zone.y1
}

/// The fraction of `r` inside `zone`.
fn share_inside(r: RectI, zone: RectI) -> f32 {
    r.intersection(&zone)
        .map_or(0.0, |i| i.area() as f32 / r.area().max(1) as f32)
}

/// Whether name part `part` lies on the row of a box of screen surface
/// `region`: the surface boxes every name slot that shows ink (Safe
/// geometry), so text in a row next to such a box, but not covered by
/// it, is a name the box does not reach. Labels and numbers in other rows
/// and columns are no names.
fn on_box_row(part: RectI, region: Region, boxes: &[BlurBox]) -> bool {
    let (cx, cy) = (part.x0 + part.x1, part.y0 + part.y1);
    let reach = part.height() as i32;
    // The middle of the row: a label just above or below a box is not in
    // its row.
    let inset = reach / 4;
    boxes.iter().filter(|b| b.region == region).any(|b| {
        2 * (b.rect.y0 + inset) <= cy
            && cy < 2 * (b.rect.y1 - inset)
            && 2 * (b.rect.x0 - reach) <= cx
            && cx < 2 * (b.rect.x1 + reach)
    })
}

/// Whether name part `part` is exposed by `boxes`: some run of its
/// columns at least half a line high (one glyph; glyphs are `aspect`
/// times wider in source pixels in a stretched HUD) is not covered over
/// the middle of its height.
pub(crate) fn exposed(part: RectI, boxes: &[BlurBox], aspect: f32) -> bool {
    let h = part.height() as f32;
    let band = (
        part.y0 + (0.2 * h).round() as i32,
        part.y1 - (0.2 * h).round() as i32,
    );
    let needed = (0.5 * h * aspect).ceil().max(1.0) as i32;
    let mut run = 0;
    for x in part.x0..part.x1 {
        let covered = boxes
            .iter()
            .any(|b| b.rect.x0 <= x && x < b.rect.x1 && b.rect.y0 <= band.0 && b.rect.y1 >= band.1);
        run = if covered { 0 } else { run + 1 };
        if run >= needed {
            return true;
        }
    }
    false
}

/// Scans the frames of `ctx.log` for which `inside` holds for the name
/// parts `select` picks, with the boxes and slots of `places`.
pub(crate) fn scan(
    ctx: &Context<'_>,
    places: &Places<'_>,
    matcher: &LeakMatcher<'_>,
    select: Select,
    inside: &dyn Fn(u32) -> bool,
) -> Scan {
    let aspect = ctx.geometry.sx / ctx.geometry.sy;
    let mut runs = Runs::default();
    let mut names = Runs::default();
    let full_spacing = ctx.log.full_every() * 2;
    for (frame, lines) in ctx.log.iter() {
        if !inside(frame) {
            continue;
        }
        let boxes = places.table.boxes(frame);
        for line in lines.protected() {
            for part in &line.parts {
                let region = match line.zone {
                    // A screen surface's zone is protected while the surface
                    // is there (it has boxes); otherwise the full-frame
                    // sample decides.
                    ZoneKind::Screen(region) if on_box_row(part.rect, region, boxes) => region,
                    ZoneKind::Screen(_) => continue,
                    // A HUD slot holds names while its region shows its
                    // structure and has boxes nearby (not under a buy menu
                    // or a map).
                    _ => match places.slots.region_of(part.rect) {
                        Some(region)
                            if super::holds_name(
                                region,
                                line,
                                frame,
                                lines.structure,
                                places.presence,
                            ) && super::plausible_name(&part.folded) =>
                        {
                            region
                        }
                        _ => continue,
                    },
                };
                let picked = match select {
                    Select::Exposed => exposed(part.rect, boxes, aspect),
                    Select::All => true,
                };
                if ctx.regions.contains(region) && picked {
                    runs.add(
                        region,
                        &part.folded,
                        ctx.log.hud_spacing(),
                        Exposure {
                            frame,
                            part: part.rect,
                            line: line.rect,
                        },
                    );
                }
            }
        }
        let full = FullFrame {
            frame,
            boxes,
            places,
            matcher,
            spacing: full_spacing,
        };
        full.scan(lines, &mut runs, &mut names);
    }
    Scan {
        runs: runs.finish(),
        out_of_scope: names
            .finish()
            .into_iter()
            .map(|run| Leak {
                frame_range: FrameRange {
                    f0: run.first(),
                    f1: run.last(),
                },
                region: None,
                zone: LeakZone::FullFrame,
                rect: run.union(),
                conf: 0.0,
                kind: LeakKind::OutOfScope,
                status: LeakStatus::Info,
                dedup: None,
            })
            .collect(),
    }
}

/// The full-frame sample of one frame.
struct FullFrame<'s> {
    frame: u32,
    boxes: &'s [BlurBox],
    places: &'s Places<'s>,
    matcher: &'s LeakMatcher<'s>,
    /// Frames between samples a run may skip.
    spacing: u32,
}

impl FullFrame<'_> {
    /// Dictionary names outside every protected zone are out of scope.
    /// Dictionary names on three or more rows of the scoreboard's name
    /// column and row grid while the scoreboard has no boxes (its gate
    /// missed it) are exposed names of the scoreboard.
    fn scan(&self, lines: &FrameLines, runs: &mut Runs, names: &mut Runs) {
        let has_boxes = |region: Region| self.boxes.iter().any(|b| b.region == region);
        let mut column: Vec<(&str, Exposure)> = Vec::new();
        for line in lines.full_frame() {
            // Read in the zone of a surface that is there.
            if self
                .places
                .screens
                .iter()
                .any(|&(region, zone)| has_boxes(region) && share_inside(line.rect, zone) >= 0.5)
            {
                continue;
            }
            for part in &line.parts {
                // Read in its HUD zone.
                if self
                    .places
                    .slots
                    .region_of(part.rect)
                    .is_some_and(|r| lines.structure.contains(r))
                {
                    continue;
                }
                if self.matcher.classify(&part.folded, part.conf) != Some(NameMatch::Dictionary) {
                    continue;
                }
                let exposure = Exposure {
                    frame: self.frame,
                    part: part.rect,
                    line: line.rect,
                };
                if !has_boxes(Region::Scoreboard)
                    && self.places.board.is_some_and(|z| centre_in(part.rect, z))
                {
                    column.push((part.folded.as_str(), exposure));
                } else {
                    names.add(Region::Manual, &part.folded, self.spacing, exposure);
                }
            }
        }
        let board = board_rows(&column, self.places.rows) >= BOARD_MIN_ROWS;
        for (folded, exposure) in column {
            if board {
                runs.add(Region::Scoreboard, folded, self.spacing, exposure);
            } else {
                names.add(Region::Manual, folded, self.spacing, exposure);
            }
        }
    }
}

/// The most rows of the scoreboard's row grid, at one slide of the board,
/// that hold a name part of `parts`; `rows` maps reference rows to source
/// rows (origin and scale).
fn board_rows(parts: &[(&str, Exposure)], rows: (f32, f32)) -> usize {
    let (oy, sy) = rows;
    let centres: Vec<f32> = parts
        .iter()
        .map(|(_, e)| ((e.part.y0 + e.part.y1) as f32 * 0.5 - oy) / sy.max(f32::EPSILON))
        .collect();
    (-BOARD_SLIDE..=BOARD_SLIDE)
        .map(|slide| {
            let mut hit = [false; 10];
            for &c in &centres {
                for (block, &first) in BOARD_ROW_CENTRES.iter().enumerate() {
                    let k = ((c - first - slide as f32) / BOARD_PITCH).round();
                    let on_row = (0.0..5.0).contains(&k)
                        && (c - first - slide as f32 - k * BOARD_PITCH).abs()
                            <= BOARD_ROW_TOLERANCE;
                    if on_row {
                        hit[block * 5 + k as usize] = true;
                    }
                }
            }
            hit.iter().filter(|&&h| h).count()
        })
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use blurrd_core::{BlurBox, RectI, Region};

    use super::{Exposure, Run, Runs, board_rows, exposed};

    fn rect(x0: i32, y0: i32, x1: i32, y1: i32) -> RectI {
        RectI::new(x0, y0, x1, y1).unwrap()
    }

    fn blur(r: RectI) -> BlurBox {
        BlurBox {
            rect: r,
            region: Region::KillFeed,
            seed: 1,
            grid_origin: (r.x0, r.y0),
            block_scale: 1.0,
        }
    }

    #[test]
    fn a_part_is_exposed_by_one_uncovered_glyph() {
        let part = rect(100, 100, 200, 120);
        assert!(exposed(part, &[], 1.0));
        assert!(!exposed(part, &[blur(rect(98, 104, 202, 116))], 1.0));
        // Nine columns open at the end: less than half the line height.
        assert!(!exposed(part, &[blur(rect(98, 100, 191, 120))], 1.0));
        // Eleven open columns are one glyph.
        assert!(exposed(part, &[blur(rect(98, 100, 189, 120))], 1.0));
        // A box that does not reach over the middle of the line does not
        // cover it.
        assert!(exposed(part, &[blur(rect(98, 110, 202, 120))], 1.0));
        // Glyphs of a squeezed HUD are narrower.
        assert!(!exposed(part, &[blur(rect(98, 100, 192, 120))], 1.0));
        assert!(exposed(part, &[blur(rect(98, 100, 192, 120))], 0.75));
    }

    #[test]
    fn runs_follow_a_name_and_end_after_a_gap() {
        let mut runs = Runs::default();
        let at = |frame, x| Exposure {
            frame,
            part: rect(x, 100, x + 50, 120),
            line: rect(x - 10, 100, x + 60, 120),
        };
        runs.add(Region::KillFeed, "nova", 6, at(0, 100));
        runs.add(Region::KillFeed, "nova", 6, at(6, 100));
        // The row moved up a slot: the name keeps the run.
        runs.add(Region::KillFeed, "nova", 6, at(12, 140));
        runs.add(Region::KillFeed, "umbra", 6, at(12, 400));
        // Too long after: a new run.
        runs.add(Region::KillFeed, "nova", 6, at(40, 140));
        let runs: Vec<Run> = runs.finish();
        let frames: Vec<Vec<u32>> = runs
            .iter()
            .map(|r| r.exposures.iter().map(|e| e.frame).collect())
            .collect();
        assert_eq!(frames, [vec![0, 6, 12], vec![12], vec![40]]);
        assert_eq!(runs[0].union(), rect(100, 100, 190, 120));
    }

    #[test]
    fn every_frame_fixes_reach_as_far_as_hud_activity_ones() {
        use blurrd_core::RegionSet;

        use crate::verify::{HudRead, LineLog, fix_range};

        let at = |frame| Exposure {
            frame,
            part: rect(100, 100, 150, 120),
            line: rect(90, 100, 160, 120),
        };
        let read = HudRead {
            feed: true,
            left: true,
        };
        let mut every = LineLog::new(30);
        every.set_hud_spacing(60);
        every.set_every_frame(true);
        let mut hud = LineLog::new(30);
        hud.set_hud_spacing(60);
        for frame in 0..600 {
            every.record(frame, read, RegionSet::empty(), Vec::new());
            if frame % 60 == 0 || frame == 230 {
                hud.record(frame, read, RegionSet::empty(), Vec::new());
            }
        }
        // The name is on frames 200..=300; HUD activity read it on 230, 240
        // and 300 only, Every frame on all of them.
        let mut runs = Runs::default();
        for frame in 200..=300 {
            runs.add(Region::KillFeed, "nova", 60, at(frame));
        }
        let every_run = runs.finish().remove(0);
        let mut runs = Runs::default();
        for frame in [230, 240, 300] {
            runs.add(Region::KillFeed, "nova", 60, at(frame));
        }
        let hud_run = runs.finish().remove(0);
        let (e0, e1) = fix_range(&every, &every_run, 600);
        let (h0, h1) = fix_range(&hud, &hud_run, 600);
        assert_eq!((h0, h1), (181, 359));
        assert!(e0 <= h0 && e1 >= h1, "{e0}..{e1} vs {h0}..{h1}");
        assert_eq!((e0, e1), (141, 359));
    }

    #[test]
    fn counts_names_on_the_scoreboard_grid() {
        let at = |y| {
            (
                "x",
                Exposure {
                    frame: 0,
                    part: rect(640, y - 8, 700, y + 8),
                    line: rect(640, y - 8, 700, y + 8),
                },
            )
        };
        // Three rows of the first block, slid 6 rows down, and one of the
        // second block.
        let board = [at(361), at(395), at(463), at(592)];
        assert_eq!(board_rows(&board, (0.0, 1.0)), 4);
        // A card and a name tag above heads: on no common grid.
        let decoys = [at(342), at(726)];
        assert!(board_rows(&decoys, (0.0, 1.0)) < 3);
        assert_eq!(board_rows(&[], (0.0, 1.0)), 0);
    }
}
