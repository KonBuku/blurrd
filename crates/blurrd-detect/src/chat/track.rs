//! Chat lines across frames: the colour and grid priors of the port spec
//! (section 10, items 1 and 2) in time.
//!
//! The per-frame reading misses lines in a few kinds of frames: the first
//! frames after a line pops in, frames where the background behind the box
//! is so bright that the strokes vanish, and the fade-out, where the text
//! and its part of the box fade together over about 0.3 s (12 to 23 frames
//! measured on recordings). [`ChatTracker`] follows every line that was
//! read once and keeps covering it while it is shown:
//!
//! - **Grid**: a line keeps its place on the line grid. When a new message
//!   arrives, every older line moves up by its height: 29 px for a
//!   one-line message, 47 px for a wrapped one. The tracker sees the move
//!   in the box top, which rises by that height within a frame or two, or,
//!   in a full chat whose oldest line leaves as the new one arrives, in
//!   lines read one step higher with the same name ends, or in another name
//!   read at the place of a line that is no longer read (a fading line
//!   pushed up by a new message). Lines read where they were overrule a box
//!   top that only seemed to rise; the darkness that makes a box part solid
//!   enough to count is learned per video, so filters that lighten the box
//!   do not hide pushes. A fading line that is no longer read thus stays
//!   covered where it went.
//! - **Hold**: a line that is not read in a frame is covered with the
//!   extent it was read with while it lies in a box band, while the box's
//!   edge ratio at its rows stays below 0.97 as the band fades (plan 7.3),
//!   and, for up to [`FADE_MAX`] frames after the band, while the strokes
//!   of its name or of its whole line (the white message keeps its
//!   contrast in greyscale video) stand out from the empty rows above it;
//!   then for [`POST_ROLL`] more frames. A line not read for more than
//!   [`HOLD_MAX`] frames is covered for the widest name from its start
//!   (the Safe slot box of the health rule, plan 7.3). A line that ended
//!   and is read again at its place with the same name end soon after (a
//!   flash or a black frame hid it) resumes and is covered in the frames
//!   between.
//! - **Backfill**: a line read for the first time is also covered in the
//!   frames before, back to the frame its message arrived in or up to two
//!   frames before its part of the box appeared, at most [`BACKFILL_MAX`]
//!   frames.
//!
//! The extent of a line comes from all its readings: per end, the
//! outermost value of the most precise ways of finding it that make up at
//! least half of the readings (a colon seen in most frames decides the
//! end, so a frame that fell back to Safe geometry does not widen the box
//! for the whole life of the line; a value read once is left out once
//! others were read repeatedly), and the text rows read most often. A
//! frame's own reading is covered too, except for a Safe guess of an end
//! or start that the line's readings know.
//!
//! Frames come out [`BACKFILL_MAX`] frames after they went in, half of
//! the analysis lookahead limit (plan 5.4, `LOOKAHEAD_MAX`).

use std::collections::VecDeque;

use blurrd_core::RectI;

use crate::detection::Detection;

use super::line::SAFE_NAME_WIDTH;
use super::{ChatDetector, ChatLine, INK_X, NameEnd, NameStart, SEARCH_Y, name_detection};

