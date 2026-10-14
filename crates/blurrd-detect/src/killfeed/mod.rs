//! Kill-feed names: killer and victim in every row (port spec section 3).
//!
//! The kill feed is a stack of right-aligned rows in the top-right corner,
//! 33 px tall on a 39 px pitch, the newest at the bottom:
//!
//! ```text
//! [badge][killer portrait][KILLER NAME][weapon][icons] | [VICTIM NAME][victim portrait]
//! ```
//!
//! The left half has the killer's team colour, the right half the
//! victim's; self, ability and spike kills have one colour and a round
//! icon between the two names. [`KillFeedDetector`] finds the rows of one
//! frame and the names in each row. It works on the feed crop of plan 5.4
//! ([`FEED_ROI`]) and reports reference-pixel boxes; following names across
//! frames is the temporal model's job.
//!
//! Nothing depends on the default team colours alone. Rows are anchored on
//! the right-aligned white victim name; where the default teal and red do
//! not explain a row's bar (colour-blind modes, custom colours, GPU colour
//! filters such as Digital Vibrance or hue and saturation changes), the
//! bar colours are learned from the frame and drive the row score and the
//! killer | victim boundary instead. A bar without usable chroma (a
//! black-and-white filter, saturation near 0) is accepted from luminance
//! structure alone; such a row is read as a one-colour row and marked
//! low confidence ([`BarEvidence::Luminance`]).
//!
//! The brightness thresholds of the white and text classes follow the
//! frame: the white level of the victim names is measured first, and a
//! darkening filter (brightness, contrast, gamma) lowers the thresholds
//! with it ([`ClassParams::for_white_level`]).
//!
//! Only the bands the detector reads are classified: the columns that find
//! the rows over the whole crop height, then the full width of every row
//! found. Classifying the whole crop would cost about as much as the
//! detector's entire budget (plan 18.12).

mod analysis;
mod hues;
mod rows;
mod tones;

pub use hues::{BarHueLearner, BarHues, TeamHue};

use blurrd_core::{NameKind, RectI, Region};

use crate::classes::{ClassParams, PixelClasses};
use crate::components::{Component, ComponentScratch};
use crate::detection::{DetectError, Detection};
use crate::image::RgbView;

/// The feed and banner crop of plan 5.4 in reference pixels: wide enough
/// for the killer side of long rows, tall enough for eight stacked rows
/// (decision C18 extends the prototype's six-row crop).
pub const FEED_ROI: RectI = RectI {
    x0: 660,
    y0: 80,
    x1: 1920,
    y1: 412,
};

/// Height of a kill-feed row.
pub const ROW_H: i32 = 33;

/// Distance between the tops of two stacked rows.
pub const ROW_PITCH: i32 = 39;

/// Columns inside every row's bar, used to find the rows (`SCORE_X`).
const SCORE_X: (i32, i32) = (1660, 1837);
/// Victim names end right of this column (right alignment).
const RIGHT_ALIGN: i32 = 1822;
/// The victim text search ends here (`VIC_TX1`).
const VICTIM_TEXT_X1: i32 = 1834;
/// The victim portrait starts at about this column; names end left of it
/// (`VIC_X1`).
pub(crate) const VICTIM_X1: i32 = 1838;
/// Left end of the searched part of a row (`FEED_X0`).
const FEED_X0: i32 = 1250;
/// The victim's bar colour is measured from this column to
/// [`VICTIM_X1`].
const VICTIM_COLOUR_X0: i32 = 1790;
/// Rows of a row's inner band (used for colours), from each edge.
const INNER_MARGIN: i32 = 4;
/// The first row's top is never above this line (`FEED_Y_TOP`).
const FEED_Y_TOP: i32 = 88;
/// Horizontal padding of a name's blur box around its text.
const NAME_PAD_X: i32 = 5;
/// The blur box covers the row's text band: from the row top + 5 to the
/// row top + 28 (text runs from + 10 to + 24, descenders to + 28).
const NAME_BAND: (i32, i32) = (5, 28);
/// The blur box reaches at least this far below the text strokes found:
/// descenders are not part of them (they touch the row edge rules), and a
/// row top found a pixel too high would otherwise cut them off.
const DESCENDER: i32 = 7;

/// How a row was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RowAnchor {
    /// From the line of the right-aligned victim name, which does not
    /// depend on the background behind the feed.
    TextLine,
    /// From the top and bottom edges of the row's bar.
    BarEdge,
}

/// What showed that a row's band is a kill-feed bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BarEvidence {
    /// The default team colours of the port spec.
    DefaultColour,
    /// Bar colours learned from the frame (a colour-blind palette, custom
    /// team colours, a hue or saturation filter).
    LearnedColour,
    /// Luminance structure only: a vertically flat band behind the victim
    /// text line, next to a textured victim portrait. The bar shows no
    /// usable chroma (a black-and-white filter or saturation near 0), so
    /// the killer and victim sides cannot be told apart: the row is read as
    /// a one-colour row, which reports every name as a victim.
    Luminance,
}

/// One kill-feed row of a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FeedRow {
    /// Top of the row in reference pixels; the row spans
    /// `top..top + ROW_H`.
    pub top: i32,
    /// How the row was found.
    pub anchor: RowAnchor,
    /// What showed that the band is a bar.
    pub evidence: BarEvidence,
}

impl FeedRow {
    /// Whether the row was found with low confidence: from luminance
    /// structure only ([`BarEvidence::Luminance`]). The health check and
    /// escalation of later stages act on it (plan 7.3).
    pub fn is_low_confidence(&self) -> bool {
        self.evidence == BarEvidence::Luminance
    }
}

