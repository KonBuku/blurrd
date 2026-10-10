//! The screen-surface detectors on synthetic videos: every name on a
//! screen surface is covered in every frame it is visible (fades and
//! slides included), in every layout, palette and GPU colour filter;
//! normal gameplay, round-end banners and world-space name tags never
//! get a box.

#![allow(
    clippy::unwrap_used,
    reason = "test helpers on synthetic frames: a panic is a failure"
)]

mod support;

use blurrd_blur::metrics::HF_RESIDUAL_MAX;
use blurrd_blur::{PixelParams, PreparedBox, YuvColour, prepare_boxes};
use blurrd_core::{BlurBox, BlurParams, FrameRange, Region, RegionSet};
use blurrd_detect::screens::temporal::{POST_ROLL, PRE_ROLL};
use blurrd_detect::screens::{self, FrameView, ScreenMap};
use blurrd_detect::{Detection, RgbView, ScreenDetector};
use blurrd_testkit::hud::banner::BannerSpec;
use blurrd_testkit::hud::killfeed::{KillFeedSpec, Team};
use blurrd_testkit::hud::match_end::{MatchEndKind, MatchEndSpec};
use blurrd_testkit::hud::recap::RecapSpec;
use blurrd_testkit::hud::scoreboard::{ScoreRow, ScoreboardMode, ScoreboardSpec};
use blurrd_testkit::hud::screen::ScreenTiming;
use blurrd_testkit::hud::world_tag::WorldTag;
use blurrd_testkit::metrics::{hf_residuals, item_coverage};
use blurrd_testkit::truth::ContentKind;
use blurrd_testkit::{
    Background, Fixture, FontSet, Layout, NameStyle, Palette, RgbFrame, Rng, Scene, Script,
    synth_name,
};

use support::{Filter, random_kill};

/// Instant opening (the measured scoreboard) and a fade with a slide.
const TIMINGS: [ScreenTiming; 2] = [
    ScreenTiming::instant(3, 6),
    ScreenTiming::animated(3, 4, 3, 4, 14.0),
];

/// Which surface a scene shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Surface {
    Teams,
    Deathmatch,
    Recap(usize),
}

const SURFACES: [Surface; 4] = [
    Surface::Teams,
    Surface::Deathmatch,
    Surface::Recap(1),
    Surface::Recap(4),
];

fn names(rng: &mut Rng, n: usize, scripts: bool) -> Vec<String> {
    (0..n)
        .map(|i| {
            if scripts {
                synth_name(
                    rng,
                    NameStyle::Script(Script::FETCHED[i % Script::FETCHED.len()]),
                )
            } else {
                synth_name(rng, NameStyle::LATIN_MIX[i % NameStyle::LATIN_MIX.len()])
            }
        })
        .collect()
}

/// A scene of 16 frames showing `surface` with `timing` over `background`,
/// with a world tag decoy away from it.
fn scene(
    fonts: &FontSet,
    layout: Layout,
    palette: Palette,
    background: Background,
    surface: Surface,
    timing: ScreenTiming,
    seed: u64,
) -> Scene {
    let mut rng = Rng::new(seed);
    let scripts = fonts.has(Script::Hangul);
    let rows = |n: usize, rng: &mut Rng| -> Vec<ScoreRow> {
        names(rng, n, scripts)
            .into_iter()
            .enumerate()
            .map(|(i, name)| ScoreRow {
                name,
                team: if i % 2 == 0 { Team::Ally } else { Team::Enemy },
                own: i == 2,
                dead: i % 4 == 1,
            })
            .collect()
    };
    let s = Scene::new(layout, background, palette, 16).with_world_tags(
        fonts,
        vec![WorldTag {
            name: synth_name(&mut rng, NameStyle::Latin(9)),
            at: (300, 960),
            shown: FrameRange { f0: 0, f1: 15 },
        }],
    );
    match surface {
        Surface::Teams => s.with_scoreboard(
            fonts,
            ScoreboardSpec {
                rows: rows(10, &mut rng),
                mode: ScoreboardMode::Teams,
                timing,
            },
        ),
        Surface::Deathmatch => s.with_scoreboard(
            fonts,
            ScoreboardSpec {
                rows: rows(14, &mut rng),
                mode: ScoreboardMode::Deathmatch,
                timing,
            },
        ),
        Surface::Recap(n) => s.with_recap(
            fonts,
            RecapSpec {
                killer: synth_name(&mut rng, NameStyle::Latin(16)),
                killer_team: Team::Enemy,
                agent: "Agent".to_owned(),
                report: names(&mut rng, n, scripts),
                timeline: 1,
                timing,
            },
        ),
    }
}

