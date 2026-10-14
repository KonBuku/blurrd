//! Turning the detectors' per-frame boxes into the project's tracks.
//!
//! The analysis hands every frame's boxes, in a fixed order, to a
//! [`TrackBuilder`]. A box continues a track of an earlier frame when the
//! detector says so (kill-feed and voice-chat names carry the identity
//! their tracker gave them), when it is the next frame of a banner episode,
//! or when it overlaps a box of the previous frame of the same region and
//! kind (text chat, screen surfaces); otherwise it starts a new track.
//! Track ids are given in the order tracks start, so the order of a
//! frame's boxes in [`BlurPlan::frame_table`] (track order) is known as soon
//! as the frame is: [`TrackBuilder::boxes_at`] returns exactly the boxes
//! the finished plan will have on that frame, which is what lets the
//! streaming `run` render frames before the analysis ends.

use std::collections::HashMap;

use blurrd_core::{
    BlurBox, BlurPlan, Confidence, NameKind, RectI, Region, RegionSet, Segment, Track, TrackId,
    TrackSource,
};

/// Smallest overlap (intersection over union) with which a box continues
/// an overlap-linked track of the previous frame.
const LINK_MIN_IOU: f32 = 0.3;

/// How a box links to the boxes of earlier frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Link {
    /// The detector's own identity of the name (unique within its region).
    Key(u32),
    /// The region shows one name at a time; consecutive frames belong
    /// together.
    Adjacent,
    /// By overlap with a box of the previous frame.
    Overlap,
}

/// Where the blur's block grid of a box starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Grid {
    /// At the box's top-left corner.
    Corner,
    /// At this source point.
    At(i32, i32),
    /// Moved by this many source pixels since the name's reference frame:
    /// the grid follows the name's motion from the track's first box.
    Moved(i32, i32),
}

/// One box of one frame, in source pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RegionBox {
    /// The region.
    pub(crate) region: Region,
    /// What the name is.
    pub(crate) kind: NameKind,
    /// The box (inside the frame).
    pub(crate) rect: RectI,
    /// How it was found.
    pub(crate) conf: Confidence,
    /// How it links to earlier frames.
    pub(crate) link: Link,
    /// Its block grid.
    pub(crate) grid: Grid,
}

/// What the builder keeps per track.
#[derive(Debug, Clone, Copy)]
struct TrackState {
    /// The last frame with a box.
    last_frame: u32,
    /// The box on that frame.
    last_rect: RectI,
    /// How the track links.
    link: Link,
    /// Top-left corner of the first box (the track anchor of the plan).
    anchor: (i32, i32),
    /// The motion of the first box, for [`Grid::Moved`].
    first_moved: (i32, i32),
}

/// Builds tracks frame by frame.
#[derive(Debug, Default)]
pub(crate) struct TrackBuilder {
    tracks: Vec<Track>,
    states: Vec<TrackState>,
    /// The track of each detector identity, per region.
    keys: HashMap<(Region, u32), usize>,
    /// Tracks with a box on the last frame pushed, in track order.
    active: Vec<usize>,
    /// The last frame pushed.
    frame: Option<u32>,
}

impl TrackBuilder {
    /// An empty builder.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Adds the boxes of `frame` (frames in increasing order).
    pub(crate) fn push(&mut self, frame: u32, boxes: &[RegionBox]) {
        self.frame = Some(frame);
        self.active.clear();
        let mut assigned: Vec<Option<usize>> = vec![None; boxes.len()];
        let continues = |state: &TrackState, link: Link| {
            state.link == link && frame > 0 && state.last_frame == frame - 1
        };
        // Identity and episode links, in box order.
        for (i, b) in boxes.iter().enumerate() {
            let free = |t: usize| !assigned.contains(&Some(t));
            let found = match b.link {
                Link::Key(key) => self
                    .keys
                    .get(&(b.region, key))
                    .copied()
                    .filter(|&t| free(t)),
                Link::Adjacent => (0..self.tracks.len()).find(|&t| {
                    let track = &self.tracks[t];
                    track.region == b.region
                        && track.kind == b.kind
                        && continues(&self.states[t], Link::Adjacent)
                        && free(t)
                }),
                Link::Overlap => None,
            };
            assigned[i] = found;
        }
        // Overlap links: best overlaps first, ties by track then box order.
        let mut pairs: Vec<(f32, usize, usize)> = Vec::new();
        for (i, b) in boxes.iter().enumerate() {
            if b.link != Link::Overlap {
                continue;
            }
            for (t, (track, state)) in self.tracks.iter().zip(&self.states).enumerate() {
                if track.region == b.region
                    && track.kind == b.kind
                    && continues(state, Link::Overlap)
                {
                    let iou = iou(state.last_rect, b.rect);
                    if iou >= LINK_MIN_IOU {
                        pairs.push((iou, t, i));
                    }
                }
            }
        }
        pairs.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        for (_, t, i) in pairs {
            if assigned[i].is_none() && !assigned.contains(&Some(t)) {
                assigned[i] = Some(t);
            }
        }
        // Everything else starts a track, in box order.
        for (i, b) in boxes.iter().enumerate() {
            let track = match assigned[i] {
                Some(t) => t,
                None => self.start(b, frame),
            };
            if let Link::Key(key) = b.link {
                self.keys.insert((b.region, key), track);
            }
            self.append(track, b, frame);
            self.active.push(track);
        }
        self.active.sort_unstable();
    }

