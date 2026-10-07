//! Boxes resolved for one frame size: padding, alignment and the soft mask.

use blurrd_core::{BlurBox, RectI};

use crate::PixelParams;

/// A [`BlurBox`] resolved for one frame size by [`prepare_boxes`].
///
/// It knows which pixels it may write (the padded box, aligned outward to
/// even coordinates for 4:2:0 chroma and clamped to the frame) and how
/// strongly each pixel is blurred (the mask, see [`PreparedBox::coverage`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PreparedBox {
    /// Pixels the blur reads and writes, in luma pixels.
    padded: RectI,
    /// The unpadded box clamped to the frame; always fully covered.
    core: RectI,
    /// Left, top, right and bottom edge of the padded rounded rectangle
    /// before alignment and clamping.
    shape: [i32; 4],
    /// Corner radius of the shape, at most half its smaller side.
    radius: f32,
    /// Width of the soft edge inside the shape.
    feather: f32,
    /// Origin of the block grid in luma pixels.
    pub(crate) grid_origin: (i32, i32),
    /// Block size in luma pixels, including the box's block scale.
    pub(crate) block: f32,
    /// Seed of the deterministic noise.
    pub(crate) seed: u32,
}

impl PreparedBox {
    /// The pixels the blur may change: the padded box aligned outward to
    /// even coordinates and clamped to the frame.
    pub fn padded(&self) -> RectI {
        self.padded
    }

    /// The unpadded box clamped to the frame. Every pixel of it is covered
    /// with full strength.
    pub fn core(&self) -> RectI {
        self.core
    }

    /// Block size in luma pixels (strength × HUD scale × block scale).
    pub fn block(&self) -> f32 {
        self.block
    }

    /// Origin of the block grid in luma pixels (follows the text).
    pub fn grid_origin(&self) -> (i32, i32) {
        self.grid_origin
    }

    /// Seed of the deterministic per-block noise (constant over a track).
    pub fn seed(&self) -> u32 {
        self.seed
    }

    /// Mask value of luma pixel `(x, y)` in `0.0..=1.0`: how much of the
    /// blurred value replaces the source there.
    ///
    /// Exactly 1.0 for every pixel of [`PreparedBox::core`], falling to 0.0
    /// across the padding (see [`mask_value`]) and 0.0 outside
    /// [`PreparedBox::padded`].
    pub fn coverage(&self, x: i32, y: i32) -> f32 {
        self.pixel_mask(self.column(x), self.row(y))
    }

    /// The mask terms of column `x` (see [`AxisMask`]).
    pub(crate) fn column(&self, x: i32) -> AxisMask {
        let [x0, _, x1, _] = self.shape;
        self.axis(
            x,
            (self.padded.x0, self.padded.x1),
            (self.core.x0, self.core.x1),
            (x0, x1),
        )
    }

    /// The mask terms of row `y` (see [`AxisMask`]).
    pub(crate) fn row(&self, y: i32) -> AxisMask {
        let [_, y0, _, y1] = self.shape;
        self.axis(
            y,
            (self.padded.y0, self.padded.y1),
            (self.core.y0, self.core.y1),
            (y0, y1),
        )
    }

    /// The mask terms of sample `v` along one axis, given the padded, core
    /// and shape extents along that axis.
    fn axis(&self, v: i32, padded: (i32, i32), core: (i32, i32), shape: (i32, i32)) -> AxisMask {
        if v < padded.0 || v >= padded.1 {
            return AxisMask::OUTSIDE;
        }
        // Coordinates relative to the start of the padded box: exact in
        // f32, and the same for a box moved by whole pixels.
        let local = |c: i32| (i64::from(c) - i64::from(padded.0)) as f32;
        let (s0, s1) = (local(shape.0), local(shape.1));
        // Distance from the shape's centre minus the half length of its
        // straight part: positive in the corner bands.
        let q = (local(v) + 0.5 - f32::midpoint(s0, s1)).abs() - ((s1 - s0) * 0.5 - self.radius);
        let core = v >= core.0 && v < core.1;
        AxisMask {
            mask: if core {
                1.0
            } else {
                mask_value(q - self.radius, self.feather)
            },
            q,
            corner: q > 0.0,
            core,
        }
    }

