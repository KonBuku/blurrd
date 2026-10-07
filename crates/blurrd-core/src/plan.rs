//! Box tracks and the blur plan.
//!
//! The detector emits one [`Track`] per name it follows. A track is a list of
//! [`Segment`]s, each holding one rectangle for a run of frames. Together
//! with the user's [`Override`]s the tracks form the [`BlurPlan`], which
//! answers "which boxes are blurred on frame N".

use std::fmt;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::geometry::RectI;
use crate::overrides::{Override, OverrideBinding, Resolution};
use crate::region::{NameKind, Region, RegionSet};

/// Identifier of a track, unique within a plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(transparent)]
pub struct TrackId(pub u32);

impl fmt::Display for TrackId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Who created a track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum TrackSource {
    /// The HUD detector.
    Detector,
    /// The OCR verification, after it still read a name.
    OcrFix,
    /// The user.
    User,
}

/// How a segment's rectangle was obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum Confidence {
    /// Tracked on the frame itself.
    Tracked,
    /// Held over a few frames without a fresh detection.
    WeakHold,
    /// Interpolated across a gap between two detections.
    GapFill,
    /// Extended before the first detection (fade-in, slide-in).
    PreRoll,
    /// Extended after the last detection (fade-out).
    PostRoll,
    /// Found from colour priors only.
    ColourPrior,
    /// Added by the OCR verification.
    OcrFix,
}

impl Confidence {
    /// The block-size multiplier for the blur.
    ///
    /// Frames around animations and gaps use 1.5× larger blocks so that
    /// combining several frames cannot recover the name (C17 in the
    /// architecture plan).
    pub const fn block_scale(self) -> f32 {
        match self {
            Self::PreRoll | Self::GapFill | Self::OcrFix => 1.5,
            Self::Tracked | Self::WeakHold | Self::PostRoll | Self::ColourPrior => 1.0,
        }
    }
}

/// One rectangle of a track, held for the frames `f0..=f1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Segment {
    /// First frame.
    pub f0: u32,
    /// Last frame (inclusive).
    pub f1: u32,
    /// The box in source pixels.
    pub rect: RectI,
    /// How the box was obtained.
    pub conf: Confidence,
    /// Offset of the name's text from the track anchor (the top-left corner
    /// of the track's first segment), in source pixels.
    ///
    /// The blur's block grid follows the text, not the box, so that a box
    /// growing during a wipe-in does not shift the pixel grid.
    pub text_offset: (i16, i16),
}

impl Segment {
    /// Whether `frame` is inside the segment.
    pub const fn contains(&self, frame: u32) -> bool {
        self.f0 <= frame && frame <= self.f1
    }
}

/// The boxes for one name over time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Track {
    /// Identifier, unique within the plan.
    pub id: TrackId,
    /// The region the name is in.
    pub region: Region,
    /// What the name is.
    pub kind: NameKind,
    /// Who created the track.
    pub source: TrackSource,
    /// The boxes, sorted by frame and non-overlapping.
    pub segments: Vec<Segment>,
}

impl Track {
    /// The segment that covers `frame`, if any.
    ///
    /// Uses a binary search, so the track must be valid
    /// ([`Track::validate`]).
    pub fn segment_at(&self, frame: u32) -> Option<&Segment> {
        let index = self.segments.partition_point(|s| s.f1 < frame);
        self.segments.get(index).filter(|s| s.f0 <= frame)
    }

