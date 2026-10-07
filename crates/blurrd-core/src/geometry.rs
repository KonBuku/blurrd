//! Integer rectangles in source pixel space.

use std::fmt;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// An axis-aligned, half-open rectangle in source pixels.
///
/// The rectangle covers the pixels with `x0 <= x < x1` and `y0 <= y < y1`,
/// so `x1 - x0` is its width. A `RectI` is never empty: [`RectI::new`] and
/// deserialisation reject rectangles without area. In JSON it is written as
/// `[x0, y0, x1, y1]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(try_from = "[i32; 4]", into = "[i32; 4]")]
#[ts(as = "[i32; 4]")]
pub struct RectI {
    /// Left edge (inclusive).
    pub x0: i32,
    /// Top edge (inclusive).
    pub y0: i32,
    /// Right edge (exclusive).
    pub x1: i32,
    /// Bottom edge (exclusive).
    pub y1: i32,
}

/// The error returned for a rectangle without area (`x1 <= x0` or `y1 <= y0`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("empty rectangle [{x0}, {y0}, {x1}, {y1}]: right and bottom must exceed left and top")]
pub struct EmptyRectError {
    /// Left edge of the rejected rectangle.
    pub x0: i32,
    /// Top edge of the rejected rectangle.
    pub y0: i32,
    /// Right edge of the rejected rectangle.
    pub x1: i32,
    /// Bottom edge of the rejected rectangle.
    pub y1: i32,
}

impl RectI {
    /// Creates a rectangle from its edges.
    ///
    /// # Errors
    ///
    /// Returns [`EmptyRectError`] when the rectangle has no area.
    pub const fn new(x0: i32, y0: i32, x1: i32, y1: i32) -> Result<Self, EmptyRectError> {
        if x1 > x0 && y1 > y0 {
            Ok(Self { x0, y0, x1, y1 })
        } else {
            Err(EmptyRectError { x0, y0, x1, y1 })
        }
    }

    /// Width in pixels.
    pub const fn width(&self) -> u32 {
        self.x1.abs_diff(self.x0)
    }

    /// Height in pixels.
    pub const fn height(&self) -> u32 {
        self.y1.abs_diff(self.y0)
    }

    /// Number of pixels covered.
    pub const fn area(&self) -> u64 {
        self.width() as u64 * self.height() as u64
    }

    /// The overlapping part of two rectangles, or `None` if they do not
    /// overlap.
    pub fn intersection(&self, other: &Self) -> Option<Self> {
        Self::new(
            self.x0.max(other.x0),
            self.y0.max(other.y0),
            self.x1.min(other.x1),
            self.y1.min(other.y1),
        )
        .ok()
    }

    /// Intersection over union, in `0.0..=1.0`.
    pub fn iou(&self, other: &Self) -> f32 {
        let Some(overlap) = self.intersection(other) else {
            return 0.0;
        };
        let inter = overlap.area();
        let union = self.area() + other.area() - inter;
        (inter as f64 / union as f64) as f32
    }

    /// Whether the rectangle lies completely inside a `width` × `height`
    /// frame whose top-left pixel is `(0, 0)`.
    pub fn is_inside(&self, width: u32, height: u32) -> bool {
        self.x0 >= 0
            && self.y0 >= 0
            && i64::from(self.x1) <= i64::from(width)
            && i64::from(self.y1) <= i64::from(height)
    }
}

impl TryFrom<[i32; 4]> for RectI {
    type Error = EmptyRectError;

    fn try_from([x0, y0, x1, y1]: [i32; 4]) -> Result<Self, Self::Error> {
        Self::new(x0, y0, x1, y1)
    }
}

impl From<RectI> for [i32; 4] {
    fn from(r: RectI) -> Self {
        [r.x0, r.y0, r.x1, r.y1]
    }
}

impl fmt::Display for RectI {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}, {}, {}, {}]", self.x0, self.y0, self.x1, self.y1)
    }
}

#[cfg(test)]
mod tests {
    use super::RectI;

    fn rect(x0: i32, y0: i32, x1: i32, y1: i32) -> RectI {
        RectI::new(x0, y0, x1, y1).unwrap()
    }

    #[test]
    fn rejects_empty_rectangles() {
        assert!(RectI::new(0, 0, 0, 10).is_err());
        assert!(RectI::new(0, 0, 10, -1).is_err());
        assert!(RectI::new(5, 5, 6, 6).is_ok());
    }

    #[test]
    fn measures_half_open_extent() {
        let r = rect(10, 20, 30, 25);
        assert_eq!((r.width(), r.height(), r.area()), (20, 5, 100));
    }

    #[test]
    fn computes_iou() {
        let a = rect(0, 0, 10, 10);
        assert!((a.iou(&a) - 1.0).abs() < f32::EPSILON);
        assert!(
            a.iou(&rect(10, 0, 20, 10)).abs() < f32::EPSILON,
            "touching edges do not overlap"
        );
        // Overlap 5x10 = 50, union 150.
        assert!((a.iou(&rect(5, 0, 15, 10)) - 1.0 / 3.0).abs() < 1e-6);
    }

    #[test]
    fn checks_frame_bounds() {
        assert!(rect(0, 0, 1920, 1080).is_inside(1920, 1080));
        assert!(!rect(0, 0, 1921, 1080).is_inside(1920, 1080));
        assert!(!rect(-1, 0, 10, 10).is_inside(1920, 1080));
    }

    #[test]
    fn serialises_as_array() {
        let r = rect(1782, 140, 1835, 163);
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(json, "[1782,140,1835,163]");
        assert_eq!(serde_json::from_str::<RectI>(&json).unwrap(), r);
        assert!(serde_json::from_str::<RectI>("[5,5,5,9]").is_err());
    }
}
