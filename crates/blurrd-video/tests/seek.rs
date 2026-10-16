//! HUD crops, RGB conversion and random access on synthetic clips.

mod support;

use std::sync::Arc;

use blurrd_blur::{BlurBackend, CpuBlur, PixelParams, YuvColour, prepare_boxes};
use blurrd_core::{BlurBox, BlurParams, RectI, Region};
use blurrd_video::{
    DecodeOptions, Decoder, FrameSeeker, PtsTable, RgbConverter, RgbImage, RoiLayout,
    SeekerOptions, VideoColour, VideoError, VideoFrame, YuvRoi, probe,
};
use support::{BITS, BLOCK, ClipSpec, TempClip, read_number, write_clip};

/// Reads the frame number drawn into `frame` through a crop.
#[allow(
    clippy::unwrap_used,
    reason = "test helper: an error is a test failure"
)]
fn number(frame: &VideoFrame) -> u32 {
    let mut roi = YuvRoi::new();
    let bits = RectI::new(0, 0, (BITS * BLOCK).cast_signed(), BLOCK.cast_signed()).unwrap();
    frame.roi_yuv(bits, &mut roi).unwrap();
    assert_eq!(roi.layout(), RoiLayout::I420);
    let width = roi.rect().unwrap().width();
    read_number(|x, y| roi.luma()[(y * width + x) as usize])
}

#[allow(
    clippy::unwrap_used,
    reason = "test helper: an error is a test failure"
)]
fn table(path: &std::path::Path) -> PtsTable {
    let mut decoder = Decoder::open(path, &DecodeOptions::default()).unwrap();
    let mut pts = Vec::new();
    while let Some(frame) = decoder.next_frame().unwrap() {
        assert_eq!(
            number(&frame),
            frame.index(),
            "picture of frame {}",
            frame.index()
        );
        pts.push(frame.pts());
    }
    PtsTable::new(decoder.time_base(), pts).unwrap()
}

#[test]
fn crops_show_the_decoded_picture_and_convert_to_rgb() {
    let clip = TempClip::new("crops", "mp4");
    // 60 frames = exactly 1 s: other lengths make the MP4 muxer round the
    // edit list to its 1/1000 movie time scale and cut the last frame.
    write_clip(clip.path(), &ClipSpec::cfr(60));
    let pts = table(clip.path());
    assert_eq!(pts.len(), 60);

    let info = probe(clip.path()).unwrap();
    let converter = RgbConverter::new(VideoColour::from_info(&info.color, info.height));
    assert!(!converter.is_tone_mapped());
    let mut decoder = Decoder::open(clip.path(), &DecodeOptions::default()).unwrap();
    let frame = decoder.next_frame().unwrap().unwrap();
    // An odd rectangle is aligned outward to even coordinates.
    let mut roi = YuvRoi::new();
    frame
        .roi_yuv(RectI::new(3, 1, 33, 17).unwrap(), &mut roi)
        .unwrap();
    assert_eq!(roi.rect(), Some(RectI::new(2, 0, 34, 18).unwrap()));
    let mut rgb = RgbImage::default();
    converter.convert(&roi, &mut rgb).unwrap();
    assert_eq!(
        (rgb.width, rgb.height, rgb.data.len()),
        (32, 18, 32 * 18 * 3)
    );
    // Frame 0 has all bits black: the bit row is dark, grey below it.
    assert!(
        rgb.data[..32 * 3].iter().all(|&v| v < 40),
        "bit row is black"
    );
    assert!(
        frame
            .roi_yuv(RectI::new(400, 300, 500, 400).unwrap(), &mut roi)
            .is_err()
    );
}

