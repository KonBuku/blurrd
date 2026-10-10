//! The kill-feed boxes of one frame from the followed names (port spec
//! 7.1 steps 6 and 7, plan 7.3).
//!
//! Every followed name gives its box at its tracked position. Composing
//! them adds what a single position misses and removes what a wrong one
//! adds:
//!
//! - **Pre-roll:** a new row wipes in behind a cover bar; its first one or
//!   two frames show only the start of the killer name left of the bar,
//!   too little for the template. Up to [`PRE_ROLL`] frames before a
//!   name's first frame get its box widened by [`PRE_X`] to the left (up
//!   to the row's first name), only at a row's birth, only where the row
//!   is already on screen and not where the detector reads names in its
//!   place (a row that moved there in a stack jump).
//! - **Escalation** (Tight mode, plan 7.3): frames whose position is not a
//!   confident match (weak hold, gap fill, pre-roll) get the name's slot
//!   box, the row from the name's portrait side to the other name.
//! - **Rigid rows, checked:** the other names of a reference row move with
//!   the followed name. When that name's template latched onto an
//!   identical name in another row, the companions would land in a row
//!   that does not hold them; a companion whose own template does not
//!   match there, in a row where the detector read other names, is left
//!   out, and after [`LATCHED`] such frames in a row the part keeps no
//!   companions at all (the row may later fade without detections).
//! - **Followed names win:** a companion's extent was read once, in its
//!   part's reference frame, and a misreading there (a cover bar edge
//!   while the row wipes in) would last the part's whole life; an
//!   uncertain position (weak hold, gap, pre-roll, post-roll) is a guess.
//!   Where another part follows the same name itself (not as a pre-roll
//!   guess), the companion box takes that part's columns (and keeps its
//!   own rows), and so does an uncertain box where that part has a
//!   confident match.
//! - **Rising rows:** a name's box also covers its rows of the frames
//!   before and after when they are a few pixels apart (a fading row
//!   slides up; see [`rise_union`]).
//! - **Gaps pinned:** a gap holds the union of the positions around it;
//!   where its frame shows the name as a clear peak of the template
//!   score, the gap is narrowed to that peak (see [`pin_gap`]).
//! - **Faded rows give way:** held and post-roll boxes (after a name's last
//!   match) are left out where the detector reads another name in their
//!   place: a row of the same colour moved up into it, and its bar kept
//!   the faded row's structure test alive.
//! - **Portrait clearance:** boxes end [`PORTRAIT_CLEARANCE`] pixels left
//!   of the victim portrait, so the soft edge of the default blur padding
//!   stays off it, but never closer to the text than the glyph halo of
//!   the goldens ([`TEXT_HALO`]); and no pre-roll or slot widening reaches
//!   left of the row's first name, where the killer portrait sits.
//! - **Merge:** boxes of the same name from several parts (IoU ≥
//!   [`MERGE_IOU`]) become their union.

use blurrd_core::{Confidence, NameKind, RectI};

use crate::detection::Detection;
use crate::killfeed::VICTIM_X1;
use crate::track::ncc::{Template, score_at};

use super::follow::{Frame, How, Pos, SAME_EXTENT};
use super::ring::Ring;
use super::{FeedBox, PRE_ROLL, Part, PartBox, SAME_NAME_IOU, STRIP};

/// The pre-roll box reaches this far left of the name's box (`PRE_X`).
const PRE_X: i32 = 80;
/// The row test reaches this far right of the name's box.
const PRE_AREA_RIGHT: i32 = 60;
/// The row is compared with the frame this many frames before the
/// earliest pre-roll frame, before the row was born.
const PRE_BIRTH: u32 = 2;
/// A pixel changed if one of its channels moved by more than this ...
const PRE_CHANGE: i32 = 40;
/// ... and an area changed if at least this share of its pixels did.
const PRE_CHANGED: f32 = 0.5;
/// A name alone in its row (a self-kill shows only the victim) has its
/// slot reach this far towards the kill icon.
const SLOT_REACH: i32 = 80;
/// Boxes end this far left of the victim portrait: the horizontal padding
/// of the default blur parameters (`BlurParams::default().pad.0`), where
/// the soft edge lies.
const PORTRAIT_CLEARANCE: i32 = 4;
/// ... but at least this far right of the name's text (the goldens'
/// glyph extents are dilated by 2 reference pixels).
const TEXT_HALO: i32 = 2;
/// Boxes overlapping this much are the same name and merged (port spec
/// 7.1 step 7).
const MERGE_IOU: f32 = 0.5;
/// A companion is checked only where the followed name itself matches at
/// least this well (`TH_STAY`): in a flash or a fade every template
/// scores low and nothing is left out.
const CLEAR_MATCH: f32 = 0.75;
/// A companion whose template scores below this (`TH`) is not at its
/// place ...
const COMPANION_MIN: f32 = 0.55;
/// ... if the detector read names in its row (row tops within this many
/// pixels) and none with its extent.
const ROW_TOLERANCE: i32 = 3;
/// A part whose companions are away from their place in this many clear
/// frames in a row follows an identical name in another row: it keeps no
/// companions from then on.
const LATCHED: u32 = 3;

