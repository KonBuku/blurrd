//! Local checks on real recordings from `BLURRD_TEST_MEDIA` (never in CI,
//! never committed). Run with
//! `cargo nextest run -p blurrd-video --release --run-ignored all real_media`.
//!
//! Throughput numbers are printed (`--no-capture`) and compared with the
//! gates of the architecture plan only in release builds.

use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

use blurrd_video::{DecodeOptions, Decoder, PtsTable, probe, pts_table};

/// Clips used by the M6 acceptance: one with dropped frames (VFR) and the
/// 3601-frame benchmark clip of the stack research.
const CLIPS: [&str; 2] = [
    "VALORANT_replay_2024.07.25-15.50.mp4",
    "VALORANT_replay_2024.06.22-11.50.mp4",
];

#[allow(
    clippy::expect_used,
    reason = "test helper: a missing variable is a test failure"
)]
fn media(name: &str) -> PathBuf {
    let dir = std::env::var_os("BLURRD_TEST_MEDIA").expect("set BLURRD_TEST_MEDIA");
    PathBuf::from(dir).join(name)
}

/// `nb_frames` of the video stream as reported by an `ffprobe` on `PATH`.
fn ffprobe_frames(path: &std::path::Path) -> Option<u32> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=nb_frames",
        ])
        .args(["-of", "csv=p=0"])
        .arg(path)
        .output()
        .ok()?;
    String::from_utf8(out.stdout).ok()?.trim().parse().ok()
}

#[test]
#[ignore = "needs BLURRD_TEST_MEDIA"]
fn real_media_decodes_every_frame_in_order() {
    for name in CLIPS {
        let path = media(name);
        let info = probe(&path).unwrap();
        let keyframes = info.keyframes.clone().unwrap();
        let start = Instant::now();
        let mut decoder = Decoder::open(&path, &DecodeOptions::default()).unwrap();
        let mut pts = Vec::new();
        let mut key_indices = Vec::new();
        while let Some(frame) = decoder.next_frame().unwrap() {
            assert_eq!(frame.index() as usize, pts.len());
            if frame.is_key() {
                key_indices.push(frame.index());
            }
            pts.push(frame.pts());
        }
        let elapsed = start.elapsed().as_secs_f64();
        let table = PtsTable::new(info.time_base, pts).unwrap();
        assert_eq!(
            pts_table(&path).unwrap(),
            table,
            "{name}: the container's PTS table is the decoded one"
        );
        let fps = f64::from(table.len()) / elapsed;
        println!(
            "{name}: {} frames, software decode {fps:.0} fps, {} keyframes, vfr {}",
            table.len(),
            keyframes.len(),
            info.is_vfr
        );
        if let Some(expected) = ffprobe_frames(&path) {
            assert_eq!(
                table.len(),
                expected,
                "{name}: frame count vs ffprobe nb_frames"
            );
        }
        assert_eq!(
            key_indices, keyframes,
            "{name}: probe keyframes vs decoded keyframes"
        );
        if !cfg!(debug_assertions) {
            assert!(fps >= 900.0, "{name}: software decode {fps:.0} fps < 900");
        }
    }
}

#[test]
#[ignore = "needs BLURRD_TEST_MEDIA and an NVIDIA GPU"]
fn real_media_nvdec_matches_software_and_is_fast() {
    use blurrd_video::{DecodePurpose, DecoderChoice, HwDecode, PixelLayout};

    let path = media(CLIPS[1]);
    let info = probe(&path).unwrap();
    let decode = |hw: HwDecode| {
        let mut decoder = Decoder::open(&path, &DecodeOptions::with_hw(hw)).unwrap();
        let start = Instant::now();
        let mut pts = Vec::new();
        while let Some(frame) = decoder.next_frame().unwrap() {
            if hw == HwDecode::Nvdec {
                assert_eq!(frame.layout(), PixelLayout::Nv12);
            }
            pts.push(frame.pts());
        }
        let fps = pts.len() as f64 / start.elapsed().as_secs_f64();
        (pts, fps, decoder.hardware_frames())
    };
    let (sw_pts, sw_fps, _) = decode(HwDecode::Software);
    let (hw_pts, hw_fps, hw_frames) = decode(HwDecode::Nvdec);
    println!("software {sw_fps:.0} fps, NVDEC {hw_fps:.0} fps (with download)");
    assert_eq!(
        hw_pts, sw_pts,
        "NVDEC and software must give the same frames"
    );
    assert_eq!(hw_frames as usize, hw_pts.len());
    if !cfg!(debug_assertions) {
        assert!(hw_fps >= 650.0, "NVDEC {hw_fps:.0} fps < 650");
    }

    let cache = std::env::temp_dir().join(format!("blurrd-caps-{}", std::process::id()));
    let first = DecoderChoice::resolve(&info, &cache, DecodePurpose::Throughput);
    let spare = DecoderChoice::resolve(&info, &cache, DecodePurpose::SpareCpu);
    println!("decoder choice: throughput {first:?}, spare CPU {spare:?}");
    assert!(cache.join("decoders.json").is_file());
    assert_eq!(spare, HwDecode::Nvdec);
    std::fs::remove_dir_all(&cache).unwrap();
}

