//! When a kill-feed name is really gone (plan 7.3, "box end by
//! structure").
//!
//! The template of a name stops matching before the name is invisible:
//! the oldest row slides up by about ten pixels while it fades, and a
//! flash or a bright background lowers the score below the tracking
//! thresholds. The box therefore does not end where the template was last
//! found. It ends where the row's structure is gone: the row's bar no
//! longer covers the band ([`BAR_MIN`]) and the name's strokes no longer
//! stand out ([`INK_MIN`]; the template must still match there, so the
//! texture of a busy background does not pass as strokes). A post-roll of
//! the measured fade length follows ([`super::POST_ROLL`]).
//!
//! Both signals are measured against the name's own look in its
//! reference frame, never against fixed colours: the bar colour is the
//! colour around the name in that frame, and the stroke contrast is
//! relative to the contrast there. Colour-blind palettes, custom colours,
//! GPU colour filters and greyscale video therefore change nothing.

use blurrd_core::RectI;

use super::follow::Frame;

/// The bar still covers the band while at least this share of the
/// reference bar pixels keeps the reference bar colour (plan 7.3: the box
/// ends when the bar presence falls below 0.1).
pub(super) const BAR_MIN: f32 = 0.1;
/// The name is still visible while its stroke contrast is at least this
/// share of the reference contrast. Measured on the tuning clips: below
/// it the strokes are within the compression noise of the background.
pub(super) const INK_MIN: f32 = 0.05;
/// Strokes count only where the name's template still matches at least
/// this well (`TH_SLIDE`): the correlation ignores contrast, so it stays
/// high on a faint name and is low on background texture.
pub(super) const INK_MATCH: f32 = 0.5;
/// A channel within this many levels of the reference bar colour counts as
/// bar.
const BAR_TOLERANCE: i32 = 20;
/// Strokes are the pixels of the text box above `bar + STROKE_SHARE ×
/// (text − bar)` ...
const STROKE_SHARE: i32 = 2;
/// ... where `bar` is this percentile of the text box ...
const BAR_PERCENTILE: usize = 40;
/// ... and `text` this one.
const TEXT_PERCENTILE: usize = 99;
/// A name whose strokes stand out less than this in its reference frame
/// has no usable stroke contrast.
const MIN_CONTRAST: i32 = 30;
/// Bar pixels are band pixels below `bar + (text − bar) / BAR_SHARE`,
/// away from the strokes and their anti-aliasing.
const BAR_SHARE: i32 = 4;
/// Fewer reference bar pixels than this give no bar presence (a name
/// whose band is mostly text or background).
const MIN_BAR_PIXELS: usize = 40;
/// The band the bar is measured in, from the row top (half-open).
const BAND: (i32, i32) = (3, 30);

/// The look of a name in its reference frame. All positions are relative
/// to the template's top-left corner, so they move with the tracked
/// position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Look {
    /// The text box (full resolution).
    text: RectI,
    /// Stroke pixels of the text box, row by row.
    strokes: Vec<bool>,
    /// Stroke contrast in the reference frame, if it has a usable one.
    contrast: Option<i32>,
    /// The bar colour around the name (R, G, B).
    bar: [u8; 3],
    /// Pixels of the half-resolution plane that showed the bar colour,
    /// relative to half the template corner.
    bar_pixels: Vec<(i16, i16)>,
}

