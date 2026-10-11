//! From per-frame reads to blur boxes: runs with bridging, pre-roll and
//! post-roll, decided with a bounded lookahead.
//!
//! A frame is *confirmed* when its native read found the surface's
//! structure ([`SurfaceFrame::slots`] is `Some`). Confirmed frames with
//! gaps of at most [`BRIDGE`] frames between them form a *run*, which
//! counts only with at least [`MIN_CONFIRMED`] confirmed frames. A
//! frame is *held* when the surface's gate made it a candidate: the panel
//! may still be on screen while only its native read fails (a filter or
//! an effect over the panel). A longer gap of at most [`HOLD`] frames
//! joins the run too if every frame of it is held. In the same way, up
//! to [`LEAD`] held frames right before a run's first confirmed frame (a
//! panel that opened while its native read still failed) and up to
//! [`TAIL`] held frames right after its last confirmed frame (a panel
//! still open while its native read failed until it closed) join the
//! run; a frame of the tail must also keep the gate's score at
//! [`TAIL_LEVEL`] of the last confirmed frame's score, so that a gate
//! that a closed panel leaves to the background's edges does not extend
//! the run.
//! The surface is on from [`PRE_ROLL`] frames before a run's first frame
//! to [`POST_ROLL`] frames after its last frame. The pre-roll covers the
//! first frames of a fade-in, while the panel is still too faint for its
//! structure to be found; the post-roll covers the end of a fade-out and
//! the encoder's ghosting of the last frame.
//!
//! While a surface is on, every name slot that showed ink in at least
//! [`MIN_CONFIRMED`] confirmed frames of the run (seen so far within the
//! lookahead) gets its slot box in every frame of the run: a name too
//! faint to show ink while fading is still covered, an empty row (fewer
//! players) stays unblurred, and a slot found in a single read (a
//! marginal structure, such as a killer's strip read into gameplay above
//! a combat report) blurs nothing. Slots are followed from frame to frame
//! by position (the same kind, overlapping columns, centres at most
//! [`MATCH_DY`] apart),
//! so a reader may miss a faint row or card without confusing the
//! others. Positions are trusted only from reads in which the slot shows
//! ink: a frame whose own read has the slot with ink uses that position;
//! any other frame (pre-roll, post-roll, a bridged gap, a missed slot, a
//! read too faint to show the name) takes the slot's nearest position
//! read with ink, grown vertically by [`UNREAD_MARGIN`] because a fading
//! panel may also be sliding.
//!
//! Decisions wait for at most [`LOOKAHEAD`] frames, well inside the
//! engine's `LOOKAHEAD_MAX` of 240.

use std::collections::VecDeque;

use blurrd_core::{NameKind, RectI, Region};

use crate::detection::Detection;

