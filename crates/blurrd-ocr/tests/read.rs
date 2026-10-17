//! Detection and recognition on rendered text (CPU in CI, DirectML
//! locally).

#![allow(
    clippy::cast_possible_wrap,
    clippy::unwrap_used,
    reason = "test helpers on small images: coordinates never wrap, a panic is a failure"
)]

mod support;

use blurrd_core::RectI;
use blurrd_ocr::{Ep, OcrEngine, OcrError, OcrOptions, RgbView, ZoneId, ZoneSpec};
use blurrd_testkit::FontSet;
use support::{Canvas, fold};

const ZONE: ZoneId = ZoneId(7);

fn engine(ep: Ep) -> Option<OcrEngine> {
    let (runtime, models) = support::ocr()?;
    let options = OcrOptions {
        ep,
        zones: vec![ZoneSpec::for_crop(ZONE, 640, 200, 1.5)],
        ..OcrOptions::default()
    };
    Some(OcrEngine::new(&runtime, &models, &options).unwrap())
}

/// Three lines of text in a 640 × 200 "zone": detection finds each line and
/// recognition reads it.
fn reads_lines(ep: Ep) {
    let Some(mut engine) = engine(ep) else {
        return;
    };
    let fonts = FontSet::latin();
    let lines = ["Sentinel Seven", "xX_Nova_Xx 2024", "Mira"];
    let mut canvas = Canvas::new(640, 200);
    let mut bounds = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        bounds.push(canvas.text(&fonts, line, 40 + 90 * i as i32, 20 + 60 * i as i32, 16.0));
    }
    let image = RgbView::new(canvas.width, canvas.height, &canvas.data).unwrap();
    let reads = engine.read(ZONE, image).unwrap();
    assert_eq!(
        reads.len(),
        lines.len(),
        "{:?}",
        reads.iter().map(|r| r.rect).collect::<Vec<_>>()
    );
    for ((read, line), &[x0, y0, x1, y1]) in reads.iter().zip(lines).zip(&bounds) {
        assert_eq!(fold(&read.rec.text), fold(line));
        assert!(read.rec.conf > 0.8, "{}", read.rec.conf);
        assert!(read.det_score > 0.5);
        // The box covers the drawn text.
        let drawn = RectI::new(x0, y0, x1, y1).unwrap();
        assert!(read.rect.iou(&drawn) > 0.4, "{} vs {drawn}", read.rect);
    }
    // No text, no boxes; the probability map stays low.
    let blank = Canvas::new(640, 200);
    let map = engine
        .detect(ZONE, RgbView::new(640, 200, &blank.data).unwrap())
        .unwrap();
    assert_eq!(map.boxes, []);
    assert!(map.prob.max_in(RectI::new(0, 0, 640, 200).unwrap()) < 0.3);
    // The same zone stretched to 4/3 of its width (a 4:3 HUD stretched to
    // 16:9) and marked with its pixel aspect reads the same lines, with
    // boxes in pixels of the stretched image.
    let wide_w = 640 * 4 / 3;
    let wide: Vec<u8> = (0..200)
        .flat_map(|y| {
            let canvas = &canvas;
            (0..wide_w).flat_map(move |x| {
                let i = 3 * (y * 640 + x * 3 / 4) as usize;
                [canvas.data[i], canvas.data[i + 1], canvas.data[i + 2]]
            })
        })
        .collect();
    let stretched = RgbView::new(wide_w, 200, &wide)
        .unwrap()
        .with_pixel_aspect(0.75);
    let reads = engine.read(ZONE, stretched).unwrap();
    assert_eq!(reads.len(), lines.len());
    for ((read, line), [x0, y0, x1, y1]) in reads.iter().zip(lines).zip(&bounds) {
        assert_eq!(fold(&read.rec.text), fold(line));
        let drawn = RectI::new(x0 * 4 / 3, *y0, x1 * 4 / 3, *y1).unwrap();
        assert!(read.rect.iou(&drawn) > 0.4, "{} vs {drawn}", read.rect);
    }
    // Reading several crops at once gives each crop its own reads, the
    // same as reading them one by one.
    let blank_view = RgbView::new(640, 200, &blank.data).unwrap();
    let many = engine
        .read_many(
            &[(ZONE, image), (ZONE, blank_view), (ZONE, stretched)],
            &|_, _| true,
        )
        .unwrap();
    assert_eq!(many.len(), 3);
    assert_eq!(many[0], engine.read(ZONE, image).unwrap());
    assert_eq!(many[1], []);
    assert_eq!(many[2], reads);
    // Only the lines the filter keeps are recognised: here those of the
    // first crop below its first line.
    let lower = engine
        .read_many(&[(ZONE, image), (ZONE, stretched)], &|crop, rect| {
            crop == 0 && rect.y0 > 60
        })
        .unwrap();
    assert_eq!(lower[0], many[0][1..]);
    assert_eq!(lower[1], []);
    assert_stalls_tolerable(&engine);
}