/// A gap is narrowed to a peak of at least this score (`TH_WEAK`, the
/// score of a fading name) ...
const PIN_MIN: f32 = 0.3;
/// ... that beats every position farther away than [`PIN_RADIUS`] by
/// this much.
const PIN_MARGIN: f32 = 0.15;
/// A narrowed gap keeps this many pixels around the peak.
const PIN_RADIUS: i32 = 2;

/// Neighbouring rows of a name this close are united (a fading row rises
/// at most 9 px, measured; a stack jump moves 39 px).
const RISE_UNION: i32 = 9;

/// Confident frames this close to an uncertain one are escalated with it
/// (the jitter gate allows no box change shorter than 3 frames).
const ESCALATE_BRIDGE: u32 = 2;

/// A rectangle relative to a template corner: `x0, y0, x1, y1`.
pub(super) type Offset = (i32, i32, i32, i32);

/// The pre-roll of a part: frames `first − frames .. first` get the box of
/// frame `first` widened to the left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PreRoll {
    first: u32,
    frames: u32,
}

impl PreRoll {
    /// The position the pre-roll gives frame `f`, if any.
    fn position(self, part: &Part, f: u32) -> Option<Pos> {
        (f < self.first && self.first - f <= self.frames)
            .then(|| part.positions.get(&self.first).copied())
            .flatten()
    }
}

/// The pre-roll of `part`, decided once its backward pass is done: only at
/// a row's birth, over the frames right before the first one where the
/// row (its bar or the cover bar that wipes it in) is already on screen,
/// down to frame `lo` (the oldest frame not handed out yet).
///
/// Both tests compare the area of the widened box with a frame from
/// before the birth instead of looking for bar colours: the row is born
/// if the area looks different in the name's first frame, and a frame
/// before it shows the row if its area differs from the birth frame as
/// well. Cover bars, palettes and colour filters change nothing. A
/// camera pan can pass both tests; that only adds a few frames of blur.
pub(super) fn pre_roll(part: &Part, ring: &Ring, lo: u32) -> Option<PreRoll> {
    let (&first, pos) = part.positions.first_key_value()?;
    let own = part.boxes.first()?;
    let b = local(place(*pos, own.offset));
    let area = RectI {
        x0: (b.x0 - PRE_X).max(0),
        y0: b.y0,
        x1: (b.x1 + PRE_AREA_RIGHT).min(STRIP.width() as i32),
        y1: b.y1,
    };
    let before = ring.frame(first.checked_sub(PRE_ROLL + PRE_BIRTH)?)?;
    let changed = |f: u32| ring.frame(f).is_some_and(|fr| changed(&before, &fr, area));
    if !changed(first) {
        return None;
    }
    // A row that moved into the place (a stack jump) changes the area as
    // well, but the detector reads its names there: that is not a birth.
    let global = RectI {
        x0: area.x0 + STRIP.x0,
        y0: area.y0 + STRIP.y0,
        x1: area.x1 + STRIP.x0,
        y1: area.y1 + STRIP.y0,
    };
    let occupied = |f: u32| {
        ring.get(f).is_some_and(|s| {
            s.detections
                .iter()
                .any(|d| d.text.intersection(&global).is_some())
        })
    };
    let frames = (1..=PRE_ROLL)
        .take_while(|&k| first >= lo + k && changed(first - k) && !occupied(first - k))
        .count() as u32;
    (frames > 0).then_some(PreRoll { first, frames })
}

