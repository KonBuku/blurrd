//! Image views and planes the detectors work on.

use blurrd_core::RectI;

/// Why an image view could not be created.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ImageError {
    /// The rows are shorter than `3 × width` bytes.
    #[error("stride {stride} is smaller than 3 x width {width}")]
    StrideTooSmall {
        /// The given stride.
        stride: usize,
        /// The given width.
        width: u32,
    },
    /// The buffer ends before the last row does.
    #[error("buffer of {len} bytes is too small for {height} rows with stride {stride}")]
    BufferTooSmall {
        /// The buffer length.
        len: usize,
        /// The given height.
        height: u32,
        /// The given stride.
        stride: usize,
    },
}

/// A borrowed 8-bit RGB image (`R, G, B` per pixel), such as a HUD crop
/// converted from the decoded YUV frame.
#[derive(Debug, Clone, Copy)]
pub struct RgbView<'a> {
    data: &'a [u8],
    width: u32,
    height: u32,
    stride: usize,
}

impl<'a> RgbView<'a> {
    /// A view of `height` rows of `width` pixels; row `y` starts at byte
    /// `y × stride`.
    ///
    /// # Errors
    ///
    /// [`ImageError`] if the stride is shorter than a row or the buffer
    /// shorter than the image.
    pub fn new(data: &'a [u8], width: u32, height: u32, stride: usize) -> Result<Self, ImageError> {
        let row = 3 * width as usize;
        if stride < row {
            return Err(ImageError::StrideTooSmall { stride, width });
        }
        let needed = if height == 0 {
            0
        } else {
            stride * (height as usize - 1) + row
        };
        if data.len() < needed {
            return Err(ImageError::BufferTooSmall {
                len: data.len(),
                height,
                stride,
            });
        }
        Ok(Self {
            data,
            width,
            height,
            stride,
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

    /// The `3 × width` bytes of row `y`.
    ///
    /// # Panics
    ///
    /// Panics if `y` is not below the height.
    pub fn row(&self, y: u32) -> &'a [u8] {
        let start = y as usize * self.stride;
        &self.data[start..start + 3 * self.width as usize]
    }

    /// The colour of pixel `(x, y)`.
    ///
    /// # Panics
    ///
    /// Panics if the pixel is outside the image.
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 3] {
        let row = self.row(y);
        let i = 3 * x as usize;
        [row[i], row[i + 1], row[i + 2]]
    }
}

/// An owned single-channel 8-bit image, row after row without padding.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GrayImage {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

impl GrayImage {
    /// A black image.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            data: vec![0; width as usize * height as usize],
        }
    }

    /// Wraps pixel data, or returns `None` if its length is not
    /// `width × height`.
    pub fn from_vec(width: u32, height: u32, data: Vec<u8>) -> Option<Self> {
        (data.len() == width as usize * height as usize).then_some(Self {
            width,
            height,
            data,
        })
    }

    /// Changes the size, reusing the allocation; the content is
    /// unspecified afterwards.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        self.data.resize(width as usize * height as usize, 0);
    }

    /// Width in pixels.
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// All pixels, row after row.
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// All pixels, row after row, writable.
    pub fn data_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }

    /// Pixel `(x, y)`.
    ///
    /// # Panics
    ///
    /// Panics if the pixel is outside the image.
    pub fn get(&self, x: u32, y: u32) -> u8 {
        self.data[y as usize * self.width as usize + x as usize]
    }

    /// Row `y`.
    ///
    /// # Panics
    ///
    /// Panics if `y` is not below the height.
    pub fn row(&self, y: u32) -> &[u8] {
        let w = self.width as usize;
        &self.data[y as usize * w..(y as usize + 1) * w]
    }
}

/// A binary mask: one bit of a per-pixel flag plane.
///
/// Pixel classes are stored as bit flags, one byte per pixel; a `BitMask`
/// selects one class. A plane of plain booleans is a `BitMask` with bit 1
/// over bytes that are 0 or 1.
#[derive(Debug, Clone, Copy)]
pub struct BitMask<'a> {
    flags: &'a [u8],
    bit: u8,
    width: u32,
    height: u32,
}

impl<'a> BitMask<'a> {
    /// The pixels of a `width × height` flag plane whose flags contain
    /// `bit`, or `None` if `flags` is shorter than the image.
    pub fn new(flags: &'a [u8], bit: u8, width: u32, height: u32) -> Option<Self> {
        (flags.len() >= width as usize * height as usize).then_some(Self {
            flags,
            bit,
            width,
            height,
        })
    }

    /// A mask over a plane the crate built with exactly `width × height`
    /// flags.
    pub(crate) fn from_plane(flags: &'a [u8], bit: u8, width: u32, height: u32) -> Self {
        debug_assert_eq!(flags.len(), width as usize * height as usize);
        Self {
            flags,
            bit,
            width,
            height,
        }
    }

    /// Width in pixels.
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// The flag bit the mask selects.
    pub(crate) const fn bit(&self) -> u8 {
        self.bit
    }

    /// The flags of row `y` (test them with [`BitMask::bit`]).
    ///
    /// # Panics
    ///
    /// Panics if `y` is not below the height.
    pub(crate) fn row(&self, y: u32) -> &'a [u8] {
        let w = self.width as usize;
        &self.flags[y as usize * w..(y as usize + 1) * w]
    }

    /// Whether pixel `(x, y)` is set; `false` outside the mask.
    pub fn get(&self, x: i32, y: i32) -> bool {
        x >= 0
            && y >= 0
            && (x as u32) < self.width
            && (y as u32) < self.height
            && self.flags[y as usize * self.width as usize + x as usize] & self.bit != 0
    }

    /// The whole mask as a rectangle, or `None` for an empty mask.
    pub fn bounds(&self) -> Option<RectI> {
        RectI::new(0, 0, self.width as i32, self.height as i32).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::{BitMask, GrayImage, ImageError, RgbView};

    #[test]
    fn validates_views() {
        let data = [0u8; 2 * 8 + 6];
        let view = RgbView::new(&data, 2, 3, 8).unwrap();
        assert_eq!(view.row(2).len(), 6);
        assert_eq!(
            RgbView::new(&data, 3, 3, 8).unwrap_err(),
            ImageError::StrideTooSmall {
                stride: 8,
                width: 3
            }
        );
        assert!(matches!(
            RgbView::new(&data, 2, 4, 8),
            Err(ImageError::BufferTooSmall { .. })
        ));
        assert!(RgbView::new(&[], 5, 0, 15).is_ok());
    }

    #[test]
    fn reads_pixels_and_bits() {
        let data = [1, 2, 3, 4, 5, 6, 0, 0, 7, 8, 9, 10, 11, 12, 0, 0];
        let view = RgbView::new(&data, 2, 2, 8).unwrap();
        assert_eq!(view.pixel(1, 1), [10, 11, 12]);
        let img = GrayImage::from_vec(2, 2, vec![1, 2, 3, 4]).unwrap();
        assert_eq!((img.get(1, 0), img.row(1)), (2, &[3u8, 4][..]));
        let flags = [0b01, 0b10, 0b11, 0];
        let m = BitMask::new(&flags, 0b10, 2, 2).unwrap();
        assert!(m.get(1, 0) && m.get(0, 1) && !m.get(0, 0) && !m.get(2, 0) && !m.get(-1, 0));
    }
}
