//! The banner settle model (port spec section 4, `_banner_parts`).
//!
//! A banner episode is a run of frames with the banner gate on, merged
//! over gaps of at most [`EPISODE_GAP`] frames. Inside an episode the name
//! slides in from the right and then stays centred: the first frame of
//! [`SETTLE_RUN`] consecutive centred detections is the settle frame. From
//! there on one fixed box covers the name until the episode ends, even
//! where the text is lost on a bright background, plus [`FADE`] frames for
//! the fade-out. Before the settle frame the boxes follow the slide-in:
//! from the left end of the sliding name to the right edge of the plate,
//! which leads the name, or to the frame edge where no full plate shows.
//!
//! An episode whose name never settles (glare or a white sky can hide
//! the centred letters, port spec failure F4) but showed a name gets the
//! fallback box instead: centred and as wide as the widest possible name
//! ([`FALLBACK_HALF`]), from the slide-in to the end of the episode. A
//! banner name on screen is never left without a box.
//!
//! [`BannerTracker`] runs this model on a stream of frames with a bounded
//! delay: a frame's box is final once the settled box is known (about
//! half a second after the name stops), once the episode has ended, or at
//! the latest [`RING`] frames after the episode started, when an
//! unsettled episode switches to the fallback box. The tracker never
//! holds more than [`RING`] frames.

use std::collections::VecDeque;

use blurrd_core::{Confidence, RectI};

use super::{BannerFrame, BannerName, CENTRE_X, NAME_BAND, NAME_PAD_X, Plate};
use crate::killfeed::FEED_ROI;

/// Gated frames closer than this belong to one episode.
const EPISODE_GAP: u32 = 10;
/// Names count only once the gate has been on for this many frames of the
/// episode (port spec section 10, item 4): the real name follows the box
/// by about 23 frames, a short flicker of the gate on bright scenery does
/// not make an episode.
const GATE_MIN_FRAMES: u32 = 3;
/// The settled box stays this many frames after the last gated frame.
const FADE: u32 = 6;
/// Consecutive centred detections that mark the settle frame.
const SETTLE_RUN: u32 = 3;
/// The settled box is sized from the centred detections of this many
/// frames from the settle frame on. The prototype used the whole episode;
/// the widths are stable after the settle frame (one width in 75 to 98 %
/// of the centred frames of 25 measured banners), so half a second
/// gives the same box with a bounded delay.
const SETTLE_SPAN: u32 = 30;
/// For this many frames after the settle frame the name may still be
/// decelerating (the centred test allows 25 px)...
const DECELERATION_FRAMES: u32 = 8;
/// ... so the settled box reaches this much further right.
const DECELERATION_PAD: i32 = 30;
/// Walking back from the settle frame, the slide-in chain tolerates gaps
/// of at most this many frames...
const SLIDE_MAX_GAP: u32 = 4;
/// ... and detections at most this far left of the later one.
const SLIDE_BACK_TOLERANCE: i32 = 8;
/// The slide-in boxes start at least this many frames before the first
/// centred detection (`SLIDE_B`).
const SLIDE_LEAD: u32 = 12;
/// Slide-in boxes reach this far left of the detections...
const SLIDE_PAD_LEFT: i32 = 12;
/// ... and at least this far right (fast, faint letters), up to the
/// plate's right edge.
const SLIDE_PAD_RIGHT: i32 = 50;
/// The right frame edge: slide-in boxes end here where no full plate
/// shows the banner's right end.
const FRAME_X1: i32 = FEED_ROI.x1;
/// A plate at least this wide shows both of its ends (the full plate is
/// 340 to 440 px wide in 94 % of the plate frames of 25 recorded
/// banners; narrower ones are cut by effects or glare).
const PLATE_FULL_W: i32 = 340;
/// Without a centred name, the slide-in is taken to last this many frames
/// from the first plate frame (it takes 10 frames on recorded banners).
const SLIDE_FRAMES: u32 = 16;
/// An unsettled episode gets the fallback box only after a name, or a
/// plate in at least this many frames: a single plate-like frame in the
/// flicker after a banner (seen once in 31 recordings) is not a banner.
const FALLBACK_MIN_PLATES: u32 = 3;
/// The banner's own ring: the most frames of an episode the tracker holds
/// before deciding their boxes (plan 5.4: 240 frames, the analyzer's
/// lookahead cap). An episode that has not settled by then gets the
/// fallback box.
pub const RING: u32 = 240;
/// Half the width of the fallback box: the widest possible name, 16 × "W"
/// in the banner font, plus [`NAME_PAD_X`]. Measured on the banners of 23
/// recordings: the widest glyph ("M") is 12 px and glyph strokes are 4 px
/// apart. No recorded name contains a "W", which the font draws wider
/// than "M"; with 15 px (1.25 × "M") 16 of them span
/// 16 × 15 + 15 × 4 = 300 px.
pub const FALLBACK_HALF: i32 = 300 / 2 + NAME_PAD_X;

