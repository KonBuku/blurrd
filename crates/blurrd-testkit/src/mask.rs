//! Binary pixel masks, the ground truth of where a name's ink is.

use blurrd_core::RectI;

/// A set of pixels inside a bounding rectangle, in frame coordinates.
///
/// Ground-truth glyph masks are stored this way: every pixel a name's
/// glyphs (including anti-aliasing, outline and shadow) touch, dilated by
/// the halo the metrics require.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mask {
    /// The rectangle the bits cover.
    area: RectI,
    /// One flag per pixel of `area`, row after row.
    bits: Vec<bool>,
}

impl Mask {
    /// An empty mask over `area`.
    pub fn empty(area: RectI) -> Self {
        Self {
            area,
            bits: vec![false; area.area() as usize],
        }
    }

    /// The mask of the pixels of a `width` × `height` coverage map placed
    /// at `origin` whose coverage is above zero.
    ///
    /// Returns `None` if no pixel is covered.
    pub fn from_coverage(origin: (i32, i32), width: u32, coverage: &[f32]) -> Option<Self> {
        if width == 0 || coverage.is_empty() {
            return None;
        }
        let height = coverage.len() / width as usize;
        let area = RectI::new(
            origin.0,
            origin.1,
            origin.0 + width as i32,
            origin.1 + height as i32,
        )
        .ok()?;
        let bits: Vec<bool> = coverage[..width as usize * height]
            .iter()
            .map(|&c| c > 0.0)
            .collect();
        Self { area, bits }.trimmed()
    }

    /// The rectangle the mask is stored over (not necessarily tight).
    pub const fn area(&self) -> RectI {
        self.area
    }

    /// Whether pixel `(x, y)` is in the mask.
    pub fn contains(&self, x: i32, y: i32) -> bool {
        let a = self.area;
        x >= a.x0
            && x < a.x1
            && y >= a.y0
            && y < a.y1
            && self.bits[((y - a.y0) as u32 * a.width() + (x - a.x0) as u32) as usize]
    }

    /// Adds pixel `(x, y)`; ignored outside [`Mask::area`].
    pub fn insert(&mut self, x: i32, y: i32) {
        let a = self.area;
        if x >= a.x0 && x < a.x1 && y >= a.y0 && y < a.y1 {
            self.bits[((y - a.y0) as u32 * a.width() + (x - a.x0) as u32) as usize] = true;
        }
    }

    /// Number of pixels in the mask.
    pub fn count(&self) -> u64 {
        self.bits.iter().filter(|&&b| b).count() as u64
    }

    /// The pixels in the mask, row by row.
    pub fn pixels(&self) -> impl Iterator<Item = (i32, i32)> + '_ {
        let a = self.area;
        let w = a.width() as usize;
        self.bits
            .iter()
            .enumerate()
            .filter(|(_, b)| **b)
            .map(move |(i, _)| (a.x0 + (i % w) as i32, a.y0 + (i / w) as i32))
    }

    /// The tight bounding rectangle of the pixels, or `None` if empty.
    pub fn bounds(&self) -> Option<RectI> {
        let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for (x, y) in self.pixels() {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x + 1);
            y1 = y1.max(y + 1);
        }
        RectI::new(x0, y0, x1, y1).ok()
    }

    /// The same pixels stored over their tight bounds, or `None` if empty.
    fn trimmed(self) -> Option<Self> {
        let bounds = self.bounds()?;
        if bounds == self.area {
            return Some(self);
        }
        let mut out = Self::empty(bounds);
        for (x, y) in self.pixels() {
            out.insert(x, y);
        }
        Some(out)
    }

    /// The mask grown by `r` pixels in every direction (a square
    /// structuring element of side `2r + 1`, which contains the disk of
    /// radius `r`, so the result is conservative).
    #[must_use]
    pub fn dilated(&self, r: u32) -> Self {
        let r = r as i32;
        let a = self.area;
        let grown = RectI {
            x0: a.x0 - r,
            y0: a.y0 - r,
            x1: a.x1 + r,
            y1: a.y1 + r,
        };
        // Separable: dilate rows into `wide`, then columns into `out`.
        let mut wide = Self::empty(grown);
        for (x, y) in self.pixels() {
            for dx in -r..=r {
                wide.insert(x + dx, y);
            }
        }
        let mut out = Self::empty(grown);
        for (x, y) in wide.pixels() {
            for dy in -r..=r {
                out.insert(x, y + dy);
            }
        }
        out
    }

    /// The union of two masks.
    #[must_use]
    pub fn union(&self, other: &Self) -> Self {
        let (a, b) = (self.area, other.area);
        let area = RectI {
            x0: a.x0.min(b.x0),
            y0: a.y0.min(b.y0),
            x1: a.x1.max(b.x1),
            y1: a.y1.max(b.y1),
        };
        let mut out = Self::empty(area);
        for (x, y) in self.pixels().chain(other.pixels()) {
            out.insert(x, y);
        }
        out
    }

    /// The pixels inside `rect`, or `None` if none remain.
    pub fn clipped(&self, rect: RectI) -> Option<Self> {
        let area = self.area.intersection(&rect)?;
        let mut out = Self::empty(area);
        for (x, y) in self.pixels() {
            out.insert(x, y);
        }
        out.trimmed()
    }
}

#[cfg(test)]
mod tests {
    use blurrd_core::RectI;

    use super::Mask;

    fn dot(x: i32, y: i32) -> Mask {
        let mut m = Mask::empty(RectI::new(x, y, x + 1, y + 1).unwrap());
        m.insert(x, y);
        m
    }

    #[test]
    fn builds_from_coverage_and_trims() {
        let cov = [0.0, 0.0, 0.0, 0.0, 0.3, 0.0, 0.0, 1.0, 0.0];
        let m = Mask::from_coverage((10, 20), 3, &cov).unwrap();
        assert_eq!(m.area(), RectI::new(11, 21, 12, 23).unwrap());
        assert_eq!(m.count(), 2);
        assert!(m.contains(11, 22) && !m.contains(10, 20));
        assert!(Mask::from_coverage((0, 0), 3, &[0.0; 9]).is_none());
    }

    #[test]
    fn dilates_with_a_square() {
        let m = dot(5, 5).dilated(2);
        assert_eq!(m.count(), 25);
        assert_eq!(m.bounds(), RectI::new(3, 3, 8, 8).ok());
        assert!(m.contains(3, 7) && !m.contains(2, 5));
    }

    #[test]
    fn unions_and_clips() {
        let u = dot(0, 0).union(&dot(4, 2));
        assert_eq!(u.count(), 2);
        assert_eq!(u.bounds(), RectI::new(0, 0, 5, 3).ok());
        let c = u.clipped(RectI::new(2, 0, 10, 10).unwrap()).unwrap();
        assert_eq!(c.pixels().collect::<Vec<_>>(), [(4, 2)]);
        assert!(u.clipped(RectI::new(1, 0, 3, 1).unwrap()).is_none());
    }
}
