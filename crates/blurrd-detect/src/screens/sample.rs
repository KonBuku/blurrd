//! Reading the source frame through the HUD geometry.
//!
//! Screen-surface constants are reference pixels of a 1920 × 1080 frame.
//! A [`ScreenMap`] maps them onto the recording (any resolution, native
//! 4:3, pillarbox, 16:10, stretched 4:3) with the calibrated
//! [`HudGeometry`], and [`RefPlane::sample`] resamples a reference
//! rectangle of a [`FrameView`] (a native crop or the quarter-scale
//! frame) onto a regular reference grid. The detectors then work in
//! reference pixels only: there is no per-resolution code.

use blurrd_core::{HudAnchor, HudGeometry, RectI};

use crate::image::RgbView;

/// Which screen position an element keeps when the frame changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Anchor {
    /// The left edge of the HUD.
    Left,
    /// The right edge of the HUD.
    Right,
    /// The horizontal centre of the HUD.
    Center,
}

impl From<HudAnchor> for Anchor {
    fn from(anchor: HudAnchor) -> Self {
        match anchor {
            HudAnchor::Left => Self::Left,
            HudAnchor::Right => Self::Right,
            HudAnchor::Center => Self::Center,
        }
    }
}

impl Anchor {
    /// The reference x of the anchor itself.
    const fn reference_x(self) -> f32 {
        match self {
            Self::Left => 0.0,
            Self::Right => 1920.0,
            Self::Center => 960.0,
        }
    }
}

/// The mapping of reference pixels onto source pixels (the part of
/// [`HudGeometry`] the screen detectors need).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenMap {
    sx: f32,
    sy: f32,
    left: f32,
    right: f32,
    center: f32,
    oy: f32,
}

impl ScreenMap {
    /// The identity mapping of a 1920 × 1080 recording.
    pub const REFERENCE: Self = Self {
        sx: 1.0,
        sy: 1.0,
        left: 0.0,
        right: 1920.0,
        center: 960.0,
        oy: 0.0,
    };

    /// The mapping of a 16:9 recording `width` × `height` pixels.
    pub fn wide(width: u32, height: u32) -> Self {
        let s = height as f32 / 1080.0;
        Self {
            sx: s,
            sy: s,
            left: 0.0,
            right: width as f32,
            center: width as f32 * 0.5,
            oy: 0.0,
        }
    }

    /// Horizontal scale from reference to source pixels.
    pub const fn sx(&self) -> f32 {
        self.sx
    }

    /// Vertical scale from reference to source pixels.
    pub const fn sy(&self) -> f32 {
        self.sy
    }

    /// Source x of reference x `x` for an element with `anchor`.
    pub fn x(&self, x: f32, anchor: Anchor) -> f32 {
        let origin = match anchor {
            Anchor::Left => self.left,
            Anchor::Right => self.right,
            Anchor::Center => self.center,
        };
        origin + (x - anchor.reference_x()) * self.sx
    }

    /// Source y of reference y `y`.
    pub fn y(&self, y: f32) -> f32 {
        self.oy + y * self.sy
    }

    /// A reference rectangle in source pixels, rounded outward.
    pub fn to_source(&self, r: RectI, anchor: Anchor) -> RectI {
        let x0 = self.x(r.x0 as f32, anchor).floor() as i32;
        let x1 = self.x(r.x1 as f32, anchor).ceil() as i32;
        let y0 = self.y(r.y0 as f32).floor() as i32;
        let y1 = self.y(r.y1 as f32).ceil() as i32;
        RectI {
            x0,
            y0,
            x1: x1.max(x0 + 1),
            y1: y1.max(y0 + 1),
        }
    }
}

impl From<&HudGeometry> for ScreenMap {
    fn from(g: &HudGeometry) -> Self {
        Self {
            sx: g.sx,
            sy: g.sy,
            left: g.anchor_left,
            right: g.anchor_right,
            center: g.anchor_center,
            oy: g.oy,
        }
    }
}

