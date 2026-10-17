//! The blurred pixels of a name part, made as the render makes them, and
//! their read.
//!
//! The blur writes only inside its padded boxes and reads only the source
//! pixels there (the locality guarantee of `blurrd-blur`), so blurring a
//! crop that holds every padded box touching a part gives exactly the
//! output's pixels of that part, minus encoder loss.

use std::path::Path;
use std::sync::Arc;

use blurrd_blur::{BlurBackend, CpuBlur, PixelParams, PreparedBox, prepare_boxes};
use blurrd_core::{AppError, BlurBox, BlurParams, ErrorKind, HudGeometry, RectI, RegionSet};
use blurrd_ocr::{LeakMatcher, NameMatch, OcrEngine, RgbView};
use blurrd_video::{
    DecodeOptions, FrameSeeker, PtsTable, RgbConverter, RgbImage, SeekerOptions, VideoColour,
    YuvRoi,
};

use super::scan::Exposure;
use super::union;
use crate::job::internal;

/// Frames the verification's seeker keeps decoded (it reads parts of the
/// same few frames again after each fix).
const SEEKER_CACHE_BYTES: usize = 96 << 20;

/// Reads the blurred pixels of name parts of one video.
pub(crate) struct Pixels<'a> {
    seeker: FrameSeeker,
    ocr: &'a mut OcrEngine,
    params: PixelParams,
    converter: RgbConverter,
    regions: RegionSet,
    /// Natural width over natural height of one source pixel of the HUD.
    pixel_aspect: f32,
    size: (u32, u32),
    prepared: Vec<PreparedBox>,
    roi: YuvRoi,
    rgb: RgbImage,
}

impl std::fmt::Debug for Pixels<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pixels")
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

/// How the blurred pixels of a video are made and read.
#[derive(Debug, Clone)]
pub(crate) struct PixelSetup {
    /// How the video decodes.
    pub(crate) decode: DecodeOptions,
    /// Its blur parameters.
    pub(crate) params: PixelParams,
    /// Its RGB conversion (the one pass 0 read the source with).
    pub(crate) converter: RgbConverter,
    /// The regions that are blurred.
    pub(crate) regions: RegionSet,
    /// Natural width over natural height of one source pixel of the HUD.
    pub(crate) pixel_aspect: f32,
    /// Frame size.
    pub(crate) size: (u32, u32),
}

impl PixelSetup {
    /// The setup for a source with the HUD at `geometry`, decoded with
    /// `decode` and converted with `converter` (for its colour `colour`),
    /// blurred with `blur` in the regions `regions`.
    ///
    /// # Errors
    ///
    /// [`blurrd_core::ErrorKind::InvalidProject`] for blur parameters that
    /// do not fit the video.
    pub(crate) fn source(
        decode: DecodeOptions,
        blur: &BlurParams,
        geometry: &HudGeometry,
        colour: VideoColour,
        converter: RgbConverter,
        regions: RegionSet,
        size: (u32, u32),
    ) -> Result<Self, AppError> {
        let params = PixelParams::new(blur, geometry.sy, colour.yuv_colour()).map_err(|e| {
            AppError::new(ErrorKind::InvalidProject, "The blur settings are invalid.")
                .with_detail(e.to_string())
        })?;
        Ok(Self {
            decode,
            params,
            converter,
            regions,
            pixel_aspect: geometry.sy / geometry.sx,
            size,
        })
    }
}

impl<'a> Pixels<'a> {
    /// Opens `input`, whose PTS table is `pts`, for reads with `ocr`.
    ///
    /// # Errors
    ///
    /// The decoder's errors.
    pub(crate) fn open(
        input: &Path,
        pts: Arc<PtsTable>,
        setup: PixelSetup,
        ocr: &'a mut OcrEngine,
    ) -> Result<Self, AppError> {
        let mut options = SeekerOptions::default();
        options.decode = setup.decode;
        options.cache_bytes = SEEKER_CACHE_BYTES;
        options.prefetch = false;
        Ok(Self {
            seeker: FrameSeeker::open(input, pts, options)?,
            ocr,
            params: setup.params,
            converter: setup.converter,
            regions: setup.regions,
            pixel_aspect: setup.pixel_aspect,
            size: setup.size,
            prepared: Vec::new(),
            roi: YuvRoi::new(),
            rgb: RgbImage::default(),
        })
    }

    /// The regions that are blurred.
    pub(crate) fn regions(&self) -> RegionSet {
        self.regions
    }

