//! Synthetic recordings for the engine's integration tests: testkit
//! scenes encoded losslessly (FFV1 in Matroska, BT.709 limited range, as
//! HD recordings are tagged), so a test knows exactly which names every
//! decoded frame shows. No footage is ever committed.

#![allow(
    dead_code,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::cast_possible_wrap,
    reason = "test helpers shared through `mod synthetic`: each test binary uses \
              a different subset, a panic is a test failure, and frame sizes \
              never wrap"
)]

use std::path::{Path, PathBuf};

use blurrd_blur::{PixelParams, PreparedBox, YuvColour, prepare_boxes};
use blurrd_core::{BlurParams, FrameRange, Project, RegionSet};
use blurrd_testkit::hud::banner::BannerSpec;
use blurrd_testkit::hud::chat::{Channel, ChatMessage, ChatSpec, Language};
use blurrd_testkit::hud::killfeed::{Kill, KillFeedSpec, Team, Weapon};
use blurrd_testkit::hud::voice::{LOCATIONS, Speaker, VoiceSpec};
use blurrd_testkit::metrics::{Evaluation, Summary};
use blurrd_testkit::{Background, FontSet, Layout, NameStyle, Palette, Rng, Scene, synth_name};
use ffmpeg_the_third::util::color;
use ffmpeg_the_third::{Rational as FfRational, codec, encoder, format, frame};

/// A folder for one test, removed when dropped.
pub struct Workdir(PathBuf);