/// A banner box of one frame, in reference pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BannerBox {
    /// Presentation index of the frame.
    pub frame: u32,
    /// The box to blur.
    pub rect: RectI,
    /// How the box was found: [`Confidence::Tracked`] for the settled
    /// name, [`Confidence::PreRoll`] for the slide-in and
    /// [`Confidence::PostRoll`] for the fade-out.
    pub conf: Confidence,
}

/// What a pending frame showed, once the gate has been on long enough.
#[derive(Debug, Clone, Copy, Default)]
struct Seen {
    /// The banner name.
    name: Option<BannerName>,
    /// The plate.
    plate: Option<Plate>,
}

/// The episode being read.
#[derive(Debug, Clone)]
struct Episode {
    /// First gated frame.
    start: u32,
    /// Last gated frame so far.
    last_gate: u32,
    /// Gated frames so far.
    gated: u32,
    /// Consecutive centred detections ending at the latest frame.
    centred_run: u32,
    /// First frame with a centred detection.
    first_centred: Option<u32>,
    /// First frame with a plate once the gate has been on long enough.
    /// Plate and name appear together, so the slide-in starts here at the
    /// latest, even where effects interrupt the slide-in chain.
    first_plate: Option<u32>,
    /// The settle frame, once found.
    settle: Option<u32>,
    /// The settled box, once sized.
    settled: Option<RectI>,
    /// Whether the episode has the fallback box (it did not settle).
    fallback: bool,
    /// Half the widest name seen so far, plus the padding.
    widest_half: i32,
    /// Frames with a plate once the gate has been on long enough.
    plates: u32,
    /// Frames with a name once the gate has been on long enough.
    names: u32,
}

impl Episode {
    /// Whether a banner name was on screen: an unsettled episode then gets
    /// the fallback box.
    const fn showed_a_name(&self) -> bool {
        self.names > 0 || self.plates >= FALLBACK_MIN_PLATES
    }
}

/// Runs the banner settle model over a stream of frames.
///
/// Feed every frame of the video in presentation order with
/// [`BannerTracker::push`]; boxes come out in frame order as soon as they
/// are final. [`BannerTracker::finish`] flushes the end of the video.
#[derive(Debug, Clone, Default)]
pub struct BannerTracker {
    /// The frame expected next.
    next: Option<u32>,
    /// The first frame whose boxes are not final yet.
    from: u32,
    /// What frames `from..next` showed (only while an episode is open).
    pending: VecDeque<Seen>,
    episode: Option<Episode>,
}

impl BannerTracker {
    /// A tracker before the first frame.
    pub fn new() -> Self {
        Self::default()
    }

    /// Every frame before this one is final: its boxes have been emitted.
    pub const fn ready_until(&self) -> u32 {
        self.from
    }

