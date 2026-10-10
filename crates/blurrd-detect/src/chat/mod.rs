//! Text-chat names: the sender of every chat line (port spec section 6).
//!
//! The text chat sits on the bottom left of the screen. Up to three
//! messages are shown on a 29 px line grid inside a dark box that
//! darkens the frame multiplicatively (four on some recordings, the oldest
//! one above the grid of three, often while it fades):
//!
//! ```text
//! (All) SenderName: message text
//! (Team) Sender Name: message text
//! (System) PlayerName has abandoned the match
//! ```
//!
//! A new message appears at the bottom line within one frame and pushes
//! the older ones up; old messages fade out together with their part of
//! the box. While the player types, the chat is open: the history of up to
//! eight lines on the same grid fills a taller panel above the input line,
//! and messages do not fade. Its lines are read like those of the closed
//! chat. The typed text is the player's own: it lies on the input line,
//! which reaches below the left-panel crop, and is not covered.
//! [`ChatDetector`] finds the box from its vertical edges, the
//! text lines inside it from their ink, and the name in each line from its
//! layout. It works on the left-panel crop of plan 5.4
//! ([`LEFT_PANEL_ROI`]) and reports reference-pixel boxes. [`ChatTracker`]
//! follows the lines across frames and covers them in the frames the
//! detector misses: pop-ins, faint lines and fade-outs.
//!
//! Nothing depends on the client language (plan 7.3). The prefix is the
//! line's first word, recognised by the parentheses it starts and ends
//! with (only parentheses reach from the ascender line to below the
//! baseline), so "(All)", "(Alle)" and "(Transmisión)" are found without a
//! list of prefixes or their widths. The name follows the prefix and ends
//! at the colon after it; where the colon cannot be seen, at the end of
//! the prefix colour (the prefix has the name's colour, the colon and the
//! message are white). Where either end stays uncertain, the box takes
//! Safe geometry: a prefix that runs into the name covers the name from the
//! end of the shortest prefix ([`NameStart::Shortest`]), and a line without
//! colon and name colour, such as a system line ("(System) Name has
//! abandoned the match", white name and phrase), is covered from the name
//! start for the widest name ([`NameEnd::Safe`]), so names of several words
//! stay covered in every language.

mod band;
mod ink;
mod line;
mod track;

pub use line::{NameEnd, NameStart};
pub use track::{BACKFILL_MAX, ChatTracker, FADE_MAX, HOLD_MAX, POST_ROLL};

use blurrd_core::{NameKind, RectI, Region};

use crate::detection::{DetectError, Detection};
use crate::image::RgbView;

/// The left-panel crop of plan 5.4 in reference pixels: voice chat and
/// text chat.
pub const LEFT_PANEL_ROI: RectI = RectI {
    x0: 0,
    y0: 550,
    x1: 780,
    y1: 1040,
};

/// Left and right edge columns of the dark chat box (`CHAT_X0`,
/// `CHAT_X1`); the open chat's input line spans them too.
pub const BOX_X: (i32, i32) = (26, 469);

/// The right edge column of the open chat's history panel (its scrollbar
/// lies to the right of it).
pub const PANEL_X1: i32 = 457;

/// Rows searched for the chat box and its lines: the port spec's rows
/// 930..1040 (`CHAT_Y0`, `CHAT_Y1`) of the closed chat, extended up over
/// the open chat's history panel, whose box starts at row 787.
pub const SEARCH_Y: (i32, i32) = (780, 1040);

/// The ink is measured from this row at least: the closed chat's three
/// lines (port spec `CHAT_Y0`). Above it, it is measured over the box
/// bands found there, a fourth line of the closed chat or the open chat's
/// panel, and while their lines fade.
const CLOSED_INK_TOP: i32 = 930;

/// Rows above a box band from which its ink is measured: the text starts
/// about 6 rows below the band top, and a fading line is told from the
/// empty rows above its text.
const INK_MARGIN: i32 = 8;

/// Columns searched for text: the inside of the box.
const INK_X: (i32, i32) = (30, BOX_X.1 - 2);

/// Padding of a name's blur box around its text: left, top, right,
/// bottom (the prototype's box).
const NAME_PAD: (i32, i32, i32, i32) = (3, 4, 4, 2);

/// The blur box reaches at least this far below the top of the text:
/// descenders are not always part of the strokes found.
const MIN_TEXT_ROWS: i32 = 16;

/// A part of the dark chat box: rows where both box edges are visible.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoxBand {
    /// First row (reference pixels).
    pub y0: i32,
    /// One past the last row.
    pub y1: i32,
    /// How bright the box is relative to the frame next to it: about 0.66
    /// for a fully visible box, rising to 1 while it fades out.
    pub ratio: f32,
}

/// One chat line with a sender name.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChatLine {
    /// Top of the line's text in reference pixels (ascenders).
    pub top: i32,
    /// One past the line's lowest text row.
    pub bottom: i32,
    /// The box band the line lies in.
    pub band: BoxBand,
    /// The sender name's text bounds.
    pub name: RectI,
    /// How the start of the name was found.
    pub start: NameStart,
    /// How the end of the name was found.
    pub end: NameEnd,
    /// The text after the name on the same line: the message (from the
    /// colon) or the system phrase. Used when messages are blurred too.
    pub message: Option<RectI>,
}

/// Reusable buffers.
#[derive(Debug, Clone, Default)]
struct Scratch {
    lines: line::LineScratch,
    /// The edges of each searched row.
    rows: Vec<band::RowEdge>,
}