/// The chat box is visible at a line while the median edge ratio over the
/// line's rows is below this (plan 7.3: the chat ends when the band ratio
/// is 0.97 or more again).
const PRESENT_RATIO: f32 = 0.97;
/// Rows from a line's top over which its part of the box is measured: the
/// text rows of one line.
const PRESENCE_ROWS: i32 = 14;
/// A line that is not read is covered with the extent it was read with
/// for at most this many frames after it was last read (twice the ±15
/// frames of the port spec: the fade alone takes up to 23 frames), then
/// for the widest name.
pub const HOLD_MAX: u32 = 30;
/// Frames a line stays covered after its part of the box was last seen.
/// The text outlasts the box edge by at most 3 frames on recordings; this
/// also bridges frames where a bright background hides the box edge.
pub const POST_ROLL: u32 = 6;
/// Frames the text of a line can outlast its part of the box while both
/// fade (the whole fade takes up to 23 frames on recordings): the line is
/// held while its text is seen for this long after the box.
pub const FADE_MAX: u32 = 24;
/// The text of a fading line is seen while this fraction of its text
/// bounds are stroke pixels ...
const FADE_INK: f32 = 0.04;
/// ... at least this many times the fraction in the empty rows above it.
const TEXT_CONTRAST: f32 = 2.0;
/// The empty rows above a line's text: this many ...
const GAP_ROWS: i32 = 5;
/// ... ending this far above the text (accents).
const TEXT_GAP: i32 = 2;
/// A line read for the first time is covered in at most this many frames
/// before, while its part of the box was there: pop-ins are missed for up
/// to 5 frames on recordings, and a line in downscaled video was first
/// read 25 frames after its box showed (720p). Frames leave the tracker
/// this many frames after they arrive.
pub const BACKFILL_MAX: usize = 120;
/// A line that ended is resumed if it is read again at its place at most
/// this many frames after it was last seen.
const RESUME_MAX: u32 = 15;
/// Frames before a line's part of the box appears that are still covered
/// when it pops in: the box takes a frame or two to show.
const POP_IN: u32 = 2;
/// Distance between the tops of two one-line messages.
const LINE_PITCH: i32 = 29;
/// Extra height of a wrapped message (its continuation line).
const WRAP_PITCH: i32 = 18;
/// What can arrive in one frame and how far it moves the older lines: one
/// message of one or two lines, or two one-line messages.
const GRID_STEPS: [i32; 3] = [LINE_PITCH, LINE_PITCH + WRAP_PITCH, 2 * LINE_PITCH];
/// The box top's rise is a grid step if it is within this many rows of one
/// (the edge of a faint box is found a few rows off).
const STEP_TOLERANCE: i32 = 6;
/// The rise of the box top is measured over at most this many frames: a
/// box part appearing with a new message can take two frames to be found.
const PUSH_WINDOW: u32 = 3;
/// A box band is solid if its darkness (1 minus its edge ratio) is at
/// least this fraction of the darkness of a fully visible box: about 0.34
/// (ratio 0.66) on recordings, less where a filter lifts the shadows or
/// lowers the contrast (0.24 with brightness +0.06 and contrast 0.85). A
/// fading box part gets lighter until its ratio reaches 1.
const SOLID_FRACTION: f32 = 0.6;
/// The darkness of a fully visible box is the median of the darkest band
/// over this many recent frames with a box: robust to dark gameplay frames
/// (where the ratio drops far below the box's) and to fades.
const DARKNESS_FRAMES: usize = 60;
/// The lowest box band reaches at least this row when it holds the newest
/// line, and the box top is then the top of the first band.
const BOX_BOTTOM_MIN: i32 = 1015;
/// A line lies in a box band if its middle row is at most this many rows
/// outside it.
const BAND_TOLERANCE: i32 = 3;
/// A reading belongs to a line whose top is at most this many rows away.
const MATCH_ROWS: i32 = 6;
/// A line's readings know its end (or start) when it was found in most of
/// at least this many of them.
const CONFIDENT_READS: u32 = 10;
/// A start or end read at least this many times outweighs values read
/// fewer times by the same way of finding it.
const REPEATED_READS: u32 = 2;
/// Text tops and bottoms read for one line vary by this many rows.
const ROW_SPREAD: i32 = 3;
/// Readings of one line end within this many pixels of each other.
const SAME_END: i32 = 8;

/// What the readings of a line say about where it is.
#[derive(Debug, Clone)]
struct Model {
    /// Per way the start was found (after the prefix, or Safe after the
    /// shortest prefix): the starts read, with how often each was read.
    starts: [Vec<(i32, u32)>; 2],
    /// Per way the end was found ([`NameEnd`] in rank order: colon,
    /// colour, Safe): the ends read, with how often each was read.
    ends: [Vec<(i32, u32)>; 3],
    /// Text tops and bottoms read, in the rows of the frame the line was
    /// first read in, with how often each was read.
    tops: Vec<(i32, u32)>,
    bottoms: Vec<(i32, u32)>,
}

impl Model {
    fn new(line: &ChatLine) -> Self {
        let mut model = Self {
            starts: Default::default(),
            ends: Default::default(),
            tops: Vec::new(),
            bottoms: Vec::new(),
        };
        model.add(line, 0);
        model
    }

    /// Adds a reading of the line, which has moved up by `shift` rows
    /// since it was first read.
    fn add(&mut self, line: &ChatLine, shift: i32) {
        let kind = usize::from(line.start == NameStart::Shortest);
        count(&mut self.starts[kind], line.name.x0);
        let kind = match line.end {
            NameEnd::Colon => 0,
            NameEnd::Colour => 1,
            NameEnd::Safe => 2,
        };
        count(&mut self.ends[kind], line.name.x1);
        count(&mut self.tops, line.top + shift);
        count(&mut self.bottoms, line.bottom + shift);
    }

