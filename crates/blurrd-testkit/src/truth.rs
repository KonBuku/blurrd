//! Ground truth of a fixture frame: every name, where its ink is, and the
//! text and art around it that must stay unblurred.

use blurrd_core::{NameKind, RectI, Region};

use crate::mask::Mask;

/// Halo added around every glyph mask, in reference pixels: anti-aliasing,
/// outline and shadow (architecture plan 5.8).
pub const GLYPH_HALO_REF_PX: f32 = 2.0;

/// Identity of a name over the frames it is visible in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ItemId(pub u32);

/// One visible player name.
#[derive(Debug, Clone, PartialEq)]
pub struct TruthItem {
    /// Stable identity over the frames the name is visible in.
    pub id: ItemId,
    /// The region the name belongs to.
    pub region: Region,
    /// What the name is inside its region.
    pub kind: NameKind,
    /// Tight bounds of the name's ink (anti-aliasing and shadow included,
    /// before the halo).
    pub text: RectI,
    /// Every pixel of the ink, dilated by [`GLYPH_HALO_REF_PX`] (scaled to
    /// the layout). This is what a blur must cover.
    pub glyphs: Mask,
    /// The slot the name can appear in: the box Safe mode would blur. It
    /// always contains [`TruthItem::glyphs`].
    pub slot: RectI,
    /// Opacity of the name in this frame (`0.0..=1.0`; fades go below 1).
    pub opacity: f32,
}

/// What a piece of non-name content is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ContentKind {
    /// Agent portrait art (kill feed, voice chat). Blurring into it counts
    /// as portrait intrusion.
    Portrait,
    /// The location line under a voice-chat speaker ("in B Main - Team").
    Location,
    /// A chat channel prefix such as "(All)".
    ChatPrefix,
    /// A chat message body after the colon.
    ChatMessage,
    /// The fixed phrase of a system chat line.
    SystemPhrase,
    /// A weapon or kill icon in the kill feed.
    Icon,
    /// The multikill badge in front of a kill-feed row.
    Badge,
    /// The title of a banner ("ACE", "CLUTCH").
    BannerTitle,
    /// A HUD notification without a player name ("X ULT READY").
    Notification,
}

/// A rectangle of non-name content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Content {
    /// What it is.
    pub kind: ContentKind,
    /// Where it is, in source pixels.
    pub rect: RectI,
}

/// The ground truth of one frame.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FrameTruth {
    /// Presentation index of the frame.
    pub frame: u32,
    /// HUD scale of the frame (`sy`), for scale-dependent tolerances.
    pub scale: f32,
    /// Every visible name.
    pub items: Vec<TruthItem>,
    /// Non-name content: portraits, location lines, messages, icons.
    pub content: Vec<Content>,
}

impl FrameTruth {
    /// The portrait rectangles of the frame.
    pub fn portraits(&self) -> impl Iterator<Item = RectI> + '_ {
        self.content
            .iter()
            .filter(|c| c.kind == ContentKind::Portrait)
            .map(|c| c.rect)
    }

    /// The items of one region.
    pub fn in_region(&self, region: Region) -> impl Iterator<Item = &TruthItem> + '_ {
        self.items.iter().filter(move |i| i.region == region)
    }
}
