//! Owned RGB and YUV frames and the drawing operations of the generator.

use blurrd_blur::{Plane, PlaneMut, YuvFrameMut};
use blurrd_core::RectI;

/// An 8-bit RGB frame (`R, G, B` per pixel, no row padding), the same
/// layout as the crops the detectors receive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbFrame {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

impl RgbFrame {
    /// A frame filled with one colour.
    pub fn filled(width: u32, height: u32, rgb: [u8; 3]) -> Self {
        let data = rgb.repeat(width as usize * height as usize);
        Self {
            width,
            height,
            data,
        }
    }

    /// Wraps packed RGB bytes, or returns `None` if their length is not
    /// `3 × width × height`.
    pub fn from_raw(width: u32, height: u32, data: Vec<u8>) -> Option<Self> {
        (data.len() == 3 * width as usize * height as usize).then_some(Self {
            width,
            height,
            data,
        })
    }

    /// Width in pixels.
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// The packed pixel bytes.
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// The whole frame as a rectangle.
    pub fn bounds(&self) -> RectI {
        RectI {
            x0: 0,
            y0: 0,
            x1: self.width as i32,
            y1: self.height as i32,
        }
    }

    fn offset(&self, x: i32, y: i32) -> Option<usize> {
        let inside = x >= 0 && y >= 0 && (x as u32) < self.width && (y as u32) < self.height;
        inside.then(|| 3 * (y as usize * self.width as usize + x as usize))
    }

    /// The colour of pixel `(x, y)`, or `None` outside the frame.
    pub fn pixel(&self, x: i32, y: i32) -> Option<[u8; 3]> {
        let i = self.offset(x, y)?;
        Some([self.data[i], self.data[i + 1], self.data[i + 2]])
    }

    /// Sets pixel `(x, y)`; ignored outside the frame.
    pub fn set(&mut self, x: i32, y: i32, rgb: [u8; 3]) {
        if let Some(i) = self.offset(x, y) {
            self.data[i..i + 3].copy_from_slice(&rgb);
        }
    }

    /// Mixes `rgb` into pixel `(x, y)` with opacity `alpha` (`0.0..=1.0`);
    /// ignored outside the frame.
    pub fn blend(&mut self, x: i32, y: i32, rgb: [u8; 3], alpha: f32) {
        let alpha = alpha.clamp(0.0, 1.0);
        if alpha <= 0.0 {
            return;
        }
        if let Some(i) = self.offset(x, y) {
            for (dst, &src) in self.data[i..i + 3].iter_mut().zip(&rgb) {
                let old = f32::from(*dst);
                let new = old + (f32::from(src) - old) * alpha;
                *dst = new.round().clamp(0.0, 255.0) as u8;
            }
        }
    }

    /// Mixes `rgb` into every pixel of `rect` (clipped to the frame).
    pub fn fill_rect(&mut self, rect: RectI, rgb: [u8; 3], alpha: f32) {
        self.for_each_in(rect, |frame, x, y| frame.blend(x, y, rgb, alpha));
    }

    /// Multiplies every pixel of `rect` by `factor` (a dark translucent
    /// panel such as the text-chat box).
    pub fn multiply_rect(&mut self, rect: RectI, factor: f32) {
        self.for_each_in(rect, |frame, x, y| {
            if let Some(i) = frame.offset(x, y) {
                for c in 0..3 {
                    let v = f32::from(frame.data[i + c]) * factor;
                    frame.data[i + c] = v.round().clamp(0.0, 255.0) as u8;
                }
            }
        });
    }

    /// Adds white light to every pixel of `rect`: `v + (255 - v) × amount`,
    /// like a flash or a bright ability washing over the HUD.
    pub fn wash_rect(&mut self, rect: RectI, amount: f32) {
        self.fill_rect(rect, [255, 255, 255], amount);
    }

    /// Fills `rect` with a rounded rectangle of radius `radius`, with an
    /// anti-aliased edge.
    pub fn fill_rounded(&mut self, rect: RectI, radius: f32, rgb: [u8; 3], alpha: f32) {
        let (cx, cy) = (
            (rect.x0 + rect.x1) as f32 * 0.5,
            (rect.y0 + rect.y1) as f32 * 0.5,
        );
        let (hw, hh) = (rect.width() as f32 * 0.5, rect.height() as f32 * 0.5);
        let r = radius.min(hw).min(hh);
        self.for_each_in(rect, |frame, x, y| {
            let qx = ((x as f32 + 0.5 - cx).abs() - (hw - r)).max(0.0);
            let qy = ((y as f32 + 0.5 - cy).abs() - (hh - r)).max(0.0);
            let d = qx.hypot(qy) - r;
            let cover = (0.5 - d).clamp(0.0, 1.0);
            frame.blend(x, y, rgb, alpha * cover);
        });
    }

