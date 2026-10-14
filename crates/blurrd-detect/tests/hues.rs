//! Kill-feed bar hues learned per video (plan 7.3, M15): colour-blind
//! palettes and GPU colour filters learn their two team colours from the
//! rows of a few frames, the learned colours become the detector's
//! default classes, and every name is still found and covered.

#![allow(
    clippy::unwrap_used,
    reason = "test helpers on small frames: a panic is a failure"
)]

mod support;

use blurrd_detect::KillFeedDetector;
use blurrd_detect::classes::ClassParams;
use blurrd_detect::killfeed::{BarEvidence, BarHueLearner, BarHues};
use blurrd_testkit::{Background, Palette};
use support::{Filter, check_with, feed_view, settled_fixture};

/// Rows of each learning frame.
const ROWS: u32 = 6;

/// Learns the hues of `palette` under `filter` from four frames of six
/// rows each.
fn learn(palette: Palette, filter: Option<Filter>) -> Option<BarHues> {
    let mut detector = KillFeedDetector::new();
    let mut learner = BarHueLearner::new();
    let mut dets = Vec::new();
    for seed in 0..4 {
        let fx = settled_fixture(100 + seed, ROWS, Background::dusk(), palette);
        let fx = filter.map_or(fx.clone(), |f| f.fixture(&fx));
        detector.detect(&feed_view(&fx.frame), &mut dets).unwrap();
        learner.push(&detector);
    }
    assert!(
        learner.rows() >= 20,
        "{palette:?} {filter:?}: {} rows",
        learner.rows()
    );
    learner.hues(&ClassParams::default())
}

/// The learned classes find every name of new frames, as default-colour
/// rows (the fast path).
fn assert_learned_path(hues: BarHues, palette: Palette, filter: Option<Filter>) {
    let mut detector = KillFeedDetector::with_params(hues.params(&ClassParams::default()));
    for seed in 0..3 {
        let fx = settled_fixture(200 + seed, ROWS, Background::dusk(), palette);
        let fx = filter.map_or(fx.clone(), |f| f.fixture(&fx));
        let what = format!("{palette:?} {filter:?} seed {seed}");
        check_with(&mut detector, &fx).assert_clean(&what);
        let rows = detector.rows();
        let fast = rows
            .iter()
            .filter(|r| r.evidence == BarEvidence::DefaultColour)
            .count();
        assert!(
            fast * 10 >= rows.len() * 9,
            "{what}: {fast} of {} rows on the learned default path",
            rows.len()
        );
    }
}

#[test]
fn the_default_palette_learns_nothing() {
    assert_eq!(learn(Palette::DEFAULT, None), None);
    // A mild saturation change keeps the default colours.
    assert_eq!(learn(Palette::DEFAULT, Some(Filter::Saturation(1.4))), None);
}

#[test]
fn colour_blind_palettes_learn_their_team_colours() {
    for palette in Palette::ALL.into_iter().skip(1) {
        let hues = learn(palette, None).unwrap_or_else(|| panic!("{palette:?}: nothing learned"));
        assert_learned_path(hues, palette, None);
    }
}

#[test]
fn colour_filters_learn_the_shifted_colours() {
    let filters = [
        Filter::Hue(120.0),
        Filter::Hue(-90.0),
        Filter::ColourBalance {
            shadows: [0.3, -0.1, -0.3],
            midtones: [0.0; 3],
            highlights: [0.2, 0.0, -0.2],
        },
    ];
    for filter in filters {
        for palette in [Palette::DEFAULT, Palette::DEUTERANOPIA] {
            if let Some(hues) = learn(palette, Some(filter)) {
                assert_learned_path(hues, palette, Some(filter));
            } else {
                // A filter that leaves both colours in the default ranges
                // needs nothing learned; the defaults still read it.
                let fx = settled_fixture(300, ROWS, Background::dusk(), palette);
                let fx = filter.fixture(&fx);
                let what = format!("{palette:?} {filter:?} (defaults)");
                check_with(&mut KillFeedDetector::new(), &fx).assert_clean(&what);
            }
        }
    }
}
