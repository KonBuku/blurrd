//! The crops of one frame that the detector stages read, and its screen
//! surfaces read by the crop workers.

use blurrd_core::{AppError, RectI, Region, RegionSet};
use blurrd_detect::banner;
use blurrd_detect::chat::LEFT_PANEL_ROI;
use blurrd_detect::killfeed::FEED_ROI;
use blurrd_detect::screens::{
    self, Anchor, FrameView, GateSignal, ScreenSignals, Slot, SurfaceFrame,
};
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
    /// The name zones of the screen surfaces (in [`Region::SCREENS`]
    /// order) in source pixels, for pass 0, on frames where the surface's
    /// gate made a candidate.
    screens: [Option<Crop>; 4],
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
            ZoneKind::Screen(region) => Region::SCREENS
                .iter()
                .position(|&r| r == region)
                .and_then(|i| self.screens[i].as_ref())
                .and_then(|crop| crop.ocr_view(rect)),
        }
    }
}

/// What pass 0 reads beyond the HUD zones, so the crop workers cut it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OcrReads {
    /// The name zones of the screen surfaces in source pixels (in
    /// [`Region::SCREENS`] order), for the analysed surfaces.
    pub(crate) screens: [Option<RectI>; 4],
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
    /// What pass 0 reads, when it runs.
    ocr: Option<OcrReads>,
    roi: YuvRoi,
    /// Reads the screen surfaces, when one is analysed.
    screens: Option<ScreenReader>,
}

