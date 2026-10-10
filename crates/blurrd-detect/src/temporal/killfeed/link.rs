//! Linking per-frame detections into detection tracks and choosing the
//! frames templates are taken from (prototype `_link` and `_references`).
//!
//! The prototype links and groups a whole clip at once. Here both run as
//! the frames arrive: a track takes its first reference when one text
//! extent has been seen [`MIN_DETECTIONS`] times, then one every
//! [`REF_STEP`] detections of that extent at the reference width. The
//! prototype's mid-life and last references need the whole track and are
//! left out: the forward pass of the newest reference covers the same
//! frames, and the last detection of a track usually lies in the fade-out,
//! where a template holds as much background as name and matches a static
//! background on its own.

use blurrd_core::{NameKind, RectI};

use crate::detection::Detection;

/// A text extent needs this many detections before it is tracked, so
/// gameplay texture that passes as a name for a frame or two is not
/// (`MINDET`).
pub(super) const MIN_DETECTIONS: u32 = 4;
/// One template reference per this many detections of a text extent
/// (`REF_STEP`).
pub(super) const REF_STEP: u32 = 40;
/// A detection track stays alive this many frames after its last
/// detection.
const ALIVE: u32 = 3;
/// Two detections are the same name if their horizontal overlap is at
/// least this share of their horizontal union ...
const MIN_X_OVERLAP: f32 = 0.6;
/// ... and their text bottoms differ by at most this much (one track per
/// row slot: a stack jump starts a new track).
const MAX_DY: i32 = 6;
/// Detections whose text starts and ends within this many pixels of a
/// group's first detection have the same text extent.
const EXTENT_TOLERANCE: i32 = 3;
/// A reference is taken only from a detection at most this far from the
/// reference width of its group.
const WIDTH_TOLERANCE: i32 = 1;
/// A text extent that reaches further left than a tracked extent of the
/// same track by more than this, with the same right end, and alternates
/// with it is not a name of its own (see [`Group::flicker`]).
const FLICKER_DX: i32 = 2 * EXTENT_TOLERANCE;

/// A frame to take a template from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Reference {
    /// The frame.
    pub(super) frame: u32,
    /// The detection the template is cut around.
    pub(super) detection: Detection,
    /// A wider reading of the same name that this reference corrects.
    pub(super) replaces: Option<Replaced>,
}

/// A text extent of a track that turned out to be a misreading: the same
/// name read with extra pixels on its left (a cover bar edge while the row
/// wipes in), no longer read once the narrower extent took over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Replaced {
    /// Left and right end of the wider text.
    pub(super) x0: i32,
    pub(super) x1: i32,
    /// The first frame after the last detection of the wider text.
    pub(super) since: u32,
}

/// Detections of one text extent inside a detection track.
#[derive(Debug, Clone)]
struct Group {
    x0: i32,
    x1: i32,
    count: u32,
    /// Frames of the first and the latest detection.
    first: u32,
    last: u32,
    /// The group alternates with a narrower tracked group of the same
    /// track: it never gives a reference.
    flicker: bool,
    /// Detections until the first reference is chosen.
    early: Vec<(u32, Detection)>,
    /// Text width of the first reference.
    width: Option<i32>,
    /// Detections at the reference width since the first reference.
    good: u32,
}

impl Group {
    fn matches(&self, text: RectI) -> bool {
        (self.x0 - text.x0).abs() <= EXTENT_TOLERANCE
            && (self.x1 - text.x1).abs() <= EXTENT_TOLERANCE
    }

    /// Whether this group is the same name as `tracked` read with extra
    /// pixels on its left: both end at the same column, this one starts
    /// clearly further left, and the detector kept reading the tracked
    /// extent, as often or more, after this one first appeared. A bright
    /// spot or the edge of the killer portrait next to the first glyph
    /// widens a detection for a few frames; a template cut there carries
    /// the portrait and its box would blur into it.
    fn flicker(&self, tracked: &Self) -> bool {
        tracked.width.is_some()
            && (tracked.x1 - self.x1).abs() <= EXTENT_TOLERANCE
            && tracked.x0 - self.x0 > FLICKER_DX
            && tracked.last > self.first
            && tracked.count >= self.count
    }

    /// Whether this group corrects `wide`: the same name read wider on the
    /// left that the detector stopped reading before this group appeared.
    fn corrects(&self, wide: &Self) -> bool {
        wide.width.is_some()
            && (wide.x1 - self.x1).abs() <= EXTENT_TOLERANCE
            && self.x0 - wide.x0 > FLICKER_DX
            && wide.last < self.first
    }