/// A frame scaled to a quarter in each direction (4 × 4 means), as the
/// engine feeds the gates.
fn quarter(frame: &RgbFrame) -> (Vec<u8>, u32, u32) {
    let (w, h) = (frame.width() / 4, frame.height() / 4);
    let src = frame.data();
    let stride = 3 * frame.width() as usize;
    let mut out = vec![0u8; (3 * w * h) as usize];
    for y in 0..h as usize {
        for x in 0..w as usize {
            for c in 0..3 {
                let mut sum = 0u32;
                for dy in 0..4 {
                    for dx in 0..4 {
                        sum += u32::from(src[(4 * y + dy) * stride + 3 * (4 * x + dx) + c]);
                    }
                }
                out[3 * (y * w as usize + x) + c] = ((sum + 8) / 16) as u8;
            }
        }
    }
    (out, w, h)
}

/// The boxes of one frame in source pixels, as the render would blur them.
fn prepared(layout: &Layout, map: &ScreenMap, detections: &[Detection]) -> Vec<PreparedBox> {
    let params = PixelParams::new(&BlurParams::default(), layout.sy, YuvColour::default()).unwrap();
    let boxes: Vec<BlurBox> = detections
        .iter()
        .map(|d| {
            let rect = map.to_source(d.rect, screens::anchor(d.region).unwrap());
            BlurBox {
                rect,
                region: d.region,
                seed: 1,
                grid_origin: (rect.x0, rect.y0),
                block_scale: 1.0,
            }
        })
        .collect();
    let mut out = vec![];
    prepare_boxes(&boxes, layout.width, layout.height, &params, &mut out);
    out
}

/// Runs a fresh detector over every frame of `scene`; returns the decided
/// frames with their boxes and the fixtures.
fn detect_all(scene: &Scene) -> (Vec<(u32, Vec<Detection>)>, Vec<Fixture>) {
    detect_filtered(scene, None)
}

/// [`detect_all`] on frames passed through `filter`.
fn detect_filtered(
    scene: &Scene,
    filter: Option<Filter>,
) -> (Vec<(u32, Vec<Detection>)>, Vec<Fixture>) {
    let layout = *scene.layout();
    let map = ScreenMap::from(&layout.geometry());
    let mut detector = ScreenDetector::new(&RegionSet::all());
    let mut fixtures: Vec<Fixture> = Vec::new();
    let mut decided = Vec::new();
    for (i, mut fx) in (0u32..).zip(scene.iter()) {
        if let Some(f) = filter {
            fx.frame = f.whole_frame(&fx.frame);
        }
        let (q, qw, qh) = quarter(&fx.frame);
        let quarter = FrameView::scaled(RgbView::new(&q, qw, qh, 3 * qw as usize).unwrap(), 0.25);
        let native = FrameView::native(
            RgbView::new(
                fx.frame.data(),
                fx.frame.width(),
                fx.frame.height(),
                3 * fx.frame.width() as usize,
            )
            .unwrap(),
        );
        detector.detect(i, &quarter, &native, &map, &mut decided);
        fixtures.push(fx);
    }
    detector.finish(&mut decided);
    assert_eq!(decided.len(), fixtures.len());
    (decided, fixtures)
}

/// What the detector did over a scene.
#[derive(Debug, Default)]
struct Outcome {
    /// Name-frames with glyph pixels not fully covered.
    leaks: Vec<String>,
    /// Name-frames in the truth.
    names: usize,
    /// Boxes over no name, outside the pre-roll and post-roll of a
    /// surface.
    false_boxes: Vec<String>,
    /// Boxes touching a world tag.
    on_world_tags: usize,
}

