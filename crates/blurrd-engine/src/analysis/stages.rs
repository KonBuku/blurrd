//! The detector stages of the analysis: each runs the per-frame detectors
//! and temporal models of some regions over the frames in order and
//! returns the boxes of every frame that became final.

use blurrd_core::{AppError, Confidence, Mode, RectI, Region, RegionSet};
use blurrd_detect::banner::{self, BannerBox};
use blurrd_detect::killfeed::BarHues;
use blurrd_detect::scene_cut::SceneCutDetector;
use blurrd_detect::screens::{self, Anchor};
use blurrd_detect::temporal::killfeed::{FrameBoxes, KillFeedFrame};
use blurrd_detect::voice::{MEASURE_PORTRAIT, VoiceFrame, VoiceFrameBoxes};
use blurrd_detect::{
    BannerDetector, BannerTracker, ChatDetector, ChatTracker, ClassParams, Detection,
    KillFeedDetector, KillFeedTracker, ScreenDetector, VoiceDetector, VoiceTracker,
};

use super::frame::FrameCrops;
use super::tracks::{Grid, Link, RegionBox};
use crate::hud::HudMap;
use crate::job::internal;

/// The temporal models whose boxes the analysis merges, in the order a
/// frame's boxes are listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Source {
    /// Kill feed.
    KillFeed = 0,
    /// Banner.
    Banner = 1,
    /// Voice chat.
    Voice = 2,
    /// Text chat.
    Chat = 3,
    /// The four screen surfaces.
    Screens = 4,
}

/// Number of [`Source`]s.
pub(crate) const SOURCES: usize = 5;

/// Boxes that became final in a stage.
#[derive(Debug, Default)]
pub(crate) struct Emitted {
    /// Per source: frames whose boxes became final, in order, with their
    /// boxes. Frames a source made final without listing them have no
    /// boxes of that source.
    pub(crate) frames: [Vec<(u32, Vec<RegionBox>)>; SOURCES],
    /// Per source: every frame before this one is final.
    pub(crate) ready: [u32; SOURCES],
    /// The largest number of frames between the newest frame pushed and a
    /// frame that became final (the temporal models' look-ahead).
    pub(crate) delay: u32,
}

impl Emitted {
    /// Notes that `source` made every frame up to `frame` final while
    /// `newest` was the newest frame pushed.
    fn note(&mut self, source: Source, frame: u32, newest: Option<u32>) {
        if let Some(newest) = newest {
            self.delay = self.delay.max(newest.saturating_sub(frame));
        }
        let ready = &mut self.ready[source as usize];
        *ready = (*ready).max(frame + 1);
    }

    /// Adds a final frame of `source` with its boxes.
    fn frame(&mut self, source: Source, frame: u32, newest: Option<u32>, boxes: Vec<RegionBox>) {
        self.note(source, frame, newest);
        self.frames[source as usize].push((frame, boxes));
    }
}

/// A detector stage: pushed every frame in order, then finished once.
pub(crate) trait Stage: Send {
    /// Runs the stage on one frame; boxes that became final go to `out`.
    /// Returns whether the frame showed HUD structure (plan 7.2: kill-feed
    /// rows, a banner gate, voice portraits, the chat box, a screen
    /// surface gate).
    fn push(&mut self, crops: &FrameCrops, out: &mut Emitted) -> Result<bool, AppError>;
    /// Makes every pushed frame final.
    fn finish(self: Box<Self>, out: &mut Emitted);
    /// Whether the stage reads the HUD (its structure signals drive pass 0).
    fn is_hud(&self) -> bool {
        true
    }
}

