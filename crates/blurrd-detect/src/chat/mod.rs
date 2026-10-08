//! Text-chat names: the sender of every chat line (port spec section 6).
//!
//! The text chat sits on the bottom left of the screen. Up to three
//! messages are shown on a 29 px line grid inside a dark box that
//! darkens the frame multiplicatively:
//!
//! ```text
//! (All) SenderName: message text
//! (Team) Sender Name: message text
//! (System) PlayerName has abandoned the match
//! ```
//!
//! A new message appears at the bottom line within one frame and pushes
//! the older ones up; old messages fade out together with their part of
//! the box. [`ChatDetector`] finds the box from its vertical edges, the
//! text lines inside it from their ink, and the name in each line from its
//! layout. It works on the left-panel crop of plan 5.4
//! ([`LEFT_PANEL_ROI`]) and reports reference-pixel boxes; following lines
//! across frames is the temporal model's job.
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

pub use line::{NameEnd, NameStart};

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
/// `CHAT_X1`).
pub const BOX_X: (i32, i32) = (26, 469);

/// Rows searched for chat lines (`CHAT_Y0`, `CHAT_Y1`).
pub const SEARCH_Y: (i32, i32) = (930, 1040);

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
    bands: band::BandScratch,
    lines: line::LineScratch,
}

/// Finds the sender names in the text chat of one frame.
///
/// Reuse one detector for all frames of a video: it keeps its buffers.
#[derive(Debug, Clone, Default)]
pub struct ChatDetector {
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

    /// The lines whose names were found in the last frame, top to bottom.
    pub fn lines(&self) -> &[ChatLine] {
        &self.lines
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
        band::find_bands(crop, origin.1, &mut self.scratch.bands, &mut self.bands);
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
