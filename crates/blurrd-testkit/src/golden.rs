//! Goldens: reviewed ground truth of real excerpts, and the harness that
//! scores boxes against it (architecture plan 11.2 and 11.3).
//!
//! A golden lists every name item of an excerpt as keyframed glyph extents
//! (the tight bounds of the text, not the blur box) plus the portrait
//! rectangles. It contains no text and no text-derived values: names are
//! personal data. Goldens live in the media folder
//! ([`crate::media::MediaDir::golden_path`]), never in the repository.
//!
//! [`Golden::truth_at`] turns a golden frame into the same [`FrameTruth`]
//! the synthetic fixtures produce, so [`evaluate`] reuses the metrics of
//! [`crate::metrics`]. [`missing_items`] is the missing-item audit: OCR
//! text proposals that match no golden item.

use blurrd_blur::PreparedBox;
use blurrd_core::{FrameRange, NameKind, RectI, Region};
use serde::{Deserialize, Serialize};

use crate::mask::Mask;
use crate::metrics::{Evaluation, Summary};
use crate::proposals::{Proposal, ProposalSource};
use crate::truth::{Content, ContentKind, FrameTruth, GLYPH_HALO_REF_PX, ItemId, TruthItem};

/// Schema identifier of golden files.
pub const GOLDEN_SCHEMA: &str = "blurrd-golden/1";

/// How an item's extent changes between two keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Interp {
    /// Keep the previous key's extent until the next key (row jumps, pops).
    Hold,
    /// Move linearly between keys, rounded outward (slides, wipes).
    Linear,
}

/// The extent of an item at a key frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Key {
    /// Presentation frame index.
    pub f: u32,
    /// Tight glyph extent in source pixels.
    pub rect: RectI,
}

/// One name item over its visible frames.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoldenItem {
    /// Identifier, unique in the golden.
    pub id: u32,
    /// Region of the name.
    pub region: Region,
    /// What the name is.
    pub kind: NameKind,
    /// Interpolation between keys.
    pub interp: Interp,
    /// Keys, sorted by frame; the first key is the first visible frame.
    pub keys: Vec<Key>,
    /// Last visible frame (inclusive), at or after the last key.
    pub last: u32,
}

impl GoldenItem {
    /// The frames the item is visible in.
    pub fn frames(&self) -> Option<FrameRange> {
        let first = self.keys.first()?.f;
        FrameRange::new(first, self.last).ok()
    }

    /// The glyph extent at `frame`, or `None` outside the item's frames.
    pub fn rect_at(&self, frame: u32) -> Option<RectI> {
        if !self.frames()?.contains(frame) {
            return None;
        }
        let next = self.keys.partition_point(|k| k.f <= frame);
        let prev = self.keys[next - 1];
        match (self.interp, self.keys.get(next)) {
            (Interp::Linear, Some(to)) => {
                let t = (frame - prev.f) as f32 / (to.f - prev.f) as f32;
                let lerp = |a: i32, b: i32| a as f32 + (b - a) as f32 * t;
                let (a, b) = (prev.rect, to.rect);
                RectI::new(
                    lerp(a.x0, b.x0).floor() as i32,
                    lerp(a.y0, b.y0).floor() as i32,
                    lerp(a.x1, b.x1).ceil() as i32,
                    lerp(a.y1, b.y1).ceil() as i32,
                )
                .ok()
            }
            _ => Some(prev.rect),
        }
    }
}

/// A rectangle of non-name content over a frame range (portraits).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    /// Inclusive frame range.
    pub frames: FrameRange,
    /// Rectangle in source pixels.
    pub rect: RectI,
}

/// The reviewed truth of one excerpt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Golden {
    /// Must be [`GOLDEN_SCHEMA`].
    pub schema: String,
    /// Excerpt id from the media manifest.
    pub excerpt: String,
    /// SHA-256 of the clip the frames refer to.
    pub clip_sha256: String,
    /// The excerpt's frame range.
    pub frames: FrameRange,
    /// HUD scale of the recording (`sy`), for the glyph halo.
    pub scale: f32,
    /// The name items.
    pub items: Vec<GoldenItem>,
    /// Portrait rectangles (blurring into them is portrait intrusion).
    pub portraits: Vec<Span>,
}

/// Why a golden was rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum GoldenError {
    /// The JSON does not have the golden's shape.
    #[error("parsing the golden: {0}")]
    Parse(String),
    /// The content breaks a rule of the format.
    #[error("invalid golden: {0}")]
    Invalid(String),
}

impl Golden {
    /// Parses and validates a golden.
    ///
    /// # Errors
    ///
    /// [`GoldenError::Parse`] or [`GoldenError::Invalid`].
    pub fn from_json(text: &str) -> Result<Self, GoldenError> {
        let golden: Self =
            serde_json::from_str(text).map_err(|e| GoldenError::Parse(e.to_string()))?;
        golden.validate()?;
        Ok(golden)
    }