    /// Adds a detection; returns a reference if it is one.
    fn add(&mut self, frame: u32, d: Detection) -> Option<Reference> {
        self.count += 1;
        self.last = frame;
        let width = d.text.x1 - d.text.x0;
        let Some(ref_width) = self.width else {
            self.early.push((frame, d));
            if self.count < MIN_DETECTIONS {
                return None;
            }
            // The first detection at the median width (any, if none is).
            let mut widths: Vec<i32> = self
                .early
                .iter()
                .map(|(_, e)| e.text.x1 - e.text.x0)
                .collect();
            widths.sort_unstable();
            let n = widths.len();
            // `2 × median`, exact for even counts.
            let median2 = widths[(n - 1) / 2] + widths[n / 2];
            let good = |e: &Detection| {
                (2 * (e.text.x1 - e.text.x0) - median2).abs() <= 2 * WIDTH_TOLERANCE
            };
            let pick = self.early.iter().position(|(_, e)| good(e)).unwrap_or(0);
            let (f, chosen) = self.early[pick];
            let w = chosen.text.x1 - chosen.text.x0;
            let near =
                |(_, e): &&(u32, Detection)| ((e.text.x1 - e.text.x0) - w).abs() <= WIDTH_TOLERANCE;
            self.good = self.early[pick + 1..].iter().filter(near).count() as u32;
            self.width = Some(w);
            self.early = Vec::new();
            return Some(Reference {
                frame: f,
                detection: chosen,
                replaces: None,
            });
        };
        if (width - ref_width).abs() > WIDTH_TOLERANCE {
            return None;
        }
        self.good += 1;
        if self.good.is_multiple_of(REF_STEP) {
            return Some(Reference {
                frame,
                detection: d,
                replaces: None,
            });
        }
        None
    }
}

/// A detection track: one name in one row slot.
#[derive(Debug, Clone)]
struct Track {
    text: RectI,
    last: u32,
    groups: Vec<Group>,
}

/// Links the detections of one kind frame by frame.
#[derive(Debug, Clone)]
pub(super) struct Linker {
    kind: NameKind,
    active: Vec<Track>,
    candidates: Vec<Detection>,
    pairs: Vec<(f32, usize, usize)>,
    used_detections: Vec<bool>,
    used_tracks: Vec<bool>,
}

impl Linker {
    /// A linker of the detections of `kind`.
    pub(super) const fn new(kind: NameKind) -> Self {
        Self {
            kind,
            active: Vec::new(),
            candidates: Vec::new(),
            pairs: Vec::new(),
            used_detections: Vec::new(),
            used_tracks: Vec::new(),
        }
    }

    /// Links the detections of `frame` (all kinds; only the linker's kind
    /// is used) and pushes references to `out`.
    pub(super) fn push(&mut self, frame: u32, detections: &[Detection], out: &mut Vec<Reference>) {
        let kind = self.kind;
        self.candidates.clear();
        self.candidates
            .extend(detections.iter().filter(|d| d.kind == kind));
        // Greedy matching on `x overlap − |dy| / 100`, best first; ties go
        // to the later detection and the later track, as in the
        // prototype's reverse tuple sort.
        self.pairs.clear();
        for (di, d) in self.candidates.iter().enumerate() {
            for (ti, t) in self.active.iter().enumerate() {
                if frame - t.last > ALIVE {
                    continue;
                }
                let overlap = (t.text.x1.min(d.text.x1) - t.text.x0.max(d.text.x0)).max(0);
                let union = (t.text.x1.max(d.text.x1) - t.text.x0.min(d.text.x0)).max(1);
                let ix = overlap as f32 / union as f32;
                let dy = d.text.y1 - t.text.y1;
                if ix >= MIN_X_OVERLAP && dy.abs() <= MAX_DY {
                    self.pairs.push((ix - dy.abs() as f32 / 100.0, di, ti));
                }
            }
        }
        self.pairs
            .sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.cmp(&a.1)).then(b.2.cmp(&a.2)));
        self.used_detections.clear();
        self.used_detections.resize(self.candidates.len(), false);
        self.used_tracks.clear();
        self.used_tracks.resize(self.active.len(), false);
        for &(_, di, ti) in &self.pairs {
            if self.used_detections[di] || self.used_tracks[ti] {
                continue;
            }
            self.used_detections[di] = true;
            self.used_tracks[ti] = true;
            let d = self.candidates[di];
            let t = &mut self.active[ti];
            t.text = d.text;
            t.last = frame;
            out.extend(add_to_groups(&mut t.groups, frame, d));
        }
        for (di, d) in self.candidates.iter().enumerate() {
            if self.used_detections[di] {
                continue;
            }
            let mut groups = Vec::new();
            out.extend(add_to_groups(&mut groups, frame, *d));
            self.active.push(Track {
                text: d.text,
                last: frame,
                groups,
            });
        }
        self.active.retain(|t| frame - t.last <= ALIVE);
    }

    /// Heap bytes held by the linker.
    pub(super) fn heap_bytes(&self) -> usize {
        self.active.capacity() * size_of::<Track>()
            + self
                .active
                .iter()
                .map(|t| {
                    t.groups.capacity() * size_of::<Group>()
                        + t.groups
                            .iter()
                            .map(|g| g.early.capacity() * size_of::<(u32, Detection)>())
                            .sum::<usize>()
                })
                .sum::<usize>()
            + self.candidates.capacity() * size_of::<Detection>()
            + self.pairs.capacity() * size_of::<(f32, usize, usize)>()
            + self.used_detections.capacity()
            + self.used_tracks.capacity()
    }
}