    /// Blurs `exposure`'s frame with `boxes` (that frame's boxes of the
    /// plan) around its name part and reads the part; how the read shows
    /// a name and its confidence, or `None` when it does not.
    ///
    /// # Errors
    ///
    /// Decoding, blur and OCR errors.
    pub(crate) fn read(
        &mut self,
        exposure: &Exposure,
        boxes: &[BlurBox],
        matcher: &LeakMatcher<'_>,
    ) -> Result<Option<(NameMatch, f32)>, AppError> {
        let frame = self.seeker.frame(exposure.frame)?;
        // The part at its line's height, as OCR read it on the source.
        let crop = RectI {
            x0: exposure.part.x0,
            y0: exposure.line.y0,
            x1: exposure.part.x1,
            y1: exposure.line.y1,
        };
        prepare_boxes(
            boxes,
            self.size.0,
            self.size.1,
            &self.params,
            &mut self.prepared,
        );
        let view = covering_view(crop, &self.prepared, self.size);
        frame.roi_yuv(view, &mut self.roi)?;
        let copied = self
            .roi
            .rect()
            .ok_or_else(|| internal("the frame copy for the verification is empty"))?;
        self.prepared
            .retain(|b| b.padded().intersection(&copied) == Some(b.padded()));
        if let Some(mut planes) = self.roi.as_frame_mut() {
            CpuBlur
                .blur(
                    &mut planes,
                    (copied.x0, copied.y0),
                    &self.prepared,
                    &self.params,
                )
                .map_err(|e| internal(format!("verification blur: {e}")))?;
        }
        self.converter.convert(&self.roi, &mut self.rgb)?;
        let whole = RgbView::new(self.rgb.width, self.rgb.height, &self.rgb.data)
            .map_err(|e| internal(format!("verification crop: {e}")))?;
        let Some(part) = crop.intersection(&copied).and_then(|c| {
            whole.crop(RectI {
                x0: c.x0 - copied.x0,
                y0: c.y0 - copied.y0,
                x1: c.x1 - copied.x0,
                y1: c.y1 - copied.y0,
            })
        }) else {
            return Ok(None);
        };
        let reads = self
            .ocr
            .recognize(&[part.with_pixel_aspect(self.pixel_aspect)])
            .map_err(|e| internal(format!("verification OCR: {e}")))?;
        Ok(reads
            .first()
            .and_then(|r| matcher.classify(&r.text, r.conf).map(|m| (m, r.conf))))
    }
}

/// The smallest view around `crop` that holds every padded box touching
/// it, and every padded box touching those, inside the frame.
fn covering_view(crop: RectI, prepared: &[PreparedBox], size: (u32, u32)) -> RectI {
    let mut view = crop;
    loop {
        let grown = prepared
            .iter()
            .map(PreparedBox::padded)
            .filter(|p| p.intersection(&view).is_some())
            .fold(view, union);
        if grown == view {
            break;
        }
        view = grown;
    }
    let frame = RectI {
        x0: 0,
        y0: 0,
        x1: size.0 as i32,
        y1: size.1 as i32,
    };
    view.intersection(&frame).unwrap_or(crop)
}

#[cfg(test)]
mod tests {
    use blurrd_blur::{PixelParams, YuvColour, prepare_boxes};
    use blurrd_core::{BlurBox, BlurParams, RectI, Region};

    use super::covering_view;

    #[test]
    fn the_view_holds_every_padded_box_touching_the_part() {
        let params = PixelParams::new(&BlurParams::default(), 1.0, YuvColour::default()).unwrap();
        let blur = |x0, y0, x1, y1| BlurBox {
            rect: RectI::new(x0, y0, x1, y1).unwrap(),
            region: Region::KillFeed,
            seed: 1,
            grid_origin: (x0, y0),
            block_scale: 1.0,
        };
        let boxes = [
            blur(100, 100, 160, 120),
            // Touches the first box's padding only.
            blur(165, 100, 260, 120),
            // Far away.
            blur(800, 500, 900, 520),
        ];
        let mut prepared = Vec::new();
        prepare_boxes(&boxes, 1920, 1080, &params, &mut prepared);
        let view = covering_view(
            RectI::new(120, 100, 150, 120).unwrap(),
            &prepared,
            (1920, 1080),
        );
        assert_eq!(
            view.intersection(&prepared[0].padded()),
            Some(prepared[0].padded())
        );
        assert_eq!(
            view.intersection(&prepared[1].padded()),
            Some(prepared[1].padded())
        );
        assert!(view.intersection(&prepared[2].padded()).is_none());
    }
}