/// Fewest confirmed frames a run needs to get boxes, and fewest reads in
/// which a slot must show ink to get its box: a structure found in one
/// frame only is a coincidence of gameplay (measured on real clips: such
/// single frames, never a real surface, which stays open for tens of
/// frames). Two frames keep a surface flashed open for 1/30 s covered.
pub const MIN_CONFIRMED: u32 = 2;
/// Most frames a decision waits for.
pub const LOOKAHEAD: u32 = 60;
/// Frames before a run's first confirmed frame that get boxes. Measured
/// on real recordings, the scoreboard and the death recap appear within
/// one frame; the pre-roll covers fades of up to four frames whose first
/// frames show no structure (the synthetic fixtures fade over four).
pub const PRE_ROLL: u32 = 4;
/// Frames after a run's last confirmed frame that get boxes: the measured
/// fade-out ends within one frame of the structure, one more frame covers
/// the encoder's ghosting of the last frame.
pub const POST_ROLL: u32 = 2;
/// Gaps of up to this many frames between confirmed frames are bridged
/// (a flash or an effect over the panel). At least `PRE_ROLL +
/// POST_ROLL`, so that two runs never claim the same frame.
pub const BRIDGE: u32 = PRE_ROLL + POST_ROLL;
/// Longest gap between confirmed frames that is bridged while the gate
/// holds every frame of it (measured: reads failed for up to 9
/// consecutive frames of an open scoreboard in a greyscale 720p copy of
/// a real recording and of an open recap under a vibrance filter). Well
/// below [`LOOKAHEAD`], so that a held run is always closed or extended
/// before a frame of its gap is decided.
pub const HOLD: u32 = 30;
/// Most frames right before a run's first confirmed frame that join the
/// run while the gate holds each of them (measured: the native read of a
/// death recap failed for the first 5 frames after it opened in a native
/// 4:3 copy of a real recording). Kept short because the gate also fires
/// on menus that can precede a surface. `LEAD + PRE_ROLL` is the longest
/// a frame without a run waits for its decision.
pub const LEAD: u32 = 12;
/// Most frames right after a run's last confirmed frame that join the
/// run while the gate holds each of them (measured: the native read of a
/// death recap failed for its last 7 frames before it closed in a
/// sharpened copy of a real recording). As short as [`LEAD`] because the
/// gate also holds gameplay that resembles a panel (stair lines under the
/// scoreboard gate at 720p): with a tail of 30 frames, the scoreboard
/// boxes of such marginal runs in 720p copies of 31 real recordings grew
/// from 364 to 677 frames. Frames of the tail wait at most `HOLD + 1`
/// frames for their decision, within [`LOOKAHEAD`].
pub const TAIL: u32 = LEAD;
/// Share of the last confirmed frame's gate score a frame of the tail
/// must keep (measured: the tail frames of the sharpened recap scored
/// 0.56 to 0.74 after a last confirmed frame at 0.64; the gate of a
/// synthetic death recap at 720p dropped from 0.98 to the 0.68 of its
/// background's edges when the panel closed).
pub const TAIL_LEVEL: f32 = 0.75;
/// Vertical growth, in reference pixels, of the boxes of frames without
/// their own read (at least the largest slide the surfaces allow).
pub const UNREAD_MARGIN: i32 = 20;
/// Largest vertical distance, in reference pixels, between the centres of
/// one slot in two frames: less than half the smallest slot pitch (the
/// scoreboard's 34 px rows).
pub const MATCH_DY: i32 = 12;

/// One name slot of a surface in one frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Slot {
    /// What the name is.
    pub kind: NameKind,
    /// The slot box in reference pixels at this frame's panel position.
    pub rect: RectI,
    /// Whether the slot shows ink (a name) in this frame.
    pub ink: bool,
}

/// What one frame showed of one surface: its name slots if the native
/// name zone was read and the panel's structure was found there, `None`
/// otherwise (no candidate, or nothing found).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SurfaceFrame {
    /// Whether the surface's gate made the frame a candidate.
    pub candidate: bool,
    /// The gate's score of the frame
    /// ([`GateSignal::score`](super::GateSignal::score)).
    pub score: f32,
    /// The name slots of a confirmed frame.
    pub slots: Option<Vec<Slot>>,
}

#[derive(Debug, Clone)]
struct Run {
    /// The first frame of the run: its first confirmed frame, or the
    /// first frame of the gate-held lead before it.
    start: u32,
    /// The run's last confirmed frame.
    last: u32,
    /// The last frame of the gate-held tail after `last` (`last` if
    /// there is none).
    tail: u32,
    /// The gate's score of the last confirmed frame.
    level: f32,
    closed: bool,
    /// Whether the gate held every frame since `last`.
    held: bool,
    /// Confirmed frames in the run.
    confirmed: u32,
    /// The slots that showed ink, each at its last known position, with
    /// the number of reads in which it showed ink.
    ink: Vec<(Slot, u32)>,
}

impl Slot {
    /// Whether `other` is the same slot in another frame.
    fn matches(&self, other: &Self) -> bool {
        let (a, b) = (self.rect, other.rect);
        self.kind == other.kind
            && a.x0 < b.x1
            && b.x0 < a.x1
            && ((a.y0 + a.y1) - (b.y0 + b.y1)).abs() <= 2 * MATCH_DY
    }
}

impl Run {
    /// The last frame of a closed run's post-roll.
    const fn end(&self) -> u32 {
        self.tail + POST_ROLL
    }

    /// Whether frame `f` gets boxes from this run (as far as it is known).
    fn covers(&self, f: u32) -> bool {
        f + PRE_ROLL >= self.start && (!self.closed || f <= self.end())
    }
}

