//! The crops of one frame that the detector stages read, and its screen
//! surfaces read by the crop workers.

use blurrd_core::{AppError, RectI, Region, RegionSet};
use blurrd_detect::chat::LEFT_PANEL_ROI;
use blurrd_detect::killfeed::FEED_ROI;
use blurrd_detect::screens::{Anchor, FrameView, SurfaceFrame};
use blurrd_detect::{ImageError, RgbView, ScreenDetector};
use blurrd_video::{RgbConverter, RgbImage, RoiLayout, VideoColour, VideoFrame, YuvRoi};

use crate::dictionary::ZoneKind;
use crate::hud::{Crop, HudMap, QUARTER, quarter, rgb_view};
use crate::job::internal;
use crate::resample::{RgbSource, resample};
use crate::scaler::SwsScaler;

/// One frame and its crops at the reference scale.
#[derive(Debug)]
pub(crate) struct FrameCrops {
    /// Presentation index.
    pub(crate) index: u32,
    /// The decoded frame (a reference, no copy), for the previews.
    pub(crate) frame: VideoFrame,
    /// The kill feed and banner area ([`FEED_ROI`]), right-anchored.
    pub(crate) feed: Crop,
    /// The same area centre-anchored for the banner, when that maps
    /// differently (HUD layouts that are not 16:9).
    banner: Option<Crop>,
    /// The left panel ([`LEFT_PANEL_ROI`]: voice chat and text chat).
    pub(crate) left: Crop,
    /// The whole reference frame as 4 × 4 means ([`QUARTER`]).
    quarter: RgbImage,
    /// The screen surfaces' gate signals and name slots of this frame,
    /// when a screen surface is analysed.
    pub(crate) surfaces: Option<[SurfaceFrame; 4]>,
    /// The kill feed and left panel areas in source pixels, for pass 0,
    /// when the reference frame is not the frame itself (otherwise `feed`
    /// and `left` are source pixels already).
    native: Option<[Crop; 2]>,
}

impl FrameCrops {
    /// The banner's crop.
    pub(crate) fn banner(&self) -> &Crop {
        self.banner.as_ref().unwrap_or(&self.feed)
    }

    /// The downscaled whole frame.
    pub(crate) fn quarter_view(&self) -> Result<RgbView<'_>, ImageError> {
        rgb_view(&self.quarter)
    }

    /// The source pixels of `rect` (source coordinates) in the area of
    /// `kind`, if this frame holds them all: what OCR reads at native
    /// resolution.
    pub(crate) fn ocr_view(&self, kind: ZoneKind, rect: RectI) -> Option<blurrd_ocr::RgbView<'_>> {
        let (feed, left) = match &self.native {
            Some([feed, left]) => (feed, left),
            None => (&self.feed, &self.left),
        };
        match kind {
            ZoneKind::FeedAndBanner => feed.ocr_view(rect),
            ZoneKind::LeftPanel => left.ocr_view(rect),
        }
    }
}

/// Makes the crops of frames; one per crop worker (it keeps buffers).
#[derive(Debug, Clone)]
pub(crate) struct Cropper {
    map: HudMap,
    reference: ReferenceFrame,
    /// The current frame at the reference scale.
    image: RgbImage,
    /// The banner's own place in the reference frame, when it is analysed
    /// and maps elsewhere than the kill feed (HUD layouts that are not
    /// 16:9).
    banner: Option<RectI>,
    /// Whether any HUD region is analysed (otherwise no HUD crop is made).
    hud: bool,
    /// Whether the downscaled whole frame is read (voice-chat scene cuts,
    /// screen-surface gates).
    quarter: bool,
    /// Whether pass 0 reads source-pixel crops.
    ocr: bool,
    roi: YuvRoi,
    /// Reads the screen surfaces, when one is analysed.
    screens: Option<ScreenReader>,
}

impl Cropper {
    /// A cropper for `regions` of frames mapped by `map`, with colours
    /// `colour` converted by `converter`; `ocr` when pass 0 reads the
    /// protected zones.
    pub(crate) fn new(
        map: HudMap,
        colour: VideoColour,
        converter: RgbConverter,
        regions: RegionSet,
        ocr: bool,
    ) -> Self {
        Self {
            map,
            reference: ReferenceFrame::new(&map, colour, converter),
            image: RgbImage::default(),
            banner: Some(map.reference_rect(FEED_ROI, Anchor::Center)).filter(|&rect| {
                regions.contains(Region::Banner)
                    && rect != map.reference_rect(FEED_ROI, Anchor::Right)
            }),
            hud: Region::HUD.iter().any(|&r| regions.contains(r)),
            quarter: regions.contains(Region::VoiceChat)
                || Region::SCREENS.iter().any(|&r| regions.contains(r)),
            ocr,
            roi: YuvRoi::new(),
            screens: Region::SCREENS
                .iter()
                .any(|&r| regions.contains(r))
                .then(|| ScreenReader::new(regions)),
        }
    }

