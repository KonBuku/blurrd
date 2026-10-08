//! Shared helpers of the detector tests: crops of synthetic frames and the
//! checks of a frame's detections against its ground truth.

#![allow(
    dead_code,
    reason = "each test binary uses a different subset of the helpers"
)]

use blurrd_blur::{PixelParams, PreparedBox, YuvColour, prepare_boxes};
use blurrd_core::{BlurBox, BlurParams, Region};
use blurrd_detect::killfeed::FEED_ROI;
use blurrd_detect::{Detection, KillFeedDetector, RgbView};
use blurrd_testkit::metrics::item_coverage;
use blurrd_testkit::{Fixture, RgbFrame};

/// The [`FEED_ROI`] of a reference-size frame, without copying.
pub(crate) fn feed_view(frame: &RgbFrame) -> RgbView<'_> {
    let stride = 3 * frame.width() as usize;
    let start = FEED_ROI.y0 as usize * stride + 3 * FEED_ROI.x0 as usize;
    RgbView::new(
        &frame.data()[start..],
        FEED_ROI.width(),
        FEED_ROI.height(),
        stride,
    )
    .unwrap()
}

/// The boxes the render would blur for `detections` with the default blur
/// parameters at reference scale.
pub(crate) fn prepared(frame: &RgbFrame, detections: &[Detection]) -> Vec<PreparedBox> {
    let params = PixelParams::new(&BlurParams::default(), 1.0, YuvColour::default()).unwrap();
    let boxes: Vec<BlurBox> = detections
        .iter()
        .map(|d| BlurBox {
            rect: d.rect,
            region: d.region,
            seed: 1,
            grid_origin: (d.text.x0, d.text.y0),
            block_scale: 1.0,
        })
        .collect();
    let mut out = vec![];
    prepare_boxes(&boxes, frame.width(), frame.height(), &params, &mut out);
    out
}

/// The kill-feed detections of one frame, checked against its truth.
#[derive(Debug, Clone, Default)]
pub(crate) struct Check {
    /// Kill-feed names in the truth.
    pub(crate) items: usize,
    /// Rows the detector found.
    pub(crate) rows: usize,
    /// Their tops.
    pub(crate) row_tops: Vec<i32>,
    /// What the detector reported.
    pub(crate) detections: Vec<Detection>,
    /// Names with glyph pixels the boxes do not fully cover.
    pub(crate) leaks: Vec<String>,
    /// Names no detection overlaps.
    pub(crate) missed: Vec<String>,
    /// Detections that overlap no kill-feed name.
    pub(crate) false_positives: Vec<String>,
}

impl Check {
    /// Panics with the details if a name is missed or a detection is false.
    pub(crate) fn assert_found(&self, context: &str) {
        assert!(
            self.missed.is_empty() && self.false_positives.is_empty(),
            "{context}: missed {:?}, false positives {:?}, detections {:?}",
            self.missed,
            self.false_positives,
            self.detections
        );
    }

    /// Panics with the details if a name is missed, a detection is false or
    /// a glyph pixel stays readable.
    pub(crate) fn assert_clean(&self, context: &str) {
        self.assert_found(context);
        assert!(
            self.leaks.is_empty(),
            "{context}: leaks {:?}, detections {:?}",
            self.leaks,
            self.detections
        );
    }
}

/// Runs a fresh detector on a fixture and checks every kill-feed name.
pub(crate) fn check_frame(fx: &Fixture) -> Check {
    check_with(&mut KillFeedDetector::new(), fx)
}

/// Runs `detector` on a fixture and checks every kill-feed name.
pub(crate) fn check_with(detector: &mut KillFeedDetector, fx: &Fixture) -> Check {
    let mut detections = vec![];
    detector
        .detect(&feed_view(&fx.frame), &mut detections)
        .unwrap();
    let boxes = prepared(&fx.frame, &detections);
    let items: Vec<_> = fx.truth.in_region(Region::KillFeed).collect();
    let coverage = item_coverage(&fx.truth, &boxes);
    let leaks = coverage
        .iter()
        .filter(|c| c.region == Region::KillFeed && c.is_leak())
        .map(|c| {
            let item = items.iter().find(|i| i.id == c.id).unwrap();
            format!(
                "{:?} {:?} {} ({} of {} px)",
                item.id, item.kind, item.text, c.leaked_pixels, c.glyph_pixels
            )
        })
        .collect();
    // One-colour rows report both names as victims (port spec 3.3), so
    // only the region has to match.
    let false_positives = detections
        .iter()
        .filter(|d| !items.iter().any(|i| i.text.intersection(&d.text).is_some()))
        .map(|d| format!("{:?} {}", d.kind, d.text))
        .collect();
    let missed = items
        .iter()
        .filter(|i| {
            !detections
                .iter()
                .any(|d| i.text.intersection(&d.text).is_some())
        })
        .map(|i| format!("{:?} {:?} {}", i.id, i.kind, i.text))
        .collect();
    Check {
        items: items.len(),
        rows: detector.rows().len(),
        row_tops: detector.rows().iter().map(|r| r.top).collect(),
        detections,
        leaks,
        missed,
        false_positives,
    }
}
