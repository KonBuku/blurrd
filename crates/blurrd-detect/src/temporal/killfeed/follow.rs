//! Following one name template through the frames (prototype `_track`).

use blurrd_core::RectI;

use crate::image::GrayImage;
use crate::killfeed::ROW_PITCH;
use crate::track::ncc::{Match, Searcher, Template};

use super::STRIP;
use super::ring::{HALF_HEIGHT, HALF_WIDTH};

/// Score that keeps a name tracked (`TH`).
const TH: f32 = 0.55;
/// Score of a weak continuation in a ±1 px window while a name fades out
/// (`TH_WEAK`).
const TH_WEAK: f32 = 0.30;
/// Score that re-acquires a name after a gap (`TH_RE`).
const TH_RE: f32 = 0.70;
/// Score of a slide-in match before a name's first frame (`TH_SLIDE`).
const TH_SLIDE: f32 = 0.5;
/// A jump to another row needs at least this score (`TH_JUMP`) ...
const TH_JUMP: f32 = 0.7;
/// ... and must beat the score at the old place by this much
/// (`JUMP_MARGIN`).
const JUMP_MARGIN: f32 = 0.15;
/// Below this score at the old place, the whole window is searched for a
/// stack jump (`TH_STAY`).
const TH_STAY: f32 = 0.75;
/// Weak continuations in a row before the name counts as lost
/// (`WEAK_RUN`).
const WEAK_RUN: u32 = 8;
/// The "stay" search: at most ±3 px around the last position.
const STAY: i32 = 3;
/// Misses after which the jump search is no longer tried.
const JUMP_MISSES: u32 = 3;
/// A stay match at most this far away is sub-pixel flicker when the old
/// position scores nearly as well.
const FLICKER_PX: i32 = 1;
/// Text whose ends lie within this many pixels of a name's text has the
/// name's extent; a detection beyond it is another name.
const SAME_EXTENT: i32 = 5;
/// "Nearly as well" for sub-pixel flicker.
const FLICKER_MARGIN: f32 = 0.03;
/// The slide-in search continues at most this many matches ...
const SLIDE_MATCHES: u32 = 6;
/// ... and gives up after this many misses before the first one (the
/// cover bar hides the text for a frame or two after the slide).
const SLIDE_MISSES: u32 = 4;
/// A slide-in match must lie left of the previous one by more than this.
const SLIDE_MIN_DX: i32 = 3;
/// A row slides in by at most this much in total (the synthetic rows by
/// 60 px; recorded rows fade in at their place after the cover bar). A
/// match further left is another row's text that sat in the slot before
/// the stack moved.
const SLIDE_MAX_TOTAL: i32 = 120;

/// Search ranges per frame, relative to the previous position:
/// `(dx range, dy range)`, inclusive.
pub(super) type Window = ((i32, i32), (i32, i32));

/// Forward in time rows only move up (a stack jump), by up to one pitch
/// plus a little (`WIN_K_FWD`).
pub(super) const WINDOW_FORWARD: Window = ((-3, 3), (-42, 3));
/// Backward in time a name sits lower before a stack jump (`WIN_K_BWD`).
pub(super) const WINDOW_BACKWARD: Window = ((-3, 3), (-3, 42));
/// New rows slide in from the left by about 20 to 35 px per frame
/// (`SLIDE_K`); rows slide horizontally only (±2 px) ...
const SLIDE: Window = ((-90, 3), (-2, 2));
/// ... unless the stack jumped up while the row slid in: then the row sat
/// one pitch lower in the earlier frame.
const SLIDE_JUMPED: Window = ((-90, 3), (ROW_PITCH - 2, ROW_PITCH + 2));

/// Frames a name may be lost (flash, orb, kill streak) and re-acquired
/// (`MAX_GAP`).
pub(super) const MAX_GAP: u32 = 45;
/// Backward in time a gap before the first detection is no row yet
/// (`MAX_GAP_BWD`).
pub(super) const MAX_GAP_BACKWARD: u32 = 3;

/// What a follower reads of one frame.
#[derive(Debug, Clone, Copy)]
pub(super) struct Frame<'a> {
    /// Frame index.
    pub(super) index: u32,
    /// The `min(R, G, B)` plane of the feed strip.
    pub(super) plane: &'a GrayImage,
    /// The strip's colours at half resolution ([`super::ring::HALF_WIDTH`]
    /// × [`super::ring::HALF_HEIGHT`], packed R, G, B).
    pub(super) colours: &'a [u8],
    /// The text bounds of the names the detector found in the frame
    /// (reference pixels).
    pub(super) names: &'a [RectI],
}

impl Frame<'_> {
    /// The colour at `(hx, hy)` of the half-resolution plane, if inside.
    pub(super) fn colour(&self, hx: i32, hy: i32) -> Option<[u8; 3]> {
        if hx < 0 || hy < 0 || hx >= HALF_WIDTH as i32 || hy >= HALF_HEIGHT as i32 {
            return None;
        }
        let i = 3 * (hy as usize * HALF_WIDTH as usize + hx as usize);
        let c = self.colours.get(i..i + 3)?;
        Some([c[0], c[1], c[2]])
    }
}

