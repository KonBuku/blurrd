//! The synthetic HUD generator: geometry, animations, layouts, palettes,
//! scripts, ground truth and the metrics on top of it.

#![allow(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] functions: a panic is a test failure"
)]

use blurrd_blur::metrics::HF_RESIDUAL_MAX;
use blurrd_blur::{PixelParams, PreparedBox, YuvColour, prepare_boxes};
use blurrd_core::{BlurBox, BlurParams, FrameRange, RectI, Region};
use blurrd_testkit::hud::banner::BannerSpec;
use blurrd_testkit::hud::chat::{Channel, ChatMessage, ChatSpec, Language};
use blurrd_testkit::hud::killfeed::{self, Kill, KillFeedSpec, Team, Weapon};
use blurrd_testkit::hud::voice::{self, Speaker, VoiceSpec};
use blurrd_testkit::metrics::{Evaluation, ItemCoverage, hf_residuals, item_coverage};
use blurrd_testkit::proposals::pixel_change_boxes;
use blurrd_testkit::truth::ContentKind;
use blurrd_testkit::{
    Background, Fixture, Flash, FontSet, FrameTruth, Layout, NameStyle, Palette, Rng, Scene,
    Script, TruthItem, synth_name,
};

fn kill(rng: &mut Rng, birth: u32, style: NameStyle, teams: (Team, Team)) -> Kill {
    let killer_len = rng.range(3, 12) as usize;
    Kill {
        killer: synth_name(rng, NameStyle::Latin(killer_len)),
        victim: synth_name(rng, style),
        killer_team: teams.0,
        victim_team: teams.1,
        weapon: *rng.pick(&[
            Weapon::Rifle,
            Weapon::Pistol,
            Weapon::Knife,
            Weapon::Ability,
        ]),
        badge: rng.chance(0.3).then(|| rng.range(2, 5) as u8),
        own: rng.chance(0.2),
        headshot: rng.chance(0.5),
        birth,
    }
}

fn message(channel: Channel, name: &str, time: u32) -> ChatMessage {
    ChatMessage {
        channel,
        name: name.to_owned(),
        team: Team::Enemy,
        text: "see you at the next round".to_owned(),
        role: (channel == Channel::Broadcast).then(|| "Caster".to_owned()),
        wrap: false,
        time,
    }
}

fn always() -> Vec<FrameRange> {
    vec![FrameRange { f0: 0, f1: 10_000 }]
}

/// A scene with every HUD element visible at frame 60.
fn full_scene(fonts: &FontSet, layout: Layout, palette: Palette, seed: u64) -> Scene {
    let mut rng = Rng::new(seed);
    let kills = (0..4)
        .map(|i| {
            let teams = match i {
                0 => (Team::Ally, Team::Enemy),
                1 => (Team::Enemy, Team::Ally),
                2 => (Team::Enemy, Team::Enemy),
                _ => (Team::Ally, Team::Ally),
            };
            kill(&mut rng, i * 5, NameStyle::LATIN_MIX[i as usize], teams)
        })
        .collect();
    let speakers = (0..2)
        .map(|slot| Speaker {
            name: synth_name(&mut rng, NameStyle::TwoWord),
            location: voice::LOCATIONS[usize::from(slot)].to_owned(),
            slot,
            shown: always(),
        })
        .collect();
    let messages = vec![
        message(Channel::All, &synth_name(&mut rng, NameStyle::Latin(9)), 0),
        message(
            Channel::Team,
            &synth_name(&mut rng, NameStyle::Latin(16)),
            5,
        ),
        message(
            Channel::System,
            &synth_name(&mut rng, NameStyle::Latin(6)),
            10,
        ),
    ];
    Scene::new(layout, Background::dusk(), palette, 120)
        .with_kill_feed(fonts, KillFeedSpec::new(kills))
        .with_banner(
            fonts,
            BannerSpec::new("ACE", &synth_name(&mut rng, NameStyle::Latin(8)), 0),
        )
        .with_voice(fonts, VoiceSpec { speakers })
        .with_chat(fonts, ChatSpec::new(Language::English, messages))
}

fn blur_params(layout: &Layout) -> PixelParams {
    PixelParams::new(&BlurParams::default(), layout.sy, YuvColour::default()).unwrap()
}

