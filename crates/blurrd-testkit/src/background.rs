//! Procedural backgrounds behind the HUD.
//!
//! They stand in for gameplay: textured noise, smooth gradients, saturated
//! walls in the team colours (which can fake kill-feed bars) and a washed
//! out sky (which hides white text).

use blurrd_core::RectI;

use crate::image::RgbFrame;
use crate::rng::hash2;

/// A background, rendered into the image area of a frame.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum Background {
    /// Smooth value noise around a base colour, with `amp` code values of
    /// variation and blobs of about `cell` pixels.
    Noise {
        /// Seed of the noise.
        seed: u64,
        /// Mean colour.
        base: [u8; 3],
        /// Variation in code values.
        amp: f32,
        /// Feature size in pixels.
        cell: u32,
    },
    /// A vertical gradient.
    Gradient {
        /// Colour of the top row.
        top: [u8; 3],
        /// Colour of the bottom row.
        bottom: [u8; 3],
    },
    /// One flat colour.
    Flat([u8; 3]),
}

impl Background {
    /// Mid-grey textured noise, the neutral default.
    pub const fn noise(seed: u64) -> Self {
        Self::Noise {
            seed,
            base: [96, 104, 110],
            amp: 60.0,
            cell: 24,
        }
    }

    /// A red-pink wall like the one that fakes kill-feed rows in the port
    /// spec (failure F8).
    pub const fn red_wall(seed: u64) -> Self {
        Self::Noise {
            seed,
            base: [214, 92, 96],
            amp: 18.0,
            cell: 40,
        }
    }

    /// A teal wall in the ally bar colour.
    pub const fn teal_wall(seed: u64) -> Self {
        Self::Noise {
            seed,
            base: [96, 184, 160],
            amp: 18.0,
            cell: 40,
        }
    }

    /// A bright, almost white sky that hides white text.
    pub const fn white_sky() -> Self {
        Self::Gradient {
            top: [236, 242, 250],
            bottom: [214, 226, 238],
        }
    }

    /// A dark-to-light gradient.
    pub const fn dusk() -> Self {
        Self::Gradient {
            top: [24, 30, 52],
            bottom: [190, 150, 120],
        }
    }

    /// Every kind of background, for fixture matrices.
    pub const fn variety(seed: u64) -> [Self; 5] {
        [
            Self::noise(seed),
            Self::red_wall(seed),
            Self::teal_wall(seed),
            Self::white_sky(),
            Self::dusk(),
        ]
    }

    /// Fills `area` of `frame`; the rest (black bars) is set to black.
    pub fn render(&self, frame: &mut RgbFrame, area: RectI) {
        let full = frame.bounds();
        frame.fill_rect(full, [0, 0, 0], 1.0);
        let height = area.height().max(1) as f32;
        frame.for_each_in(area, |f, x, y| {
            let rgb = match *self {
                Self::Flat(c) => c,
                Self::Gradient { top, bottom } => {
                    let t = (y - area.y0) as f32 / height;
                    [0, 1, 2].map(|c| lerp(top[c], bottom[c], t))
                }
                Self::Noise {
                    seed,
                    base,
                    amp,
                    cell,
                } => {
                    let n = value_noise(x, y, cell.max(1), seed);
                    let fine = (hash2(x, y, seed ^ 0x5EED) & 0xFF) as f32 / 255.0 - 0.5;
                    let d = (n - 0.5) * 2.0 * amp + fine * amp * 0.25;
                    base.map(|c| (f32::from(c) + d).round().clamp(0.0, 255.0) as u8)
                }
            };
            f.set(x, y, rgb);
        });
    }
}

fn lerp(a: u8, b: u8, t: f32) -> u8 {
    (f32::from(a) + (f32::from(b) - f32::from(a)) * t)
        .round()
        .clamp(0.0, 255.0) as u8
}

/// Bilinearly interpolated lattice noise in `0.0..=1.0`.
fn value_noise(x: i32, y: i32, cell: u32, seed: u64) -> f32 {
    let c = cell as i32;
    let (gx, gy) = (x.div_euclid(c), y.div_euclid(c));
    let (fx, fy) = (
        x.rem_euclid(c) as f32 / c as f32,
        y.rem_euclid(c) as f32 / c as f32,
    );
    let v = |cx: i32, cy: i32| (hash2(cx, cy, seed) & 0xFFFF) as f32 / 65535.0;
    let smooth = |t: f32| t * t * (3.0 - 2.0 * t);
    let (sx, sy) = (smooth(fx), smooth(fy));
    let top = v(gx, gy) + (v(gx + 1, gy) - v(gx, gy)) * sx;
    let bottom = v(gx, gy + 1) + (v(gx + 1, gy + 1) - v(gx, gy + 1)) * sx;
    top + (bottom - top) * sy
}

#[cfg(test)]
mod tests {
    use blurrd_core::RectI;

    use super::Background;
    use crate::image::RgbFrame;

    #[test]
    fn fills_only_the_image_area() {
        let mut f = RgbFrame::filled(40, 10, [1, 2, 3]);
        Background::Flat([200, 100, 50]).render(&mut f, RectI::new(10, 0, 30, 10).unwrap());
        assert_eq!(f.pixel(5, 5), Some([0, 0, 0]));
        assert_eq!(f.pixel(15, 5), Some([200, 100, 50]));
    }

    #[test]
    fn noise_is_deterministic_and_varied() {
        let area = RectI::new(0, 0, 64, 64).unwrap();
        let mut a = RgbFrame::filled(64, 64, [0, 0, 0]);
        let mut b = a.clone();
        Background::noise(7).render(&mut a, area);
        Background::noise(7).render(&mut b, area);
        assert_eq!(a, b);
        let distinct: std::collections::HashSet<_> =
            (0..64).map(|x| a.pixel(x, 20).unwrap()).collect();
        assert!(distinct.len() > 8);
    }

    #[test]
    fn gradient_runs_top_to_bottom() {
        let mut f = RgbFrame::filled(4, 11, [0, 0, 0]);
        let area = f.bounds();
        Background::Gradient {
            top: [0, 0, 0],
            bottom: [200, 200, 200],
        }
        .render(&mut f, area);
        assert_eq!(f.pixel(0, 0), Some([0, 0, 0]));
        assert!(f.pixel(0, 10).unwrap()[0] > 170);
    }
}
