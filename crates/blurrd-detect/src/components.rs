//! Connected components, runs and summed-area tables of binary masks.

use blurrd_core::RectI;

use crate::image::BitMask;

/// Which neighbours connect two pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Connectivity {
    /// Left, right, up and down.
    Four,
    /// The four plus the diagonals (OpenCV's default for text).
    Eight,
}

/// One connected component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Component {
    /// Bounding box (half-open) in mask coordinates.
    pub bounds: RectI,
    /// Number of pixels.
    pub area: u32,
}

/// Reusable buffers for [`components`].
#[derive(Debug, Clone, Default)]
pub struct ComponentScratch {
    seen: Vec<bool>,
    stack: Vec<(i32, i32)>,
}

/// The connected components of the set pixels of `mask` inside `roi`, as if
/// the mask were cropped to `roi` (pixels outside it connect nothing).
///
/// Components are returned in raster order of their first pixel (top to
/// bottom, then left to right), like OpenCV's labels. `out` is cleared
/// first.
pub fn components(
    mask: &BitMask<'_>,
    roi: RectI,
    connectivity: Connectivity,
    scratch: &mut ComponentScratch,
    out: &mut Vec<Component>,
) {
    out.clear();
    let Some(roi) = mask.bounds().and_then(|b| b.intersection(&roi)) else {
        return;
    };
    let (w, h) = (roi.width() as usize, roi.height() as usize);
    scratch.seen.clear();
    scratch.seen.resize(w * h, false);
    let index = |x: i32, y: i32| (y - roi.y0) as usize * w + (x - roi.x0) as usize;
    let neighbours: &[(i32, i32)] = match connectivity {
        Connectivity::Four => &[(1, 0), (-1, 0), (0, 1), (0, -1)],
        Connectivity::Eight => &[
            (1, 0),
            (-1, 0),
            (0, 1),
            (0, -1),
            (1, 1),
            (1, -1),
            (-1, 1),
            (-1, -1),
        ],
    };
    for y in roi.y0..roi.y1 {
        for x in roi.x0..roi.x1 {
            if scratch.seen[index(x, y)] || !mask.get(x, y) {
                continue;
            }
            scratch.seen[index(x, y)] = true;
            scratch.stack.push((x, y));
            let (mut x0, mut y0, mut x1, mut y1, mut area) = (x, y, x + 1, y + 1, 0u32);
            while let Some((px, py)) = scratch.stack.pop() {
                area += 1;
                x0 = x0.min(px);
                y0 = y0.min(py);
                x1 = x1.max(px + 1);
                y1 = y1.max(py + 1);
                for &(dx, dy) in neighbours {
                    let (nx, ny) = (px + dx, py + dy);
                    let inside = nx >= roi.x0 && nx < roi.x1 && ny >= roi.y0 && ny < roi.y1;
                    if inside && !scratch.seen[index(nx, ny)] && mask.get(nx, ny) {
                        scratch.seen[index(nx, ny)] = true;
                        scratch.stack.push((nx, ny));
                    }
                }
            }
            out.push(Component {
                bounds: RectI { x0, y0, x1, y1 },
                area,
            });
        }
    }
}

/// A half-open run `start..end` of consecutive indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Run {
    /// First index.
    pub start: u32,
    /// One past the last index.
    pub end: u32,
}

impl Run {
    /// Number of indices in the run.
    pub const fn len(&self) -> u32 {
        self.end - self.start
    }

    /// Whether the run is empty.
    pub const fn is_empty(&self) -> bool {
        self.end == self.start
    }
}

/// The maximal runs of `true` in `values`.
pub fn runs(values: impl IntoIterator<Item = bool>) -> Vec<Run> {
    let mut out = Vec::new();
    let mut start = None;
    let mut i = 0u32;
    for v in values {
        match (v, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                out.push(Run { start: s, end: i });
                start = None;
            }
            _ => {}
        }
        i += 1;
    }
    if let Some(s) = start {
        out.push(Run { start: s, end: i });
    }
    out
}

/// Merges runs separated by at most `gap` indices (runs must be sorted and
/// disjoint, as [`runs`] returns them).
pub fn merge_runs(runs: &[Run], gap: u32) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::with_capacity(runs.len());
    for &r in runs {
        match out.last_mut() {
            Some(last) if r.start.saturating_sub(last.end) <= gap => last.end = last.end.max(r.end),
            _ => out.push(r),
        }
    }
    out
}

/// A summed-area table of a mask: the number of set pixels in any
/// rectangle in constant time.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Integral {
    width: u32,
    height: u32,
    /// `(width + 1) × (height + 1)` prefix sums with a zero first row and
    /// column.
    sums: Vec<u32>,
}

impl Integral {
    /// The table of `mask`.
    pub fn new(mask: &BitMask<'_>) -> Self {
        let mut table = Self::default();
        table.rebuild(mask);
        table
    }

