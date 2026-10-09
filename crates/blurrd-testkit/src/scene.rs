//! Scenes: a background and HUD elements over a number of frames.

use blurrd_core::RectI;

use crate::background::Background;
use crate::font::FontSet;
use crate::hud::banner::{Banner, BannerSpec, UltReady, UltReadySpec};
use crate::hud::chat::{Chat, ChatSpec};
use crate::hud::killfeed::{KillFeed, KillFeedSpec};
use crate::hud::match_end::{MatchEnd, MatchEndSpec};
use crate::hud::recap::{Recap, RecapSpec};
use crate::hud::scoreboard::{Scoreboard, ScoreboardSpec};
use crate::hud::spectating::{Spectating, SpectatingSpec};
use crate::hud::voice::{Voice, VoiceSpec};
use crate::hud::world_tag::{WorldTag, WorldTags};
use crate::image::RgbFrame;
use crate::layout::Layout;
use crate::paint::Painter;
use crate::palette::Palette;
use crate::truth::FrameTruth;

/// A flash of white light over part of the frame (a flashbang, a bright
/// ability), fading linearly over its frames.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Flash {
    /// First frame.
    pub start: u32,
    /// Number of frames.
    pub frames: u32,
    /// Strength in the first frame (`0.0..=1.0`, the share of white).
    pub amount: f32,
    /// The affected area in source pixels, or the whole frame.
    pub area: Option<RectI>,
}

/// One generated frame with its ground truth.
#[derive(Debug, Clone, PartialEq)]
pub struct Fixture {
    /// The frame.
    pub frame: RgbFrame,
    /// What is in it.
    pub truth: FrameTruth,
}

/// A synthetic video: background, HUD elements and effects in one layout.
///
/// Text is rendered when an element is added, so rendering a frame only
/// composites. Rendering is deterministic: the same scene and frame index
/// always give the same pixels.
#[derive(Debug, Clone)]
pub struct Scene {
    layout: Layout,
    background: Background,
    palette: Palette,
    frames: u32,
    kill_feed: Option<KillFeed>,
    banner: Option<Banner>,
    ult_ready: Option<UltReady>,
    voice: Option<Voice>,
    chat: Option<Chat>,
    scoreboards: Vec<Scoreboard>,
    recaps: Vec<Recap>,
    spectating: Vec<Spectating>,
    match_ends: Vec<MatchEnd>,
    world_tags: Option<WorldTags>,
    flashes: Vec<Flash>,
}

impl Scene {
    /// An empty scene of `frames` frames.
    pub fn new(layout: Layout, background: Background, palette: Palette, frames: u32) -> Self {
        Self {
            layout,
            background,
            palette,
            frames,
            kill_feed: None,
            banner: None,
            ult_ready: None,
            voice: None,
            chat: None,
            scoreboards: Vec::new(),
            recaps: Vec::new(),
            spectating: Vec::new(),
            match_ends: Vec::new(),
            world_tags: None,
            flashes: Vec::new(),
        }
    }

    /// The scene's layout.
    pub const fn layout(&self) -> &Layout {
        &self.layout
    }

    /// Number of frames.
    pub const fn frames(&self) -> u32 {
        self.frames
    }

    /// Adds a kill feed.
    #[must_use]
    pub fn with_kill_feed(mut self, fonts: &FontSet, spec: KillFeedSpec) -> Self {
        self.kill_feed = Some(KillFeed::new(fonts, &self.layout, spec));
        self
    }

    /// Adds a banner.
    #[must_use]
    pub fn with_banner(mut self, fonts: &FontSet, spec: BannerSpec) -> Self {
        self.banner = Some(Banner::new(fonts, &self.layout, spec));
        self
    }

    /// Adds an "X ULT READY" notification.
    #[must_use]
    pub fn with_ult_ready(mut self, fonts: &FontSet, spec: UltReadySpec) -> Self {
        self.ult_ready = Some(UltReady::new(fonts, &self.layout, spec));
        self
    }

