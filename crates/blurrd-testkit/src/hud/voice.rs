//! Voice-chat speakers on the left.
//!
//! Geometry in reference pixels from the port spec (section 5): slot `k`
//! has its top at y `678 − 40k` (slot 0 at the bottom, up to four slots),
//! a portrait at x 16..52, the bold white name from x 58 (11 px capitals,
//! baseline at the slot top + 16) and a grey location line below it
//! (baseline + 32). Entries appear and disappear within one frame.

use blurrd_core::{FrameRange, NameKind, RectI, Region};

use crate::font::FontSet;
use crate::layout::{Anchor, Layout};
use crate::paint::{Label, NameDraw, Painter, rect_f};
use crate::palette::{TEXT_GREY, TEXT_WHITE};
use crate::truth::{ContentKind, ItemId};

/// Number of speaker slots.
pub const SLOTS: u8 = 4;
/// Top of slot 0.
pub const SLOT0_TOP: i32 = 678;
/// Distance between slot tops.
pub const SLOT_PITCH: i32 = 40;
/// Left edge of the name.
pub const NAME_X: f32 = 58.0;
/// Capital height of the name.
pub const NAME_CAP: f32 = 11.0;
/// Right edge of the name slot.
pub const SLOT_X1: i32 = 199;
/// Item ids of speakers start here.
const ID_BASE: u32 = 3000;

/// One speaker entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Speaker {
    /// The speaker's name.
    pub name: String,
    /// The location line, for example "in B Main - Team".
    pub location: String,
    /// The slot (0 = bottom, at most [`SLOTS`] − 1).
    pub slot: u8,
    /// Frame ranges in which the entry is shown; gaps are blink-offs.
    pub shown: Vec<FrameRange>,
}

/// The voice chat of a scene.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceSpec {
    /// The speakers.
    pub speakers: Vec<Speaker>,
}

/// Made-up location lines (generic place words, no map names).
pub const LOCATIONS: [&str; 4] = [
    "in B Main - Team",
    "in A Site - Team",
    "in Mid Hall - Team",
    "in C Long - Team",
];

#[derive(Debug, Clone, PartialEq)]
struct Entry {
    speaker: Speaker,
    index: u32,
    name: Label,
    location: Label,
}

/// The voice chat rendered for one layout.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Voice {
    entries: Vec<Entry>,
}

impl Voice {
    pub(crate) fn new(fonts: &FontSet, layout: &Layout, spec: VoiceSpec) -> Self {
        let entries = (0..)
            .zip(spec.speakers)
            .map(|(index, speaker)| Entry {
                name: Label::new(fonts, layout, &speaker.name, NAME_CAP),
                location: Label::new(fonts, layout, &speaker.location, 8.0),
                speaker,
                index,
            })
            .collect();
        Self { entries }
    }

    pub(crate) fn paint(&self, p: &mut Painter<'_>, frame: u32) {
        let a = Anchor::Left;
        for e in &self.entries {
            if !e.speaker.shown.iter().any(|r| r.contains(frame)) {
                continue;
            }
            let y = SLOT0_TOP - SLOT_PITCH * i32::from(e.speaker.slot.min(SLOTS - 1));
            p.portrait(
                RectI {
                    x0: 16,
                    y0: y + 2,
                    x1: 52,
                    y1: y + 38,
                },
                a,
                u64::from(e.index) * 104_729 + 17,
                1.0,
            );
            p.name(
                NameDraw {
                    id: ItemId(ID_BASE + e.index),
                    region: Region::VoiceChat,
                    kind: NameKind::Speaker,
                    slot: rect_f(55.0, (y + 2) as f32, SLOT_X1 as f32, (y + 20) as f32),
                },
                &e.name,
                (NAME_X, (y + 16) as f32),
                a,
                TEXT_WHITE,
                1.0,
                None,
            );
            p.label(
                ContentKind::Location,
                &e.location,
                (NAME_X, (y + 32) as f32),
                a,
                TEXT_GREY,
                1.0,
                None,
            );
        }
    }
}