/// Adds a detection to the group of its text extent (a new group if none
/// matches).
fn add_to_groups(groups: &mut Vec<Group>, frame: u32, d: Detection) -> Option<Reference> {
    let index = if let Some(i) = groups.iter().position(|g| g.matches(d.text)) {
        i
    } else {
        groups.push(Group {
            x0: d.text.x0,
            x1: d.text.x1,
            count: 0,
            first: frame,
            last: frame,
            flicker: false,
            early: Vec::new(),
            width: None,
            good: 0,
        });
        groups.len() - 1
    };
    let first = groups[index].width.is_none();
    let mut reference = groups[index].add(frame, d)?;
    let group = &groups[index];
    let flicker = group.flicker
        || groups
            .iter()
            .enumerate()
            .any(|(i, tracked)| i != index && group.flicker(tracked));
    if first {
        reference.replaces = groups
            .iter()
            .enumerate()
            .filter(|&(i, wide)| i != index && group.corrects(wide))
            .map(|(_, wide)| Replaced {
                x0: wide.x0,
                x1: wide.x1,
                since: wide.last + 1,
            })
            .next();
    }
    groups[index].flicker = flicker;
    (!flicker).then_some(reference)
}

#[cfg(test)]
mod tests {
    use blurrd_core::{NameKind, RectI, Region};

    use super::{Linker, Reference};
    use crate::detection::Detection;

    /// A victim name with text from `x0` to 1830 in the row at top 96.
    fn victim(x0: i32) -> Detection {
        let text = RectI {
            x0,
            y0: 107,
            x1: 1830,
            y1: 121,
        };
        Detection {
            region: Region::KillFeed,
            kind: NameKind::Victim,
            text,
            rect: RectI {
                x0: x0 - 5,
                y0: 101,
                x1: 1835,
                y1: 128,
            },
            row_top: Some(96),
        }
    }

    /// Links the readings `x0s` (one per frame from frame 0) and returns
    /// the references taken.
    fn link(x0s: &[i32]) -> Vec<Reference> {
        let mut linker = Linker::new(NameKind::Victim);
        let mut out = Vec::new();
        for (f, &x0) in (0..).zip(x0s) {
            linker.push(f, &[victim(x0)], &mut out);
        }
        out
    }

    #[test]
    fn a_wider_reading_that_alternates_gives_no_reference() {
        // The name is read from 1700; a bright spot widens it to 1688 in
        // some frames, as often as four times.
        let refs = link(&[1700, 1700, 1700, 1700, 1688, 1700, 1688, 1700, 1688, 1688]);
        assert_eq!(refs.len(), 1, "{refs:?}");
        assert_eq!(refs[0].detection.text.x0, 1700);
    }

    #[test]
    fn a_narrower_reading_corrects_a_wider_one_that_ended() {
        // The row wipes in read from 1688 (the cover bar's edge), then the
        // detector reads the name from 1700 only.
        let refs = link(&[1688, 1688, 1688, 1688, 1688, 1700, 1700, 1700, 1700]);
        assert_eq!(refs.len(), 2, "{refs:?}");
        assert_eq!(refs[0].replaces, None);
        let fix = refs[1].replaces.expect("the wide reading is replaced");
        assert_eq!((fix.x0, fix.x1, fix.since), (1688, 1830, 5));
    }

    #[test]
    fn a_wider_reading_that_lasts_is_a_name_of_its_own() {
        // The narrower extent stops being read: the wider one is kept.
        let refs = link(&[1700, 1700, 1700, 1700, 1688, 1688, 1688, 1688, 1688]);
        assert_eq!(refs.len(), 2, "{refs:?}");
        assert!(refs.iter().all(|r| r.replaces.is_none()));
    }
}