/// The temporal stage of one surface.
#[derive(Debug, Clone)]
pub struct SurfaceTrack {
    region: Region,
    /// Frames not yet decided, with their reads.
    pending: VecDeque<(u32, Option<Vec<Slot>>)>,
    /// Runs that may still cover pending frames, oldest first.
    runs: VecDeque<Run>,
    /// The newest frame pushed.
    newest: Option<u32>,
    /// The first frame of the current streak of held frames (up to the
    /// newest frame), if the newest frame is held.
    held_since: Option<u32>,
}

impl SurfaceTrack {
    /// An empty track for `region`.
    pub fn new(region: Region) -> Self {
        Self {
            region,
            pending: VecDeque::new(),
            runs: VecDeque::new(),
            newest: None,
            held_since: None,
        }
    }

    /// Adds the next frame (frames must come in order without gaps) and
    /// appends every frame that can now be decided to `out` as
    /// `(frame, boxes)`.
    pub fn push(&mut self, frame: u32, data: SurfaceFrame, out: &mut Vec<(u32, Vec<Detection>)>) {
        self.newest = Some(frame);
        let held = data.candidate || data.slots.is_some();
        if held {
            self.held_since.get_or_insert(frame);
        } else {
            self.held_since = None;
        }
        if let Some(run) = self.runs.back_mut()
            && !run.closed
        {
            run.held &= held;
            if run.held
                && run.tail + 1 == frame
                && frame <= run.last + TAIL
                && data.score >= TAIL_LEVEL * run.level
            {
                run.tail = frame;
            }
            let gap = frame - run.last - 1;
            if gap > BRIDGE && (!run.held || gap > HOLD) {
                run.closed = true;
            }
        }
        if let Some(slots) = &data.slots {
            let joins = self.runs.back().is_some_and(|r| !r.closed);
            if !joins {
                // The gate-held lead, but no frame a previous run covers.
                let mut start = self
                    .held_since
                    .unwrap_or(frame)
                    .max(frame.saturating_sub(LEAD));
                if let Some(previous) = self.runs.back() {
                    // A run whose tail reached this frame overlaps it.
                    start = start.max(previous.end() + 1).min(frame);
                }
                self.runs.push_back(Run {
                    start,
                    last: frame,
                    tail: frame,
                    level: data.score,
                    closed: false,
                    held: true,
                    confirmed: 0,
                    ink: Vec::new(),
                });
            }
            if let Some(run) = self.runs.back_mut() {
                run.last = frame;
                run.tail = frame;
                run.level = data.score;
                run.held = true;
                run.confirmed += 1;
                for s in slots.iter().filter(|s| s.ink) {
                    match run.ink.iter_mut().find(|(k, _)| k.matches(s)) {
                        Some((known, seen)) => {
                            known.rect = s.rect;
                            *seen += 1;
                        }
                        None => run.ink.push((*s, 1)),
                    }
                }
            }
        }
        self.pending.push_back((frame, data.slots));
        self.decide(out, false);
    }

    /// Decides every pending frame (end of the video).
    pub fn finish(mut self, out: &mut Vec<(u32, Vec<Detection>)>) {
        for run in &mut self.runs {
            run.closed = true;
        }
        self.decide(out, true);
    }

    /// Emits the pending frames, oldest first, as long as each can be
    /// decided.
    fn decide(&mut self, out: &mut Vec<(u32, Vec<Detection>)>, all: bool) {
        let Some(newest) = self.newest else {
            return;
        };
        while let Some(&(f, _)) = self.pending.front() {
            let run = self.runs.iter().position(|r| r.covers(f));
            let ready = all
                || match run {
                    // On: wait for the run's end or the lookahead.
                    Some(i) => self.runs[i].closed || newest >= f + LOOKAHEAD,
                    // Off: unless a run starts within the pre-roll, or a
                    // run that starts later can still reach back to `f`
                    // through a gate-held lead.
                    None => {
                        newest >= f + PRE_ROLL
                            && (newest >= f + LEAD + PRE_ROLL
                                || self.held_since.is_none_or(|h| f + PRE_ROLL < h))
                    }
                };
            if !ready {
                break;
            }
            let boxes = match run {
                Some(i) if self.runs[i].confirmed >= MIN_CONFIRMED => self.boxes(f, &self.runs[i]),
                _ => Vec::new(),
            };
            self.pending.pop_front();
            out.push((f, boxes));
            // Runs that end before the next pending frame are done.
            while self.runs.front().is_some_and(|r| r.closed && r.end() <= f) {
                self.runs.pop_front();
            }
        }
    }