    /// The crops of `frame`, and its screen surfaces read.
    pub(crate) fn crops(&mut self, frame: VideoFrame) -> Result<FrameCrops, AppError> {
        if self.hud || self.quarter {
            self.reference.build(&frame, &mut self.image)?;
        }
        let crop = |this: &Self, rect: RectI, anchor: Anchor| {
            if this.hud {
                Crop::cut(&this.image, this.map.reference_rect(rect, anchor))
            } else {
                Crop::default()
            }
        };
        let feed = crop(self, FEED_ROI, Anchor::Right);
        let banner = self.banner.map(|rect| Crop::cut(&self.image, rect));
        let left = crop(self, LEFT_PANEL_ROI, Anchor::Left);
        let mut quarter_image = RgbImage::default();
        if self.quarter {
            quarter(&self.image, &mut quarter_image);
        }
        let surfaces = match &mut self.screens {
            Some(reader) => Some(reader.read(&self.image, &quarter_image, &self.map)?),
            None => None,
        };
        let native = if self.ocr && !self.map.is_identity() {
            Some([
                self.native_crop(&frame, FEED_ROI, Anchor::Right)?,
                self.native_crop(&frame, LEFT_PANEL_ROI, Anchor::Left)?,
            ])
        } else {
            None
        };
        Ok(FrameCrops {
            index: frame.index(),
            frame,
            feed,
            banner,
            left,
            quarter: quarter_image,
            surfaces,
            native,
        })
    }

    /// The source pixels of reference rectangle `rect` kept at `anchor`.
    fn native_crop(
        &mut self,
        frame: &VideoFrame,
        rect: RectI,
        anchor: Anchor,
    ) -> Result<Crop, AppError> {
        let Some(source) = self.map.source_rect(rect, anchor) else {
            return Ok(Crop::default());
        };
        frame.roi_yuv(source, &mut self.roi)?;
        let mut image = RgbImage::default();
        self.reference.converter.convert(&self.roi, &mut image)?;
        Ok(Crop::from_image(image, self.roi.rect().unwrap_or(source)))
    }
}

/// Brings whole frames to the reference scale.
///
/// A frame already at the reference scale is converted to RGB as it is.
/// Other 8-bit and 10-bit SDR frames are scaled and converted in one step
/// by [`SwsScaler`], the arithmetic the detectors were validated on
/// (10-bit samples rounded to 8 bits first, as the converter rounds
/// them). HDR frames are tone-mapped at their own size and then resampled
/// (bicubic, [`resample`]): no validated reference exists for them.
#[derive(Debug, Clone)]
struct ReferenceFrame {
    size: (u32, u32),
    identity: bool,
    colour: VideoColour,
    converter: RgbConverter,
    /// The whole decoded frame.
    roi: YuvRoi,
    /// 8-bit luma (from 10-bit samples) and 8-bit U and V planes (from
    /// interleaved or 10-bit chroma).
    planes: [Vec<u8>; 3],
    /// The tone-mapped frame at its own size (HDR).
    native: RgbImage,
    /// The scaler for the current source size.
    scaling: Option<Scaling>,
}

/// A scaler and the frames it is for.
#[derive(Debug, Clone)]
struct Scaling {
    source: (u32, u32),
    scaler: SwsScaler,
}

impl ReferenceFrame {
    fn new(map: &HudMap, colour: VideoColour, converter: RgbConverter) -> Self {
        Self {
            size: map.reference_size(),
            identity: map.is_identity(),
            colour,
            converter,
            roi: YuvRoi::new(),
            planes: Default::default(),
            native: RgbImage::default(),
            scaling: None,
        }
    }