    /// Writes the golden as pretty-printed JSON.
    ///
    /// # Errors
    ///
    /// [`GoldenError::Invalid`] if the golden breaks a rule of the format.
    pub fn to_json(&self) -> Result<String, GoldenError> {
        self.validate()?;
        serde_json::to_string_pretty(self).map_err(|e| GoldenError::Invalid(e.to_string()))
    }

    fn validate(&self) -> Result<(), GoldenError> {
        let invalid = |msg: String| Err(GoldenError::Invalid(msg));
        if self.schema != GOLDEN_SCHEMA {
            return invalid(format!(
                "schema `{}`, expected `{GOLDEN_SCHEMA}`",
                self.schema
            ));
        }
        if !(self.scale.is_finite() && self.scale > 0.0) {
            return invalid(format!("scale {} must be positive", self.scale));
        }
        let mut ids: Vec<u32> = self.items.iter().map(|i| i.id).collect();
        ids.sort_unstable();
        if ids.windows(2).any(|w| w[0] == w[1]) {
            return invalid("duplicate item ids".to_owned());
        }
        for item in &self.items {
            let Some(range) = item.frames() else {
                return invalid(format!(
                    "item {}: no keys or last frame before the first",
                    item.id
                ));
            };
            if item.keys.windows(2).any(|w| w[1].f <= w[0].f) {
                return invalid(format!(
                    "item {}: keys are not strictly increasing",
                    item.id
                ));
            }
            let inside = item.keys.last().is_some_and(|k| k.f <= item.last);
            if !inside || range.f0 < self.frames.f0 || range.f1 > self.frames.f1 {
                return invalid(format!("item {}: frames outside the excerpt", item.id));
            }
        }
        if let Some(p) = self
            .portraits
            .iter()
            .find(|p| p.frames.f0 < self.frames.f0 || p.frames.f1 > self.frames.f1)
        {
            return invalid(format!("portrait {} outside the excerpt", p.rect));
        }
        Ok(())
    }

    /// The truth of `frame`: every visible item with its glyph extent as
    /// the glyph mask, dilated by [`GLYPH_HALO_REF_PX`] × scale, and the
    /// portraits.
    pub fn truth_at(&self, frame: u32) -> FrameTruth {
        let halo = (GLYPH_HALO_REF_PX * self.scale).ceil().max(1.0) as u32;
        let items = self
            .items
            .iter()
            .filter_map(|item| {
                let text = item.rect_at(frame)?;
                let mut ink = Mask::empty(text);
                for y in text.y0..text.y1 {
                    for x in text.x0..text.x1 {
                        ink.insert(x, y);
                    }
                }
                let glyphs = ink.dilated(halo);
                let slot = glyphs.area();
                Some(TruthItem {
                    id: ItemId(item.id),
                    region: item.region,
                    kind: item.kind,
                    text,
                    glyphs,
                    slot,
                    opacity: 1.0,
                })
            })
            .collect();
        let content = self
            .portraits
            .iter()
            .filter(|p| p.frames.contains(frame))
            .map(|p| Content {
                kind: ContentKind::Portrait,
                rect: p.rect,
            })
            .collect();
        FrameTruth {
            frame,
            scale: self.scale,
            items,
            content,
        }
    }
}

/// Scores the boxes of every frame of the golden's excerpt.
///
/// `boxes(frame)` returns the prepared boxes the render would apply to
/// that frame (detector output, a plan lookup, or anything else).
pub fn evaluate(golden: &Golden, mut boxes: impl FnMut(u32) -> Vec<PreparedBox>) -> Summary {
    let mut eval = Evaluation::new();
    for frame in golden.frames.f0..=golden.frames.f1 {
        eval.add_frame(&golden.truth_at(frame), &boxes(frame));
    }
    eval.summary().clone()
}

