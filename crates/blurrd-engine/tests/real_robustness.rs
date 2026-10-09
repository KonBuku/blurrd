//! Pass 0 on augmented real recordings (local only, never in CI): the
//! dictionary built from a filtered or rescaled copy of a clip must hold
//! the names of the dictionary built from the unchanged clip.
//!
//! `BLURRD_PASS0_AUGMENTED` names a folder of excerpts made with FFmpeg,
//! `<clip>__<variant>.mp4`, where variant `none` is the unchanged excerpt
//! and every other variant (colour filters such as hue, saturation,
//! vibrance, gamma, levels and sharpening; rescales to 720p, 1440p and
//! 2160p; a 4:3 squeeze with pillarbox bars (`pillar`) and 16:10 padding
//! (`pad1610`)) is compared with it. The HUD geometry follows the frame
//! size and the variant. Only counts are printed: no names, no text.
//!
//! Every variant at the reference height or above must keep 90 % of the
//! names. Variants below it (720p) are reported only: their threshold
//! comes with the normalised sub-1080p geometry of milestone M15, as for
//! the detectors (plan section 18.14).
//!
//! ```text
//! cargo nextest run -p blurrd-engine --release --run-ignored all real_augmented --no-capture
//! ```

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "local measurement: a missing file or a failed decode is a test failure; bar widths are small positive pixel counts"
)]

mod support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use blurrd_core::{HudGeometry, HudLayout, RectI};
use blurrd_engine::DictionaryBuilder;
use blurrd_ocr::{Ep, NameDictionary, OcrEngine, OcrOptions, RgbView};
use blurrd_video::{DecodeOptions, Decoder, RgbConverter, RgbImage, VideoColour, YuvRoi, probe};

/// The HUD geometry of an excerpt of `width` × `height` pixels made from a
/// 16:9 recording by `variant` (what calibration would find):
///
/// - `pillar`: squeezed to 4:3 and pillarboxed back to 16:9 (the HUD is
///   scaled by 0.75 horizontally between the bars);
/// - `pad1610`: padded to 16:10 with equal bars above and below;
/// - anything else: 16:9 at any size.
fn geometry(variant: &str, width: u32, height: u32) -> HudGeometry {
    match variant {
        "pillar" => {
            let content = height as f32 * 4.0 / 3.0;
            let bar = (width as f32 - content) / 2.0;
            HudGeometry {
                sx: content / 1920.0,
                sy: height as f32 / 1080.0,
                anchor_left: bar,
                anchor_right: bar + content,
                anchor_center: width as f32 / 2.0,
                layout: HudLayout::Pillarbox {
                    bar_px: bar.round() as u16,
                },
                ..HudGeometry::default()
            }
        }
        "pad1610" => {
            let scale = width as f32 / 1920.0;
            HudGeometry {
                sx: scale,
                sy: scale,
                anchor_left: 0.0,
                anchor_right: width as f32,
                anchor_center: width as f32 / 2.0,
                oy: (height as f32 - 1080.0 * scale) / 2.0,
                ..HudGeometry::default()
            }
        }
        _ => {
            let scale = height as f32 / 1080.0;
            HudGeometry {
                sx: scale,
                sy: scale,
                anchor_left: 0.0,
                anchor_right: width as f32,
                anchor_center: width as f32 / 2.0,
                layout: HudLayout::Wide16x9,
                ..HudGeometry::default()
            }
        }
    }
}

/// Builds the dictionary of one file (every
/// [`DictionaryBuilder::CADENCE`]th frame); returns it with the frame
/// height of the file.
fn dictionary(engine: &mut OcrEngine, path: &Path, variant: &str) -> (NameDictionary, u32) {
    let info = probe(path).unwrap();
    let geometry = geometry(variant, info.width, info.height);
    let mut builder = DictionaryBuilder::new(&geometry, info.width, info.height);
    let converter = RgbConverter::new(VideoColour::from_info(&info.color, info.height));
    let mut decoder = Decoder::open(path, &DecodeOptions::default()).unwrap();
    let zones = builder.zones().to_vec();
    let mut roi = YuvRoi::new();
    let mut rgb = RgbImage::default();
    while let Some(frame) = decoder.next_frame().unwrap() {
        if !DictionaryBuilder::wants(frame.index(), false) {
            continue;
        }
        for zone in &zones {
            frame.roi_yuv(zone.rect, &mut roi).unwrap();
            converter.convert(&roi, &mut rgb).unwrap();
            // The ROI is aligned outward to even coordinates.
            let aligned = roi.rect().unwrap();
            let crop = RgbView::new(rgb.width, rgb.height, &rgb.data)
                .unwrap()
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
        }
    }
    (builder.finish().0, info.height)
}

#[test]
#[ignore = "needs BLURRD_PASS0_AUGMENTED, ONNX Runtime, the models and a GPU"]
fn real_augmented_dictionaries_keep_the_names() {
    let Some(dir) = std::env::var_os("BLURRD_PASS0_AUGMENTED").map(PathBuf::from) else {
        eprintln!("skipped: BLURRD_PASS0_AUGMENTED is not set");
        return;
    };
    let Some((runtime, models)) = support::ocr() else {
        return;
    };
    // clip -> variant -> file
    let mut clips: BTreeMap<String, BTreeMap<String, PathBuf>> = BTreeMap::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
        if let Some((clip, variant)) = stem.split_once("__") {
            clips
                .entry(clip.to_owned())
                .or_default()
                .insert(variant.to_owned(), path.clone());
        }
    }
    // Zone sizes depend only on the reference geometry, so one engine
    // serves every resolution.
    let options = OcrOptions {
        ep: Ep::DirectMl,
        zones: DictionaryBuilder::new(&HudGeometry::default(), 1920, 1080).zone_specs(),
        ..OcrOptions::default()
    };
    let mut engine = OcrEngine::new(&runtime, &models, &options).unwrap();
    // variant -> (names kept, names in the unchanged dictionaries, lowest
    // frame height)
    let mut kept: BTreeMap<String, (usize, usize, u32)> = BTreeMap::new();
    for (clip, variants) in &clips {
        let (reference, _) = dictionary(&mut engine, &variants["none"], "none");
        // Names read at least twice: single reads include misreads that
        // no other copy repeats.
        let names: Vec<&str> = reference
            .iter()
            .filter(|(_, stats)| stats.count >= 2)
            .map(|(name, _)| name)
            .collect();
        for (variant, path) in variants.iter().filter(|(v, _)| *v != "none") {
            let (other, height) = dictionary(&mut engine, path, variant);
            let found = names
                .iter()
                .filter(|n| support::was_read(&other, n))
                .count();
            eprintln!(
                "{clip} {variant}: {found}/{} names kept ({:.1} %)",
                names.len(),
                100.0 * found as f32 / names.len().max(1) as f32
            );
            let entry = kept.entry(variant.clone()).or_insert((0, 0, height));
            entry.0 += found;
            entry.1 += names.len();
            entry.2 = entry.2.min(height);
        }
    }
    let mut failed = Vec::new();
    for (variant, (found, total, height)) in &kept {
        let below = (*found as f32) < 0.9 * *total as f32;
        let checked = *height >= 1080;
        eprintln!(
            "{variant}: {found}/{total} names kept ({:.1} %){}",
            100.0 * *found as f32 / (*total).max(1) as f32,
            if checked {
                ""
            } else {
                "  (below 1080p: reported only)"
            }
        );
        if below && checked {
            failed.push(format!("{variant}: {found}/{total}"));
        }
    }
    assert!(failed.is_empty(), "below 90 %: {failed:?}");
}
