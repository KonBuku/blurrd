//! User edits on top of the detector tracks.
//!
//! Overrides carry no track ids, so they survive a re-analysis that replaces
//! every track. `Suppress` and `Adjust` name the box they act on by an anchor
//! rectangle and bind to the detector track that overlaps the anchor best
//! (same region, overlapping frames, IoU at least [`SUPPRESS_MIN_IOU`] or
//! [`ADJUST_MIN_IOU`]). Bindings are derived from the current tracks every
//! time a plan is resolved and are never stored. An override that finds no
//! such track is *unbound*: it is inactive and never applied to another box.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::geometry::RectI;
use crate::plan::Track;
use crate::region::Region;

/// Minimum IoU between a `Suppress` anchor and a detector box.
///
/// Stricter than [`ADJUST_MIN_IOU`] because a wrong suppression removes
/// protection from a name.
pub const SUPPRESS_MIN_IOU: f32 = 0.7;

/// Minimum IoU between an `Adjust` anchor and a detector box.
pub const ADJUST_MIN_IOU: f32 = 0.5;

/// A user edit, applied in list order after the detector tracks.
///
/// Frame ranges are inclusive presentation frame indices; rectangles are in
/// source pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(tag = "op")]
#[non_exhaustive]
pub enum Override {
    /// Adds a box of its own for frames `f0..=f1`.
    Add {
        /// Stable id of the edit (for undo, warnings and the UI).
        uid: Uuid,
        /// Region the box belongs to, usually [`Region::Manual`].
        region: Region,
        /// First frame.
        f0: u32,
        /// Last frame (inclusive).
        f1: u32,
        /// The box.
        rect: RectI,
    },
    /// Hides the detector box at `anchor` for frames `f0..=f1`.
    ///
    /// Within the range, a box of the bound track is hidden on every frame
    /// on which it still overlaps the anchor by at least
    /// [`SUPPRESS_MIN_IOU`].
    Suppress {
        /// Stable id of the edit.
        uid: Uuid,
        /// Region of the box to hide.
        region: Region,
        /// First frame.
        f0: u32,
        /// Last frame (inclusive).
        f1: u32,
        /// Where the hidden box was when the user hid it.
        anchor: RectI,
    },
    /// Replaces the detector box at `anchor` with `rect` for frames
    /// `f0..=f1`.
    ///
    /// Within the range, a box of the bound track is replaced on every frame
    /// on which it still overlaps the anchor by at least [`ADJUST_MIN_IOU`].
    Adjust {
        /// Stable id of the edit.
        uid: Uuid,
        /// Region of the box to change.
        region: Region,
        /// First frame.
        f0: u32,
        /// Last frame (inclusive).
        f1: u32,
        /// Where the box was when the user changed it.
        anchor: RectI,
        /// The new box.
        rect: RectI,
    },
}

impl Override {
    /// The stable id of the edit.
    pub const fn uid(&self) -> Uuid {
        match *self {
            Self::Add { uid, .. } | Self::Suppress { uid, .. } | Self::Adjust { uid, .. } => uid,
        }
    }

    /// The region the edit acts on.
    pub const fn region(&self) -> Region {
        match *self {
            Self::Add { region, .. }
            | Self::Suppress { region, .. }
            | Self::Adjust { region, .. } => region,
        }
    }

    /// The inclusive frame range `(f0, f1)`.
    pub const fn frames(&self) -> (u32, u32) {
        match *self {
            Self::Add { f0, f1, .. }
            | Self::Suppress { f0, f1, .. }
            | Self::Adjust { f0, f1, .. } => (f0, f1),
        }
    }

    /// Whether `frame` is inside the edit's frame range.
    pub const fn covers(&self, frame: u32) -> bool {
        let (f0, f1) = self.frames();
        f0 <= frame && frame <= f1
    }

    /// The anchor and the minimum IoU for edits that bind to a track.
    const fn anchor(&self) -> Option<(RectI, f32)> {
        match *self {
            Self::Add { .. } => None,
            Self::Suppress { anchor, .. } => Some((anchor, SUPPRESS_MIN_IOU)),
            Self::Adjust { anchor, .. } => Some((anchor, ADJUST_MIN_IOU)),
        }
    }
}

/// Whether an override is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum BindStatus {
    /// The override is active.
    Bound,
    /// No matching detector box exists; the override is inactive and should
    /// be reported to the user (`OverrideUnbound`).
    Unbound,
}

/// The binding state of one override after resolving it against the tracks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct OverrideBinding {
    /// The override's id.
    pub uid: Uuid,
    /// Whether it is active.
    pub status: BindStatus,
}

/// The overrides of a plan resolved against its tracks.
#[derive(Debug)]
pub(crate) struct Resolution {
    /// For every track (by index), the indexes of the `Suppress` and `Adjust`
    /// overrides bound to it, in list order.
    per_track: Vec<Vec<usize>>,
    /// One binding per override, in list order.
    pub(crate) bindings: Vec<OverrideBinding>,
}

impl Resolution {
    /// Binds every override to at most one track.
    ///
    /// The bound track is the one of the override's region whose segments,
    /// within the override's frame range, reach the highest IoU with the
    /// anchor, provided it meets the minimum. Ties go to the earlier track.
    pub(crate) fn new(tracks: &[Track], overrides: &[Override]) -> Self {
        let mut per_track = vec![Vec::new(); tracks.len()];
        let mut bindings = Vec::with_capacity(overrides.len());
        for (index, edit) in overrides.iter().enumerate() {
            let status = match edit.anchor() {
                None => BindStatus::Bound,
                Some((anchor, min_iou)) => {
                    let (f0, f1) = edit.frames();
                    let best = tracks
                        .iter()
                        .enumerate()
                        .filter(|(_, track)| track.region == edit.region())
                        .filter_map(|(t, track)| {
                            let iou = track
                                .segments_between(f0, f1)
                                .map(|s| s.rect.iou(&anchor))
                                .fold(0.0_f32, f32::max);
                            (iou >= min_iou).then_some((t, iou))
                        })
                        .fold(None, |best: Option<(usize, f32)>, (t, iou)| match best {
                            Some((_, best_iou)) if best_iou >= iou => best,
                            _ => Some((t, iou)),
                        });
                    match best {
                        Some((t, _)) => {
                            per_track[t].push(index);
                            BindStatus::Bound
                        }
                        None => BindStatus::Unbound,
                    }
                }
            };
            bindings.push(OverrideBinding {
                uid: edit.uid(),
                status,
            });
        }
        Self {
            per_track,
            bindings,
        }
    }

    /// Applies the overrides bound to track `track_index` to its box `rect`
    /// on `frame`. Returns the box to blur, or `None` if it is suppressed.
    pub(crate) fn apply(
        &self,
        overrides: &[Override],
        track_index: usize,
        frame: u32,
        rect: RectI,
    ) -> Option<RectI> {
        let mut out = rect;
        for &index in &self.per_track[track_index] {
            let edit = &overrides[index];
            if !edit.covers(frame) {
                continue;
            }
            match *edit {
                Override::Suppress { anchor, .. } if rect.iou(&anchor) >= SUPPRESS_MIN_IOU => {
                    return None;
                }
                Override::Adjust {
                    anchor, rect: new, ..
                } if rect.iou(&anchor) >= ADJUST_MIN_IOU => {
                    out = new;
                }
                _ => {}
            }
        }
        Some(out)
    }
}