impl Look {
    /// The look of a name in `frame` with the template corner at `corner`
    /// (strip coordinates); `text` is the name's text and `row_top` the top
    /// of its row, both relative to the corner.
    pub(super) fn new(frame: &Frame<'_>, corner: (i32, i32), text: RectI, row_top: i32) -> Self {
        let mut values = Vec::with_capacity(text.area() as usize);
        for y in text.y0..text.y1 {
            for x in text.x0..text.x1 {
                values.push(pixel(frame, corner.0 + x, corner.1 + y));
            }
        }
        let mut sorted = values.clone();
        sorted.sort_unstable();
        let at = |p: usize| {
            sorted
                .get((sorted.len().saturating_sub(1)) * p / 100)
                .copied()
        };
        let (bar_level, text_level) = (
            at(BAR_PERCENTILE).unwrap_or(0),
            at(TEXT_PERCENTILE).unwrap_or(0),
        );
        let stroke_min = bar_level + (text_level - bar_level) / STROKE_SHARE;
        let strokes: Vec<bool> = values.iter().map(|&v| v >= stroke_min).collect();
        let mut look = Self {
            text,
            strokes,
            contrast: None,
            bar: [0; 3],
            bar_pixels: Vec::new(),
        };
        if text_level - bar_level >= MIN_CONTRAST {
            look.contrast = look
                .stroke_contrast(frame, corner)
                .filter(|&c| c >= MIN_CONTRAST);
        }
        look.learn_bar(
            frame,
            corner,
            row_top,
            bar_level + (text_level - bar_level) / BAR_SHARE,
        );
        look
    }

    /// Learns the bar colour from the band pixels darker than `bar_max`
    /// (on the `min(R, G, B)` plane) over the text box's width plus the
    /// template margin.
    fn learn_bar(&mut self, frame: &Frame<'_>, corner: (i32, i32), row_top: i32, bar_max: i32) {
        let half = (corner.0.div_euclid(2), corner.1.div_euclid(2));
        let band = (row_top + BAND.0, row_top + BAND.1);
        let mut candidates = Vec::new();
        let mut channels: [Vec<u8>; 3] = Default::default();
        for hy in (corner.1 + band.0).div_euclid(2)..(corner.1 + band.1).div_euclid(2) {
            for hx in (corner.0 + self.text.x0).div_euclid(2) - 3
                ..(corner.0 + self.text.x1).div_euclid(2) + 3
            {
                let Some(c) = frame.colour(hx, hy) else {
                    continue;
                };
                if i32::from(c[0].min(c[1]).min(c[2])) >= bar_max {
                    continue;
                }
                candidates.push((hx, hy, c));
                for (list, v) in channels.iter_mut().zip(c) {
                    list.push(v);
                }
            }
        }
        if candidates.len() < MIN_BAR_PIXELS {
            return;
        }
        for (i, list) in channels.iter_mut().enumerate() {
            let mid = list.len() / 2;
            self.bar[i] = *list.select_nth_unstable(mid).1;
        }
        let bar = self.bar;
        self.bar_pixels = candidates
            .into_iter()
            .filter(|&(_, _, c)| is_bar(c, bar))
            .map(|(hx, hy, _)| ((hx - half.0) as i16, (hy - half.1) as i16))
            .collect();
        if self.bar_pixels.len() < MIN_BAR_PIXELS {
            self.bar_pixels.clear();
        }
    }

    /// Heap bytes held by the look.
    pub(super) fn heap_bytes(&self) -> usize {
        self.strokes.capacity() + self.bar_pixels.capacity() * size_of::<(i16, i16)>()
    }

    /// Whether the name's row is still on screen with the template corner
    /// at `corner`, where the template scores `score`: the bar still covers
    /// the band or the strokes still stand out.
    pub(super) fn present(&self, frame: &Frame<'_>, corner: (i32, i32), score: f32) -> bool {
        self.bar_presence(frame, corner) >= BAR_MIN
            || (score >= INK_MATCH && self.ink(frame, corner) >= INK_MIN)
    }

    /// The share of the reference bar pixels that show the bar colour.
    fn bar_presence(&self, frame: &Frame<'_>, corner: (i32, i32)) -> f32 {
        if self.bar_pixels.is_empty() {
            return 0.0;
        }
        let half = (corner.0.div_euclid(2), corner.1.div_euclid(2));
        let hits = self
            .bar_pixels
            .iter()
            .filter(|&&(dx, dy)| {
                frame
                    .colour(half.0 + i32::from(dx), half.1 + i32::from(dy))
                    .is_some_and(|c| is_bar(c, self.bar))
            })
            .count();
        hits as f32 / self.bar_pixels.len() as f32
    }

