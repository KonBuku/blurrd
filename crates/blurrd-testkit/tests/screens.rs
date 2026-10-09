//! The synthetic screen surfaces: scoreboard, death recap, "Spectating"
//! banner, match-end and round-end screens, and world-space name tags.

#![allow(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] functions: a panic is a test failure"
)]

use blurrd_blur::{PixelParams, PreparedBox, YuvColour, prepare_boxes};
use blurrd_core::{BlurBox, BlurParams, FrameRange, NameKind, Region};
use blurrd_testkit::hud::killfeed::Team;
use blurrd_testkit::hud::match_end::{MatchEndKind, MatchEndSpec};
use blurrd_testkit::hud::recap::RecapSpec;
use blurrd_testkit::hud::scoreboard::{self, ScoreRow, ScoreboardMode, ScoreboardSpec};
use blurrd_testkit::hud::screen::ScreenTiming;
use blurrd_testkit::hud::spectating::SpectatingSpec;
use blurrd_testkit::hud::world_tag::WorldTag;
use blurrd_testkit::metrics::item_coverage;
use blurrd_testkit::truth::ContentKind;
use blurrd_testkit::{
    Background, Fixture, FontSet, Layout, NameStyle, Palette, Rng, Scene, Script, synth_name,
};

/// A fade and slide like a panel animating in and out.
const ANIMATED: ScreenTiming = ScreenTiming::animated(2, 4, 2, 4, 16.0);

fn names(rng: &mut Rng, n: usize) -> Vec<String> {
    (0..n)
        .map(|i| synth_name(rng, NameStyle::LATIN_MIX[i % NameStyle::LATIN_MIX.len()]))
        .collect()
}

fn team_board(rng: &mut Rng, timing: ScreenTiming) -> ScoreboardSpec {
    let rows = names(rng, 10)
        .into_iter()
        .enumerate()
        .map(|(i, name)| ScoreRow {
            name,
            team: if i % 2 == 0 { Team::Ally } else { Team::Enemy },
            own: i == 4,
            dead: i % 3 == 1,
        })
        .collect();
    ScoreboardSpec {
        rows,
        mode: ScoreboardMode::Teams,
        timing,
    }
}

fn recap(rng: &mut Rng, rows: usize, timing: ScreenTiming) -> RecapSpec {
    RecapSpec {
        killer: synth_name(rng, NameStyle::Latin(16)),
        killer_team: Team::Enemy,
        agent: "Agent".to_owned(),
        report: names(rng, rows),
        timeline: 1,
        timing,
    }
}

/// Every screen surface in one scene: board, recap, banner and
/// match-end screen one after the other, over a world tag.
fn all_surfaces(fonts: &FontSet, layout: Layout, palette: Palette, seed: u64) -> Scene {
    let mut rng = Rng::new(seed);
    let at = |start| ScreenTiming { start, ..ANIMATED };
    Scene::new(layout, Background::noise(seed), palette, 50)
        .with_scoreboard(fonts, team_board(&mut rng, at(2)))
        .with_recap(fonts, recap(&mut rng, 3, at(14)))
        .with_spectating(
            fonts,
            SpectatingSpec {
                name: synth_name(&mut rng, NameStyle::TwoWord),
                timing: at(26),
            },
        )
        .with_match_end(
            fonts,
            MatchEndSpec {
                kind: MatchEndKind::Victory,
                names: names(&mut rng, 5),
                mvp: 2,
                timing: at(38),
            },
        )
        .with_world_tags(
            fonts,
            vec![WorldTag {
                name: synth_name(&mut rng, NameStyle::Latin(8)),
                at: (300, 600),
                shown: FrameRange { f0: 0, f1: 49 },
            }],
        )
}

fn slot_boxes(layout: &Layout, fx: &Fixture) -> Vec<PreparedBox> {
    let params = PixelParams::new(&BlurParams::default(), layout.sy, YuvColour::default()).unwrap();
    let boxes: Vec<BlurBox> = fx
        .truth
        .items
        .iter()
        .map(|i| BlurBox {
            rect: i.slot,
            region: i.region,
            seed: 1,
            grid_origin: (i.slot.x0, i.slot.y0),
            block_scale: 1.0,
        })
        .collect();
    let mut out = vec![];
    prepare_boxes(&boxes, layout.width, layout.height, &params, &mut out);
    out
}

