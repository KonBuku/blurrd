//! Voice-chat entries over time: bidirectional hysteresis (port spec
//! section 7.2, plan 7.3).
//!
//! The per-frame detector misses an entry for single frames (a flash, a
//! bright background swallowing the white name) and reports texture as an
//! entry for single frames. [`VoiceTracker`] turns the per-slot evidence
//! into boxes:
//!
//! - **Tracks.** Consecutive entry frames of a slot form a track. A track
//!   counts only after two consecutive entry frames or one strong name
//!   frame (port spec F12: gameplay texture passes for one frame).
//! - **Identity.** Entry frames that follow each other belong to one track
//!   (a speaker change from one frame to the next only widens the box to
//!   the wider name). Across frames without an entry detection, the
//!   portrait signature tells whether the portrait is still the same one:
//!   an entry keeps its portrait while it is shown. The game behind the
//!   portrait's transparent corners moves the signature, so each frame is
//!   compared with its neighbour.
//! - **Backfill.** When a track counts, it reaches back to its first
//!   entry frame (offline, not causal like the prototype) and further
//!   while the same portrait is visible (frames whose name the detector
//!   missed).
//! - **Holds and gaps.** After its last entry frame a track holds while
//!   the same portrait stays visible, and bridges a gap of a few frames to
//!   the next entry frame with the same portrait; then one frame of
//!   post-roll (port spec 7.2).
//! - **Scene cuts** ([`crate::scene_cut`]) stop the extensions: no
//!   backfill, hold, gap or post-roll crosses a cut, but a track that
//!   keeps detecting on both sides of a cut is not ended by it.
//!
//! The box of a frame comes from the whole track: the widest name and the
//! tallest rows any frame of the track showed, or the full name slot for a
//! track that never showed a name (entries found from their structure
//! alone). Decisions reach at most [`HOLD`] frames back, so the boxes of a
//! frame are final [`DELAY`] frames later.

use std::collections::VecDeque;

use blurrd_core::{Confidence, Mode, RectI};

use super::{BOX_ROWS, PortraitSignature, SLOT_X1, SLOTS, SlotEvidence, SlotHit, entry_detection};

/// Name score at which one entry frame is enough to start a track.
pub const STRONG_SCORE: f32 = 2.0;
/// Largest portrait-signature distance
/// ([`PortraitSignature::distance`]) between two neighbouring frames of
/// the same entry. Measured on the recordings: 99 % of neighbouring
/// frames of one entry within 0.27 (99.9 % within 0.6, when the game
/// behind the portrait corners changes fast), different agent portraits
/// at 1.09 (median).
pub const SAME_PORTRAIT: f32 = 0.5;
/// Portrait texture a frame needs to extend a track without an entry
/// detection: half a full portrait, the least the detector keeps.
pub const HOLD_PORTRAIT: f32 = super::MEASURE_PORTRAIT;
/// Most frames a track reaches beyond its entry frames on the strength of
/// its portrait alone (backfill and hold), one second at 60 fps.
pub const HOLD: u32 = 60;
/// Longest run of frames without a portrait match that a track bridges
/// between two entry frames of the same entry (blink-offs last 5 to 10
/// frames, port spec section 5).
pub const GAP: u32 = 12;
/// Frames added after a track (post-roll, port spec 7.2: entries pop off
/// within one frame). There is no pre-roll: the backfill reaches the
/// first frame that shows the portrait.
pub const POST_ROLL: u32 = 1;
/// Frames after which the boxes of a frame are final: a track that counts
/// at frame `n` reaches back at most [`HOLD`] + 1 frames (backfill from its
/// first entry frame `n - 1`); gap fills and post-rolls reach less far.
pub const DELAY: u32 = HOLD + 2;

/// One frame for [`VoiceTracker::push`].
#[derive(Debug, Clone, Copy)]
pub struct VoiceFrame<'a> {
    /// Frame index; frames must be pushed in order without gaps.
    pub frame: u32,
    /// What the detector measured in every slot of the frame
    /// ([`super::VoiceDetector::slots`]).
    pub slots: &'a [SlotEvidence; SLOTS],
    /// Whether a scene cut that concerns the voice chat lies between the
    /// previous frame and this one ([`crate::scene_cut::SceneCut::is_cut_for`]).
    pub cut: bool,
}