    /// The segments that overlap the frames `f0..=f1`, in order.
    pub fn segments_between(&self, f0: u32, f1: u32) -> impl Iterator<Item = &Segment> + '_ {
        let start = self.segments.partition_point(|s| s.f1 < f0);
        self.segments[start..]
            .iter()
            .take_while(move |s| s.f0 <= f1)
    }

    /// The origin of the blur block grid for `segment`: the track anchor
    /// moved by the segment's text offset.
    fn grid_origin(&self, segment: &Segment) -> (i32, i32) {
        let anchor = self
            .segments
            .first()
            .map_or((0, 0), |s| (s.rect.x0, s.rect.y0));
        (
            anchor.0 + i32::from(segment.text_offset.0),
            anchor.1 + i32::from(segment.text_offset.1),
        )
    }

    /// The noise seed of the track's boxes, derived from its id and region.
    fn seed(&self) -> u32 {
        // Hash the region's key rather than its position in the enum, so
        // that adding regions never changes the seeds of existing tracks.
        let region = self
            .region
            .key()
            .bytes()
            .fold(0_u64, |acc, b| acc.rotate_left(8) ^ u64::from(b));
        mix32(u64::from(self.id.0) ^ region.rotate_left(32))
    }

    /// Checks the track's invariants.
    ///
    /// # Errors
    ///
    /// Returns a [`PlanError`] if the track has no segments, if a segment
    /// ends before it starts, overlaps or precedes the previous segment,
    /// ends at or after `frame_count`, or has a rectangle outside the
    /// `width` × `height` frame.
    pub fn validate(&self, frame_count: u32, width: u32, height: u32) -> Result<(), PlanError> {
        if self.segments.is_empty() {
            return Err(PlanError::EmptyTrack(self.id));
        }
        let mut previous_end: Option<u32> = None;
        for (index, segment) in self.segments.iter().enumerate() {
            let ordered =
                segment.f0 <= segment.f1 && previous_end.is_none_or(|end| segment.f0 > end);
            if !ordered {
                return Err(PlanError::UnorderedSegment {
                    track: self.id,
                    index,
                });
            }
            if segment.f1 >= frame_count {
                return Err(PlanError::TrackBeyondEnd {
                    track: self.id,
                    index,
                    frame_count,
                });
            }
            if !segment.rect.is_inside(width, height) {
                return Err(PlanError::TrackRectOutsideFrame {
                    track: self.id,
                    index,
                    rect: segment.rect,
                });
            }
            previous_end = Some(segment.f1);
        }
        Ok(())
    }
}

/// One box to blur on one frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlurBox {
    /// The box in source pixels (unpadded).
    pub rect: RectI,
    /// The region the box belongs to.
    pub region: Region,
    /// Seed for the optional blur noise; constant over a track.
    pub seed: u32,
    /// Origin of the block grid in source pixels; follows the text.
    pub grid_origin: (i32, i32),
    /// Block-size multiplier, 1.0 or 1.5 (see [`Confidence::block_scale`]).
    pub block_scale: f32,
}

/// Why a plan, track or override is invalid.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PlanError {
    /// A track without segments.
    #[error("track {0} has no segments")]
    EmptyTrack(TrackId),
    /// Two tracks share an id.
    #[error("track id {0} is used more than once")]
    DuplicateTrackId(TrackId),
    /// A segment ends before it starts or does not follow the previous one.
    #[error(
        "track {track}: segment {index} ends before it starts or overlaps the previous segment"
    )]
    UnorderedSegment {
        /// The track.
        track: TrackId,
        /// Index of the segment in the track.
        index: usize,
    },
    /// A segment reaches past the last frame.
    #[error(
        "track {track}: segment {index} reaches past the last frame (frame count {frame_count})"
    )]
    TrackBeyondEnd {
        /// The track.
        track: TrackId,
        /// Index of the segment in the track.
        index: usize,
        /// Number of frames in the media.
        frame_count: u32,
    },
    /// A segment's rectangle is not inside the frame.
    #[error("track {track}: segment {index} rectangle {rect} lies outside the frame")]
    TrackRectOutsideFrame {
        /// The track.
        track: TrackId,
        /// Index of the segment in the track.
        index: usize,
        /// The offending rectangle.
        rect: RectI,
    },
    /// Two overrides share an id.
    #[error("override {0} is listed more than once")]
    DuplicateOverride(uuid::Uuid),
    /// An override's frame range is reversed or reaches past the last frame.
    #[error("override {uid}: frames {f0}..={f1} are reversed or reach past the last frame")]
    OverrideFrames {
        /// The override.
        uid: uuid::Uuid,
        /// First frame.
        f0: u32,
        /// Last frame.
        f1: u32,
    },
    /// An override's rectangle or anchor is not inside the frame.
    #[error("override {uid}: rectangle {rect} lies outside the frame")]
    OverrideRectOutsideFrame {
        /// The override.
        uid: uuid::Uuid,
        /// The offending rectangle.
        rect: RectI,
    },
}