    /// Calls `f` for every pixel of `rect` that lies inside the frame.
    pub(crate) fn for_each_in(&mut self, rect: RectI, mut f: impl FnMut(&mut Self, i32, i32)) {
        let Some(r) = rect.intersection(&self.bounds()) else {
            return;
        };
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                f(self, x, y);
            }
        }
    }

    /// Converts the frame to 8-bit I420 with BT.709 coefficients in limited
    /// range, the encoding of typical HD recordings. Chroma is the average
    /// of each 2×2 block.
    pub fn to_i420(&self) -> I420Frame {
        let (w, h) = (self.width as usize, self.height as usize);
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        let mut y_plane = vec![0u8; w * h];
        let mut u_plane = vec![0u8; cw * ch];
        let mut v_plane = vec![0u8; cw * ch];
        let rgb = |i: usize| {
            [
                f32::from(self.data[i]),
                f32::from(self.data[i + 1]),
                f32::from(self.data[i + 2]),
            ]
        };
        for py in 0..h {
            for px in 0..w {
                let [r, g, b] = rgb(3 * (py * w + px));
                let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                y_plane[py * w + px] = (16.0 + luma * 219.0 / 255.0).round() as u8;
            }
        }
        for cy in 0..ch {
            for cx in 0..cw {
                let (mut sum, mut n) = ([0.0f32; 3], 0.0f32);
                for py in 2 * cy..(2 * cy + 2).min(h) {
                    for px in 2 * cx..(2 * cx + 2).min(w) {
                        let p = rgb(3 * (py * w + px));
                        for c in 0..3 {
                            sum[c] += p[c];
                        }
                        n += 1.0;
                    }
                }
                let [r, g, b] = sum.map(|s| s / n);
                let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                let u = (b - luma) / 1.8556;
                let v = (r - luma) / 1.5748;
                u_plane[cy * cw + cx] = (128.0 + u * 224.0 / 255.0).round().clamp(0.0, 255.0) as u8;
                v_plane[cy * cw + cx] = (128.0 + v * 224.0 / 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
        I420Frame {
            width: self.width,
            height: self.height,
            y: y_plane,
            u: u_plane,
            v: v_plane,
        }
    }
}

/// An owned 8-bit I420 frame, for running the real blur on fixtures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct I420Frame {
    width: u32,
    height: u32,
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
}

impl I420Frame {
    /// Width in pixels.
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// A read-only view of the luma plane.
    pub fn luma(&self) -> Plane<'_> {
        Plane {
            data: &self.y,
            stride: self.width as usize,
            width: self.width,
            height: self.height,
        }
    }

    /// The Y, U and V planes, each packed without padding (chroma planes
    /// are `width.div_ceil(2)` samples wide), for writing test clips.
    pub fn planes(&self) -> [&[u8]; 3] {
        [&self.y, &self.u, &self.v]
    }

    /// A writable view for [`blurrd_blur::BlurBackend::blur`].
    pub fn as_frame_mut(&mut self) -> YuvFrameMut<'_> {
        let (cw, ch) = (self.width.div_ceil(2), self.height.div_ceil(2));
        YuvFrameMut::I420 {
            y: PlaneMut {
                data: &mut self.y,
                stride: self.width as usize,
                width: self.width,
                height: self.height,
            },
            u: PlaneMut {
                data: &mut self.u,
                stride: cw as usize,
                width: cw,
                height: ch,
            },
            v: PlaneMut {
                data: &mut self.v,
                stride: cw as usize,
                width: cw,
                height: ch,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use blurrd_core::RectI;

    use super::RgbFrame;

    #[test]
    fn draws_and_clips() {
        let mut f = RgbFrame::filled(8, 4, [10, 20, 30]);
        f.fill_rect(RectI::new(-2, -2, 2, 2).unwrap(), [255, 0, 0], 1.0);
        assert_eq!(f.pixel(1, 1), Some([255, 0, 0]));
        assert_eq!(f.pixel(2, 1), Some([10, 20, 30]));
        assert_eq!(f.pixel(8, 0), None);
        f.multiply_rect(f.bounds(), 0.5);
        assert_eq!(f.pixel(5, 3), Some([5, 10, 15]));
        f.wash_rect(f.bounds(), 1.0);
        assert_eq!(f.pixel(5, 3), Some([255, 255, 255]));
    }

    #[test]
    fn converts_to_limited_range_yuv() {
        let white = RgbFrame::filled(4, 4, [255, 255, 255]).to_i420();
        assert!(white.y.iter().all(|&v| v == 235));
        assert!(white.u.iter().chain(&white.v).all(|&v| v == 128));
        let black = RgbFrame::filled(3, 3, [0, 0, 0]).to_i420();
        assert!(black.y.iter().all(|&v| v == 16));
        assert_eq!(black.u.len(), 4);
        let red = RgbFrame::filled(2, 2, [255, 0, 0]).to_i420();
        assert_eq!((red.y[0], red.u[0], red.v[0]), (63, 102, 240));
    }

    #[test]
    fn rejects_wrong_raw_length() {
        assert!(RgbFrame::from_raw(2, 2, vec![0; 12]).is_some());
        assert!(RgbFrame::from_raw(2, 2, vec![0; 11]).is_none());
    }
}