    /// Adds what the detector saw in frame `frame` and appends the boxes
    /// that became final to `out`.
    ///
    /// Frames are expected in order without gaps. A frame that does not
    /// follow the previous one (a seek) ends the open episode first, as if
    /// the banner had disappeared.
    pub fn push(&mut self, frame: u32, seen: &BannerFrame, out: &mut Vec<BannerBox>) {
        if self.next.is_some_and(|next| next != frame) {
            self.end_episode(out);
        }
        self.next = Some(frame + 1);
        let episode = match &mut self.episode {
            Some(episode) => episode,
            None if seen.gate => {
                self.from = frame;
                self.episode.insert(Episode {
                    start: frame,
                    last_gate: frame,
                    gated: 0,
                    centred_run: 0,
                    first_centred: None,
                    first_plate: None,
                    settle: None,
                    settled: None,
                    fallback: false,
                    widest_half: 0,
                    plates: 0,
                    names: 0,
                })
            }
            None => {
                self.from = frame + 1;
                return;
            }
        };
        if seen.gate {
            episode.last_gate = frame;
            episode.gated += 1;
        }
        let confirmed = episode.gated >= GATE_MIN_FRAMES;
        let name = seen.name.filter(|_| confirmed);
        let plate = seen.plate.filter(|_| confirmed);
        if plate.is_some() {
            episode.first_plate.get_or_insert(frame);
            episode.plates += 1;
        }
        self.pending.push_back(Seen { name, plate });
        if let Some(n) = name {
            episode.names += 1;
            episode.widest_half = episode
                .widest_half
                .max((n.span.x1 - n.span.x0 + 1) / 2 + NAME_PAD_X);
        }
        if name.is_some_and(|n| n.is_centred()) {
            episode.centred_run += 1;
            episode.first_centred.get_or_insert(frame);
        } else {
            episode.centred_run = 0;
        }
        if episode.settle.is_none() && episode.centred_run >= SETTLE_RUN {
            episode.settle = Some(frame + 1 - SETTLE_RUN);
        }
        if frame - episode.last_gate > EPISODE_GAP {
            self.end_episode(out);
            return;
        }
        if let Some(settle) = episode.settle
            && episode.settled.is_none()
            && !episode.fallback
            && (frame + 1 >= settle + SETTLE_SPAN || self.pending.len() >= RING as usize)
        {
            self.settle(out);
        } else if episode.settle.is_none()
            && !episode.fallback
            && self.pending.len() >= RING as usize
        {
            if episode.showed_a_name() {
                self.fall_back();
            } else {
                // No banner name yet (a bright scene keeps the gate on):
                // only the slide-in lead can still get a box, the plate
                // frames seen so far count.
                let drop = self.pending.len() - (SLIDE_LEAD as usize + 1);
                self.pending.drain(..drop);
                self.from += drop as u32;
            }
        }
        self.emit_held(out);
    }

    /// Ends the video: emits the boxes of every remaining frame.
    pub fn finish(&mut self, out: &mut Vec<BannerBox>) {
        self.end_episode(out);
        self.next = None;
    }

    /// Ends the open episode at its last gated frame and emits the boxes of
    /// all pending frames.
    fn end_episode(&mut self, out: &mut Vec<BannerBox>) {
        if let Some(episode) = &self.episode
            && !episode.fallback
            && episode.settled.is_none()
        {
            if episode.settle.is_some() {
                self.settle(out);
            } else if episode.showed_a_name() {
                self.fall_back();
            }
        }
        self.emit_held(out);
        // What is left lies after the fade-out, or belongs to an episode
        // without a banner name: no boxes.
        self.from += self.pending.len() as u32;
        self.pending.clear();
        self.episode = None;
    }

