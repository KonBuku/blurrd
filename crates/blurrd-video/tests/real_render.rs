//! Render throughput and output checks on real recordings from
//! `BLURRD_TEST_MEDIA` (never in CI, never committed). Run with
//! `cargo nextest run -p blurrd-video --release --run-ignored all real_render --no-capture`.
//!
//! Throughput gates of the architecture plan (section 11.4) are asserted in
//! release builds only.

mod support;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use blurrd_blur::CpuBlur;
use blurrd_core::{
    BlurPlan, Codec, EncoderSetting, FrameBoxTable, NvencSessions, Override, RectI, Region,
    RegionSet,
};
use blurrd_video::{
    DecodeOptions, EncodeOptions, MuxOptions, PtsTable, TranscodeReport, Transcoder, probe,
};
use support::{decode_table, pixel_params};

/// The 3601-frame 1080p60 H.264 benchmark clip of the stack research.
const CLIP: &str = "VALORANT_replay_2024.06.22-11.50.mp4";

#[allow(
    clippy::expect_used,
    reason = "test helper: a missing variable is a test failure"
)]
fn media(name: &str) -> PathBuf {
    let dir = std::env::var_os("BLURRD_TEST_MEDIA").expect("set BLURRD_TEST_MEDIA");
    PathBuf::from(dir).join(name)
}

/// Sixteen 160×30 boxes in the kill-feed area on every frame: more than a
/// real kill feed ever shows, so the numbers are a worst case.
#[allow(
    clippy::unwrap_used,
    reason = "test helper: an error is a test failure"
)]
fn sixteen_boxes(frames: u32) -> FrameBoxTable {
    let overrides = (0..16u8)
        .map(|i| Override::Add {
            uid: uuid::Uuid::from_u128(u128::from(i) + 1),
            region: Region::Manual,
            f0: 0,
            f1: frames - 1,
            rect: RectI::new(
                1500 - i32::from(i % 2) * 200,
                100 + i32::from(i / 2) * 38,
                1660 - i32::from(i % 2) * 200,
                130 + i32::from(i / 2) * 38,
            )
            .unwrap(),
        })
        .collect();
    BlurPlan {
        frame_count: frames,
        tracks: Vec::new(),
        overrides,
    }
    .frame_table(&RegionSet::all())
}

/// Renders the clip and returns the report and frames per second.
#[allow(
    clippy::unwrap_used,
    reason = "test helper: an error is a test failure"
)]
fn render(
    source: &Path,
    pts: &Arc<PtsTable>,
    table: &FrameBoxTable,
    options: &EncodeOptions,
    output: &Path,
) -> (TranscodeReport, f64) {
    let info = probe(source).unwrap();
    let transcoder = Transcoder::open(
        &info,
        Arc::clone(pts),
        output,
        &DecodeOptions::default(),
        options,
        MuxOptions::default(),
    )
    .unwrap();
    let report = transcoder
        .run(table, &CpuBlur, &pixel_params(&info), &|| false, |_| {})
        .unwrap();
    let fps = f64::from(report.frames) / report.elapsed.as_secs_f64();
    (report, fps)
}

#[test]
#[ignore = "needs BLURRD_TEST_MEDIA and an NVIDIA GPU"]
fn real_render_h264_nvenc_throughput() {
    let source = media(CLIP);
    let (pts, keys) = decode_table(&source);
    let pts = Arc::new(pts);
    let output =
        std::env::temp_dir().join(format!("blurrd-real-render-{}.mp4", std::process::id()));
    let boxes = sixteen_boxes(pts.len());
    let none = BlurPlan {
        frame_count: pts.len(),
        tracks: Vec::new(),
        overrides: Vec::new(),
    }
    .frame_table(&RegionSet::all());
    // Plan gates (section 11.4): one session at least 450 fps, two at
    // least 800 fps.
    for (sessions, gate) in [(NvencSessions::One, 450.0), (NvencSessions::Two, 800.0)] {
        let mut options = EncodeOptions::default();
        options.encoder = EncoderSetting::Named(String::from("h264_nvenc"));
        options.sessions = sessions;
        for (label, table) in [("no boxes", &none), ("16 boxes per frame", &boxes)] {
            let (report, fps) = render(&source, &pts, table, &options, &output);
            let per_frame =
                report.blur_time.as_secs_f64() * 1000.0 / f64::from(report.blurred_frames.max(1));
            println!(
                "{CLIP}: {} session(s), {label}: {} frames, {fps:.0} fps ({:?}), copy+blur \
                 {per_frame:.3} ms per blurred frame",
                report.sessions, report.frames, report.elapsed
            );
            let (out_pts, out_keys) = decode_table(&output);
            assert_eq!(out_pts.as_slice(), pts.as_slice(), "PTS list");
            assert_eq!(out_keys, keys, "keyframes");
            // Free the disk right away (each output is ~350 MB).
            std::fs::remove_file(&output).unwrap();
            if !cfg!(debug_assertions) {
                assert!(fps >= gate, "{label}: {fps:.0} fps < {gate}");
                // The blur cost gate (0.4 ms including the copy) is measured
                // with one session; with two, both decoders load every core
                // and the same work takes longer in wall time.
                if report.sessions == 1 {
                    assert!(
                        per_frame <= 0.4,
                        "{label}: copy+blur {per_frame:.3} ms > 0.4 ms"
                    );
                }
            }
        }
    }
}

