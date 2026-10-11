//! The kill-feed temporal model (port spec section 7.1).
//!
//! The per-frame detector ([`crate::KillFeedDetector`]) misses names while
//! a row animates: the first frames of a row slide in from the left behind
//! a cover bar, a flash or a bright background washes a row out for a few
//! frames, and the oldest row fades away. [`KillFeedTracker`] follows every
//! name through these frames by template matching:
//!
//! 1. Detections of each kind are linked frame by frame into detection
//!    tracks (one name in one row slot).
//! 2. A track that showed the same text extent four times gives a
//!    template reference: the name and a few pixels around it on the
//!    `min(R, G, B)` plane, which is robust to flashes, fades and bright
//!    backgrounds because the correlation is normalised. More
//!    references follow during the track's life.
//! 3. Each reference is followed backward in time (to the row's slide-in)
//!    and forward (through stack jumps, flashes, gaps and the fade-out)
//!    with [`crate::track::ncc`].
//! 4. The blur box of every frame is the name's box moved by the tracked
//!    offset: a fixed size, so it never jitters. The other names of the
//!    reference row move with it (a row is rigid), which covers a name
//!    whose own template latched onto an identical name in the next row.
//! 5. A name ends where its row's structure is gone, not where the template
//!    was last found (measured against the bar colour and the stroke
//!    contrast of the reference frame), followed by a post-roll of the
//!    measured fade length ([`POST_ROLL`]).
//! 6. The boxes of a frame are composed from all followed names: motion
//!    unions, a pre-roll at a row's birth, slot boxes for uncertain
//!    frames, checked companions, portrait clearance and a merge of
//!    duplicates (see the `compose` module).
//! 7. Every reading of the frame that the boxes leave readable gets a box,
//!    so the tracker never covers less than the per-frame detector, also
//!    at the end of a video, where no later reference can be followed
//!    back.
//!
//! The prototype kept the strips of the whole clip in memory and tracked
//! mid-life references backward over the entire life. This tracker is
//! streaming: it keeps a ring of the last [`RING_FRAMES`] frames, follows
//! references forward as frames arrive, follows them backward only inside
//! the ring, and hands out the boxes of a frame [`DELAY`] frames after it
//! was pushed. Its memory does not depend on the length of the video.

mod compose;
mod end;
mod follow;
mod link;
mod ring;

use std::collections::BTreeMap;

use blurrd_core::{Confidence, NameKind, RectI, Region};

use crate::detection::Detection;
use crate::image::RgbView;
use crate::killfeed::{FEED_ROI, ROW_PITCH};
use crate::track::ncc::{Searcher, Template, score_at};

use compose::{Companion, PreRoll};
use end::Look;
use follow::{Follower, How, MAX_GAP, MAX_GAP_BACKWARD, Pos, SAME_EXTENT};
use link::{Linker, Reference, Replaced};
use ring::Ring;

/// The part of the feed crop the tracker keeps per frame, in reference
/// pixels: from the left end of the searched rows (`FEED_X0` of the
/// prototype) to the right edge, over the crop's full height.
pub const STRIP: RectI = RectI {
    x0: 1250,
    y0: FEED_ROI.y0,
    x1: FEED_ROI.x1,
    y1: FEED_ROI.y1,
};

/// Frames the tracker keeps.
pub const RING_FRAMES: u32 = 128;

/// The boxes of a frame are handed out once this many later frames have
/// been pushed: a name lost for up to 45 frames (`MAX_GAP`) can still
/// be re-acquired and the gap filled, and a reference taken later can
/// still be followed back to the frame.
pub const DELAY: u32 = RING_FRAMES - 1;

/// Frames after the end of a name's row structure that keep its box: the
/// longest measured fade (plan 7.3). On the 20 tuning clips with clean row
/// ends (68 names, no other row moving into the place) a fading row slides
/// up by 3 to 9 px (median 8) and is visible for 2 to 5 frames (median 4)
/// from its first upward step. The boxes cover every visible frame of all
/// 68 fades and stay 5 or 6 frames past the last visible one in 61 of them.
/// Slower fades (the synthetic ones take 18 frames) are held by the
/// structure end, not by the post-roll.
pub const POST_ROLL: u32 = 5;

/// Frames before a name's first frame that can get the pre-roll box at a
/// row's birth (`PRE`; see the `compose` module).
pub const PRE_ROLL: u32 = 3;

/// Frames before a name's first match that can lead in on the detector's
/// readings ([`Part::lead_in`]); as many as a backward gap may span
/// before the name counts as not yet there.
const LEAD_FRAMES: u32 = MAX_GAP_BACKWARD;

/// While a fading row is held, it may have slid up this far since the
/// last match (measured: at most 9 px while visible; the window leaves
/// room for a slower slide) ...
const FADE_RISE: i32 = 14;
/// ... or down this far (sub-pixel jitter) ...
const FADE_DROP: i32 = 2;
/// ... and this far sideways.
const FADE_DX: i32 = 2;

