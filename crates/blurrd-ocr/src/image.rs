//! Borrowed RGB images, the input of every OCR call.

use blurrd_core::RectI;

use crate::error::OcrError;

/// A borrowed 8-bit RGB image: `R, G, B` per pixel, rows `stride` bytes
/// apart. Crops of a larger image are views with the parent's stride, so
/// cropping never copies.
///
/// A view also knows its pixel aspect ratio ([`RgbView::pixel_aspect`]):
/// the HUD of a stretched 4:3 recording is wider than its natural shape,
/// and OCR resamples such images back to natural proportions so text
/// reaches the models at the same size in every layout.
#[derive(Debug, Clone, Copy)]
pub struct RgbView<'a> {
    width: u32,
    height: u32,
    stride: usize,
    pixel_aspect: f32,
    data: &'a [u8],
}

impl<'a> RgbView<'a> {
    /// Wraps packed rows (`stride = 3 × width`).
    ///
    /// # Errors
    ///
    /// [`OcrError::InvalidInput`] if the image is empty or `data` is
    /// shorter than `3 × width × height` bytes.
    pub fn new(width: u32, height: u32, data: &'a [u8]) -> Result<Self, OcrError> {
        Self::with_stride(width, height, 3 * width as usize, data)
    }

    /// Wraps rows that are `stride` bytes apart.
    ///
    /// # Errors
    ///
    /// [`OcrError::InvalidInput`] if the image is empty, `stride` is
    /// smaller than a row or `data` does not hold every row.
    pub fn with_stride(
        width: u32,
        height: u32,
        stride: usize,
        data: &'a [u8],
    ) -> Result<Self, OcrError> {
        let row = 3 * width as usize;
        if width == 0 || height == 0 {
            return Err(OcrError::InvalidInput(format!(
                "empty image {width}x{height}"
            )));
        }
        let needed = (height as usize - 1) * stride + row;
        if stride < row || data.len() < needed {
            return Err(OcrError::InvalidInput(format!(
                "{width}x{height} RGB image with stride {stride} needs {needed} bytes, got {}",
                data.len()
            )));
        }
        Ok(Self {
            width,
            height,
            stride,
            pixel_aspect: 1.0,
            data,
        })
    }

    /// The same pixels with a pixel aspect ratio of `aspect`: the natural
    /// width of one pixel divided by its natural height. Content scaled by
    /// `sx` horizontally and `sy` vertically has the aspect `sy / sx`; for
    /// example 0.75 for a 4:3 HUD stretched to 16:9. Values that are not
    /// finite and positive mean 1 (square pixels, the default).
    #[must_use]
    pub fn with_pixel_aspect(self, aspect: f32) -> Self {
        Self {
            pixel_aspect: if aspect.is_finite() && aspect > 0.0 {
                aspect
            } else {
                1.0
            },
            ..self
        }
    }

    /// The pixel aspect ratio (natural width over natural height of one
    /// pixel; 1 for square pixels).
    pub const fn pixel_aspect(&self) -> f32 {
        self.pixel_aspect
    }

    /// The width of the image in natural proportions: [`Self::width`]
    /// times [`Self::pixel_aspect`].
    pub fn natural_width(&self) -> f32 {
        self.width as f32 * self.pixel_aspect
    }

    /// Width in pixels.
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// The whole image as a rectangle.
    pub const fn bounds(&self) -> RectI {
        RectI {
            x0: 0,
            y0: 0,
            x1: self.width as i32,
            y1: self.height as i32,
        }
    }

    /// The `R, G, B` bytes of row `y` (`y < height`).
    pub(crate) fn row(&self, y: u32) -> &'a [u8] {
        let start = y as usize * self.stride;
        &self.data[start..start + 3 * self.width as usize]
    }

    /// The part of the image inside `rect`, clipped to the image, or `None`
    /// if nothing is left. The crop keeps the pixel aspect ratio.
    pub fn crop(&self, rect: RectI) -> Option<RgbView<'a>> {
        let clipped = rect.intersection(&self.bounds())?;
        let start = clipped.y0 as usize * self.stride + 3 * clipped.x0 as usize;
        Some(RgbView {
            width: clipped.width(),
            height: clipped.height(),
            stride: self.stride,
            pixel_aspect: self.pixel_aspect,
            data: &self.data[start..],
        })
    }
}

#[cfg(test)]
mod tests {
    use blurrd_core::RectI;

    use super::RgbView;

    #[test]
    fn crops_are_views_into_the_parent() {
        // 4x2 image whose red channel holds the pixel index.
        let data: Vec<u8> = (0..8u8).flat_map(|i| [i, 0, 0]).collect();
        let image = RgbView::new(4, 2, &data).unwrap();
        let crop = image.crop(RectI::new(1, 1, 9, 9).unwrap()).unwrap();
        assert_eq!((crop.width(), crop.height()), (3, 1));
        assert_eq!(crop.row(0), &[5, 0, 0, 6, 0, 0, 7, 0, 0]);
        assert!(image.crop(RectI::new(4, 0, 6, 2).unwrap()).is_none());
        // Crops keep the pixel aspect ratio.
        let wide = image.with_pixel_aspect(0.75);
        let crop = wide.crop(RectI::new(0, 0, 2, 2).unwrap()).unwrap();
        assert_eq!(crop.pixel_aspect(), 0.75);
        assert!((crop.natural_width() - 1.5).abs() < 1e-6);
        assert_eq!(image.with_pixel_aspect(f32::NAN).pixel_aspect(), 1.0);
        assert_eq!(image.with_pixel_aspect(0.0).pixel_aspect(), 1.0);
    }

    #[test]
    fn rejects_short_buffers() {
        assert!(RgbView::new(4, 2, &[0; 23]).is_err());
        assert!(RgbView::new(0, 2, &[]).is_err());
        assert!(RgbView::with_stride(4, 2, 11, &[0; 24]).is_err());
        assert!(RgbView::with_stride(4, 2, 16, &[0; 28]).is_ok());
    }
}
