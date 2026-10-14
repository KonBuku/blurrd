//! The health check of a HUD region (plan 7.3).
//!
//! A region's structure (kill-feed rows, a chat box with text in it, a
//! voice-chat entry's portrait and location line) shows that names are on
//! screen, independently of the detector that reads them. Where the
//! structure stays but the detector reports no name for more than
//! [`HEALTH_FRAMES`] frames, the detector has failed (a palette, a filter
//! or a layout it does not read): [`HealthCheck`] covers that whole span
//! with the structure's slot boxes and reports it, so the engine emits a
//! `RegionHealth` warning and marks the span in the timeline.
//!
//! Shorter spans are left alone: names fade in and out over a few frames,
//! and the temporal models bridge short gaps. The check delays frames by at
//! most [`HEALTH_FRAMES`] + 1, far below the engine's `LOOKAHEAD_MAX` of
//! 240 (plan 5.4).
//!
//! The slot boxes come from the detectors' structure:
//! [`KillFeedDetector::row_slots`](crate::KillFeedDetector::row_slots),
//! [`ChatDetector::band_slots`](crate::ChatDetector::band_slots) and
//! [`SlotEvidence::slot_box`](crate::voice::SlotEvidence::slot_box) of the
//! slots that [show an entry](crate::voice::SlotEvidence::shows_entry).

use std::collections::VecDeque;

use blurrd_core::{FrameRange, RectI, Region};

/// Frames with structure but without a name after which a span counts as
/// failed (plan 7.3: "> 30 frames").
pub const HEALTH_FRAMES: u32 = 30;

/// A span of a region in which the structure stayed but no name was
/// found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HealthSpan {
    /// The region.
    pub region: Region,
    /// The frames of the span.
    pub frames: FrameRange,
}

/// A frame waiting for the decision whether its span failed.
#[derive(Debug, Clone)]
struct Pending {
    frame: u32,
    slots: Vec<RectI>,
}

/// The health check of one region ([module docs](self)).
///
/// Push every frame in order with the slot boxes of the structure the
/// frame shows and whether the detector found a name in it; drain the
/// frames in order with the slot boxes they get (none outside failed
/// spans).
#[derive(Debug, Clone)]
pub struct HealthCheck {
    region: Region,
    /// The frames of the current run of frames with structure and without
    /// names, while it is not longer than [`HEALTH_FRAMES`].
    run: Vec<Pending>,
    /// Whether the current run is longer than [`HEALTH_FRAMES`]: its frames
    /// get their slot boxes as they come.
    failed: bool,
    /// First frame of the current run.
    run_start: u32,
    /// Last frame of the current run.
    run_last: u32,
    /// The next frame expected.
    next: Option<u32>,
    ready: VecDeque<(u32, Vec<RectI>)>,
    spans: Vec<HealthSpan>,
}

impl HealthCheck {
    /// A health check for `region`.
    pub const fn new(region: Region) -> Self {
        Self {
            region,
            run: Vec::new(),
            failed: false,
            run_start: 0,
            run_last: 0,
            next: None,
            ready: VecDeque::new(),
            spans: Vec::new(),
        }
    }

    /// The region checked.
    pub const fn region(&self) -> Region {
        self.region
    }

    /// Pushes frame `frame`: the slot boxes of the structure it shows
    /// (empty without structure) and whether the detector found a name.
    /// A frame that does not follow the previous one ends the run before
    /// it.
    pub fn push(&mut self, frame: u32, slots: &[RectI], names: bool) {
        if self.next.is_some_and(|n| n != frame) {
            self.end_run();
        }
        self.next = Some(frame.saturating_add(1));
        if slots.is_empty() || names {
            self.end_run();
            self.ready.push_back((frame, Vec::new()));
            return;
        }
        self.run_last = frame;
        if self.failed {
            self.ready.push_back((frame, slots.to_vec()));
            return;
        }
        if self.run.is_empty() {
            self.run_start = frame;
        }
        self.run.push(Pending {
            frame,
            slots: slots.to_vec(),
        });
        if self.run.len() as u32 > HEALTH_FRAMES {
            self.failed = true;
            self.ready
                .extend(self.run.drain(..).map(|p| (p.frame, p.slots)));
        }
    }