/// Whether at least [`PRE_CHANGED`] of the pixels of `area` (strip
/// coordinates) changed colour between frames `a` and `b`, on the
/// half-resolution colour planes.
fn changed(a: &Frame<'_>, b: &Frame<'_>, area: RectI) -> bool {
    let (mut moved, mut total) = (0u32, 0u32);
    for hy in area.y0.div_euclid(2)..area.y1.div_euclid(2) {
        for hx in area.x0.div_euclid(2)..area.x1.div_euclid(2) {
            if let (Some(p), Some(q)) = (a.colour(hx, hy), b.colour(hx, hy)) {
                total += 1;
                let delta = p
                    .iter()
                    .zip(q)
                    .map(|(&u, v)| (i32::from(u) - i32::from(v)).abs())
                    .max()
                    .unwrap_or(0);
                moved += u32::from(delta > PRE_CHANGE);
            }
        }
    }
    total > 0 && moved as f32 >= PRE_CHANGED * total as f32
}

/// One box before merging.
#[derive(Debug, Clone, Copy)]
struct Candidate {
    /// The final box.
    rect: RectI,
    /// The name's own box at its position(s), to compare names.
    name: RectI,
    kind: NameKind,
    confidence: Confidence,
    part: u32,
    offset: (i16, i16),
    /// Index of the part in the tracker.
    owner: usize,
    /// The part's followed name (not a companion).
    own: bool,
}

/// Composes the boxes of frame `f` from `parts` into `out`; `frame` is the
/// frame's strip and `detections` its kill-feed detections.
pub(super) fn compose(
    f: u32,
    parts: &mut [Part],
    frame: Option<&Frame<'_>>,
    detections: &[Detection],
    out: &mut Vec<FeedBox>,
) {
    let mut candidates = Vec::new();
    for (owner, part) in parts.iter_mut().enumerate() {
        part_candidates(f, owner, part, frame, detections, &mut candidates);
    }
    // A companion's extent was read once, in its part's reference frame,
    // and an uncertain position is a guess: a part that follows the name
    // itself has its extent right, and a confident match its position.
    // Such a box keeps its rows (the vertical evidence of its own part)
    // but takes the columns of the box that follows the name. A pre-roll
    // box is no such evidence: it is a guess where the row will be, not
    // a reading of the name. The right end never shrinks: a misreading
    // that reaches too far left is the edge of the killer portrait or of
    // a cover bar, while a short reading lost its last glyph.
    let kept: Vec<Candidate> = candidates
        .iter()
        .map(|&c| {
            let confident = c.own && c.confidence == Confidence::Tracked;
            let follower = candidates.iter().find(|d| {
                d.own
                    && d.owner != c.owner
                    && if c.own {
                        d.confidence == Confidence::Tracked
                    } else {
                        d.confidence != Confidence::PreRoll
                    }
                    && d.kind == c.kind
                    && d.name.iou(&c.name) >= SAME_NAME_IOU
            });
            match follower {
                Some(d) if !confident => Candidate {
                    rect: RectI {
                        x0: d.rect.x0,
                        x1: d.rect.x1.max(c.rect.x1),
                        ..c.rect
                    },
                    ..c
                },
                _ => c,
            }
        })
        .collect();
    let mut merged: Vec<Candidate> = Vec::with_capacity(kept.len());
    for c in kept {
        if let Some(m) = merged.iter_mut().find(|m| m.rect.iou(&c.rect) >= MERGE_IOU) {
            m.rect = union(m.rect, c.rect);
            if rank(c.confidence) > rank(m.confidence) {
                // The weakest evidence decides how the box is blurred; the
                // grid stays anchored to the first name.
                m.confidence = c.confidence;
            }
        } else {
            merged.push(c);
        }
    }
    out.extend(merged.into_iter().map(|c| FeedBox {
        rect: c.rect,
        kind: c.kind,
        confidence: c.confidence,
        part: c.part,
        offset: c.offset,
    }));
}

