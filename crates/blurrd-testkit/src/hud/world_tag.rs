//! Name tags above players' heads in the game world.
//!
//! They are out of scope by user decision D8: blurrd never blurs them,
//! and the screen-surface detectors must never box them. The fixtures
//! draw them as decoys, the way they look on real recordings: a short
//! bold white name with a drop shadow over a player-like figure, at any
//! position in the image. They are recorded as
//! [`ContentKind::WorldTag`] content, not as names.

use blurrd_core::{FrameRange, RectI};

use crate::font::FontSet;
use crate::layout::{Anchor, Layout};
use crate::paint::{Label, Painter};
use crate::palette::TEXT_WHITE;
use crate::truth::ContentKind;

/// Capital height of a world tag, as measured near the crosshair.
pub const TAG_CAP: f32 = 10.0;

/// One name tag in the world.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldTag {
    /// The name shown.
    pub name: String,
    /// Centre of the tag's baseline in reference pixels (centre-anchored).
    pub at: (i32, i32),
    /// Frames in which it is visible.
    pub shown: FrameRange,
}

/// The world tags of a scene, rendered for one layout.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WorldTags {
    tags: Vec<(WorldTag, Label)>,
}

impl WorldTags {
    pub(crate) fn new(fonts: &FontSet, layout: &Layout, tags: Vec<WorldTag>) -> Self {
        let tags = tags
            .into_iter()
            .map(|t| {
                let label = Label::new(fonts, layout, &t.name, TAG_CAP);
                (t, label)
            })
            .collect();
        Self { tags }
    }

    pub(crate) fn paint(&self, p: &mut Painter<'_>, frame: u32) {
        let a = Anchor::Center;
        for (tag, label) in &self.tags {
            if !tag.shown.contains(frame) {
                continue;
            }
            let (x, y) = tag.at;
            // A blocky figure under the tag.
            p.fill(
                RectI {
                    x0: x - 14,
                    y0: y + 8,
                    x1: x + 14,
                    y1: y + 90,
                },
                a,
                [70, 64, 60],
                1.0,
            );
            let pen = (x as f32 - label.width * 0.5, y as f32);
            p.label(ContentKind::WorldTag, label, pen, a, TEXT_WHITE, 1.0, None);
        }
    }
}
