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
    shape: [f32; 4],
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

    /// Mask value of luma pixel `(x, y)` in `0.0..=1.0`: how much of the
    /// blurred value replaces the source there.
    ///
    /// Exactly 1.0 for every pixel of [`PreparedBox::core`], falling to 0.0
    /// across the padding (see [`mask_value`]) and 0.0 outside
    /// [`PreparedBox::padded`].
    pub fn coverage(&self, x: i32, y: i32) -> f32 {
        if contains(&self.core, x, y) {
            return 1.0;
        }
        if !contains(&self.padded, x, y) {
            return 0.0;
        }
        let [x0, y0, x1, y1] = self.shape;
        let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
        let (cx, cy) = (f32::midpoint(x0, x1), f32::midpoint(y0, y1));
        let qx = (px - cx).abs() - ((x1 - x0) * 0.5 - self.radius);
        let qy = (py - cy).abs() - ((y1 - y0) * 0.5 - self.radius);
        let outside = qx.max(0.0).hypot(qy.max(0.0));
        let sdf = outside + qx.max(qy).min(0.0) - self.radius;
        mask_value(sdf, self.feather)
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

/// Whether `(x, y)` lies inside the half-open rectangle.
const fn contains(rect: &RectI, x: i32, y: i32) -> bool {
    x >= rect.x0 && x < rect.x1 && y >= rect.y0 && y < rect.y1
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
    t * t * 2.0f32.mul_add(-t, 3.0)
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
            shape: shape.map(|v| v as f32),
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
