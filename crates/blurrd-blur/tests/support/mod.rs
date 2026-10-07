//! Owned synthetic frames for tests and benchmarks.
//!
//! Shared by the integration tests (`mod support;`) and the benchmarks
//! (`#[path = "../tests/support/mod.rs"]`). Content is generated from a
//! seed; no real footage is used.

#![allow(dead_code, reason = "each test binary uses a different subset")]
#![allow(
    clippy::many_single_char_names,
    clippy::cast_possible_wrap,
    clippy::expect_used,
    reason = "test code: short pixel coordinate names, small synthetic sizes, panics are failures"
)]

use blurrd_blur::{PlaneMut, YuvFrameMut};
use blurrd_core::RectI;

/// Layout of an [`OwnedFrame`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Format {
    I420,
    Nv12,
    P010,
}

impl Format {
    pub(crate) const ALL: [Self; 3] = [Self::I420, Self::Nv12, Self::P010];

    fn bytes(self) -> usize {
        if self == Self::P010 { 2 } else { 1 }
    }

    /// Largest code value.
    pub(crate) fn max(self) -> u16 {
        if self == Self::P010 { 1023 } else { 255 }
    }
}

/// A frame that owns its planes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OwnedFrame {
    pub(crate) format: Format,
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// Plane bytes: Y, then U and V (I420) or UV (NV12, P010).
    pub(crate) planes: Vec<Vec<u8>>,
}

/// A small, fast, deterministic generator (SplitMix64).
#[derive(Debug, Clone)]
pub(crate) struct Rng(pub(crate) u64);

impl Rng {
    pub(crate) fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A value in `0..n`.
    pub(crate) fn below(&mut self, n: u32) -> u32 {
        (self.next() % u64::from(n)) as u32
    }
}

impl OwnedFrame {
    /// A frame filled with `value` in channels Y, U and V.
    pub(crate) fn flat(format: Format, width: u32, height: u32, value: [u16; 3]) -> Self {
        let (cw, ch) = (width.div_ceil(2), height.div_ceil(2));
        let b = format.bytes();
        let mut planes = vec![vec![0; (width * height) as usize * b]];
        match format {
            Format::I420 => {
                planes.push(vec![0; (cw * ch) as usize]);
                planes.push(vec![0; (cw * ch) as usize]);
            }
            Format::Nv12 | Format::P010 => planes.push(vec![0; (cw * ch) as usize * 2 * b]),
        }
        let mut frame = Self {
            format,
            width,
            height,
            planes,
        };
        for (c, &v) in value.iter().enumerate() {
            let (w, h) = frame.channel_size(c);
            for y in 0..h {
                for x in 0..w {
                    frame.set(c, x, y, v);
                }
            }
        }
        frame
    }

    /// A frame with random code values in every channel.
    pub(crate) fn noise(format: Format, width: u32, height: u32, seed: u64) -> Self {
        let mut frame = Self::flat(format, width, height, [0; 3]);
        let mut rng = Rng(seed);
        let max = u32::from(format.max()) + 1;
        for c in 0..3 {
            let (w, h) = frame.channel_size(c);
            for y in 0..h {
                for x in 0..w {
                    frame.set(c, x, y, rng.below(max) as u16);
                }
            }
        }
        frame
    }

    /// Size of channel `c` (0 = Y, 1 = U, 2 = V).
    pub(crate) fn channel_size(&self, c: usize) -> (u32, u32) {
        if c == 0 {
            (self.width, self.height)
        } else {
            (self.width.div_ceil(2), self.height.div_ceil(2))
        }
    }

    /// Plane index and byte offset of channel `c` at `(x, y)`.
    fn locate(&self, c: usize, x: u32, y: u32) -> (usize, usize) {
        let b = self.format.bytes();
        let (w, _) = self.channel_size(c);
        let pixel = (y * w + x) as usize;
        match (self.format, c) {
            (_, 0) => (0, pixel * b),
            (Format::I420, _) => (c, pixel),
            _ => (1, (pixel * 2 + (c - 1)) * b),
        }
    }

