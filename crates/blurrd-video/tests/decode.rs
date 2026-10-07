//! Probe and sequential decode of synthetic clips.

mod support;

use blurrd_video::{DecodeOptions, Decoder, PixelLayout, PtsTable, probe};
use support::{ClipSpec, TempClip, write_clip};

#[allow(
    clippy::unwrap_used,
    reason = "test helper: an error is a test failure"
)]
fn decode_all(path: &std::path::Path) -> (Vec<u32>, Vec<i64>) {
    let mut decoder = Decoder::open(path, &DecodeOptions::default()).unwrap();
    let (mut indices, mut pts) = (Vec::new(), Vec::new());
    while let Some(frame) = decoder.next_frame().unwrap() {
        assert_eq!(frame.layout(), PixelLayout::I420);
        assert_eq!((frame.width(), frame.height()), (320, 240));
        indices.push(frame.index());
        pts.push(frame.pts());
    }
    (indices, pts)
}

#[test]
fn probes_and_decodes_a_constant_rate_clip() {
    for ext in ["mp4", "mkv"] {
        let clip = TempClip::new("cfr", ext);
        write_clip(clip.path(), &ClipSpec::cfr(90));

        let info = probe(clip.path()).unwrap();
        assert_eq!((info.width, info.height), (320, 240), "{ext}");
        assert_eq!(info.video_codec, "mpeg4");
        assert_eq!(info.bit_depth, 8);
        assert!(
            !info.has_b_frames && !info.is_vfr && !info.has_edit_list,
            "{ext}: {info:?}"
        );
        assert_eq!(info.keyframes.as_deref(), Some(&[0, 30, 60][..]), "{ext}");
        assert!(
            (info.fps.as_f64() - 60.0).abs() < 0.01,
            "{ext}: {:?}",
            info.fps
        );
        assert!(info.audio.is_empty() && !info.has_subtitles);

        let (indices, pts) = decode_all(clip.path());
        assert_eq!(indices, (0..90).collect::<Vec<_>>(), "{ext}");
        let table = PtsTable::new(info.time_base, pts).unwrap();
        assert_eq!(table.len(), 90);
        if let Some(count) = info.frame_count {
            assert_eq!(count, table.len(), "{ext}");
        }
    }
}

#[test]
fn reports_missing_and_invalid_files() {
    let missing = std::env::temp_dir().join("blurrd-video-tests-does-not-exist.mp4");
    let error = probe(&missing).unwrap_err();
    assert_eq!(error.kind(), blurrd_core::ErrorKind::NotFound);

    let clip = TempClip::new("garbage", "mp4");
    std::fs::write(clip.path(), b"not a video at all, just text").unwrap();
    let error = Decoder::open(clip.path(), &DecodeOptions::default()).unwrap_err();
    assert_eq!(
        error.kind(),
        blurrd_core::ErrorKind::UnsupportedMedia,
        "{error}"
    );
}

#[test]
fn never_opens_other_protocols() {
    // A playlist that points at a network URL must not be followed.
    let clip = TempClip::new("playlist", "m3u8");
    std::fs::write(
        clip.path(),
        "#EXTM3U\n#EXTINF:1,\nhttp://127.0.0.1:9/x.ts\n",
    )
    .unwrap();
    assert!(Decoder::open(clip.path(), &DecodeOptions::default()).is_err());
}
