//! Rendering synthetic clips: the output keeps the source's frame count,
//! timestamps, keyframes and (normalised) colour tags; mismatches and
//! cancellation leave no file behind. CI runs the software test encoder;
//! the `gpu_` variants use NVENC locally.

mod support;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use blurrd_blur::CpuBlur;
use blurrd_core::{EncoderSetting, RectI};
use blurrd_video::{
    DecodeOptions, EncodeOptions, MuxOptions, PtsTable, TranscodeReport, Transcoder, VideoError,
    output_color, probe,
};
use support::{ClipSpec, TempClip, box_table, decode_table, pixel_params, write_clip};

/// Renders `source` into `output` with `encoder`, one box on frames
/// `boxes` (none when the range is empty).
#[allow(
    clippy::unwrap_used,
    reason = "test helper: an error is a test failure"
)]
fn render(
    source: &Path,
    output: &Path,
    encoder: &str,
    boxes: (u32, u32),
    cancel_after: Option<u32>,
) -> Result<TranscodeReport, VideoError> {
    let info = probe(source).unwrap();
    let (pts, _) = decode_table(source);
    let table = box_table(
        pts.len(),
        boxes.0,
        boxes.1,
        RectI::new(64, 64, 160, 96).unwrap(),
    );
    let mut options = EncodeOptions::default();
    options.encoder = EncoderSetting::Named(encoder.to_owned());
    let transcoder = Transcoder::open(
        &info,
        Arc::new(pts),
        output,
        &DecodeOptions::default(),
        &options,
        MuxOptions::default(),
    )?;
    let sent = AtomicU32::new(0);
    let cancel = || cancel_after.is_some_and(|n| sent.fetch_add(1, Ordering::Relaxed) >= n);
    let mut last = 0;
    let report = transcoder.run(&table, &CpuBlur, &pixel_params(&info), &cancel, |n| {
        assert_eq!(n, last + 1, "progress counts every frame");
        last = n;
    })?;
    Ok(report)
}

/// Files in the clip's folder whose name starts with `stem`.
#[allow(
    clippy::unwrap_used,
    reason = "test helper: an error is a test failure"
)]
fn leftovers(output: &Path) -> Vec<PathBuf> {
    let stem = output.file_stem().unwrap().to_string_lossy().into_owned();
    std::fs::read_dir(output.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(&stem)
        })
        .collect()
}

/// Checks that `output` has the frames, PTS, keyframes and normalised
/// colour tags of `source`.
#[allow(
    clippy::unwrap_used,
    reason = "test helper: an error is a test failure"
)]
fn assert_same_timeline(source: &Path, output: &Path) {
    let (source_pts, source_keys) = decode_table(source);
    let (output_pts, output_keys) = decode_table(output);
    assert_eq!(output_pts.len(), source_pts.len(), "frame count");
    assert_eq!(
        rescaled(&output_pts, &source_pts),
        source_pts.as_slice(),
        "PTS list"
    );
    assert_eq!(output_keys, source_keys, "keyframes");
    let (input, out) = (probe(source).unwrap(), probe(output).unwrap());
    assert_eq!(out.keyframes, input.keyframes, "probed keyframes");
    assert_eq!(
        out.color,
        output_color(&input.color, input.height),
        "colour tags"
    );
}

/// `output` PTS in the time base of `source` (containers may store a
/// multiple of the source's time scale).
#[allow(
    clippy::unwrap_used,
    reason = "test helper: an error is a test failure"
)]
fn rescaled(output: &PtsTable, source: &PtsTable) -> Vec<i64> {
    let (o, s) = (output.time_base(), source.time_base());
    output
        .as_slice()
        .iter()
        .map(|&p| {
            let num = i128::from(p) * i128::from(o.num) * i128::from(s.den);
            let den = i128::from(o.den) * i128::from(s.num);
            i64::try_from((num + den / 2).div_euclid(den)).unwrap()
        })
        .collect()
}

#[test]
fn renders_with_the_source_timeline() {
    let source = TempClip::new("render-src", "mp4");
    write_clip(source.path(), &ClipSpec::cfr(90));
    let output = TempClip::new("render-out", "mp4");
    let report = render(source.path(), output.path(), "mpeg4", (10, 19), None).unwrap();
    assert_eq!(report.frames, 90);
    assert_eq!(report.blurred_frames, 10);
    assert_eq!(report.encoder, "mpeg4");
    assert_same_timeline(source.path(), output.path());
    assert_eq!(leftovers(output.path()), [output.path().to_path_buf()]);
}