/// The template spans the text plus this many pixels on each side ...
const TEMPLATE_MARGIN_X: i32 = 6;
/// ... and these rows of the row band (from the row top, half-open).
const TEMPLATE_BAND: (i32, i32) = (4, 29);
/// A template flatter than this (standard deviation of `min(R, G, B)`)
/// has nothing to follow.
const MIN_TEMPLATE_STD: f64 = 3.0;

/// The killer portrait is measured in the reference frame and the frames
/// before it that read the row's first name at the same place, up to this
/// many frames in all.
const CLEARANCE_FRAMES: u32 = 4;

/// A part whose box overlaps a new reference's box this much in the
/// reference frame follows the same name.
const SAME_NAME_IOU: f32 = 0.7;
/// A part's box grows by at most this much to a longer reading of its name
/// (about two glyphs), unless both readings start at the same column.
const MAX_GROW: i32 = 16;
/// A part following a misread (wider) extent of a name is in the name's
/// row if its box overlaps the name's box this much.
const RETIRE_IOU: f32 = 0.5;

/// The widest roman numeral of a multikill badge ("III").
const BADGE_MAX_WIDTH: i32 = 24;
/// In a one-colour row the badge sits left of the killer portrait: at
/// least this far left of the next name (the names of a one-colour row
/// are only a round icon apart).
const BADGE_MIN_GAP: i32 = 60;

/// One frame for [`KillFeedTracker::push`].
#[derive(Debug, Clone, Copy)]
pub struct KillFeedFrame<'a> {
    /// Frame index; frames must be pushed in order without gaps.
    pub frame: u32,
    /// The [`FEED_ROI`] of the frame in reference scale (the crop the
    /// detector read).
    pub crop: RgbView<'a>,
    /// The kill-feed detections of the frame.
    pub detections: &'a [Detection],
}

/// One blur box of the kill feed in one frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FeedBox {
    /// The box in reference pixels.
    pub rect: RectI,
    /// Killer or victim (both names of a one-colour row are victims).
    pub kind: NameKind,
    /// How the box was found.
    pub confidence: Confidence,
    /// The tracked name the box belongs to (unique per tracker).
    pub part: u32,
    /// How far the name has moved since its reference frame: the anchor
    /// of the block grid, so the blur pattern moves with the name.
    pub offset: (i16, i16),
}

/// The kill-feed boxes of one frame.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FrameBoxes {
    /// Frame index.
    pub frame: u32,
    /// The boxes.
    pub boxes: Vec<FeedBox>,
}

/// Why a frame was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum TrackError {
    /// The crop does not have the size of [`FEED_ROI`].
    #[error("crop is {actual_width} x {actual_height} px, the tracker expects {width} x {height}")]
    CropSize {
        /// Expected width.
        width: u32,
        /// Expected height.
        height: u32,
        /// Width of the given crop.
        actual_width: u32,
        /// Height of the given crop.
        actual_height: u32,
    },
    /// Frames were pushed out of order or with a gap.
    #[error("frame {actual} pushed, the tracker expects frame {expected}")]
    FrameOrder {
        /// The frame the tracker expected.
        expected: u32,
        /// The frame that was pushed.
        actual: u32,
    },
}

/// A name of a part's row, relative to the part's template position
/// (`x0, y0, x1, y1` minus the template's top-left corner).
#[derive(Debug, Clone)]
struct PartBox {
    id: u32,
    kind: NameKind,
    /// The blur box.
    offset: (i32, i32, i32, i32),
    /// The text bounds.
    text: (i32, i32, i32, i32),
    /// The slot box (Tight mode escalates uncertain frames to it).
    slot: (i32, i32, i32, i32),
    /// The template of a companion name (`None` for the followed name and
    /// for a companion too flat to cut one).
    companion: Option<Companion>,
}

/// One followed reference with the names of its row.
#[derive(Debug, Clone)]
struct Part {
    kind: NameKind,
    /// The text bounds of the followed name in the reference frame.
    text: RectI,
    template: Template,
    /// Template position in the reference frame (strip coordinates).
    origin: (i32, i32),
    /// The reference name first, then the other names of its row.
    boxes: Vec<PartBox>,
    /// Template positions per frame.
    positions: BTreeMap<u32, Pos>,
    forward: Follower,
    /// A newer reference of the same name took over the forward pass:
    /// this part's last frame is not the end of the name.
    superseded: bool,
    /// The name's look in the reference frame, to tell when its row is
    /// gone.
    look: Look,
    /// The last frame of the name's row structure, once decided.
    end: Option<u32>,
    /// The row top relative to the template's top-left corner.
    row_top: i32,
    /// The left edge of the box of the row's first name, relative to the
    /// template's top-left corner.
    row_x0: i32,
    /// The left limit of the row's boxes, right of the killer portrait,
    /// relative to the template's top-left corner, if a portrait was
    /// found ([`compose::killer_clearance`]).
    killer_clear: Option<i32>,
    /// The pre-roll before the name's first frame, at a row's birth.
    pre_roll: Option<PreRoll>,
    /// Clear frames in a row that showed a companion away from its place
    /// (the followed template latched onto an identical name in another
    /// row).
    misplaced: u32,
}