/// Runs a fresh detector over every frame of `scene` (optionally
/// filtered) and checks every frame against its truth.
fn run(scene: &Scene, filter: Option<Filter>) -> Outcome {
    let layout = *scene.layout();
    let map = ScreenMap::from(&layout.geometry());
    let (decided, fixtures) = detect_filtered(scene, filter);
    let mut out = Outcome::default();
    let named: Vec<u32> = fixtures
        .iter()
        .filter(|fx| fx.truth.items.iter().any(|i| i.region.is_screen()))
        .map(|fx| fx.truth.frame)
        .collect();
    let in_roll = |f: u32| {
        named
            .iter()
            .any(|&n| f + PRE_ROLL >= n && f <= n + POST_ROLL)
    };
    for ((frame, detections), fx) in decided.iter().zip(&fixtures) {
        let boxes = prepared(&layout, &map, detections);
        let screen_items: Vec<_> = fx
            .truth
            .items
            .iter()
            .filter(|i| i.region.is_screen())
            .collect();
        out.names += screen_items.len();
        for c in item_coverage(&fx.truth, &boxes)
            .iter()
            .filter(|c| c.is_leak())
        {
            out.leaks.push(format!(
                "{}x{} frame {frame} {:?}: {} of {} px",
                layout.width, layout.height, c.id, c.leaked_pixels, c.glyph_pixels
            ));
        }
        for (d, b) in detections.iter().zip(&boxes) {
            let rect = b.padded();
            if fx
                .truth
                .content
                .iter()
                .any(|c| c.kind == ContentKind::WorldTag && c.rect.intersection(&rect).is_some())
            {
                out.on_world_tags += 1;
            }
            let over_name = screen_items
                .iter()
                .any(|i| i.slot.intersection(&rect).is_some());
            if !over_name && !in_roll(*frame) {
                out.false_boxes.push(format!(
                    "{}x{} frame {frame} {:?} {}",
                    layout.width, layout.height, d.region, d.rect
                ));
            }
        }
    }
    out
}

fn assert_clean(out: &Outcome, context: &str) {
    assert!(out.names > 0, "{context}: no names drawn");
    assert!(out.leaks.is_empty(), "{context}: leaks {:?}", out.leaks);
    assert!(
        out.false_boxes.is_empty(),
        "{context}: false boxes {:?}",
        out.false_boxes
    );
    assert_eq!(out.on_world_tags, 0, "{context}: world tag boxed");
}

#[test]
fn every_surface_is_covered_in_every_layout() {
    let fonts = FontSet::latin();
    for (k, layout) in (0u64..).zip(Layout::matrix()) {
        for surface in SURFACES {
            for timing in TIMINGS {
                let s = scene(
                    &fonts,
                    layout,
                    Palette::DEFAULT,
                    Background::noise(k),
                    surface,
                    timing,
                    k,
                );
                assert_clean(
                    &run(&s, None),
                    &format!("{:?} {surface:?} {timing:?}", layout.kind),
                );
            }
        }
    }
}

#[test]
fn colour_blind_palettes_and_backgrounds_change_nothing() {
    let fonts = FontSet::latin();
    for (k, palette) in (0u64..).zip(Palette::ALL) {
        for background in Background::variety(k) {
            for surface in SURFACES {
                let s = scene(
                    &fonts,
                    Layout::reference(),
                    palette,
                    background,
                    surface,
                    TIMINGS[1],
                    k,
                );
                assert_clean(
                    &run(&s, None),
                    &format!("{} {background:?} {surface:?}", palette.name),
                );
            }
        }
    }
}

#[test]
fn gpu_colour_filters_change_nothing() {
    let fonts = FontSet::latin();
    let filters = [
        Filter::Hue(120.0),
        Filter::Hue(-90.0),
        Filter::Saturation(0.0),
        Filter::Saturation(1.8),
        Filter::Vibrance(0.8),
        Filter::Gamma(0.8),
        Filter::Gamma(1.5),
        Filter::Levels {
            brightness: -0.12,
            contrast: 0.85,
        },
        Filter::Levels {
            brightness: 0.1,
            contrast: 1.3,
        },
    ];
    for filter in filters {
        for surface in SURFACES {
            let s = scene(
                &fonts,
                Layout::reference(),
                Palette::DEFAULT,
                Background::noise(2),
                surface,
                TIMINGS[1],
                2,
            );
            assert_clean(&run(&s, Some(filter)), &format!("{filter:?} {surface:?}"));
        }
    }
}

