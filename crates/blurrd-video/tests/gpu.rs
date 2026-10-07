//! Hardware decoding on a local GPU (not available on CI runners). Run with
//! `cargo nextest run -p blurrd-video --run-ignored all gpu`.

mod support;

use blurrd_video::{DecodeOptions, Decoder, HwDecode, PixelLayout};
use support::{ClipSpec, TempClip, write_clip};

#[allow(
    clippy::unwrap_used,
    reason = "test helper: an error is a test failure"
)]
fn decode(path: &std::path::Path, hw: HwDecode) -> (Vec<i64>, Vec<PixelLayout>, u32) {
    let mut decoder = Decoder::open(path, &DecodeOptions::with_hw(hw)).unwrap();
    let (mut pts, mut layouts) = (Vec::new(), Vec::new());
    while let Some(frame) = decoder.next_frame().unwrap() {
        assert_eq!(frame.index() as usize, pts.len());
        pts.push(frame.pts());
        layouts.push(frame.layout());
    }
    (pts, layouts, decoder.hardware_frames())
}

#[test]
#[ignore = "needs an NVIDIA GPU"]
fn gpu_nvdec_matches_software_decoding() {
    let clip = TempClip::new("nvdec", "mp4");
    write_clip(clip.path(), &ClipSpec::cfr(120));
    let (sw_pts, _, sw_hw) = decode(clip.path(), HwDecode::Software);
    let (hw_pts, layouts, hw_frames) = decode(clip.path(), HwDecode::Nvdec);
    assert_eq!(sw_hw, 0);
    assert_eq!(hw_pts, sw_pts);
    assert_eq!(hw_frames, 120, "every frame must come from NVDEC");
    assert!(layouts.iter().all(|&l| l == PixelLayout::Nv12));
}

#[test]
#[ignore = "needs an NVIDIA GPU (the clip is encoded with NVENC)"]
fn gpu_d3d11va_matches_software_decoding() {
    // D3D11VA has no MPEG-4 Part 2 path, so the clip is H.264.
    let clip = TempClip::new("d3d11va", "mkv");
    let spec = ClipSpec {
        encoder: "h264_nvenc",
        ..ClipSpec::cfr(60)
    };
    write_clip(clip.path(), &spec);
    let (sw_pts, _, _) = decode(clip.path(), HwDecode::Software);
    let (hw_pts, _, hw_frames) = decode(clip.path(), HwDecode::D3d11va);
    assert_eq!(hw_pts, sw_pts);
    assert_eq!(hw_frames, 60);
}