#[test]
fn blurring_a_crop_matches_blurring_a_larger_crop() {
    let clip = TempClip::new("locality", "mkv");
    write_clip(clip.path(), &ClipSpec::cfr(5));
    let mut decoder = Decoder::open(clip.path(), &DecodeOptions::default()).unwrap();
    let frame = decoder.next_frame().unwrap().unwrap();

    let params = PixelParams::new(&BlurParams::default(), 1.0, YuvColour::default()).unwrap();
    let boxes = [BlurBox {
        rect: RectI::new(60, 40, 140, 80).unwrap(),
        region: Region::KillFeed,
        seed: 7,
        grid_origin: (61, 43),
        block_scale: 1.0,
    }];
    let mut prepared = Vec::new();
    prepare_boxes(
        &boxes,
        frame.width(),
        frame.height(),
        &params,
        &mut prepared,
    );

    let mut full = YuvRoi::new();
    frame
        .roi_yuv(RectI::new(0, 0, 320, 240).unwrap(), &mut full)
        .unwrap();
    CpuBlur
        .blur(
            &mut full.as_frame_mut().unwrap(),
            (0, 0),
            &prepared,
            &params,
        )
        .unwrap();
    let mut crop = YuvRoi::new();
    let rect = RectI::new(40, 20, 200, 120).unwrap();
    frame.roi_yuv(rect, &mut crop).unwrap();
    CpuBlur
        .blur(
            &mut crop.as_frame_mut().unwrap(),
            (rect.x0, rect.y0),
            &prepared,
            &params,
        )
        .unwrap();

    let crop_w = rect.width() as usize;
    for y in 0..rect.height() as usize {
        let full_row = &full.luma()[(y + 20) * 320 + 40..][..crop_w];
        assert_eq!(&crop.luma()[y * crop_w..][..crop_w], full_row, "row {y}");
    }
}

#[test]
fn seeker_returns_the_right_picture_for_any_index() {
    let clip = TempClip::new("seek", "mp4");
    write_clip(clip.path(), &ClipSpec::cfr(150));
    let pts = Arc::new(table(clip.path()));
    let mut seeker =
        FrameSeeker::open(clip.path(), Arc::clone(&pts), SeekerOptions::default()).unwrap();
    assert_eq!(seeker.frame_count(), 150);
    for index in [75, 3, 149, 0, 60, 61, 62, 30, 29, 120, 119, 31, 148, 1] {
        let frame = seeker.frame(index).unwrap();
        assert_eq!(frame.index(), index);
        assert_eq!(Some(frame.pts()), pts.pts(index));
        assert_eq!(number(&frame), index, "picture of frame {index}");
    }
    assert!(matches!(
        seeker.frame(150),
        Err(VideoError::FrameOutOfRange {
            index: 150,
            count: 150
        })
    ));
}

#[test]
fn keyframe_seeks_land_on_the_keyframe_before_a_time() {
    let clip = TempClip::new("keyframes", "mp4");
    // Keyframes every 30 frames.
    write_clip(clip.path(), &ClipSpec::cfr(150));
    let pts = table(clip.path());
    let mut decoder = Decoder::open(clip.path(), &DecodeOptions::default()).unwrap();
    for (target, keyframe) in [(100, 90), (30, 30), (149, 120), (0, 0), (59, 30)] {
        decoder.seek_keyframe(pts.pts(target).unwrap()).unwrap();
        let frame = decoder.next_frame().unwrap().unwrap();
        assert!(frame.is_key(), "seek to {target}");
        assert_eq!(number(&frame), keyframe, "seek to {target}");
        assert_eq!(Some(frame.pts()), pts.pts(keyframe));
        // Indices count from the keyframe; decoding goes on from there.
        assert_eq!(frame.index(), 0);
        let next = decoder.next_frame().unwrap().unwrap();
        assert_eq!((next.index(), number(&next)), (1, keyframe + 1));
    }
    // A time before the first frame gives the first frame.
    decoder.seek_keyframe(pts.pts(0).unwrap() - 100).unwrap();
    let frame = decoder.next_frame().unwrap().unwrap();
    assert_eq!(number(&frame), 0);
}

#[test]
fn seeker_detects_a_table_from_another_file() {
    let clip = TempClip::new("mismatch", "mkv");
    write_clip(clip.path(), &ClipSpec::cfr(60));
    let real = table(clip.path());
    let shifted: Vec<i64> = real.as_slice().iter().map(|p| p + 1).collect();
    let wrong = Arc::new(PtsTable::new(real.time_base(), shifted).unwrap());
    let mut options = SeekerOptions::default();
    options.prefetch = false;
    let mut seeker = FrameSeeker::open(clip.path(), wrong, options).unwrap();
    let error = seeker.frame(10).unwrap_err();
    assert_eq!(
        error.kind(),
        blurrd_core::ErrorKind::ProjectMismatch,
        "{error}"
    );
}
