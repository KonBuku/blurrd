//! From per-frame reads to blur boxes: runs with bridging, pre-roll and
//! post-roll, decided with a bounded lookahead.
//!
//! A frame is *confirmed* when its native read found the surface's
//! structure ([`SurfaceFrame::slots`] is `Some`). Confirmed frames with
//! gaps of at most [`BRIDGE`] frames between them form a *run*, which
//! counts only with at least [`MIN_CONFIRMED`] confirmed frames; the
//! surface is on from [`PRE_ROLL`] frames before a run's first confirmed
//! frame to [`POST_ROLL`] frames after its last one. The pre-roll covers
//! the first frames of a fade-in, while the panel is still too faint for
//! its structure to be found; the post-roll covers the end of a fade-out
//! and the encoder's ghosting of the last frame.
//!
//! While a surface is on, every name slot that showed ink in any
//! confirmed frame of the run (seen so far within the lookahead) gets its
//! slot box in every frame of the run: a name too faint to show ink
//! while fading is still covered, and an empty row (fewer players) stays
//! unblurred. Slots are followed from frame to frame by position (the
//! same kind, overlapping columns, centres at most [`MATCH_DY`] apart),
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

/// Fewest confirmed frames a run needs to get boxes: a structure found in
/// one frame only is a coincidence of gameplay (measured on real clips:
/// such single frames, never a real surface, which stays open for tens
/// of frames). Two frames keep a surface flashed open for 1/30 s
/// covered.
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
    /// The name slots of a confirmed frame.
    pub slots: Option<Vec<Slot>>,
}

#[derive(Debug, Clone)]
struct Run {
    first: u32,
    last: u32,
    closed: bool,
    /// Confirmed frames in the run.
    confirmed: u32,
    /// The slots that showed ink, each at its last known position.
    ink: Vec<Slot>,
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
    /// Whether frame `f` gets boxes from this run (as far as it is known).
    fn covers(&self, f: u32) -> bool {
        f + PRE_ROLL >= self.first && (!self.closed || f <= self.last + POST_ROLL)
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
}

impl SurfaceTrack {
    /// An empty track for `region`.
    pub fn new(region: Region) -> Self {
        Self {
            region,
            pending: VecDeque::new(),
            runs: VecDeque::new(),
            newest: None,
        }
    }

    /// Adds the next frame (frames must come in order without gaps) and
    /// appends every frame that can now be decided to `out` as
    /// `(frame, boxes)`.
    pub fn push(&mut self, frame: u32, data: SurfaceFrame, out: &mut Vec<(u32, Vec<Detection>)>) {
        self.newest = Some(frame);
        if let Some(run) = self.runs.back_mut()
            && !run.closed
            && frame > run.last + BRIDGE + 1
        {
            run.closed = true;
        }
        if let Some(slots) = &data.slots {
            let joins = self.runs.back().is_some_and(|r| !r.closed);
            if !joins {
                self.runs.push_back(Run {
                    first: frame,
                    last: frame,
                    closed: false,
                    confirmed: 0,
                    ink: Vec::new(),
                });
            }
            if let Some(run) = self.runs.back_mut() {
                run.last = frame;
                run.confirmed += 1;
                for s in slots {
                    match run.ink.iter_mut().find(|k| k.matches(s)) {
                        Some(known) if s.ink => known.rect = s.rect,
                        None if s.ink => run.ink.push(*s),
                        _ => {}
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
                    // Off: unless a run starts within the pre-roll.
                    None => newest >= f + PRE_ROLL,
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
            while self
                .runs
                .front()
                .is_some_and(|r| r.closed && r.last + POST_ROLL <= f)
            {
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
            .map(|known| {
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
        BRIDGE, LOOKAHEAD, POST_ROLL, PRE_ROLL, Slot, SurfaceFrame, SurfaceTrack, UNREAD_MARGIN,
    };

    fn frame(read: bool, ink: bool, y: i32) -> SurfaceFrame {
        SurfaceFrame {
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
        // Read at 8..=11; ink only at 10 (a name too faint before).
        for f in &mut frames[8..12] {
            *f = (true, false);
        }
        frames[10].1 = true;
        let out = run(&frames);
        let boxed: Vec<u32> = out.iter().filter(|o| o.1 > 0).map(|o| o.0).collect();
        let first = 8 - PRE_ROLL;
        let last = 11 + POST_ROLL;
        assert_eq!(boxed, (first..=last).collect::<Vec<_>>());
        // Only the read with ink gives a position: frame 10 uses its own,
        // every other frame that one grown by the margin.
        assert_eq!(out[10].2, 110);
        for f in [first, 9, 11, last] {
            assert_eq!(out[f as usize].2, 110 - UNREAD_MARGIN);
        }
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
}