#[test]
#[ignore = "needs BLURRD_TEST_MEDIA and ffmpeg on PATH"]
fn real_media_rgb_matches_ffmpeg_bgr24() {
    use blurrd_core::RectI;
    use blurrd_video::{RgbConverter, RgbImage, VideoColour, YuvRoi};

    const STEP: u32 = 72;
    const FRAMES: u32 = 50;
    let path = media(CLIPS[1]);
    let info = probe(&path).unwrap();
    let (w, h) = (info.width as usize, info.height as usize);
    let reference = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&path)
        // Single quotes protect the comma from the filtergraph parser.
        .args(["-vf", &format!("select='not(mod(n,{STEP}))'")])
        .args(["-fps_mode", "passthrough"])
        .args([
            "-frames:v",
            &FRAMES.to_string(),
            "-f",
            "rawvideo",
            "-pix_fmt",
            "bgr24",
            "-",
        ])
        .output()
        .unwrap()
        .stdout;
    assert_eq!(
        reference.len(),
        w * h * 3 * FRAMES as usize,
        "ffmpeg output size"
    );

    let converter = RgbConverter::new(VideoColour::from_info(&info.color, info.height));
    let full = RectI::new(0, 0, info.width.cast_signed(), info.height.cast_signed()).unwrap();
    let (mut roi, mut rgb) = (YuvRoi::new(), RgbImage::default());
    let mut decoder = Decoder::open(&path, &DecodeOptions::default()).unwrap();
    let (mut max, mut sum, mut count, mut compared) = (0u8, 0u64, 0u64, 0u32);
    while let Some(frame) = decoder.next_frame().unwrap() {
        if frame.index() % STEP != 0 {
            continue;
        }
        frame.roi_yuv(full, &mut roi).unwrap();
        converter.convert(&roi, &mut rgb).unwrap();
        let offset = compared as usize * w * h * 3;
        let expected = &reference[offset..offset + w * h * 3];
        for (ours, theirs) in rgb
            .data
            .as_chunks::<3>()
            .0
            .iter()
            .zip(expected.as_chunks::<3>().0)
        {
            for (a, b) in ours.iter().zip(theirs.iter().rev()) {
                let d = a.abs_diff(*b);
                max = max.max(d);
                sum += u64::from(d);
                count += 1;
            }
        }
        compared += 1;
        if compared == FRAMES {
            break;
        }
    }
    let mean = sum as f64 / count as f64;
    println!("RGB vs ffmpeg bgr24 on {compared} frames: max |d| {max}, mean {mean:.3}");
    assert_eq!(compared, FRAMES);
    // Plan gate: max 3, mean 0.5. The conversion reproduces swscale
    // exactly, so anything above zero means the reference changed.
    assert!(max <= 3 && mean <= 0.5, "max {max}, mean {mean:.3}");
}

#[test]
#[ignore = "needs BLURRD_TEST_MEDIA"]
fn real_media_seeks_fast_to_exact_frames() {
    use std::sync::Arc;
    use std::time::Duration;

    use blurrd_video::{FrameSeeker, HwDecode, SeekerOptions};

    let path = media(CLIPS[1]);
    let mut decoder = Decoder::open(&path, &DecodeOptions::default()).unwrap();
    let mut pts = Vec::new();
    while let Some(frame) = decoder.next_frame().unwrap() {
        pts.push(frame.pts());
    }
    let table = Arc::new(PtsTable::new(decoder.time_base(), pts).unwrap());
    let p95 = |mut v: Vec<Duration>| {
        v.sort();
        v[(v.len() * 95).div_ceil(100) - 1]
    };
    for hw in [HwDecode::Software, HwDecode::Nvdec] {
        let mut options = SeekerOptions::default();
        options.decode = DecodeOptions::with_hw(hw);
        let mut seeker = FrameSeeker::open(&path, Arc::clone(&table), options).unwrap();
        let (mut random, mut cached) = (Vec::new(), Vec::new());
        let mut state = 0x2545_f491_u32;
        for _ in 0..100 {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let index = state % table.len();
            let start = Instant::now();
            let frame = seeker.frame(index).unwrap();
            random.push(start.elapsed());
            assert_eq!(Some(frame.pts()), table.pts(index));
            // Let the prefetch fill the GOP, then step through it.
            std::thread::sleep(Duration::from_millis(60));
            for step in 1..=5 {
                let next = (index + step).min(table.len() - 1);
                let start = Instant::now();
                let frame = seeker.frame(next).unwrap();
                cached.push(start.elapsed());
                assert_eq!(Some(frame.pts()), table.pts(next));
            }
        }
        let (random_p95, cached_max) = (p95(random), cached.iter().max().copied().unwrap());
        println!("{hw:?}: random seek p95 {random_p95:?}, cached GOP max {cached_max:?}");
        if !cfg!(debug_assertions) {
            assert!(
                random_p95 <= Duration::from_millis(150),
                "{hw:?}: random p95 {random_p95:?}"
            );
            assert!(
                cached_max < Duration::from_millis(50),
                "{hw:?}: cached {cached_max:?}"
            );
        }
    }
}
