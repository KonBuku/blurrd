//! The low-resolution gate: may a screen surface be open in this frame?
//!
//! The gate reads the quarter-scale frame only (plan 5.4,
//! `HudRoi::Screen`), so it runs on every frame of normal gameplay at a
//! small cost. Each surface is described by the straight edges of its
//! panel ([`GateSpec`]): vertical edges that stay in place and horizontal
//! edges that move together when the panel slides. The gate samples the
//! surface's bounding box on a grid of [`GATE_STEP`] reference pixels
//! (one quarter-scale pixel at 1080p) and scores every edge
//! ([`super::edges`]). A frame whose score reaches the surface's
//! threshold is a *candidate*: only then is the surface's native name
//! zone cropped and read, and only a read that finds the panel's
//! structure at native scale turns the surface on ([`super::temporal`]).
//! The threshold is therefore set for recall, not precision.

use blurrd_core::{RectI, Region};

use super::edges;
use super::sample::{Anchor, FrameView, RefPlane, ScreenMap};

/// Reference pixels per gate cell.
pub const GATE_STEP: i32 = 4;

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
    /// the vertical offset of the panel in reference pixels. `vertical`
    /// and `horizontal` are reused sampling buffers.
    fn score(
        &self,
        view: &FrameView<'_>,
        map: &ScreenMap,
        vertical: &mut RefPlane,
        horizontal: &mut RefPlane,
    ) -> (f32, i32) {
        let mut total = 0.0;
        for &(x, y0, y1) in self.vertical {
            // `edges::vertical` reads two cells left and one right of the
            // line, at up to one cell off.
            let strip = RectI {
                x0: x - 4 * GATE_STEP,
                y0,
                x1: x + 3 * GATE_STEP,
                y1,
            };
            vertical.sample(view, map, self.anchor, strip, GATE_STEP);
            total += edges::vertical(vertical, x, y0, y1, 1).0;
        }
        let mut dy = 0;
        if let Some(rows) = self.horizontal_rows() {
            horizontal.sample(view, map, self.anchor, rows, GATE_STEP);
            let search = self.slide / GATE_STEP;
            let (h, offset) = edges::horizontal_set(horizontal, self.horizontal, search);
            total += h * self.horizontal.len() as f32;
            dy = offset;
        }
        let n = (self.vertical.len() + self.horizontal.len()).max(1);
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
        let (score, offset) = spec.score(view, map, &mut self.vertical, &mut self.horizontal);
        GateSignal {
            score,
            candidate: score >= spec.threshold,
            offset,
        }
    }
}
