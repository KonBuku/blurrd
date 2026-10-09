//! Zero-mean normalised cross-correlation (port spec section 7.1).
//!
//! The kill-feed tracker follows a name by correlating a template of it
//! with later frames on the `min(R, G, B)` plane. The score is OpenCV's
//! `TM_CCOEFF_NORMED`:
//!
//! ```text
//!            Σ (T − mean T) · (I − mean I)
//! score = ─────────────────────────────────────
//!          √( Σ (T − mean T)² · Σ (I − mean I)² )
//! ```
//!
//! over the template's pixels `T` and the image pixels `I` under it. It is
//! invariant to brightness and contrast changes of the window, which is
//! what keeps a name matched while it fades, flashes or sits on a bright
//! background. The sums are exact integers, so the score is the exact
//! value (OpenCV computes the correlation in floating point); a window or
//! template without any variance scores 0, as in OpenCV.

use blurrd_core::RectI;

use crate::image::GrayImage;

/// Scores within this much of the best score are near-ties: an identical
/// name in the row above or below (the same killer, the same weapon).
/// Among them the smallest displacement wins (port spec 7.1).
pub const TIE_MARGIN: f32 = 0.04;

/// A template cut from a gray plane, with the sums the score needs.
#[derive(Debug, Clone, PartialEq)]
pub struct Template {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    sum: u64,
    /// `Σ (T − mean T)²`.
    variance_sum: f64,
}