    /// The boxes of pending frame `f` in `run`.
    fn boxes(&self, f: u32, run: &Run) -> Vec<Detection> {
        let own = self
            .pending
            .iter()
            .find(|(g, _)| *g == f)
            .and_then(|(_, s)| s.as_ref());
        run.ink
            .iter()
            .filter(|(_, seen)| *seen >= MIN_CONFIRMED)
            .map(|(known, _)| {
                let read = own.and_then(|s| s.iter().find(|s| s.ink && s.matches(known)));
                let (rect, margin) = match read {
                    Some(slot) => (slot.rect, 0),
                    None => (self.nearest(f, run, known), UNREAD_MARGIN),
                };
                let rect = RectI {
                    y0: rect.y0 - margin,
                    y1: rect.y1 + margin,
                    ..rect
                };
                Detection {
                    region: self.region,
                    kind: known.kind,
                    text: rect,
                    rect,
                    row_top: None,
                }
            })
            .collect()
    }

    /// The position of `known` in the pending read of `run` nearest to
    /// frame `f`, or its last known position.
    fn nearest(&self, f: u32, run: &Run, known: &Slot) -> RectI {
        self.pending
            .iter()
            .filter(|(g, _)| run.covers(*g))
            .filter_map(|(g, s)| {
                let slot = s.as_ref()?.iter().find(|s| s.ink && s.matches(known))?;
                Some((g.abs_diff(f), slot.rect))
            })
            .min_by_key(|(d, _)| *d)
            .map_or(known.rect, |(_, r)| r)
    }
}

#[cfg(test)]
mod tests {
    use blurrd_core::{NameKind, RectI, Region};

    use super::{
        BRIDGE, HOLD, LEAD, LOOKAHEAD, POST_ROLL, PRE_ROLL, Slot, SurfaceFrame, SurfaceTrack, TAIL,
        TAIL_LEVEL, UNREAD_MARGIN,
    };

    fn frame(read: bool, ink: bool, y: i32) -> SurfaceFrame {
        SurfaceFrame {
            candidate: read,
            score: 0.0,
            slots: read.then(|| {
                vec![Slot {
                    kind: NameKind::ScoreboardRow,
                    rect: RectI::new(10, y, 50, y + 20).unwrap(),
                    ink,
                }]
            }),
        }
    }

    /// Runs a track over `(read, ink)` frames; returns `(frame, boxes,
    /// top of the first box)`.
    fn run(frames: &[(bool, bool)]) -> Vec<(u32, usize, i32)> {
        let mut track = SurfaceTrack::new(Region::Scoreboard);
        let mut out = vec![];
        for (i, &(read, ink)) in (0u32..).zip(frames) {
            track.push(i, frame(read, ink, 100 + i as i32), &mut out);
        }
        track.finish(&mut out);
        assert_eq!(out.len(), frames.len());
        out.iter()
            .map(|(f, b)| (*f, b.len(), b.first().map_or(-1, |d| d.rect.y0)))
            .collect()
    }

    #[test]
    fn covers_pre_roll_run_and_post_roll_with_union_of_ink() {
        let mut frames = vec![(false, false); 20];
        // Read at 8..=11; ink only at 10 and 11 (a name too faint before).
        for f in &mut frames[8..12] {
            *f = (true, false);
        }
        frames[10].1 = true;
        frames[11].1 = true;
        let out = run(&frames);
        let boxed: Vec<u32> = out.iter().filter(|o| o.1 > 0).map(|o| o.0).collect();
        let first = 8 - PRE_ROLL;
        let last = 11 + POST_ROLL;
        assert_eq!(boxed, (first..=last).collect::<Vec<_>>());
        // Only the reads with ink give a position: frames 10 and 11 use
        // their own, every other frame the nearest one grown by the margin.
        assert_eq!(out[10].2, 110);
        assert_eq!(out[11].2, 111);
        for f in [first, 9] {
            assert_eq!(out[f as usize].2, 110 - UNREAD_MARGIN);
        }
        assert_eq!(out[last as usize].2, 111 - UNREAD_MARGIN);
    }

