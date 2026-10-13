//! The kill-feed detector on synthetic frames from blurrd-testkit: every
//! name of every settled row is found, and nothing else is.

#![allow(
    clippy::cast_possible_wrap,
    clippy::unwrap_used,
    reason = "test helpers on small frames: coordinates never wrap, a panic is a failure"
)]

mod support;

use blurrd_core::{NameKind, Region};
use blurrd_detect::KillFeedDetector;
use blurrd_testkit::hud::killfeed::KillFeedSpec;
use blurrd_testkit::{Background, FontSet, Layout, Palette, Rng, Scene};

use support::{feed_view, random_kill, settled_feed};

#[test]
fn finds_every_settled_name() {
    let backgrounds = [
        Background::noise(3),
        Background::dusk(),
        Background::white_sky(),
    ];
    let mut names = 0;
    for seed in 0..24u64 {
        let rows = 1 + (seed % 8) as u32;
        let background = backgrounds[(seed % 3) as usize];
        let check = settled_feed(seed, rows, background, Palette::DEFAULT);
        check.assert_clean(&format!("seed {seed}, {rows} rows"));
        assert_eq!(check.rows, rows as usize, "seed {seed}: rows");
        names += check.items;
    }
    assert!(names >= 200, "{names} names checked");
}

#[test]
fn walls_in_a_bar_colour_add_no_rows() {
    // A red or teal wall behind the feed passes the bar-colour score; the
    // wall guard keeps only rows with a text line or a victim portrait.
    for (i, background) in [Background::red_wall(5), Background::teal_wall(6)]
        .into_iter()
        .enumerate()
    {
        for rows in [1, 3, 6] {
            let check = settled_feed(300 + i as u64, rows, background, Palette::DEFAULT);
            let context = format!("wall {i}, {rows} rows");
            check.assert_clean(&context);
            assert_eq!(check.rows, rows as usize, "{context}: {:?}", check.row_tops);
        }
        let empty = Scene::new(Layout::reference(), background, Palette::DEFAULT, 1).render(0);
        let mut detector = KillFeedDetector::new();
        detector
            .detect(&feed_view(&empty.frame), &mut vec![])
            .unwrap();
        assert!(
            detector.rows().is_empty(),
            "wall {i}: {:?}",
            detector.rows()
        );
    }
}

#[test]
fn reports_rows_top_to_bottom_with_their_names() {
    let check = settled_feed(7, 5, Background::noise(1), Palette::DEFAULT);
    check.assert_found("five rows");
    let tops: Vec<i32> = check.row_tops.clone();
    assert_eq!(tops.len(), 5);
    for (i, top) in tops.iter().enumerate() {
        // The text-line anchor follows the tallest glyph at the right end
        // of the victim name, so it may sit a few pixels off the bar.
        let truth = 96 + 39 * i as i32;
        assert!((top - truth).abs() <= 3, "row {i}: {top} for {truth}");
    }
    for d in &check.detections {
        assert_eq!(d.region, Region::KillFeed);
        assert!(matches!(d.kind, NameKind::Killer | NameKind::Victim));
        let top = d.row_top.unwrap();
        assert!(tops.contains(&top));
        assert_eq!(d.rect.y0, top + 5);
        assert_eq!(d.rect.y1, (top + 28).max(d.text.y1 + 7));
        assert_eq!((d.rect.x0, d.rect.x1), (d.text.x0 - 5, d.text.x1 + 5));
    }
}

#[test]
fn an_empty_feed_has_no_rows_and_no_names() {
    let scene = Scene::new(
        Layout::reference(),
        Background::noise(9),
        Palette::DEFAULT,
        1,
    );
    let fx = scene.render(0);
    let mut detector = KillFeedDetector::new();
    let mut out = vec![];
    detector.detect(&feed_view(&fx.frame), &mut out).unwrap();
    assert!(out.is_empty() && detector.rows().is_empty());
}

#[test]
fn rejects_crops_of_another_size() {
    let fx = Scene::new(
        Layout::reference(),
        Background::noise(9),
        Palette::DEFAULT,
        1,
    )
    .render(0);
    let full = blurrd_detect::RgbView::new(
        fx.frame.data(),
        fx.frame.width(),
        fx.frame.height(),
        3 * fx.frame.width() as usize,
    )
    .unwrap();
    let err = KillFeedDetector::new()
        .detect(&full, &mut vec![])
        .unwrap_err();
    assert!(err.to_string().contains("1260 x 332"), "{err}");
}

#[test]
fn reusing_a_detector_gives_the_same_result() {
    let fonts = FontSet::latin();
    let mut rng = Rng::new(42);
    let kills = (0..6).map(|i| random_kill(&mut rng, i)).collect();
    let scene = Scene::new(
        Layout::reference(),
        Background::dusk(),
        Palette::DEFAULT,
        30,
    )
    .with_kill_feed(&fonts, KillFeedSpec::new(kills));
    let mut reused = KillFeedDetector::new();
    for frame in [12, 0, 25, 12, 3] {
        let fx = scene.render(frame);
        let view = feed_view(&fx.frame);
        let (mut a, mut b) = (vec![], vec![]);
        reused.detect(&view, &mut a).unwrap();
        KillFeedDetector::new().detect(&view, &mut b).unwrap();
        assert_eq!(a, b, "frame {frame}");
    }
}

#[test]
fn finds_victim_names_that_end_in_a_deep_descender() {
    // Some glyphs of real names (Cyrillic ф in a real recording) reach
    // from above the x-height to 27 px below the row top, deeper than the
    // synthetic font's descenders. Replace the victim name's last glyph
    // with such a stroke: the name must still end at the right alignment,
    // or the whole row reports nothing.
    const TOP: i32 = 96;
    let fonts = FontSet::latin();
    for seed in 0..8u64 {
        let mut rng = Rng::new(900 + seed);
        let kill = random_kill(&mut rng, 0);
        let scene = Scene::new(Layout::reference(), Background::dusk(), Palette::DEFAULT, 8)
            .with_kill_feed(&fonts, KillFeedSpec::new(vec![kill]));
        let mut fx = scene.render(5);
        let victim = fx
            .truth
            .in_region(Region::KillFeed)
            .find(|i| i.kind == NameKind::Victim)
            .map(|i| i.text)
            .unwrap();
        // Clear the last glyph and a gap before it, then draw the new one.
        let glyph = victim.x1 - 9..victim.x1 - 1;
        for x in glyph.start - 2..glyph.end {
            let bar = fx.frame.pixel(x, TOP + 5).unwrap();
            for y in TOP + 8..TOP + 29 {
                fx.frame.set(x, y, bar);
            }
        }
        // A ф: a two-pixel stem from above the x-height to + 27 through
        // a ring on the line.
        let white = [255, 255, 255];
        let stem = glyph.start + 3;
        for y in TOP + 11..TOP + 27 {
            fx.frame.set(stem, y, white);
            fx.frame.set(stem + 1, y, white);
        }
        for y in TOP + 14..TOP + 23 {
            let edge = !(TOP + 16..TOP + 21).contains(&y);
            for x in glyph.clone() {
                if edge || x <= glyph.start + 1 || x >= glyph.end - 2 {
                    fx.frame.set(x, y, white);
                }
            }
        }
        let check = support::check_frame(&fx);
        check.assert_clean(&format!("seed {seed}"));
        check.assert_found(&format!("seed {seed}"));
    }
}
