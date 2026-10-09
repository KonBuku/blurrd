//! Model benchmark (local only): the bundled PP-OCRv5 mobile models against
//! the PP-OCRv6 small models and the PP-OCRv5 per-script recognisers in
//! `models/candidates/` (`cargo xtask fetch-models --candidates`).
//!
//! Every model reads the same synthetic names, drawn at the HUD's text size
//! (capitals 11 px tall, as at 1080p) on a dark background: recognition on
//! tight crops per script, and detection plus recognition on zone-sized
//! images with several lines. Prints exact-read rates and run times; the
//! choice is recorded in `models.toml`.
//!
//! ```text
//! cargo nextest run -p blurrd-ocr --release --run-ignored all bench_ --no-capture
//! ```
//!
//! `BLURRD_BENCH_EP=cpu` runs on the CPU instead of DirectML.

#![allow(
    clippy::cast_possible_wrap,
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "local measurement: a missing file is a test failure"
)]

mod support;

use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Instant;

use blurrd_core::RectI;
use blurrd_ocr::{Ep, ModelSet, OcrEngine, OcrOptions, RgbView, ZoneId, ZoneSpec, fold};
use blurrd_testkit::font::{Script, TextStyle};
use blurrd_testkit::{FontSet, NameStyle, Rng, synth_name};
use support::Canvas;

/// Capital height of HUD names at 1080p, in pixels.
const CAP_PX: f32 = 11.0;
/// Names per style.
const PER_STYLE: usize = 120;
/// The zone of the detection benchmark: a kill-feed-sized image detected
/// at twice its size, as pass 0 does.
const ZONE: ZoneId = ZoneId(0);

/// Names drawn from a fixed character list, for scripts the testkit has
/// no style for and for the frequent subset of large scripts.
#[derive(Debug, Clone, Copy)]
struct CharStyle {
    label: &'static str,
    chars: &'static str,
    /// Shortest name, in characters.
    min_len: u32,
    /// Number of possible lengths, starting at `min_len`.
    lengths: u32,
}

/// Cyrillic names (the testkit has no Cyrillic style).
const CYRILLIC: CharStyle = CharStyle {
    label: "Cyrillic",
    chars: "абвгдежзийклмнопрстуфхцчшщыэюяАБВГДЕЖЗИКЛМНОПРСТУФХЦЧШЭЮЯ",
    min_len: 4,
    lengths: 8,
};

/// Hangul names from frequent syllables. The testkit's Hangul style draws
/// uniformly from all 11,172 precomposed syllables, almost all of which
/// never occur in real text; at HUD size every recogniser reads them as a
/// similar frequent syllable (U+C50F as U+C313), so that style only
/// measures out-of-vocabulary syllables.
const HANGUL_COMMON: CharStyle = CharStyle {
    label: "HangulCommon",
    chars: "김이박최정강조윤장임한오서신권황안송류홍민수지현영준연우진성호은희경하도유재상태동혁기원나래아름빛별솔결",
    min_len: 2,
    lengths: 3,
};

/// Japanese names from kana only (the testkit's style also draws rare
/// kanji, which the same out-of-vocabulary effect hides).
const KANA: CharStyle = CharStyle {
    label: "Kana",
    chars: "あいうえおかきくけこさしすせそたちつてとなにぬねのはひふへほまみむめもやゆよらりるれろわをんアイウエオカキクケコサシスセソタチツテトナニヌネノハヒフヘホマミムメモヤユヨラリルレロワン",
    min_len: 3,
    lengths: 4,
};

/// A recogniser to compare: label, model file and character list, both in
/// `models/` or `models/candidates/`.
struct Rec {
    label: &'static str,
    model: &'static str,
    dict: &'static str,
}

const BUNDLED_DET: &str = "ch_PP-OCRv5_det_mobile.onnx";