/// One blur box of the voice chat in one frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VoiceBox {
    /// The box in reference pixels.
    pub rect: RectI,
    /// The name's text bounds in reference pixels (the anchor of the block
    /// grid).
    pub text: RectI,
    /// The slot (0 = bottom).
    pub slot: u8,
    /// How the box was found: `Tracked` on name frames, `WeakHold` on
    /// structure-only frames, backfill and portrait holds, `GapFill` and
    /// `PostRoll` on the extensions.
    pub confidence: Confidence,
    /// The track the box belongs to (unique per tracker).
    pub track: u32,
}

/// The voice-chat boxes of one frame, bottom slot first.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VoiceFrameBoxes {
    /// Frame index.
    pub frame: u32,
    /// The boxes.
    pub boxes: Vec<VoiceBox>,
}

/// Why a frame was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum VoiceTrackError {
    /// The frame does not follow the previous one.
    #[error("frame {actual} pushed, the tracker expects frame {expected}")]
    FrameOrder {
        /// The frame the tracker expected.
        expected: u32,
        /// The frame that was pushed.
        actual: u32,
    },
}

/// What the tracker keeps of one slot of one buffered frame.
#[derive(Debug, Clone, Copy)]
struct Cell {
    hit: SlotHit,
    portrait: f32,
    signature: PortraitSignature,
    /// The track covering the frame and how.
    cover: Option<(u32, Confidence)>,
}

/// A buffered frame.
#[derive(Debug, Clone)]
struct Buffered {
    frame: u32,
    cells: [Cell; SLOTS],
}

/// The geometry of a track: what its frames showed of the name.
#[derive(Debug, Clone, Copy)]
struct Geometry {
    id: u32,
    slot: u8,
    top: i32,
    /// Right end of the widest name seen, `None` before a name frame.
    name_x1: Option<i32>,
    /// The tallest box rows seen.
    rows: (i32, i32),
    /// The last frame the track covers so far.
    last: u32,
}

impl Geometry {
    /// The box of a frame of the track: the widest name seen, or the
    /// whole name slot for frames not confirmed by their own reading
    /// ([`Confidence::escalates`], plan 7.3), before a name frame and in
    /// Safe mode (`safe`).
    fn boxed(&self, confidence: Confidence, safe: bool) -> VoiceBox {
        let x1 = match self.name_x1 {
            Some(x) if !confidence.escalates() && !safe => x,
            _ => SLOT_X1,
        };
        let d = entry_detection(self.top, self.rows, x1);
        VoiceBox {
            rect: d.rect,
            text: d.text,
            slot: self.slot,
            confidence,
            track: self.id,
        }
    }

    /// Takes in the name of an entry frame.
    fn add(&mut self, evidence: &SlotEvidence) {
        if evidence.hit == SlotHit::Name {
            self.name_x1 = Some(
                self.name_x1
                    .map_or(evidence.name_x1, |x| x.max(evidence.name_x1)),
            );
            self.rows = (
                self.rows.0.min(evidence.rows.0),
                self.rows.1.max(evidence.rows.1),
            );
        }
    }
}

/// The open track of a slot.
#[derive(Debug, Clone, Copy)]
struct Open {
    id: u32,
    /// First entry frame.
    start: u32,
    /// Last entry frame.
    last_hit: u32,
    /// Signature at the first entry frame.
    first_signature: PortraitSignature,
    /// Signature of the latest frame the track covers with its portrait
    /// (entry frame or hold).
    recent_signature: PortraitSignature,
    /// Consecutive entry frames up to `last_hit`.
    run: u32,
    /// Whether the track counts (two consecutive entry frames or one
    /// strong name frame).
    counts: bool,
    /// Whether the portrait hold after `last_hit` still continues.
    holding: bool,
    /// First frame no extension of the track may reach: the frame of a
    /// scene cut after the track started.
    barrier: Option<u32>,
}

