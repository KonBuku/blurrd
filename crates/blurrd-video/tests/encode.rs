//! Encoding decoded frames: forced keyframes and timestamps with the
//! software test encoder (CI), NVENC and the encoder probe (locally, run
//! with `cargo nextest run -p blurrd-video --run-ignored all gpu`).

mod support;

use std::path::Path;

use blurrd_core::EncoderSetting;
use blurrd_video::{
    DecodeOptions, Decoder, EncodeOptions, EncoderInput, VideoEncoder, probe, probe_encoders,
};
use support::{ClipSpec, TempClip, write_clip};

/// What came out of the encoder, in decode order.
struct Encoded {
    /// Presentation timestamps of the packets.
    pts: Vec<i64>,
    /// Presentation indices of the keyframe packets.
    keyframes: Vec<u32>,
    /// Source PTS of every frame.
    source_pts: Vec<i64>,
}

/// Decodes `path` and encodes every frame with `encoder`, asking for a
/// keyframe at each index in `keyframes`.
#[allow(
    clippy::unwrap_used,
    reason = "test helper: an error is a test failure"
)]
fn encode(path: &Path, encoder: &str, keyframes: &[u32], interval: Option<u32>) -> Encoded {
    let info = probe(path).unwrap();
    let mut decoder = Decoder::open(path, &DecodeOptions::default()).unwrap();
    let mut frame = decoder.next_frame().unwrap().unwrap();
    let mut input = EncoderInput::from_media(&info, frame.layout());
    if let Some(interval) = interval {
        input.keyframe_interval = interval;
    }
    let mut options = EncodeOptions::default();
    options.encoder = EncoderSetting::Named(encoder.to_owned());
    let (mut enc, fallbacks) = VideoEncoder::open(&input, &options).unwrap();
    assert_eq!(fallbacks, Vec::new());
    assert_eq!(enc.name(), encoder);

    let mut packets = Vec::new();
    let mut source_pts = Vec::new();
    loop {
        source_pts.push(frame.pts());
        let key = keyframes.contains(&frame.index());
        enc.send(&mut frame, key).unwrap();
        while let Some(packet) = enc.receive().unwrap() {
            packets.push(packet);
        }
        match decoder.next_frame().unwrap() {
            Some(next) => frame = next,
            None => break,
        }
    }
    enc.flush().unwrap();
    while let Some(packet) = enc.receive().unwrap() {
        packets.push(packet);
    }
    let pts: Vec<i64> = packets.iter().map(|p| p.pts().unwrap()).collect();
    let keyframes = packets
        .iter()
        .filter(|p| p.is_key())
        .map(|p| source_pts.binary_search(&p.pts().unwrap()).unwrap() as u32)
        .collect();
    assert!(packets.iter().all(|p| !p.data().is_empty()));
    Encoded {
        pts,
        keyframes,
        source_pts,
    }
}

#[test]
fn keeps_timestamps_and_places_keyframes_where_asked() {
    let clip = TempClip::new("encode-keys", "mp4");
    write_clip(clip.path(), &ClipSpec::cfr(90));
    // The source's keyframes (every 30 frames): the encoder's own interval
    // is one more than the longest source interval, so it adds none.
    let encoded = encode(clip.path(), "mpeg4", &[0, 30, 60], None);
    assert_eq!(
        encoded.pts, encoded.source_pts,
        "one packet per frame, same PTS"
    );
    assert_eq!(encoded.keyframes, [0, 30, 60]);
    // Any other frames, with a long interval.
    let encoded = encode(clip.path(), "mpeg4", &[0, 45, 77], Some(500));
    assert_eq!(encoded.keyframes, [0, 45, 77]);
}

#[test]
fn rejects_frames_of_another_layout() {
    let clip = TempClip::new("encode-layout", "mp4");
    write_clip(clip.path(), &ClipSpec::cfr(2));
    let info = probe(clip.path()).unwrap();
    let mut decoder = Decoder::open(clip.path(), &DecodeOptions::default()).unwrap();
    let mut frame = decoder.next_frame().unwrap().unwrap();
    let mut input = EncoderInput::from_media(&info, frame.layout());
    input.width += 16;
    let mut options = EncodeOptions::default();
    options.encoder = EncoderSetting::Named(String::from("mpeg4"));
    let (mut encoder, _) = VideoEncoder::open(&input, &options).unwrap();
    assert!(encoder.send(&mut frame, true).is_err());
}

#[test]
#[ignore = "needs an NVIDIA GPU"]
fn gpu_nvenc_forces_idr_frames_exactly() {
    let clip = TempClip::new("encode-nvenc", "mp4");
    write_clip(clip.path(), &ClipSpec::cfr(120));
    let encoded = encode(clip.path(), "h264_nvenc", &[0, 45, 77, 78], Some(1000));
    assert_eq!(encoded.pts, encoded.source_pts);
    assert_eq!(encoded.keyframes, [0, 45, 77, 78]);
}

#[test]
#[ignore = "needs an NVIDIA GPU"]
fn gpu_probe_finds_nvenc_and_caches_the_result() {
    let dir = std::env::temp_dir().join(format!("blurrd-encoder-probe-{}", std::process::id()));
    let encoders = probe_encoders(&dir);
    let find = |name: &str| encoders.iter().find(|e| e.name == name).unwrap();
    assert!(find("h264_nvenc").available && find("h264_nvenc").hardware);
    assert!(find("hevc_nvenc").ten_bit, "HEVC NVENC encodes 10-bit");
    assert!(find("h264_mf").available, "Media Foundation always exists");
    assert!(dir.join("encoders.json").is_file());
    assert_eq!(
        probe_encoders(&dir),
        encoders,
        "second call reads the cache"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