/// The stages for `regions`: the kill feed alone (the costliest detector),
/// banner plus voice plus chat, and the screen surfaces, so they can run on
/// three threads. The HUD trackers draw their boxes in `mode`; the kill
/// feed reads the video's learned bar colours `hues` first.
pub(crate) fn stages(
    regions: RegionSet,
    map: HudMap,
    mode: Mode,
    hues: Option<&BarHues>,
) -> Vec<Box<dyn Stage>> {
    let mut out: Vec<Box<dyn Stage>> = Vec::new();
    if regions.contains(Region::KillFeed) {
        out.push(Box::new(KillFeedStage::new(map, mode, hues)));
    }
    let panel = [Region::Banner, Region::VoiceChat, Region::TextChat];
    if panel.iter().any(|&r| regions.contains(r)) {
        out.push(Box::new(PanelStage::new(regions, map, mode)));
    }
    if Region::SCREENS.iter().any(|&r| regions.contains(r)) {
        out.push(Box::new(ScreenStage::new(regions, map)));
    }
    out
}

/// Which sources the stages for `regions` feed.
pub(crate) fn sources(regions: RegionSet) -> [bool; SOURCES] {
    [
        regions.contains(Region::KillFeed),
        regions.contains(Region::Banner),
        regions.contains(Region::VoiceChat),
        regions.contains(Region::TextChat),
        Region::SCREENS.iter().any(|&r| regions.contains(r)),
    ]
}

/// A detector error: the crops are made for the detectors, so it is a bug.
fn detector_error(error: &dyn std::fmt::Display) -> AppError {
    internal(format!("detector: {error}"))
}

/// The kill feed: per-frame rows and names, then the tracker.
struct KillFeedStage {
    map: HudMap,
    detector: KillFeedDetector,
    tracker: KillFeedTracker,
    detections: Vec<Detection>,
    ready: Vec<FrameBoxes>,
}

impl KillFeedStage {
    fn new(map: HudMap, mode: Mode, hues: Option<&BarHues>) -> Self {
        let detector = match hues {
            Some(hues) => KillFeedDetector::with_params(hues.params(&ClassParams::default())),
            None => KillFeedDetector::new(),
        };
        let mut tracker = KillFeedTracker::new();
        tracker.set_mode(mode);
        Self {
            map,
            detector,
            tracker,
            detections: Vec::new(),
            ready: Vec::new(),
        }
    }

    fn emit(&mut self, newest: Option<u32>, out: &mut Emitted) {
        let screen = self.map.screen_map();
        let (sx, sy) = (screen.sx(), screen.sy());
        for frame in self.ready.drain(..) {
            let boxes = frame
                .boxes
                .iter()
                .filter_map(|b| {
                    Some(RegionBox {
                        region: Region::KillFeed,
                        kind: b.kind,
                        rect: self.map.source_rect(b.rect, Anchor::Right)?,
                        conf: b.confidence,
                        link: Link::Key(b.part),
                        grid: Grid::Moved(
                            (f32::from(b.offset.0) * sx).round() as i32,
                            (f32::from(b.offset.1) * sy).round() as i32,
                        ),
                    })
                })
                .collect();
            out.frame(Source::KillFeed, frame.frame, newest, boxes);
        }
    }
}

impl Stage for KillFeedStage {
    fn push(&mut self, crops: &FrameCrops, out: &mut Emitted) -> Result<bool, AppError> {
        let view = crops.feed.view().map_err(|e| detector_error(&e))?;
        self.detector
            .detect(&view, &mut self.detections)
            .map_err(|e| detector_error(&e))?;
        let structure = !self.detector.rows().is_empty();
        self.tracker
            .push(&KillFeedFrame {
                frame: crops.index,
                crop: view,
                detections: &self.detections,
            })
            .map_err(|e| detector_error(&e))?;
        self.tracker.drain_ready(&mut self.ready);
        self.emit(Some(crops.index), out);
        Ok(structure)
    }

    fn finish(mut self: Box<Self>, out: &mut Emitted) {
        self.tracker.finish(&mut self.ready);
        self.emit(None, out);
    }
}

/// Banner, voice chat and text chat (the cheap HUD detectors).
struct PanelStage {
    map: HudMap,
    /// Banner, voice, chat enabled.
    enabled: [bool; 3],
    banner: BannerDetector,
    banner_tracker: BannerTracker,
    banner_ready: Vec<BannerBox>,
    voice: VoiceDetector,
    voice_tracker: VoiceTracker,
    voice_ready: Vec<VoiceFrameBoxes>,
    cuts: SceneCutDetector,
    chat: ChatDetector,
    chat_tracker: ChatTracker,
    chat_ready: Vec<(u32, Vec<Detection>)>,
    detections: Vec<Detection>,
}

