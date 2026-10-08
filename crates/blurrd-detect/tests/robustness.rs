//! The kill-feed detector does not depend on the default team colours:
//! colour-blind palettes and the colour filters of GPU drivers change the
//! bars, and every settled name must still be found and fully covered.

#![allow(
    clippy::unwrap_used,
    reason = "test helpers on small frames: a panic is a failure"
)]

mod support;

use blurrd_testkit::{Background, Palette};

use support::{Filter, check_frame, settled_feed, settled_fixture};

/// Backgrounds for the robustness checks: textured, dark and bright.
const BACKGROUNDS: [Background; 3] = [
    Background::noise(3),
    Background::dusk(),
    Background::white_sky(),
];

/// Filters with realistic strengths: hue shifts, desaturated and vivid
/// colours, Digital Vibrance, dark and bright gamma, washed-out and punchy
/// levels.
const FILTERS: [Filter; 9] = [
    Filter::Hue(40.0),
    Filter::Hue(120.0),
    Filter::Hue(-90.0),
    Filter::Saturation(0.4),
    Filter::Saturation(1.8),
    Filter::Vibrance(1.0),
    Filter::Gamma(0.8),
    Filter::Gamma(1.3),
    Filter::Levels {
        brightness: 0.06,
        contrast: 0.85,
    },
];

#[test]
fn colour_blind_palettes_leave_no_name_readable() {
    let mut names = 0;
    for palette in &Palette::ALL[1..] {
        for seed in 0..16u64 {
            let rows = 1 + (seed % 8) as u32;
            let background = BACKGROUNDS[(seed % 3) as usize];
            let check = settled_feed(100 + seed, rows, background, *palette);
            let context = format!("{}, seed {seed}, {rows} rows", palette.name);
            check.assert_clean(&context);
            assert_eq!(check.rows, rows as usize, "{context}: rows");
            names += check.items;
        }
    }
    assert!(names >= 300, "{names} names checked");
}

#[test]
fn gpu_colour_filters_leave_no_name_readable() {
    let mut names = 0;
    for (f, filter) in FILTERS.iter().enumerate() {
        for palette in [Palette::DEFAULT, Palette::DEUTERANOPIA] {
            for seed in 0..6u64 {
                let rows = 2 + ((seed + f as u64) % 7) as u32;
                let background = BACKGROUNDS[((seed + f as u64) % 3) as usize];
                let fx = settled_fixture(200 + seed, rows, background, palette);
                let check = check_frame(&filter.fixture(&fx));
                let context = format!("{filter:?}, {}, seed {seed}, {rows} rows", palette.name);
                check.assert_clean(&context);
                assert_eq!(check.rows, rows as usize, "{context}: rows");
                names += check.items;
            }
        }
    }
    assert!(names >= 500, "{names} names checked");
}

#[test]
fn filters_keep_white_text_and_change_the_bars() {
    // The filters are strong enough to move the bars out of the default
    // classes, and keep the names white (the detector's colour-free
    // anchor).
    let teal = [100, 186, 162];
    let white = [245, 245, 245];
    let params = blurrd_detect::ClassParams::default();
    let bar = blurrd_detect::Class::Bar.bit();
    for filter in [Filter::Hue(120.0), Filter::Saturation(0.4)] {
        let filtered = filter_pixel(filter, teal);
        assert_eq!(params.colour_flags(filtered) & bar, 0, "{filter:?}");
        let text = filter_pixel(filter, white);
        assert!(text.iter().all(|&c| c >= 235), "{filter:?}: {text:?}");
    }
}

/// The filtered colour of one pixel.
fn filter_pixel(filter: Filter, rgb: [u8; 3]) -> [u8; 3] {
    let frame = blurrd_testkit::RgbFrame::filled(1920, 1080, rgb);
    filter.frame(&frame).pixel(1800, 200).unwrap()
}