#[test]
fn every_surface_in_every_layout_is_covered_by_its_slots() {
    let fonts = FontSet::latin();
    for (k, layout) in (0u64..).zip(Layout::matrix()) {
        let scene = all_surfaces(&fonts, layout, Palette::DEFAULT, k);
        let mut seen = [0usize; 4];
        for fx in scene.iter() {
            for item in &fx.truth.items {
                let glyphs = item.glyphs.bounds().unwrap();
                let s = item.slot;
                assert!(
                    s.x0 <= glyphs.x0
                        && s.y0 <= glyphs.y0
                        && s.x1 >= glyphs.x1
                        && s.y1 >= glyphs.y1,
                    "{:?}: slot {s} misses {glyphs}",
                    layout.kind
                );
                let i = Region::SCREENS
                    .iter()
                    .position(|r| *r == item.region)
                    .unwrap();
                seen[i] += 1;
            }
            let boxes = slot_boxes(&layout, &fx);
            assert!(
                item_coverage(&fx.truth, &boxes)
                    .iter()
                    .all(|c| !c.is_leak()),
                "{:?} frame {}",
                layout.kind,
                fx.truth.frame
            );
        }
        // Ten rows, four recap names, one spectated, five cards, over 10
        // frames each.
        assert_eq!(seen, [10 * 10, 4 * 10, 10, 5 * 10], "{:?}", layout.kind);
    }
}

#[test]
fn surfaces_fade_and_slide_in_and_out() {
    let fonts = FontSet::latin();
    let scene = all_surfaces(&fonts, Layout::reference(), Palette::DEFAULT, 3);
    let first = scene.render(2);
    let settled = scene.render(6);
    // The first row's player is alive (dead players' names are dimmer).
    let row = |fx: &Fixture| {
        fx.truth
            .in_region(Region::Scoreboard)
            .min_by_key(|i| i.id)
            .map(|i| (i.opacity, i.text.y0))
            .unwrap()
    };
    let (a0, y0) = row(&first);
    let (a1, y1) = row(&settled);
    assert!(a0 < 0.5 && (a1 - 1.0).abs() < f32::EPSILON);
    assert!(y0 > y1, "slides up into place");
    // Nothing before the first frame or after the fade-out.
    assert_eq!(scene.render(1).truth.items, []);
    assert!(
        scene
            .render(16)
            .truth
            .in_region(Region::Scoreboard)
            .next()
            .is_none()
    );
}

#[test]
fn scoreboard_rows_follow_the_measured_grid() {
    let fonts = FontSet::latin();
    let mut rng = Rng::new(5);
    let scene = Scene::new(
        Layout::reference(),
        Background::noise(5),
        Palette::DEFAULT,
        3,
    )
    .with_scoreboard(&fonts, team_board(&mut rng, ScreenTiming::instant(0, 3)));
    let fx = scene.render(1);
    let mut tops: Vec<i32> = fx
        .truth
        .in_region(Region::Scoreboard)
        .map(|i| i.slot.y0 - 2)
        .collect();
    tops.sort_unstable();
    assert_eq!(tops, [338, 372, 406, 440, 474, 569, 603, 637, 671, 705]);
    for item in fx.truth.in_region(Region::Scoreboard) {
        assert_eq!(item.kind, NameKind::ScoreboardRow);
        assert!(
            item.text.x0 >= scoreboard::NAME_CELL_X.0 && item.text.x1 <= scoreboard::NAME_CELL_X.1
        );
    }
    // Agent icons, column titles and numbers are content, not names.
    for kind in [
        ContentKind::Portrait,
        ContentKind::ScreenText,
        ContentKind::Stat,
    ] {
        assert!(fx.truth.content.iter().any(|c| c.kind == kind), "{kind:?}");
    }
}

#[test]
fn deathmatch_boards_hold_fourteen_rows() {
    let fonts = FontSet::latin();
    let mut rng = Rng::new(9);
    let rows = names(&mut rng, scoreboard::MAX_ROWS + 2)
        .into_iter()
        .map(|name| ScoreRow {
            name,
            team: Team::Ally,
            own: false,
            dead: false,
        })
        .collect();
    let spec = ScoreboardSpec {
        rows,
        mode: ScoreboardMode::Deathmatch,
        timing: ScreenTiming::instant(0, 2),
    };
    let fx = Scene::new(Layout::reference(), Background::dusk(), Palette::DEFAULT, 2)
        .with_scoreboard(&fonts, spec)
        .render(0);
    assert_eq!(
        fx.truth.in_region(Region::Scoreboard).count(),
        scoreboard::MAX_ROWS
    );
}