impl Workdir {
    /// An empty folder named after the test.
    pub fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("blurrd-engine-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    /// A path inside the folder.
    pub fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    /// The folder.
    pub fn dir(&self) -> &Path {
        &self.0
    }
}

impl Drop for Workdir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A HUD scene of `frames` frames in `layout`: five kill-feed rows (one
/// every 30 frames), an ACE banner, two voice speakers and three chat
/// lines, over a noise background.
pub fn hud_scene(layout: Layout, frames: u32, seed: u64) -> Scene {
    let fonts = FontSet::latin();
    let mut rng = Rng::new(seed);
    let mut name = || synth_name(&mut rng, NameStyle::LATIN_MIX[seed as usize % 3]);
    let kills: Vec<Kill> = (0..5)
        .map(|i| Kill {
            killer: name(),
            victim: name(),
            killer_team: Team::Ally,
            victim_team: Team::Enemy,
            weapon: Weapon::Rifle,
            badge: None,
            own: false,
            headshot: i % 2 == 0,
            birth: 10 + 30 * i,
        })
        .collect();
    let speakers: Vec<Speaker> = (0..2u8)
        .map(|slot| Speaker {
            name: name(),
            location: LOCATIONS[slot as usize].to_owned(),
            slot,
            shown: vec![FrameRange {
                f0: 20 + 40 * u32::from(slot),
                f1: 150 + 40 * u32::from(slot),
            }],
        })
        .collect();
    let messages: Vec<ChatMessage> = [Channel::All, Channel::Team, Channel::Party]
        .into_iter()
        .enumerate()
        .map(|(i, channel)| ChatMessage {
            channel,
            name: name(),
            team: if i % 2 == 0 { Team::Enemy } else { Team::Ally },
            text: "rotate mid now".to_owned(),
            role: None,
            wrap: false,
            time: 30 + 25 * i as u32,
        })
        .collect();
    let banner = name();
    Scene::new(layout, Background::noise(seed), Palette::DEFAULT, frames)
        .with_kill_feed(&fonts, KillFeedSpec::new(kills))
        .with_banner(&fonts, BannerSpec::new("ACE", &banner, 60))
        .with_voice(&fonts, VoiceSpec { speakers })
        .with_chat(&fonts, ChatSpec::new(Language::English, messages))
}

/// A scene of `frames` frames in `layout` without any HUD element: only
/// the noise background.
pub fn empty_scene(layout: Layout, frames: u32, seed: u64) -> Scene {
    Scene::new(layout, Background::noise(seed), Palette::DEFAULT, frames)
}

/// Encodes every frame of `scene` into `path` (FFV1 in Matroska, 60 fps,
/// BT.709 limited range).
pub fn write_clip(path: &Path, scene: &Scene) {
    ffmpeg_the_third::init().unwrap();
    let first = scene.render(0).frame;
    let (width, height) = (first.width(), first.height());
    let mut out = format::output(path).unwrap();
    let codec = encoder::find_by_name("ffv1").unwrap();
    let mut ctx = codec::Context::new_with_codec(codec)
        .encoder()
        .video()
        .unwrap();
    ctx.set_width(width);
    ctx.set_height(height);
    ctx.set_format(format::Pixel::YUV420P);
    ctx.set_time_base(FfRational::new(1, 60));
    ctx.set_colorspace(color::Space::BT709);
    ctx.set_color_range(color::Range::MPEG);
    ctx.set_flags(codec::Flags::GLOBAL_HEADER);
    let mut enc = ctx.open().unwrap();
    let mut stream = out.add_stream(codec).unwrap();
    stream.set_parameters(codec::Parameters::from(&enc));
    stream.set_time_base(FfRational::new(1, 60));
    out.write_header().unwrap();
    let stream_tb = out.stream(0).unwrap().time_base();
    let mut picture = frame::Video::new(format::Pixel::YUV420P, width, height);
    let mut packet = codec::packet::Packet::empty();
    let mut write_packets = |enc: &mut encoder::video::Encoder,
                             out: &mut format::context::Output| {
        while enc.receive_packet(&mut packet).is_ok() {
            packet.set_stream(0);
            packet.rescale_ts(FfRational::new(1, 60), stream_tb);
            packet.write_interleaved(out).unwrap();
        }
    };
    for fixture in scene.iter() {
        let i420 = fixture.frame.to_i420();
        for (plane, data) in i420.planes().into_iter().enumerate() {
            let w = if plane == 0 {
                width as usize
            } else {
                width.div_ceil(2) as usize
            };
            let stride = picture.stride(plane);
            for (dst, src) in picture
                .data_mut(plane)
                .chunks_mut(stride)
                .zip(data.chunks(w))
            {
                dst[..w].copy_from_slice(src);
            }
        }
        picture.set_pts(Some(i64::from(fixture.truth.frame)));
        enc.send_frame(&picture).unwrap();
        write_packets(&mut enc, &mut out);
    }
    enc.send_eof().unwrap();
    write_packets(&mut enc, &mut out);
    out.write_trailer().unwrap();
}

/// Scores the boxes of `project` on every frame of `scene` against its
/// ground truth (plan 11.3: geometric leak, over-blur, portraits, false
/// positives, jitter, flicker), with the project's blur parameters.
pub fn evaluate(project: &Project, scene: &Scene, regions: RegionSet) -> Summary {
    let params = PixelParams::new(
        &BlurParams::default(),
        project.geometry.sy,
        YuvColour::default(),
    )
    .unwrap();
    let table = project.plan().frame_table(&regions);
    let mut evaluation = Evaluation::new();
    let mut prepared: Vec<PreparedBox> = Vec::new();
    for fixture in scene.iter() {
        let boxes = table.boxes(fixture.truth.frame);
        prepare_boxes(
            boxes,
            fixture.frame.width(),
            fixture.frame.height(),
            &params,
            &mut prepared,
        );
        evaluation.add_frame(&fixture.truth, &prepared);
    }
    evaluation.summary().clone()
}

/// The boxes of `project` (in `regions`) whose rectangle overlaps a
/// portrait of `scene` by more than `tolerance` columns, as `frame: region
/// rect` lines. The padding and feather around a box are not counted: they
/// follow the blur settings, the box itself follows the detectors' portrait
/// clearance. Boxes mapped from the reference grid onto a smaller frame
/// are rounded outward, which may add one source column.
pub fn boxes_on_portraits(
    project: &Project,
    scene: &Scene,
    regions: RegionSet,
    tolerance: u32,
) -> Vec<String> {
    let table = project.plan().frame_table(&regions);
    let mut out = Vec::new();
    for fixture in scene.iter() {
        let truth = &fixture.truth;
        for b in table.boxes(truth.frame) {
            let overlaps = truth.portraits().any(|p| {
                p.intersection(&b.rect)
                    .is_some_and(|i| i.width() > tolerance)
            });
            if overlaps {
                out.push(format!("{}: {:?} {:?}", truth.frame, b.region, b.rect));
            }
        }
    }
    out
}

/// The SHA-256 of the video stream of `path`: every packet's timestamps,
/// keyframe flag and data, in file order. Two files with the same digest
/// hold the same encoded video, whatever their container headers say.
pub fn video_stream_digest(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    let mut input = format::input(path).unwrap();
    let index = input
        .streams()
        .best(ffmpeg_the_third::media::Type::Video)
        .unwrap()
        .index();
    let mut hash = Sha256::new();
    let mut packets = 0u32;
    for (stream, packet) in input.packets().map(Result::unwrap) {
        if stream.index() != index {
            continue;
        }
        packets += 1;
        hash.update(packet.pts().unwrap_or(i64::MIN).to_le_bytes());
        hash.update(packet.dts().unwrap_or(i64::MIN).to_le_bytes());
        hash.update([u8::from(packet.is_key())]);
        hash.update(packet.data().unwrap_or_default());
    }
    hash.update(packets.to_le_bytes());
    hash.finalize()
        .iter()
        .fold(String::with_capacity(64), |mut hex, b| {
            use std::fmt::Write as _;
            let _ = write!(hex, "{b:02x}");
            hex
        })
}

/// The number of frames of the video stream of `path` and how many of
/// them are keyframes (from the packets).
pub fn frames_and_keyframes(path: &Path) -> (u32, u32) {
    let mut input = format::input(path).unwrap();
    let index = input
        .streams()
        .best(ffmpeg_the_third::media::Type::Video)
        .unwrap()
        .index();
    let (mut frames, mut keys) = (0, 0);
    for (stream, packet) in input.packets().map(Result::unwrap) {
        if stream.index() == index {
            frames += 1;
            keys += u32::from(packet.is_key());
        }
    }
    (frames, keys)
}
