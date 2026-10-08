//! Splitting a render into keyframe-aligned segments for parallel
//! encoder sessions.
//!
//! With two NVENC sessions, each session decodes and encodes whole
//! segments that start at a source keyframe, so every segment starts with
//! an IDR frame and the sessions' packets can be stitched in frame order.
//! Segments are handed out in order, one at a time, to whichever session
//! is free; their length shrinks towards the end of the video, so both
//! sessions finish at about the same time, and is capped, so the packets a
//! session encodes ahead of the other stay within a few megabytes.

/// Frames a segment has at least (unless the video ends first).
const MIN_SEGMENT: u32 = 120;
/// Frames a segment has at most, unless the next keyframe is further away.
const MAX_SEGMENT: u32 = 600;

/// A half-open range of presentation frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Segment {
    /// First frame.
    pub(crate) start: u32,
    /// One past the last frame.
    pub(crate) end: u32,
}

/// Hands out the segments of a video in order.
#[derive(Debug)]
pub(crate) struct Scheduler {
    /// Possible segment starts after frame 0 (source keyframes), sorted.
    boundaries: Vec<u32>,
    count: u32,
    cursor: u32,
    sessions: u32,
}

impl Scheduler {
    /// A scheduler for a video of `count` frames with the given source
    /// keyframes, encoded by `sessions` sessions. One session gets the whole
    /// video as one segment.
    pub(crate) fn new(count: u32, keyframes: &[u32], sessions: u32) -> Self {
        let mut boundaries: Vec<u32> = keyframes
            .iter()
            .copied()
            .filter(|&k| k > 0 && k < count)
            .collect();
        boundaries.sort_unstable();
        boundaries.dedup();
        Self {
            boundaries,
            count,
            cursor: 0,
            sessions: sessions.max(1),
        }
    }

    /// Whether a video of `count` frames is long enough to be worth
    /// splitting between two sessions.
    pub(crate) fn worth_splitting(count: u32) -> bool {
        count >= 2 * MIN_SEGMENT
    }

    /// The next segment, or `None` when the whole video is handed out.
    pub(crate) fn next_segment(&mut self) -> Option<Segment> {
        if self.cursor >= self.count {
            return None;
        }
        let start = self.cursor;
        let end = if self.sessions == 1 {
            self.count
        } else {
            let remaining = self.count - start;
            let target = (remaining / (2 * self.sessions)).clamp(MIN_SEGMENT, MAX_SEGMENT);
            let first_allowed = start.saturating_add(target);
            let end = self
                .boundaries
                .iter()
                .copied()
                .find(|&k| k >= first_allowed)
                .unwrap_or(self.count);
            // Do not leave a stub at the end of the video.
            if self.count - end < MIN_SEGMENT / 2 {
                self.count
            } else {
                end
            }
        };
        self.cursor = end;
        Some(Segment { start, end })
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_SEGMENT, MIN_SEGMENT, Scheduler, Segment};

    fn all(mut scheduler: Scheduler) -> Vec<Segment> {
        std::iter::from_fn(|| scheduler.next_segment()).collect()
    }

    #[test]
    fn one_session_gets_the_whole_video() {
        let keys: Vec<u32> = (0..3601).step_by(60).collect();
        assert_eq!(
            all(Scheduler::new(3601, &keys, 1)),
            [Segment {
                start: 0,
                end: 3601
            }]
        );
    }

    #[test]
    fn segments_start_at_keyframes_and_cover_the_video() {
        for (count, step) in [(3601, 60), (36_000, 60), (500, 7), (2000, 250), (900, 1000)] {
            let keys: Vec<u32> = (0..count).step_by(step).collect();
            let segments = all(Scheduler::new(count, &keys, 2));
            assert_eq!(segments[0].start, 0);
            assert_eq!(segments.last().unwrap().end, count);
            for pair in segments.windows(2) {
                assert_eq!(pair[0].end, pair[1].start, "contiguous");
                assert!(keys.contains(&pair[1].start), "starts at a keyframe");
            }
            for s in &segments[..segments.len() - 1] {
                let len = s.end - s.start;
                assert!(len >= MIN_SEGMENT, "{count}/{step}: {s:?} too short");
                assert!(
                    len <= MAX_SEGMENT + step as u32,
                    "{count}/{step}: {s:?} too long"
                );
            }
        }
    }

    #[test]
    fn segments_shrink_towards_the_end() {
        let keys: Vec<u32> = (0..3601).step_by(60).collect();
        let lengths: Vec<u32> = all(Scheduler::new(3601, &keys, 2))
            .iter()
            .map(|s| s.end - s.start)
            .collect();
        assert!(lengths.first() > lengths.last(), "{lengths:?}");
        assert!(lengths.len() >= 6, "{lengths:?}");
    }

    #[test]
    fn short_videos_are_not_split() {
        assert!(!Scheduler::worth_splitting(2 * MIN_SEGMENT - 1));
        assert!(Scheduler::worth_splitting(2 * MIN_SEGMENT));
    }
}