/// The candidates of one part (index `owner`) in frame `f`.
fn part_candidates(
    f: u32,
    owner: usize,
    part: &mut Part,
    frame: Option<&Frame<'_>>,
    detections: &[Detection],
    out: &mut Vec<Candidate>,
) {
    let (pos, confidence) = match part.position(f) {
        Some(found) => found,
        None => match part.pre_roll.and_then(|p| p.position(part, f)) {
            Some(pos) => (pos, Confidence::PreRoll),
            None => return,
        },
    };
    let pos = match (confidence, frame) {
        (Confidence::GapFill, Some(fr)) => pin_gap(&part.template, pos, fr),
        _ => pos,
    };
    let pos = if confidence == Confidence::PreRoll {
        pos
    } else {
        rise_union(part, f, pos)
    };
    let seen = companions_seen(part, pos, confidence, frame, detections);
    if seen.contains(&Seen::Away) {
        part.misplaced += 1;
    } else if seen.contains(&Seen::Here) {
        part.misplaced = 0;
    }
    let latched = part.misplaced >= LATCHED;
    let offset = (
        (pos.x0 - part.origin.0) as i16,
        (pos.y0 - part.origin.1) as i16,
    );
    // Escalated frames come in segments: a confident match within
    // [`ESCALATE_BRIDGE`] frames of an uncertain frame is escalated too,
    // so the slot box does not flicker on and off.
    let escalate = uncertain(confidence)
        || confidence == Confidence::Tracked
            && (1..=ESCALATE_BRIDGE).any(|k| {
                [f.checked_sub(k), f.checked_add(k)]
                    .into_iter()
                    .flatten()
                    .any(|g| part.position(g).is_some_and(|(_, c)| uncertain(c)))
            });
    let after_last_match = matches!(confidence, Confidence::PostRoll)
        || part.positions.get(&f).is_some_and(|p| p.how == How::Hold);
    for (i, b) in part.boxes.iter().enumerate() {
        if i > 0 && (latched || seen[i] == Seen::Away) {
            continue;
        }
        if after_last_match && taken(place(pos, b.text), detections) {
            // Another row moved into the place of the faded one: its bar
            // is not this name's.
            continue;
        }
        let name = place(pos, b.offset);
        let mut rect = name;
        if confidence == Confidence::PreRoll {
            rect.x0 = (rect.x0 - PRE_X).max(STRIP.x0);
        }
        if escalate {
            rect = union(rect, place(pos, b.slot));
        }
        // The killer portrait sits left of the row's first name: no
        // widening reaches past that name's box.
        rect.x0 = rect.x0.max(STRIP.x0 + pos.x0 + part.row_x0);
        let text_x1 = STRIP.x0 + pos.x1 + b.text.2;
        rect.x1 = rect
            .x1
            .min((VICTIM_X1 - PORTRAIT_CLEARANCE).max(text_x1 + TEXT_HALO));
        out.push(Candidate {
            rect,
            name,
            kind: b.kind,
            confidence,
            part: b.id,
            offset,
            owner,
            own: i == 0,
        });
    }
}

/// A gap position narrowed to where its frame shows the name: a gap holds
/// the union of the positions before and after it, and while a row slides
/// in that union reaches over the place of the moving killer portrait.
/// If the template's best score over the union is at least [`PIN_MIN`]
/// and beats every position more than [`PIN_RADIUS`] pixels away by
/// [`PIN_MARGIN`], the name is there (a clear single peak, even in a
/// faint frame); the gap keeps [`PIN_RADIUS`] around it. Otherwise the
/// whole union stays.
fn pin_gap(template: &Template, pos: Pos, frame: &Frame<'_>) -> Pos {
    if pos.x1 - pos.x0 <= 2 * PIN_RADIUS && pos.y1 - pos.y0 <= 2 * PIN_RADIUS {
        return pos;
    }
    let scores: Vec<(i32, i32, f32)> = (pos.y0..=pos.y1)
        .flat_map(|y| (pos.x0..=pos.x1).map(move |x| (x, y)))
        .filter_map(|(x, y)| score_at(frame.plane, template, x, y).map(|s| (x, y, s)))
        .collect();
    let Some(&(bx, by, best)) = scores.iter().max_by(|a, b| a.2.total_cmp(&b.2)) else {
        return pos;
    };
    let far =
        |&&(x, y, _): &&(i32, i32, f32)| (x - bx).abs() > PIN_RADIUS || (y - by).abs() > PIN_RADIUS;
    let runner_up = scores
        .iter()
        .filter(far)
        .map(|c| c.2)
        .fold(f32::MIN, f32::max);
    if best < PIN_MIN || best < runner_up + PIN_MARGIN {
        return pos;
    }
    Pos {
        x0: (bx - PIN_RADIUS).max(pos.x0),
        y0: (by - PIN_RADIUS).max(pos.y0),
        x1: (bx + PIN_RADIUS).min(pos.x1),
        y1: (by + PIN_RADIUS).min(pos.y1),
        how: pos.how,
    }
}