impl PanelStage {
    fn new(regions: RegionSet, map: HudMap, mode: Mode) -> Self {
        let mut banner_tracker = BannerTracker::new();
        banner_tracker.set_mode(mode);
        let mut voice_tracker = VoiceTracker::new();
        voice_tracker.set_mode(mode);
        let mut chat_tracker = ChatTracker::new();
        chat_tracker.set_mode(mode);
        Self {
            map,
            enabled: [
                regions.contains(Region::Banner),
                regions.contains(Region::VoiceChat),
                regions.contains(Region::TextChat),
            ],
            banner: BannerDetector::new(),
            banner_tracker,
            banner_ready: Vec::new(),
            voice: VoiceDetector::new(),
            voice_tracker,
            voice_ready: Vec::new(),
            cuts: SceneCutDetector::default(),
            chat: ChatDetector::new(),
            chat_tracker,
            chat_ready: Vec::new(),
            detections: Vec::new(),
        }
    }

    /// A box at `rect` (reference pixels, kept at `anchor`) in source
    /// pixels, `None` outside the frame.
    fn place(&self, rect: RectI, anchor: Anchor) -> Option<RectI> {
        self.map.source_rect(rect, anchor)
    }

    /// Moves the final boxes to `out`.
    fn emit(&mut self, newest: Option<u32>, out: &mut Emitted) {
        if self.enabled[0] {
            let mut current: Option<(u32, Vec<RegionBox>)> = None;
            for b in std::mem::take(&mut self.banner_ready) {
                if current.as_ref().is_some_and(|(f, _)| *f != b.frame)
                    && let Some((f, boxes)) = current.take()
                {
                    out.frame(Source::Banner, f, newest, boxes);
                }
                let entry = current.get_or_insert_with(|| (b.frame, Vec::new()));
                entry.1.extend(
                    self.place(b.rect, banner::ANCHOR.into())
                        .map(|rect| RegionBox {
                            region: Region::Banner,
                            kind: blurrd_core::NameKind::Banner,
                            rect,
                            conf: b.conf,
                            link: Link::Adjacent,
                            grid: Grid::Corner,
                        }),
                );
            }
            if let Some((f, boxes)) = current {
                out.frame(Source::Banner, f, newest, boxes);
            }
            if let (Some(ready), Some(n)) =
                (self.banner_tracker.ready_until().checked_sub(1), newest)
            {
                out.note(Source::Banner, ready, Some(n));
            }
        }
        for frame in std::mem::take(&mut self.voice_ready) {
            let boxes = frame
                .boxes
                .iter()
                .filter_map(|b| {
                    let (gx, gy) = self.map.point_to_source(b.text.x0, b.text.y0, Anchor::Left);
                    Some(RegionBox {
                        region: Region::VoiceChat,
                        kind: blurrd_core::NameKind::Speaker,
                        rect: self.place(b.rect, Anchor::Left)?,
                        conf: b.confidence,
                        link: Link::Key(b.track),
                        grid: Grid::At(gx, gy),
                    })
                })
                .collect();
            out.frame(Source::Voice, frame.frame, newest, boxes);
        }
        for (frame, detections) in std::mem::take(&mut self.chat_ready) {
            let boxes = detections
                .iter()
                .filter_map(|d| {
                    Some(RegionBox {
                        region: d.region,
                        kind: d.kind,
                        rect: self.place(d.rect, Anchor::Left)?,
                        conf: Confidence::Tracked,
                        link: Link::Overlap,
                        grid: Grid::Corner,
                    })
                })
                .collect();
            out.frame(Source::Chat, frame, newest, boxes);
        }
    }
}