#[test]
fn renders_variable_rate_and_b_frame_sources() {
    // Dropped frames (variable rate) and B-frames in the source; the output
    // keeps every timestamp.
    let mut pts: Vec<i64> = (0..120).collect();
    pts.retain(|p| p % 17 != 5);
    let spec = ClipSpec {
        time_base: (1, 600),
        pts: pts.iter().map(|p| p * 10).collect(),
        b_frames: 2,
        gop: 24,
        ..ClipSpec::cfr(0)
    };
    for ext in ["mp4", "mkv"] {
        let source = TempClip::new("render-vfr", ext);
        write_clip(source.path(), &spec);
        let output = TempClip::new("render-vfr-out", ext);
        render(source.path(), output.path(), "mpeg4", (0, 5), None).unwrap();
        assert_same_timeline(source.path(), output.path());
    }
}

#[test]
fn renders_edit_list_sources() {
    let cases = [
        // Presentation starts 10 frames after the media start.
        ClipSpec {
            pts: (10..100).collect(),
            ..ClipSpec::cfr(0)
        },
        // The edit cuts the last frame.
        ClipSpec::cfr(40),
        // Edit list with B-frames and a variable rate.
        ClipSpec {
            time_base: (1, 1000),
            pts: (0..96)
                .scan(0, |t, i| {
                    let now = *t;
                    *t += [16, 17, 17, 33, 50][i % 5];
                    Some(now)
                })
                .collect(),
            b_frames: 2,
            gop: 24,
            ..ClipSpec::cfr(0)
        },
    ];
    for spec in cases {
        let source = TempClip::new("render-elst", "mp4");
        write_clip(source.path(), &spec);
        assert!(probe(source.path()).unwrap().has_edit_list);
        let output = TempClip::new("render-elst-out", "mp4");
        render(source.path(), output.path(), "mpeg4", (0, 3), None).unwrap();
        assert_same_timeline(source.path(), output.path());
    }
}

#[test]
fn a_changed_video_is_rejected_without_output() {
    let source = TempClip::new("render-mismatch", "mp4");
    write_clip(source.path(), &ClipSpec::cfr(30));
    let output = TempClip::new("render-mismatch-out", "mp4");
    let info = probe(source.path()).unwrap();
    let (pts, _) = decode_table(source.path());
    let mut shifted = pts.as_slice().to_vec();
    shifted[20] += 1;
    let wrong = PtsTable::new(pts.time_base(), shifted).unwrap();
    let mut options = EncodeOptions::default();
    options.encoder = EncoderSetting::Named(String::from("mpeg4"));
    let transcoder = Transcoder::open(
        &info,
        Arc::new(wrong),
        output.path(),
        &DecodeOptions::default(),
        &options,
        MuxOptions::default(),
    )
    .unwrap();
    let error = transcoder
        .run(
            &box_table(30, 1, 0, RectI::new(0, 0, 2, 2).unwrap()),
            &CpuBlur,
            &pixel_params(&info),
            &|| false,
            |_| {},
        )
        .unwrap_err();
    assert!(
        matches!(error, VideoError::PtsMismatch { index: 20, .. }),
        "{error}"
    );
    assert_eq!(leftovers(output.path()), Vec::<PathBuf>::new());
}

#[test]
fn canceling_leaves_no_file() {
    let source = TempClip::new("render-cancel", "mp4");
    write_clip(source.path(), &ClipSpec::cfr(60));
    let output = TempClip::new("render-cancel-out", "mp4");
    let error = render(source.path(), output.path(), "mpeg4", (1, 0), Some(20)).unwrap_err();
    assert!(matches!(error, VideoError::Canceled), "{error}");
    assert_eq!(leftovers(output.path()), Vec::<PathBuf>::new());
}

#[test]
fn never_writes_over_the_input() {
    let source = TempClip::new("render-self", "mp4");
    write_clip(source.path(), &ClipSpec::cfr(5));
    let error = render(source.path(), source.path(), "mpeg4", (1, 0), None).unwrap_err();
    assert!(matches!(error, VideoError::Unsupported(_)), "{error}");
    assert!(probe(source.path()).is_ok(), "the input is intact");
}

#[test]
#[ignore = "needs an NVIDIA GPU"]
fn gpu_nvenc_renders_with_the_source_timeline() {
    let spec = ClipSpec {
        width: 640,
        height: 360,
        ..ClipSpec::cfr(150)
    };
    let source = TempClip::new("render-nvenc", "mp4");
    write_clip(source.path(), &spec);
    let output = TempClip::new("render-nvenc-out", "mp4");
    let report = render(source.path(), output.path(), "h264_nvenc", (30, 70), None).unwrap();
    assert_eq!(report.blurred_frames, 41);
    assert_same_timeline(source.path(), output.path());
}