impl Part {
    /// The last frame of the name, once decided: the last frame its row
    /// structure was on screen.
    fn end(&self) -> Option<u32> {
        self.end.filter(|_| !self.superseded)
    }

    /// Decides the end of the name once the forward pass is over: the
    /// frames after the last match are held while the row's structure is
    /// still on screen, at most [`MAX_GAP`] frames (the template would
    /// have found a name still there by then).
    fn close(&mut self, ring: &Ring, searcher: &mut Searcher) {
        if self.end.is_some() || self.superseded || !self.forward.is_done() {
            return;
        }
        let last = self.forward.last();
        let mut end = last;
        if let Some(at) = self.positions.get(&last) {
            let anchor = (at.x1, at.y1);
            for g in last + 1..=last + MAX_GAP {
                let Some(frame) = ring.frame(g) else { break };
                // A fading row slides up: look for the name a little higher.
                let (slid, slid_score) = searcher
                    .find(
                        frame.plane,
                        &self.template,
                        anchor,
                        (-FADE_DX, FADE_DX),
                        (-FADE_RISE, FADE_DROP),
                    )
                    .map_or((anchor, -1.0), |m| ((m.x, m.y), m.score));
                let anchor_score =
                    score_at(frame.plane, &self.template, anchor.0, anchor.1).unwrap_or(-1.0);
                if !self.look.present(&frame, slid, slid_score)
                    && !self.look.present(&frame, anchor, anchor_score)
                {
                    break;
                }
                self.positions
                    .insert(g, Pos::union(anchor, slid, How::Hold));
                end = g;
            }
        }
        self.end = Some(end);
    }

    /// Extends the name's positions to the frames right before its first
    /// one (down to frame `lo`, at most [`LEAD_FRAMES`]) where the template
    /// does not match yet but the detector reads the name: a row that is
    /// still fading in, or one born a row lower and moved up by a stack
    /// jump in the next frame. Neither the backward pass nor the slide-in
    /// search finds such a frame, and the pre-roll leaves frames with
    /// readings alone (they look like a stack jump).
    ///
    /// The frames lead in on [`lead_reading`]; the position moves with the
    /// reading's baseline.
    fn lead_in(&mut self, ring: &Ring, lo: u32) {
        let Some((&first, &start)) = self.positions.first_key_value() else {
            return;
        };
        let t = self.forward.text();
        let mut pos = start;
        for g in (lo..first).rev().take(LEAD_FRAMES as usize) {
            let Some(dy) = ring
                .get(g)
                .and_then(|slot| lead_reading(compose::place(pos, t), &slot.names))
            else {
                break;
            };
            pos = Pos {
                y0: pos.y0 + dy,
                y1: pos.y1 + dy,
                how: How::Lead,
                ..pos
            };
            self.positions.insert(g, pos);
        }
    }

    /// Whether the part can no longer contribute to frame `f` or later.
    fn finished_before(&self, f: u32) -> bool {
        self.forward.is_done()
            && match self.end() {
                Some(end) => f > end + POST_ROLL,
                None => f > self.forward.last(),
            }
    }

    /// The part's own box in frame `f`; with `matched`, only if the
    /// template matched there.
    fn own_box(&self, f: u32, matched: bool) -> Option<RectI> {
        let p = self.positions.get(&f)?;
        let o = self.boxes.first()?.offset;
        (!matched || matches!(p.how, How::Matched | How::Slide)).then_some(RectI {
            x0: STRIP.x0 + p.x0 + o.0,
            y0: STRIP.y0 + p.y0 + o.1,
            x1: STRIP.x0 + p.x1 + o.2,
            y1: STRIP.y0 + p.y1 + o.3,
        })
    }

    /// The template position the part gives frame `f`, with its
    /// confidence.
    fn position(&self, f: u32) -> Option<(Pos, Confidence)> {
        if let Some(p) = self.positions.get(&f) {
            let confidence = match p.how {
                How::Matched | How::Slide => Confidence::Tracked,
                How::Weak | How::Hold => Confidence::WeakHold,
                How::Gap | How::Lead => Confidence::GapFill,
            };
            return Some((*p, confidence));
        }
        let end = self.end()?;
        if f > end && f - end <= POST_ROLL {
            let p = self.positions.get(&end)?;
            return Some((*p, Confidence::PostRoll));
        }
        None
    }
}

/// Follows kill-feed names through the frames of a video.
///
/// Push every frame in order with [`KillFeedTracker::push`]; the boxes of
/// frame `f` become available through [`KillFeedTracker::drain_ready`]
/// after frame `f + DELAY` was pushed, the rest after
/// [`KillFeedTracker::finish`].
#[derive(Debug, Clone)]
pub struct KillFeedTracker {
    ring: Ring,
    linkers: [Linker; 2],
    parts: Vec<Part>,
    searcher: Searcher,
    detections: Vec<Detection>,
    references: Vec<Reference>,
    ready: Vec<FrameBoxes>,
    next_part: u32,
    /// The frame the next push must carry (`None` before the first).
    next_frame: Option<u32>,
    /// The next frame to hand out.
    next_emit: u32,
    /// The readings of the last frames handed out (frame index and
    /// readings), for the frames after them (`compose::cover_readings`).
    emitted: Vec<(u32, Vec<Detection>)>,
}