    /// The mask of the pixel at a column and a row.
    ///
    /// Away from the rounded corners the signed distance of a rounded
    /// rectangle is the larger of the two axis distances, so the mask is
    /// the smaller of the column and row masks there; only pixels in both a
    /// corner column and a corner row need the full distance. Pixels of the
    /// unpadded box are always 1.0.
    pub(crate) fn pixel_mask(&self, column: AxisMask, row: AxisMask) -> f32 {
        if column.core && row.core {
            1.0
        } else if column.corner && row.corner {
            // Most corner pixels lie fully outside the rounded corner or
            // fully inside its soft edge; decide those without a square
            // root.
            let squared = column.q * column.q + row.q * row.q;
            let inner = self.radius - self.feather;
            if squared >= self.radius * self.radius {
                0.0
            } else if inner >= 0.0 && squared <= inner * inner {
                1.0
            } else {
                mask_value(squared.sqrt() - self.radius, self.feather)
            }
        } else {
            column.mask.min(row.mask)
        }
    }

    /// Everything the mask and the block grid of the box depend on,
    /// relative to the top-left corner of the padded box: equal keys mean
    /// equal masks and grids, wherever the boxes are. The seed is not part
    /// of it.
    pub(crate) fn shape_key(&self) -> [i64; 15] {
        let (x, y) = (i64::from(self.padded.x0), i64::from(self.padded.y0));
        let [s0, s1, s2, s3] = self.shape.map(i64::from);
        [
            i64::from(self.padded.x1) - x,
            i64::from(self.padded.y1) - y,
            i64::from(self.core.x0) - x,
            i64::from(self.core.y0) - y,
            i64::from(self.core.x1) - x,
            i64::from(self.core.y1) - y,
            s0 - x,
            s1 - y,
            s2 - x,
            s3 - y,
            i64::from(self.grid_origin.0) - x,
            i64::from(self.grid_origin.1) - y,
            i64::from(self.radius.to_bits()),
            i64::from(self.feather.to_bits()),
            i64::from(self.block.to_bits()),
        ]
    }

    /// Mask value of chroma pixel `(cx, cy)`: the maximum of the luma mask
    /// over the 2×2 luma pixels it covers.
    ///
    /// A max-pool (not an average) keeps every chroma sample that touches a
    /// fully covered luma pixel fully covered too.
    pub fn chroma_coverage(&self, cx: i32, cy: i32) -> f32 {
        let (x, y) = (2 * cx, 2 * cy);
        self.coverage(x, y)
            .max(self.coverage(x + 1, y))
            .max(self.coverage(x, y + 1))
            .max(self.coverage(x + 1, y + 1))
    }
}

/// The terms of a box's mask along one axis at one column or row.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct AxisMask {
    /// The mask along this axis alone (1.0 inside the unpadded box).
    pub(crate) mask: f32,
    /// Distance from the shape's centre minus the half length of the
    /// shape's straight part.
    q: f32,
    /// Whether the sample lies in a corner band (`q > 0`).
    pub(crate) corner: bool,
    /// Whether the sample lies inside the unpadded box along this axis.
    core: bool,
}

impl AxisMask {
    /// A column or row outside the padded box: mask 0.
    pub(crate) const OUTSIDE: Self = Self {
        mask: 0.0,
        q: f32::INFINITY,
        corner: false,
        core: false,
    };
}