    /// The columns to cover: for each end, the outermost value of the most
    /// precise ways of finding it that make up at least half of the
    /// readings. A colon seen in most frames decides the end, so a frame
    /// that fell back to Safe geometry does not widen the box for the whole
    /// life of the line, while a colon seen in a few frames only (a broken
    /// glyph that looked like one) does not cut it short; the same holds
    /// for a start found after the prefix. Within one way, a value read
    /// once is left out once others were read repeatedly ([`outermost`]).
    fn columns(&self) -> (i32, i32) {
        let x0 = majority(&self.starts, i32::MAX, i32::min);
        let x1 = majority(&self.ends, i32::MIN, i32::max);
        (x0, x1)
    }

    /// Whether the colon was seen in most of at least [`CONFIDENT_READS`]
    /// readings: then a frame that misses it does not move the end.
    fn end_confident(&self) -> bool {
        confident(&self.ends)
    }

    /// Whether the start after the prefix was found in most of at least
    /// [`CONFIDENT_READS`] readings.
    fn start_confident(&self) -> bool {
        confident(&self.starts)
    }

    /// The top of the line's text after it moved up by `shift` rows: the
    /// highest top read near the most frequent one. Faint capitals are not
    /// read in every frame, and a glyph of the line above that touches the
    /// line is not part of it.
    fn top(&self, shift: i32) -> i32 {
        let usual = most_frequent(&self.tops);
        self.tops
            .iter()
            .map(|&(t, _)| t)
            .filter(|t| (t - usual).abs() <= ROW_SPREAD)
            .min()
            .unwrap_or(usual)
            - shift
    }

    /// The bottom of the line's text after it moved up by `shift` rows: the
    /// lowest bottom read near the most frequent one.
    fn bottom(&self, shift: i32) -> i32 {
        let usual = most_frequent(&self.bottoms);
        self.bottoms
            .iter()
            .map(|&(b, _)| b)
            .filter(|b| (b - usual).abs() <= ROW_SPREAD)
            .max()
            .unwrap_or(usual)
            - shift
    }

    /// The line's text bounds after it moved up by `shift` rows.
    fn text(&self, shift: i32) -> RectI {
        let (x0, x1) = self.columns();
        let top = self.top(shift);
        RectI {
            x0,
            y0: top,
            x1,
            y1: self.bottom(shift).max(top + 1),
        }
    }

    /// Whether `line` ends where this line does, within a few pixels.
    /// Starts are not compared: where the prefix runs into the name, the
    /// start is found a few pixels off.
    fn ends_alike(&self, line: &ChatLine) -> bool {
        (line.name.x1 - self.columns().1).abs() <= SAME_END
    }
}

/// The outermost value (`outer`) of the leading kinds in `kinds` (values
/// read and how often, most precise kind first) that make up at least half
/// of the readings.
fn majority(kinds: &[Vec<(i32, u32)>], none: i32, outer: fn(i32, i32) -> i32) -> i32 {
    let total: u32 = kinds.iter().map(|kind| reads(kind)).sum();
    let mut counted = 0;
    let mut value = none;
    for kind in kinds {
        counted += reads(kind);
        value = outermost(kind, value, outer);
        if 2 * counted >= total {
            break;
        }
    }
    value
}

/// The outermost (`outer`) of `value` and the values in `counts` read at
/// least [`REPEATED_READS`] times, or of all of them if none was: one
/// misreading (a prefix taken for the name, a stray colon) does not widen
/// the box for the whole life of the line.
fn outermost(counts: &[(i32, u32)], value: i32, outer: fn(i32, i32) -> i32) -> i32 {
    let repeated = counts.iter().any(|&(_, n)| n >= REPEATED_READS);
    counts
        .iter()
        .filter(|&&(_, n)| !repeated || n >= REPEATED_READS)
        .fold(value, |acc, &(v, _)| outer(acc, v))
}

/// How many readings `counts` holds.
fn reads(counts: &[(i32, u32)]) -> u32 {
    counts.iter().map(|&(_, n)| n).sum()
}

/// Whether the most precise kind makes up most of at least
/// [`CONFIDENT_READS`] readings.
fn confident(kinds: &[Vec<(i32, u32)>]) -> bool {
    let total: u32 = kinds.iter().map(|kind| reads(kind)).sum();
    let first = kinds.first().map_or(0, |kind| reads(kind));
    first >= CONFIDENT_READS && 2 * first >= total
}

/// Counts one more occurrence of `value`.
fn count(counts: &mut Vec<(i32, u32)>, value: i32) {
    match counts.iter_mut().find(|(v, _)| *v == value) {
        Some((_, n)) => *n += 1,
        None => counts.push((value, 1)),
    }
}

/// The most frequent value, the smallest of equally frequent ones.
fn most_frequent(counts: &[(i32, u32)]) -> i32 {
    counts
        .iter()
        .max_by_key(|&&(v, n)| (n, std::cmp::Reverse(v)))
        .map_or(0, |&(v, _)| v)
}

