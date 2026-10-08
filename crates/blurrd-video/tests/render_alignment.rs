//! Which output frames a box changes, checked bit for bit.
//!
//! The renders use the lossless `ffv1` test encoder, so every output
//! picture is exactly the picture the encoder received and any difference
//! to the source is the blur:
//!
//! - **P-frame corruption:** a box on one inter-coded frame changes that
//!   frame only, and every later frame of its GOP decodes exactly like the
//!   source. Blurring the decoder's own frame instead of a copy would
//!   corrupt the frames decoded from it after the blur; how many depends on
//!   how far the decoder runs ahead of the encoder, so the copy-on-write
//!   rule itself is also unit-tested (`frames::tests`).
//! - **One-frame alignment:** a box on frame `k` changes exactly output
//!   frame `k`, and that frame shows source picture `k`, for clips with
//!   B-frames, a variable frame rate and edit lists.
//!
//! The software tests run in CI; the `gpu_` variants decode with NVDEC
//! (H.264 from NVENC) and run locally.

mod support;

use std::path::Path;
use std::sync::Arc;

use blurrd_blur::CpuBlur;
use blurrd_core::{EncoderSetting, RectI};
use blurrd_video::{
    DecodeOptions, EncodeOptions, HwDecode, MuxOptions, TranscodeReport, Transcoder, probe,
};
use support::{
    BITS, BLOCK, ClipSpec, TempClip, box_table, decode_table, pictures, pixel_params, read_number,
    write_clip,
};

/// The box: below the frame-number blocks, inside a 320×240 picture.
const BOX: (i32, i32, i32, i32) = (96, 96, 200, 136);

/// Renders `source` to a lossless Matroska file with one box on frame
/// `frame`, decoding with `hw`.
#[allow(
    clippy::unwrap_used,
    reason = "test helper: an error is a test failure"
)]
fn render_one_box(source: &Path, output: &Path, frame: u32, hw: HwDecode) -> TranscodeReport {
    let info = probe(source).unwrap();
    let (pts, _) = decode_table(source);
    let rect = RectI::new(BOX.0, BOX.1, BOX.2, BOX.3).unwrap();
    let table = box_table(pts.len(), frame, frame, rect);
    let mut options = EncodeOptions::default();
    options.encoder = EncoderSetting::Named(String::from("ffv1"));
    let transcoder = Transcoder::open(
        &info,
        Arc::new(pts),
        output,
        &DecodeOptions::with_hw(hw),
        &options,
        MuxOptions::default(),
    )
    .unwrap();
    transcoder
        .run(&table, &CpuBlur, &pixel_params(&info), &|| false, |_| {})
        .unwrap()
}

/// Indices of the output pictures that differ from the source's.
fn changed(source: &[Vec<u8>], output: &[Vec<u8>]) -> Vec<u32> {
    assert_eq!(output.len(), source.len(), "frame count");
    (0..source.len())
        .filter(|&i| source[i] != output[i])
        .map(|i| i as u32)
        .collect()
}

/// The frame number drawn into a picture (luma of a 320-wide frame).
fn number(picture: &[u8], width: u32) -> u32 {
    read_number(|x, y| picture[(y * width + x) as usize])
}

/// Checks that a box on each of `frames` changes exactly that output
/// frame, which shows the source picture of the same index.
fn check_one_frame_boxes(source: &Path, frames: &[u32], hw: HwDecode) {
    let source_pictures = pictures(source);
    for &frame in frames {
        let output = TempClip::new("one-box", "mkv");
        let report = render_one_box(source, output.path(), frame, hw);
        assert_eq!(report.blurred_frames, 1);
        let output_pictures = pictures(output.path());
        assert_eq!(
            changed(&source_pictures, &output_pictures),
            [frame],
            "{}: box on frame {frame}",
            source.display()
        );
        assert_eq!(
            number(&output_pictures[frame as usize], 320),
            number(&source_pictures[frame as usize], 320),
            "frame {frame} shows the right picture"
        );
    }
}