    /// `frame` at the reference scale, into `out`.
    fn build(&mut self, frame: &VideoFrame, out: &mut RgbImage) -> Result<(), AppError> {
        let whole = RectI {
            x0: 0,
            y0: 0,
            x1: frame.width() as i32,
            y1: frame.height() as i32,
        };
        frame.roi_yuv(whole, &mut self.roi)?;
        if self.identity {
            self.converter.convert(&self.roi, out)?;
            return Ok(());
        }
        let rect = self
            .roi
            .rect()
            .ok_or_else(|| internal("the frame copy is empty"))?;
        let layout = self.roi.layout();
        let sdr = matches!(layout, RoiLayout::I420 | RoiLayout::Nv12 | RoiLayout::P010)
            && !self.converter.is_tone_mapped();
        if !sdr {
            return self.resampled(out);
        }
        let [y8, u8, v8] = &mut self.planes;
        let (luma, (c0, c1)) = (self.roi.luma(), self.roi.chroma());
        let (y, u, v): (&[u8], &[u8], &[u8]) = match (layout, c1) {
            (RoiLayout::I420, Some(c1)) => (luma, c0, c1),
            (RoiLayout::P010, _) => {
                // Rounded to 8 bits like the converter's 10-bit SDR path.
                let to8 = |s: &[u8]| {
                    let ten = u16::from_le_bytes([s[0], s[1]]) >> 6;
                    ((ten + 2) >> 2).min(255) as u8
                };
                y8.clear();
                y8.extend(luma.as_chunks::<2>().0.iter().map(|s| to8(s)));
                u8.clear();
                v8.clear();
                for pair in c0.as_chunks::<4>().0 {
                    u8.push(to8(&pair[..2]));
                    v8.push(to8(&pair[2..]));
                }
                (y8, u8, v8)
            }
            _ => {
                u8.clear();
                v8.clear();
                for pair in c0.as_chunks::<2>().0 {
                    u8.push(pair[0]);
                    v8.push(pair[1]);
                }
                (luma, u8, v8)
            }
        };
        let source = (rect.width(), rect.height());
        let scaling = match &mut self.scaling {
            Some(s) if s.source == source => s,
            slot => slot.insert(Scaling {
                source,
                scaler: SwsScaler::new(source, self.size, self.colour.matrix, self.colour.range),
            }),
        };
        scaling.scaler.scale(y, u, v, out);
        Ok(())
    }

    /// The copied frame converted at its own size and resampled to the
    /// reference scale.
    fn resampled(&mut self, out: &mut RgbImage) -> Result<(), AppError> {
        self.converter.convert(&self.roi, &mut self.native)?;
        let (w, h) = (self.native.width as usize, self.native.height as usize);
        let (rw, rh) = (self.size.0 as usize, self.size.1 as usize);
        let (fx, fy) = (w as f32 / rw as f32, h as f32 / rh as f32);
        out.width = self.size.0;
        out.height = self.size.1;
        resample(
            RgbSource {
                data: &self.native.data,
                width: w,
                height: h,
            },
            rw,
            rh,
            1.0 / fx,
            1.0 / fy,
            |i| (i as f32 + 0.5) * fx,
            |j| (j as f32 + 0.5) * fy,
            &mut out.data,
        );
        Ok(())
    }
}

/// Reads the screen surfaces of single frames: the quarter-scale gates
/// and, for a candidate surface, the name slots in the reference frame.
/// Both depend on the frame alone (the readers keep only scratch
/// buffers), so the crop workers read in parallel and the screen stage's
/// temporal model takes the results in frame order.
#[derive(Debug, Clone)]
struct ScreenReader {
    detector: ScreenDetector,
}

impl ScreenReader {
    fn new(regions: RegionSet) -> Self {
        Self {
            detector: ScreenDetector::new(&regions),
        }
    }

    /// The surfaces of the reference frame `image`, whose 4 × 4 means are
    /// `quarter`.
    fn read(
        &mut self,
        image: &RgbImage,
        quarter: &RgbImage,
        map: &HudMap,
    ) -> Result<[SurfaceFrame; 4], AppError> {
        fn view(image: &RgbImage) -> Result<RgbView<'_>, AppError> {
            rgb_view(image).map_err(|e| internal(format!("detector: {e}")))
        }
        let reference = map.reference_map();
        let signals = self.detector.gate(
            &FrameView::scaled(view(quarter)?, 1.0 / QUARTER as f32),
            reference,
        );
        let whole = FrameView::native(view(image)?);
        let mut surfaces: [SurfaceFrame; 4] = Default::default();
        for (i, region) in Region::SCREENS.into_iter().enumerate() {
            let signal = signals[i];
            surfaces[i].candidate = signal.candidate;
            surfaces[i].score = signal.score;
            if signal.candidate {
                surfaces[i].slots = self.detector.read(region, &whole, reference, &signal);
            }
        }
        Ok(surfaces)
    }
}