/// Position `pos` of frame `f` widened vertically to the rows the name has
/// in the frames before and after it, where those differ by at most
/// [`RISE_UNION`] pixels (port spec 7.1 step 6, vertically only). A
/// fading row slides up while its template still matches, and a match on
/// faint strokes can sit a few pixels off; the union keeps the strokes
/// inside without lagging a frame. Horizontal motion (slide-in) and stack
/// jumps are left alone: their union would reach over the portraits.
fn rise_union(part: &Part, f: u32, pos: Pos) -> Pos {
    [f.checked_sub(1), f.checked_add(1)]
        .into_iter()
        .flatten()
        .filter_map(|g| part.positions.get(&g))
        .filter(|n| {
            n.x0 == pos.x0
                && n.x1 == pos.x1
                && (n.y0 - pos.y0).abs() <= RISE_UNION
                && (n.y1 - pos.y1).abs() <= RISE_UNION
        })
        .fold(pos, |acc, n| Pos {
            y0: acc.y0.min(n.y0),
            y1: acc.y1.max(n.y1),
            ..acc
        })
}

/// Whether the detector read another name on `text`: one that reaches
/// beyond it. A fading name is often read shorter than it is; that
/// reading lies inside its text and does not count.
fn taken(text: RectI, detections: &[Detection]) -> bool {
    detections.iter().any(|d| {
        d.text.intersection(&text).is_some()
            && (d.text.x0 < text.x0 - SAME_EXTENT || d.text.x1 > text.x1 + SAME_EXTENT)
    })
}

/// What a frame shows of a companion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Seen {
    /// Its template matches at its place.
    Here,
    /// Its template does not match there, and the detector read names in
    /// its row but none with its extent.
    Away,
    /// Nothing decides it: not a companion, the followed name itself is
    /// not a clear match (a fade, a flash, a gap), or no detection in the
    /// row says otherwise.
    Unknown,
}

/// What the frame shows of each name of `part` with the followed name at
/// `pos` (found as `confidence`).
fn companions_seen(
    part: &Part,
    pos: Pos,
    confidence: Confidence,
    frame: Option<&Frame<'_>>,
    detections: &[Detection],
) -> Vec<Seen> {
    let clear = confidence == Confidence::Tracked
        && frame.is_some_and(|fr| {
            score_at(fr.plane, &part.template, pos.x0, pos.y0).is_some_and(|s| s >= CLEAR_MATCH)
        });
    part.boxes
        .iter()
        .map(|b| match (&b.companion, frame) {
            (Some(c), Some(fr)) if clear => companion_seen(part, b, c, pos, fr, detections),
            _ => Seen::Unknown,
        })
        .collect()
}

/// Where companion `b` of `part` is in `frame` with the followed name at
/// `pos`.
fn companion_seen(
    part: &Part,
    b: &PartBox,
    companion: &Companion,
    pos: Pos,
    frame: &Frame<'_>,
    detections: &[Detection],
) -> Seen {
    let corner = (pos.x0 + companion.corner.0, pos.y0 + companion.corner.1);
    match score_at(frame.plane, &companion.template, corner.0, corner.1) {
        Some(s) if s >= COMPANION_MIN => return Seen::Here,
        Some(_) => {}
        None => return Seen::Unknown,
    }
    let top = STRIP.y0 + pos.y0 + part.row_top;
    let text = place(pos, b.text);
    let mut row = detections
        .iter()
        .filter(|d| d.row_top.is_some_and(|t| (t - top).abs() <= ROW_TOLERANCE))
        .peekable();
    if row.peek().is_none() {
        return Seen::Unknown;
    }
    let read = row.any(|d| {
        (d.text.x0 - text.x0).abs() <= SAME_EXTENT && (d.text.x1 - text.x1).abs() <= SAME_EXTENT
    });
    if read { Seen::Unknown } else { Seen::Away }
}