/// Part of a source frame: a crop at native scale or the whole frame at a
/// reduced scale.
#[derive(Debug, Clone, Copy)]
pub struct FrameView<'a> {
    /// The pixels.
    pub pixels: RgbView<'a>,
    /// Source pixel shown at view pixel `(0, 0)`.
    pub origin: (i32, i32),
    /// View pixels per source pixel (1 for a native crop, 0.25 for the
    /// quarter-scale frame).
    pub scale: f32,
}

impl<'a> FrameView<'a> {
    /// A native-scale view of the whole frame.
    pub const fn native(pixels: RgbView<'a>) -> Self {
        Self {
            pixels,
            origin: (0, 0),
            scale: 1.0,
        }
    }

    /// A native-scale crop whose top-left pixel is source pixel `origin`.
    pub const fn crop(pixels: RgbView<'a>, origin: (i32, i32)) -> Self {
        Self {
            pixels,
            origin,
            scale: 1.0,
        }
    }

    /// The whole frame at `scale` view pixels per source pixel.
    pub const fn scaled(pixels: RgbView<'a>, scale: f32) -> Self {
        Self {
            pixels,
            origin: (0, 0),
            scale,
        }
    }
}

/// A reference rectangle resampled onto a grid of `step` × `step`
/// reference pixels, as colours (`R, G, B` per cell).
///
/// Cells outside the view are black. Each cell is the mean of the view
/// pixels its footprint covers, or the nearest pixel if the footprint is
/// smaller than one view pixel (an upscaled reference grid).
#[derive(Debug, Clone)]
pub struct RefPlane {
    rect: RectI,
    step: i32,
    width: usize,
    height: usize,
    data: Vec<[i16; 3]>,
    edges_x: Vec<(usize, usize)>,
    edges_y: Vec<(usize, usize)>,
    columns: Vec<u32>,
}

impl Default for RefPlane {
    fn default() -> Self {
        Self {
            rect: RectI {
                x0: 0,
                y0: 0,
                x1: 0,
                y1: 0,
            },
            step: 1,
            width: 0,
            height: 0,
            data: Vec::new(),
            edges_x: Vec::new(),
            edges_y: Vec::new(),
            columns: Vec::new(),
        }
    }
}

impl RefPlane {
    /// An empty plane.
    pub fn new() -> Self {
        Self::default()
    }

