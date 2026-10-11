//! The kill-feed detector does not depend on the default team colours:
//! colour-blind palettes and the colour filters of GPU drivers change the
//! bars, and every settled name must still be found and fully covered.

#![allow(
    clippy::unwrap_used,
    reason = "test helpers on small frames: a panic is a failure"
)]

mod support;

use blurrd_testkit::{Background, Palette};

use blurrd_detect::KillFeedDetector;
use blurrd_detect::killfeed::BarEvidence;

use support::{Filter, check_frame, check_with, settled_feed, settled_fixture};

/// Backgrounds for the robustness checks: textured, dark and bright.
const BACKGROUNDS: [Background; 3] = [
    Background::noise(3),
    Background::dusk(),
    Background::white_sky(),
];

/// Filters with realistic strengths: hue shifts, desaturated and vivid
/// colours down to black and white (NVIDIA Freestyle's Black & White
/// filter, saturation sliders near 0), Digital Vibrance, dark and bright
/// gamma, washed-out, darkened and punchy levels, and a warm colour
/// balance tint of the shadows and highlights. The rendered name
/// strokes are sharper than recorded ones, so only the darkest levels push
/// them below the tuned text thresholds (recordings cross them from a
/// brightness offset of about −0.10); that case fails without the
/// white-level adjustment.
const FILTERS: [Filter; 16] = [
    Filter::Hue(40.0),
    Filter::Hue(120.0),
    Filter::Hue(-90.0),
    Filter::Saturation(0.0),
    Filter::Saturation(0.1),
    Filter::Saturation(0.4),
    Filter::Saturation(1.8),
    Filter::Vibrance(1.0),
    Filter::Gamma(0.8),
    Filter::Gamma(1.3),
    Filter::Gamma(1.5),
    Filter::Levels {
        brightness: 0.06,
        contrast: 0.85,
    },
    Filter::Levels {
        brightness: -0.10,
        contrast: 1.0,
    },
    Filter::Levels {
        brightness: -0.12,
        contrast: 0.85,
    },
    Filter::Levels {
        brightness: -0.3,
        contrast: 0.9,
    },
    Filter::ColourBalance {
        shadows: [0.3, -0.1, -0.3],
        midtones: [0.0; 3],
        highlights: [0.2, 0.0, -0.2],
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
    assert!(names >= 800, "{names} names checked");
}

#[test]
fn rows_report_what_showed_the_bar() {
    // Default colours, learned colours, and luminance alone in black and
    // white video, which is the low-confidence case.
    let cases = [
        (Palette::DEFAULT, None, BarEvidence::DefaultColour),
        (Palette::TRITANOPIA, None, BarEvidence::LearnedColour),
        (
            Palette::DEFAULT,
            Some(Filter::Saturation(0.0)),
            BarEvidence::Luminance,
        ),
    ];
    for (palette, filter, evidence) in cases {
        let fx = settled_fixture(300, 4, Background::noise(5), palette);
        let fx = filter.map_or_else(|| fx.clone(), |f| f.fixture(&fx));
        let mut detector = KillFeedDetector::new();
        let context = format!("{}, {filter:?}", palette.name);
        check_with(&mut detector, &fx).assert_clean(&context);
        let rows = detector.rows();
        assert_eq!(rows.len(), 4, "{context}");
        assert!(
            rows.iter().all(|r| r.evidence == evidence),
            "{context}: {rows:?}"
        );
        let low = evidence == BarEvidence::Luminance;
        assert!(
            rows.iter().all(|r| r.is_low_confidence() == low),
            "{context}"
        );
    }
}

#[test]
fn filters_keep_white_text_and_change_the_bars() {
    // The filters are strong enough to move the bars out of the default
    // classes, and keep the names white (the detector's hue-free anchor;
    // darkening filters lower the white level the thresholds follow).
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