    /// Code value of channel `c` at `(x, y)`.
    pub(crate) fn get(&self, c: usize, x: u32, y: u32) -> u16 {
        let (p, i) = self.locate(c, x, y);
        match self.format {
            Format::P010 => u16::from_le_bytes([self.planes[p][i], self.planes[p][i + 1]]) >> 6,
            _ => u16::from(self.planes[p][i]),
        }
    }

    /// Sets the code value of channel `c` at `(x, y)`.
    pub(crate) fn set(&mut self, c: usize, x: u32, y: u32, value: u16) {
        let (p, i) = self.locate(c, x, y);
        match self.format {
            Format::P010 => {
                self.planes[p][i..i + 2].copy_from_slice(&(value << 6).to_le_bytes());
            }
            _ => self.planes[p][i] = value as u8,
        }
    }

    /// The part of the frame inside `rect` (with an even origin) as a new
    /// frame.
    pub(crate) fn crop(&self, rect: RectI) -> Self {
        let mut out = Self::flat(self.format, rect.width(), rect.height(), [0; 3]);
        for c in 0..3 {
            let s = if c == 0 { 1 } else { 2 };
            let (cw, ch) = out.channel_size(c);
            for y in 0..ch {
                for x in 0..cw {
                    let v = self.get(c, rect.x0 as u32 / s + x, rect.y0 as u32 / s + y);
                    out.set(c, x, y, v);
                }
            }
        }
        out
    }

    /// A writable view for the blur.
    pub(crate) fn view(&mut self) -> YuvFrameMut<'_> {
        let b = self.format.bytes();
        let (w, h) = (self.width, self.height);
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        let format = self.format;
        let mut planes = self.planes.iter_mut().map(Vec::as_mut_slice);
        let mut plane = |stride: usize, width: u32, height: u32| PlaneMut {
            data: planes.next().unwrap_or_default(),
            stride,
            width,
            height,
        };
        let y = plane(w as usize * b, w, h);
        match format {
            Format::I420 => {
                let u = plane(cw as usize, cw, ch);
                let v = plane(cw as usize, cw, ch);
                YuvFrameMut::I420 { y, u, v }
            }
            Format::Nv12 => YuvFrameMut::Nv12 {
                y,
                uv: plane(cw as usize * 2, cw, ch),
            },
            Format::P010 => YuvFrameMut::P010 {
                y,
                uv: plane(cw as usize * 4, cw, ch),
            },
        }
    }

    /// Draws a row of `count` synthetic, name-like glyphs into the luma
    /// plane: blocky characters `height` pixels tall made of strokes
    /// `stroke` pixels wide, starting at `origin`, in code value `ink`.
    /// Returns the bounding box of the text.
    pub(crate) fn draw_text(
        &mut self,
        rng: &mut Rng,
        origin: (u32, u32),
        count: u32,
        height: u32,
        stroke: u32,
        ink: u16,
    ) -> RectI {
        let glyph_w = height * 3 / 5;
        let mut pen = origin.0;
        for _ in 0..count {
            // Each glyph: up to three vertical and three horizontal strokes.
            let pattern = rng.next() | 1;
            let cols = [0, (glyph_w - stroke) / 2, glyph_w - stroke];
            let rows = [0, (height - stroke) / 2, height - stroke];
            for (i, col) in cols.into_iter().enumerate() {
                if pattern >> i & 1 == 1 {
                    self.fill_luma(pen + col, origin.1, stroke, height, ink);
                }
            }
            for (i, row) in rows.into_iter().enumerate() {
                if pattern >> (i + 3) & 1 == 1 {
                    self.fill_luma(pen, origin.1 + row, glyph_w, stroke, ink);
                }
            }
            pen += glyph_w + stroke;
        }
        RectI::new(
            origin.0 as i32,
            origin.1 as i32,
            pen as i32,
            (origin.1 + height) as i32,
        )
        .expect("text has an area")
    }

    fn fill_luma(&mut self, x: u32, y: u32, w: u32, h: u32, value: u16) {
        for yy in y..(y + h).min(self.height) {
            for xx in x..(x + w).min(self.width) {
                self.set(0, xx, yy, value);
            }
        }
    }
}