    /// Starts a track with box `b` on `frame`; returns its index.
    fn start(&mut self, b: &RegionBox, frame: u32) -> usize {
        let index = self.tracks.len();
        self.tracks.push(Track {
            id: TrackId(u32::try_from(index + 1).unwrap_or(u32::MAX)),
            region: b.region,
            kind: b.kind,
            source: TrackSource::Detector,
            segments: Vec::new(),
        });
        self.states.push(TrackState {
            last_frame: frame,
            last_rect: b.rect,
            link: b.link,
            anchor: (b.rect.x0, b.rect.y0),
            first_moved: match b.grid {
                Grid::Moved(x, y) => (x, y),
                Grid::Corner | Grid::At(..) => (0, 0),
            },
        });
        index
    }

    /// Adds box `b` on `frame` to track `t`.
    fn append(&mut self, t: usize, b: &RegionBox, frame: u32) {
        let state = &mut self.states[t];
        let grid = match b.grid {
            Grid::Corner => (b.rect.x0, b.rect.y0),
            Grid::At(x, y) => (x, y),
            Grid::Moved(x, y) => (
                state.anchor.0 + x - state.first_moved.0,
                state.anchor.1 + y - state.first_moved.1,
            ),
        };
        let offset = (
            clamp_i16(grid.0 - state.anchor.0),
            clamp_i16(grid.1 - state.anchor.1),
        );
        state.last_frame = frame;
        state.last_rect = b.rect;
        let segments = &mut self.tracks[t].segments;
        match segments.last_mut() {
            Some(last)
                if frame > 0
                    && last.f1 == frame - 1
                    && last.rect == b.rect
                    && last.conf == b.conf
                    && last.text_offset == offset =>
            {
                last.f1 = frame;
            }
            _ => segments.push(Segment {
                f0: frame,
                f1: frame,
                rect: b.rect,
                conf: b.conf,
                text_offset: offset,
            }),
        }
    }

    /// The boxes the finished plan has on the last frame pushed, in its
    /// order, for the `enabled` regions (written into `out`, cleared
    /// first).
    pub(crate) fn boxes_at(&self, enabled: RegionSet, out: &mut Vec<BlurBox>) {
        out.clear();
        if self.frame.is_none() {
            return;
        }
        // A one-frame plan of the active tracks, each reduced to its box on
        // this frame with the same block grid origin: `frame_table` then
        // derives every box exactly as it will from the full tracks.
        let tracks = self
            .active
            .iter()
            .filter_map(|&t| {
                let track = &self.tracks[t];
                let state = &self.states[t];
                let segment = track.segments.last()?;
                let grid = (
                    state.anchor.0 + i32::from(segment.text_offset.0),
                    state.anchor.1 + i32::from(segment.text_offset.1),
                );
                Some(Track {
                    id: track.id,
                    region: track.region,
                    kind: track.kind,
                    source: track.source,
                    segments: vec![Segment {
                        f0: 0,
                        f1: 0,
                        text_offset: (
                            clamp_i16(grid.0 - segment.rect.x0),
                            clamp_i16(grid.1 - segment.rect.y0),
                        ),
                        ..*segment
                    }],
                })
            })
            .collect();
        let plan = BlurPlan {
            frame_count: 1,
            tracks,
            overrides: Vec::new(),
        };
        out.extend_from_slice(plan.frame_table(&enabled).boxes(0));
    }

    /// The tracks, in id order.
    pub(crate) fn finish(self) -> Vec<Track> {
        self.tracks
    }
}

/// `value` saturated to the `i16` range.
fn clamp_i16(value: i32) -> i16 {
    value.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
}