impl Template {
    /// The pixels of `img` inside `rect`, or `None` if `rect` is empty or
    /// not fully inside the image.
    pub fn from_plane(img: &GrayImage, rect: RectI) -> Option<Self> {
        if rect.x0 < 0
            || rect.y0 < 0
            || rect.x1 > img.width() as i32
            || rect.y1 > img.height() as i32
            || rect.x1 <= rect.x0
            || rect.y1 <= rect.y0
        {
            return None;
        }
        let (width, height) = (rect.width(), rect.height());
        let mut pixels = Vec::with_capacity(width as usize * height as usize);
        for y in rect.y0..rect.y1 {
            pixels.extend_from_slice(&img.row(y as u32)[rect.x0 as usize..rect.x1 as usize]);
        }
        let sum: u64 = pixels.iter().map(|&p| u64::from(p)).sum();
        let sum_sq: u64 = pixels.iter().map(|&p| u64::from(p) * u64::from(p)).sum();
        let n = pixels.len() as f64;
        let variance_sum = (sum_sq as f64 - (sum as f64) * (sum as f64) / n).max(0.0);
        Some(Self {
            width,
            height,
            pixels,
            sum,
            variance_sum,
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

    /// The standard deviation of the template's pixels.
    pub fn std_dev(&self) -> f64 {
        (self.variance_sum / self.pixels.len() as f64).sqrt()
    }

    /// Heap bytes held by the template.
    pub fn heap_bytes(&self) -> usize {
        self.pixels.capacity()
    }

    fn row(&self, y: usize) -> &[u8] {
        let w = self.width as usize;
        &self.pixels[y * w..(y + 1) * w]
    }
}

/// The best position of a template in a search window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Match {
    /// Left edge of the template at the match.
    pub x: i32,
    /// Top edge of the template at the match.
    pub y: i32,
    /// The best score in the window (`−1..=1`).
    pub score: f32,
}

/// The score of `tpl` with its top-left corner at `(x, y)` of `img`, or
/// `None` if the template does not fit inside the image there.
pub fn score_at(img: &GrayImage, tpl: &Template, x: i32, y: i32) -> Option<f32> {
    if x < 0 || y < 0 || x as u32 + tpl.width > img.width() || y as u32 + tpl.height > img.height()
    {
        return None;
    }
    let (w, x) = (tpl.width as usize, x as usize);
    let (mut sum, mut sum_sq) = (0u64, 0u64);
    for row in 0..tpl.height {
        for &p in &img.row(y as u32 + row)[x..x + w] {
            sum += u64::from(p);
            sum_sq += u64::from(p) * u64::from(p);
        }
    }
    Some(tpl.score(cross(img, tpl, x, y as usize), sum, sum_sq))
}

/// `Σ T · I` of the template at `(x, y)`.
fn cross(img: &GrayImage, tpl: &Template, x: usize, y: usize) -> u64 {
    let w = tpl.width as usize;
    (0..tpl.height as usize)
        .map(|row| u64::from(dot(&img.row((y + row) as u32)[x..x + w], tpl.row(row))))
        .sum()
}

/// `Σ a·b` over one row: at most a few hundred terms of at most 255², so
/// it fits `u32`. Written as a plain sum of products so that the compiler
/// vectorises it.
fn dot(a: &[u8], b: &[u8]) -> u32 {
    a.iter()
        .zip(b)
        .map(|(&x, &y)| u32::from(x) * u32::from(y))
        .sum()
}

impl Template {
    /// The score from the cross term and the window's sum and sum of
    /// squares.
    fn score(&self, cross: u64, sum: u64, sum_sq: u64) -> f32 {
        let n = self.pixels.len() as f64;
        let numerator = cross as f64 - (self.sum as f64) * (sum as f64) / n;
        let window_variance = (sum_sq as f64 - (sum as f64) * (sum as f64) / n).max(0.0);
        normalise(numerator, (window_variance * self.variance_sum).sqrt())
    }
}

/// OpenCV's final step of `TM_CCOEFF_NORMED` (`common_matchTemplate`):
/// rounding errors that push the ratio slightly past ±1 are clamped, and a
/// window or template without variance scores 0.
fn normalise(numerator: f64, denominator: f64) -> f32 {
    let score = if numerator.abs() < denominator {
        numerator / denominator
    } else if numerator.abs() < denominator * 1.125 {
        numerator.signum()
    } else {
        0.0
    };
    score as f32
}

/// Searches templates in windows around a position; keeps its buffers.
#[derive(Debug, Clone, Default)]
pub struct Searcher {
    scores: Vec<f32>,
    /// Summed-area tables of the searched area: pixel sums and sums of
    /// squares, `(width + 1) × (height + 1)`.
    sums: Vec<u64>,
    squares: Vec<u64>,
}

impl Searcher {
    /// An empty searcher.
    pub fn new() -> Self {
        Self::default()
    }

    /// The best match of `tpl` in `img` among the top-left positions
    /// `(cx + dx, cy + dy)` with `dx` in `dx.0..=dx.1` and `dy` in
    /// `dy.0..=dy.1`, clipped to the positions where the template fits.
    ///
    /// Among near-ties (scores within [`TIE_MARGIN`] of the best) the
    /// position closest to `(cx, cy)` (smallest `|dx| + |dy|`, then the
    /// first in raster order) wins; the returned score is the best score
    /// of the window, as in the prototype. `None` if no position fits.
    pub fn find(
        &mut self,
        img: &GrayImage,
        tpl: &Template,
        (cx, cy): (i32, i32),
        dx: (i32, i32),
        dy: (i32, i32),
    ) -> Option<Match> {
        let x0 = (cx + dx.0).max(0);
        let x1 = (cx + dx.1).min(img.width() as i32 - tpl.width as i32);
        let y0 = (cy + dy.0).max(0);
        let y1 = (cy + dy.1).min(img.height() as i32 - tpl.height as i32);
        if x1 < x0 || y1 < y0 {
            return None;
        }
        // Window sums from summed-area tables of the searched area; only the
        // cross term is computed per position.
        let (tw, th) = (tpl.width as usize, tpl.height as usize);
        let (aw, ah) = ((x1 - x0) as usize + tw, (y1 - y0) as usize + th);
        let stride = aw + 1;
        self.sums.clear();
        self.sums.resize(stride * (ah + 1), 0);
        self.squares.clear();
        self.squares.resize(stride * (ah + 1), 0);
        for y in 0..ah {
            let row = &img.row(y0 as u32 + y as u32)[x0 as usize..x0 as usize + aw];
            let (mut run, mut run_sq) = (0u64, 0u64);
            for (x, &p) in row.iter().enumerate() {
                run += u64::from(p);
                run_sq += u64::from(p) * u64::from(p);
                let i = (y + 1) * stride + x + 1;
                self.sums[i] = self.sums[i - stride] + run;
                self.squares[i] = self.squares[i - stride] + run_sq;
            }
        }
        let area = |t: &[u64], x: usize, y: usize| {
            t[(y + th) * stride + x + tw] + t[y * stride + x]
                - t[y * stride + x + tw]
                - t[(y + th) * stride + x]
        };
        self.scores.clear();
        let mut best = f32::NEG_INFINITY;
        for y in 0..=(y1 - y0) as usize {
            for x in 0..=(x1 - x0) as usize {
                let c = cross(img, tpl, x0 as usize + x, y0 as usize + y);
                let s = tpl.score(c, area(&self.sums, x, y), area(&self.squares, x, y));
                best = best.max(s);
                self.scores.push(s);
            }
        }
        let width = (x1 - x0 + 1) as usize;
        let mut pick = None;
        for (i, &s) in self.scores.iter().enumerate() {
            if s >= best - TIE_MARGIN {
                let (x, y) = (x0 + (i % width) as i32, y0 + (i / width) as i32);
                let distance = (x - cx).abs() + (y - cy).abs();
                if pick.is_none_or(|(d, _, _)| distance < d) {
                    pick = Some((distance, x, y));
                }
            }
        }
        pick.map(|(_, x, y)| Match { x, y, score: best })
    }
}

#[cfg(test)]
mod tests {
    use blurrd_core::RectI;

    use super::{Searcher, Template, score_at};
    use crate::image::GrayImage;

    fn image(width: u32, height: u32, f: impl Fn(u32, u32) -> u8) -> GrayImage {
        let data = (0..height)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .map(|(x, y)| f(x, y))
            .collect();
        GrayImage::from_vec(width, height, data).unwrap()
    }

    /// Pseudo-random texture.
    fn noise(x: u32, y: u32) -> u8 {
        let h =
            (x.wrapping_mul(73_856_093) ^ y.wrapping_mul(19_349_663)).wrapping_mul(2_654_435_761);
        (h >> 24) as u8
    }

    fn rect(x0: i32, y0: i32, x1: i32, y1: i32) -> RectI {
        RectI::new(x0, y0, x1, y1).unwrap()
    }

    /// The definition in floating point.
    fn naive(img: &GrayImage, tpl: &GrayImage, x: u32, y: u32) -> f64 {
        let n = f64::from(tpl.width() * tpl.height());
        let px = |dx: u32, dy: u32| f64::from(img.get(x + dx, y + dy));
        let tp = |dx: u32, dy: u32| f64::from(tpl.get(dx, dy));
        let coords: Vec<(u32, u32)> = (0..tpl.height())
            .flat_map(|dy| (0..tpl.width()).map(move |dx| (dx, dy)))
            .collect();
        let mi = coords.iter().map(|&(a, b)| px(a, b)).sum::<f64>() / n;
        let mt = coords.iter().map(|&(a, b)| tp(a, b)).sum::<f64>() / n;
        let num: f64 = coords
            .iter()
            .map(|&(a, b)| (px(a, b) - mi) * (tp(a, b) - mt))
            .sum();
        let vi: f64 = coords.iter().map(|&(a, b)| (px(a, b) - mi).powi(2)).sum();
        let vt: f64 = coords.iter().map(|&(a, b)| (tp(a, b) - mt).powi(2)).sum();
        if vi * vt == 0.0 {
            0.0
        } else {
            num / (vi * vt).sqrt()
        }
    }

    #[test]
    fn matches_the_definition() {
        let img = image(40, 30, |x, y| noise(x, y) / 2 + (x * 3) as u8);
        let tpl_img = image(9, 6, |x, y| noise(x + 100, y * 7));
        let tpl = Template::from_plane(&tpl_img, rect(0, 0, 9, 6)).unwrap();
        for (x, y) in [(0, 0), (5, 3), (31, 24), (17, 11)] {
            let got = f64::from(score_at(&img, &tpl, x, y).unwrap());
            let want = naive(&img, &tpl_img, x as u32, y as u32);
            assert!((got - want).abs() < 1e-5, "({x}, {y}): {got} vs {want}");
        }
    }

    #[test]
    fn scores_exact_and_inverted_copies_and_flat_windows() {
        let img = image(40, 20, |x, y| if x < 25 { noise(x, y) } else { 90 });
        let tpl = Template::from_plane(&img, rect(4, 5, 14, 12)).unwrap();
        assert!((score_at(&img, &tpl, 4, 5).unwrap() - 1.0).abs() < 1e-6);
        let inverted = image(40, 20, |x, y| 255 - noise(x, y));
        assert!((score_at(&inverted, &tpl, 4, 5).unwrap() + 1.0).abs() < 1e-6);
        // Brightness and contrast changes do not change the score.
        let faded = image(40, 20, |x, y| 60 + noise(x, y) / 3);
        assert!(score_at(&faded, &tpl, 4, 5).unwrap() > 0.99);
        // A flat window scores 0, like OpenCV.
        assert!(score_at(&img, &tpl, 25, 0).unwrap().abs() < f32::EPSILON);
    }

    #[test]
    fn finds_the_template_and_prefers_the_nearest_tie() {
        // The same pattern twice, 12 rows apart.
        let img = image(40, 40, |x, y| noise(x, y % 12));
        let tpl = Template::from_plane(&img, rect(10, 14, 20, 20)).unwrap();
        let mut s = Searcher::new();
        let m = s.find(&img, &tpl, (11, 15), (-3, 3), (-3, 3)).unwrap();
        assert_eq!((m.x, m.y), (10, 14));
        assert!((m.score - 1.0).abs() < 1e-6);
        // Both copies match exactly; the one closer to the start wins.
        let up = s.find(&img, &tpl, (10, 5), (-3, 3), (-12, 12)).unwrap();
        assert_eq!((up.x, up.y), (10, 2));
        let down = s.find(&img, &tpl, (10, 10), (-3, 3), (-12, 12)).unwrap();
        assert_eq!((down.x, down.y), (10, 14));
    }

    #[test]
    fn clips_windows_to_the_image() {
        let img = image(20, 10, noise);
        let tpl = Template::from_plane(&img, rect(0, 0, 8, 4)).unwrap();
        let mut s = Searcher::new();
        assert!(s.find(&img, &tpl, (0, 0), (-5, -1), (0, 0)).is_none());
        let m = s.find(&img, &tpl, (2, 2), (-5, 50), (-5, 50)).unwrap();
        assert_eq!((m.x, m.y), (0, 0));
        assert!(Template::from_plane(&img, rect(15, 0, 21, 4)).is_none());
        assert!(score_at(&img, &tpl, 13, 0).is_none());
        assert!(score_at(&img, &tpl, -1, 0).is_none());
        assert!(score_at(&img, &tpl, 12, 6).is_some());
    }

    #[test]
    fn reports_the_template_spread() {
        let flat = image(6, 6, |_, _| 7);
        let t = Template::from_plane(&flat, rect(0, 0, 6, 6)).unwrap();
        assert!(t.std_dev() < 1e-9);
        let two = image(2, 1, |x, _| if x == 0 { 0 } else { 10 });
        let t = Template::from_plane(&two, rect(0, 0, 2, 1)).unwrap();
        assert!((t.std_dev() - 5.0).abs() < 1e-9);
    }
}