impl Default for KillFeedTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl KillFeedTracker {
    /// An empty tracker.
    pub fn new() -> Self {
        Self {
            ring: Ring::new(RING_FRAMES as usize),
            linkers: [Linker::new(NameKind::Killer), Linker::new(NameKind::Victim)],
            parts: Vec::new(),
            searcher: Searcher::new(),
            detections: Vec::new(),
            references: Vec::new(),
            ready: Vec::new(),
            next_part: 0,
            next_frame: None,
            next_emit: 0,
            emitted: Vec::new(),
        }
    }

    /// Adds the next frame.
    ///
    /// # Errors
    ///
    /// [`TrackError::CropSize`] if the crop is not the size of
    /// [`FEED_ROI`], [`TrackError::FrameOrder`] if the frame does not
    /// follow the previous one.
    pub fn push(&mut self, input: &KillFeedFrame<'_>) -> Result<(), TrackError> {
        let (width, height) = (FEED_ROI.width(), FEED_ROI.height());
        if (input.crop.width(), input.crop.height()) != (width, height) {
            return Err(TrackError::CropSize {
                width,
                height,
                actual_width: input.crop.width(),
                actual_height: input.crop.height(),
            });
        }
        let frame = input.frame;
        match self.next_frame {
            Some(expected) if expected != frame => {
                return Err(TrackError::FrameOrder {
                    expected,
                    actual: frame,
                });
            }
            Some(_) => {}
            None => self.next_emit = frame,
        }
        self.next_frame = Some(frame + 1);

        self.detections.clear();
        self.detections.extend(
            input
                .detections
                .iter()
                .filter(|d| d.region == Region::KillFeed && !is_badge(d, input.detections)),
        );
        self.ring.store(frame, &input.crop, &self.detections);

        // Follow every live name into the new frame.
        if let Some(current) = self.ring.frame(frame) {
            for part in &mut self.parts {
                let Part {
                    template,
                    positions,
                    forward,
                    ..
                } = part;
                forward.step(&current, template, &mut self.searcher, |g, p| {
                    positions.insert(g, p);
                });
                part.close(&self.ring, &mut self.searcher);
            }
        }

        self.references.clear();
        for linker in &mut self.linkers {
            linker.push(frame, &self.detections, &mut self.references);
        }
        for i in 0..self.references.len() {
            let r = self.references[i];
            self.add_part(r, frame);
        }

        while self.next_emit + DELAY <= frame {
            self.emit(self.next_emit);
            self.next_emit += 1;
        }
        Ok(())
    }

    /// Moves the boxes of every frame that is final into `out`, in frame
    /// order.
    pub fn drain_ready(&mut self, out: &mut Vec<FrameBoxes>) {
        out.append(&mut self.ready);
    }

    /// Ends the video: decides the remaining frames and moves their boxes
    /// into `out` (with any not yet drained). The tracker is empty
    /// afterwards and can start a new video.
    pub fn finish(&mut self, out: &mut Vec<FrameBoxes>) {
        if let Some(next) = self.next_frame {
            let newest = next - 1;
            for part in &mut self.parts {
                part.forward.stop();
                part.close(&self.ring, &mut self.searcher);
            }
            while self.next_emit <= newest {
                self.emit(self.next_emit);
                self.next_emit += 1;
            }
        }
        out.append(&mut self.ready);
        *self = Self::new();
    }

    /// Heap bytes the tracker holds: the frame ring, the templates and the
    /// positions of the names it follows, and boxes not yet drained.
    pub fn heap_bytes(&self) -> usize {
        let parts: usize = self
            .parts
            .iter()
            .map(|p| {
                p.template.heap_bytes()
                    + p.look.heap_bytes()
                    + p.boxes.capacity() * size_of::<PartBox>()
                    + p
                        .boxes
                        .iter()
                        .filter_map(|b| b.companion.as_ref())
                        .map(|c| c.template.heap_bytes())
                        .sum::<usize>()
                    // A B-tree entry plus its share of node overhead.
                    + p.positions.len() * 2 * size_of::<(u32, Pos)>()
            })
            .sum();
        self.ring.heap_bytes()
            + self.linkers.iter().map(Linker::heap_bytes).sum::<usize>()
            + self.parts.capacity() * size_of::<Part>()
            + parts
            + self.detections.capacity() * size_of::<Detection>()
            + self.references.capacity() * size_of::<Reference>()
            + self
                .emitted
                .iter()
                .map(|(_, ds)| {
                    size_of::<(u32, Vec<Detection>)>() + ds.capacity() * size_of::<Detection>()
                })
                .sum::<usize>()
            + self
                .ready
                .iter()
                .map(|f| size_of::<FrameBoxes>() + f.boxes.capacity() * size_of::<FeedBox>())
                .sum::<usize>()
    }