impl Cropper {
    /// A cropper for `regions` of frames mapped by `map`, with colours
    /// `colour` converted by `converter`; `ocr` says what pass 0 reads
    /// when it runs.
    pub(crate) fn new(
        map: HudMap,
        colour: VideoColour,
        converter: RgbConverter,
        regions: RegionSet,
        ocr: Option<OcrReads>,
    ) -> Self {
        Self {
            map,
            reference: ReferenceFrame::new(&map, colour, converter),
            image: RgbImage::default(),
            banner: Some(map.reference_rect(FEED_ROI, banner::ANCHOR.into())).filter(|&rect| {
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
        let crop = |this: &Self, rect: RectI, anchor: Anchor| {
            if this.hud {
                Crop::black(this.map.reference_rect(rect, anchor))
            } else {
                Crop::default()
            }
        };
        let mut feed = crop(self, FEED_ROI, Anchor::Right);
        let mut banner = self.banner.map(Crop::black);
        let mut left = crop(self, LEFT_PANEL_ROI, Anchor::Left);
        let mut quarter_image = RgbImage::default();
        // Whether `self.image` holds this frame at the reference scale.
        let mut whole = false;
        if self.reference.identity {
            // The same bytes as cutting the converted whole frame, without
            // converting all of it: the crops converted on their own, the
            // quarter view from block means.
            for crop in [&mut feed, &mut left].into_iter().chain(banner.as_mut()) {
                self.reference.convert_into(&frame, crop)?;
            }
            if self.quarter && !self.reference.means(&frame, &mut quarter_image)? {
                self.reference.build(&frame, &mut self.image)?;
                whole = true;
                quarter(&self.image, &mut quarter_image);
            }
        } else if self.hud || self.quarter {
            self.reference.build(&frame, &mut self.image)?;
            whole = true;
            for crop in [&mut feed, &mut left].into_iter().chain(banner.as_mut()) {
                crop.fill(&self.image, (0, 0));
            }
            if self.quarter {
                quarter(&self.image, &mut quarter_image);
            }
        }
        let surfaces = match &mut self.screens {
            Some(reader) => {
                let signals = reader.gate(&quarter_image, &self.map)?;
                let mut surfaces: [SurfaceFrame; 4] = Default::default();
                for ((surface, signal), region) in
                    surfaces.iter_mut().zip(&signals).zip(Region::SCREENS)
                {
                    surface.candidate = signal.candidate;
                    surface.score = signal.score;
                    if !signal.candidate {
                        continue;
                    }
                    // A frame at the reference scale converts only the
                    // surface's zone (the same bytes as in the whole frame).
                    let view = if whole {
                        FrameView::native(detector_view(&self.image)?)
                    } else {
                        let zone = screens::zone(region, self.map.reference_map());
                        match zone {
                            Some(zone) => match self.reference.convert_part(&frame, zone)? {
                                Some(origin) => {
                                    FrameView::crop(detector_view(&self.reference.part)?, origin)
                                }
                                None => continue,
                            },
                            None => continue,
                        }
                    };
                    surface.slots = reader.slots(region, &view, &self.map, signal);
                }
                Some(surfaces)
            }
            None => None,
        };
        let native = if self.ocr.is_some() && !self.map.is_identity() {
            Some([
                self.native_crop(&frame, FEED_ROI, Anchor::Right)?,
                self.native_crop(&frame, LEFT_PANEL_ROI, Anchor::Left)?,
            ])
        } else {
            None
        };
        let screens = self.ocr_crops(&frame, surfaces.as_ref())?;
        Ok(FrameCrops {
            index: frame.index(),
            frame,
            feed,
            banner,
            left,
            quarter: quarter_image,
            surfaces,
            native,
            screens,
        })
    }

    /// What pass 0 reads of `frame` beyond the HUD zones: the name zones
    /// of the screen surfaces whose gate made a candidate (`surfaces`).
    fn ocr_crops(
        &mut self,
        frame: &VideoFrame,
        surfaces: Option<&[SurfaceFrame; 4]>,
    ) -> Result<[Option<Crop>; 4], AppError> {
        let mut screens: [Option<Crop>; 4] = Default::default();
        let Some(reads) = self.ocr else {
            return Ok(screens);
        };
        if let Some(surfaces) = surfaces {
            for ((slot, zone), surface) in screens.iter_mut().zip(reads.screens).zip(surfaces) {
                if let (Some(zone), true) = (zone, surface.candidate) {
                    *slot = Some(self.source_crop(frame, zone)?);
                }
            }
        }
        Ok(screens)
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
        self.source_crop(frame, source)
    }

    /// The source pixels of `source` (source coordinates).
    fn source_crop(&mut self, frame: &VideoFrame, source: RectI) -> Result<Crop, AppError> {
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
    /// A converted part of a frame at the reference scale.
    part: RgbImage,
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
            part: RgbImage::default(),
            scaling: None,
        }
    }

    /// Converts the part of `crop` that lies inside `frame`, a frame at the
    /// reference scale, into it.
    fn convert_into(&mut self, frame: &VideoFrame, crop: &mut Crop) -> Result<(), AppError> {
        let Some(rect) = crop.rect() else {
            return Ok(());
        };
        if let Some(origin) = self.convert_part(frame, rect)? {
            crop.fill(&self.part, origin);
        }
        Ok(())
    }

    /// Converts the part of `rect` that lies inside `frame`, a frame at the
    /// reference scale, into `self.part`; returns where `self.part` starts
    /// in the frame, or `None` when `rect` is outside it. Every pixel
    /// converts on its own with the chroma of its 2 × 2 block (the part is
    /// aligned to even coordinates), so the bytes are those of the
    /// converted whole frame.
    fn convert_part(
        &mut self,
        frame: &VideoFrame,
        rect: RectI,
    ) -> Result<Option<(i32, i32)>, AppError> {
        let bounds = RectI {
            x0: 0,
            y0: 0,
            x1: frame.width() as i32,
            y1: frame.height() as i32,
        };
        let Some(rect) = rect.intersection(&bounds) else {
            return Ok(None);
        };
        frame.roi_yuv(rect, &mut self.roi)?;
        self.converter.convert(&self.roi, &mut self.part)?;
        Ok(Some(
            self.roi.rect().map_or((rect.x0, rect.y0), |r| (r.x0, r.y0)),
        ))
    }

    /// The 4 × 4 means of `frame`, a frame at the reference scale, into
    /// `out` (the same bytes as [`quarter`] of the converted frame), when
    /// the converter makes them without converting the frame (8-bit SDR);
    /// otherwise `false`.
    fn means(&mut self, frame: &VideoFrame, out: &mut RgbImage) -> Result<bool, AppError> {
        let whole = RectI {
            x0: 0,
            y0: 0,
            x1: frame.width() as i32,
            y1: frame.height() as i32,
        };
        frame.roi_yuv(whole, &mut self.roi)?;
        Ok(self.converter.convert_means_4x4(&self.roi, out)?)
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

/// A detector view of `image`.
fn detector_view(image: &RgbImage) -> Result<RgbView<'_>, AppError> {
    rgb_view(image).map_err(|e| internal(format!("detector: {e}")))
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

    /// The gate signals of the screen surfaces of a reference frame whose
    /// 4 × 4 means are `quarter`.
    fn gate(&mut self, quarter: &RgbImage, map: &HudMap) -> Result<ScreenSignals, AppError> {
        Ok(self.detector.gate(
            &FrameView::scaled(detector_view(quarter)?, 1.0 / QUARTER as f32),
            map.reference_map(),
        ))
    }

    /// The name slots of candidate surface `region` in `pixels` (the
    /// reference frame, or the surface's zone of it).
    fn slots(
        &mut self,
        region: Region,
        pixels: &FrameView<'_>,
        map: &HudMap,
        signal: &GateSignal,
    ) -> Option<Vec<Slot>> {
        self.detector
            .read(region, pixels, map.reference_map(), signal)
    }
}
