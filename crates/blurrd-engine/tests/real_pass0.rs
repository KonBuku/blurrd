//! Pass 0 on real recordings from `BLURRD_TEST_MEDIA` (local only, never
//! in CI): OCR throughput per protected-zone frame after deduplication,
//! and the kill-feed positive control (names the detector finds that OCR
//! reads and that the dictionary holds). Only counts are printed: no
//! names, no text.
//!
//! ```text
//! cargo nextest run -p blurrd-engine --release --run-ignored all real_ --no-capture
//! ```
//!
//! `BLURRD_PASS0_CLIPS=<n>` limits the run to the first `n` clips of the
//! manifest (default: all), `BLURRD_PASS0_EP=cpu` runs on the CPU and
//! `BLURRD_PASS0_DET_SCALE=<s>` detects at another scale than
//! [`blurrd_engine::dictionary::DET_SCALE`] (how that value was chosen).

#![allow(
    clippy::cast_possible_wrap,
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "local measurement: a missing file or a failed decode is a test failure"
)]

mod support;

use std::time::Instant;

use blurrd_core::{HudGeometry, RectI};
use blurrd_detect::KillFeedDetector;
use blurrd_detect::killfeed::FEED_ROI;
use blurrd_engine::{DictionaryBuilder, ZoneKind};
use blurrd_ocr::{Ep, OcrEngine, OcrOptions, RgbView, ZoneSpec, fold};
use blurrd_testkit::media::{Manifest, MediaDir};
use blurrd_video::{DecodeOptions, Decoder, RgbConverter, RgbImage, VideoColour, YuvRoi, probe};

/// Recognition confidence a crop read must reach to count as read.
const READ_CONF: f32 = 0.8;

#[derive(Debug, Default)]
struct Totals {
    frames: u64,
    zone_crops: u64,
    zone_crops_read: u64,
    ocr_seconds: f64,
    detections: u64,
    crop_reads: u64,
    in_dictionary: u64,
    names: u64,
}