/// Finds the sender names in the text chat of one frame.
///
/// Reuse one detector for all frames of a video: it keeps its buffers.
#[derive(Debug, Clone, Default)]
pub struct ChatDetector {
    edges: Vec<f32>,
    /// Frames since the box was last found.
    frames_without_box: u32,
    /// Whether the ink was measured in the last frame.
    ink_measured: bool,
    /// Rows the ink area reaches above [`CLOSED_INK_TOP`] (the open chat's
    /// panel), and frames since a band last asked for that.
    ink_extra: i32,
    ink_extra_age: u32,
    bands: Vec<BoxBand>,
    lines: Vec<ChatLine>,
    scratch: Scratch,
}

impl ChatDetector {
    /// A detector with the constants of the port spec.
    pub fn new() -> Self {
        Self::default()
    }

    /// The parts of the chat box found in the last frame, top to bottom.
    /// They are a structure signal of their own: the box is there while
    /// messages are shown, whether or not their names were found.
    pub fn bands(&self) -> &[BoxBand] {
        &self.bands
    }

    /// How visible the chat box was in each searched row of the last frame
    /// (`SEARCH_Y.0 + i` for entry `i`): the brightness inside the box
    /// relative to outside, at its left and right edges (the closed box's
    /// or the open panel's), whichever is closer to 1, or at the left edge
    /// alone where that is a clear step. About 0.66 inside a fully visible
    /// box, rising to 1 while it fades out, around 1 without a box.
    pub fn edge_ratios(&self) -> &[f32] {
        &self.edges
    }

    /// The lines whose names were found in the last frame, top to bottom.
    pub fn lines(&self) -> &[ChatLine] {
        &self.lines
    }

    /// The text stroke pixels inside `rect` (reference pixels) in the last
    /// frame, or `None` if the ink was not measured: it is measured while
    /// the box is visible and for [`FADE_MAX`] frames after.
    pub fn strokes_in(&self, rect: RectI) -> Option<u32> {
        self.ink_measured
            .then(|| self.scratch.lines.strokes_in(rect))
    }

    /// Detects the sender names in `crop`, the [`LEFT_PANEL_ROI`] of a
    /// frame in reference scale, and replaces the content of `out` with
    /// them, top to bottom.
    ///
    /// # Errors
    ///
    /// [`DetectError::CropSize`] if the crop is not the size of
    /// [`LEFT_PANEL_ROI`].
    pub fn detect(
        &mut self,
        crop: &RgbView<'_>,
        out: &mut Vec<Detection>,
    ) -> Result<(), DetectError> {
        out.clear();
        self.lines.clear();
        let (width, height) = (LEFT_PANEL_ROI.width(), LEFT_PANEL_ROI.height());
        if (crop.width(), crop.height()) != (width, height) {
            return Err(DetectError::CropSize {
                width,
                height,
                actual_width: crop.width(),
                actual_height: crop.height(),
            });
        }
        let origin = (LEFT_PANEL_ROI.x0, LEFT_PANEL_ROI.y0);
        band::find_bands(
            crop,
            origin.1,
            &mut self.edges,
            &mut self.scratch.rows,
            &mut self.bands,
        );
        // The ink is measured while the box is visible and through the
        // fade after it, where text can outlast the box edge.
        self.frames_without_box = if self.bands.is_empty() {
            self.frames_without_box.saturating_add(1)
        } else {
            0
        };
        self.ink_measured = self.frames_without_box <= track::FADE_MAX;
        if !self.ink_measured {
            return Ok(());
        }
        let top = self.ink_top();
        self.scratch.lines.compute_ink(crop, origin, top);
        if self.bands.is_empty() {
            return Ok(());
        }
        let lines = &mut self.lines;
        line::find_names(
            crop,
            origin,
            &self.bands,
            &mut self.scratch.lines,
            |found, band| {
                let name = RectI {
                    x0: found.cols.0,
                    y0: found.rows.0,
                    x1: found.cols.1,
                    y1: found.rows.1,
                };
                lines.push(ChatLine {
                    top: found.rows.0,
                    bottom: found.rows.1,
                    band: *band,
                    name,
                    start: found.start,
                    end: found.end,
                    message: found.message.map(|(x0, x1)| RectI {
                        x0,
                        y0: found.rows.0,
                        x1,
                        y1: found.rows.1,
                    }),
                });
                out.push(name_detection(name));
            },
        );
        Ok(())
    }

    /// The first row of this frame's ink area: [`CLOSED_INK_TOP`], or
    /// above it over the topmost box band. It stays up for [`FADE_MAX`]
    /// frames after the band is gone, while the lines that were there
    /// fade.
    fn ink_top(&mut self) -> i32 {
        let wanted = self
            .bands
            .first()
            .map_or(0, |band| CLOSED_INK_TOP - (band.y0 - INK_MARGIN))
            .max(0);
        if wanted >= self.ink_extra || self.ink_extra_age >= track::FADE_MAX {
            self.ink_extra = wanted;
            self.ink_extra_age = 0;
        } else {
            self.ink_extra_age += 1;
        }
        CLOSED_INK_TOP - self.ink_extra
    }
}

/// The detection of a sender name with text bounds `text`.
fn name_detection(text: RectI) -> Detection {
    Detection {
        region: Region::TextChat,
        kind: NameKind::ChatSender,
        text,
        rect: RectI {
            x0: text.x0 - NAME_PAD.0,
            y0: text.y0 - NAME_PAD.1,
            x1: text.x1 + NAME_PAD.2,
            y1: text.y1.max(text.y0 + MIN_TEXT_ROWS) + NAME_PAD.3,
        },
        row_top: Some(text.y0),
    }
}