/// One chat line followed across frames.
#[derive(Debug, Clone, Copy)]
struct Track {
    id: u32,
    /// Rows the line moved up since it was first read.
    shift: i32,
    /// Last frame the line was read in.
    last_read: u32,
    /// Last frame it lay in a box band.
    last_band: u32,
    /// Last frame its part of the box was visible in.
    last_box: u32,
    /// Last frame its box part or, through the fade, its text was seen in.
    last_seen: u32,
}

impl Track {
    /// The line `id`, read in frame `frame` where it has not moved.
    const fn new(id: u32, frame: u32) -> Self {
        Self {
            id,
            shift: 0,
            last_read: frame,
            last_band: frame,
            last_box: frame,
            last_seen: frame,
        }
    }
}

/// A frame's own reading of a line.
#[derive(Debug, Clone, Copy)]
struct Reading {
    text: RectI,
    start: NameStart,
    end: NameEnd,
}

/// One line covered in one frame.
#[derive(Debug, Clone, Copy)]
struct Covered {
    /// The track.
    id: u32,
    /// Rows the line had moved up in that frame.
    shift: i32,
    /// The frame's own reading of the line, if it was read.
    own: Option<Reading>,
    /// Whether the line is covered for the widest name: it has not been
    /// read for more than [`HOLD_MAX`] frames.
    safe: bool,
}

/// A frame waiting for later frames that may still add to it.
#[derive(Debug, Clone)]
struct PendingFrame {
    frame: u32,
    /// The box's edge ratio per searched row ([`ChatDetector::edge_ratios`]).
    edges: Vec<f32>,
    /// Whether a new message arrived in this frame.
    arrived: bool,
    lines: Vec<Covered>,
}

/// Follows chat lines across the frames of one video and covers each line
/// in every frame it is shown in.
///
/// Push every frame's [`ChatDetector`] state in order with
/// [`push`](Self::push); [`drain_ready`](Self::drain_ready) returns the
/// final detections of frames that later frames can no longer change, and
/// [`finish`](Self::finish) the rest after the last frame.
#[derive(Debug, Clone, Default)]
pub struct ChatTracker {
    /// Live lines.
    tracks: Vec<Track>,
    /// The model of every line that is live or covered in a pending frame,
    /// by track id.
    models: Vec<(u32, Model)>,
    pending: VecDeque<PendingFrame>,
    next_id: u32,
    /// Recent box tops and their frames, oldest first.
    box_tops: VecDeque<(u32, i32)>,
    /// The darkness of the darkest band in recent frames with a box (see
    /// [`SOLID_FRACTION`] and [`DARKNESS_FRAMES`]), oldest first.
    darkness: VecDeque<f32>,
    /// Buffer for the median of `darkness`.
    darkness_sorted: Vec<f32>,
    /// Lines that ended but are still covered in a pending frame: one read
    /// again at its place resumes.
    ended: Vec<Track>,
}

impl ChatTracker {
    /// An empty tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds the state of `detector` after it read frame `frame`. Frames
    /// must be pushed in increasing order.
    pub fn push(&mut self, frame: u32, detector: &ChatDetector) {
        let arrived = self.follow_box_top(frame, detector);
        let (mut lines, read) = self.cover_read(frame, detector, arrived);
        self.hold_unread(frame, detector, &read, &mut lines);
        self.pending.push_back(PendingFrame {
            frame,
            edges: detector.edge_ratios().to_vec(),
            arrived,
            lines,
        });
    }

    /// Covers the lines read in this frame: each joins the live line at its
    /// place or starts a new one, backfilled. Returns the covered lines and
    /// which live lines were read.
    fn cover_read(
        &mut self,
        frame: u32,
        detector: &ChatDetector,
        arrived: bool,
    ) -> (Vec<Covered>, Vec<bool>) {
        let mut lines: Vec<Covered> = Vec::new();
        let mut read = vec![false; self.tracks.len()];
        for line in detector.lines() {
            let own = Some(Reading {
                text: line.name,
                start: line.start,
                end: line.end,
            });
            let mut nearest = (0..self.tracks.len())
                .filter(|&i| !read[i])
                .map(|i| (i, (self.top_of(&self.tracks[i]) - line.top).abs()))
                .filter(|&(_, d)| d <= MATCH_ROWS)
                .min_by_key(|&(_, d)| d)
                .map(|(i, _)| i);
            let mut replaced = None;
            if let Some(i) = nearest.filter(|&i| self.is_replaced(frame, &self.tracks[i], line)) {
                // Another message took the line's place: it arrived after
                // the line was last read and pushed the line up, unless a
                // line above was read where it was (then the old line had
                // faded out).
                if !read.iter().any(|&r| r) {
                    self.push_up(i, line.top);
                }
                replaced = Some(self.tracks[i].id);
                nearest = None;
            }
            let resumed = if nearest.is_none() {
                self.resume(frame, line)
            } else {
                None
            };
            if let Some(track) = resumed {
                nearest = Some(self.tracks.len());
                self.tracks.push(track);
                read.push(false);
            }
            let (id, shift) = if let Some(i) = nearest {
                read[i] = true;
                let track = &mut self.tracks[i];
                *track = Track {
                    shift: track.shift,
                    ..Track::new(track.id, frame)
                };
                let (id, shift) = (track.id, track.shift);
                if let Some(model) = self.model_mut(id) {
                    model.add(line, shift);
                }
                (id, shift)
            } else {
                let id = self.next_id;
                self.next_id = self.next_id.wrapping_add(1);
                self.models.push((id, Model::new(line)));
                self.backfill(id, line.top, arrived, replaced);
                self.tracks.push(Track::new(id, frame));
                read.push(true);
                (id, 0)
            };
            lines.push(Covered {
                id,
                shift,
                own,
                safe: false,
            });
        }
        (lines, read)
    }

