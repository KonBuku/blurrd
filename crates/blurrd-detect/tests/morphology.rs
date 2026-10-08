//! Morphology, connected components and summed-area tables against naive
//! reference implementations on random images.

#![allow(
    clippy::cast_possible_wrap,
    clippy::many_single_char_names,
    clippy::unwrap_used,
    reason = "test helpers on tiny images: short coordinate names, a panic is a failure"
)]

use blurrd_core::RectI;
use blurrd_detect::components::{
    ComponentScratch, Connectivity, Integral, components, merge_runs, runs,
};
use blurrd_detect::morph::{MorphScratch, dilate, erode, white_tophat};
use blurrd_detect::{BitMask, GrayImage};
use proptest::prelude::*;

/// Naive rectangle filter with OpenCV semantics: the window of `(x, y)` is
/// `[x - kw/2, x - kw/2 + kw) × [y - kh/2, y - kh/2 + kh)`, pixels outside
/// the image are ignored.
fn naive(src: &GrayImage, kw: u32, kh: u32, max: bool) -> GrayImage {
    let (w, h) = (src.width() as i32, src.height() as i32);
    let (kw, kh) = (kw.max(1) as i32, kh.max(1) as i32);
    let mut out = GrayImage::new(src.width(), src.height());
    for y in 0..h {
        for x in 0..w {
            let mut acc: Option<u8> = None;
            for wy in y - kh / 2..y - kh / 2 + kh {
                for wx in x - kw / 2..x - kw / 2 + kw {
                    if wx >= 0 && wy >= 0 && wx < w && wy < h {
                        let v = src.get(wx as u32, wy as u32);
                        acc = Some(acc.map_or(v, |a| if max { a.max(v) } else { a.min(v) }));
                    }
                }
            }
            out.data_mut()[(y * w + x) as usize] = acc.unwrap_or(0);
        }
    }
    out
}

/// Naive 4/8-connected flood fill labelling, components sorted like the
/// real one (raster order of the first pixel).
fn naive_components(flags: &[u8], w: i32, h: i32, eight: bool) -> Vec<(RectI, u32)> {
    let mut label = vec![0usize; flags.len()];
    let mut out = Vec::new();
    for start in 0..flags.len() {
        if flags[start] == 0 || label[start] != 0 {
            continue;
        }
        let id = out.len() + 1;
        let mut todo = vec![start];
        label[start] = id;
        let (mut x0, mut y0, mut x1, mut y1, mut area) = (i32::MAX, i32::MAX, 0, 0, 0u32);
        while let Some(i) = todo.pop() {
            let (x, y) = ((i as i32) % w, (i as i32) / w);
            area += 1;
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x + 1);
            y1 = y1.max(y + 1);
            for dy in -1..=1 {
                for dx in -1i32..=1 {
                    if (dx == 0 && dy == 0) || (!eight && dx != 0 && dy != 0) {
                        continue;
                    }
                    let (nx, ny) = (x + dx, y + dy);
                    if nx >= 0 && ny >= 0 && nx < w && ny < h {
                        let j = (ny * w + nx) as usize;
                        if flags[j] != 0 && label[j] == 0 {
                            label[j] = id;
                            todo.push(j);
                        }
                    }
                }
            }
        }
        out.push((RectI::new(x0, y0, x1, y1).unwrap(), area));
    }
    out
}

fn image() -> impl Strategy<Value = GrayImage> {
    (1u32..24, 1u32..24).prop_flat_map(|(w, h)| {
        prop::collection::vec(any::<u8>(), (w * h) as usize)
            .prop_map(move |data| GrayImage::from_vec(w, h, data).unwrap())
    })
}

fn binary() -> impl Strategy<Value = (Vec<u8>, i32, i32)> {
    (1i32..20, 1i32..20).prop_flat_map(|(w, h)| {
        prop::collection::vec(prop::bool::weighted(0.45), (w * h) as usize)
            .prop_map(move |bits| (bits.into_iter().map(u8::from).collect(), w, h))
    })
}

proptest! {
    #[test]
    fn erode_and_dilate_match_naive(src in image(), kw in 0u32..10, kh in 0u32..10) {
        let mut scratch = MorphScratch::default();
        let mut out = GrayImage::default();
        erode(&src, kw, kh, &mut out, &mut scratch);
        prop_assert_eq!(&out, &naive(&src, kw, kh, false));
        dilate(&src, kw, kh, &mut out, &mut scratch);
        prop_assert_eq!(&out, &naive(&src, kw, kh, true));
    }

    #[test]
    fn tophat_matches_naive(src in image(), k in 1u32..10) {
        let mut scratch = MorphScratch::default();
        let mut out = GrayImage::default();
        white_tophat(&src, k, k, &mut out, &mut scratch);
        let opened = naive(&naive(&src, k, k, false), k, k, true);
        let expected: Vec<u8> = src.data().iter().zip(opened.data()).map(|(s, o)| s.saturating_sub(*o)).collect();
        prop_assert_eq!(out.data(), &expected[..]);
    }

    #[test]
    fn components_match_naive((flags, w, h) in binary(), eight in any::<bool>()) {
        let mask = BitMask::new(&flags, 1, w as u32, h as u32).unwrap();
        let conn = if eight { Connectivity::Eight } else { Connectivity::Four };
        let mut out = vec![];
        components(&mask, mask.bounds().unwrap(), conn, &mut ComponentScratch::default(), &mut out);
        let got: Vec<(RectI, u32)> = out.iter().map(|c| (c.bounds, c.area)).collect();
        prop_assert_eq!(got, naive_components(&flags, w, h, eight));
    }

    #[test]
    fn integral_matches_counting(
        (flags, w, h) in binary(),
        r in (-3i32..22, -3i32..22, 1i32..12, 1i32..12),
    ) {
        let mask = BitMask::new(&flags, 1, w as u32, h as u32).unwrap();
        let rect = RectI::new(r.0, r.1, r.0 + r.2, r.1 + r.3).unwrap();
        let expected = (rect.y0.max(0)..rect.y1.min(h))
            .flat_map(|y| (rect.x0.max(0)..rect.x1.min(w)).map(move |x| (x, y)))
            .filter(|&(x, y)| flags[(y * w + x) as usize] != 0)
            .count() as u32;
        prop_assert_eq!(Integral::new(&mask).sum(rect), expected);
    }

    #[test]
    fn merged_runs_cover_the_same_values(values in prop::collection::vec(any::<bool>(), 0..60), gap in 0u32..4) {
        let r = runs(values.iter().copied());
        let set: usize = r.iter().map(|run| run.len() as usize).sum();
        prop_assert_eq!(set, values.iter().filter(|v| **v).count());
        let merged = merge_runs(&r, gap);
        for pair in merged.windows(2) {
            prop_assert!(pair[1].start - pair[0].end > gap);
        }
        for run in &r {
            prop_assert!(merged.iter().any(|m| m.start <= run.start && run.end <= m.end));
        }
    }
}