/// Detector tracks plus user overrides for one video.
///
/// A plan is plain data. Call [`BlurPlan::validate`] on plans from untrusted
/// sources (files, IPC) before using them; lookups on invalid plans never
/// panic but may miss boxes.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct BlurPlan {
    /// Number of frames of the media.
    pub frame_count: u32,
    /// Detector and OCR tracks.
    pub tracks: Vec<Track>,
    /// User edits, applied in order after the tracks.
    pub overrides: Vec<Override>,
}

impl BlurPlan {
    /// Checks every track and override against the frame count and the
    /// `width` × `height` frame.
    ///
    /// # Errors
    ///
    /// Returns the first [`PlanError`] found.
    pub fn validate(&self, width: u32, height: u32) -> Result<(), PlanError> {
        let mut ids: Vec<TrackId> = self.tracks.iter().map(|t| t.id).collect();
        ids.sort_unstable();
        if let Some(pair) = ids.windows(2).find(|w| w[0] == w[1]) {
            return Err(PlanError::DuplicateTrackId(pair[0]));
        }
        for track in &self.tracks {
            track.validate(self.frame_count, width, height)?;
        }
        let mut uids: Vec<uuid::Uuid> = self.overrides.iter().map(Override::uid).collect();
        uids.sort_unstable();
        if let Some(pair) = uids.windows(2).find(|w| w[0] == w[1]) {
            return Err(PlanError::DuplicateOverride(pair[0]));
        }
        for edit in &self.overrides {
            let uid = edit.uid();
            let (f0, f1) = edit.frames();
            if f0 > f1 || f1 >= self.frame_count {
                return Err(PlanError::OverrideFrames { uid, f0, f1 });
            }
            let rects = match *edit {
                Override::Add { rect, .. } => [Some(rect), None],
                Override::Suppress { anchor, .. } => [Some(anchor), None],
                Override::Adjust { anchor, rect, .. } => [Some(anchor), Some(rect)],
            };
            if let Some(rect) = rects
                .into_iter()
                .flatten()
                .find(|r| !r.is_inside(width, height))
            {
                return Err(PlanError::OverrideRectOutsideFrame { uid, rect });
            }
        }
        Ok(())
    }

    /// The binding state of every override, in list order.
    pub fn bindings(&self) -> Vec<OverrideBinding> {
        Resolution::new(&self.tracks, &self.overrides).bindings
    }

    /// Writes the boxes of `frame` into `out` (cleared first).
    ///
    /// Boxes of regions not in `enabled` are skipped, except
    /// [`Region::Manual`] boxes, which are always applied. The order is the
    /// track order followed by the `Add` overrides in list order, the same
    /// as [`BlurPlan::frame_table`]. This resolves the overrides on every
    /// call; use [`BlurPlan::frame_table`] for more than a few frames.
    pub fn boxes_at(&self, frame: u32, enabled: &RegionSet, out: &mut Vec<BlurBox>) {
        out.clear();
        let resolution = Resolution::new(&self.tracks, &self.overrides);
        for (index, track) in self.tracks.iter().enumerate() {
            if !is_visible(track.region, *enabled) {
                continue;
            }
            if let Some(segment) = track.segment_at(frame)
                && let Some(rect) = resolution.apply(&self.overrides, index, frame, segment.rect)
            {
                out.push(track_box(track, segment, rect));
            }
        }
        out.extend(
            self.overrides
                .iter()
                .filter(|edit| edit.covers(frame))
                .filter_map(|edit| added_box(edit, *enabled)),
        );
    }