/// Stalls after the warm-up (runs as slow as creating and warming up the
/// session) stay rare; the fixed shapes rule out
/// recompilation by construction.
fn assert_stalls_tolerable(engine: &OcrEngine) {
    for (shape, stats) in engine.stats() {
        assert!(stats.stalls_tolerable(), "{shape:?}: {stats:?}");
    }
}

#[test]
fn cpu_reads_lines() {
    reads_lines(Ep::Cpu);
}

#[test]
#[ignore = "needs a DirectX 12 GPU"]
fn gpu_directml_reads_lines() {
    reads_lines(Ep::DirectMl);
}

/// Crops of every width bucket, more than one batch, in input order.
fn recognizes_batches(ep: Ep) {
    let Some(mut engine) = engine(ep) else {
        return;
    };
    let fonts = FontSet::latin();
    let words = [
        "Nova", "Raze", "Kestrel", "Umbra", "Vortex", "Harbor", "Sage",
    ];
    let texts: Vec<String> = (0..20)
        .map(|i| {
            let word = words[i % words.len()];
            match i % 3 {
                0 => word.to_owned(),
                1 => format!("Player {word} Seven"),
                _ => format!("A Much Longer Line Of Text With {word} On Top"),
            }
        })
        .collect();
    let canvases: Vec<Canvas> = texts
        .iter()
        .map(|t| {
            let mut canvas = Canvas::new(700, 32);
            canvas.text(&fonts, t, 6, 7, 16.0);
            canvas
        })
        .collect();
    let crops: Vec<RgbView<'_>> = canvases
        .iter()
        .zip(&texts)
        .map(|(c, t)| {
            let width =
                (fonts.measure(t, blurrd_testkit::font::TextStyle::cap(16.0)) as u32 + 14).min(700);
            RgbView::new(c.width, c.height, &c.data)
                .unwrap()
                .crop(RectI::new(0, 0, width as i32, 32).unwrap())
                .unwrap()
        })
        .collect();
    let reads = engine.recognize(&crops).unwrap();
    assert_eq!(reads.len(), texts.len());
    for (read, text) in reads.iter().zip(&texts) {
        assert_eq!(fold(&read.text), fold(text));
    }
    let used: Vec<u64> = engine.stats()[1..].iter().map(|(_, s)| s.runs).collect();
    assert!(used.iter().all(|&runs| runs > 0), "{used:?}");
    assert_stalls_tolerable(&engine);
}

#[test]
fn cpu_recognizes_batches() {
    recognizes_batches(Ep::Cpu);
}

#[test]
#[ignore = "needs a DirectX 12 GPU"]
fn gpu_directml_recognizes_batches() {
    recognizes_batches(Ep::DirectMl);
}

#[test]
fn unknown_zone_is_an_error() {
    let Some(mut engine) = engine(Ep::Cpu) else {
        return;
    };
    let canvas = Canvas::new(64, 32);
    let image = RgbView::new(64, 32, &canvas.data).unwrap();
    let error = engine.detect(ZoneId(99), image).unwrap_err();
    assert!(matches!(error, OcrError::InvalidInput(_)), "{error}");
}