    /// Starts following reference `r`: backward inside the ring, then
    /// forward up to frame `newest`.
    fn add_part(&mut self, r: Reference, newest: u32) {
        let d = r.detection;
        if let Some(wide) = r.replaces {
            self.retire(&d, wide);
        }
        // A part that matched the same name in the reference frame already
        // follows it (the same detection track, or the track that continues
        // the name after a stack jump): the new reference adds nothing.
        // A template that latched onto an identical name in another row
        // does not overlap the detection, so its name gets a new part.
        let known = self.parts.iter().any(|p| {
            p.kind == d.kind
                && p.own_box(r.frame, true)
                    .is_some_and(|b| b.iou(&d.rect) >= SAME_NAME_IOU)
        });
        // A part's name (followed or companion) may have been read without
        // its last glyph in the part's reference frame (a dark or
        // washed-out frame): its box grows to the longer reading on the
        // right, where no portrait is at risk. A reading taken while the
        // row still wipes in can lack several glyphs: a name in the same
        // row that starts at the same column is the same name and grows
        // without the limit.
        for p in &mut self.parts {
            let Some(&pos) = p.positions.get(&r.frame) else {
                continue;
            };
            for b in &mut p.boxes {
                let rect = compose::place(pos, b.offset);
                let grow = d.rect.x1 - rect.x1;
                let text = compose::place(pos, b.text);
                let same_start = (text.x0 - d.text.x0).abs() <= SAME_EXTENT
                    && (text.y1 - d.text.y1).abs() <= SAME_EXTENT;
                if b.kind == d.kind
                    && grow > 0
                    && (same_start || grow <= MAX_GROW && rect.iou(&d.rect) >= SAME_NAME_IOU)
                {
                    b.offset.2 += grow;
                    b.text.2 += grow;
                    b.slot.2 = b.slot.2.max(b.offset.2);
                }
            }
        }
        if known {
            return;
        }
        let Some(mut part) = self.cut_part(r) else {
            return;
        };
        let text = part.forward.text();
        let mut insert = |g: u32, p: Pos| {
            part.positions.insert(g, p);
        };
        // Backward in time, down to the oldest frame not handed out yet.
        let lo = self.next_emit;
        let mut back = Follower::new(text, part.origin, r.frame, false);
        let mut f = r.frame;
        while f > lo && !back.is_done() {
            f -= 1;
            let Some(fr) = self.ring.frame(f) else { break };
            back.step(&fr, &part.template, &mut self.searcher, &mut insert);
        }
        let ring = &self.ring;
        back.slide_in(
            lo,
            &part.template,
            &mut self.searcher,
            |g| ring.frame(g),
            &mut insert,
        );
        part.lead_in(&self.ring, lo);
        part.pre_roll = compose::pre_roll(&part, &self.ring, lo);
        // Forward up to the newest frame.
        for g in r.frame + 1..=newest {
            if part.forward.is_done() {
                break;
            }
            let Some(fr) = self.ring.frame(g) else { break };
            let Part {
                template,
                positions,
                forward,
                ..
            } = &mut part;
            forward.step(&fr, template, &mut self.searcher, |g, p| {
                positions.insert(g, p);
            });
        }
        part.close(&self.ring, &mut self.searcher);

        // A part that only held the name weakly or across a gap in the
        // reference frame hands the forward pass over to the new one.
        for older in &mut self.parts {
            if older.kind == d.kind
                && !older.forward.is_done()
                && older
                    .own_box(r.frame, false)
                    .is_some_and(|b| b.iou(&d.rect) >= SAME_NAME_IOU)
            {
                older.forward.stop();
                older.superseded = true;
            }
        }
        self.parts.push(part);
    }

    /// Retires the parts that follow `wide`, a misreading of the name of
    /// detection `d`: they give no positions from the frame the misreading
    /// ended, where the parts of the correct reading cover the name.
    fn retire(&mut self, d: &Detection, wide: Replaced) {
        for p in &mut self.parts {
            let same_row = p
                .positions
                .range(wide.since.saturating_sub(1)..)
                .next()
                .is_some_and(|(_, at)| {
                    let o = p.boxes.first().map_or((0, 0, 0, 0), |b| b.offset);
                    compose::place(*at, o).iou(&d.rect) >= RETIRE_IOU
                });
            if p.kind == d.kind
                && (p.text.x0 - wide.x0).abs() <= SAME_EXTENT
                && (p.text.x1 - wide.x1).abs() <= SAME_EXTENT
                && same_row
            {
                p.forward.stop();
                p.superseded = true;
                p.positions.retain(|&g, _| g < wide.since);
            }
        }
    }