    /// Builds the flat per-frame box table used by the render.
    ///
    /// Equivalent to calling [`BlurPlan::boxes_at`] for every frame, but
    /// resolves the overrides once. Boxes on frames at or beyond
    /// `frame_count` are dropped.
    pub fn frame_table(&self, enabled: &RegionSet) -> FrameBoxTable {
        let resolution = Resolution::new(&self.tracks, &self.overrides);
        let frames = self.frame_count as usize;
        // Two passes over the same boxes: count per frame, then write each
        // box into its frame's slice. Linear in the number of boxes and keeps
        // the per-frame order of `boxes_at`.
        let mut offsets = vec![0_u32; frames + 1];
        self.for_each_box(&resolution, *enabled, |frame, _| {
            offsets[frame as usize + 1] += 1;
        });
        for i in 1..offsets.len() {
            offsets[i] += offsets[i - 1];
        }
        let mut boxes = vec![PLACEHOLDER_BOX; offsets[frames] as usize];
        let mut cursor = offsets[..frames].to_vec();
        self.for_each_box(&resolution, *enabled, |frame, blur_box| {
            let slot = &mut cursor[frame as usize];
            boxes[*slot as usize] = blur_box;
            *slot += 1;
        });
        FrameBoxTable { offsets, boxes }
    }

    /// Calls `visit` for every box of every frame below `frame_count`: the
    /// track boxes track by track, then the `Add` overrides in list order.
    fn for_each_box(
        &self,
        resolution: &Resolution,
        enabled: RegionSet,
        mut visit: impl FnMut(u32, BlurBox),
    ) {
        let Some(last_frame) = self.frame_count.checked_sub(1) else {
            return;
        };
        for (index, track) in self.tracks.iter().enumerate() {
            if !is_visible(track.region, enabled) {
                continue;
            }
            for segment in &track.segments {
                let template = track_box(track, segment, segment.rect);
                for frame in segment.f0..=segment.f1.min(last_frame) {
                    if let Some(rect) =
                        resolution.apply(&self.overrides, index, frame, segment.rect)
                    {
                        visit(frame, BlurBox { rect, ..template });
                    }
                }
            }
        }
        for edit in &self.overrides {
            if let Some(blur_box) = added_box(edit, enabled) {
                let (f0, f1) = edit.frames();
                for frame in f0..=f1.min(last_frame) {
                    visit(frame, blur_box);
                }
            }
        }
    }

    /// Per-region coverage of the timeline, for the density lanes.
    ///
    /// The frames are split into `buckets` equal parts (at most one per
    /// frame); each lane holds, per bucket, the fraction of frames with at
    /// least one box of its region. All regions count. Only regions with at
    /// least one box get a lane, in [`Region::ALL`] order.
    pub fn density(&self, buckets: u32) -> DensityLanes {
        let buckets = buckets.min(self.frame_count);
        let table = self.frame_table(&RegionSet::all());
        let mut lanes = Vec::new();
        for region in Region::ALL {
            let has_region = |frame: u32| table.boxes(frame).iter().any(|b| b.region == region);
            if !(0..self.frame_count).any(has_region) {
                continue;
            }
            let coverage = (0..buckets)
                .map(|bucket| {
                    let (start, end) = bucket_bounds(bucket, buckets, self.frame_count);
                    let hits = (start..end).filter(|&f| has_region(f)).count();
                    hits as f32 / (end - start) as f32
                })
                .collect();
            lanes.push(DensityLane { region, coverage });
        }
        DensityLanes {
            frame_count: self.frame_count,
            buckets,
            lanes,
        }
    }
}

/// The boxes of every frame in one flat buffer, built once per render.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameBoxTable {
    /// `offsets[f]..offsets[f + 1]` indexes the boxes of frame `f`.
    offsets: Vec<u32>,
    boxes: Vec<BlurBox>,
}

impl FrameBoxTable {
    /// Number of frames in the table.
    pub fn frame_count(&self) -> u32 {
        (self.offsets.len() - 1) as u32
    }