/// Reusable buffers of the row analysis.
#[derive(Debug, Clone, Default)]
struct Scratch {
    components: ComponentScratch,
    found: Vec<Component>,
    comps: Vec<analysis::Comp>,
    clusters: Vec<analysis::Cluster>,
    profile: analysis::Profile,
    rows: rows::RowScratch,
    /// Bar colours learned by the row finder in the current frame.
    tones: Vec<tones::Tone>,
}

/// Finds the killer and victim names in the kill-feed rows of one frame.
///
/// Reuse one detector for all frames of a video: it keeps its buffers.
#[derive(Debug, Clone, Default)]
pub struct KillFeedDetector {
    params: ClassParams,
    classes: PixelClasses,
    rows: Vec<FeedRow>,
    scratch: Scratch,
}

impl KillFeedDetector {
    /// A detector with the pixel classes of the port spec.
    pub fn new() -> Self {
        Self::default()
    }

    /// A detector with other pixel-class thresholds (for example learned
    /// bar hues).
    pub fn with_params(params: ClassParams) -> Self {
        Self {
            params,
            ..Self::default()
        }
    }

    /// The rows found in the last frame, top to bottom.
    pub fn rows(&self) -> &[FeedRow] {
        &self.rows
    }

    /// Appends the slot boxes of the rows found in the last frame to
    /// `out`: each row's text band over the searched width, from the left
    /// end of the killer search to the victim portrait. The row structure
    /// shows names whether or not they were read, so the health check
    /// covers a failed span with these ([`crate::health`]).
    pub fn row_slots(&self, out: &mut Vec<RectI>) {
        out.extend(self.rows.iter().map(|r| RectI {
            x0: FEED_X0,
            y0: r.top + NAME_BAND.0,
            x1: VICTIM_X1,
            y1: r.top + NAME_BAND.1,
        }));
    }

    /// Detects the names in `crop`, the [`FEED_ROI`] of a frame in
    /// reference scale, and replaces the content of `out` with them.
    ///
    /// Detections come row by row from the top, the victims of a row before
    /// its killer, each with the row it was found in.
    ///
    /// # Errors
    ///
    /// [`DetectError::CropSize`] if the crop is not the size of
    /// [`FEED_ROI`].
    pub fn detect(
        &mut self,
        crop: &RgbView<'_>,
        out: &mut Vec<Detection>,
    ) -> Result<(), DetectError> {
        out.clear();
        self.rows.clear();
        let (width, height) = (FEED_ROI.width(), FEED_ROI.height());
        if (crop.width(), crop.height()) != (width, height) {
            return Err(DetectError::CropSize {
                width,
                height,
                actual_width: crop.width(),
                actual_height: crop.height(),
            });
        }
        // The row finder reads the score columns over the whole height.
        let score = to_crop(RectI {
            x0: SCORE_X.0,
            y0: FEED_ROI.y0,
            x1: SCORE_X.1,
            y1: FEED_ROI.y1,
        });
        self.classes.compute_rect(crop, score, &self.params);
        // A darkening filter or a colour tint: classify again with
        // thresholds that follow the white point of the names.
        let params = rows::white_point(&self.classes, &self.params)
            .map_or(self.params, |(white, chroma)| {
                self.params.for_white_point(white, chroma)
            });
        if params != self.params {
            self.classes.compute_rect(crop, score, &params);
        }
        rows::find_rows(
            crop,
            &self.classes,
            &mut self.scratch.rows,
            &mut self.scratch.tones,
            &mut self.rows,
        );

        // Then every row band at full width. Bands closer than the top-hat
        // margin are classified together, so that no band's margin clears
        // the text flags of another.
        let margin = 2 * (params.tophat_size / 2) as i32;
        let mut pending: Option<(i32, i32)> = None;
        for row in &self.rows {
            let (y0, y1) = (row.top, row.top + ROW_H);
            pending = match pending {
                Some((a, b)) if y0 <= b + margin => Some((a, y1.max(b))),
                Some((a, b)) => {
                    classify_band(&mut self.classes, crop, &params, a, b);
                    Some((y0, y1))
                }
                None => Some((y0, y1)),
            };
        }
        if let Some((a, b)) = pending {
            classify_band(&mut self.classes, crop, &params, a, b);
        }
        for row in &self.rows {
            let emit = |kind, text| out.push(name_detection(kind, text, row.top));
            analysis::analyze_row(&self.classes, *row, &mut self.scratch, emit);
        }
        Ok(())
    }
}

/// Classifies the searched columns of reference rows `y0..y1`.
fn classify_band(
    classes: &mut PixelClasses,
    crop: &RgbView<'_>,
    params: &ClassParams,
    y0: i32,
    y1: i32,
) {
    let band = to_crop(RectI {
        x0: FEED_X0,
        y0,
        x1: VICTIM_X1,
        y1,
    });
    classes.compute_rect(crop, band, params);
}

/// A reference rectangle in crop coordinates.
const fn to_crop(r: RectI) -> RectI {
    RectI {
        x0: r.x0 - FEED_ROI.x0,
        y0: r.y0 - FEED_ROI.y0,
        x1: r.x1 - FEED_ROI.x0,
        y1: r.y1 - FEED_ROI.y0,
    }
}

/// The detection of a name with text bounds `text` in the row at `top`.
fn name_detection(kind: NameKind, text: RectI, top: i32) -> Detection {
    Detection {
        region: Region::KillFeed,
        kind,
        text,
        rect: RectI {
            x0: text.x0 - NAME_PAD_X,
            y0: top + NAME_BAND.0,
            x1: text.x1 + NAME_PAD_X,
            y1: (top + NAME_BAND.1).max(text.y1 + DESCENDER),
        },
        row_top: Some(top),
    }
}