    /// The stroke contrast as a share of the reference contrast (0 for a
    /// name without a usable reference contrast).
    fn ink(&self, frame: &Frame<'_>, corner: (i32, i32)) -> f32 {
        match (self.contrast, self.stroke_contrast(frame, corner)) {
            (Some(reference), Some(c)) => c as f32 / reference as f32,
            _ => 0.0,
        }
    }

    /// Mean of the stroke pixels minus the median of the other pixels of
    /// the text box.
    fn stroke_contrast(&self, frame: &Frame<'_>, corner: (i32, i32)) -> Option<i32> {
        let (mut sum, mut count) = (0i64, 0i64);
        let mut rest = Vec::with_capacity(self.strokes.len());
        let mut strokes = self.strokes.iter();
        for y in self.text.y0..self.text.y1 {
            for x in self.text.x0..self.text.x1 {
                let v = pixel(frame, corner.0 + x, corner.1 + y);
                if strokes.next().copied().unwrap_or(false) {
                    sum += i64::from(v);
                    count += 1;
                } else {
                    rest.push(v);
                }
            }
        }
        if count == 0 || rest.is_empty() {
            return None;
        }
        let mid = rest.len() / 2;
        let median = *rest.select_nth_unstable(mid).1;
        Some((sum / count) as i32 - median)
    }
}

/// `min(R, G, B)` at `(x, y)` of the strip, 0 outside.
fn pixel(frame: &Frame<'_>, x: i32, y: i32) -> i32 {
    if x < 0 || y < 0 || x >= frame.plane.width() as i32 || y >= frame.plane.height() as i32 {
        return 0;
    }
    i32::from(frame.plane.get(x as u32, y as u32))
}

/// Whether colour `c` is the bar colour `bar`.
fn is_bar(c: [u8; 3], bar: [u8; 3]) -> bool {
    c.iter()
        .zip(bar)
        .all(|(&v, b)| (i32::from(v) - i32::from(b)).abs() <= BAR_TOLERANCE)
}

#[cfg(test)]
mod tests {
    use blurrd_core::RectI;

    use super::Look;
    use crate::image::GrayImage;
    use crate::temporal::killfeed::STRIP;
    use crate::temporal::killfeed::follow::Frame;
    use crate::temporal::killfeed::ring::{HALF_HEIGHT, HALF_WIDTH};

    /// Template corner, row top and text box (strip coordinates) of the
    /// test name.
    const CORNER: (i32, i32) = (400, 20);
    const ROW_TOP: i32 = 16;
    const TEXT: RectI = RectI {
        x0: 406,
        y0: 27,
        x1: 466,
        y1: 41,
    };
    const BAR: [u8; 3] = [30, 170, 150];

    /// A strip in packed RGB: background `bg(x, y)`, the row's bar over
    /// `bar` of its opacity, strokes (every third column of the text box)
    /// of contrast `ink` (0 to 1) against the pixel under them.
    fn strip(bg: impl Fn(i32, i32) -> [u8; 3], bar: f32, ink: f32) -> Vec<u8> {
        let (w, h) = (STRIP.width() as i32, STRIP.height() as i32);
        let mut rgb = Vec::with_capacity((w * h * 3) as usize);
        for y in 0..h {
            for x in 0..w {
                let mut c = bg(x, y);
                let in_bar = (360..520).contains(&x) && (ROW_TOP..ROW_TOP + 33).contains(&y);
                if in_bar {
                    for (v, b) in c.iter_mut().zip(BAR) {
                        *v = (f32::from(*v) * (1.0 - bar) + f32::from(b) * bar).round() as u8;
                    }
                }
                let stroke = (TEXT.x0..TEXT.x1).contains(&x)
                    && (TEXT.y0..TEXT.y1).contains(&y)
                    && (x - TEXT.x0) % 3 == 0;
                if stroke {
                    for v in &mut c {
                        *v = (f32::from(*v) + (250.0 - f32::from(*v)) * ink).round() as u8;
                    }
                }
                rgb.extend_from_slice(&c);
            }
        }
        rgb
    }