    /// The boxes of `frame`; empty for frames without boxes or beyond the
    /// end.
    pub fn boxes(&self, frame: u32) -> &[BlurBox] {
        let f = frame as usize;
        match (self.offsets.get(f), self.offsets.get(f + 1)) {
            (Some(&start), Some(&end)) => &self.boxes[start as usize..end as usize],
            _ => &[],
        }
    }

    /// Total number of boxes over all frames.
    pub fn total_boxes(&self) -> usize {
        self.boxes.len()
    }
}

/// Timeline coverage per region, see [`BlurPlan::density`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct DensityLanes {
    /// Number of frames of the media.
    pub frame_count: u32,
    /// Number of buckets per lane.
    pub buckets: u32,
    /// One lane per region with boxes.
    pub lanes: Vec<DensityLane>,
}

/// Coverage of one region over the timeline buckets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct DensityLane {
    /// The region.
    pub region: Region,
    /// Per bucket, the fraction (0.0 to 1.0) of frames with a box.
    pub coverage: Vec<f32>,
}

/// Initial value of the table slots; every slot is overwritten.
const PLACEHOLDER_BOX: BlurBox = BlurBox {
    rect: RectI {
        x0: 0,
        y0: 0,
        x1: 1,
        y1: 1,
    },
    region: Region::Manual,
    seed: 0,
    grid_origin: (0, 0),
    block_scale: 1.0,
};

/// Frames `start..end` of `bucket` when `frame_count` frames are split into
/// `buckets` near-equal parts (`buckets <= frame_count`, so none is empty).
fn bucket_bounds(bucket: u32, buckets: u32, frame_count: u32) -> (u32, u32) {
    let at = |b: u32| (u64::from(b) * u64::from(frame_count) / u64::from(buckets)) as u32;
    (at(bucket), at(bucket + 1))
}

/// Whether boxes of `region` are applied when `enabled` regions are on.
fn is_visible(region: Region, enabled: RegionSet) -> bool {
    region == Region::Manual || enabled.contains(region)
}

/// The box of `segment` of `track`, with `rect` after overrides.
fn track_box(track: &Track, segment: &Segment, rect: RectI) -> BlurBox {
    BlurBox {
        rect,
        region: track.region,
        seed: track.seed(),
        grid_origin: track.grid_origin(segment),
        block_scale: segment.conf.block_scale(),
    }
}

/// The box of an `Add` override if its region is visible.
fn added_box(edit: &Override, enabled: RegionSet) -> Option<BlurBox> {
    match *edit {
        Override::Add {
            uid, region, rect, ..
        } if is_visible(region, enabled) => {
            let (high, low) = uid.as_u64_pair();
            Some(BlurBox {
                rect,
                region,
                seed: mix32(high ^ low.rotate_left(32)),
                grid_origin: (rect.x0, rect.y0),
                block_scale: 1.0,
            })
        }
        _ => None,
    }
}