    #[test]
    fn covers_the_gate_held_lead_before_the_first_read() {
        let roll = |c: &str, n: u32| c.repeat(n as usize);
        // Five held frames before the first confirmed one, after gameplay.
        let p = format!("{}{}CCCC{}", roll(".", 20), roll("h", 5), roll(".", 10));
        let boxes = held_run(&p);
        let first = 20 - PRE_ROLL as usize;
        assert!(
            boxes[first..29 + POST_ROLL as usize]
                .iter()
                .all(|&b| b == 1)
        );
        assert_eq!(boxes.iter().sum::<usize>(), 29 + POST_ROLL as usize - first);
        // A longer held streak reaches back `LEAD` frames only.
        let p = format!("{}{}CCCC{}", roll(".", 10), roll("h", 40), roll(".", 10));
        let boxes = held_run(&p);
        let start = 50 - (LEAD + PRE_ROLL) as usize;
        assert_eq!(boxes[start - 1], 0);
        assert!(boxes[start..54].iter().all(|&b| b == 1));
        // A frame the gate did not hold ends the lead.
        let p = format!(
            "{}{}.{}CCCC{}",
            roll(".", 20),
            roll("h", 6),
            roll("h", 3),
            roll(".", 10)
        );
        let boxes = held_run(&p);
        let start = 27 - PRE_ROLL as usize;
        assert_eq!(boxes[start - 1], 0);
        assert_eq!(boxes[start], 1);
        // Held frames that no confirmed frame follows get no box.
        let p = format!("{}{}", roll("h", 20), roll(".", 10));
        assert_eq!(held_run(&p).iter().sum::<usize>(), 0);
    }

    #[test]
    fn a_slot_with_ink_in_a_single_read_gets_no_box() {
        let slot = |y: i32, ink: bool| Slot {
            kind: NameKind::RecapName,
            rect: RectI::new(10, y, 50, y + 20).unwrap(),
            ink,
        };
        let mut track = SurfaceTrack::new(Region::DeathRecap);
        let mut out = vec![];
        for i in 0..10 {
            // A card read in every frame; a strip above it in frame 4 only.
            let mut slots = vec![slot(200, true)];
            if i == 4 {
                slots.push(slot(100, true));
            }
            let data = SurfaceFrame {
                candidate: true,
                score: 0.0,
                slots: Some(slots),
            };
            track.push(i, data, &mut out);
        }
        track.finish(&mut out);
        assert_eq!(out.len(), 10);
        assert!(out.iter().all(|(_, b)| b.len() == 1 && b[0].rect.y0 == 200));
    }

    #[test]
    fn a_single_confirmed_frame_gives_no_box() {
        let mut frames = vec![(false, false); 12];
        frames[6] = (true, true);
        assert!(run(&frames).iter().all(|o| o.1 == 0));
        frames[7] = (true, true);
        assert!(run(&frames).iter().any(|o| o.1 == 1));
    }

    #[test]
    fn reads_without_ink_give_no_box() {
        let out = run(&[(true, false); 8]);
        assert!(out.iter().all(|o| o.1 == 0));
    }

    #[test]
    fn bridges_short_gaps_and_bounds_the_delay() {
        let mut frames = vec![(true, true); 5];
        frames.extend(std::iter::repeat_n((false, false), BRIDGE as usize));
        frames.extend(vec![(true, true); 5]);
        let out = run(&frames);
        assert!(out.iter().all(|o| o.1 == 1), "{out:?}");

        let mut track = SurfaceTrack::new(Region::Scoreboard);
        let mut decided = vec![];
        for i in 0..200 {
            track.push(i, frame(true, true, 0), &mut decided);
            assert!(i - decided.len() as u32 <= LOOKAHEAD);
        }
        let mut track = SurfaceTrack::new(Region::Scoreboard);
        let mut decided = vec![];
        for i in 0..50 {
            track.push(i, frame(false, false, 0), &mut decided);
            assert!(i - decided.len() as u32 <= PRE_ROLL);
        }
        // Frames the gate holds without a read may still join a run's lead.
        let mut track = SurfaceTrack::new(Region::Scoreboard);
        let mut decided = vec![];
        let held = SurfaceFrame {
            candidate: true,
            score: 0.0,
            slots: None,
        };
        for i in 0..50 {
            track.push(i, held.clone(), &mut decided);
            assert!(i - decided.len() as u32 <= LEAD + PRE_ROLL);
        }
    }