/// The template of a companion name, cut in the reference frame.
#[derive(Debug, Clone)]
pub(super) struct Companion {
    pub(super) template: Template,
    /// Its top-left corner relative to the part's template corner.
    pub(super) corner: (i32, i32),
}

/// The slot boxes of the names of one row (boxes relative to any common
/// origin, in the row's order of the input): each name's box widened
/// towards the kill icon, up to the other name; a name alone reaches
/// [`SLOT_REACH`] towards where the icon is (left of a victim, right of a
/// killer).
pub(super) fn slots(row: &[(NameKind, Offset)]) -> Vec<Offset> {
    row.iter()
        .map(|&(kind, b)| {
            let left = row.iter().filter(|o| o.1.2 <= b.0).map(|o| o.1.2).max();
            let right = row.iter().filter(|o| o.1.0 >= b.2).map(|o| o.1.0).min();
            match (left, right) {
                (_, Some(r)) if left.is_none() => (b.0, b.1, r.max(b.2), b.3),
                (Some(l), _) => (l.min(b.0), b.1, b.2, b.3),
                _ if kind == NameKind::Killer => (b.0, b.1, b.2 + SLOT_REACH, b.3),
                _ => (b.0 - SLOT_REACH, b.1, b.2, b.3),
            }
        })
        .collect()
}

/// Whether a frame found as `c` is escalated to the slot box (plan 7.3).
const fn uncertain(c: Confidence) -> bool {
    matches!(
        c,
        Confidence::WeakHold | Confidence::GapFill | Confidence::PreRoll
    )
}

/// How uncertain a confidence is (higher: less certain).
const fn rank(c: Confidence) -> u8 {
    match c {
        Confidence::Tracked => 0,
        Confidence::PostRoll => 1,
        Confidence::WeakHold => 2,
        Confidence::GapFill => 3,
        // Pre-roll, colour prior, OCR fix and anything added later.
        _ => 4,
    }
}

/// Box `offset` (relative to the template corner) at position `pos`, in
/// reference pixels.
pub(super) const fn place(pos: Pos, offset: Offset) -> RectI {
    RectI {
        x0: STRIP.x0 + pos.x0 + offset.0,
        y0: STRIP.y0 + pos.y0 + offset.1,
        x1: STRIP.x0 + pos.x1 + offset.2,
        y1: STRIP.y0 + pos.y1 + offset.3,
    }
}

/// A reference rectangle in strip coordinates.
const fn local(r: RectI) -> RectI {
    RectI {
        x0: r.x0 - STRIP.x0,
        y0: r.y0 - STRIP.y0,
        x1: r.x1 - STRIP.x0,
        y1: r.y1 - STRIP.y0,
    }
}

/// The smallest rectangle containing both.
fn union(a: RectI, b: RectI) -> RectI {
    RectI {
        x0: a.x0.min(b.x0),
        y0: a.y0.min(b.y0),
        x1: a.x1.max(b.x1),
        y1: a.y1.max(b.y1),
    }
}

#[cfg(test)]
mod tests {
    use blurrd_core::{BlurParams, NameKind};

    use super::{PORTRAIT_CLEARANCE, SLOT_REACH, slots};

    #[test]
    fn clearance_is_the_default_padding() {
        assert_eq!(PORTRAIT_CLEARANCE, i32::from(BlurParams::default().pad.0));
    }

    #[test]
    fn slots_reach_the_other_name_or_towards_the_icon() {
        let killer = (NameKind::Killer, (100, 5, 160, 28));
        let victim = (NameKind::Victim, (300, 5, 380, 28));
        assert_eq!(
            slots(&[victim, killer]),
            vec![(160, 5, 380, 28), (100, 5, 300, 28)]
        );
        // One-colour rows: both names are victims, the order decides.
        let left = (NameKind::Victim, (100, 5, 160, 28));
        assert_eq!(
            slots(&[left, victim]),
            vec![(100, 5, 300, 28), (160, 5, 380, 28)]
        );
        // A self-kill: only the victim, the icon on its left.
        assert_eq!(slots(&[victim]), vec![(300 - SLOT_REACH, 5, 380, 28)]);
        assert_eq!(slots(&[killer]), vec![(100, 5, 160 + SLOT_REACH, 28)]);
    }
}
