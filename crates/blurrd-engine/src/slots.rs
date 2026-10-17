//! The name slots of the HUD regions: where in a frame a text line can be
//! a player name of a region (plan 7.2, "in a name slot").
//!
//! The protected zones are wide (they hold the slide-in of kill-feed rows
//! and every chat layout), so they also show text that is no name: HUD
//! numbers, labels below the kill feed, the game world behind the panel
//! (distance markers, name tags above heads, which are out of scope).
//! Pass 0 adds only names inside a slot to the dictionary, and the
//! verification only checks name parts inside a slot. The slots are
//! reference rectangles kept at their element's anchor, so they follow
//! the calibrated HUD geometry at every resolution and layout.

use blurrd_core::{HudGeometry, RectI, Region};
use blurrd_detect::banner::{self, NAME_STRIP};
use blurrd_detect::chat;
use blurrd_detect::killfeed::{FEED_ROI, ROW_H, ROW_PITCH};
use blurrd_detect::screens::{Anchor, ScreenMap};
use blurrd_detect::temporal::killfeed::STRIP;
use blurrd_detect::voice;

/// Top of the first kill-feed row (reference row; the calibration's slot
/// grid).
const FEED_FIRST_ROW: i32 = 96;
/// Kill-feed rows that can stack (decision C18: eight rows).
const FEED_ROWS: i32 = 8;
/// Right end of the kill-feed names: victim names are right-aligned at
/// reference column 1822, the row's bar ends at column 1837.
const FEED_X1: i32 = 1840;
/// Rows a slot reaches beyond its element's nominal rows (sliding rows,
/// rounding of the geometry).
const MARGIN: i32 = 4;
/// Height of a voice-chat speaker's text band below its slot top.
const VOICE_TEXT_H: i32 = 24;

/// The name slots of the HUD regions of one video, in source pixels.
#[derive(Debug, Clone)]
pub(crate) struct NameSlots {
    /// In the order they are tried: the banner's strip lies inside the
    /// kill feed's area.
    slots: [(Region, RectI); 4],
}

impl NameSlots {
    /// The slots of a video whose HUD sits at `geometry`.
    pub(crate) fn new(geometry: &HudGeometry) -> Self {
        let map = ScreenMap::from(geometry);
        let feed = RectI {
            x0: STRIP.x0,
            y0: FEED_FIRST_ROW - MARGIN,
            x1: FEED_X1,
            // The last row's bottom: text just below it is no row's.
            y1: (FEED_FIRST_ROW + (FEED_ROWS - 1) * ROW_PITCH + ROW_H).min(FEED_ROI.y1),
        };
        let voice = RectI {
            x0: voice::TEXT_X - MARGIN,
            y0: voice::slot_top(voice::SLOTS - 1) - MARGIN,
            x1: voice::SLOT_X1 + MARGIN,
            y1: voice::SLOT0_TOP + VOICE_TEXT_H + MARGIN,
        };
        let chat = RectI {
            x0: chat::BOX_X.0,
            y0: chat::SEARCH_Y.0,
            x1: chat::BOX_X.1,
            y1: chat::SEARCH_Y.1,
        };
        let banner = RectI {
            y0: banner::NAME_BAND.0 - MARGIN,
            y1: banner::NAME_BAND.1 + MARGIN,
            ..NAME_STRIP
        };
        Self {
            slots: [
                (Region::Banner, map.to_source(banner, banner::ANCHOR.into())),
                (Region::KillFeed, map.to_source(feed, Anchor::Right)),
                (Region::VoiceChat, map.to_source(voice, Anchor::Left)),
                (Region::TextChat, map.to_source(chat, Anchor::Left)),
            ],
        }
    }

    /// The region whose name slot holds the centre of `rect` (source
    /// pixels), if any.
    pub(crate) fn region_of(&self, rect: RectI) -> Option<Region> {
        let (cx, cy) = (rect.x0 + rect.x1, rect.y0 + rect.y1);
        self.slots
            .iter()
            .find(|(_, s)| 2 * s.x0 <= cx && cx < 2 * s.x1 && 2 * s.y0 <= cy && cy < 2 * s.y1)
            .map(|&(region, _)| region)
    }
}

#[cfg(test)]
mod tests {
    use blurrd_core::{HudGeometry, RectI, Region};

    use super::NameSlots;

    fn rect(x0: i32, y0: i32, x1: i32, y1: i32) -> RectI {
        RectI::new(x0, y0, x1, y1).unwrap()
    }

    #[test]
    fn slots_hold_names_and_not_the_text_around_them() {
        let slots = NameSlots::new(&HudGeometry::default());
        // A victim name of the second kill-feed row.
        assert_eq!(
            slots.region_of(rect(1760, 140, 1820, 158)),
            Some(Region::KillFeed)
        );
        // Text under the eighth row and right of the bars.
        assert_eq!(slots.region_of(rect(1724, 398, 1772, 412)), None);
        assert_eq!(slots.region_of(rect(1869, 140, 1900, 158)), None);
        // The banner's name inside the feed's area.
        assert_eq!(
            slots.region_of(rect(900, 302, 1000, 325)),
            Some(Region::Banner)
        );
        // A speaker, a chat line, a name tag above a head in the panel.
        assert_eq!(
            slots.region_of(rect(58, 680, 140, 698)),
            Some(Region::VoiceChat)
        );
        assert_eq!(
            slots.region_of(rect(40, 990, 160, 1010)),
            Some(Region::TextChat)
        );
        assert_eq!(slots.region_of(rect(626, 607, 699, 628)), None);
    }

    #[test]
    fn slots_follow_the_hud_geometry() {
        // 1440p: everything 4/3 as large.
        let qhd = HudGeometry {
            sx: 4.0 / 3.0,
            sy: 4.0 / 3.0,
            anchor_right: 2560.0,
            anchor_center: 1280.0,
            ..HudGeometry::default()
        };
        let slots = NameSlots::new(&qhd);
        assert_eq!(
            slots.region_of(rect(2347, 187, 2427, 211)),
            Some(Region::KillFeed)
        );
        assert_eq!(slots.region_of(rect(980, 140, 1080, 158)), None);
    }
}