#[test]
fn blurring_a_p_frame_leaves_the_frames_predicted_from_it_alone() {
    // Keyframes at 0, 30, 60; frames 1..29 are P-frames that predict from
    // their predecessor.
    let source = TempClip::new("pframe", "mp4");
    write_clip(source.path(), &ClipSpec::cfr(90));
    let info = probe(source.path()).unwrap();
    assert!(!info.has_b_frames);
    let source_pictures = pictures(source.path());
    for frame in [10, 29, 30, 0] {
        let output = TempClip::new("pframe-out", "mkv");
        render_one_box(source.path(), output.path(), frame, HwDecode::Software);
        let output_pictures = pictures(output.path());
        assert_eq!(
            changed(&source_pictures, &output_pictures),
            [frame],
            "a box on frame {frame} must not reach frame {} or later",
            frame + 1
        );
        // The blur stays inside the padded box: the frame-number blocks
        // above it are untouched.
        let width = 320;
        let picture = &output_pictures[frame as usize];
        let source_picture = &source_pictures[frame as usize];
        for y in 0..BLOCK {
            for x in 0..BITS * BLOCK {
                let at = (y * width + x) as usize;
                assert_eq!(picture[at], source_picture[at], "({x}, {y})");
            }
        }
    }
}

#[test]
fn a_one_frame_box_lands_on_its_frame_with_b_frames_vfr_and_edit_lists() {
    let vfr: Vec<i64> = (0..96)
        .scan(0, |t, i| {
            let now = *t;
            *t += [16, 17, 17, 33, 50][i % 5];
            Some(now)
        })
        .collect();
    let cases = [
        (
            "bframes",
            "mp4",
            ClipSpec {
                b_frames: 2,
                gop: 24,
                ..ClipSpec::cfr(96)
            },
        ),
        (
            "bframes",
            "mkv",
            ClipSpec {
                b_frames: 2,
                gop: 24,
                ..ClipSpec::cfr(96)
            },
        ),
        (
            "vfr",
            "mkv",
            ClipSpec {
                time_base: (1, 1000),
                pts: vfr.clone(),
                gop: 24,
                ..ClipSpec::cfr(0)
            },
        ),
        (
            "elst-offset",
            "mp4",
            ClipSpec {
                pts: (10..100).collect(),
                ..ClipSpec::cfr(0)
            },
        ),
        (
            "elst-b-vfr",
            "mp4",
            ClipSpec {
                time_base: (1, 1000),
                pts: vfr,
                b_frames: 2,
                gop: 24,
                ..ClipSpec::cfr(0)
            },
        ),
    ];
    for (name, ext, spec) in cases {
        let source = TempClip::new(&format!("align-{name}"), ext);
        write_clip(source.path(), &spec);
        check_one_frame_boxes(
            source.path(),
            &[0, 1, 2, 23, 24, 25, 50],
            HwDecode::Software,
        );
    }
}

/// H.264 from NVENC with B-frames, decoded with NVDEC.
fn nvenc(spec: ClipSpec) -> ClipSpec {
    ClipSpec {
        encoder: "h264_nvenc",
        ..spec
    }
}

#[test]
#[ignore = "needs an NVIDIA GPU"]
fn gpu_nvdec_p_frames_and_one_frame_boxes() {
    let cases = [
        ("pframes", "mp4", nvenc(ClipSpec::cfr(90))),
        (
            "bframes",
            "mkv",
            nvenc(ClipSpec {
                b_frames: 2,
                gop: 24,
                ..ClipSpec::cfr(96)
            }),
        ),
        (
            "offset",
            "mp4",
            nvenc(ClipSpec {
                pts: (10..100).collect(),
                ..ClipSpec::cfr(0)
            }),
        ),
    ];
    for (name, ext, spec) in cases {
        let source = TempClip::new(&format!("gpu-align-{name}"), ext);
        write_clip(source.path(), &spec);
        check_one_frame_boxes(source.path(), &[0, 1, 10, 24, 25, 60], HwDecode::Nvdec);
    }
}
