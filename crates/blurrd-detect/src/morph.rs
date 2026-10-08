//! Morphology with rectangular structuring elements.
//!
//! The results match OpenCV's `erode`, `dilate` and
//! `morphologyEx(MORPH_TOPHAT)` with a rectangular kernel, the default
//! anchor (the kernel centre, `size / 2`) and the default border, which the
//! prototype was tuned with: pixels outside the image are ignored (erosion
//! treats them as +∞, dilation as −∞).
//!
//! A rectangle is separable, so each operation is a horizontal pass and a
//! vertical pass. Both are written as element-wise minima or maxima of
//! shifted slices, which the compiler vectorises; for the 5 and 7 pixel
//! kernels of the detectors that is faster than a running-extremum
//! algorithm.

use crate::image::GrayImage;

/// Reusable buffers for the morphology functions.
#[derive(Debug, Clone, Default)]
pub struct MorphScratch {
    row: Vec<u8>,
    tmp: GrayImage,
    opened: GrayImage,
}

#[derive(Clone, Copy)]
enum Op {
    Min,
    Max,
}

impl Op {
    /// The value outside the image that leaves the result unchanged.
    const fn neutral(self) -> u8 {
        match self {
            Self::Min => u8::MAX,
            Self::Max => 0,
        }
    }

    fn apply(self, acc: &mut [u8], other: &[u8]) {
        match self {
            Self::Min => acc
                .iter_mut()
                .zip(other)
                .for_each(|(a, b)| *a = (*a).min(*b)),
            Self::Max => acc
                .iter_mut()
                .zip(other)
                .for_each(|(a, b)| *a = (*a).max(*b)),
        }
    }
}

/// Erosion: the minimum over a `kw × kh` rectangle anchored at its centre.
///
/// `dst` is resized to the size of `src`. Kernel sizes of 0 are treated
/// as 1 (no change along that axis).
pub fn erode(src: &GrayImage, kw: u32, kh: u32, dst: &mut GrayImage, scratch: &mut MorphScratch) {
    rect_filter(src, kw, kh, Op::Min, dst, scratch);
}

/// Dilation: the maximum over a `kw × kh` rectangle anchored at its
/// centre. See [`erode`].
pub fn dilate(src: &GrayImage, kw: u32, kh: u32, dst: &mut GrayImage, scratch: &mut MorphScratch) {
    rect_filter(src, kw, kh, Op::Max, dst, scratch);
}

/// White top-hat: `src − dilate(erode(src))` with a `kw × kh` rectangle.
///
/// Bright details smaller than the kernel (text strokes) keep their
/// contrast to the local background; larger bright areas become 0.
pub fn white_tophat(
    src: &GrayImage,
    kw: u32,
    kh: u32,
    dst: &mut GrayImage,
    scratch: &mut MorphScratch,
) {
    let mut eroded = std::mem::take(&mut scratch.opened);
    rect_filter(src, kw, kh, Op::Min, &mut eroded, scratch);
    rect_filter(&eroded, kw, kh, Op::Max, dst, scratch);
    // With an odd kernel the opening is never brighter than the source.
    // With an even one both passes use the same anchor, so it can be; the
    // difference then saturates at 0, as in OpenCV.
    for (d, s) in dst.data_mut().iter_mut().zip(src.data()) {
        *d = s.saturating_sub(*d);
    }
    scratch.opened = eroded;
}

fn rect_filter(
    src: &GrayImage,
    kw: u32,
    kh: u32,
    op: Op,
    dst: &mut GrayImage,
    scratch: &mut MorphScratch,
) {
    let (w, h) = (src.width() as usize, src.height() as usize);
    let (kw, kh) = (kw.max(1) as usize, kh.max(1) as usize);
    let (ax, ay) = (kw / 2, kh / 2);
    dst.resize(src.width(), src.height());
    let tmp = &mut scratch.tmp;
    tmp.resize(src.width(), src.height());
    // Horizontal pass into `tmp`: pad each row with the neutral value.
    let row = &mut scratch.row;
    row.clear();
    row.resize(w + kw - 1, op.neutral());
    for y in 0..h {
        row[ax..ax + w].copy_from_slice(&src.data()[y * w..(y + 1) * w]);
        let out = &mut tmp.data_mut()[y * w..(y + 1) * w];
        out.copy_from_slice(&row[..w]);
        for i in 1..kw {
            op.apply(out, &row[i..i + w]);
        }
    }
    // Vertical pass into `dst`: rows outside the image are skipped.
    for y in 0..h {
        let first = y.saturating_sub(ay);
        let last = (y + kh - ay).min(h);
        let out = &mut dst.data_mut()[y * w..(y + 1) * w];
        out.copy_from_slice(&tmp.data()[first * w..(first + 1) * w]);
        for r in first + 1..last {
            op.apply(out, &tmp.data()[r * w..(r + 1) * w]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MorphScratch, dilate, erode, white_tophat};
    use crate::image::GrayImage;

    #[test]
    fn single_bright_pixel() {
        let mut src = GrayImage::new(7, 5);
        src.data_mut()[2 * 7 + 3] = 200;
        let mut s = MorphScratch::default();
        let mut out = GrayImage::default();
        dilate(&src, 3, 3, &mut out, &mut s);
        assert_eq!(
            out.data().iter().map(|&v| u32::from(v == 200)).sum::<u32>(),
            9
        );
        erode(&src, 3, 3, &mut out, &mut s);
        assert!(out.data().iter().all(|&v| v == 0));
        white_tophat(&src, 3, 3, &mut out, &mut s);
        assert_eq!(out, src, "a 1-pixel detail survives the top-hat unchanged");
    }

    #[test]
    fn borders_are_ignored() {
        // A uniform image stays uniform: the outside neither darkens the
        // erosion nor brightens the dilation.
        let src = GrayImage::from_vec(4, 3, vec![90; 12]).unwrap();
        let mut s = MorphScratch::default();
        let mut out = GrayImage::default();
        erode(&src, 7, 7, &mut out, &mut s);
        assert_eq!(out, src);
        dilate(&src, 7, 7, &mut out, &mut s);
        assert_eq!(out, src);
    }

    #[test]
    fn even_kernels_anchor_right_of_centre() {
        // OpenCV anchors a kernel of width 2 at x = 1: the window of x is
        // [x - 1, x].
        let src = GrayImage::from_vec(4, 1, vec![0, 0, 9, 0]).unwrap();
        let mut s = MorphScratch::default();
        let mut out = GrayImage::default();
        dilate(&src, 2, 1, &mut out, &mut s);
        assert_eq!(out.data(), [0, 0, 9, 9]);
    }
}