/// The soft edge: mask value for a signed distance `sdf` to the edge of the
/// padded rounded rectangle (negative inside).
///
/// 0.0 on and outside the edge, 1.0 from `feather` pixels inside it, and a
/// smoothstep in between. A feather of 0 gives a hard edge.
pub fn mask_value(sdf: f32, feather: f32) -> f32 {
    if feather <= 0.0 {
        return if sdf < 0.0 { 1.0 } else { 0.0 };
    }
    let t = (-sdf / feather).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Resolves boxes for a `frame_w` × `frame_h` frame.
///
/// Each box is padded by [`PixelParams::pad`], aligned outward to even
/// coordinates (floor for the left and top edges, ceiling for the right and
/// bottom edges) and clamped to the frame. Boxes that do not overlap the
/// frame are dropped. A box's block scale below 1.0 (or not a number) is
/// treated as 1.0, because a smaller block would weaken the blur.
///
/// `out` is cleared first, so one buffer can be reused for every frame.
pub fn prepare_boxes(
    boxes: &[BlurBox],
    frame_w: u32,
    frame_h: u32,
    params: &PixelParams,
    out: &mut Vec<PreparedBox>,
) {
    out.clear();
    let (w, h) = (
        i32::try_from(frame_w).unwrap_or(i32::MAX),
        i32::try_from(frame_h).unwrap_or(i32::MAX),
    );
    let Ok(frame) = RectI::new(0, 0, w, h) else {
        return;
    };
    let (pad_x, pad_y) = params.pad;
    out.extend(boxes.iter().filter_map(|b| {
        let core = b.rect.intersection(&frame)?;
        let shape = [
            b.rect.x0.saturating_sub(pad_x),
            b.rect.y0.saturating_sub(pad_y),
            b.rect.x1.saturating_add(pad_x),
            b.rect.y1.saturating_add(pad_y),
        ];
        let aligned = RectI::new(
            shape[0] & !1,
            shape[1] & !1,
            shape[2].saturating_add(1) & !1,
            shape[3].saturating_add(1) & !1,
        )
        .ok()?;
        let padded = aligned.intersection(&frame)?;
        let half_min = (shape[2] - shape[0]).min(shape[3] - shape[1]) as f32 * 0.5;
        let scale = if b.block_scale.is_finite() {
            b.block_scale.max(1.0)
        } else {
            1.0
        };
        Some(PreparedBox {
            padded,
            core,
            shape,
            radius: params.radius.min(half_min),
            feather: params.feather,
            grid_origin: b.grid_origin,
            block: params.block * scale,
            seed: b.seed,
        })
    }));
}

#[cfg(test)]
mod tests {
    use blurrd_core::{BlurBox, BlurParams, RectI, Region};

    use super::{PreparedBox, mask_value, prepare_boxes};
    use crate::{PixelParams, YuvColour};

    fn blur_box(x0: i32, y0: i32, x1: i32, y1: i32) -> BlurBox {
        BlurBox {
            rect: RectI::new(x0, y0, x1, y1).unwrap(),
            region: Region::KillFeed,
            seed: 7,
            grid_origin: (x0, y0),
            block_scale: 1.0,
        }
    }

    fn prepare(boxes: &[BlurBox], w: u32, h: u32) -> Vec<PreparedBox> {
        let params = PixelParams::new(&BlurParams::default(), 1.0, YuvColour::default()).unwrap();
        let mut out = vec![];
        prepare_boxes(boxes, w, h, &params, &mut out);
        out
    }

    #[test]
    fn pads_and_aligns_outward() {
        let [b] = prepare(&[blur_box(101, 51, 160, 70)], 1920, 1080)[..] else {
            panic!("one box expected");
        };
        // Default padding (4, 3): shape [97, 48, 164, 73] -> even [96, 48, 164, 74].
        assert_eq!(b.padded(), RectI::new(96, 48, 164, 74).unwrap());
        assert_eq!(b.core(), RectI::new(101, 51, 160, 70).unwrap());
    }

    #[test]
    fn clamps_to_the_frame_and_drops_outside_boxes() {
        let boxes = prepare(
            &[blur_box(-10, -10, 20, 20), blur_box(2000, 0, 2100, 30)],
            1921,
            1080,
        );
        assert_eq!(boxes.len(), 1);
        assert_eq!(boxes[0].padded(), RectI::new(0, 0, 24, 24).unwrap());
        assert_eq!(boxes[0].core(), RectI::new(0, 0, 20, 20).unwrap());
        let edge = prepare(&[blur_box(1900, 0, 1921, 30)], 1921, 1080);
        assert_eq!(edge[0].padded().x1, 1921, "odd frame width is kept");
    }

    #[test]
    fn covers_the_core_and_fades_in_the_padding() {
        let b = prepare(&[blur_box(100, 50, 160, 70)], 1920, 1080)[0];
        for y in 50..70 {
            for x in 100..160 {
                assert!((b.coverage(x, y) - 1.0).abs() < f32::EPSILON);
            }
        }
        // Shape edge at x = 96 with a 1.5 px feather: pixel 96 (centre 96.5)
        // is partly covered, pixel 97 (centre 97.5) fully.
        assert!(b.coverage(96, 60) > 0.0 && b.coverage(96, 60) < 1.0);
        assert!((b.coverage(97, 60) - 1.0).abs() < f32::EPSILON);
        assert!(
            b.coverage(95, 60).abs() < f32::EPSILON,
            "outside the padded box"
        );
        assert!((b.chroma_coverage(49, 25) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn treats_small_block_scales_as_one() {
        let mut b = blur_box(0, 0, 10, 10);
        b.block_scale = f32::NAN;
        assert!((prepare(&[b], 64, 64)[0].block() - 10.0).abs() < 1e-6);
        b.block_scale = 1.5;
        assert!((prepare(&[b], 64, 64)[0].block() - 15.0).abs() < 1e-6);
    }

    #[test]
    fn mask_is_a_smoothstep() {
        assert!(mask_value(0.0, 1.5).abs() < f32::EPSILON);
        assert!((mask_value(-1.5, 1.5) - 1.0).abs() < f32::EPSILON);
        assert!((mask_value(-0.75, 1.5) - 0.5).abs() < 1e-6);
        assert!((mask_value(-0.1, 0.0) - 1.0).abs() < f32::EPSILON);
        assert!(mask_value(0.0, 0.0).abs() < f32::EPSILON);
    }
}