/// Prepared boxes for rectangles, as the render would blur them.
fn boxes(layout: &Layout, rects: impl IntoIterator<Item = RectI>) -> Vec<PreparedBox> {
    let bb: Vec<BlurBox> = rects
        .into_iter()
        .map(|rect| BlurBox {
            rect,
            region: Region::Manual,
            seed: 3,
            grid_origin: (rect.x0, rect.y0),
            block_scale: 1.0,
        })
        .collect();
    let mut out = vec![];
    prepare_boxes(
        &bb,
        layout.width,
        layout.height,
        &blur_params(layout),
        &mut out,
    );
    out
}

fn items(truth: &FrameTruth, region: Region) -> Vec<&TruthItem> {
    truth.in_region(region).collect()
}

fn check_truth(fx: &Fixture) {
    let bounds = fx.frame.bounds();
    for item in &fx.truth.items {
        let glyphs = item.glyphs.bounds().unwrap();
        assert!(glyphs.intersection(&bounds).is_some(), "{item:?}");
        assert!(item.glyphs.count() > 20, "{:?}", item.id);
        let s = item.slot;
        assert!(
            s.x0 <= glyphs.x0 && s.y0 <= glyphs.y0 && s.x1 >= glyphs.x1 && s.y1 >= glyphs.y1,
            "slot {s} does not contain the glyphs {glyphs}"
        );
        assert!(
            item.text.x0 >= glyphs.x0 && item.text.x1 <= glyphs.x1,
            "text inside its halo"
        );
    }
}

#[test]
fn every_layout_and_palette_is_covered_by_slot_boxes() {
    let fonts = FontSet::latin();
    for layout in Layout::matrix() {
        for (i, palette) in Palette::ALL.into_iter().enumerate() {
            let scene = full_scene(&fonts, layout, palette, 11 + i as u64);
            let fx = scene.render(60);
            check_truth(&fx);
            let t = &fx.truth;
            let counts = [
                Region::KillFeed,
                Region::Banner,
                Region::VoiceChat,
                Region::TextChat,
            ]
            .map(|r| items(t, r).len());
            assert_eq!(counts, [8, 1, 2, 3], "{:?} {}", layout.kind, palette.name);
            // Safe-mode slot boxes leave nothing readable ...
            let slots = boxes(&layout, t.items.iter().map(|i| i.slot));
            assert!(
                item_coverage(t, &slots).iter().all(|c| !c.is_leak()),
                "{:?} {}",
                layout.kind,
                palette.name
            );
            // ... while boxes that miss the halo leak.
            let tight = boxes(
                &layout,
                t.items.iter().map(|i| RectI {
                    x1: i.text.x1 - 3,
                    ..i.text
                }),
            );
            assert!(item_coverage(t, &tight).iter().all(ItemCoverage::is_leak));
        }
    }
}

#[test]
fn blurred_slots_pass_the_hf_residual_gate() {
    let fonts = FontSet::latin();
    for layout in [Layout::reference(), Layout::wide(1280, 720)] {
        let fx = full_scene(&fonts, layout, Palette::DEFAULT, 5).render(60);
        let slots = boxes(&layout, fx.truth.items.iter().map(|i| i.slot));
        let residuals = hf_residuals(&fx.frame, &fx.truth, &slots, &blur_params(&layout)).unwrap();
        let measured: Vec<f64> = residuals.iter().filter_map(|(_, r)| *r).collect();
        assert!(measured.len() >= 10, "most names have measurable detail");
        for r in measured {
            assert!(r <= HF_RESIDUAL_MAX, "residual {r}");
        }
        // Without a blur the residual is 1.
        let none = hf_residuals(&fx.frame, &fx.truth, &[], &blur_params(&layout)).unwrap();
        assert!(
            none.iter()
                .filter_map(|(_, r)| *r)
                .all(|r| (r - 1.0).abs() < 1e-9)
        );
    }
}

#[test]
fn rendering_is_deterministic() {
    let fonts = FontSet::latin();
    let a = full_scene(&fonts, Layout::reference(), Palette::DEFAULT, 1);
    let b = full_scene(&fonts, Layout::reference(), Palette::DEFAULT, 1);
    // Compared with `assert!` so that a failure does not print the frames.
    assert!(a.render(40) == b.render(40), "same scene, same frame");
    assert!(a.render(6).frame != a.render(7).frame, "rows slide in");
}