#[test]
fn recap_height_follows_its_rows() {
    let fonts = FontSet::latin();
    let mut rng = Rng::new(4);
    for rows in 1..=5 {
        let spec = recap(&mut rng, rows, ScreenTiming::instant(0, 2));
        let top = spec.top();
        let fx = Scene::new(
            Layout::reference(),
            Background::noise(4),
            Palette::DEFAULT,
            2,
        )
        .with_recap(&fonts, spec)
        .render(0);
        let items: Vec<_> = fx.truth.in_region(Region::DeathRecap).collect();
        assert_eq!(items.len(), rows + 1);
        // The killer's strip sits 114 px below the panel top.
        let strip = items.iter().map(|i| i.slot.y0).min().unwrap();
        assert_eq!(strip, top + 115);
    }
}

#[test]
fn round_end_banners_and_world_tags_are_not_names() {
    let fonts = FontSet::latin();
    let mut rng = Rng::new(8);
    let tag = synth_name(&mut rng, NameStyle::Latin(10));
    let scene = Scene::new(
        Layout::reference(),
        Background::noise(8),
        Palette::DEFAULT,
        30,
    )
    .with_match_end(
        &fonts,
        MatchEndSpec {
            kind: MatchEndKind::RoundEnd,
            names: names(&mut rng, 5),
            mvp: 0,
            timing: ScreenTiming::animated(0, 6, 10, 6, 10.0),
        },
    )
    .with_world_tags(
        &fonts,
        vec![WorldTag {
            name: tag,
            at: (1100, 560),
            shown: FrameRange { f0: 0, f1: 29 },
        }],
    );
    for fx in scene.iter() {
        assert_eq!(fx.truth.items, []);
        assert!(
            fx.truth
                .content
                .iter()
                .any(|c| c.kind == ContentKind::WorldTag)
        );
    }
    assert!(
        scene
            .render(8)
            .truth
            .content
            .iter()
            .any(|c| c.kind == ContentKind::BannerTitle)
    );
}

#[test]
fn colour_blind_palettes_tint_the_rows() {
    let fonts = FontSet::latin();
    let pixel = |palette| {
        let mut rng = Rng::new(2);
        let fx = Scene::new(
            Layout::reference(),
            Background::Flat([40, 40, 40]),
            palette,
            2,
        )
        .with_scoreboard(&fonts, team_board(&mut rng, ScreenTiming::instant(0, 2)))
        .render(0);
        fx.frame.pixel(1200, 350).unwrap()
    };
    let default = pixel(Palette::DEFAULT);
    for palette in &Palette::ALL[1..] {
        assert_ne!(pixel(*palette), default, "{}", palette.name);
    }
}

#[test]
fn names_in_other_scripts_fill_the_slots() {
    let fonts = match FontSet::with_fetched() {
        Ok(fonts) => fonts,
        Err(missing) if !blurrd_testkit::font::fetched_fonts_required() => {
            eprintln!("skipping: {missing}");
            return;
        }
        Err(missing) => panic!("{missing}"),
    };
    let mut rng = Rng::new(31);
    let rows = Script::FETCHED
        .iter()
        .map(|s| ScoreRow {
            name: synth_name(&mut rng, NameStyle::Script(*s)),
            team: Team::Enemy,
            own: false,
            dead: false,
        })
        .collect();
    let layout = Layout::reference();
    let fx = Scene::new(layout, Background::noise(31), Palette::DEFAULT, 2)
        .with_scoreboard(
            &fonts,
            ScoreboardSpec {
                rows,
                mode: ScoreboardMode::Teams,
                timing: ScreenTiming::instant(0, 2),
            },
        )
        .render(0);
    assert_eq!(fx.truth.items.len(), Script::FETCHED.len());
    for item in &fx.truth.items {
        assert!(item.glyphs.count() > 40, "{:?}", item.id);
    }
    let boxes = slot_boxes(&layout, &fx);
    assert!(
        item_coverage(&fx.truth, &boxes)
            .iter()
            .all(|c| !c.is_leak())
    );
}

#[test]
fn slots_stay_inside_the_frame_bounds() {
    let fonts = FontSet::latin();
    for layout in Layout::matrix() {
        let fx = all_surfaces(&fonts, layout, Palette::PROTANOPIA, 6).render(8);
        let bounds = fx.frame.bounds();
        for item in &fx.truth.items {
            assert!(
                item.slot.intersection(&bounds) == Some(item.slot),
                "{:?} {}",
                layout.kind,
                item.slot
            );
        }
    }
}