    /// Sizes the settled box and emits the slide-in boxes of the frames
    /// before the settle frame.
    fn settle(&mut self, out: &mut Vec<BannerBox>) {
        let Some(episode) = &mut self.episode else {
            return;
        };
        let Some(settle) = episode.settle else {
            return;
        };
        let from = self.from;
        let seen_at = |f: u32| {
            f.checked_sub(from)
                .and_then(|i| self.pending.get(i as usize).copied())
                .unwrap_or_default()
        };
        let name_at = |f: u32| seen_at(f).name;
        // Half the median width of the centred names (rounded down, as the
        // prototype's `int(np.median(w / 2))`), plus the padding.
        let mut widths: Vec<i32> = (settle..settle + SETTLE_SPAN)
            .filter_map(name_at)
            .filter(BannerName::is_centred)
            .map(|n| n.span.x1 - n.span.x0)
            .collect();
        widths.sort_unstable();
        let m = widths.len() / 2;
        let median2 = match widths.len() {
            0 => 0,
            n if n % 2 == 1 => 2 * widths[m],
            _ => widths[m - 1] + widths[m],
        };
        let half = median2 / 4 + NAME_PAD_X;
        let settled = RectI {
            x0: CENTRE_X - half,
            y0: NAME_BAND.0,
            x1: CENTRE_X + half,
            y1: NAME_BAND.1,
        };
        episode.settled = Some(settled);

        // The slide-in chain: walking back from the settle frame, each
        // accepted name is right of (or at most a little left of) the
        // later one.
        let mut chain: Vec<(u32, RectI)> = Vec::new();
        let (mut x_ref, mut last) = (settled.x0, settle);
        let mut f = settle;
        while f > episode.start && last - (f - 1) <= SLIDE_MAX_GAP {
            f -= 1;
            if let Some(n) = name_at(f)
                && n.span.x0 >= x_ref - SLIDE_BACK_TOLERANCE
            {
                chain.push((f, n.span));
                x_ref = n.span.x0;
                last = f;
            }
        }
        let accepted = |g: u32| chain.iter().find(|(h, _)| *h == g).map(|(_, r)| *r);
        let mut start = chain.last().map_or(settle, |(a, _)| a.saturating_sub(1));
        if let Some(p) = episode.first_plate {
            start = start.min(p.saturating_sub(1));
        }
        if let Some(c) = episode.first_centred {
            start = start.min(c.saturating_sub(SLIDE_LEAD));
        }
        // The name never shows before the banner box: the slide-in starts
        // in the episode.
        let start = start.max(from);
        for g in start..settle {
            let near = (g.saturating_sub(1)..=g + 1)
                .filter_map(accepted)
                .reduce(|a, b| RectI {
                    x0: a.x0.min(b.x0),
                    x1: a.x1.max(b.x1),
                    ..a
                });
            let right = slide_right(
                seen_at(g).plate,
                [g.checked_sub(1), Some(g + 1)]
                    .into_iter()
                    .flatten()
                    .map(|h| seen_at(h).plate),
            );
            // The name slides in from the right and stops at the settled
            // position, so its letters never lie left of the settled box: a
            // part of the name read alone (a filter breaking the letters
            // apart) moves the box's left end no further right than that.
            let (x0, x1) = near.map_or((settled.x0, right), |r| {
                (
                    (r.x0 - SLIDE_PAD_LEFT).min(settled.x0),
                    right.max(r.x1 + SLIDE_PAD_RIGHT).min(FRAME_X1),
                )
            });
            out.push(BannerBox {
                frame: g,
                rect: RectI {
                    x0,
                    y0: NAME_BAND.0,
                    x1,
                    y1: NAME_BAND.1,
                },
                conf: Confidence::PreRoll,
            });
        }
        // Frames before the slide-in show the box without a name.
        let done = settle - from;
        self.pending.drain(..done as usize);
        self.from = settle;
    }

    /// Switches the open episode to the fallback box: the frames before
    /// the slide-in get no box, the others are emitted by
    /// [`Self::emit_held`].
    fn fall_back(&mut self) {
        let Some(episode) = &mut self.episode else {
            return;
        };
        episode.fallback = true;
        let mut start = episode
            .first_plate
            .map_or(u32::MAX, |p| p.saturating_sub(1));
        if let Some(c) = episode.first_centred {
            start = start.min(c.saturating_sub(SLIDE_LEAD));
        }
        let skip = start
            .saturating_sub(self.from)
            .min(self.pending.len() as u32);
        self.pending.drain(..skip as usize);
        self.from += skip;
    }

    /// Emits the held boxes (settled or fallback) of pending frames up to
    /// the end of the fade-out after the last gated frame.
    fn emit_held(&mut self, out: &mut Vec<BannerBox>) {
        let Some(episode) = &self.episode else {
            return;
        };
        let settled = episode.settle.zip(episode.settled);
        if settled.is_none() && !episode.fallback {
            return;
        }
        let mut previous: Option<Plate> = None;
        while self.from <= episode.last_gate + FADE
            && let Some(seen) = self.pending.pop_front()
        {
            let g = self.from;
            let (rect, conf) = match settled {
                Some((settle, settled)) if g < settle + DECELERATION_FRAMES => (
                    RectI {
                        x1: settled.x1 + DECELERATION_PAD,
                        ..settled
                    },
                    Confidence::Tracked,
                ),
                Some((_, settled)) => (settled, Confidence::Tracked),
                None => {
                    let next = self.pending.front().and_then(|s| s.plate);
                    fallback_box(episode, g, seen, slide_right(seen.plate, [previous, next]))
                }
            };
            previous = seen.plate;
            let conf = if g > episode.last_gate {
                Confidence::PostRoll
            } else {
                conf
            };
            out.push(BannerBox {
                frame: g,
                rect,
                conf,
            });
            self.from += 1;
        }
    }
}