    /// Covers the live lines not read in this frame: held while their part
    /// of the box is visible and through the fade after it while their text
    /// is still there, unless a line read in this frame took their place or
    /// they moved out of the chat. Lines no longer held end.
    fn hold_unread(
        &mut self,
        frame: u32,
        detector: &ChatDetector,
        read: &[bool],
        lines: &mut Vec<Covered>,
    ) {
        let edges = detector.edge_ratios();
        let mut alive = Vec::with_capacity(self.tracks.len());
        for (i, &was_read) in read.iter().enumerate() {
            if was_read {
                alive.push(true);
                continue;
            }
            let track = &self.tracks[i];
            let top = self.top_of(track);
            let taken = lines
                .iter()
                .any(|c| (self.covered_top(c) - top).abs() <= MATCH_ROWS);
            let in_band = in_band(detector, top);
            // After the band is gone, the edge ratio follows the box through
            // its fade, frame after frame, and the text can outlast both.
            let after_band = frame.saturating_sub(track.last_band) <= FADE_MAX;
            let in_box = in_band
                || (after_band && frame.saturating_sub(track.last_box) <= 1 && present(edges, top));
            let fading = !in_box
                && after_band
                && self
                    .model(track.id)
                    .is_some_and(|m| line_visible(detector, m.text(track.shift)));
            let track = &mut self.tracks[i];
            if in_band {
                track.last_band = frame;
            }
            if in_box {
                track.last_box = frame;
            }
            if in_box || fading {
                track.last_seen = frame;
            }
            let held =
                !taken && top >= SEARCH_Y.0 && frame.saturating_sub(track.last_seen) <= POST_ROLL;
            if held {
                lines.push(Covered {
                    id: track.id,
                    shift: track.shift,
                    own: None,
                    safe: frame.saturating_sub(track.last_read) > HOLD_MAX,
                });
            }
            alive.push(held);
        }
        let mut alive = alive.into_iter();
        let ended = &mut self.ended;
        self.tracks.retain(|track| {
            let held = alive.next().unwrap_or(false);
            if !held {
                ended.push(*track);
            }
            held
        });
    }

    /// Moves every line up when a new message arrives. Returns whether a
    /// message arrived in this frame and moved lines; a message that showed
    /// before this frame marks that frame as its arrival and covers the
    /// moved lines there too.
    ///
    /// A new message raises the box top by a grid step, within a frame or
    /// two. The box top alone also jumps when part of the box is not found
    /// in a frame (a background without contrast at its edges), so lines
    /// read at their old places with their old name ends overrule it. In a
    /// full chat the oldest line leaves as the new one arrives and the box
    /// top stays; there, lines read one step higher with their name ends
    /// show the move.
    fn follow_box_top(&mut self, frame: u32, detector: &ChatDetector) -> bool {
        let rise = self.box_rise(frame, detector);
        let read_at = |track: &Track, top: i32| {
            self.model(track.id).is_some_and(|model| {
                detector
                    .lines()
                    .iter()
                    .any(|l| (l.top - top).abs() <= MATCH_ROWS && model.ends_alike(l))
            })
        };
        let evidence = |shift: i32| {
            let moved = self
                .tracks
                .iter()
                .filter(|t| read_at(t, self.top_of(t) - shift))
                .count();
            let stayed = self
                .tracks
                .iter()
                .filter(|t| read_at(t, self.top_of(t)))
                .count();
            (moved, stayed)
        };
        let (shift, since) = if let Some((since, step)) = rise {
            let (moved, stayed) = evidence(step);
            // A rise that no line read in this frame confirms counts only
            // for a box in one piece: a separate band above it is gameplay
            // texture or a fading line, not a pushed one.
            if stayed > moved || (moved == 0 && detector.bands().len() > 1) {
                return false;
            }
            // Later rises are measured from the grown box.
            let grown = self.box_tops.back().copied();
            self.box_tops.clear();
            self.box_tops.extend(grown);
            (step, Some(since))
        } else {
            let best = GRID_STEPS
                .into_iter()
                .map(|step| (step, evidence(step)))
                .max_by_key(|&(_, (moved, _))| moved);
            match best {
                Some((step, (moved, stayed))) if moved > 0 && moved > stayed => {
                    // The message arrived after the last frame a line was
                    // read in.
                    let since = self
                        .pending
                        .iter()
                        .rev()
                        .find(|p| p.lines.iter().any(|c| c.own.is_some()))
                        .map(|p| p.frame);
                    (step, since)
                }
                _ => return false,
            }
        };
        if self.tracks.is_empty() {
            // The box just appeared, or its edge was found again: nothing
            // tells when a message arrived.
            return false;
        }
        for track in &mut self.tracks {
            track.shift += shift;
        }
        let Some(since) = since else {
            return true;
        };
        self.cover_moved(since, shift)
    }