    /// Rebuilds the table for `mask`, reusing the allocation.
    pub fn rebuild(&mut self, mask: &BitMask<'_>) {
        let (w, h) = (mask.width() as usize, mask.height() as usize);
        self.width = mask.width();
        self.height = mask.height();
        self.sums.clear();
        self.sums.resize((w + 1) * (h + 1), 0);
        for y in 0..h {
            let mut row = 0u32;
            for x in 0..w {
                row += u32::from(mask.get(x as i32, y as i32));
                self.sums[(y + 1) * (w + 1) + x + 1] = self.sums[y * (w + 1) + x + 1] + row;
            }
        }
    }

    /// Set pixels inside `rect`, clipped to the mask.
    pub fn sum(&self, rect: RectI) -> u32 {
        let clip = |v: i32, max: u32| v.clamp(0, max as i32) as usize;
        let (x0, x1) = (clip(rect.x0, self.width), clip(rect.x1, self.width));
        let (y0, y1) = (clip(rect.y0, self.height), clip(rect.y1, self.height));
        if x1 <= x0 || y1 <= y0 {
            return 0;
        }
        let stride = self.width as usize + 1;
        let at = |x: usize, y: usize| self.sums[y * stride + x];
        at(x1, y1) + at(x0, y0) - at(x0, y1) - at(x1, y0)
    }

    /// Fraction of set pixels inside `rect` clipped to the mask (0 for an
    /// empty intersection).
    pub fn mean(&self, rect: RectI) -> f32 {
        let clipped = RectI::new(
            rect.x0.max(0),
            rect.y0.max(0),
            rect.x1.min(self.width as i32),
            rect.y1.min(self.height as i32),
        );
        match clipped {
            Ok(r) => self.sum(r) as f32 / r.area() as f32,
            Err(_) => 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use blurrd_core::RectI;

    use super::{ComponentScratch, Connectivity, Integral, Run, components, merge_runs, runs};
    use crate::image::BitMask;

    fn rect(x0: i32, y0: i32, x1: i32, y1: i32) -> RectI {
        RectI::new(x0, y0, x1, y1).unwrap()
    }

    #[test]
    fn labels_diagonals_by_connectivity() {
        #[rustfmt::skip]
        let flags = [
            1, 0, 0, 0,
            0, 1, 0, 1,
            0, 0, 0, 1,
        ];
        let mask = BitMask::new(&flags, 1, 4, 3).unwrap();
        let mut s = ComponentScratch::default();
        let mut out = vec![];
        components(
            &mask,
            rect(0, 0, 4, 3),
            Connectivity::Eight,
            &mut s,
            &mut out,
        );
        assert_eq!(out.len(), 2);
        assert_eq!((out[0].bounds, out[0].area), (rect(0, 0, 2, 2), 2));
        assert_eq!((out[1].bounds, out[1].area), (rect(3, 1, 4, 3), 2));
        components(
            &mask,
            rect(0, 0, 4, 3),
            Connectivity::Four,
            &mut s,
            &mut out,
        );
        assert_eq!(out.len(), 3);
        // Cropping splits nothing here but drops the left pixels.
        components(
            &mask,
            rect(1, 0, 4, 3),
            Connectivity::Eight,
            &mut s,
            &mut out,
        );
        assert_eq!(out.len(), 2);
        components(
            &mask,
            rect(9, 9, 12, 12),
            Connectivity::Eight,
            &mut s,
            &mut out,
        );
        assert_eq!(out, []);
    }

    #[test]
    fn finds_and_merges_runs() {
        let r = runs([false, true, true, false, false, true, false, true]);
        assert_eq!(
            r,
            [
                Run { start: 1, end: 3 },
                Run { start: 5, end: 6 },
                Run { start: 7, end: 8 }
            ]
        );
        assert_eq!(
            merge_runs(&r, 1),
            [Run { start: 1, end: 3 }, Run { start: 5, end: 8 }]
        );
        assert_eq!(merge_runs(&r, 2), [Run { start: 1, end: 8 }]);
        assert_eq!(runs([false, false]), []);
        assert_eq!(r[0].len(), 2);
    }

    #[test]
    fn sums_rectangles() {
        let flags = [1, 1, 0, 0, 1, 0, 1, 1, 1];
        let mask = BitMask::new(&flags, 1, 3, 3).unwrap();
        let t = Integral::new(&mask);
        assert_eq!(t.sum(rect(0, 0, 3, 3)), 6);
        assert_eq!(t.sum(rect(1, 1, 3, 3)), 3);
        assert_eq!(t.sum(rect(-5, -5, 1, 1)), 1);
        assert!((t.mean(rect(0, 2, 9, 9)) - 1.0).abs() < f32::EPSILON);
        assert!(t.mean(rect(5, 5, 9, 9)).abs() < f32::EPSILON);
    }
}