    /// A new part for reference `r`: the template cut around the name in
    /// the reference frame, the boxes of the names of its row, and the
    /// reference position. `None` if the frame left the ring or the
    /// template is too flat to follow.
    fn cut_part(&mut self, r: Reference) -> Option<Part> {
        let slot = self.ring.get(r.frame)?;
        let d = r.detection;
        let top = d.row_top?;
        let local = template_area(d.text, top);
        let template = Template::from_plane(&slot.mn, local)?;
        if template.std_dev() < MIN_TEMPLATE_STD {
            return None;
        }
        let frame = self.ring.frame(r.frame)?;
        let origin = (local.x0, local.y0);
        let area = RectI {
            x0: local.x0 + STRIP.x0,
            y0: local.y0 + STRIP.y0,
            x1: local.x1 + STRIP.x0,
            y1: local.y1 + STRIP.y0,
        };
        let relative = |rect: RectI| {
            (
                rect.x0 - area.x0,
                rect.y0 - area.y0,
                rect.x1 - area.x0,
                rect.y1 - area.y0,
            )
        };
        // The followed name first, then the other names of its row.
        let row: Vec<&Detection> = std::iter::once(&r.detection)
            .chain(
                slot.detections
                    .iter()
                    .filter(|c| c.row_top == d.row_top && c.text != d.text),
            )
            .collect();
        let slots = compose::slots(
            &row.iter()
                .map(|c| (c.kind, relative(c.rect)))
                .collect::<Vec<_>>(),
        );
        let mut boxes = Vec::with_capacity(row.len());
        for (i, (c, slot_box)) in row.iter().zip(slots).enumerate() {
            let companion = (i > 0)
                .then(|| {
                    let at = template_area(c.text, top);
                    Template::from_plane(&slot.mn, at)
                        .filter(|t| t.std_dev() >= MIN_TEMPLATE_STD)
                        .map(|template| Companion {
                            template,
                            corner: (at.x0 - local.x0, at.y0 - local.y0),
                        })
                })
                .flatten();
            boxes.push(PartBox {
                id: self.next_part + i as u32,
                kind: c.kind,
                offset: relative(c.rect),
                text: relative(c.text),
                slot: slot_box,
                companion,
            });
        }
        self.next_part += boxes.len() as u32;
        let row_x0 = boxes.iter().map(|b| b.offset.0).min().unwrap_or(0);
        let mut positions = BTreeMap::new();
        positions.insert(
            r.frame,
            Pos {
                x0: origin.0,
                y0: origin.1,
                x1: origin.0,
                y1: origin.1,
                how: How::Matched,
            },
        );
        let text = relative(d.text);
        let look = Look::new(
            &frame,
            origin,
            RectI {
                x0: text.0,
                y0: text.1,
                x1: text.2,
                y1: text.3,
            },
            top - area.y0,
        );
        Some(Part {
            kind: d.kind,
            text: d.text,
            template,
            origin,
            boxes,
            positions,
            forward: Follower::new(text, origin, r.frame, true),
            superseded: false,
            look,
            end: None,
            row_top: top - area.y0,
            row_x0,
            killer_clear: self.killer_clear(r.frame, top, &row).map(|x| x - origin.0),
            pre_roll: None,
            misplaced: 0,
        })
    }

    /// The killer portrait's clearance (strip coordinates) of the feed
    /// row at `top` with the detections `row`, seen in frame `frame`: the
    /// lowest of the frames up to it that read the row's first name at the
    /// same place (the background behind a translucent bar changes from
    /// frame to frame, the portrait does not), never right of the row's
    /// other names.
    fn killer_clear(&self, frame: u32, top: i32, row: &[&Detection]) -> Option<i32> {
        let first = row.iter().map(|c| c.text).min_by_key(|t| t.x0)?;
        let bound = row
            .iter()
            .map(|c| c.text.x0)
            .filter(|&x| x > first.x0)
            .min()
            .map_or(i32::MAX, |x| x - STRIP.x0 - compose::TEXT_HALO);
        (0..CLEARANCE_FRAMES)
            .filter_map(|k| self.ring.get(frame.checked_sub(k)?))
            .filter(|s| s.names.contains(&first))
            .filter_map(|s| {
                compose::killer_clearance(&s.mn, top - STRIP.y0, compose::local(first), bound)
            })
            .min()
    }

    /// Hands out frame `f` and forgets what only earlier frames needed.
    fn emit(&mut self, f: u32) {
        let mut boxes = Vec::new();
        let mut clearances = Vec::new();
        compose::compose(f, &mut self.parts, &self.ring, &mut boxes, &mut clearances);
        compose::cover_readings(
            f,
            &self.ring,
            &self.emitted,
            &clearances,
            &mut self.next_part,
            &mut boxes,
        );
        // Keep the readings of the last frames handed out, reusing the
        // oldest entry's buffer.
        let mut entry = if self.emitted.len() >= compose::ESCALATE_BRIDGE as usize {
            self.emitted.remove(0)
        } else {
            (f, Vec::new())
        };
        entry.0 = f;
        entry.1.clear();
        if let Some(slot) = self.ring.get(f) {
            entry.1.extend_from_slice(&slot.detections);
        }
        self.emitted.push(entry);
        self.ready.push(FrameBoxes { frame: f, boxes });
        let keep = f.saturating_sub(POST_ROLL);
        self.parts.retain_mut(|p| {
            if p.finished_before(f + 1) {
                return false;
            }
            // The post-roll reads the last position; keep it.
            let end = p.end();
            p.positions.retain(|&g, _| g >= keep || Some(g) == end);
            true
        });
    }
}