    /// Adds voice-chat speakers.
    #[must_use]
    pub fn with_voice(mut self, fonts: &FontSet, spec: VoiceSpec) -> Self {
        self.voice = Some(Voice::new(fonts, &self.layout, spec));
        self
    }

    /// Adds a text chat.
    #[must_use]
    pub fn with_chat(mut self, fonts: &FontSet, spec: ChatSpec) -> Self {
        self.chat = Some(Chat::new(fonts, &self.layout, spec));
        self
    }

    /// Adds a scoreboard (several can be added for separate openings).
    #[must_use]
    pub fn with_scoreboard(mut self, fonts: &FontSet, spec: ScoreboardSpec) -> Self {
        self.scoreboards
            .push(Scoreboard::new(fonts, &self.layout, spec));
        self
    }

    /// Adds a death recap.
    #[must_use]
    pub fn with_recap(mut self, fonts: &FontSet, spec: RecapSpec) -> Self {
        self.recaps.push(Recap::new(fonts, &self.layout, spec));
        self
    }

    /// Adds a "Spectating" banner.
    #[must_use]
    pub fn with_spectating(mut self, fonts: &FontSet, spec: SpectatingSpec) -> Self {
        self.spectating
            .push(Spectating::new(fonts, &self.layout, spec));
        self
    }

    /// Adds a match-end, MVP or round-end screen.
    #[must_use]
    pub fn with_match_end(mut self, fonts: &FontSet, spec: MatchEndSpec) -> Self {
        self.match_ends
            .push(MatchEnd::new(fonts, &self.layout, spec));
        self
    }

    /// Adds name tags above players' heads (out of scope; decoys that must
    /// never be blurred).
    #[must_use]
    pub fn with_world_tags(mut self, fonts: &FontSet, tags: Vec<WorldTag>) -> Self {
        self.world_tags = Some(WorldTags::new(fonts, &self.layout, tags));
        self
    }

    /// Adds a flash.
    #[must_use]
    pub fn with_flash(mut self, flash: Flash) -> Self {
        self.flashes.push(flash);
        self
    }

    /// Renders frame `index`.
    pub fn render(&self, index: u32) -> Fixture {
        let mut frame = RgbFrame::filled(self.layout.width, self.layout.height, [0, 0, 0]);
        self.background.render(&mut frame, self.layout.image);
        let mut truth = FrameTruth {
            frame: index,
            scale: self.layout.sy,
            ..FrameTruth::default()
        };
        {
            let mut p = Painter {
                frame: &mut frame,
                layout: &self.layout,
                truth: &mut truth,
            };
            if let Some(tags) = &self.world_tags {
                tags.paint(&mut p, index);
            }
            if let Some(feed) = &self.kill_feed {
                feed.paint(&mut p, &self.palette, index);
            }
            if let Some(banner) = &self.banner {
                banner.paint(&mut p, index);
            }
            if let Some(ult_ready) = &self.ult_ready {
                ult_ready.paint(&mut p, index);
            }
            if let Some(voice) = &self.voice {
                voice.paint(&mut p, index);
            }
            if let Some(chat) = &self.chat {
                chat.paint(&mut p, &self.palette, index);
            }
            for screen in &self.spectating {
                screen.paint(&mut p, index);
            }
            for screen in &self.recaps {
                screen.paint(&mut p, &self.palette, index);
            }
            for screen in &self.scoreboards {
                screen.paint(&mut p, &self.palette, index);
            }
            for screen in &self.match_ends {
                screen.paint(&mut p, &self.palette, index);
            }
        }
        for flash in &self.flashes {
            if index >= flash.start && index - flash.start < flash.frames {
                let t = (index - flash.start) as f32 / flash.frames as f32;
                let area = flash.area.unwrap_or_else(|| frame.bounds());
                frame.wash_rect(area, flash.amount * (1.0 - t));
            }
        }
        Fixture { frame, truth }
    }

    /// Renders every frame in order.
    pub fn iter(&self) -> impl Iterator<Item = Fixture> + '_ {
        (0..self.frames).map(|i| self.render(i))
    }
}