const RECOGNISERS: [Rec; 8] = [
    Rec {
        label: "v5 mobile (bundled)",
        model: "ch_PP-OCRv5_rec_mobile.onnx",
        dict: "ppocrv5_dict.txt",
    },
    Rec {
        label: "v6 small",
        model: "candidates/PP-OCRv6_rec_small.onnx",
        dict: "candidates/ppocrv6_dict.txt",
    },
    Rec {
        label: "v5 latin",
        model: "candidates/latin_PP-OCRv5_rec_mobile.onnx",
        dict: "candidates/ppocrv5_latin_dict.txt",
    },
    Rec {
        label: "v5 korean",
        model: "candidates/korean_PP-OCRv5_rec_mobile.onnx",
        dict: "candidates/ppocrv5_korean_dict.txt",
    },
    Rec {
        label: "v5 cyrillic",
        model: "candidates/cyrillic_PP-OCRv5_rec_mobile.onnx",
        dict: "candidates/ppocrv5_cyrillic_dict.txt",
    },
    Rec {
        label: "v5 arabic",
        model: "candidates/arabic_PP-OCRv5_rec_mobile.onnx",
        dict: "candidates/ppocrv5_arabic_dict.txt",
    },
    Rec {
        label: "v5 thai",
        model: "candidates/th_PP-OCRv5_rec_mobile.onnx",
        dict: "candidates/ppocrv5_th_dict.txt",
    },
    Rec {
        label: "v5 devanagari",
        model: "candidates/devanagari_PP-OCRv5_rec_mobile.onnx",
        dict: "candidates/ppocrv5_devanagari_dict.txt",
    },
];

/// One style of names in the benchmark.
#[derive(Debug, Clone, Copy)]
enum Style {
    Name(NameStyle),
    Chars(CharStyle),
}

impl Style {
    fn label(self) -> String {
        match self {
            Self::Name(style) => format!("{style:?}"),
            Self::Chars(style) => style.label.to_owned(),
        }
    }

    fn name(self, rng: &mut Rng) -> String {
        match self {
            Self::Name(style) => synth_name(rng, style),
            Self::Chars(style) => {
                let chars: Vec<char> = style.chars.chars().collect();
                let len = style.min_len + rng.below(style.lengths);
                (0..len).map(|_| *rng.pick(&chars)).collect()
            }
        }
    }
}

fn styles() -> Vec<Style> {
    let mut styles: Vec<Style> = NameStyle::LATIN_MIX.into_iter().map(Style::Name).collect();
    styles.push(Style::Chars(CYRILLIC));
    styles.extend(
        [
            Script::Hangul,
            Script::Japanese,
            Script::Thai,
            Script::Arabic,
            Script::Devanagari,
        ]
        .map(|s| Style::Name(NameStyle::Script(s))),
    );
    styles.extend([Style::Chars(HANGUL_COMMON), Style::Chars(KANA)]);
    styles
}

fn model_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("BLURRD_MODEL_DIR")?);
    if dir.join("candidates").is_dir() {
        Some(dir)
    } else {
        eprintln!("skipped: run `cargo xtask fetch-models --candidates` first");
        None
    }
}

fn ep() -> Ep {
    match std::env::var("BLURRD_BENCH_EP").as_deref() {
        Ok("cpu") => Ep::Cpu,
        _ => Ep::DirectMl,
    }
}

/// A rendered name and its tight crop (3 px margin, as around a detector
/// box).
struct Sample {
    truth: String,
    canvas: Canvas,
    rect: RectI,
}

fn samples(fonts: &FontSet, style: Style, seed: u64) -> Vec<Sample> {
    let mut rng = Rng::new(seed);
    let mut out = Vec::new();
    while out.len() < PER_STYLE {
        let truth = style.name(&mut rng);
        if !fonts.covers(&truth) {
            continue;
        }
        let width = fonts.measure(&truth, TextStyle::cap(CAP_PX)).ceil() as i32 + 12;
        let mut canvas = Canvas::new(width.max(16) as u32 + 8, 28);
        let [x0, y0, x1, y1] = canvas.text(fonts, &truth, 6, 7, CAP_PX);
        let rect = RectI::new(x0 - 3, y0 - 3, x1 + 3, y1 + 3).unwrap();
        out.push(Sample {
            truth,
            canvas,
            rect,
        });
    }
    out
}

