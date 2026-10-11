//! The low-resolution gate: may a screen surface be open in this frame?
//!
//! The gate reads the quarter-scale frame only (plan 5.4,
//! `HudRoi::Screen`), so it runs on every frame of normal gameplay at a
//! small cost. Each surface is described by the straight edges of its
//! panel ([`GateSpec`]): vertical edges that stay in place, outlines of
//! cards that may sit anywhere in a range of rows, and horizontal edges
//! that move together when the panel slides. The gate samples the
//! surface's bounding box on a grid of [`GATE_STEP`] reference pixels
//! (one quarter-scale pixel at 1080p; card outlines on a grid of at
//! least one view pixel, [`outline_step`]) and scores every edge
//! ([`super::edges`]). A frame whose score reaches the surface's
//! threshold is a *candidate*: only then is the surface's native name
//! zone cropped and read, and only a read that finds the panel's
//! structure at native scale turns the surface on ([`super::temporal`]).
//! The threshold is therefore set for recall, not precision.

use blurrd_core::{RectI, Region};

use super::edges;
use super::sample::{Anchor, FrameView, RefPlane, ScreenMap};

/// Reference pixels per gate cell (one quarter-scale pixel of a 1080p
/// recording).
pub const GATE_STEP: i32 = 4;

/// Most reference pixels per cell of the card outlines (one
/// quarter-scale pixel of a 540p recording).
pub const MAX_OUTLINE_STEP: i32 = 8;

/// The cell of the card outlines ([`GateSpec::outlines`]) for `view`
/// mapped by `map`: [`GATE_STEP`], or one view pixel if that spans more
/// reference pixels (a recording below 1080p, a horizontally squeezed
/// one), up to [`MAX_OUTLINE_STEP`].
///
/// A finer grid than the view repeats view pixels unevenly, so a thin
/// outline one view pixel wide fills one cell in some columns and two in
/// others, and the line tests miss it in some rows (measured on a real
/// death recap scaled to 720p: the right card outline scored 0.10 on
/// 4-pixel cells and 1.0 on cells of one view pixel). Panel edges keep
/// the fine grid: they are steps, not lines, and the "switch player"
/// bar, a line that the gate scores as an edge, lost frames at 720p on
/// cells of one view pixel.
pub fn outline_step(view: &FrameView<'_>, map: &ScreenMap) -> i32 {
    let scale = view.scale * map.sx().min(map.sy());
    if scale <= 0.0 {
        return GATE_STEP;
    }
    // A small tolerance keeps exact 1080p (four reference pixels per view
    // pixel) on the fine grid despite rounding.
    let pixel = (1.0 / scale - 0.05).ceil() as i32;
    pixel.clamp(GATE_STEP, MAX_OUTLINE_STEP)
}

/// The candidate threshold of every surface's gate.
pub const GATE_THRESHOLD: f32 = 0.35;

/// The panel structure of one surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GateSpec {
    /// The surface.
    pub region: Region,
    /// How the panel is anchored.
    pub anchor: Anchor,
    /// Vertical edges `(x, y0, y1)` that do not move.
    pub vertical: &'static [(i32, i32, i32)],
    /// Columns of thin vertical lines that run side by side anywhere
    /// within [`GateSpec::outline_span`] (the left and right outlines of
    /// a list of cards of any length). They count as one edge each, all
    /// with the score of the weakest of them in their best window of
    /// [`GateSpec::outline_rows`] rows ([`edges::window_score`]).
    pub outlines: &'static [i32],
    /// The rows `(y0, y1)` in which [`GateSpec::outlines`] may lie.
    pub outline_span: (i32, i32),
    /// Reference rows over which [`GateSpec::outlines`] are scored.
    pub outline_rows: i32,
    /// Horizontal edges `(y, x0, x1)` that move together vertically.
    pub horizontal: &'static [(i32, i32, i32)],
    /// How far the panel may be off its place (calibration error or a
    /// slide), in reference pixels.
    pub slide: i32,
    /// Score at or above which a frame is a candidate.
    pub threshold: f32,
}