impl Stage for PanelStage {
    fn push(&mut self, crops: &FrameCrops, out: &mut Emitted) -> Result<bool, AppError> {
        let frame = crops.index;
        let mut structure = false;
        if self.enabled[0] {
            let view = crops.banner().view().map_err(|e| detector_error(&e))?;
            let seen = self.banner.detect(&view).map_err(|e| detector_error(&e))?;
            structure |= seen.gate;
            self.banner_tracker
                .push(frame, &seen, &mut self.banner_ready);
        }
        let left = crops.left.view().map_err(|e| detector_error(&e))?;
        if self.enabled[1] {
            self.voice
                .detect(&left, &mut self.detections)
                .map_err(|e| detector_error(&e))?;
            structure |= self
                .voice
                .slots()
                .iter()
                .any(|s| s.portrait >= MEASURE_PORTRAIT);
            let quarter = crops.quarter_view().map_err(|e| detector_error(&e))?;
            let cut = self.cuts.push(&quarter, &[(Region::VoiceChat, left)]);
            self.voice_tracker
                .push(&VoiceFrame {
                    frame,
                    slots: self.voice.slots(),
                    cut: frame > 0 && cut.is_cut_for(Region::VoiceChat),
                })
                .map_err(|e| detector_error(&e))?;
            self.voice_tracker.drain_ready(&mut self.voice_ready);
        }
        if self.enabled[2] {
            self.chat
                .detect(&left, &mut self.detections)
                .map_err(|e| detector_error(&e))?;
            structure |= !self.chat.bands().is_empty();
            self.chat_tracker.push(frame, &self.chat);
            self.chat_tracker.drain_ready(&mut self.chat_ready);
        }
        self.emit(Some(frame), out);
        Ok(structure)
    }

    fn finish(mut self: Box<Self>, out: &mut Emitted) {
        self.banner_tracker.finish(&mut self.banner_ready);
        self.voice_tracker.finish(&mut self.voice_ready);
        self.chat_tracker.finish(&mut self.chat_ready);
        self.emit(None, out);
    }
}

/// The screen surfaces' temporal stage; the crop workers already gated
/// every frame and read the native zone of a candidate surface.
struct ScreenStage {
    map: HudMap,
    detector: ScreenDetector,
    ready: Vec<(u32, Vec<Detection>)>,
}

impl ScreenStage {
    fn new(regions: RegionSet, map: HudMap) -> Self {
        Self {
            map,
            detector: ScreenDetector::new(&regions),
            ready: Vec::new(),
        }
    }
}

/// Moves the final screen-surface boxes of `ready` to `out`.
fn emit_screens(
    map: &HudMap,
    ready: &mut Vec<(u32, Vec<Detection>)>,
    newest: Option<u32>,
    out: &mut Emitted,
) {
    for (frame, detections) in ready.drain(..) {
        let boxes = detections
            .iter()
            .filter_map(|d| {
                Some(RegionBox {
                    region: d.region,
                    kind: d.kind,
                    rect: map.source_rect(d.rect, screens::anchor(d.region)?)?,
                    conf: Confidence::Tracked,
                    link: Link::Overlap,
                    grid: Grid::Corner,
                })
            })
            .collect();
        out.frame(Source::Screens, frame, newest, boxes);
    }
}

impl Stage for ScreenStage {
    fn push(&mut self, crops: &FrameCrops, out: &mut Emitted) -> Result<bool, AppError> {
        // The crop workers gated and read the frame (see `Cropper`).
        let surfaces = crops
            .surfaces
            .clone()
            .ok_or_else(|| internal("the screen surfaces of a frame were not read"))?;
        let structure = surfaces.iter().any(|s| s.candidate);
        self.detector.push(crops.index, surfaces, &mut self.ready);
        emit_screens(&self.map, &mut self.ready, Some(crops.index), out);
        Ok(structure)
    }

    fn is_hud(&self) -> bool {
        false
    }

    fn finish(self: Box<Self>, out: &mut Emitted) {
        let Self {
            map,
            detector,
            mut ready,
            ..
        } = *self;
        detector.finish(&mut ready);
        emit_screens(&map, &mut ready, None, out);
    }
}