/// Follows the voice-chat entries of one video and decides their boxes.
///
/// Push every frame in order with [`VoiceTracker::push`]; the boxes of
/// frame `f` become available through [`VoiceTracker::drain_ready`] after
/// frame `f + DELAY` was pushed, the rest after [`VoiceTracker::finish`].
#[derive(Debug, Clone, Default)]
pub struct VoiceTracker {
    /// Frames not yet handed out, oldest first.
    buffer: VecDeque<Buffered>,
    /// The frame the next push must carry (`None` before the first).
    next: Option<u32>,
    /// Frames with a scene cut before them that are still buffered.
    cuts: VecDeque<u32>,
    /// The open track per slot.
    open: [Option<Open>; SLOTS],
    /// Geometry of the tracks that still cover buffered frames.
    tracks: Vec<Geometry>,
    /// Next track id.
    next_id: u32,
    /// Decided frames not yet drained.
    ready: Vec<VoiceFrameBoxes>,
    /// The box geometry ([`VoiceTracker::set_mode`]).
    mode: Mode,
}

impl VoiceTracker {
    /// An empty tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the box geometry. [`Mode::Safe`] boxes every frame over the
    /// whole name slot, as Tight mode does for frames of low confidence
    /// only (plan 7.3).
    pub const fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
    }

    /// Adds the next frame.
    ///
    /// # Errors
    ///
    /// [`VoiceTrackError::FrameOrder`] if the frame does not follow the
    /// previous one.
    pub fn push(&mut self, input: &VoiceFrame<'_>) -> Result<(), VoiceTrackError> {
        let n = input.frame;
        if let Some(expected) = self.next
            && expected != n
        {
            return Err(VoiceTrackError::FrameOrder {
                expected,
                actual: n,
            });
        }
        self.next = Some(n.saturating_add(1));
        if input.cut {
            self.cuts.push_back(n);
        }
        self.buffer.push_back(Buffered {
            frame: n,
            cells: input.slots.map(|e| Cell {
                hit: e.hit,
                portrait: e.portrait,
                signature: e.signature,
                cover: None,
            }),
        });
        for (k, evidence) in input.slots.iter().enumerate() {
            if input.cut
                && let Some(open) = &mut self.open[k]
            {
                open.barrier = Some(n);
            }
            self.step(k, n, evidence);
        }
        while self
            .buffer
            .front()
            .is_some_and(|b| b.frame.saturating_add(DELAY) <= n)
        {
            self.emit_front();
        }
        Ok(())
    }

    /// Moves the boxes of every frame that is final into `out`, in frame
    /// order.
    pub fn drain_ready(&mut self, out: &mut Vec<VoiceFrameBoxes>) {
        out.append(&mut self.ready);
    }

    /// Ends the video: closes the open tracks, decides the remaining
    /// frames and moves their boxes into `out` (with any not yet drained).
    /// The tracker is empty afterwards and can start a new video.
    pub fn finish(&mut self, out: &mut Vec<VoiceFrameBoxes>) {
        for k in 0..SLOTS {
            self.close(k);
        }
        while !self.buffer.is_empty() {
            self.emit_front();
        }
        self.drain_ready(out);
        *self = Self::default();
    }

    /// Heap bytes the tracker holds: the frame buffer, the track
    /// geometries and decided frames not yet drained.
    pub fn heap_bytes(&self) -> usize {
        self.buffer.capacity() * size_of::<Buffered>()
            + self.tracks.capacity() * size_of::<Geometry>()
            + self.ready.capacity() * size_of::<VoiceFrameBoxes>()
            + self
                .ready
                .iter()
                .map(|r| r.boxes.capacity() * size_of::<VoiceBox>())
                .sum::<usize>()
    }

    /// The buffered cell of slot `k` in frame `f`, if still buffered.
    fn cell(&mut self, f: u32, k: usize) -> Option<&mut Cell> {
        let first = self.buffer.front()?.frame;
        let i = usize::try_from(f.checked_sub(first)?).ok()?;
        self.buffer.get_mut(i).map(|b| &mut b.cells[k])
    }

    /// Covers frame `f` of slot `k` with track `id`, unless another track
    /// covers it already; a gap fill upgrades a hold of the same track.
    fn cover(&mut self, f: u32, k: usize, id: u32, confidence: Confidence) {
        let Some(cell) = self.cell(f, k) else {
            return;
        };
        match cell.cover {
            None => cell.cover = Some((id, confidence)),
            Some((owner, Confidence::WeakHold))
                if owner == id && confidence == Confidence::GapFill =>
            {
                cell.cover = Some((id, confidence));
            }
            Some(_) => return,
        }
        if let Some(g) = self.tracks.iter_mut().find(|g| g.id == id) {
            g.last = g.last.max(f);
        }
    }

    /// The earliest frame an extension backwards from `start` may reach:
    /// not before the latest scene cut at or before `start`.
    fn floor(&self, start: u32) -> u32 {
        self.cuts
            .iter()
            .rev()
            .find(|&&c| c <= start)
            .copied()
            .unwrap_or(0)
    }

    /// Processes slot `k` of the newest frame `n`.
    fn step(&mut self, k: usize, n: u32, evidence: &SlotEvidence) {
        if evidence.is_entry() {
            self.entry(k, n, evidence);
        } else {
            self.miss(k, n, evidence);
        }
    }

    /// An entry frame in slot `k`.
    fn entry(&mut self, k: usize, n: u32, evidence: &SlotEvidence) {
        let signature = evidence.signature;
        let continues = self.open[k].is_some_and(|open| {
            let same = open.recent_signature.distance(&signature) <= SAME_PORTRAIT;
            let crossed = open.barrier.is_some_and(|b| b <= n);
            let bridged = open.holding || n - open.last_hit <= GAP + 1;
            // Entry frames that follow each other stay one track, also
            // across a cut; a counting track bridges a hold or a gap to the
            // same portrait, but not across a cut.
            open.last_hit + 1 == n || (same && open.counts && !crossed && bridged)
        });
        if !continues {
            self.close(k);
            let id = self.next_id;
            self.next_id += 1;
            self.tracks.push(Geometry {
                id,
                slot: k as u8,
                top: evidence.top,
                name_x1: None,
                rows: BOX_ROWS,
                last: n,
            });
            self.open[k] = Some(Open {
                id,
                start: n,
                last_hit: n,
                first_signature: signature,
                recent_signature: signature,
                run: 0,
                counts: false,
                holding: true,
                barrier: None,
            });
        }
        let Some(mut open) = self.open[k] else {
            return;
        };
        if let Some(g) = self.tracks.iter_mut().find(|g| g.id == open.id) {
            g.add(evidence);
        }
        open.run = if open.last_hit + 1 == n {
            open.run + 1
        } else {
            1
        };
        let confidence = if evidence.hit == SlotHit::Name {
            Confidence::Tracked
        } else {
            Confidence::WeakHold
        };
        if open.counts {
            for f in open.last_hit + 1..n {
                self.cover(f, k, open.id, Confidence::GapFill);
            }
            self.cover(n, k, open.id, confidence);
        }
        open.last_hit = n;
        open.recent_signature = signature;
        open.holding = true;
        // Extensions after a hit beyond the cut stay on its side.
        if open.barrier.is_some_and(|b| b <= n) {
            open.barrier = None;
        }
        let strong = evidence.hit == SlotHit::Name && evidence.score >= STRONG_SCORE;
        if !open.counts && (open.run >= 2 || strong) {
            open.counts = true;
            self.open[k] = Some(open);
            self.start_counting(k, &open);
        } else {
            self.open[k] = Some(open);
        }
    }

    /// A track of slot `k` starts to count: cover its entry frames so far
    /// and backfill while its portrait shows.
    fn start_counting(&mut self, k: usize, open: &Open) {
        for f in open.start..=open.last_hit {
            let hit = self.cell(f, k).map_or(SlotHit::None, |c| c.hit);
            let confidence = if hit == SlotHit::Name {
                Confidence::Tracked
            } else {
                Confidence::WeakHold
            };
            self.cover(f, k, open.id, confidence);
        }
        let floor = self.floor(open.start);
        let mut first = open.start;
        let mut reference = open.first_signature;
        while first > floor && open.start - (first - 1) <= HOLD {
            let f = first - 1;
            let Some(signature) = self.cell(f, k).and_then(|c| {
                let matches = c.cover.is_none()
                    && c.portrait >= HOLD_PORTRAIT
                    && c.signature.distance(&reference) <= SAME_PORTRAIT;
                matches.then_some(c.signature)
            }) else {
                break;
            };
            self.cover(f, k, open.id, Confidence::WeakHold);
            reference = signature;
            first = f;
        }
    }

    /// A frame without an entry in slot `k`.
    fn miss(&mut self, k: usize, n: u32, evidence: &SlotEvidence) {
        let Some(mut open) = self.open[k] else {
            return;
        };
        let crossed = open.barrier.is_some_and(|b| b <= n);
        if open.counts && open.holding && !crossed {
            let holds = n - open.last_hit <= HOLD
                && evidence.portrait >= HOLD_PORTRAIT
                && evidence.signature.distance(&open.recent_signature) <= SAME_PORTRAIT;
            if holds {
                self.cover(n, k, open.id, Confidence::WeakHold);
                open.recent_signature = evidence.signature;
            } else {
                open.holding = false;
            }
        } else {
            open.holding = false;
        }
        let gap_over = crossed || n - open.last_hit > GAP;
        self.open[k] = Some(open);
        if !open.holding && gap_over {
            self.close(k);
        }
    }

    /// Closes the open track of slot `k`: adds the post-roll after the last
    /// frame it covers (not across a scene cut). A track that never
    /// counted is dropped.
    fn close(&mut self, k: usize) {
        let Some(open) = self.open[k].take() else {
            return;
        };
        if !open.counts {
            self.tracks.retain(|g| g.id != open.id);
            return;
        }
        let Some(last) = self.tracks.iter().find(|g| g.id == open.id).map(|g| g.last) else {
            return;
        };
        for r in 1..=POST_ROLL {
            let f = last + r;
            // The post-roll stops at a cut and at the next entry: a new
            // speaker in the slot gets its own box.
            let entry = self.cell(f, k).is_some_and(|c| c.hit != SlotHit::None);
            if entry || open.barrier.is_some_and(|b| f >= b) {
                break;
            }
            self.cover(f, k, open.id, Confidence::PostRoll);
        }
    }

    /// Decides the oldest buffered frame.
    fn emit_front(&mut self) {
        let Some(front) = self.buffer.pop_front() else {
            return;
        };
        let safe = self.mode == Mode::Safe;
        let boxes = front
            .cells
            .iter()
            .filter_map(|c| c.cover)
            .filter_map(|(id, confidence)| {
                self.tracks
                    .iter()
                    .find(|g| g.id == id)
                    .map(|g| g.boxed(confidence, safe))
            })
            .collect();
        let open = self.open.map(|o| o.map(|o| o.id));
        self.tracks
            .retain(|g| g.last > front.frame || open.contains(&Some(g.id)));
        while self.cuts.front().is_some_and(|&c| c <= front.frame) {
            self.cuts.pop_front();
        }
        self.ready.push(VoiceFrameBoxes {
            frame: front.frame,
            boxes,
        });
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use blurrd_core::Confidence;

    use super::{DELAY, GAP, HOLD, VoiceFrame, VoiceFrameBoxes, VoiceTrackError, VoiceTracker};
    use crate::voice::{
        BOX_ROWS, PortraitSignature, SIGNATURE_SIDE, SLOT_X1, SLOTS, SlotEvidence, SlotHit,
        entry_detection, slot_top,
    };

    /// What one slot shows in one frame of a test sequence.
    #[derive(Debug, Clone, Copy, PartialEq)]
    enum Show {
        /// Nothing: no portrait.
        Empty,
        /// An entry the detector found, with portrait `p`, name end `x1`
        /// and name score `score`.
        Name { p: u8, x1: i32, score: f32 },
        /// An entry found from its structure alone.
        Structure { p: u8 },
        /// Portrait `p` visible, but no entry detected.
        Portrait { p: u8 },
        /// A portrait between portraits 1 (`t` = 0) and 2 (`t` = 30),
        /// no entry detected.
        Between { t: u8 },
    }

    /// The signature of test portrait `p`: a texture of its own.
    fn pattern(p: u8) -> [u8; SIGNATURE_SIDE * SIGNATURE_SIDE] {
        let mut out = [0; SIGNATURE_SIDE * SIGNATURE_SIDE];
        for (i, v) in out.iter_mut().enumerate() {
            *v = (40 + (i * (2 * usize::from(p) + 1) * 37 + usize::from(p) * 13) % 160) as u8;
        }
        out
    }

    fn evidence(k: usize, show: Show) -> SlotEvidence {
        let signature = |p: u8| PortraitSignature(pattern(p));
        let base = SlotEvidence {
            slot: k as u8,
            top: slot_top(k),
            rows: BOX_ROWS,
            ..SlotEvidence::default()
        };
        match show {
            Show::Empty => base,
            Show::Name { p, x1, score } => SlotEvidence {
                portrait: 3.0,
                score,
                hit: SlotHit::Name,
                name_x1: x1,
                signature: signature(p),
                ..base
            },
            Show::Structure { p } => SlotEvidence {
                portrait: 3.0,
                score: 0.5,
                hit: SlotHit::Structure,
                name_x1: SLOT_X1,
                signature: signature(p),
                ..base
            },
            Show::Portrait { p } => SlotEvidence {
                portrait: 3.0,
                signature: signature(p),
                ..base
            },
            Show::Between { t } => {
                let (a, b) = (pattern(1), pattern(2));
                let mut mixed = a;
                for ((m, &a), &b) in mixed.iter_mut().zip(&a).zip(&b) {
                    let (a, b, t) = (i32::from(a), i32::from(b), i32::from(t));
                    *m = (a + (b - a) * t / 30) as u8;
                }
                SlotEvidence {
                    portrait: 3.0,
                    signature: PortraitSignature(mixed),
                    ..base
                }
            }
        }
    }

    /// Runs slot 0 through a tracker, with scene cuts before the frames in
    /// `cuts`, and returns the boxes and the confidence per frame.
    fn run(shows: &[Show], cuts: &[u32]) -> (Vec<VoiceFrameBoxes>, Vec<Option<Confidence>>) {
        let mut tracker = VoiceTracker::new();
        let mut out = vec![];
        for (f, &show) in shows.iter().enumerate() {
            let mut slots = [SlotEvidence::default(); SLOTS];
            for (k, s) in slots.iter_mut().enumerate() {
                *s = evidence(k, if k == 0 { show } else { Show::Empty });
            }
            let frame = f as u32;
            tracker
                .push(&VoiceFrame {
                    frame,
                    slots: &slots,
                    cut: cuts.contains(&frame),
                })
                .unwrap();
            tracker.drain_ready(&mut out);
            // Frames become final exactly DELAY frames later.
            assert_eq!(out.len(), (f + 1).saturating_sub(DELAY as usize));
        }
        tracker.finish(&mut out);
        assert_eq!(out.len(), shows.len());
        let confidences = out
            .iter()
            .map(|b| b.boxes.first().map(|v| v.confidence))
            .collect();
        (out, confidences)
    }

    const NAME: Show = Show::Name {
        p: 100,
        x1: 120,
        score: 1.5,
    };

    #[test]
    fn single_frames_count_only_when_strong() {
        let mut shows = vec![Show::Empty; 10];
        shows[3] = NAME;
        assert!(run(&shows, &[]).1.iter().all(Option::is_none));
        shows[3] = Show::Name {
            p: 100,
            x1: 120,
            score: 2.5,
        };
        let (_, c) = run(&shows, &[]);
        assert_eq!(c[3], Some(Confidence::Tracked));
        assert_eq!(c[4], Some(Confidence::PostRoll));
        assert_eq!(c.iter().flatten().count(), 2);
    }

    #[test]
    fn tracks_backfill_hold_and_roll_while_the_portrait_shows() {
        // Portrait without a detected name, two name frames, portrait
        // again, then nothing.
        let mut shows = vec![Show::Empty; 30];
        for s in &mut shows[5..10] {
            *s = Show::Portrait { p: 100 };
        }
        shows[10] = NAME;
        shows[11] = NAME;
        for s in &mut shows[12..20] {
            *s = Show::Portrait { p: 100 };
        }
        let (boxes, c) = run(&shows, &[]);
        assert!(c[5..10].iter().all(|&c| c == Some(Confidence::WeakHold)));
        assert_eq!(c[10..12], [Some(Confidence::Tracked); 2]);
        assert!(c[12..20].iter().all(|&c| c == Some(Confidence::WeakHold)));
        assert_eq!(c[20], Some(Confidence::PostRoll));
        assert!(c[..5].iter().chain(&c[21..]).all(Option::is_none));
        // Held frames are escalated to the whole name slot (plan 7.3);
        // tracked and post-roll frames keep the name's width.
        let x1 = |f: usize| boxes[f].boxes[0].rect.x1;
        let slot = entry_detection(slot_top(0), BOX_ROWS, SLOT_X1).rect.x1;
        assert!((5..10).chain(12..20).all(|f| x1(f) == slot));
        assert!(x1(10) < slot && x1(20) == x1(10));
    }

    #[test]
    fn holds_end_after_a_second_or_at_another_portrait() {
        let mut shows = vec![NAME, NAME];
        shows.extend(vec![Show::Portrait { p: 100 }; HOLD as usize + 20]);
        let (_, c) = run(&shows, &[]);
        let held = c
            .iter()
            .filter(|&&c| c == Some(Confidence::WeakHold))
            .count();
        assert_eq!(held, HOLD as usize);
        let mut shows = vec![NAME, NAME];
        shows.extend(vec![Show::Portrait { p: 160 }; 10]);
        let (_, c) = run(&shows, &[]);
        assert_eq!(c[2], Some(Confidence::PostRoll));
        assert!(c[3..].iter().all(Option::is_none));
    }

    #[test]
    fn short_gaps_are_bridged_long_ones_split() {
        for (gap, bridged) in [(GAP as usize, true), (GAP as usize + 1, false)] {
            let mut shows = vec![NAME, NAME];
            shows.extend(vec![Show::Empty; gap]);
            shows.extend([NAME, NAME, Show::Empty]);
            let (out, c) = run(&shows, &[]);
            let tracks: BTreeSet<u32> = out
                .iter()
                .flat_map(|b| b.boxes.iter().map(|v| v.track))
                .collect();
            assert_eq!(tracks.len(), if bridged { 1 } else { 2 }, "gap {gap}");
            let filled = c
                .iter()
                .filter(|&&c| c == Some(Confidence::GapFill))
                .count();
            assert_eq!(filled, if bridged { gap } else { 0 }, "gap {gap}");
        }
    }

    #[test]
    fn neighbouring_entry_frames_stay_one_track_whatever_the_portrait() {
        // The game behind the portrait corners can move the signature
        // from frame to frame, and a speaker change from one frame to the
        // next only widens the box: both stay one track.
        let other = Show::Name {
            p: 200,
            x1: 180,
            score: 1.5,
        };
        let shows = [NAME, NAME, NAME, other, other, other, Show::Empty];
        let (out, c) = run(&shows, &[]);
        assert!(c[..6].iter().all(|&c| c == Some(Confidence::Tracked)));
        assert!(
            out[..6]
                .iter()
                .all(|b| b.boxes[0].track == out[0].boxes[0].track)
        );
        let wide = entry_detection(slot_top(0), BOX_ROWS, 180).rect;
        assert!(out[..6].iter().all(|b| b.boxes[0].rect == wide));
        // After a gap, another portrait starts a new track, and the
        // post-roll of the old one stops at the next entry.
        let shows = [NAME, NAME, Show::Empty, other, other, Show::Empty];
        let (out, c) = run(&shows, &[]);
        assert_eq!(c[2], Some(Confidence::PostRoll));
        assert_ne!(out[3].boxes[0].track, out[1].boxes[0].track);
        let shows = [NAME, NAME, Show::Portrait { p: 200 }, other, other];
        let (_, c) = run(&shows, &[]);
        assert_eq!(c[2], Some(Confidence::PostRoll));
    }

    #[test]
    fn holds_follow_a_slowly_changing_portrait() {
        // Each frame differs from its neighbour by less than the identity
        // limit, the last one from the first by far more.
        let first = Show::Name {
            p: 1,
            x1: 120,
            score: 1.5,
        };
        let mut shows = vec![first, first];
        shows.extend((1..=30).map(|t| Show::Between { t }));
        shows.push(Show::Empty);
        let (_, c) = run(&shows, &[]);
        assert!(c[2..32].iter().all(|&c| c == Some(Confidence::WeakHold)));
        let signature = |show| evidence(0, show).signature;
        assert!(signature(first).distance(&signature(Show::Between { t: 30 })) > 0.5);
    }

    #[test]
    fn boxes_take_the_widest_name_of_the_track() {
        let name = |x1| Show::Name {
            p: 100,
            x1,
            score: 1.5,
        };
        let shows = [name(90), name(130), name(110), Show::Empty];
        let (out, _) = run(&shows, &[]);
        let expected = entry_detection(slot_top(0), BOX_ROWS, 130).rect;
        assert!(out[..3].iter().all(|b| b.boxes[0].rect == expected));
    }

    #[test]
    fn structure_only_tracks_box_the_whole_name_slot() {
        let shows = [
            Show::Structure { p: 90 },
            Show::Structure { p: 92 },
            Show::Empty,
        ];
        let (out, c) = run(&shows, &[]);
        assert_eq!(c[..2], [Some(Confidence::WeakHold); 2]);
        let expected = entry_detection(slot_top(0), BOX_ROWS, SLOT_X1).rect;
        assert_eq!(out[0].boxes[0].rect, expected);
    }

    #[test]
    fn scene_cuts_stop_extensions_but_not_detecting_tracks() {
        // Portrait before the cut at 5, entry after it: no backfill across
        // the cut.
        let mut shows = vec![Show::Portrait { p: 100 }; 5];
        shows.extend([NAME, NAME, NAME]);
        shows.extend(vec![Show::Portrait { p: 100 }; 4]);
        let (_, c) = run(&shows, &[5]);
        assert!(c[..5].iter().all(Option::is_none));
        // Hold after the entry, cut at 10: the hold stops there, no
        // post-roll across it.
        let (_, c) = run(&shows, &[5, 10]);
        assert_eq!(c[8..10], [Some(Confidence::WeakHold); 2]);
        assert!(c[10..].iter().all(Option::is_none));
        // An entry detected on both sides of a cut stays one track.
        let shows = [NAME, NAME, NAME, NAME, Show::Empty];
        let (out, _) = run(&shows, &[2]);
        assert_eq!(out[0].boxes[0].track, out[3].boxes[0].track);
        // A gap that spans a cut is not filled.
        let shows = [NAME, NAME, Show::Empty, Show::Empty, NAME, NAME];
        let (out, c) = run(&shows, &[3]);
        assert_eq!(c[2], Some(Confidence::PostRoll));
        assert_eq!(c[3], None);
        assert_ne!(out[4].boxes[0].track, out[1].boxes[0].track);
    }

    #[test]
    fn frames_must_follow_each_other() {
        let slots = [SlotEvidence::default(); SLOTS];
        let mut tracker = VoiceTracker::new();
        let frame = |frame| VoiceFrame {
            frame,
            slots: &slots,
            cut: false,
        };
        tracker.push(&frame(7)).unwrap();
        assert_eq!(
            tracker.push(&frame(9)),
            Err(VoiceTrackError::FrameOrder {
                expected: 8,
                actual: 9
            })
        );
        let mut out = vec![];
        tracker.finish(&mut out);
        assert_eq!(out.len(), 1);
        // After finish the tracker starts a new video.
        tracker.push(&frame(0)).unwrap();
        assert!(tracker.heap_bytes() > 0);
    }
}