#[test]
fn kill_feed_stacks_eight_rows_inside_the_crop() {
    let fonts = FontSet::latin();
    let mut rng = Rng::new(9);
    let kills = (0..8)
        .map(|i| kill(&mut rng, i, NameStyle::Latin(10), (Team::Ally, Team::Enemy)))
        .collect();
    let layout = Layout::reference();
    let scene = Scene::new(layout, Background::noise(2), Palette::DEFAULT, 40)
        .with_kill_feed(&fonts, KillFeedSpec::new(kills));
    let fx = scene.render(30);
    let mut tops: Vec<i32> = items(&fx.truth, Region::KillFeed)
        .iter()
        .map(|i| i.slot.y0)
        .collect();
    tops.dedup();
    assert_eq!(tops.len(), 8);
    assert!(tops.windows(2).all(|w| w[1] - w[0] == killfeed::ROW_PITCH));
    // The extended feed crop of the plan (C18) reaches y 412.
    let last = items(&fx.truth, Region::KillFeed)
        .iter()
        .map(|i| i.slot.y1)
        .max()
        .unwrap();
    assert!(last <= 412, "{last}");
}

#[test]
fn kill_feed_rows_wipe_in_jump_up_and_fade() {
    let fonts = FontSet::latin();
    let mut rng = Rng::new(4);
    let mut first = kill(&mut rng, 0, NameStyle::Latin(8), (Team::Ally, Team::Enemy));
    first.badge = None;
    let second = kill(&mut rng, 50, NameStyle::Latin(8), (Team::Enemy, Team::Ally));
    let mut spec = KillFeedSpec::new(vec![first, second]);
    spec.life = 100;
    let layout = Layout::reference();
    let scene = Scene::new(layout, Background::noise(1), Palette::DEFAULT, 200)
        .with_kill_feed(&fonts, spec);
    let victim_of = |frame: u32, id: u32| {
        scene
            .render(frame)
            .truth
            .items
            .into_iter()
            .find(|i| i.id.0 == id)
    };
    // Cover frame: the bar is there, the names are not.
    assert!(victim_of(50, 1003).is_none() && victim_of(50, 1002).is_none());
    // Slide-in: 60, 25 and 8 px short of the final position.
    let settled = victim_of(54, 1003).unwrap().text.x0;
    for (frame, dx) in [(51, -60), (52, -25), (53, -8)] {
        let x = victim_of(frame, 1003).unwrap().text.x0;
        assert!(
            (x - settled - dx).abs() <= 1,
            "frame {frame}: {x} vs {settled}"
        );
    }
    // Fade-out of the first row, then the second row jumps up a pitch.
    let fading = victim_of(110, 1001).unwrap();
    assert!(fading.opacity < 0.6 && fading.opacity > 0.0);
    let before = victim_of(117, 1003).unwrap().text.y0;
    assert!(victim_of(118, 1001).is_none());
    let after = victim_of(118, 1003).unwrap().text.y0;
    assert_eq!(before - after, killfeed::ROW_PITCH);
}

#[test]
fn colour_blind_palettes_change_the_bars() {
    let fonts = FontSet::latin();
    let mut rng = Rng::new(6);
    for palette in Palette::ALL {
        let mut k = kill(&mut rng, 0, NameStyle::Latin(6), (Team::Ally, Team::Enemy));
        k.own = false;
        let layout = Layout::reference();
        let scene = Scene::new(layout, Background::Flat([0, 0, 0]), palette, 10)
            .with_kill_feed(&fonts, KillFeedSpec::new(vec![k]));
        let fx = scene.render(5);
        // Bar pixel right of the victim name, below the text.
        let bar = fx.frame.pixel(1836, 96 + 30).unwrap();
        for (got, want) in bar.iter().zip(palette.enemy) {
            let expected = (f32::from(want) * 0.92).round();
            assert!(
                (f32::from(*got) - expected).abs() <= 1.0,
                "{}",
                palette.name
            );
        }
    }
}

