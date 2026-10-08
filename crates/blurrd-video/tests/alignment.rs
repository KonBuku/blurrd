//! Frame alignment on clips whose decode order, timing or presentation
//! differ from the naive "n-th packet = n-th frame": B-frames, variable
//! frame rate and edit lists. Every frame shows its own number (see
//! `support`), so the tests check the picture behind each index, both when
//! decoding sequentially and when seeking.
//!
//! The software tests run in CI; the `gpu_` variants repeat them with
//! NVDEC on H.264 from NVENC and run locally.

mod support;

use std::path::Path;
use std::sync::Arc;

use blurrd_core::RectI;
use blurrd_video::{
    DecodeOptions, Decoder, FrameSeeker, HwDecode, PtsTable, SeekerOptions, VideoFrame, YuvRoi,
    probe,
};
use support::{BITS, BLOCK, ClipSpec, TempClip, presented_packets, read_number, write_clip};

#[allow(
    clippy::unwrap_used,
    reason = "test helper: an error is a test failure"
)]
fn number(frame: &VideoFrame) -> u32 {
    let mut roi = YuvRoi::new();
    let bits = RectI::new(0, 0, (BITS * BLOCK).cast_signed(), BLOCK.cast_signed()).unwrap();
    frame.roi_yuv(bits, &mut roi).unwrap();
    let width = roi.rect().unwrap().width();
    // NV12 and I420 luma are both 8-bit, one byte per sample.
    read_number(|x, y| roi.luma()[(y * width + x) as usize])
}

/// What sequential decoding produced.
struct Decoded {
    table: PtsTable,
    /// Frame number drawn into each decoded picture.
    numbers: Vec<u32>,
    keyframes: Vec<u32>,
}

#[allow(
    clippy::unwrap_used,
    reason = "test helper: an error is a test failure"
)]
fn decode(path: &Path, hw: HwDecode) -> Decoded {
    let mut decoder = Decoder::open(path, &DecodeOptions::with_hw(hw)).unwrap();
    let (mut pts, mut numbers, mut keyframes) = (Vec::new(), Vec::new(), Vec::new());
    while let Some(frame) = decoder.next_frame().unwrap() {
        assert_eq!(frame.index() as usize, pts.len(), "indices are consecutive");
        if frame.is_key() {
            keyframes.push(frame.index());
        }
        numbers.push(number(&frame));
        pts.push(frame.pts());
    }
    if hw != HwDecode::Software {
        assert_eq!(
            decoder.hardware_frames() as usize,
            pts.len(),
            "all frames on the GPU"
        );
    }
    Decoded {
        table: PtsTable::new(decoder.time_base(), pts).unwrap(),
        numbers,
        keyframes,
    }
}

/// Seeks to a fixed pseudo-random sequence of frames (with backward jumps,
/// GOP boundaries and the last frame) and checks PTS and picture.
#[allow(
    clippy::unwrap_used,
    reason = "test helper: an error is a test failure"
)]
fn check_seeking(path: &Path, decoded: &Decoded, hw: HwDecode) {
    let table = Arc::new(decoded.table.clone());
    let mut options = SeekerOptions::default();
    options.decode = DecodeOptions::with_hw(hw);
    let mut seeker = FrameSeeker::open(path, Arc::clone(&table), options).unwrap();
    let count = table.len();
    let mut state = 12_345u32;
    let mut targets = vec![count - 1, 0, count / 2, count / 2 - 1, 1];
    targets.extend((0..40).map(|_| {
        state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        (state >> 8) % count
    }));
    for index in targets {
        let frame = seeker.frame(index).unwrap();
        assert_eq!(frame.index(), index);
        assert_eq!(Some(frame.pts()), table.pts(index), "PTS of frame {index}");
        assert_eq!(
            number(&frame),
            decoded.numbers[index as usize],
            "picture of frame {index}"
        );
    }
}

/// Decodes a clip of `written` frames, checks it against the probe and the
/// expected pictures, then checks seeking.
///
/// The decoder must output exactly the frames the demuxer presents, picture
/// `i` at index `i`. MP4 edit lists may cut the final frame (the muxer
/// rounds the edit to its movie time scale), never more.
#[allow(
    clippy::unwrap_used,
    reason = "test helper: an error is a test failure"
)]
fn check_clip(path: &Path, hw: HwDecode, written: u32) -> Decoded {
    let decoded = decode(path, hw);
    let presented = presented_packets(path);
    assert!(
        presented + 1 >= written,
        "at most the last of {written} frames is cut"
    );
    assert_eq!(
        decoded.numbers,
        sequence(presented),
        "pictures in presentation order"
    );
    let info = probe(path).unwrap();
    assert_eq!(
        info.keyframes.as_deref(),
        Some(&decoded.keyframes[..]),
        "probe keyframes"
    );
    check_seeking(path, &decoded, hw);
    decoded
}

