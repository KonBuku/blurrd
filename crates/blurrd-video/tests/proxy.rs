//! The preview proxy of a synthetic clip: every frame, downscaled, with
//! the source's timestamps and a keyframe every half second; an
//! unfinished proxy leaves no file. CI encodes with the software test
//! encoder.

mod support;

use blurrd_core::{EncoderSetting, RectI};
use blurrd_video::{
    DecodeOptions, Decoder, EncodeOptions, ProxyWriter, RgbConverter, RgbImage, VideoColour,
    YuvRoi, probe,
};
use support::{ClipSpec, TempClip, decode_table, read_number, write_clip};

fn options() -> EncodeOptions {
    let mut options = EncodeOptions::default();
    options.encoder = EncoderSetting::Named(String::from("mpeg4"));
    options
}

#[test]
fn proxies_hold_every_frame_downscaled_with_the_source_timing() {
    let source = TempClip::new("proxy-source", "mkv");
    let mut spec = ClipSpec::cfr(75);
    (spec.width, spec.height) = (1280, 720);
    write_clip(source.path(), &spec);
    let info = probe(source.path()).unwrap();
    assert!(ProxyWriter::needed(&info));

    let output = TempClip::new("proxy-out", "mp4");
    let (mut writer, fallbacks) = ProxyWriter::create(&info, output.path(), &options()).unwrap();
    assert_eq!(fallbacks.len(), 0);
    assert_eq!(writer.encoder(), "mpeg4");
    let mut decoder = Decoder::open(source.path(), &DecodeOptions::default()).unwrap();
    while let Some(frame) = decoder.next_frame().unwrap() {
        writer.push(&frame).unwrap();
    }
    assert_eq!(writer.finish().unwrap(), output.path());

    let proxy = probe(output.path()).unwrap();
    assert_eq!((proxy.width, proxy.height), (960, 540));
    let (source_pts, _) = decode_table(source.path());
    let (proxy_pts, keys) = decode_table(output.path());
    assert_eq!(proxy_pts.len(), 75);
    let seconds = |table: &blurrd_video::PtsTable, i: u32| {
        table.pts(i).unwrap() as f64 * table.time_base().as_f64()
    };
    for i in 0..75 {
        assert!(
            (seconds(&proxy_pts, i) - seconds(&source_pts, i)).abs() < 1e-3,
            "frame {i}"
        );
    }
    assert_eq!(keys, [0, 30, 60], "a keyframe every half second");

    // Each proxy frame shows its own frame number (blocks scaled by 3/4).
    let converter = RgbConverter::new(VideoColour::from_info(&proxy.color, proxy.height));
    let mut decoder = Decoder::open(output.path(), &DecodeOptions::default()).unwrap();
    let (mut roi, mut rgb) = (YuvRoi::new(), RgbImage::default());
    while let Some(frame) = decoder.next_frame().unwrap() {
        frame
            .roi_yuv(RectI::new(0, 0, 960, 540).unwrap(), &mut roi)
            .unwrap();
        converter.convert(&roi, &mut rgb).unwrap();
        let number = read_number(|x, y| {
            let (px, py) = (x * 3 / 4, y * 3 / 4);
            rgb.data[3 * (py * 960 + px) as usize + 1]
        });
        assert_eq!(number, frame.index());
    }
}

#[test]
fn an_unfinished_proxy_leaves_no_file() {
    let source = TempClip::new("proxy-drop", "mkv");
    write_clip(source.path(), &ClipSpec::cfr(10));
    let info = probe(source.path()).unwrap();
    let output = TempClip::new("proxy-drop-out", "mp4");
    let (mut writer, _) = ProxyWriter::create(&info, output.path(), &options()).unwrap();
    let mut decoder = Decoder::open(source.path(), &DecodeOptions::default()).unwrap();
    writer
        .push(&decoder.next_frame().unwrap().unwrap())
        .unwrap();
    drop(writer);
    let dir = output.path().parent().unwrap();
    let stem = output
        .path()
        .file_stem()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let left: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(&stem))
        .collect();
    assert_eq!(left, Vec::<String>::new());
}