/// The missing-item audit: proposals with an OCR text box that overlap no
/// golden item in any of their frames. After review this must be empty
/// (plan 11.2); every entry is either a missed name or a non-name to
/// whitelist.
pub fn missing_items<'a>(golden: &Golden, proposals: &'a [Proposal]) -> Vec<&'a Proposal> {
    proposals
        .iter()
        .filter(|p| p.sources.contains(ProposalSource::OcrText))
        .filter(|p| {
            !p.keys.iter().any(|(frame, rect)| {
                golden
                    .items
                    .iter()
                    .filter_map(|i| i.rect_at(*frame))
                    .any(|r| r.intersection(rect).is_some())
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use blurrd_core::{FrameRange, NameKind, RectI, Region};

    use super::{Golden, GoldenError, GoldenItem, Interp, Key, Span, missing_items};
    use crate::proposals::{ProposalBuilder, ProposalSource};

    fn rect(x0: i32, y0: i32, x1: i32, y1: i32) -> RectI {
        RectI::new(x0, y0, x1, y1).unwrap()
    }

    fn sample() -> Golden {
        Golden {
            schema: super::GOLDEN_SCHEMA.to_owned(),
            excerpt: "E1".to_owned(),
            clip_sha256: "a".repeat(64),
            frames: FrameRange { f0: 100, f1: 140 },
            scale: 1.0,
            items: vec![
                GoldenItem {
                    id: 1,
                    region: Region::KillFeed,
                    kind: NameKind::Victim,
                    interp: Interp::Hold,
                    keys: vec![
                        Key {
                            f: 100,
                            rect: rect(1786, 146, 1831, 160),
                        },
                        Key {
                            f: 120,
                            rect: rect(1786, 107, 1831, 121),
                        },
                    ],
                    last: 130,
                },
                GoldenItem {
                    id: 2,
                    region: Region::Banner,
                    kind: NameKind::Banner,
                    interp: Interp::Linear,
                    keys: vec![
                        Key {
                            f: 110,
                            rect: rect(1100, 307, 1160, 320),
                        },
                        Key {
                            f: 120,
                            rect: rect(900, 307, 960, 320),
                        },
                    ],
                    last: 140,
                },
            ],
            portraits: vec![Span {
                frames: FrameRange { f0: 100, f1: 130 },
                rect: rect(1838, 140, 1900, 173),
            }],
        }
    }

    #[test]
    fn interpolates_keys() {
        let g = sample();
        let (hold, linear) = (&g.items[0], &g.items[1]);
        assert_eq!(hold.rect_at(99), None);
        assert_eq!(hold.rect_at(119), Some(rect(1786, 146, 1831, 160)));
        assert_eq!(hold.rect_at(125), Some(rect(1786, 107, 1831, 121)));
        assert_eq!(hold.rect_at(131), None);
        assert_eq!(linear.rect_at(115), Some(rect(1000, 307, 1060, 320)));
        assert_eq!(linear.rect_at(140), Some(rect(900, 307, 960, 320)));
    }

    #[test]
    fn round_trips_through_json() {
        let g = sample();
        let json = g.to_json().unwrap();
        assert!(json.contains("\"schema\": \"blurrd-golden/1\""));
        assert_eq!(Golden::from_json(&json).unwrap(), g);
    }

    #[test]
    fn rejects_invalid_goldens() {
        let mut g = sample();
        g.items[0].last = 150;
        assert!(matches!(g.to_json(), Err(GoldenError::Invalid(_))));
        let mut g = sample();
        g.items[1].keys.swap(0, 1);
        assert!(g.to_json().is_err());
        let mut g = sample();
        g.items[1].id = 1;
        assert!(g.to_json().is_err());
        let mut g = sample();
        g.schema = "blurrd-golden/2".to_owned();
        assert!(g.to_json().is_err());
        assert!(matches!(
            Golden::from_json("{}"),
            Err(GoldenError::Parse(_))
        ));
    }

    #[test]
    fn builds_frame_truth_with_a_halo() {
        let t = sample().truth_at(125);
        assert_eq!(t.items.len(), 2);
        let victim = &t.items[0];
        assert_eq!(victim.text, rect(1786, 107, 1831, 121));
        assert_eq!(victim.glyphs.bounds(), Some(rect(1784, 105, 1833, 123)));
        assert_eq!(t.portraits().count(), 1);
        assert!(sample().truth_at(135).portraits().next().is_none());
    }

    #[test]
    fn evaluates_boxes_against_the_truth() {
        use blurrd_blur::{PixelParams, YuvColour, prepare_boxes};
        use blurrd_core::{BlurBox, BlurParams};

        let g = sample();
        let params = PixelParams::new(&BlurParams::default(), 1.0, YuvColour::default()).unwrap();
        // Boxes exactly on the truth's halo cover everything ...
        let covering = super::evaluate(&g, |frame| {
            let boxes: Vec<BlurBox> = g
                .truth_at(frame)
                .items
                .iter()
                .map(|i| BlurBox {
                    rect: i.slot,
                    region: i.region,
                    seed: 1,
                    grid_origin: (i.slot.x0, i.slot.y0),
                    block_scale: 1.0,
                })
                .collect();
            let mut out = vec![];
            prepare_boxes(&boxes, 1920, 1080, &params, &mut out);
            out
        });
        assert_eq!(covering.frames, 41);
        assert_eq!(covering.item_frames, 31 + 31);
        assert_eq!(covering.leaked_item_frames, 0);
        // ... no boxes leak every item-frame.
        let none = super::evaluate(&g, |_| Vec::new());
        assert_eq!(none.leaked_item_frames, 62);
        assert_eq!(none.longest_leak_run, 31);
    }

    #[test]
    fn audits_unmatched_text_proposals() {
        let g = sample();
        let mut b = ProposalBuilder::default();
        b.push_frame(
            125,
            &[
                (ProposalSource::OcrText, rect(1790, 108, 1820, 120)),
                (ProposalSource::OcrText, rect(600, 900, 700, 915)),
                (ProposalSource::PixelChange, rect(100, 100, 120, 120)),
            ],
        );
        let proposals = b.finish();
        let missing = missing_items(&g, &proposals);
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].keys[0].1, rect(600, 900, 700, 915));
    }
}