#[test]
fn banners_slide_in_and_settle_for_short_and_long_names() {
    let fonts = FontSet::latin();
    for len in [3, 16] {
        let name = synth_name(&mut Rng::new(len as u64), NameStyle::Latin(len));
        let spec = BannerSpec::new("CLUTCH", &name, 10);
        let settle = spec.settle();
        let scene = Scene::new(
            Layout::reference(),
            Background::white_sky(),
            Palette::DEFAULT,
            300,
        )
        .with_banner(&fonts, spec);
        let at = |f: u32| {
            items(&scene.render(f).truth, Region::Banner)
                .first()
                .map(|i| (*i).clone())
        };
        assert!(at(12).is_none(), "the box appears before the name");
        let xs: Vec<i32> = (14..=settle).map(|f| at(f).unwrap().text.x0).collect();
        assert!(xs.windows(2).all(|w| w[1] <= w[0]), "moves left: {xs:?}");
        assert!(xs[0] - xs[1] >= 30, "fast start");
        let settled = at(settle + 20).unwrap();
        let centre = i32::midpoint(settled.text.x0, settled.text.x1);
        assert!((centre - 960).abs() <= 3, "centred: {centre}");
        // The slot is the sixteen-"W" width, so it holds the longest name.
        assert!(settled.slot.width() > 150 && settled.slot.x0 <= settled.text.x0);
    }
}

#[test]
fn voice_has_four_slots_two_word_names_and_blinks() {
    let fonts = FontSet::latin();
    let mut rng = Rng::new(12);
    let speakers = (0..voice::SLOTS)
        .map(|slot| Speaker {
            name: synth_name(&mut rng, NameStyle::TwoWord),
            location: voice::LOCATIONS[usize::from(slot)].to_owned(),
            slot,
            shown: if slot == 3 {
                vec![FrameRange { f0: 0, f1: 9 }, FrameRange { f0: 16, f1: 40 }]
            } else {
                always()
            },
        })
        .collect();
    let scene = Scene::new(
        Layout::reference(),
        Background::noise(4),
        Palette::DEFAULT,
        40,
    )
    .with_voice(&fonts, VoiceSpec { speakers });
    let fx = scene.render(5);
    let v = items(&fx.truth, Region::VoiceChat);
    assert_eq!(v.len(), 4);
    for item in &v {
        // Never reaches the location line below the name.
        assert!(item.text.y1 <= item.slot.y1);
        let loc = fx
            .truth
            .content
            .iter()
            .filter(|c| c.kind == ContentKind::Location)
            .all(|c| c.rect.y0 >= item.text.y1 || c.rect.y1 <= item.text.y0);
        assert!(loc);
    }
    assert_eq!(items(&scene.render(12).truth, Region::VoiceChat).len(), 3);
}

#[test]
fn chat_prefixes_move_the_name_per_language() {
    let fonts = FontSet::latin();
    let layout = Layout::reference();
    let mut starts = vec![];
    for language in Language::ALL {
        let messages = vec![message(Channel::Broadcast, "Ravelin", 0)];
        let scene = Scene::new(layout, Background::noise(8), Palette::DEFAULT, 10)
            .with_chat(&fonts, ChatSpec::new(language, messages));
        let fx = scene.render(1);
        let name = items(&fx.truth, Region::TextChat)[0].text;
        let prefix = fx
            .truth
            .content
            .iter()
            .find(|c| c.kind == ContentKind::ChatPrefix)
            .unwrap()
            .rect;
        assert!(prefix.x1 < name.x0);
        starts.push(name.x0);
    }
    assert!(starts[0] < starts[1] && starts[0] < starts[2], "{starts:?}");
}

#[test]
fn chat_lines_push_up_wrap_and_fade() {
    let fonts = FontSet::latin();
    let mut wrapped = message(Channel::Team, "Second", 20);
    wrapped.wrap = true;
    wrapped.text = "this message is long enough to wrap onto a second line".to_owned();
    let mut spec = ChatSpec::new(
        Language::English,
        vec![
            message(Channel::All, "First", 0),
            wrapped,
            message(Channel::System, "Third", 40),
            message(Channel::All, "Fourth", 60),
        ],
    );
    spec.life = 100;
    let scene = Scene::new(
        Layout::reference(),
        Background::noise(5),
        Palette::DEFAULT,
        200,
    )
    .with_chat(&fonts, spec);
    let top = |f: u32, id: u32| {
        scene
            .render(f)
            .truth
            .items
            .into_iter()
            .find(|i| i.id.0 == id)
            .map(|i| i.text.y0)
    };
    let first_alone = top(10, 4000).unwrap();
    // The wrapped message pushes the first line up by a line pitch plus the
    // continuation offset.
    let pushed = top(25, 4000).unwrap();
    assert!(
        (first_alone - pushed - 29 - 18).abs() <= 1,
        "{first_alone} {pushed}"
    );
    // A fourth message drops the oldest line (three at most).
    assert!(top(70, 4000).is_none());
    assert!(top(70, 4003).is_some());
    // System lines have no colon: the phrase follows the name.
    let fx = scene.render(45);
    assert!(
        fx.truth
            .content
            .iter()
            .any(|c| c.kind == ContentKind::SystemPhrase)
    );
    // Fading: the newest line fades after its life.
    let fading = scene
        .render(165)
        .truth
        .items
        .into_iter()
        .find(|i| i.id.0 == 4003)
        .unwrap();
    assert!(fading.opacity < 1.0);
}

