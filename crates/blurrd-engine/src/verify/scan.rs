//! Finding the exposed name parts in the lines pass 0 read, and grouping
//! them into runs.

use blurrd_core::{BlurBox, FrameBoxTable, RectI, Region};

use super::{Context, Presence, union};
use crate::dictionary::{DictionaryBuilder, ZoneKind};
use crate::slots::NameSlots;

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

/// Where the names of one video can be, with the plan's boxes.
pub(crate) struct Places<'a> {
    /// The plan's boxes per frame.
    table: &'a FrameBoxTable,
    /// What confirms a HUD slot.
    presence: &'a Presence,
    /// The HUD regions' name slots.
    slots: NameSlots,
}

impl<'a> Places<'a> {
    /// The places with the HUD slots `slots`, the boxes of `table` and
    /// what `presence` confirms.
    pub(crate) fn new(slots: NameSlots, table: &'a FrameBoxTable, presence: &'a Presence) -> Self {
        Self {
            table,
            presence,
            slots,
        }
    }
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

/// Scans the frames of `ctx.log` for which `inside` holds for exposed
/// name parts, with the boxes and slots of `places`.
pub(crate) fn scan(
    ctx: &Context<'_>,
    places: &Places<'_>,
    inside: &dyn Fn(u32) -> bool,
) -> Vec<Run> {
    let aspect = ctx.geometry.sx / ctx.geometry.sy;
    let mut runs = Runs::default();
    for (frame, lines) in ctx.log.iter() {
        if !inside(frame) {
            continue;
        }
        let boxes = places.table.boxes(frame);
        for line in lines.protected() {
            for part in &line.parts {
                let region = match line.zone {
                    // A screen surface's zone is protected while the surface
                    // is there (it has boxes).
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
                if ctx.regions.contains(region) && exposed(part.rect, boxes, aspect) {
                    runs.add(
                        region,
                        &part.folded,
                        DictionaryBuilder::CADENCE,
                        Exposure {
                            frame,
                            part: part.rect,
                            line: line.rect,
                        },
                    );
                }
            }
        }
    }
    runs.finish()
}

#[cfg(test)]
mod tests {
    use blurrd_core::{BlurBox, RectI, Region};

    use super::{Exposure, Run, Runs, exposed};

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
}
