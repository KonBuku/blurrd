//! Pixel classes on synthetic kill-feed frames from blurrd-testkit.

#![allow(
    clippy::cast_possible_wrap,
    clippy::unwrap_used,
    reason = "test helpers on small frames: coordinates never wrap, a panic is a failure"
)]

use blurrd_core::{RectI, Region};
use blurrd_detect::components::Integral;
use blurrd_detect::{Class, ClassParams, PixelClasses, RgbView, hsv};
use blurrd_testkit::hud::killfeed::{self, Kill, KillFeedSpec, Team, Weapon};
use blurrd_testkit::{
    Background, Fixture, FontSet, Layout, NameStyle, Palette, Rng, Scene, synth_name,
};

fn feed_scene(palette: Palette, rows: u32) -> Fixture {
    let fonts = FontSet::latin();
    let mut rng = Rng::new(u64::from(rows));
    let kills = (0..rows)
        .map(|i| Kill {
            killer: synth_name(&mut rng, NameStyle::Latin(8)),
            victim: synth_name(&mut rng, NameStyle::Latin(11)),
            killer_team: Team::Ally,
            victim_team: Team::Enemy,
            weapon: Weapon::Rifle,
            badge: None,
            own: false,
            headshot: false,
            birth: i,
        })
        .collect();
    Scene::new(Layout::reference(), Background::noise(7), palette, 40)
        .with_kill_feed(&fonts, KillFeedSpec::new(kills))
        .render(30)
}

fn classify(fx: &Fixture, params: &ClassParams) -> PixelClasses {
    let f = &fx.frame;
    let view = RgbView::new(f.data(), f.width(), f.height(), 3 * f.width() as usize).unwrap();
    let mut classes = PixelClasses::new();
    classes.compute(&view, params);
    classes
}

/// Mean of the bar class over the score columns (x 1660..1837, port spec
/// 3.1) in the inner rows of each kill-feed row.
fn bar_means(classes: &PixelClasses, rows: u32) -> Vec<f32> {
    let bar = Integral::new(&classes.mask(Class::Bar));
    (0..rows as i32)
        .map(|i| {
            let y0 = killfeed::FIRST_ROW_TOP + killfeed::ROW_PITCH * i;
            bar.mean(RectI::new(1660, y0 + 4, 1837, y0 + killfeed::ROW_H - 4).unwrap())
        })
        .collect()
}

#[test]
fn default_classes_find_bars_and_name_strokes() {
    let fx = feed_scene(Palette::DEFAULT, 6);
    let classes = classify(&fx, &ClassParams::default());
    for mean in bar_means(&classes, 6) {
        assert!(mean >= 0.5, "bar mean {mean}");
    }
    // Every name has text-stroke pixels on its glyphs ...
    let names: Vec<_> = fx.truth.in_region(Region::KillFeed).collect();
    assert_eq!(names.len(), 12);
    for item in &names {
        let strokes = item
            .glyphs
            .pixels()
            .filter(|&(x, y)| classes.has(x as u32, y as u32, Class::Text))
            .count();
        assert!(strokes >= 15, "{:?}: {strokes}", item.id);
    }
    // ... and the background and bars have none: stroke pixels lie on
    // names or on other HUD content (icons, portraits).
    let stray = (0..fx.frame.height() as i32)
        .flat_map(|y| (0..fx.frame.width() as i32).map(move |x| (x, y)))
        .filter(|&(x, y)| classes.has(x as u32, y as u32, Class::Text))
        .filter(|&(x, y)| !names.iter().any(|i| i.glyphs.contains(x, y)))
        .filter(|&(x, y)| {
            !fx.truth
                .content
                .iter()
                .any(|c| x >= c.rect.x0 && x < c.rect.x1 && y >= c.rect.y0 && y < c.rect.y1)
        })
        .count();
    assert_eq!(stray, 0);
}

#[test]
fn colour_blind_bars_need_learned_hues() {
    for palette in &Palette::ALL[1..] {
        let fx = feed_scene(*palette, 3);
        // The default hue ranges miss the bars ...
        let fixed = classify(&fx, &ClassParams::default());
        for mean in bar_means(&fixed, 3) {
            assert!(mean < 0.1, "{}: {mean}", palette.name);
        }
        // ... hue ranges centred on the palette's two colours find them as
        // well as the default ranges find the default bars (the row test
        // of the port spec needs a mean of 0.5): the score columns hold
        // the end of the killer half and the victim half.
        let around = |rgb| {
            let h = hsv(rgb)[0];
            (h.saturating_sub(12), (h + 12).min(179))
        };
        let only_teal = |range| ClassParams {
            teal_hue: range,
            red_hue_max: 0,
            red_hue_min: 180,
            ..ClassParams::default()
        };
        let ally = bar_means(&classify(&fx, &only_teal(around(palette.ally))), 3);
        let enemy = bar_means(&classify(&fx, &only_teal(around(palette.enemy))), 3);
        for (a, e) in ally.iter().zip(&enemy) {
            assert!(*e >= 0.3 && a + e >= 0.5, "{}: {a} + {e}", palette.name);
        }
    }
}