#[test]
fn flashes_wash_out_the_hud_but_not_the_truth() {
    let fonts = FontSet::latin();
    let base = full_scene(&fonts, Layout::reference(), Palette::DEFAULT, 3);
    let flashed = base.clone().with_flash(Flash {
        start: 60,
        frames: 10,
        amount: 0.8,
        area: None,
    });
    let (a, b) = (base.render(60), flashed.render(60));
    assert_eq!(a.truth, b.truth);
    let dark = |f: &Fixture| f.frame.data().iter().filter(|&&v| v < 100).count();
    assert!(dark(&b) * 10 < dark(&a));
}

#[test]
fn names_in_other_scripts_render_with_fetched_fonts() {
    let fonts = match FontSet::with_fetched() {
        Ok(fonts) => fonts,
        Err(missing) if !blurrd_testkit::font::fetched_fonts_required() => {
            eprintln!("skipping: {missing}");
            return;
        }
        Err(missing) => panic!("{missing}"),
    };
    let mut rng = Rng::new(21);
    for script in Script::FETCHED {
        let name = synth_name(&mut rng, NameStyle::Script(script));
        assert!(fonts.covers(&name), "{script:?}");
        let speakers = vec![Speaker {
            name,
            location: voice::LOCATIONS[0].to_owned(),
            slot: 0,
            shown: always(),
        }];
        let layout = Layout::reference();
        let scene = Scene::new(layout, Background::noise(6), Palette::DEFAULT, 2)
            .with_voice(&fonts, VoiceSpec { speakers });
        let fx = scene.render(0);
        check_truth(&fx);
        let item = items(&fx.truth, Region::VoiceChat)[0];
        assert!(item.glyphs.count() > 60, "{script:?}");
        let slot = boxes(&layout, [item.slot]);
        assert!(!item_coverage(&fx.truth, &slot)[0].is_leak());
    }
}

#[test]
fn evaluation_over_a_sequence_with_slot_boxes_is_clean() {
    let fonts = FontSet::latin();
    let layout = Layout::reference();
    let scene = full_scene(&fonts, layout, Palette::DEFAULT, 17);
    let mut eval = Evaluation::new();
    for fx in scene.iter() {
        let slots = boxes(&layout, fx.truth.items.iter().map(|i| i.slot));
        eval.add_frame(&fx.truth, &slots);
    }
    let s = eval.summary();
    assert_eq!(s.frames, 120);
    assert!(s.item_frames > 1000);
    assert_eq!(s.leaked_item_frames, 0);
    assert_eq!(s.flicker_runs, 0);
    assert_eq!(s.false_positive_box_frames, 0);
    assert!((s.recall() - 1.0).abs() < f64::EPSILON);
    assert!(
        s.over_blur.ratio() > 0.0,
        "slot boxes are wider than the names"
    );
}

#[test]
fn pixel_changes_propose_a_new_row() {
    let fonts = FontSet::latin();
    let mut rng = Rng::new(30);
    let layout = Layout::reference();
    let scene = Scene::new(layout, Background::noise(9), Palette::DEFAULT, 20).with_kill_feed(
        &fonts,
        KillFeedSpec::new(vec![kill(
            &mut rng,
            10,
            NameStyle::Latin(9),
            (Team::Ally, Team::Enemy),
        )]),
    );
    let (before, after) = (scene.render(9), scene.render(14));
    let zone = RectI::new(660, 80, 1920, 412).unwrap();
    let changes = pixel_change_boxes(&before.frame, &after.frame, zone, 24, 3, 20);
    for item in &after.truth.items {
        assert!(
            changes.iter().any(|c| c.intersection(&item.text).is_some()),
            "{:?} not proposed",
            item.id
        );
    }
}