    /// Covers the live lines, which moved up by `shift` rows, at their new
    /// places in the pending frames after `since`: those frames already
    /// showed the new message, which arrived in the first of them. Returns
    /// whether it arrived in the current frame instead (no pending frame
    /// follows `since`).
    fn cover_moved(&mut self, since: u32, shift: i32) -> bool {
        let mut first = None;
        for (k, pending) in self.pending.iter().enumerate().rev() {
            if pending.frame <= since {
                break;
            }
            first = Some(k);
        }
        let Some(first) = first else {
            return true;
        };
        self.pending[first].arrived = true;
        for k in first..self.pending.len() {
            let moved: Vec<Covered> = self.pending[k]
                .lines
                .iter()
                .filter(|c| self.tracks.iter().any(|t| t.id == c.id))
                .map(|c| Covered {
                    id: c.id,
                    shift: c.shift + shift,
                    own: None,
                    safe: c.safe,
                })
                .collect();
            for covered in moved {
                let top = self.covered_top(&covered);
                if !self.pending[k]
                    .lines
                    .iter()
                    .any(|c| (self.covered_top(c) - top).abs() <= MATCH_ROWS)
                {
                    self.pending[k].lines.push(covered);
                }
            }
        }
        false
    }

    /// Follows the box top. Returns the frame before a rise of the box top
    /// by a grid step and the step, if the box top rose.
    fn box_rise(&mut self, frame: u32, detector: &ChatDetector) -> Option<(u32, i32)> {
        let bands = detector.bands();
        // Learned from the darkest band, so that filters that lighten the
        // box do not make every band look faded.
        let darkest = bands.iter().map(|b| 1.0 - b.ratio).fold(0.0, f32::max);
        if !bands.is_empty() {
            if self.darkness.len() == DARKNESS_FRAMES {
                self.darkness.pop_front();
            }
            self.darkness.push_back(darkest);
        }
        let top = bands
            .first()
            .filter(|_| bands.last().is_some_and(|b| b.y1 >= BOX_BOTTOM_MIN))
            .map(|band| band.y0)?;
        // A new message comes with its part of the box fully dark; a fading
        // box breaks up into faint bands whose tops jump.
        let solid = darkest >= SOLID_FRACTION * self.full_darkness();
        while self
            .box_tops
            .front()
            .is_some_and(|&(f, _)| frame.saturating_sub(f) > PUSH_WINDOW)
        {
            self.box_tops.pop_front();
        }
        // The lowest box top of the window is where the box was before the
        // message arrived; frames after the last one with it were already
        // showing the message.
        let reference = self.box_tops.iter().copied().max_by_key(|&(f, t)| (t, f));
        self.box_tops.push_back((frame, top));
        let (since, low) = reference.filter(|_| solid)?;
        GRID_STEPS
            .into_iter()
            .find(|step| (step - (low - top)).abs() <= STEP_TOLERANCE)
            .map(|step| (since, step))
    }

    /// The darkness of a fully visible box: the median of the recent
    /// darkest bands.
    fn full_darkness(&mut self) -> f32 {
        let sorted = &mut self.darkness_sorted;
        sorted.clear();
        sorted.extend(self.darkness.iter().copied());
        if sorted.is_empty() {
            return 0.0;
        }
        let mid = sorted.len() / 2;
        *sorted.select_nth_unstable_by(mid, f32::total_cmp).1
    }