    #[test]
    fn separate_runs_keep_their_own_names() {
        let mut frames = vec![(true, true); 3];
        frames.extend(std::iter::repeat_n((false, false), 20));
        frames.extend(vec![(true, false); 3]);
        let out = run(&frames);
        // The second run shows no ink, so it gets no boxes even though the
        // first one did.
        assert!(out[23..].iter().all(|o| o.1 == 0), "{out:?}");
        assert!(out[..3].iter().all(|o| o.1 == 1));
    }

    /// Boxes per frame of a track over frames that are confirmed (`C`),
    /// held by the gate without a read (`h`) or neither (`.`).
    fn held_run(pattern: &str) -> Vec<usize> {
        let mut track = SurfaceTrack::new(Region::Scoreboard);
        let mut out = vec![];
        for (i, c) in (0u32..).zip(pattern.chars()) {
            let data = match c {
                'C' => frame(true, true, 100),
                'h' => SurfaceFrame {
                    candidate: true,
                    score: 0.0,
                    slots: None,
                },
                _ => frame(false, false, 100),
            };
            track.push(i, data, &mut out);
        }
        track.finish(&mut out);
        assert_eq!(out.len(), pattern.len());
        out.iter().map(|(_, b)| b.len()).collect()
    }

    #[test]
    fn bridges_gaps_the_gate_holds_until_a_confirmed_frame() {
        let gap = |c: &str, n: u32| c.repeat(n as usize);
        // A held gap longer than the bridge, ended by a confirmed frame.
        let p = format!("CCC{}CCC", gap("h", HOLD));
        assert!(held_run(&p).iter().all(|&b| b == 1));
        // A held gap longer than `HOLD` is not bridged: only the first
        // run's tail and post-roll and the second run's lead and pre-roll
        // reach into it.
        let extra = 5;
        let n = TAIL + POST_ROLL + LEAD + PRE_ROLL + extra;
        let p = format!("CCC{}CCC", gap("h", n));
        assert_eq!(
            held_run(&p).iter().filter(|&&b| b == 0).count(),
            extra as usize
        );
    }

    #[test]
    fn covers_the_gate_held_tail_after_the_last_read() {
        let roll = |c: &str, n: u32| c.repeat(n as usize);
        // Reads fail for the last 7 frames before the panel closes.
        let p = format!("{}CCCC{}{}", roll(".", 10), roll("h", 7), roll(".", 20));
        let boxes = held_run(&p);
        let first = 10 - PRE_ROLL as usize;
        let last = 20 + POST_ROLL as usize;
        assert!(boxes[first..=last].iter().all(|&b| b == 1), "{boxes:?}");
        assert_eq!(boxes.iter().sum::<usize>(), last + 1 - first);
        // A frame the gate did not hold ends the tail.
        let p = format!("CCC{}.{}", roll("h", 4), roll(".", 20));
        let boxes = held_run(&p);
        assert_eq!(boxes.iter().sum::<usize>(), 7 + POST_ROLL as usize);
        // A longer held streak is covered for `TAIL` frames only.
        let p = format!("CCC{}{}", roll("h", 2 * HOLD), roll(".", 10));
        let boxes = held_run(&p);
        assert_eq!(boxes.iter().sum::<usize>(), (3 + TAIL + POST_ROLL) as usize);
        // A gate whose score drops well below that of the last confirmed
        // frame (the background's edges after the panel closed) gives no
        // tail.
        let mut track = SurfaceTrack::new(Region::Scoreboard);
        let mut out = vec![];
        for i in 0..20 {
            let score = match i {
                0..3 => 1.0,
                3..6 => TAIL_LEVEL - 0.01,
                _ => 0.0,
            };
            let data = SurfaceFrame {
                candidate: i < 6,
                score,
                slots: frame(i < 3, true, 100).slots,
            };
            track.push(i, data, &mut out);
        }
        track.finish(&mut out);
        let lit = out.iter().filter(|(_, b)| !b.is_empty()).count();
        assert_eq!(lit, (3 + POST_ROLL) as usize);
        // The decision delay stays within the lookahead.
        let mut track = SurfaceTrack::new(Region::Scoreboard);
        let mut decided = vec![];
        let held = SurfaceFrame {
            candidate: true,
            score: 0.0,
            slots: None,
        };
        for i in 0..200 {
            let data = if i < 3 {
                frame(true, true, 100)
            } else {
                held.clone()
            };
            track.push(i, data, &mut decided);
            assert!(i + 1 - decided.len() as u32 <= LOOKAHEAD);
        }
    }
}