#[test]
#[ignore = "needs BLURRD_TEST_MEDIA and an NVIDIA GPU"]
fn real_render_hevc_split_encode_throughput() {
    let source = media(CLIP);
    let (pts, keys) = decode_table(&source);
    let pts = Arc::new(pts);
    let output = std::env::temp_dir().join(format!("blurrd-real-hevc-{}.mp4", std::process::id()));
    let mut options = EncodeOptions::default();
    options.codec = Codec::Hevc;
    let (report, fps) = render(&source, &pts, &sixteen_boxes(pts.len()), &options, &output);
    println!(
        "{CLIP}: HEVC, {} session(s), split-frame encoding, 16 boxes per frame: {} frames, \
         {fps:.0} fps ({:?}), encoder {}",
        report.sessions, report.frames, report.elapsed, report.encoder
    );
    let (out_pts, out_keys) = decode_table(&output);
    std::fs::remove_file(&output).unwrap();
    assert_eq!(report.encoder, "hevc_nvenc");
    assert_eq!(out_pts.as_slice(), pts.as_slice(), "PTS list");
    assert_eq!(out_keys, keys, "keyframes");
    if !cfg!(debug_assertions) {
        // Plan gate (section 11.4): HEVC split-frame encoding at 650 fps.
        assert!(fps >= 650.0, "HEVC split encode {fps:.0} fps < 650");
    }
}

#[test]
#[ignore = "needs BLURRD_TEST_MEDIA and an NVIDIA GPU"]
fn real_render_copies_audio_and_reserves_the_index() {
    use blurrd_video::MoovLayout;
    use ffmpeg_the_third::media::Type;
    use support::{stream_packets, top_level_boxes};

    let source = media(CLIP);
    let (pts, _) = decode_table(&source);
    let pts = Arc::new(pts);
    let output =
        std::env::temp_dir().join(format!("blurrd-real-streams-{}.mp4", std::process::id()));
    let options = EncodeOptions::default();
    let (report, _) = render(&source, &pts, &sixteen_boxes(pts.len()), &options, &output);
    let source_audio = stream_packets(&source, Type::Audio);
    let output_audio = stream_packets(&output, Type::Audio);
    let boxes = top_level_boxes(&output);
    let out_info = probe(&output).unwrap();
    let source_info = probe(&source).unwrap();
    std::fs::remove_file(&output).unwrap();
    println!(
        "{CLIP}: encoder {} ({} sessions), {} audio packets, index {:?}, boxes {boxes:?}",
        report.encoder,
        report.sessions,
        output_audio.len(),
        report.moov
    );
    assert_eq!(report.encoder, "h264_nvenc", "Auto picks NVENC");
    assert_eq!(output_audio.len(), source_audio.len(), "audio packet count");
    assert!(
        output_audio == source_audio,
        "audio packets are bit-identical"
    );
    assert!(matches!(report.moov, MoovLayout::Reserved { .. }));
    assert_eq!(boxes[..2], ["ftyp", "moov"], "index before the media data");
    assert_eq!(
        out_info.color,
        blurrd_video::output_color(&source_info.color, source_info.height),
        "normalised colour tags"
    );
}