    /// Resumes the line that `line` ends like, if one ended at its place
    /// and was seen at most [`RESUME_MAX`] frames ago (a flash, a black
    /// frame or a background without contrast hid it): it is covered in the
    /// pending frames since it ended and returned to be followed again.
    fn resume(&mut self, frame: u32, line: &ChatLine) -> Option<Track> {
        let k = self.ended.iter().position(|track| {
            frame.saturating_sub(track.last_seen) <= RESUME_MAX
                && (self.top_of(track) - line.top).abs() <= MATCH_ROWS
                && self
                    .model(track.id)
                    .is_some_and(|model| model.ends_alike(line))
        })?;
        let track = self.ended.swap_remove(k);
        let since = self
            .pending
            .iter()
            .rposition(|p| p.lines.iter().any(|c| c.id == track.id))
            .map_or(0, |k| k + 1);
        for pending in self.pending.iter_mut().skip(since) {
            pending.lines.push(Covered {
                id: track.id,
                shift: track.shift,
                own: None,
                safe: false,
            });
        }
        Some(track)
    }

    /// Whether `line`, read in frame `frame` at the place of `track`, is
    /// another message: its name ends at a colon elsewhere than the colon
    /// the track's readings know, and the track was not read in the frame
    /// before (a colon misread in one frame of a line read throughout does
    /// not split it).
    fn is_replaced(&self, frame: u32, track: &Track, line: &ChatLine) -> bool {
        frame.saturating_sub(track.last_read) > 1
            && line.end == NameEnd::Colon
            && self
                .model(track.id)
                .is_some_and(|model| model.end_confident() && !model.ends_alike(line))
    }

    /// Moves the live lines up for a message read at row `top` in the place
    /// of the line `self.tracks[i]`: the line goes one line above the new
    /// message (a grid step up), the lines above it with it, and they are
    /// covered at their new places since the line was last read. Nothing
    /// moves if that is not a grid step.
    fn push_up(&mut self, i: usize, top: i32) {
        let shift = self.top_of(&self.tracks[i]) - (top - LINE_PITCH);
        if !GRID_STEPS
            .iter()
            .any(|step| (step - shift).abs() <= STEP_TOLERANCE)
        {
            return;
        }
        let since = self.tracks[i].last_read;
        for track in &mut self.tracks {
            track.shift += shift;
        }
        // The box top may still be growing with the new message: later
        // rises are measured from here, so the push is not counted twice.
        let latest = self.box_tops.back().copied();
        self.box_tops.clear();
        self.box_tops.extend(latest);
        self.cover_moved(since, shift);
    }

    /// Covers the line `id`, first read at row `top`, in the frames before,
    /// back to the frame its message arrived in (`arrived`: it arrived in
    /// the current frame) or up to [`POP_IN`] frames before its part of the
    /// box appeared. The line `replaced`, whose place the new line took, is
    /// not in the way.
    fn backfill(&mut self, id: u32, top: i32, arrived: bool, replaced: Option<u32>) {
        let mut arrived = arrived;
        let mut absent = 0;
        for k in (0..self.pending.len()).rev() {
            let pending = &self.pending[k];
            if arrived
                || pending.lines.iter().any(|c| {
                    Some(c.id) != replaced && (self.covered_top(c) - top).abs() <= MATCH_ROWS
                })
            {
                break;
            }
            if present(&pending.edges, top) {
                absent = 0;
            } else {
                absent += 1;
                if absent > POP_IN {
                    break;
                }
            }
            arrived = pending.arrived;
            self.pending[k].lines.push(Covered {
                id,
                shift: 0,
                own: None,
                safe: false,
            });
        }
    }

    fn model(&self, id: u32) -> Option<&Model> {
        self.models.iter().find(|(i, _)| *i == id).map(|(_, m)| m)
    }

    fn model_mut(&mut self, id: u32) -> Option<&mut Model> {
        self.models
            .iter_mut()
            .find(|(i, _)| *i == id)
            .map(|(_, m)| m)
    }

    /// The current top of a live line.
    fn top_of(&self, track: &Track) -> i32 {
        self.model(track.id)
            .map_or(i32::MIN / 2, |m| m.top(track.shift))
    }

    /// The top of a covered line in its frame.
    fn covered_top(&self, covered: &Covered) -> i32 {
        self.model(covered.id)
            .map_or(i32::MIN / 2, |m| m.top(covered.shift))
    }

    /// Moves the detections of frames that no later frame can change into
    /// `out`, oldest first, as `(frame, detections)`.
    pub fn drain_ready(&mut self, out: &mut Vec<(u32, Vec<Detection>)>) {
        while self.pending.len() > BACKFILL_MAX {
            self.emit_oldest(out);
        }
    }