/// How a frame's position was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum How {
    /// The template matched (`'m'`, and the reference frame itself).
    Matched,
    /// The row slid in from the left before the name's first match
    /// (`'s'`).
    Slide,
    /// A weak match in a ±1 px window while the name fades out (`'w'`).
    Weak,
    /// A gap between two matches: the union of both positions (`'g'`).
    Gap,
    /// After the last match, while the row's structure is still on
    /// screen: the union of the last match and the place the fading row
    /// slid to ([`super::end`]).
    Hold,
}

/// Template positions of one frame: the top-left corners span
/// `(x0, y0)..=(x1, y1)` (a gap holds the union of two positions).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct Pos {
    pub(super) x0: i32,
    pub(super) y0: i32,
    pub(super) x1: i32,
    pub(super) y1: i32,
    pub(super) how: How,
}

impl Pos {
    fn at((x, y): (i32, i32), how: How) -> Self {
        Self {
            x0: x,
            y0: y,
            x1: x,
            y1: y,
            how,
        }
    }

    /// The union of two positions, found as `how`.
    pub(super) fn union(a: (i32, i32), b: (i32, i32), how: How) -> Self {
        Self {
            x0: a.0.min(b.0),
            y0: a.1.min(b.1),
            x1: a.0.max(b.0),
            y1: a.1.max(b.1),
            how,
        }
    }
}

/// The state of following one template in one direction.
#[derive(Debug, Clone)]
pub(super) struct Follower {
    /// The name's text relative to the template's top-left corner.
    text: (i32, i32, i32, i32),
    pos: (i32, i32),
    last: u32,
    miss: u32,
    weak: u32,
    forward: bool,
    window: Window,
    max_gap: u32,
    done: bool,
}

impl Follower {
    /// Starts at the template's position `pos` in frame `start`; `text` is
    /// the name's text relative to the template's top-left corner.
    pub(super) fn new(
        text: (i32, i32, i32, i32),
        pos: (i32, i32),
        start: u32,
        forward: bool,
    ) -> Self {
        let (window, max_gap) = if forward {
            (WINDOW_FORWARD, MAX_GAP)
        } else {
            (WINDOW_BACKWARD, MAX_GAP_BACKWARD)
        };
        Self {
            text,
            pos,
            last: start,
            miss: 0,
            weak: 0,
            forward,
            window,
            max_gap,
            done: false,
        }
    }

    /// The name's text relative to the template's top-left corner.
    pub(super) const fn text(&self) -> (i32, i32, i32, i32) {
        self.text
    }

    /// Whether the name was lost for good.
    pub(super) const fn is_done(&self) -> bool {
        self.done
    }

    /// Stops following.
    pub(super) fn stop(&mut self) {
        self.done = true;
    }

    /// The last frame with a position.
    pub(super) const fn last(&self) -> u32 {
        self.last
    }

    /// Searches `frame` (the next frame in the follower's direction) and
    /// reports the positions it decides through `emit` (the frame itself,
    /// and the frames of a gap it closes).
    pub(super) fn step(
        &mut self,
        frame: &Frame<'_>,
        tpl: &Template,
        searcher: &mut Searcher,
        mut emit: impl FnMut(u32, Pos),
    ) {
        if self.done {
            return;
        }
        let (f, img) = (frame.index, frame.plane);
        let (win, pos) = (self.window, self.pos);
        let own = |m: &Match| !self.conflicts((m.x, m.y), frame.names);
        // Stay in place while the name still matches there; only then look
        // for a stack jump (an identical name in the row above or below must
        // not pull the box away from its own row).
        let stay = (
            (win.0.0.max(-STAY), win.0.1.min(STAY)),
            (win.1.0.max(-STAY), win.1.1.min(STAY)),
        );
        let mut found = searcher.find(img, tpl, pos, stay.0, stay.1).filter(own);
        if let Some(m) = found
            && (m.x, m.y) != pos
            && (m.x - pos.0).abs() <= FLICKER_PX
            && (m.y - pos.1).abs() <= FLICKER_PX
            && let Some(at) = searcher.find(img, tpl, pos, (0, 0), (0, 0)).filter(own)
            && at.score >= m.score - FLICKER_MARGIN
        {
            // Sub-pixel flicker between two positions: keep the box where it
            // was.
            found = Some(at);
        }
        let stay_score = found.map(|m| m.score);
        if stay_score.is_none_or(|s| s < TH_STAY) && stay != win && self.miss <= JUMP_MISSES {
            // The text at the old place is no longer clearly this name (it
            // fades, is washed out, or the stack jumped and another row's
            // name sits there now): jump only if the name is clearly better
            // elsewhere.
            if let Some(j) = searcher.find(img, tpl, pos, win.0, win.1).filter(own) {
                let base = stay_score.unwrap_or(-1.0).max(0.0);
                if (j.y - pos.1).abs() <= STAY
                    || (j.score >= TH_JUMP && j.score >= base + JUMP_MARGIN)
                {
                    found = Some(j);
                }
            }
        }
        let threshold = if self.miss == 0 { TH } else { TH.max(TH_RE) };
        if let Some(m) = found
            && m.score >= threshold
        {
            let p = (m.x, m.y);
            if self.miss > 0 {
                // Re-acquired after a gap: hold the union of both positions
                // over the gap.
                for g in self.between(f) {
                    emit(g, Pos::union(pos, p, How::Gap));
                }
            }
            emit(f, Pos::at(p, How::Matched));
            (self.pos, self.last, self.miss, self.weak) = (p, f, 0, 0);
            return;
        }
        if self.miss == 0 && self.weak < WEAK_RUN && self.forward {
            // Weak hold: the name fades out (forward in time only). The stack
            // can still jump up while it fades: the faded name one pitch up
            // wins over whatever now sits at the old place if it matches
            // clearly better.
            let held = searcher
                .find(img, tpl, pos, (-1, 1), (-1, 1))
                .filter(|w| w.score >= TH_WEAK && own(w));
            let jumped = searcher
                .find(
                    img,
                    tpl,
                    (pos.0, pos.1 - ROW_PITCH),
                    (-STAY, STAY),
                    (-STAY, STAY),
                )
                .filter(|j| {
                    j.score >= TH_WEAK
                        && j.score >= held.map_or(0.0, |w| w.score) + JUMP_MARGIN
                        && own(j)
                });
            if let Some(w) = jumped.or(held) {
                let p = (w.x, w.y);
                emit(f, Pos::at(p, How::Weak));
                (self.pos, self.last) = (p, f);
                self.weak += 1;
                return;
            }
        }
        self.miss += 1;
        if self.miss > self.max_gap {
            self.done = true;
        }
    }

