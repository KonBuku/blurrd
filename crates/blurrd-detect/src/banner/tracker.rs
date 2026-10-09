//! The banner settle model (port spec section 4, `_banner_parts`).
//!
//! A banner episode is a run of frames with the banner gate on, merged
//! over gaps of at most [`EPISODE_GAP`] frames. Inside an episode the name
//! slides in from the right and then stays centred: the first frame of
//! [`SETTLE_RUN`] consecutive centred detections is the settle frame. From
//! there on one fixed box covers the name until the episode ends, even
//! where the text is lost on a bright background, plus [`FADE`] frames for
//! the fade-out. Before the settle frame the boxes follow the slide-in.
//!
//! [`BannerTracker`] runs this model on a stream of frames with a bounded
//! delay: a frame's box is final once the settled box is known (about
//! half a second after the name stops) or once the episode has ended.

use std::collections::VecDeque;

use blurrd_core::{Confidence, RectI};

use super::{BannerFrame, BannerName, CENTRE_X, NAME_BAND, NAME_PAD_X};

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
/// ... and this far right (fast, faint letters)...
const SLIDE_PAD_RIGHT: i32 = 50;
/// ... but not beyond this column (the stored banner strip's end).
const SLIDE_X1_MAX: i32 = 1420;

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
    /// The names of frames `from..next` (only while an episode is open).
    pending: VecDeque<Option<BannerName>>,
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
        if confirmed && seen.plate.is_some() {
            episode.first_plate.get_or_insert(frame);
        }
        self.pending.push_back(name);
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
            && frame + 1 >= settle + SETTLE_SPAN
        {
            self.settle(out);
        }
        self.emit_settled(out);
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
            && episode.settle.is_some()
            && episode.settled.is_none()
        {
            self.settle(out);
        }
        self.emit_settled(out);
        // What is left lies after the fade-out, or belongs to an episode
        // that never settled: no boxes.
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
        let name_at = |f: u32| self.pending.get((f - from) as usize).copied().flatten();
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
            let (x0, x1) = near.map_or((settled.x0, SLIDE_X1_MAX), |r| {
                (
                    r.x0 - SLIDE_PAD_LEFT,
                    (r.x1 + SLIDE_PAD_RIGHT).min(SLIDE_X1_MAX),
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

    /// Emits the settled boxes of pending frames up to the end of the
    /// fade-out after the last gated frame.
    fn emit_settled(&mut self, out: &mut Vec<BannerBox>) {
        let Some(episode) = &self.episode else {
            return;
        };
        let (Some(settle), Some(settled)) = (episode.settle, episode.settled) else {
            return;
        };
        while !self.pending.is_empty() && self.from <= episode.last_gate + FADE {
            let g = self.from;
            let rect = if g < settle + DECELERATION_FRAMES {
                RectI {
                    x1: settled.x1 + DECELERATION_PAD,
                    ..settled
                }
            } else {
                settled
            };
            let conf = if g > episode.last_gate {
                Confidence::PostRoll
            } else {
                Confidence::Tracked
            };
            out.push(BannerBox {
                frame: g,
                rect,
                conf,
            });
            self.pending.pop_front();
            self.from += 1;
        }
    }
}
