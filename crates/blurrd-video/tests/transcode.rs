//! Rendering synthetic clips: the output keeps the source's frame count,
//! timestamps, keyframes and (normalised) colour tags; mismatches and
//! cancellation leave no file behind. CI runs the software test encoder;
//! the `gpu_` variants use NVENC locally.

mod support;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use blurrd_blur::CpuBlur;
use blurrd_core::{BlurBox, EncoderSetting, FrameBoxTable, NvencSessions, RectI};
use blurrd_video::{
    BoxSource, DecodeOptions, EncodeOptions, MuxOptions, PtsTable, TranscodeReport, Transcoder,
    VideoError, output_color, probe,
};
use support::{ClipSpec, TempClip, box_table, decode_table, pictures, pixel_params, write_clip};

/// Renders `source` into `output` with `encoder`, one box on frames
/// `boxes` (none when the range is empty).
fn render(
    source: &Path,
    output: &Path,
    encoder: &str,
    boxes: (u32, u32),
    cancel_after: Option<u32>,
) -> Result<TranscodeReport, VideoError> {
    let mut options = EncodeOptions::default();
    options.encoder = EncoderSetting::Named(encoder.to_owned());
    render_with(source, output, &options, boxes, cancel_after)
}

/// Renders `source` into `output` with `options`, one box on frames
/// `boxes` (none when the range is empty), canceling after
/// `cancel_after` frames if given.
#[allow(
    clippy::unwrap_used,
    reason = "test helper: an error is a test failure"
)]
fn render_with(
    source: &Path,
    output: &Path,
    options: &EncodeOptions,
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
    let transcoder = Transcoder::open(
        &info,
        Arc::new(pts),
        output,
        &DecodeOptions::default(),
        options,
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
fn two_sessions_stitch_into_the_same_video() {
    // 600 frames with a keyframe every 30: about five segments.
    let spec = ClipSpec {
        gop: 30,
        ..ClipSpec::cfr(600)
    };
    // Matroska like the outputs, so timestamps compare exactly (Matroska
    // stores milliseconds).
    let source = TempClip::new("render-sessions", "mkv");
    write_clip(source.path(), &spec);
    let mut options = EncodeOptions::default();
    // Lossless and intra-only: the pictures must not depend on how the
    // frames were split between sessions.
    options.encoder = EncoderSetting::Named(String::from("ffv1"));
    options.sessions = NvencSessions::One;
    let one = TempClip::new("render-sessions-1", "mkv");
    let single = render_with(source.path(), one.path(), &options, (100, 420), None).unwrap();
    options.sessions = NvencSessions::Two;
    let two = TempClip::new("render-sessions-2", "mkv");
    let split = render_with(source.path(), two.path(), &options, (100, 420), None).unwrap();
    assert_eq!((single.sessions, split.sessions), (1, 2));
    assert_eq!(split.blurred_frames, 321);
    let (split_pictures, single_pictures) = (pictures(two.path()), pictures(one.path()));
    assert_eq!(split_pictures.len(), single_pictures.len(), "frame count");
    let differing: Vec<usize> = (0..split_pictures.len())
        .filter(|&i| split_pictures[i] != single_pictures[i])
        .collect();
    assert_eq!(differing, Vec::<usize>::new(), "frames with other pictures");
    let (source_pts, _) = decode_table(source.path());
    let (split_pts, _) = decode_table(two.path());
    assert_eq!(rescaled(&split_pts, &source_pts), source_pts.as_slice());
    // An inter-coded test encoder keeps the source's keyframes too.
    options.encoder = EncoderSetting::Named(String::from("mpeg4"));
    let inter = TempClip::new("render-sessions-3", "mkv");
    let report = render_with(source.path(), inter.path(), &options, (0, 599), None).unwrap();
    assert_eq!(report.sessions, 2);
    assert_same_timeline(source.path(), inter.path());
}

/// Renders `source` into `output` like [`render_with`], taking the frames
/// outside `changed` from the earlier output `previous`.
#[allow(
    clippy::unwrap_used,
    reason = "test helper: an error is a test failure"
)]
fn render_reusing(
    source: &Path,
    output: &Path,
    options: &EncodeOptions,
    boxes: (u32, u32),
    previous: &Path,
    changed: std::ops::Range<u32>,
) -> TranscodeReport {
    let info = probe(source).unwrap();
    let (pts, _) = decode_table(source);
    let table = box_table(
        pts.len(),
        boxes.0,
        boxes.1,
        RectI::new(64, 64, 160, 96).unwrap(),
    );
    let mut transcoder = Transcoder::open(
        &info,
        Arc::new(pts),
        output,
        &DecodeOptions::default(),
        options,
        MuxOptions::default(),
    )
    .unwrap();
    transcoder.reuse_previous(previous, std::slice::from_ref(&changed));
    transcoder
        .run(&table, &CpuBlur, &pixel_params(&info), &|| false, |_| {})
        .unwrap()
}

