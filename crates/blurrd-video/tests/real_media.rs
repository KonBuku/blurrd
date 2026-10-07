//! Local checks on real recordings from `BLURRD_TEST_MEDIA` (never in CI,
//! never committed). Run with
//! `cargo nextest run -p blurrd-video --release --run-ignored all real_media`.
//!
//! Throughput numbers are printed (`--no-capture`) and compared with the
//! gates of the architecture plan only in release builds.

use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

use blurrd_video::{DecodeOptions, Decoder, PtsTable, probe};

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