    /// Moves the detections of all remaining frames into `out` after the
    /// last frame was pushed, and resets the tracker for another video.
    pub fn finish(&mut self, out: &mut Vec<(u32, Vec<Detection>)>) {
        while !self.pending.is_empty() {
            self.emit_oldest(out);
        }
        *self = Self::default();
    }

    fn emit_oldest(&mut self, out: &mut Vec<(u32, Vec<Detection>)>) {
        let Some(pending) = self.pending.pop_front() else {
            return;
        };
        let detections = pending
            .lines
            .iter()
            .filter_map(|covered| {
                let model = self.model(covered.id)?;
                let mut text = model.text(covered.shift);
                if covered.safe {
                    text.x1 = text.x1.max(text.x0 + SAFE_NAME_WIDTH as i32);
                }
                if let Some(own) = covered.own {
                    // The frame's own reading, except for the Safe guesses
                    // of an end or a start that the line's readings know.
                    let mut own_text = own.text;
                    if own.end != NameEnd::Colon && model.end_confident() {
                        own_text.x1 = own_text.x1.min(text.x1);
                    }
                    if own.start == NameStart::Shortest && model.start_confident() {
                        own_text.x0 = own_text.x0.max(text.x0);
                    }
                    text = union(text, own_text);
                }
                Some(name_detection(text))
            })
            .collect();
        out.push((pending.frame, detections));
        // Forget the models of lines no frame refers to any more.
        let (tracks, pending) = (&self.tracks, &self.pending);
        self.models.retain(|(id, _)| {
            tracks.iter().any(|t| t.id == *id)
                || pending.iter().any(|p| p.lines.iter().any(|c| c.id == *id))
        });
        let models = &self.models;
        let newest = self.pending.back().map_or(0, |p| p.frame);
        self.ended.retain(|track| {
            newest.saturating_sub(track.last_seen) <= RESUME_MAX
                && models.iter().any(|(id, _)| *id == track.id)
        });
    }
}

/// Whether the line whose text starts at row `top` lies in a box band of
/// the detector's last frame (its middle row, with the line finder's
/// tolerance).
fn in_band(detector: &ChatDetector, top: i32) -> bool {
    let middle = top + PRESENCE_ROWS / 2;
    detector
        .bands()
        .iter()
        .any(|b| b.y0 - BAND_TOLERANCE <= middle && middle < b.y1 + BAND_TOLERANCE)
}

/// Whether a line whose name has text bounds `name` is still visible in
/// the detector's last frame: its name, or the whole line across the box
/// (a coloured name loses its contrast in greyscale video, the white
/// message keeps it).
fn line_visible(detector: &ChatDetector, name: RectI) -> bool {
    let line = RectI {
        x0: INK_X.0,
        x1: INK_X.1,
        ..name
    };
    text_visible(detector, name) || text_visible(detector, line)
}

/// Whether text is still visible in `text` in the detector's last frame:
/// enough stroke pixels, and far more than in the empty rows above it. A
/// textured background has strokes in both.
fn text_visible(detector: &ChatDetector, text: RectI) -> bool {
    let above = RectI {
        y0: text.y0 - TEXT_GAP - GAP_ROWS,
        y1: text.y0 - TEXT_GAP,
        ..text
    };
    let (Some(inside), Some(gap)) = (detector.strokes_in(text), detector.strokes_in(above)) else {
        return false;
    };
    let density = |n: u32, r: RectI| n as f32 / (r.width() * r.height()).max(1) as f32;
    let (inside, gap) = (density(inside, text), density(gap, above));
    inside >= FADE_INK && inside >= TEXT_CONTRAST * gap
}

/// Whether the chat box is visible at the line whose text starts at row
/// `top`, from the edge ratios of a frame.
fn present(edges: &[f32], top: i32) -> bool {
    let row = |y: i32| (y - SEARCH_Y.0).clamp(0, edges.len() as i32) as usize;
    let rows = &edges[row(top)..row(top + PRESENCE_ROWS)];
    let mut ratios = [0.0; PRESENCE_ROWS as usize];
    let ratios = &mut ratios[..rows.len()];
    ratios.copy_from_slice(rows);
    if ratios.is_empty() {
        return false;
    }
    let mid = ratios.len() / 2;
    *ratios.select_nth_unstable_by(mid, f32::total_cmp).1 < PRESENT_RATIO
}

/// The smallest rectangle containing `a` and `b`.
fn union(a: RectI, b: RectI) -> RectI {
    RectI {
        x0: a.x0.min(b.x0),
        y0: a.y0.min(b.y0),
        x1: a.x1.max(b.x1),
        y1: a.y1.max(b.y1),
    }
}