/// Intersection over union of two rectangles.
fn iou(a: RectI, b: RectI) -> f32 {
    let area = |r: RectI| r.width() as f32 * r.height() as f32;
    match a.intersection(&b) {
        Some(i) => {
            let inter = area(i);
            inter / (area(a) + area(b) - inter)
        }
        None => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use blurrd_core::{BlurPlan, Confidence, NameKind, RectI, Region, RegionSet};

    use super::{Grid, Link, RegionBox, TrackBuilder};

    fn rect(x0: i32, y0: i32, w: i32, h: i32) -> RectI {
        RectI::new(x0, y0, x0 + w, y0 + h).unwrap()
    }

    fn feed(part: u32, y: i32, moved: i32) -> RegionBox {
        RegionBox {
            region: Region::KillFeed,
            kind: NameKind::Victim,
            rect: rect(1780, y, 50, 23),
            conf: Confidence::Tracked,
            link: Link::Key(part),
            grid: Grid::Moved(0, moved),
        }
    }

    fn chat(y: i32) -> RegionBox {
        RegionBox {
            region: Region::TextChat,
            kind: NameKind::ChatSender,
            rect: rect(40, y, 90, 18),
            conf: Confidence::Tracked,
            link: Link::Overlap,
            grid: Grid::At(40, y),
        }
    }

    fn banner(width: i32) -> RegionBox {
        RegionBox {
            region: Region::Banner,
            kind: NameKind::Banner,
            rect: rect(960 - width / 2, 300, width, 27),
            conf: Confidence::PreRoll,
            link: Link::Adjacent,
            grid: Grid::At(960 - width / 2, 300),
        }
    }

    /// A short scene: a kill-feed row that jumps up, a chat line pushed up,
    /// a banner episode, a second row, gaps.
    fn scene(frame: u32) -> Vec<RegionBox> {
        let mut boxes = Vec::new();
        if frame < 40 {
            let y = if frame < 20 { 140 } else { 101 };
            boxes.push(feed(7, y, y - 140));
        }
        if (10..50).contains(&frame) {
            boxes.push(feed(9, 179, 0));
        }
        if (5..30).contains(&frame) {
            boxes.push(chat(900 - 2 * (frame as i32 - 5).min(10)));
        }
        if (35..45).contains(&frame) {
            boxes.push(chat(700));
        }
        if (12..25).contains(&frame) {
            boxes.push(banner(100 + 4 * frame as i32));
        }
        if (27..33).contains(&frame) {
            boxes.push(banner(160));
        }
        boxes
    }

    #[test]
    fn tracks_follow_identities_episodes_and_overlaps() {
        let enabled = RegionSet::all();
        let mut builder = TrackBuilder::new();
        let mut streamed = Vec::new();
        let mut boxes = Vec::new();
        for frame in 0..60 {
            builder.push(frame, &scene(frame));
            builder.boxes_at(enabled, &mut boxes);
            streamed.push(boxes.clone());
        }
        let plan = BlurPlan {
            frame_count: 60,
            tracks: builder.finish(),
            overrides: Vec::new(),
        };
        assert_eq!(plan.validate(1920, 1080), Ok(()));
        // Every frame of the plan blurs exactly the boxes pushed for it, and
        // exactly the boxes the builder gave while streaming.
        let table = plan.frame_table(&enabled);
        for (frame, boxes) in (0..).zip(&streamed) {
            assert_eq!(table.boxes(frame), boxes.as_slice(), "frame {frame}");
            let mut planned: Vec<RectI> = table.boxes(frame).iter().map(|b| b.rect).collect();
            let mut pushed: Vec<RectI> = scene(frame).iter().map(|b| b.rect).collect();
            planned.sort_by_key(|r| (r.x0, r.y0, r.x1, r.y1));
            pushed.sort_by_key(|r| (r.x0, r.y0, r.x1, r.y1));
            assert_eq!(planned, pushed, "frame {frame}");
        }
        // Rows keep their identity through the jump, chat lines and banner
        // episodes split where they stop.
        let regions: Vec<Region> = plan.tracks.iter().map(|t| t.region).collect();
        assert_eq!(
            regions,
            [
                Region::KillFeed,
                Region::TextChat,
                Region::KillFeed,
                Region::Banner,
                Region::Banner,
                Region::TextChat
            ]
        );
        // The grid moved with the row: 39 px up from frame 20.
        let row = &plan.tracks[0];
        assert_eq!(row.segments.len(), 2);
        assert_eq!(row.segments[1].text_offset, (0, -39));
    }

    #[test]
    fn a_repeated_identity_on_one_frame_starts_a_new_track() {
        let mut builder = TrackBuilder::new();
        builder.push(0, &[feed(1, 140, 0), feed(1, 179, 0)]);
        builder.push(1, &[feed(1, 179, 0)]);
        let tracks = builder.finish();
        assert_eq!(tracks.len(), 2);
        assert_eq!(
            tracks[1].segments[0].f1, 1,
            "the key follows the newest track"
        );
    }
}