/// The right end of a slide-in box in a frame with the plate `own` and
/// neighbouring frames with the plates `neighbours`: the right edge of
/// the plate, which leads the sliding name (the widest of the full plates
/// around, as the frames' timing may differ by one), or the frame edge
/// where the frame shows no full plate.
fn slide_right(own: Option<Plate>, neighbours: impl IntoIterator<Item = Option<Plate>>) -> i32 {
    let full_x1 = |p: Option<Plate>| p.filter(|p| p.x1 - p.x0 >= PLATE_FULL_W).map(|p| p.x1);
    full_x1(own).map_or(FRAME_X1, |x1| {
        neighbours
            .into_iter()
            .filter_map(full_x1)
            .fold(x1, i32::max)
            .min(FRAME_X1)
    })
}

/// The fallback box of frame `g` in an unsettled episode: centred and as
/// wide as the widest possible name (or the widest name seen), reaching
/// `right` ([`slide_right`]) while the name may still slide in (until the
/// first centred name, or [`SLIDE_FRAMES`] after the first plate), and
/// always covering the name found in the frame.
fn fallback_box(episode: &Episode, g: u32, seen: Seen, right: i32) -> (RectI, Confidence) {
    let half = FALLBACK_HALF.max(episode.widest_half);
    let mut rect = RectI {
        x0: CENTRE_X - half,
        y0: NAME_BAND.0,
        x1: CENTRE_X + half,
        y1: NAME_BAND.1,
    };
    let slide_end = episode
        .first_centred
        .or_else(|| episode.first_plate.map(|p| p + SLIDE_FRAMES));
    let sliding = slide_end.is_none_or(|end| g < end);
    if sliding {
        rect.x1 = rect.x1.max(right);
    }
    if let Some(n) = seen.name {
        rect.x0 = rect.x0.min(n.span.x0 - NAME_PAD_X);
        rect.x1 = rect.x1.max(n.span.x1 + NAME_PAD_X);
    }
    let conf = if sliding {
        Confidence::PreRoll
    } else {
        Confidence::WeakHold
    };
    (rect, conf)
}

#[cfg(test)]
mod tests {
    use blurrd_core::{Confidence, RectI};

    use super::{BannerBox, BannerTracker, FALLBACK_HALF, RING};
    use crate::banner::{BannerFrame, BannerName, Plate};

    /// A gated frame with a plate and a name at `x0..x1` (not centred
    /// unless it is symmetric around x 960).
    fn named(x0: i32, x1: i32) -> BannerFrame {
        let text = RectI {
            x0,
            y0: 306,
            x1,
            y1: 318,
        };
        let plate = Plate {
            x0: i32::midpoint(x0, x1) - 195,
            x1: i32::midpoint(x0, x1) + 195,
        };
        BannerFrame {
            gate: true,
            plate: Some(plate),
            name: Some(BannerName {
                text,
                span: text,
                ink: 200,
                plate,
            }),
        }
    }

    /// Pushes `frames` and returns the boxes per frame, checking the
    /// tracker's delay after every frame.
    fn run(frames: &[BannerFrame]) -> Vec<Vec<BannerBox>> {
        let mut tracker = BannerTracker::new();
        let mut out = vec![];
        for (f, seen) in frames.iter().enumerate() {
            let f = f as u32;
            tracker.push(f, seen, &mut out);
            assert!(f + 1 - tracker.ready_until() <= RING, "frame {f}");
        }
        tracker.finish(&mut out);
        let mut boxes = vec![vec![]; frames.len()];
        for b in out {
            boxes[b.frame as usize].push(b);
        }
        boxes
    }

    #[test]
    fn an_unsettled_banner_gets_the_fallback_box_with_a_bounded_delay() {
        // The box fades in, the name slides in and then is only ever seen
        // off-centre (glare over its left half) for ten seconds.
        let mut frames = vec![
            BannerFrame {
                gate: true,
                ..BannerFrame::default()
            };
            23
        ];
        frames.extend((0..10).map(|i| named(1180 - 25 * i, 1250 - 25 * i)));
        frames.extend((0..600).map(|_| named(965, 995)));
        frames.extend([BannerFrame::default(); 30]);
        let boxes = run(&frames);
        assert!(boxes[..22].iter().all(Vec::is_empty), "before the plate");
        for (f, b) in boxes.iter().enumerate().take(633).skip(22) {
            assert_eq!(b.len(), 1, "frame {f}");
            let r = b[0].rect;
            assert!(
                r.x0 <= 960 - FALLBACK_HALF && r.x1 >= 960 + FALLBACK_HALF,
                "frame {f}"
            );
            if let Some(n) = frames[f].name {
                assert!(r.x0 < n.text.x0 && r.x1 > n.text.x1, "frame {f}");
            }
            assert_ne!(b[0].conf, Confidence::Tracked);
        }
        // Six frames of fade-out, then nothing.
        assert!(
            boxes[633..639]
                .iter()
                .all(|b| b[0].conf == Confidence::PostRoll)
        );
        assert!(boxes[639..].iter().all(Vec::is_empty));
    }