    /// Moves the decided frames, in order, into `out`: each with the slot
    /// boxes it gets (empty outside failed spans).
    pub fn drain_ready(&mut self, out: &mut Vec<(u32, Vec<RectI>)>) {
        out.extend(self.ready.drain(..));
    }

    /// Ends the video: decides the frames still waiting and moves every
    /// frame into `out`.
    pub fn finish(&mut self, out: &mut Vec<(u32, Vec<RectI>)>) {
        self.end_run();
        self.drain_ready(out);
    }

    /// The failed spans found so far (a span still running is reported
    /// when it ends).
    pub fn spans(&self) -> &[HealthSpan] {
        &self.spans
    }

    /// Ends the current run: a failed run becomes a span, a short one's
    /// frames get no boxes.
    fn end_run(&mut self) {
        if self.failed {
            self.spans.push(HealthSpan {
                region: self.region,
                frames: FrameRange {
                    f0: self.run_start,
                    f1: self.run_last.max(self.run_start),
                },
            });
            self.failed = false;
        }
        self.ready
            .extend(self.run.drain(..).map(|p| (p.frame, Vec::new())));
    }
}

#[cfg(test)]
mod tests {
    use blurrd_core::{FrameRange, RectI, Region};

    use super::{HEALTH_FRAMES, HealthCheck};

    const SLOT: RectI = RectI {
        x0: 10,
        y0: 20,
        x1: 200,
        y1: 40,
    };

    /// Runs `frames` frames; frames in `missing` show the structure without
    /// a name, the others with one; frames in `bare` show no structure.
    fn run(frames: u32, missing: std::ops::Range<u32>) -> (HealthCheck, Vec<(u32, Vec<RectI>)>) {
        let mut check = HealthCheck::new(Region::KillFeed);
        let mut out = Vec::new();
        for f in 0..frames {
            check.push(f, &[SLOT], !missing.contains(&f));
            check.drain_ready(&mut out);
        }
        check.finish(&mut out);
        (check, out)
    }

    #[test]
    fn a_short_gap_is_left_alone() {
        let (check, out) = run(100, 20..20 + HEALTH_FRAMES);
        assert_eq!(check.spans(), &[]);
        assert_eq!(out.len(), 100);
        assert!(
            out.iter()
                .enumerate()
                .all(|(i, (f, b))| *f == i as u32 && b.is_empty())
        );
    }

    #[test]
    fn a_long_gap_gets_slot_boxes_and_is_reported() {
        let (check, out) = run(100, 20..61);
        assert_eq!(check.spans().len(), 1);
        assert_eq!(check.spans()[0].frames, FrameRange { f0: 20, f1: 60 });
        assert_eq!(out.len(), 100);
        for (f, boxes) in &out {
            assert_eq!(!boxes.is_empty(), (20..61).contains(f), "frame {f}");
        }
    }

    #[test]
    fn frames_are_released_in_order_within_the_delay() {
        let mut check = HealthCheck::new(Region::TextChat);
        let mut out = Vec::new();
        let mut released = 0u32;
        for f in 0..200u32 {
            check.push(f, &[SLOT], false);
            check.drain_ready(&mut out);
            for (k, (g, _)) in out.iter().enumerate() {
                assert_eq!(*g, released + k as u32);
            }
            released += out.len() as u32;
            out.clear();
            assert!(f + 1 - released <= HEALTH_FRAMES + 1, "frame {f}");
        }
        check.finish(&mut out);
        assert_eq!(released + out.len() as u32, 200);
        assert_eq!(check.spans()[0].frames, FrameRange { f0: 0, f1: 199 });
    }

    #[test]
    fn frames_without_structure_or_a_jump_end_a_run() {
        let mut check = HealthCheck::new(Region::VoiceChat);
        let mut out = Vec::new();
        for f in 0..20 {
            check.push(f, &[SLOT], false);
        }
        check.push(20, &[], false);
        for f in 21..41 {
            check.push(f, &[SLOT], false);
        }
        // A jump in the frame numbers ends the run too.
        for f in 50..70 {
            check.push(f, &[SLOT], false);
        }
        check.finish(&mut out);
        assert_eq!(check.spans(), &[]);
        assert!(out.iter().all(|(_, b)| b.is_empty()));
    }
}