    /// Resamples `rect` (reference pixels, `anchor`-anchored) of `view`
    /// with cells of `step` reference pixels, reusing the buffers.
    pub fn sample(
        &mut self,
        view: &FrameView<'_>,
        map: &ScreenMap,
        anchor: Anchor,
        rect: RectI,
        step: i32,
    ) {
        let step = step.max(1);
        self.rect = rect;
        self.step = step;
        self.width = ((rect.width() as i32 + step - 1) / step) as usize;
        self.height = ((rect.height() as i32 + step - 1) / step) as usize;
        self.data.clear();
        self.data.resize(self.width * self.height, [0; 3]);
        let (vw, vh) = (view.pixels.width() as i32, view.pixels.height() as i32);
        if vw == 0 || vh == 0 {
            return;
        }
        let to_view_x = |x: i32| (map.x(x as f32, anchor) - view.origin.0 as f32) * view.scale;
        let to_view_y = |y: i32| (map.y(y as f32) - view.origin.1 as f32) * view.scale;
        footprints(
            &mut self.edges_x,
            self.width,
            |i| {
                let x = rect.x0 + step * i as i32;
                (to_view_x(x), to_view_x(x + step))
            },
            vw,
        );
        footprints(
            &mut self.edges_y,
            self.height,
            |i| {
                let y = rect.y0 + step * i as i32;
                (to_view_y(y), to_view_y(y + step))
            },
            vh,
        );
        let width = self.width;
        // The view columns any cell covers (cells outside the view have
        // empty footprints).
        let span = self
            .edges_x
            .iter()
            .filter(|&&(x0, x1)| x1 > x0)
            .fold(None, |acc: Option<(usize, usize)>, &(x0, x1)| {
                Some(acc.map_or((x0, x1), |(a, b)| (a.min(x0), b.max(x1))))
            })
            .unwrap_or((0, 0));
        // Whether every cell covers at most one view column (1080p and
        // below at step 1): then a cell row is a plain copy.
        let single_columns = self.edges_x.iter().all(|&(x0, x1)| x1 - x0 <= 1);
        for (cy, &(y0, y1)) in self.edges_y.iter().enumerate() {
            if y0 >= y1 {
                continue;
            }
            let cells = &mut self.data[cy * width..(cy + 1) * width];
            if y1 - y0 == 1 {
                // One view row per cell row (1080p and below): sum along x
                // only.
                let row = view.pixels.row(y0 as u32);
                if single_columns {
                    for (cell, &(x0, x1)) in cells.iter_mut().zip(&self.edges_x) {
                        if x1 > x0 {
                            let px = &row[3 * x0..3 * x0 + 3];
                            *cell = [px[0].into(), px[1].into(), px[2].into()];
                        }
                    }
                } else {
                    for (cell, &(x0, x1)) in cells.iter_mut().zip(&self.edges_x) {
                        *cell = mean(&row[3 * x0..3 * x1]);
                    }
                }
                continue;
            }
            // Sum the footprint's rows per view column (a plain, vectorised
            // add), then every cell's sum from those column sums: the
            // cell loop runs once per cell row, not once per view row.
            self.columns.clear();
            self.columns.resize(3 * (span.1 - span.0), 0);
            for y in y0..y1 {
                let row = &view.pixels.row(y as u32)[3 * span.0..3 * span.1];
                for (sum, &v) in self.columns.iter_mut().zip(row) {
                    *sum += u32::from(v);
                }
            }
            let h = (y1 - y0) as u32;
            for (cell, &(x0, x1)) in cells.iter_mut().zip(&self.edges_x) {
                let n = h * (x1 - x0) as u32;
                if n > 0 {
                    let mut sum = [0u32; 3];
                    for px in self.columns[3 * (x0 - span.0)..3 * (x1 - span.0)]
                        .as_chunks::<3>()
                        .0
                    {
                        sum = [sum[0] + px[0], sum[1] + px[1], sum[2] + px[2]];
                    }
                    *cell = sum.map(|s| ((s + n / 2) / n) as i16);
                }
            }
        }
    }

    /// The sampled reference rectangle.
    pub const fn rect(&self) -> RectI {
        self.rect
    }

    /// Reference pixels per cell.
    pub const fn step(&self) -> i32 {
        self.step
    }

    /// Width in cells.
    pub const fn width(&self) -> usize {
        self.width
    }

    /// Height in cells.
    pub const fn height(&self) -> usize {
        self.height
    }

    /// The colour of cell `(cx, cy)`, or black outside the plane.
    pub fn at(&self, cx: i32, cy: i32) -> [i16; 3] {
        if cx < 0 || cy < 0 || cx as usize >= self.width || cy as usize >= self.height {
            return [0; 3];
        }
        self.data[cy as usize * self.width + cx as usize]
    }

    /// The cells of row `cy`, or `None` outside the plane.
    pub fn row(&self, cy: i32) -> Option<&[[i16; 3]]> {
        (cy >= 0 && (cy as usize) < self.height)
            .then(|| &self.data[cy as usize * self.width..(cy as usize + 1) * self.width])
    }

    /// The cell containing reference pixel `(x, y)` (may lie outside).
    pub fn cell(&self, x: i32, y: i32) -> (i32, i32) {
        (
            (x - self.rect.x0).div_euclid(self.step),
            (y - self.rect.y0).div_euclid(self.step),
        )
    }

