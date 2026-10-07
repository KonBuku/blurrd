//! Synthetic blur plans for tests and benchmarks.
//!
//! Shared by the integration tests (`mod support;`) and the benchmarks
//! (`#[path = "../tests/support/mod.rs"]`). Plans are deterministic and
//! contain no real names or footage.

use blurrd_core::{
    BlurPlan, Confidence, NameKind, Override, RectI, Region, Segment, Track, TrackId, TrackSource,
};
use uuid::Uuid;

/// Shape of a synthetic plan.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PlanShape {
    /// Number of frames of the media.
    pub(crate) frame_count: u32,
    /// Number of tracks, spread evenly over the timeline.
    pub(crate) tracks: u32,
    /// Segments per track (each one a stack jump or a wipe step).
    pub(crate) segments_per_track: u32,
    /// Frames a track lasts.
    pub(crate) track_len: u32,
    /// Number of overrides (Add, Suppress, Adjust in turn).
    pub(crate) overrides: u32,
}

impl PlanShape {
    /// The acceptance plan of milestone M4: 3601 frames (one minute at
    /// 60 fps), 300 tracks.
    pub(crate) const MINUTE: Self = Self {
        frame_count: 3601,
        tracks: 300,
        segments_per_track: 8,
        track_len: 240,
        overrides: 40,
    };

    /// A dense ten-minute plan at 60 fps: a kill-feed event every two
    /// seconds plus voice and chat names, each track with many segments.
    pub(crate) const TEN_MINUTES: Self = Self {
        frame_count: 36_000,
        tracks: 600,
        segments_per_track: 20,
        track_len: 300,
        overrides: 100,
    };
}

/// A small deterministic pseudo-random generator (SplitMix64).
#[derive(Debug)]
pub(crate) struct Rng(u64);

impl Rng {
    /// A generator with a fixed seed.
    pub(crate) fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// The next value in `0..bound` (`bound > 0`).
    pub(crate) fn below(&mut self, bound: u32) -> u32 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) % u64::from(bound)) as u32
    }
}

/// Builds a valid plan for a 1920 × 1080 frame. Rectangles are written as
/// literals because every one of them has area by construction.
pub(crate) fn synthetic_plan(shape: PlanShape) -> BlurPlan {
    let mut rng = Rng::new(0x5EED);
    let regions = Region::DETECTED;
    let latest_start = shape.frame_count - shape.track_len;
    let tracks: Vec<Track> = (0..shape.tracks)
        .map(|i| {
            let region = regions[i as usize % regions.len()];
            let start = (u64::from(i) * u64::from(latest_start) / u64::from(shape.tracks)) as u32;
            let (x0, y0, w) = match region {
                Region::KillFeed => (
                    1500 + rng.below(200).cast_signed(),
                    80 + 39 * rng.below(6).cast_signed(),
                    120,
                ),
                Region::Banner => (760, 180, 400),
                Region::VoiceChat => (60, 560 + 40 * rng.below(4).cast_signed(), 140),
                _ => (40, 700 + 29 * rng.below(8).cast_signed(), 110),
            };
            let step = shape.track_len / shape.segments_per_track;
            let segments = (0..shape.segments_per_track)
                .map(|s| {
                    let f0 = start + s * step;
                    let f1 = if s + 1 == shape.segments_per_track {
                        start + shape.track_len - 1
                    } else {
                        f0 + step - 1
                    };
                    // Rows move up one 39 px pitch every other segment, the
                    // box grows during a wipe-in on the others.
                    let dy = -39 * (s / 2).cast_signed();
                    let grow = if s % 2 == 0 { 0 } else { 6 };
                    let y = (y0 + dy).max(0);
                    Segment {
                        f0,
                        f1,
                        rect: RectI {
                            x0: x0 - grow,
                            y0: y,
                            x1: x0 + w,
                            y1: y + 23,
                        },
                        conf: if s == 0 {
                            Confidence::PreRoll
                        } else {
                            Confidence::Tracked
                        },
                        text_offset: (0, (y - y0) as i16),
                    }
                })
                .collect();
            Track {
                id: TrackId(i),
                region,
                kind: NameKind::Victim,
                source: TrackSource::Detector,
                segments,
            }
        })
        .collect();
    let overrides = (0..shape.overrides)
        .map(|i| {
            let uid = Uuid::from_u128(0xB1_u128 << 64 | u128::from(i));
            let track = &tracks[rng.below(shape.tracks) as usize];
            let segment = track.segments[rng.below(shape.segments_per_track) as usize];
            match i % 3 {
                0 => Override::Add {
                    uid,
                    region: Region::Manual,
                    f0: segment.f0,
                    f1: segment.f1,
                    rect: RectI {
                        x0: 40,
                        y0: 40,
                        x1: 200,
                        y1: 70,
                    },
                },
                1 => Override::Suppress {
                    uid,
                    region: track.region,
                    f0: segment.f0,
                    f1: segment.f1,
                    anchor: segment.rect,
                },
                _ => Override::Adjust {
                    uid,
                    region: track.region,
                    f0: segment.f0,
                    f1: segment.f1,
                    anchor: segment.rect,
                    rect: RectI {
                        x0: segment.rect.x0 - 4,
                        ..segment.rect
                    },
                },
            }
        })
        .collect();
    BlurPlan {
        frame_count: shape.frame_count,
        tracks,
        overrides,
    }
}
