//! The crops of one frame that the detector stages read, and its screen
//! surfaces read by the crop workers.

use blurrd_core::{AppError, RectI, Region, RegionSet};
use blurrd_detect::chat::LEFT_PANEL_ROI;
use blurrd_detect::killfeed::FEED_ROI;
use blurrd_detect::screens::{self, Anchor, FrameView, SurfaceFrame};
use blurrd_detect::{ImageError, RgbView, ScreenDetector};
use blurrd_video::{RgbConverter, RgbImage, VideoFrame, YuvRoi};

use crate::hud::{HudMap, RefCrop};
use crate::job::internal;

/// One frame and its crops at the reference scale.
#[derive(Debug)]
pub(crate) struct FrameCrops {
    /// Presentation index.
    pub(crate) index: u32,
    /// The decoded frame (a reference, no copy): the screen surfaces read
    /// their native zones from it.
    pub(crate) frame: VideoFrame,
    /// The kill feed and banner area ([`FEED_ROI`]), right-anchored.
    pub(crate) feed: RefCrop,
    /// The same area centre-anchored for the banner, when that maps
    /// differently (HUD layouts that are not 16:9).
    banner: Option<RefCrop>,
    /// The left panel ([`LEFT_PANEL_ROI`]: voice chat and text chat).
    pub(crate) left: RefCrop,
    /// The whole frame downscaled ([`HudMap::quarter`]).
    quarter: RgbImage,
    /// The screen surfaces' gate signals and name slots of this frame,
    /// when a screen surface is analysed.
    pub(crate) surfaces: Option<[SurfaceFrame; 4]>,
}

impl FrameCrops {
    /// The banner's crop.
    pub(crate) fn banner(&self) -> &RefCrop {
        self.banner.as_ref().unwrap_or(&self.feed)
    }

    /// The downscaled whole frame.
    pub(crate) fn quarter_view(&self) -> Result<RgbView<'_>, ImageError> {
        RgbView::new(
            &self.quarter.data,
            self.quarter.width,
            self.quarter.height,
            3 * self.quarter.width as usize,
        )
    }
}

/// Makes the crops of frames; one per crop worker (it keeps buffers).
#[derive(Debug, Clone)]
pub(crate) struct Cropper {
    map: HudMap,
    converter: RgbConverter,
    /// Whether the banner needs its own crop.
    separate_banner: bool,
    /// Whether any HUD region is analysed (otherwise no HUD crop is made).
    hud: bool,
    /// Whether the downscaled whole frame is read (voice-chat scene cuts,
    /// screen-surface gates).
    quarter: bool,
    roi: YuvRoi,
    /// Reads the screen surfaces, when one is analysed.
    screens: Option<ScreenReader>,
}

impl Cropper {
    /// A cropper for `regions` of frames mapped by `map`.
    pub(crate) fn new(map: HudMap, converter: RgbConverter, regions: RegionSet) -> Self {
        let separate_banner = regions.contains(Region::Banner)
            && map.source_rect(FEED_ROI, Anchor::Right)
                != map.source_rect(FEED_ROI, Anchor::Center);
        Self {
            map,
            converter,
            separate_banner,
            hud: Region::HUD.iter().any(|&r| regions.contains(r)),
            quarter: regions.contains(Region::VoiceChat)
                || Region::SCREENS.iter().any(|&r| regions.contains(r)),
            roi: YuvRoi::new(),
            screens: Region::SCREENS
                .iter()
                .any(|&r| regions.contains(r))
                .then(|| ScreenReader::new(regions)),
        }
    }

    /// The crops of `frame`, and its screen surfaces read.
    pub(crate) fn crops(&mut self, frame: VideoFrame) -> Result<FrameCrops, AppError> {
        let (scale, width, height) = self.map.quarter();
        let mut quarter = RgbImage::default();
        if self.quarter {
            frame.thumbnail(width, height, &self.converter, &mut quarter)?;
        }
        let crop = |this: &mut Self, rect: RectI, anchor: Anchor| {
            if this.hud {
                RefCrop::extract(
                    &frame,
                    &this.map,
                    rect,
                    anchor,
                    &this.converter,
                    &mut this.roi,
                )
            } else {
                Ok(RefCrop::default())
            }
        };
        let feed = crop(self, FEED_ROI, Anchor::Right)?;
        let banner = if self.separate_banner {
            Some(crop(self, FEED_ROI, Anchor::Center)?)
        } else {
            None
        };
        let left = crop(self, LEFT_PANEL_ROI, Anchor::Left)?;
        let surfaces = match &mut self.screens {
            Some(reader) => {
                Some(reader.read(&frame, &quarter, scale, &self.map, &self.converter)?)
            }
            None => None,
        };
        Ok(FrameCrops {
            index: frame.index(),
            frame,
            feed,
            banner,
            left,
            quarter,
            surfaces,
        })
    }
}

/// Reads the screen surfaces of single frames: the quarter-scale gates
/// and, for a candidate surface, the name slots in its native zone. Both
/// depend on the frame alone (the readers keep only scratch buffers), so
/// the crop workers read in parallel and the screen stage's temporal
/// model takes the results in frame order.
#[derive(Debug, Clone)]
struct ScreenReader {
    detector: ScreenDetector,
    roi: YuvRoi,
    zone: RgbImage,
}

impl ScreenReader {
    fn new(regions: RegionSet) -> Self {
        Self {
            detector: ScreenDetector::new(&regions),
            roi: YuvRoi::new(),
            zone: RgbImage::default(),
        }
    }

    /// The surfaces of `frame`, whose downscaled copy is `quarter` (at
    /// `scale` view pixels per source pixel).
    fn read(
        &mut self,
        frame: &VideoFrame,
        quarter: &RgbImage,
        scale: f32,
        map: &HudMap,
        converter: &RgbConverter,
    ) -> Result<[SurfaceFrame; 4], AppError> {
        let detector_error = |e: ImageError| internal(format!("detector: {e}"));
        let screen_map = *map.screen_map();
        let quarter = RgbView::new(
            &quarter.data,
            quarter.width,
            quarter.height,
            3 * quarter.width as usize,
        )
        .map_err(detector_error)?;
        let signals = self
            .detector
            .gate(&FrameView::scaled(quarter, scale), &screen_map);
        let mut surfaces: [SurfaceFrame; 4] = Default::default();
        for (i, region) in Region::SCREENS.into_iter().enumerate() {
            let signal = signals[i];
            surfaces[i].candidate = signal.candidate;
            surfaces[i].score = signal.score;
            let zone =
                screens::zone(region, &screen_map).and_then(|z| z.intersection(&map.frame()));
            let (true, Some(zone)) = (signal.candidate, zone) else {
                continue;
            };
            frame.roi_yuv(zone, &mut self.roi)?;
            converter.convert(&self.roi, &mut self.zone)?;
            let origin = self.roi.rect().map_or((zone.x0, zone.y0), |r| (r.x0, r.y0));
            let pixels = RgbView::new(
                &self.zone.data,
                self.zone.width,
                self.zone.height,
                3 * self.zone.width as usize,
            )
            .map_err(detector_error)?;
            surfaces[i].slots = self.detector.read(
                region,
                &FrameView::crop(pixels, origin),
                &screen_map,
                &signal,
            );
        }
        Ok(surfaces)
    }
}