    #[test]
    fn a_bright_scene_without_a_plate_gets_no_box_and_no_backlog() {
        let frames = vec![
            BannerFrame {
                gate: true,
                ..BannerFrame::default()
            };
            2000
        ];
        assert!(run(&frames).iter().all(Vec::is_empty));
    }

    #[test]
    fn a_lone_plate_frame_in_a_flicker_gets_no_box() {
        let mut frames = vec![
            BannerFrame {
                gate: true,
                ..BannerFrame::default()
            };
            14
        ];
        frames[11].plate = Some(Plate { x0: 800, x1: 1040 });
        frames.extend([BannerFrame::default(); 20]);
        assert!(run(&frames).iter().all(Vec::is_empty));
    }

    #[test]
    fn slide_in_boxes_reach_the_plate_edge_or_the_frame_edge() {
        let mut frames = vec![
            BannerFrame {
                gate: true,
                ..BannerFrame::default()
            };
            23
        ];
        frames.extend((0..10).map(|i| named(1180 - 25 * i, 1250 - 25 * i)));
        // An effect hides the plate of one slide-in frame.
        frames[27].plate = None;
        frames.extend((0..100).map(|_| named(930, 990)));
        let boxes = run(&frames);
        // The lead before the name has no plate: up to the frame edge.
        for b in &boxes[21..23] {
            assert_eq!(b[0].rect.x1, 1920);
        }
        for f in 23..33 {
            let b = boxes[f][0];
            assert_eq!(b.conf, Confidence::PreRoll, "frame {f}");
            let name = frames[f].name.unwrap();
            assert!(b.rect.x0 < name.text.x0, "frame {f}");
            let edge = match frames[f].plate {
                None => 1920,
                Some(_) => frames[f - 1..=f + 1]
                    .iter()
                    .filter_map(|n| n.plate.map(|p| p.x1))
                    .max()
                    .unwrap(),
            };
            assert_eq!(b.rect.x1, edge, "frame {f}");
        }
    }

    #[test]
    fn a_name_read_in_parts_before_it_settles_keeps_its_left_end_covered() {
        // A sharpening filter breaks the centred name (x 900..1020) apart:
        // for half a second only its right part is read, so the banner
        // settles late and those frames are part of the slide-in.
        let mut frames = vec![
            BannerFrame {
                gate: true,
                ..BannerFrame::default()
            };
            23
        ];
        frames.extend((0..10).map(|i| named(1180 - 25 * i, 1250 - 25 * i)));
        frames.extend((0..30).map(|_| named(975, 1018)));
        frames.extend((0..60).map(|_| named(900, 1020)));
        let boxes = run(&frames);
        for (f, b) in boxes.iter().enumerate().take(63).skip(33) {
            assert_eq!(b[0].conf, Confidence::PreRoll, "frame {f}");
            assert!(
                b[0].rect.x0 < 900 && b[0].rect.x1 > 1020,
                "frame {f}: {b:?}"
            );
        }
        assert_eq!(boxes[100][0].conf, Confidence::Tracked);
    }

    #[test]
    fn a_settling_banner_keeps_the_tracked_box() {
        let mut frames = vec![
            BannerFrame {
                gate: true,
                ..BannerFrame::default()
            };
            23
        ];
        frames.extend((0..10).map(|i| named(1180 - 25 * i, 1250 - 25 * i)));
        frames.extend((0..100).map(|_| named(930, 990)));
        let boxes = run(&frames);
        let settled = &boxes[60];
        assert_eq!(settled.len(), 1);
        assert_eq!(settled[0].conf, Confidence::Tracked);
        assert_eq!(settled[0].rect.x0 + settled[0].rect.x1, 1920);
    }
}