impl GateSpec {
    /// The score of the structure in `view`: the mean over all edges, the
    /// horizontal ones at their best common offset. Returns the score and
    /// the vertical offset of the panel in reference pixels. `vertical`,
    /// `horizontal` and `counts` are reused buffers.
    fn score(
        &self,
        view: &FrameView<'_>,
        map: &ScreenMap,
        vertical: &mut RefPlane,
        horizontal: &mut RefPlane,
        counts: &mut Vec<Vec<Vec<u32>>>,
    ) -> (f32, i32) {
        let mut total = 0.0;
        // The edge functions read two cells left and one right of the
        // line, at up to one cell off.
        let strip = |x: i32, y0: i32, y1: i32, step: i32| RectI {
            x0: x - 4 * step,
            y0,
            x1: x + 3 * step,
            y1,
        };
        for &(x, y0, y1) in self.vertical {
            vertical.sample(
                view,
                map,
                self.anchor,
                strip(x, y0, y1, GATE_STEP),
                GATE_STEP,
            );
            total += edges::vertical(vertical, x, y0, y1, 1).0;
        }
        if !self.outlines.is_empty() {
            let step = outline_step(view, map);
            let (y0, y1) = self.outline_span;
            counts.resize_with(self.outlines.len(), Vec::new);
            for (&x, column) in self.outlines.iter().zip(counts.iter_mut()) {
                // A thin line's test reads up to three cells right of it.
                let rect = RectI {
                    x1: x + 5 * step,
                    ..strip(x, y0, y1, step)
                };
                vertical.sample(view, map, self.anchor, rect, step);
                edges::line_prefixes(vertical, x, y0, y1, 1, column);
            }
            let n = (self.outline_rows / step).max(1) as usize;
            let score = edges::window_score(&counts[..self.outlines.len()], n);
            total += score * self.outlines.len() as f32;
        }
        let mut dy = 0;
        if let Some(rows) = self.horizontal_rows() {
            horizontal.sample(view, map, self.anchor, rows, GATE_STEP);
            let search = self.slide / GATE_STEP;
            let (h, offset) = edges::horizontal_set(horizontal, self.horizontal, search);
            total += h * self.horizontal.len() as f32;
            dy = offset;
        }
        let n = (self.vertical.len() + self.outlines.len() + self.horizontal.len()).max(1);
        (total / n as f32, dy)
    }

    /// The rectangle every horizontal edge can reach: their columns, and
    /// their rows with the slide plus the three cells `edges` reads around
    /// a line at one cell off.
    fn horizontal_rows(&self) -> Option<RectI> {
        let margin = self.slide + 4 * GATE_STEP;
        let mut it = self.horizontal.iter();
        let &(y, x0, x1) = it.next()?;
        let mut r = RectI {
            x0,
            y0: y,
            x1,
            y1: y,
        };
        for &(y, x0, x1) in it {
            r.x0 = r.x0.min(x0);
            r.x1 = r.x1.max(x1);
            r.y0 = r.y0.min(y);
            r.y1 = r.y1.max(y);
        }
        r.y0 -= margin;
        r.y1 += margin;
        Some(r)
    }
}

/// What the gate saw of one surface in one frame.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct GateSignal {
    /// The structure score (`0.0..=1.0`).
    pub score: f32,
    /// Whether the score reaches the surface's threshold: read the native
    /// name zone.
    pub candidate: bool,
    /// Vertical offset of the panel from its place, in reference pixels.
    pub offset: i32,
}

/// Evaluates the gates of several surfaces, reusing the sampling buffers.
#[derive(Debug, Clone, Default)]
pub struct Gates {
    vertical: RefPlane,
    horizontal: RefPlane,
    /// Line counts of the outline columns ([`edges::line_prefixes`]).
    counts: Vec<Vec<Vec<u32>>>,
}

impl Gates {
    /// A gate evaluator.
    pub fn new() -> Self {
        Self::default()
    }

    /// The signal of `spec` in `view` (normally the quarter-scale frame).
    pub fn evaluate(
        &mut self,
        spec: &GateSpec,
        view: &FrameView<'_>,
        map: &ScreenMap,
    ) -> GateSignal {
        let (score, offset) = spec.score(
            view,
            map,
            &mut self.vertical,
            &mut self.horizontal,
            &mut self.counts,
        );
        GateSignal {
            score,
            candidate: score >= spec.threshold,
            offset,
        }
    }
}