/// The template area of a name with text bounds `text` in the row at
/// `top` (strip coordinates): the text plus a margin on each side, over
/// the row's text band.
fn template_area(text: RectI, top: i32) -> RectI {
    RectI {
        x0: (text.x0 - TEMPLATE_MARGIN_X).max(STRIP.x0) - STRIP.x0,
        y0: (top + TEMPLATE_BAND.0).max(STRIP.y0) - STRIP.y0,
        x1: (text.x1 + TEMPLATE_MARGIN_X).min(STRIP.x1) - STRIP.x0,
        y1: (top + TEMPLATE_BAND.1).min(STRIP.y1) - STRIP.y0,
    }
}

/// How far below `text` (a name's text bounds where it is followed next)
/// the detector read the name in `names` (the text bounds of a frame
/// before), if it did: text inside the name's extent (a fading row is
/// often read short), with the name's baseline or one [`ROW_PITCH`] lower
/// (a stack jump right after the row's birth), all within
/// [`SAME_EXTENT`]. Of several such readings the one closest to the
/// name's extent wins; a wider reading is another name. A shorter name
/// one row lower inside the extent (victims are right-aligned) can lead
/// the box a row down for up to [`LEAD_FRAMES`] frames: over-blur, which
/// is safer than telling it from the name's own stack jump.
fn lead_reading(text: RectI, names: &[RectI]) -> Option<i32> {
    names
        .iter()
        .filter(|n| n.x0 >= text.x0 - SAME_EXTENT && n.x1 <= text.x1 + SAME_EXTENT)
        .filter_map(|n| {
            let dy = n.y1 - text.y1;
            (dy.abs() <= SAME_EXTENT || (dy - ROW_PITCH).abs() <= SAME_EXTENT)
                .then_some((dy, (n.x0 - text.x0).abs() + (n.x1 - text.x1).abs()))
        })
        .min_by_key(|&(_, error)| error)
        .map(|(dy, _)| dy)
}