#[test]
fn names_in_other_scripts_are_covered() {
    let fonts = match FontSet::with_fetched() {
        Ok(fonts) => fonts,
        Err(missing) if !blurrd_testkit::font::fetched_fonts_required() => {
            eprintln!("skipping: {missing}");
            return;
        }
        Err(missing) => panic!("{missing}"),
    };
    for surface in [Surface::Teams, Surface::Recap(3)] {
        let s = scene(
            &fonts,
            Layout::reference(),
            Palette::DEFAULT,
            Background::dusk(),
            surface,
            TIMINGS[0],
            7,
        );
        assert_clean(&run(&s, None), &format!("{surface:?}"));
    }
}

#[test]
fn normal_gameplay_round_ends_and_world_tags_get_no_box() {
    let fonts = FontSet::latin();
    for (k, layout) in (0u64..).zip([
        Layout::reference(),
        Layout::wide(1280, 720),
        Layout::native_4x3(1280, 960),
    ]) {
        for background in Background::variety(k) {
            let mut rng = Rng::new(k);
            let kills = (0..6).map(|i| random_kill(&mut rng, i)).collect();
            let tags = (0i32..6)
                .zip(NameStyle::LATIN_MIX.into_iter().cycle())
                .map(|(i, style)| WorldTag {
                    name: synth_name(&mut rng, style),
                    at: (400 + 220 * i, 500 + 60 * (i % 3)),
                    shown: FrameRange { f0: 0, f1: 23 },
                })
                .collect();
            let s = Scene::new(layout, background, Palette::DEFAULT, 24)
                .with_kill_feed(&fonts, KillFeedSpec::new(kills))
                .with_banner(
                    &fonts,
                    BannerSpec::new("ACE", &synth_name(&mut rng, NameStyle::Latin(8)), 2),
                )
                .with_match_end(
                    &fonts,
                    MatchEndSpec {
                        kind: MatchEndKind::RoundEnd,
                        names: vec![],
                        mvp: 0,
                        timing: ScreenTiming::animated(10, 4, 6, 4, 10.0),
                    },
                )
                .with_world_tags(&fonts, tags);
            let out = run(&s, None);
            assert_eq!(out.names, 0);
            assert!(
                out.false_boxes.is_empty(),
                "{:?} {background:?}: {:?}",
                layout.kind,
                out.false_boxes
            );
        }
    }
}

#[test]
fn blurred_boxes_pass_the_hf_residual_gate() {
    let fonts = FontSet::latin();
    for layout in [Layout::reference(), Layout::wide(1280, 720)] {
        let map = ScreenMap::from(&layout.geometry());
        let params =
            PixelParams::new(&BlurParams::default(), layout.sy, YuvColour::default()).unwrap();
        let mut measured = 0;
        for surface in SURFACES {
            let s = scene(
                &fonts,
                layout,
                Palette::DEFAULT,
                Background::dusk(),
                surface,
                TIMINGS[1],
                4,
            );
            let (decided, fixtures) = detect_all(&s);
            for ((_, detections), fx) in decided.iter().zip(&fixtures) {
                let boxes = prepared(&layout, &map, detections);
                let residuals = hf_residuals(&fx.frame, &fx.truth, &boxes, &params).unwrap();
                for (id, r) in residuals
                    .into_iter()
                    .filter_map(|(id, r)| r.map(|r| (id, r)))
                {
                    assert!(r <= HF_RESIDUAL_MAX, "{surface:?} {id:?}: residual {r}");
                    measured += 1;
                }
            }
        }
        assert!(measured >= 100, "{measured} names with measurable detail");
    }
}

#[test]
fn disabled_surfaces_are_not_read() {
    let fonts = FontSet::latin();
    let s = scene(
        &fonts,
        Layout::reference(),
        Palette::DEFAULT,
        Background::noise(1),
        Surface::Teams,
        TIMINGS[0],
        1,
    );
    let layout = *s.layout();
    let map = ScreenMap::from(&layout.geometry());
    let mut detector = ScreenDetector::new(&RegionSet::empty().with(Region::DeathRecap));
    let fx = s.render(5);
    let (q, qw, qh) = quarter(&fx.frame);
    let quarter = FrameView::scaled(RgbView::new(&q, qw, qh, 3 * qw as usize).unwrap(), 0.25);
    let signals = detector.gate(&quarter, &map);
    // The scoreboard is on screen but not enabled: its gate is not run.
    assert!(signals[0].score.abs() < f32::EPSILON);
}
