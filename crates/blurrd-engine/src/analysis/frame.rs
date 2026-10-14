//! The crops of one frame that the detector stages read.

use blurrd_core::{RectI, Region, RegionSet};
use blurrd_detect::chat::LEFT_PANEL_ROI;
use blurrd_detect::killfeed::FEED_ROI;
use blurrd_detect::screens::Anchor;
use blurrd_detect::{ImageError, RgbView};
use blurrd_video::{RgbConverter, RgbImage, VideoError, VideoFrame, YuvRoi};

use crate::hud::{HudMap, RefCrop};

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
    /// View pixels per source pixel of `quarter`.
    pub(crate) quarter_scale: f32,
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
        }
    }

    /// The crops of `frame`.
    pub(crate) fn crops(&mut self, frame: VideoFrame) -> Result<FrameCrops, VideoError> {
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
        Ok(FrameCrops {
            index: frame.index(),
            frame,
            feed,
            banner,
            left,
            quarter,
            quarter_scale: scale,
        })
    }
}