#[test]
#[ignore = "needs BLURRD_TEST_MEDIA, ONNX Runtime, the models and a GPU"]
fn real_pass0_throughput_and_kill_feed_positive_control() {
    let Some(media) = MediaDir::from_env() else {
        eprintln!("skipped: BLURRD_TEST_MEDIA is not set");
        return;
    };
    let Some((runtime, models)) = support::ocr() else {
        return;
    };
    let manifest = Manifest::load_repository().unwrap();
    let limit = std::env::var("BLURRD_PASS0_CLIPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(usize::MAX);
    let ep = match std::env::var("BLURRD_PASS0_EP").as_deref() {
        Ok("cpu") => Ep::Cpu,
        _ => Ep::DirectMl,
    };
    let geometry = HudGeometry::default();
    let builder = DictionaryBuilder::new(&geometry, 1920, 1080);
    let zones = match std::env::var("BLURRD_PASS0_DET_SCALE")
        .ok()
        .and_then(|v| v.parse().ok())
    {
        Some(scale) => builder
            .zones()
            .iter()
            .map(|z| ZoneSpec::for_crop(z.id, z.reference.width(), z.reference.height(), scale))
            .collect(),
        None => builder.zone_specs(),
    };
    let options = OcrOptions {
        ep,
        zones,
        ..OcrOptions::default()
    };
    let mut engine = OcrEngine::new(&runtime, &models, &options).unwrap();
    support::assert_fixed_shapes(&engine, &options.zones);
    let mut all = Totals::default();
    let mut held_out = Totals::default();
    let started = Instant::now();
    for clip in manifest.clips.iter().take(limit) {
        let path = media.clip_path(clip);
        let info = probe(&path).unwrap();
        if (info.width, info.height) != (1920, 1080) {
            eprintln!(
                "{}: {}x{} skipped (1080p only)",
                clip.id, info.width, info.height
            );
            continue;
        }
        let totals = run_clip(&mut engine, &path, &info);
        eprintln!(
            "{}{}: {} frames read, zone crops {}/{} after dedup, {:.0} zone-frames/s, \
             kill feed: {} detections, crop read {:.1} %, in dictionary {:.1} %, {} names",
            clip.id,
            if clip.held_out { " (held out)" } else { "" },
            totals.frames,
            totals.zone_crops_read,
            totals.zone_crops,
            totals.zone_crops_read as f64 / totals.ocr_seconds,
            totals.detections,
            percent(totals.crop_reads, totals.detections),
            percent(totals.in_dictionary, totals.detections),
            totals.names,
        );
        for target in std::iter::once(&mut all).chain(clip.held_out.then_some(&mut held_out)) {
            target.frames += totals.frames;
            target.zone_crops += totals.zone_crops;
            target.zone_crops_read += totals.zone_crops_read;
            target.ocr_seconds += totals.ocr_seconds;
            target.detections += totals.detections;
            target.crop_reads += totals.crop_reads;
            target.in_dictionary += totals.in_dictionary;
            target.names += totals.names;
        }
    }
    eprintln!("wall time {:.1} s", started.elapsed().as_secs_f64());
    report(&engine, &all, &held_out);
}

/// Prints the totals and the session statistics and checks the gates.
fn report(engine: &OcrEngine, all: &Totals, held_out: &Totals) {
    for (label, t) in [("all", all), ("held out", held_out)] {
        if t.zone_crops_read == 0 {
            eprintln!("{label}: no frames");
            continue;
        }
        eprintln!(
            "{label}: {} source frames read, {} of {} zone crops OCR'd ({:.1} % skipped), \
             {:.0} zone-frames/s ({:.2} ms each), kill feed positive control: crop read \
             {:.1} %, in dictionary {:.1} % of {} detections",
            t.frames,
            t.zone_crops_read,
            t.zone_crops,
            100.0 - percent(t.zone_crops_read, t.zone_crops),
            t.zone_crops_read as f64 / t.ocr_seconds,
            1000.0 * t.ocr_seconds / t.zone_crops_read.max(1) as f64,
            percent(t.crop_reads, t.detections),
            percent(t.in_dictionary, t.detections),
            t.detections,
        );
    }
    let stats = engine.stats();
    for (shape, s) in &stats {
        eprintln!(
            "session {shape:?}: created in {:.1} ms, warm-up {:.1} ms, {} runs, \
             mean {:.2} ms, max {:.2} ms, {} stalls (slower than creation + warm-up), total {:.1} s",
            s.create_ms,
            s.warmup_ms,
            s.runs,
            s.mean_ms(),
            s.max_ms,
            s.slow_runs,
            s.total_ms / 1000.0
        );
    }
    // Fixed shapes rule out recompilation by construction (checked after
    // the engine was built). Stalls measure a GPU shared with other work
    // and are reported; only a rate a recompilation would cause fails.
    support::assert_stalls_tolerable(engine);
    // The positive control of plan 11.3: OCR reads the names the detector
    // found on the clean source. Dictionary coverage is reported only: the
    // per-item control of verification (M17) reads every item's own crop.
    assert!(percent(all.crop_reads, all.detections) >= 90.0);
}

fn percent(part: u64, whole: u64) -> f64 {
    100.0 * part as f64 / whole.max(1) as f64
}

/// Pass 0 over one clip (every [`DictionaryBuilder::CADENCE`]th frame) and
/// the positive control of its kill-feed detections.
fn run_clip(
    engine: &mut OcrEngine,
    path: &std::path::Path,
    info: &blurrd_core::MediaInfo,
) -> Totals {
    let mut builder = DictionaryBuilder::new(&HudGeometry::default(), info.width, info.height);
    let converter = RgbConverter::new(VideoColour::from_info(&info.color, info.height));
    let mut decoder = Decoder::open(path, &DecodeOptions::default()).unwrap();
    let zones = builder.zones().to_vec();
    let mut rois: Vec<(YuvRoi, RgbImage)> = zones
        .iter()
        .map(|_| (YuvRoi::new(), RgbImage::default()))
        .collect();
    let mut detector = KillFeedDetector::new();
    let mut detections = Vec::new();
    // Confident reads of detector boxes, checked against the final
    // dictionary at the end (folded text stays in memory).
    let mut reads: Vec<Option<String>> = Vec::new();
    let mut frames = 0;
    while let Some(frame) = decoder.next_frame().unwrap() {
        if !DictionaryBuilder::wants(frame.index(), false) {
            continue;
        }
        frames += 1;
        for (zone, (roi, rgb)) in zones.iter().zip(&mut rois) {
            frame.roi_yuv(zone.rect, roi).unwrap();
            converter.convert(roi, rgb).unwrap();
            // The ROI is aligned outward to even coordinates.
            let aligned = roi.rect().unwrap();
            let view = RgbView::new(rgb.width, rgb.height, &rgb.data).unwrap();
            let crop = view
                .crop(
                    RectI::new(
                        zone.rect.x0 - aligned.x0,
                        zone.rect.y0 - aligned.y0,
                        zone.rect.x1 - aligned.x0,
                        zone.rect.y1 - aligned.y0,
                    )
                    .unwrap(),
                )
                .unwrap();
            builder.push_zone(engine, zone.id, crop).unwrap();
            if zone.kind != ZoneKind::FeedAndBanner {
                continue;
            }
            // The feed zone is FEED_ROI at 1080p: the detector's crop.
            let feed = blurrd_detect::RgbView::new(
                &rgb.data[3
                    * ((zone.rect.y0 - aligned.y0) as usize * rgb.width as usize
                        + (zone.rect.x0 - aligned.x0) as usize)..],
                FEED_ROI.width(),
                FEED_ROI.height(),
                3 * rgb.width as usize,
            )
            .unwrap();
            detector.detect(&feed, &mut detections).unwrap();
            let crops: Vec<RgbView<'_>> = detections
                .iter()
                .filter_map(|d| {
                    let r = d.text;
                    crop.crop(
                        RectI::new(
                            r.x0 - FEED_ROI.x0 - 3,
                            r.y0 - FEED_ROI.y0 - 3,
                            r.x1 - FEED_ROI.x0 + 3,
                            r.y1 - FEED_ROI.y0 + 3,
                        )
                        .ok()?,
                    )
                })
                .collect();
            for read in engine.recognize(&crops).unwrap() {
                reads.push(
                    (read.conf >= READ_CONF && !fold(&read.text).is_empty()).then_some(read.text),
                );
            }
        }
    }
    let stats = builder.stats();
    let dictionary = builder.dictionary();
    let crop_reads = reads.iter().flatten().count() as u64;
    let in_dictionary = reads
        .iter()
        .flatten()
        .filter(|text| support::was_read(dictionary, text))
        .count() as u64;
    Totals {
        frames,
        zone_crops: stats.zone_crops,
        zone_crops_read: stats.zone_crops_read,
        ocr_seconds: stats.ocr_seconds,
        detections: reads.len() as u64,
        crop_reads,
        in_dictionary,
        names: dictionary.len() as u64,
    }
}