/// Whether `d` is the roman numeral of a multikill badge. In a one-colour
/// row (no killer detection) the badge passes as a name: a short one,
/// left of the killer portrait, with both names of the row to its right.
/// A short killer name of a row read from luminance (also all victims)
/// has only the victim to its right.
fn is_badge(d: &Detection, all: &[Detection]) -> bool {
    if d.kind != NameKind::Victim || d.text.x1 - d.text.x0 > BADGE_MAX_WIDTH {
        return false;
    }
    let row = || {
        all.iter()
            .filter(|o| o.row_top == d.row_top && o.region == d.region)
    };
    if row().any(|o| o.kind == NameKind::Killer || o.text.x0 < d.text.x0) {
        return false;
    }
    let right = || row().filter(|o| o.text.x0 > d.text.x0);
    right().count() >= 2
        && right()
            .map(|o| o.text.x0 - d.text.x1)
            .min()
            .is_some_and(|gap| gap >= BADGE_MIN_GAP)
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        reason = "test helpers on small frames: a panic is a failure"
    )]

    use blurrd_core::{NameKind, RectI, Region};

    use super::{FEED_ROI, FrameBoxes, KillFeedFrame, KillFeedTracker, ROW_PITCH, lead_reading};
    use crate::detection::Detection;
    use crate::image::RgbView;

    /// A name's text bounds from `x0` to `x1` with its baseline at `y1`.
    fn text(x0: i32, x1: i32, y1: i32) -> RectI {
        RectI {
            x0,
            y0: y1 - 14,
            x1,
            y1,
        }
    }

    #[test]
    fn leads_in_on_a_short_reading_at_the_place_of_the_name() {
        let name = text(1731, 1830, 199);
        // A fading-in victim read without its first glyphs, a little lower.
        assert_eq!(lead_reading(name, &[text(1765, 1830, 196)]), Some(-3));
        // The whole name.
        assert_eq!(lead_reading(name, &[text(1730, 1831, 199)]), Some(0));
    }

    #[test]
    fn leads_in_on_a_reading_one_row_lower() {
        // Born a row lower, moved up by a stack jump in the next frame.
        let name = text(1786, 1830, 157);
        assert_eq!(
            lead_reading(name, &[text(1787, 1823, 157 + ROW_PITCH)]),
            Some(ROW_PITCH)
        );
    }

    #[test]
    fn leads_in_on_a_shorter_victim_one_row_lower() {
        // Documented over-blur: a shorter victim in the row below lies
        // inside the name's extent (victims are right-aligned) and passes
        // for the name born a row lower.
        let name = text(1731, 1830, 157);
        let below = text(1770, 1830, 157 + ROW_PITCH);
        assert_eq!(lead_reading(name, &[below]), Some(ROW_PITCH));
        // The name's own reading at its place wins over it.
        let own = text(1765, 1830, 157);
        assert_eq!(lead_reading(name, &[below, own]), Some(0));
    }

    #[test]
    fn does_not_lead_in_on_other_names() {
        let name = text(1786, 1830, 157);
        // A longer name at the place (the row that sat there before).
        assert_eq!(lead_reading(name, &[text(1731, 1830, 160)]), None);
        // A name in another row.
        assert_eq!(
            lead_reading(name, &[text(1786, 1830, 157 - ROW_PITCH)]),
            None
        );
        assert_eq!(lead_reading(name, &[text(1786, 1830, 177)]), None);
        assert_eq!(lead_reading(name, &[]), None);
    }

    /// A kill-feed reading in the row at `top` (the detector's band rule
    /// for the box).
    fn reading(kind: NameKind, x0: i32, x1: i32, top: i32) -> Detection {
        Detection {
            region: Region::KillFeed,
            kind,
            text: RectI {
                x0,
                y0: top + 10,
                x1,
                y1: top + 24,
            },
            rect: RectI {
                x0: x0 - 2,
                y0: top + 3,
                x1: x1 + 2,
                y1: top + 31,
            },
            row_top: Some(top),
        }
    }

    /// Runs the tracker over flat frames (nothing to cut a template from:
    /// no name becomes a part) with the readings `readings(frame)`.
    fn track_flat(frames: u32, readings: impl Fn(u32) -> Vec<Detection>) -> Vec<FrameBoxes> {
        let (w, h) = (FEED_ROI.width(), FEED_ROI.height());
        let crop = vec![90_u8; (3 * w * h) as usize];
        let view = RgbView::new(&crop, w, h, 3 * w as usize).unwrap();
        let mut tracker = KillFeedTracker::new();
        let mut out = Vec::new();
        for frame in 0..frames {
            let detections = readings(frame);
            tracker
                .push(&KillFeedFrame {
                    frame,
                    crop: view,
                    detections: &detections,
                })
                .unwrap();
            tracker.drain_ready(&mut out);
        }
        tracker.finish(&mut out);
        assert_eq!(out.len(), frames as usize);
        out
    }

    /// Whether a box of `boxes` contains the text of `d`.
    fn boxed(boxes: &FrameBoxes, d: &Detection) -> bool {
        boxes.boxes.iter().any(|b| {
            b.rect.x0 <= d.text.x0
                && b.rect.x1 >= d.text.x1
                && b.rect.y0 <= d.text.y0
                && b.rect.y1 >= d.text.y1
        })
    }

    #[test]
    fn boxes_readings_that_no_part_follows() {
        // A row born in the last frames of a video: too few readings for
        // a reference, and no later frame to follow one back from.
        let killer = reading(NameKind::Killer, 1500, 1600, 120);
        let victim = reading(NameKind::Victim, 1700, 1820, 120);
        for born in [1, 2, 3] {
            let frames = 40;
            let out = track_flat(frames, |f| {
                if f + born >= frames {
                    vec![killer, victim]
                } else {
                    Vec::new()
                }
            });
            for entry in &out[(frames - born) as usize..] {
                assert!(boxed(entry, &killer) && boxed(entry, &victim), "{entry:?}");
                // Escalated to the slot box: the killer's reaches the
                // victim.
                assert!(entry.boxes.iter().any(|b| b.rect.x1 >= victim.rect.x0));
            }
            assert!(
                out[..(frames - born) as usize]
                    .iter()
                    .all(|e| e.boxes.is_empty())
            );
        }
    }

    #[test]
    fn boxes_readings_whose_extent_alternates() {
        // The name's readings alternate between two widths every frame (and
        // miss single frames): the linker never sees one extent four times.
        let wide = reading(NameKind::Victim, 1690, 1820, 160);
        let narrow = reading(NameKind::Victim, 1722, 1820, 160);
        let read = |f: u32| match f % 5 {
            0 | 2 => vec![wide],
            1 | 3 => vec![narrow],
            _ => Vec::new(),
        };
        let out = track_flat(200, read);
        for (f, entry) in (0..).zip(&out) {
            // Frames without a reading between two that read it are bridged
            // (the last one has no frame after it).
            let expected = match read(f).first() {
                Some(d) => Some(*d),
                None if f + 1 < 200 => Some(narrow),
                None => None,
            };
            if let Some(d) = expected {
                assert!(boxed(entry, &d), "frame {f}: {entry:?}");
            }
        }
    }

    #[test]
    fn leads_in_on_the_reading_closest_to_the_name() {
        let name = text(1700, 1830, 160);
        let short = text(1760, 1830, 160);
        let lower = text(1701, 1830, 160 + ROW_PITCH);
        assert_eq!(lead_reading(name, &[short, lower]), Some(ROW_PITCH));
    }
}