    /// Whether the name's text at template position `p` lies on the text
    /// of another name the detector found in the frame: a name of a
    /// different extent, such as a longer name ending in the same letters
    /// that moved into the old place of a fading row (names are
    /// right-aligned). The template then matches the wrong name.
    fn conflicts(&self, p: (i32, i32), names: &[RectI]) -> bool {
        let t = self.text;
        let text = RectI {
            x0: STRIP.x0 + p.0 + t.0,
            y0: STRIP.y0 + p.1 + t.1,
            x1: STRIP.x0 + p.0 + t.2,
            y1: STRIP.y0 + p.1 + t.3,
        };
        names.iter().any(|n| {
            n.intersection(&text).is_some()
                && ((n.x0 - text.x0).abs() > SAME_EXTENT || (n.x1 - text.x1).abs() > SAME_EXTENT)
        })
    }

    /// Frames strictly between the last position and `f`.
    fn between(&self, f: u32) -> impl Iterator<Item = u32> + use<> {
        let (a, b) = if self.forward {
            (self.last + 1, f)
        } else {
            (f + 1, self.last)
        };
        a..b
    }

    /// The slide-in search before a name's first frame (backward pass
    /// only): the row enters from the left, and a cover bar hides the text
    /// for a frame or two in between. Continues from the earliest position
    /// over frames `first − 1` down to `lo`, reading them with `frame`.
    pub(super) fn slide_in<'a>(
        &mut self,
        lo: u32,
        tpl: &Template,
        searcher: &mut Searcher,
        frame: impl Fn(u32) -> Option<Frame<'a>>,
        mut emit: impl FnMut(u32, Pos),
    ) {
        let (mut f, mut miss, mut matches) = (self.last, 0, 0);
        let start = self.pos.0;
        while matches < SLIDE_MATCHES && miss < SLIDE_MISSES {
            if f <= lo {
                break;
            }
            f -= 1;
            let Some(fr) = frame(f) else { break };
            // Slid horizontally, or one pitch lower before a stack jump:
            // the better of the two.
            let found = [SLIDE, SLIDE_JUMPED]
                .into_iter()
                .filter_map(|w| searcher.find(fr.plane, tpl, self.pos, w.0, w.1))
                .filter(|m| {
                    m.score >= TH_SLIDE
                        && m.x < self.pos.0 - SLIDE_MIN_DX
                        && m.x >= start - SLIDE_MAX_TOTAL
                        && !self.conflicts((m.x, m.y), fr.names)
                })
                .max_by(|a, b| a.score.total_cmp(&b.score));
            match found {
                Some(m) => {
                    let p = (m.x, m.y);
                    for g in self.between(f) {
                        emit(g, Pos::union(self.pos, p, How::Gap));
                    }
                    emit(f, Pos::at(p, How::Slide));
                    (self.pos, self.last, miss) = (p, f, 0);
                    matches += 1;
                }
                // Before its slide the row's content was not there: a miss
                // after a slide match ends the search.
                None if matches > 0 => break,
                None => miss += 1,
            }
        }
    }
}