#[test]
fn a_render_again_encodes_only_the_changed_gops() {
    // 600 frames with a keyframe every 30.
    let spec = ClipSpec {
        gop: 30,
        ..ClipSpec::cfr(600)
    };
    let source = TempClip::new("render-reuse", "mkv");
    write_clip(source.path(), &spec);
    let mut options = EncodeOptions::default();
    // Lossless and intra-only, so whole renders and partial ones must
    // show the same pictures.
    options.encoder = EncoderSetting::Named(String::from("ffv1"));
    for sessions in [NvencSessions::One, NvencSessions::Two] {
        options.sessions = sessions;
        let earlier = TempClip::new("render-reuse-1", "mkv");
        render_with(source.path(), earlier.path(), &options, (100, 420), None).unwrap();
        // The box now lasts 20 frames longer: only the GOP 420..450 is
        // encoded again.
        let again = TempClip::new("render-reuse-2", "mkv");
        let report = render_reusing(
            source.path(),
            again.path(),
            &options,
            (100, 440),
            earlier.path(),
            421..441,
        );
        assert_eq!((report.frames, report.reused_frames), (600, 570));
        let whole = TempClip::new("render-reuse-3", "mkv");
        render_with(source.path(), whole.path(), &options, (100, 440), None).unwrap();
        assert!(pictures(again.path()) == pictures(whole.path()), "pictures");
        let (source_pts, _) = decode_table(source.path());
        let (again_pts, _) = decode_table(again.path());
        assert_eq!(rescaled(&again_pts, &source_pts), source_pts.as_slice());
    }
    // An inter-coded encoder: the copied parts start at the source's
    // keyframes, which are keyframes of both renders.
    options.encoder = EncoderSetting::Named(String::from("mpeg4"));
    options.sessions = NvencSessions::One;
    let earlier = TempClip::new("render-reuse-4", "mkv");
    render_with(source.path(), earlier.path(), &options, (100, 420), None).unwrap();
    let again = TempClip::new("render-reuse-5", "mkv");
    let report = render_reusing(
        source.path(),
        again.path(),
        &options,
        (100, 440),
        earlier.path(),
        421..441,
    );
    assert_eq!(report.reused_frames, 570);
    assert_same_timeline(source.path(), again.path());
    // An earlier output of another encoder is not used.
    options.encoder = EncoderSetting::Named(String::from("ffv1"));
    let other = TempClip::new("render-reuse-6", "mkv");
    let report = render_reusing(
        source.path(),
        other.path(),
        &options,
        (100, 440),
        earlier.path(),
        421..441,
    );
    assert_eq!(report.reused_frames, 0);
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

/// A box source whose producer stopped at frame `end`.
struct StoppedAt {
    table: FrameBoxTable,
    end: u32,
}

impl BoxSource for StoppedAt {
    fn frame_count(&self) -> u32 {
        self.table.frame_count()
    }

    fn boxes_into(&self, frame: u32, out: &mut Vec<BlurBox>) -> Result<(), VideoError> {
        if frame >= self.end {
            return Err(VideoError::Canceled);
        }
        self.table.boxes_into(frame, out)
    }
}

#[test]
fn a_box_source_that_stops_stops_the_render() {
    let source = TempClip::new("render-source-stop", "mp4");
    write_clip(source.path(), &ClipSpec::cfr(60));
    let output = TempClip::new("render-source-stop-out", "mp4");
    let info = probe(source.path()).unwrap();
    let (pts, _) = decode_table(source.path());
    let boxes = StoppedAt {
        table: box_table(pts.len(), 5, 40, RectI::new(64, 64, 160, 96).unwrap()),
        end: 30,
    };
    let mut options = EncodeOptions::default();
    options.encoder = EncoderSetting::Named(String::from("mpeg4"));
    let transcoder = Transcoder::open(
        &info,
        Arc::new(pts),
        output.path(),
        &DecodeOptions::default(),
        &options,
        MuxOptions::default(),
    )
    .unwrap();
    let error = transcoder
        .run(&boxes, &CpuBlur, &pixel_params(&info), &|| false, |_| {})
        .unwrap_err();
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

#[test]
#[ignore = "needs an NVIDIA GPU"]
fn gpu_nvenc_render_again_copies_h264_into_mp4() {
    let spec = ClipSpec {
        width: 640,
        height: 360,
        gop: 30,
        ..ClipSpec::cfr(300)
    };
    let source = TempClip::new("render-nvenc-reuse", "mp4");
    write_clip(source.path(), &spec);
    let mut options = EncodeOptions::default();
    options.encoder = EncoderSetting::Named(String::from("h264_nvenc"));
    let earlier = TempClip::new("render-nvenc-reuse-1", "mp4");
    render_with(source.path(), earlier.path(), &options, (30, 70), None).unwrap();
    let again = TempClip::new("render-nvenc-reuse-2", "mp4");
    let report = render_reusing(
        source.path(),
        again.path(),
        &options,
        (30, 100),
        earlier.path(),
        71..101,
    );
    // The GOPs 60..90 and 90..120 are encoded again.
    assert_eq!(report.reused_frames, 240);
    assert_same_timeline(source.path(), again.path());
    assert_eq!(pictures(again.path()).len(), 300);
}

#[test]
#[ignore = "needs an NVIDIA GPU"]
fn gpu_two_nvenc_sessions_keep_the_timeline() {
    let spec = ClipSpec {
        width: 640,
        height: 360,
        gop: 60,
        ..ClipSpec::cfr(900)
    };
    let source = TempClip::new("render-nvenc2", "mp4");
    write_clip(source.path(), &spec);
    let output = TempClip::new("render-nvenc2-out", "mp4");
    let report = render(source.path(), output.path(), "h264_nvenc", (100, 700), None).unwrap();
    assert_eq!(report.sessions, 2, "H.264 NVENC uses two sessions");
    assert_same_timeline(source.path(), output.path());
}