#[test]
#[ignore = "needs the candidate models; local benchmark"]
fn bench_recognisers_per_script() {
    let Some((runtime, _)) = support::ocr() else {
        return;
    };
    let Some(dir) = model_dir() else {
        return;
    };
    let fonts = FontSet::with_fetched().expect("cargo xtask fetch-fonts");
    let styles = styles();
    let sets: Vec<(Style, Vec<Sample>)> = styles
        .iter()
        .enumerate()
        .map(|(i, &s)| (s, samples(&fonts, s, 1000 + i as u64)))
        .collect();
    for rec in &RECOGNISERS {
        let models = ModelSet::from_files(
            &dir.join(BUNDLED_DET),
            &dir.join(rec.model),
            &dir.join(rec.dict),
        )
        .unwrap();
        let options = OcrOptions {
            ep: ep(),
            zones: vec![ZoneSpec::for_crop(ZONE, 64, 32, 1.0)],
            ..OcrOptions::default()
        };
        let mut engine = OcrEngine::new(&runtime, &models, &options).unwrap();
        let mut line = String::new();
        let started = Instant::now();
        let mut crops_total = 0usize;
        for (style, set) in &sets {
            let crops: Vec<RgbView<'_>> = set
                .iter()
                .map(|s| {
                    RgbView::new(s.canvas.width, s.canvas.height, &s.canvas.data)
                        .unwrap()
                        .crop(s.rect)
                        .unwrap()
                })
                .collect();
            crops_total += crops.len();
            let reads = engine.recognize(&crops).unwrap();
            let exact = reads
                .iter()
                .zip(set)
                .filter(|(r, s)| fold(&r.text) == fold(&s.truth))
                .count();
            // Writing into a String cannot fail.
            let _ = write!(
                line,
                " {}={:.0}%",
                style.label(),
                100.0 * exact as f32 / set.len() as f32
            );
        }
        let ms = started.elapsed().as_secs_f64() * 1000.0 / crops_total as f64;
        let rec_ms: Vec<String> = engine.stats()[1..]
            .iter()
            .map(|(shape, s)| format!("{}x{} {:.2} ms", shape[0], shape[3], s.mean_ms()))
            .collect();
        eprintln!(
            "{}: {ms:.2} ms/crop, batches [{}];{line}",
            rec.label,
            rec_ms.join(", ")
        );
    }
}

/// Lines of names stacked like a kill feed, detected and read whole.
#[test]
#[ignore = "needs the candidate models; local benchmark"]
fn bench_detection_and_recognition() {
    let Some((runtime, _)) = support::ocr() else {
        return;
    };
    let Some(dir) = model_dir() else {
        return;
    };
    let fonts = FontSet::latin();
    let configs: [(&str, &str, &str, &str); 2] = [
        (
            "v5 mobile (bundled)",
            BUNDLED_DET,
            "ch_PP-OCRv5_rec_mobile.onnx",
            "ppocrv5_dict.txt",
        ),
        (
            "v6 small",
            "candidates/PP-OCRv6_det_small.onnx",
            "candidates/PP-OCRv6_rec_small.onnx",
            "candidates/ppocrv6_dict.txt",
        ),
    ];
    let (zone_w, zone_h) = (1260u32, 332u32);
    let mut rng = Rng::new(77);
    let images: Vec<(Canvas, Vec<String>)> = (0..40)
        .map(|_| {
            let mut canvas = Canvas::new(zone_w, zone_h);
            let mut names = Vec::new();
            for row in 0..8 {
                let style = *rng.pick(&NameStyle::LATIN_MIX);
                let (a, b) = (synth_name(&mut rng, style), synth_name(&mut rng, style));
                let y = 12 + row * 40;
                let wb = fonts.measure(&b, TextStyle::cap(CAP_PX)) as i32;
                canvas.text(&fonts, &a, 700, y, CAP_PX);
                canvas.text(&fonts, &b, zone_w as i32 - 30 - wb, y, CAP_PX);
                names.push(a);
                names.push(b);
            }
            (canvas, names)
        })
        .collect();
    for (label, det, rec, dict) in configs {
        let models = ModelSet::from_files(&dir.join(det), &dir.join(rec), &dir.join(dict)).unwrap();
        let options = OcrOptions {
            ep: ep(),
            zones: vec![ZoneSpec::for_crop(ZONE, zone_w, zone_h, 2.0)],
            ..OcrOptions::default()
        };
        let mut engine = OcrEngine::new(&runtime, &models, &options).unwrap();
        let (mut found, mut total) = (0, 0);
        let started = Instant::now();
        for (canvas, names) in &images {
            let image = RgbView::new(canvas.width, canvas.height, &canvas.data).unwrap();
            let reads = engine.read(ZONE, image).unwrap();
            let read: Vec<String> = reads.iter().map(|r| fold(&r.rec.text)).collect();
            total += names.len();
            found += names
                .iter()
                .filter(|n| read.iter().any(|r| r == &fold(n)))
                .count();
        }
        let ms = started.elapsed().as_secs_f64() * 1000.0 / images.len() as f64;
        let det_stats = engine.stats()[0].1;
        eprintln!(
            "{label}: {found}/{total} names read exactly ({:.1} %), {ms:.1} ms per zone \
             (detection {:.2} ms, session created in {:.0} ms)",
            100.0 * found as f32 / total as f32,
            det_stats.mean_ms(),
            det_stats.create_ms
        );
    }
}