/// Folds a 64-bit value into a well-mixed 32-bit seed (the SplitMix64
/// finaliser). Seeds must stay stable across versions so that re-renders of
/// a project produce the same noise.
const fn mix32(value: u64) -> u32 {
    let mut z = value.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (z ^ (z >> 31)) as u32
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::{BlurPlan, Confidence, PlanError, Segment, Track, TrackId, TrackSource};
    use crate::geometry::RectI;
    use crate::overrides::{BindStatus, Override};
    use crate::region::{NameKind, Region, RegionSet};

    fn rect(x0: i32, y0: i32, x1: i32, y1: i32) -> RectI {
        RectI::new(x0, y0, x1, y1).unwrap()
    }

    fn segment(f0: u32, f1: u32, rect: RectI, text_offset: (i16, i16)) -> Segment {
        Segment {
            f0,
            f1,
            rect,
            conf: Confidence::Tracked,
            text_offset,
        }
    }

    fn track(id: u32, region: Region, segments: Vec<Segment>) -> Track {
        Track {
            id: TrackId(id),
            region,
            kind: NameKind::Victim,
            source: TrackSource::Detector,
            segments,
        }
    }

    /// A kill-feed victim that jumps up one row at frame 50.
    fn feed_plan() -> BlurPlan {
        BlurPlan {
            frame_count: 100,
            tracks: vec![track(
                17,
                Region::KillFeed,
                vec![
                    segment(10, 49, rect(1782, 140, 1835, 163), (0, 0)),
                    segment(50, 90, rect(1782, 101, 1835, 124), (0, -39)),
                ],
            )],
            overrides: Vec::new(),
        }
    }

    fn uid(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn boxes(plan: &BlurPlan, frame: u32) -> Vec<RectI> {
        let mut out = Vec::new();
        plan.boxes_at(frame, &RegionSet::all(), &mut out);
        out.iter().map(|b| b.rect).collect()
    }

    #[test]
    fn looks_up_segments() {
        let plan = feed_plan();
        assert_eq!(boxes(&plan, 9), []);
        assert_eq!(boxes(&plan, 10), [rect(1782, 140, 1835, 163)]);
        assert_eq!(boxes(&plan, 50), [rect(1782, 101, 1835, 124)]);
        assert_eq!(boxes(&plan, 91), []);
    }

    #[test]
    fn grid_follows_the_text_offset() {
        let plan = feed_plan();
        let mut out = Vec::new();
        plan.boxes_at(60, &RegionSet::all(), &mut out);
        assert_eq!(out[0].grid_origin, (1782, 101));
        plan.boxes_at(20, &RegionSet::all(), &mut out);
        assert_eq!(out[0].grid_origin, (1782, 140));
    }

    #[test]
    fn uncertain_segments_use_larger_blocks() {
        let mut plan = feed_plan();
        plan.tracks[0].segments[0].conf = Confidence::PreRoll;
        let table = plan.frame_table(&RegionSet::all());
        assert!((table.boxes(20)[0].block_scale - 1.5).abs() < f32::EPSILON);
        assert!((table.boxes(60)[0].block_scale - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn disabled_regions_are_skipped_but_manual_boxes_stay() {
        let mut plan = feed_plan();
        plan.overrides.push(Override::Add {
            uid: uid(1),
            region: Region::Manual,
            f0: 0,
            f1: 99,
            rect: rect(40, 40, 200, 70),
        });
        let mut out = Vec::new();
        plan.boxes_at(20, &RegionSet::empty(), &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].region, Region::Manual);
    }

    #[test]
    fn suppress_binds_by_iou_and_hides_only_matching_frames() {
        let mut plan = feed_plan();
        plan.overrides.push(Override::Suppress {
            uid: uid(2),
            region: Region::KillFeed,
            f0: 0,
            f1: 99,
            anchor: rect(1783, 140, 1835, 163),
        });
        assert_eq!(plan.bindings()[0].status, BindStatus::Bound);
        assert!(boxes(&plan, 20).is_empty(), "matching row is hidden");
        assert_eq!(
            boxes(&plan, 60).len(),
            1,
            "after the stack jump the box no longer matches"
        );
    }

    #[test]
    fn unbound_suppress_hides_nothing() {
        let mut plan = feed_plan();
        // Same frames, wrong region; then right region, too little overlap.
        plan.overrides.push(Override::Suppress {
            uid: uid(3),
            region: Region::Banner,
            f0: 0,
            f1: 99,
            anchor: rect(1782, 140, 1835, 163),
        });
        plan.overrides.push(Override::Suppress {
            uid: uid(4),
            region: Region::KillFeed,
            f0: 0,
            f1: 99,
            anchor: rect(1782, 150, 1835, 173),
        });
        assert!(
            plan.bindings()
                .iter()
                .all(|b| b.status == BindStatus::Unbound)
        );
        assert_eq!(
            plan.frame_table(&RegionSet::all()),
            feed_plan().frame_table(&RegionSet::all())
        );
    }

    #[test]
    fn adjust_replaces_the_box() {
        let mut plan = feed_plan();
        plan.overrides.push(Override::Adjust {
            uid: uid(5),
            region: Region::KillFeed,
            f0: 10,
            f1: 20,
            anchor: rect(1782, 140, 1835, 163),
            rect: rect(1778, 140, 1835, 163),
        });
        assert_eq!(boxes(&plan, 15), [rect(1778, 140, 1835, 163)]);
        assert_eq!(boxes(&plan, 21), [rect(1782, 140, 1835, 163)]);
    }

    #[test]
    fn adjust_binds_to_the_best_track_only() {
        let mut plan = feed_plan();
        plan.tracks.push(track(
            18,
            Region::KillFeed,
            vec![segment(10, 49, rect(1780, 140, 1835, 163), (0, 0))],
        ));
        plan.overrides.push(Override::Adjust {
            uid: uid(6),
            region: Region::KillFeed,
            f0: 10,
            f1: 20,
            anchor: rect(1780, 140, 1835, 163),
            rect: rect(1700, 140, 1835, 163),
        });
        assert_eq!(
            boxes(&plan, 15),
            [rect(1782, 140, 1835, 163), rect(1700, 140, 1835, 163)]
        );
    }

    #[test]
    fn validates_tracks_and_overrides() {
        let plan = feed_plan();
        assert_eq!(plan.validate(1920, 1080), Ok(()));
        assert!(matches!(
            plan.validate(1800, 1080),
            Err(PlanError::TrackRectOutsideFrame { .. })
        ));

        let mut overlapping = feed_plan();
        overlapping.tracks[0].segments[1].f0 = 49;
        assert!(matches!(
            overlapping.validate(1920, 1080),
            Err(PlanError::UnorderedSegment { index: 1, .. })
        ));

        let mut too_long = feed_plan();
        too_long.frame_count = 90;
        assert!(matches!(
            too_long.validate(1920, 1080),
            Err(PlanError::TrackBeyondEnd { .. })
        ));

        let mut duplicate = feed_plan();
        duplicate.tracks.push(duplicate.tracks[0].clone());
        assert_eq!(
            duplicate.validate(1920, 1080),
            Err(PlanError::DuplicateTrackId(TrackId(17)))
        );

        let mut reversed = feed_plan();
        reversed.overrides.push(Override::Add {
            uid: uid(7),
            region: Region::Manual,
            f0: 5,
            f1: 4,
            rect: rect(0, 0, 10, 10),
        });
        assert!(matches!(
            reversed.validate(1920, 1080),
            Err(PlanError::OverrideFrames { .. })
        ));
    }

    #[test]
    fn density_reports_coverage_per_region() {
        let plan = feed_plan();
        let lanes = plan.density(10);
        assert_eq!(lanes.buckets, 10);
        assert_eq!(lanes.lanes.len(), 1);
        let coverage = &lanes.lanes[0].coverage;
        assert!(coverage[0].abs() < f32::EPSILON);
        assert!((coverage[1] - 1.0).abs() < f32::EPSILON);
        // Frames 90..=99: only frame 90 has a box.
        assert!((coverage[9] - 0.1).abs() < 1e-6);
        assert_eq!(
            plan.density(1000).buckets,
            100,
            "at most one bucket per frame"
        );
    }

    #[test]
    fn serialises_like_the_project_format() {
        let mut plan = feed_plan();
        plan.overrides.push(Override::Suppress {
            uid: uid(8),
            region: Region::KillFeed,
            f0: 90,
            f1: 90,
            anchor: rect(1782, 101, 1835, 124),
        });
        let json = serde_json::to_value(&plan).unwrap();
        assert_eq!(
            json["tracks"][0]["segments"][1]["rect"],
            serde_json::json!([1782, 101, 1835, 124])
        );
        assert_eq!(
            json["tracks"][0]["segments"][1]["text_offset"],
            serde_json::json!([0, -39])
        );
        assert_eq!(json["tracks"][0]["region"], "KillFeed");
        assert_eq!(json["overrides"][0]["op"], "Suppress");
        let back: BlurPlan = serde_json::from_value(json).unwrap();
        assert_eq!(back, plan);
    }
}