    /// The tracker's view of a strip: `min(R, G, B)` and the 2 × 2 colours.
    fn planes(rgb: &[u8]) -> (GrayImage, Vec<u8>) {
        let (w, h) = (STRIP.width() as usize, STRIP.height() as usize);
        let mn = rgb
            .as_chunks::<3>()
            .0
            .iter()
            .map(|p| p[0].min(p[1]).min(p[2]))
            .collect();
        let mut colours = Vec::new();
        for hy in 0..HALF_HEIGHT as usize {
            for hx in 0..HALF_WIDTH as usize {
                for c in 0..3 {
                    let at = |x: usize, y: usize| u16::from(rgb[3 * (y * w + x) + c]);
                    let sum = at(2 * hx, 2 * hy)
                        + at(2 * hx + 1, 2 * hy)
                        + at(2 * hx, 2 * hy + 1)
                        + at(2 * hx + 1, 2 * hy + 1);
                    colours.push(((sum + 2) / 4) as u8);
                }
            }
        }
        (
            GrayImage::from_vec(w as u32, h as u32, mn).unwrap(),
            colours,
        )
    }

    fn frame<'a>(plane: &'a GrayImage, colours: &'a [u8]) -> Frame<'a> {
        Frame {
            index: 0,
            plane,
            colours,
            names: &[],
        }
    }

    fn look(rgb: &[u8]) -> Look {
        let (plane, colours) = planes(rgb);
        let text = RectI {
            x0: TEXT.x0 - CORNER.0,
            y0: TEXT.y0 - CORNER.1,
            x1: TEXT.x1 - CORNER.0,
            y1: TEXT.y1 - CORNER.1,
        };
        Look::new(&frame(&plane, &colours), CORNER, text, ROW_TOP - CORNER.1)
    }

    /// Whether the name counts as present in `rgb` where the template
    /// scores `score`.
    fn present(look: &Look, rgb: &[u8], score: f32) -> bool {
        let (plane, colours) = planes(rgb);
        look.present(&frame(&plane, &colours), CORNER, score)
    }

    fn grey(_: i32, _: i32) -> [u8; 3] {
        [70, 70, 70]
    }

    fn texture(x: i32, y: i32) -> [u8; 3] {
        let v = ((x * 37 + y * 91) % 160 + 40) as u8;
        [v, v / 2 + 60, 255 - v]
    }

    #[test]
    fn a_settled_row_is_present() {
        let reference = strip(grey, 1.0, 1.0);
        let look = look(&reference);
        assert!(look.contrast.is_some());
        assert_eq!(look.bar, BAR);
        assert!(present(&look, &reference, 1.0));
        // The bar alone keeps the row, whatever the template scores.
        assert!(present(&look, &strip(grey, 1.0, 0.0), -1.0));
    }

    #[test]
    fn faint_strokes_count_only_where_the_template_matches() {
        let look = look(&strip(grey, 1.0, 1.0));
        // The bar has faded into the background, the strokes have not.
        let faint = strip(grey, 0.2, 0.3);
        assert!(present(&look, &faint, 0.9));
        assert!(!present(&look, &faint, 0.2));
        // Nearly invisible strokes do not keep the row.
        assert!(!present(&look, &strip(grey, 0.2, 0.02), 0.9));
    }

    #[test]
    fn a_gone_row_is_not_present_on_any_background() {
        let look = look(&strip(grey, 1.0, 1.0));
        assert!(!present(&look, &strip(grey, 0.0, 0.0), 0.0));
        assert!(!present(&look, &strip(texture, 0.0, 0.0), 0.3));
    }

    #[test]
    fn learns_the_bar_colour_of_any_palette() {
        // A magenta bar on a dark background: no default colour involved.
        let magenta = |x: i32, y: i32| {
            if (360..520).contains(&x) && (ROW_TOP..ROW_TOP + 33).contains(&y) {
                [190, 40, 170]
            } else {
                [20, 20, 25]
            }
        };
        let reference = strip(magenta, 0.0, 1.0);
        let look = look(&reference);
        assert_eq!(look.bar, [190, 40, 170]);
        assert!(present(&look, &strip(magenta, 0.0, 0.0), -1.0));
        assert!(!present(&look, &strip(|_, _| [20, 20, 25], 0.0, 0.0), -1.0));
    }
}