    /// Luminance-like value of a cell: the mean of its three channels.
    pub fn luma(&self, cx: i32, cy: i32) -> i16 {
        let [r, g, b] = self.at(cx, cy);
        (r + g + b) / 3
    }
}

/// The mean colour of packed RGB pixels (black for none).
fn mean(pixels: &[u8]) -> [i16; 3] {
    let n = (pixels.len() / 3) as u32;
    if n == 1 {
        return [pixels[0].into(), pixels[1].into(), pixels[2].into()];
    }
    if n == 0 {
        return [0; 3];
    }
    let mut sum = [0u32; 3];
    for px in pixels.as_chunks::<3>().0 {
        sum[0] += u32::from(px[0]);
        sum[1] += u32::from(px[1]);
        sum[2] += u32::from(px[2]);
    }
    sum.map(|s| ((s + n / 2) / n) as i16)
}

/// The view pixel range `[a, b)` every cell covers, clamped to the view;
/// an empty footprint (an upscaled grid) takes the nearest pixel, a cell
/// outside the view gets an empty range.
fn footprints(
    out: &mut Vec<(usize, usize)>,
    cells: usize,
    bounds: impl Fn(usize) -> (f32, f32),
    size: i32,
) {
    out.clear();
    for i in 0..cells {
        let (a, b) = bounds(i);
        let (mut lo, mut hi) = (a.round() as i32, b.round() as i32);
        if hi <= lo {
            lo = f32::midpoint(a, b).floor() as i32;
            hi = lo + 1;
        }
        let (lo, hi) = (lo.clamp(0, size), hi.clamp(0, size));
        out.push((lo as usize, hi.max(lo) as usize));
    }
}

#[cfg(test)]
mod tests {
    use blurrd_core::RectI;

    use super::{Anchor, FrameView, RefPlane, ScreenMap};
    use crate::image::RgbView;

    #[test]
    fn maps_anchors_like_the_layout() {
        let m = ScreenMap::wide(2560, 1440);
        assert!((m.x(1900.0, Anchor::Right) - (2560.0 - 20.0 * 4.0 / 3.0)).abs() < 1e-3);
        assert!((m.x(960.0, Anchor::Center) - 1280.0).abs() < 1e-3);
        let r = m.to_source(RectI::new(0, 0, 3, 3).unwrap(), Anchor::Left);
        assert_eq!(r, RectI::new(0, 0, 4, 4).unwrap());
    }

    #[test]
    fn samples_averages_and_upscales() {
        // A 4 × 2 frame: left half 0, right half 200.
        let mut data = vec![];
        for _ in 0..2 {
            for x in 0..4 {
                let v = if x < 2 { 0 } else { 200 };
                data.extend([v, v, v]);
            }
        }
        let view = FrameView::native(RgbView::new(&data, 4, 2, 12).unwrap());
        let mut plane = RefPlane::new();
        // Reference = source: two cells of 2 × 2 px.
        plane.sample(
            &view,
            &ScreenMap::REFERENCE,
            Anchor::Left,
            RectI::new(0, 0, 4, 2).unwrap(),
            2,
        );
        assert_eq!((plane.width(), plane.height()), (2, 1));
        assert_eq!(plane.at(0, 0), [0; 3]);
        assert_eq!(plane.at(1, 0), [200; 3]);
        assert_eq!(plane.at(5, 0), [0; 3]);
        // A quarter-scale view of an 8 × 4 source maps 4 reference px to
        // one view pixel; reference step 1 picks the nearest pixel.
        let quarter = FrameView::scaled(RgbView::new(&data, 4, 2, 12).unwrap(), 0.5);
        plane.sample(
            &quarter,
            &ScreenMap::REFERENCE,
            Anchor::Left,
            RectI::new(0, 0, 8, 4).unwrap(),
            1,
        );
        assert_eq!(plane.at(3, 0), [0; 3]);
        assert_eq!(plane.at(4, 0), [200; 3]);
        assert_eq!(plane.cell(5, 2), (5, 2));
    }
}