/// Frame durations in milliseconds for the variable-rate clips: mostly
/// ~60 fps with jitter, dropped frames and a stall.
fn vfr_pts(frames: usize) -> Vec<i64> {
    let pattern = [16, 17, 17, 33, 16, 17, 50, 17, 16, 100, 17, 17];
    let mut pts = Vec::with_capacity(frames);
    let mut t = 0;
    for i in 0..frames {
        pts.push(t);
        t += pattern[i % pattern.len()];
    }
    pts
}

fn b_frame_spec(frames: u32) -> ClipSpec {
    ClipSpec {
        b_frames: 2,
        gop: 24,
        ..ClipSpec::cfr(frames)
    }
}

fn vfr_spec(frames: usize) -> ClipSpec {
    ClipSpec {
        time_base: (1, 1000),
        pts: vfr_pts(frames),
        gop: 24,
        ..ClipSpec::cfr(0)
    }
}

fn sequence(n: u32) -> Vec<u32> {
    (0..n).collect()
}

#[test]
fn b_frames_decode_and_seek_in_presentation_order() {
    for ext in ["mp4", "mkv"] {
        let clip = TempClip::new("bframes", ext);
        write_clip(clip.path(), &b_frame_spec(120));
        let info = probe(clip.path()).unwrap();
        assert!(info.has_b_frames, "{ext}: {info:?}");
        check_clip(clip.path(), HwDecode::Software, 120);
    }
}

#[test]
fn variable_frame_rate_keeps_every_timestamp() {
    for ext in ["mp4", "mkv"] {
        let clip = TempClip::new("vfr", ext);
        let spec = vfr_spec(120);
        write_clip(clip.path(), &spec);
        let info = probe(clip.path()).unwrap();
        assert!(info.is_vfr, "{ext}: {info:?}");
        let decoded = check_clip(clip.path(), HwDecode::Software, 120);
        // The decoded PTS are the written ones in the stream's time base
        // (as far as the container presents them).
        let tb = decoded.table.time_base();
        let rescaled: Vec<i64> = decoded
            .table
            .as_slice()
            .iter()
            .map(|&p| {
                (p - decoded.table.as_slice()[0]) * 1000 * i64::from(tb.num) / i64::from(tb.den)
            })
            .collect();
        assert_eq!(rescaled, spec.pts[..rescaled.len()], "{ext}: timestamps");
    }
}

#[test]
fn edit_list_with_a_start_offset() {
    // Presentation starts 10 frames after the media start: the MP4 muxer
    // writes an empty edit.
    let clip = TempClip::new("offset", "mp4");
    let spec = ClipSpec {
        pts: (10..100).collect(),
        ..ClipSpec::cfr(0)
    };
    write_clip(clip.path(), &spec);
    let info = probe(clip.path()).unwrap();
    assert!(info.has_edit_list, "{info:?}");
    let decoded = check_clip(clip.path(), HwDecode::Software, 90);
    assert!(decoded.table.len() >= 89);
}

#[test]
fn edit_list_that_cuts_the_last_frame() {
    // 40 frames at 60 fps last 666.67 ms; the MP4 muxer rounds the edit to
    // its 1/1000 movie time scale (666 ms), so the demuxer discards the
    // last frame. The decoder output (39 frames) is the truth.
    let clip = TempClip::new("cut", "mp4");
    write_clip(clip.path(), &ClipSpec::cfr(40));
    let info = probe(clip.path()).unwrap();
    assert!(info.has_edit_list, "{info:?}");
    let decoded = check_clip(clip.path(), HwDecode::Software, 40);
    assert_eq!(decoded.table.len(), 39);
}

#[test]
fn edit_list_with_b_frames_and_variable_rate() {
    let clip = TempClip::new("mixed", "mp4");
    let spec = ClipSpec {
        b_frames: 2,
        ..vfr_spec(96)
    };
    write_clip(clip.path(), &spec);
    let info = probe(clip.path()).unwrap();
    assert!(
        info.has_b_frames && info.is_vfr && info.has_edit_list,
        "{info:?}"
    );
    check_clip(clip.path(), HwDecode::Software, 96);
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
fn gpu_nvdec_alignment_on_b_frames_vfr_and_edit_lists() {
    let cases = [
        ("bframes", "mp4", nvenc(b_frame_spec(120)), 120),
        ("bframes", "mkv", nvenc(b_frame_spec(120)), 120),
        (
            "vfr",
            "mp4",
            nvenc(ClipSpec {
                b_frames: 2,
                ..vfr_spec(120)
            }),
            120,
        ),
        (
            "offset",
            "mp4",
            nvenc(ClipSpec {
                pts: (10..100).collect(),
                ..ClipSpec::cfr(0)
            }),
            90,
        ),
        ("cut", "mp4", nvenc(ClipSpec::cfr(40)), 40),
    ];
    for (name, ext, spec, frames) in cases {
        let clip = TempClip::new(&format!("gpu-{name}"), ext);
        write_clip(clip.path(), &spec);
        let software = check_clip(clip.path(), HwDecode::Software, frames);
        let nvdec = check_clip(clip.path(), HwDecode::Nvdec, frames);
        assert_eq!(nvdec.table, software.table, "{name}.{ext}: same PTS table");
    }
}
