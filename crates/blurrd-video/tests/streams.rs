//! The streams besides the video and the container: audio and subtitles
//! are copied packet for packet, AAC re-encoding and removal work,
//! recorder metadata is stripped by default, and MP4 outputs get their
//! index in reserved space at the front without a rewrite pass.

mod support;

use std::path::Path;
use std::sync::Arc;

use blurrd_blur::CpuBlur;
use blurrd_core::{AudioMode, EncoderSetting, MetadataMode, MoovMode, RectI};
use blurrd_video::{
    DecodeOptions, EncodeOptions, MoovLayout, MuxOptions, TranscodeReport, Transcoder, probe,
};
use ffmpeg_the_third::media::Type;
use support::{
    ClipSpec, TempClip, box_table, decode_table, pixel_params, stream_packets, top_level_boxes,
    write_clip,
};

/// A clip with video, AAC audio, subtitles and recorder tags.
fn full_spec() -> ClipSpec {
    ClipSpec {
        audio: true,
        subtitles: true,
        tags: vec![
            ("title", "Recorder title"),
            ("comment", "recorded by someone"),
        ],
        ..ClipSpec::cfr(120)
    }
}

/// Renders `source` into `output` with the test encoder and `mux`.
#[allow(
    clippy::unwrap_used,
    reason = "test helper: an error is a test failure"
)]
fn render(source: &Path, output: &Path, mux: MuxOptions) -> TranscodeReport {
    let info = probe(source).unwrap();
    let (pts, _) = decode_table(source);
    let table = box_table(pts.len(), 10, 20, RectI::new(8, 8, 64, 32).unwrap());
    let mut options = EncodeOptions::default();
    options.encoder = EncoderSetting::Named(String::from("mpeg4"));
    let transcoder = Transcoder::open(
        &info,
        Arc::new(pts),
        output,
        &DecodeOptions::default(),
        &options,
        mux,
    )
    .unwrap();
    transcoder
        .run(&table, &CpuBlur, &pixel_params(&info), &|| false, |_| {})
        .unwrap()
}

/// Container-level tags of a file.
#[allow(
    clippy::unwrap_used,
    reason = "test helper: an error is a test failure"
)]
fn tags(path: &Path) -> Vec<(String, String)> {
    let input = ffmpeg_the_third::format::input(path).unwrap();
    input
        .metadata()
        .iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
}

/// The language tag of the audio stream.
#[allow(
    clippy::unwrap_used,
    reason = "test helper: an error is a test failure"
)]
fn audio_language(path: &Path) -> Option<String> {
    let input = ffmpeg_the_third::format::input(path).unwrap();
    let stream = input.streams().best(Type::Audio)?;
    stream.metadata().get("language").map(str::to_owned)
}

#[test]
fn copies_audio_and_subtitles_and_strips_recorder_tags() {
    for ext in ["mp4", "mkv"] {
        let source = TempClip::new("streams-src", ext);
        write_clip(source.path(), &full_spec());
        let output = TempClip::new("streams-out", ext);
        let report = render(source.path(), output.path(), MuxOptions::default());
        assert_eq!(report.skipped_streams, Vec::new(), "{ext}");
        let source_audio = stream_packets(source.path(), Type::Audio);
        assert!(source_audio.len() > 50, "{ext}: the clip has audio");
        assert_eq!(
            stream_packets(output.path(), Type::Audio),
            source_audio,
            "{ext}: audio packets are bit-identical"
        );
        let source_subtitles = stream_packets(source.path(), Type::Subtitle);
        assert!(
            !source_subtitles.is_empty(),
            "{ext}: the clip has subtitles"
        );
        assert_eq!(
            stream_packets(output.path(), Type::Subtitle),
            source_subtitles,
            "{ext}: subtitle packets are bit-identical"
        );
        let output_tags = tags(output.path());
        assert!(
            !output_tags
                .iter()
                .any(|(k, _)| k == "title" || k == "comment"),
            "{ext}: recorder tags stripped: {output_tags:?}"
        );
        assert_eq!(
            audio_language(output.path()).as_deref(),
            Some("deu"),
            "{ext}"
        );
    }
}

#[test]
fn keeps_every_tag_when_asked() {
    let source = TempClip::new("streams-all", "mkv");
    write_clip(source.path(), &full_spec());
    let output = TempClip::new("streams-all-out", "mkv");
    let mut mux = MuxOptions::default();
    mux.metadata = MetadataMode::All;
    render(source.path(), output.path(), mux);
    let output_tags = tags(output.path());
    // Matroska writes tag names in upper case.
    assert!(
        output_tags
            .iter()
            .any(|(k, v)| k.eq_ignore_ascii_case("title") && v == "Recorder title"),
        "{output_tags:?}"
    );
}

#[test]
fn reencodes_or_removes_audio() {
    let source = TempClip::new("streams-aac", "mkv");
    write_clip(source.path(), &full_spec());
    let source_audio = stream_packets(source.path(), Type::Audio);

    let aac = TempClip::new("streams-aac-out", "mp4");
    let mut mux = MuxOptions::default();
    mux.audio = AudioMode::Aac(96);
    render(source.path(), aac.path(), mux.clone());
    let info = probe(aac.path()).unwrap();
    assert_eq!(info.audio.len(), 1);
    assert_eq!(info.audio[0].codec, "aac");
    assert_eq!(info.audio[0].sample_rate, 48_000);
    let packets = stream_packets(aac.path(), Type::Audio);
    // Same duration: one 1024-sample packet per source packet, give or
    // take the encoder's priming.
    assert!(
        packets.len().abs_diff(source_audio.len()) <= 2,
        "{} vs {}",
        packets.len(),
        source_audio.len()
    );

    let silent = TempClip::new("streams-silent-out", "mp4");
    mux.audio = AudioMode::Remove;
    let report = render(source.path(), silent.path(), mux);
    assert_eq!(probe(silent.path()).unwrap().audio, Vec::new());
    assert!(
        report.skipped_streams.iter().any(|s| s.codec == "aac"),
        "the removed audio stream is listed: {:?}",
        report.skipped_streams
    );
}

#[test]
fn leaves_out_subtitles_the_container_cannot_store() {
    // SubRip from Matroska cannot go into MP4.
    let source = TempClip::new("streams-srt", "mkv");
    write_clip(source.path(), &full_spec());
    let output = TempClip::new("streams-srt-out", "mp4");
    let report = render(source.path(), output.path(), MuxOptions::default());
    assert_eq!(report.skipped_streams.len(), 1);
    assert_eq!(report.skipped_streams[0].codec, "subrip");
    assert_eq!(stream_packets(output.path(), Type::Subtitle), Vec::new());
    // Matroska and MP4 count time differently; the audio data is the same.
    let data = |path: &Path| -> Vec<Vec<u8>> {
        stream_packets(path, Type::Audio)
            .into_iter()
            .map(|packet| packet.3)
            .collect()
    };
    assert_eq!(data(output.path()), data(source.path()));
}

#[test]
fn mp4_index_placement() {
    let source = TempClip::new("streams-moov", "mp4");
    write_clip(source.path(), &full_spec());
    let output = TempClip::new("streams-moov-out", "mp4");

    // Default: the index goes into reserved space before the media data,
    // padded with a free box; no rewrite pass.
    let report = render(source.path(), output.path(), MuxOptions::default());
    assert!(
        matches!(report.moov, MoovLayout::Reserved { .. }),
        "{report:?}"
    );
    // FFmpeg puts its own small free box before the media data as well.
    let boxes = top_level_boxes(output.path());
    assert_eq!(boxes[..3], ["ftyp", "moov", "free"], "{boxes:?}");
    assert_eq!(boxes.last().map(String::as_str), Some("mdat"), "{boxes:?}");
    assert!(
        boxes[2..boxes.len() - 1].iter().all(|b| b == "free"),
        "{boxes:?}"
    );

    let mut mux = MuxOptions::default();
    mux.moov = MoovMode::Faststart;
    let report = render(source.path(), output.path(), mux.clone());
    assert_eq!(report.moov, MoovLayout::Faststart);
    let boxes = top_level_boxes(output.path());
    assert_eq!(boxes.first().map(String::as_str), Some("ftyp"));
    assert!(
        boxes.iter().position(|b| b == "moov") < boxes.iter().position(|b| b == "mdat"),
        "{boxes:?}"
    );

    mux.moov = MoovMode::AtEnd;
    let report = render(source.path(), output.path(), mux);
    assert_eq!(report.moov, MoovLayout::AtEnd);
    let boxes = top_level_boxes(output.path());
    assert!(
        boxes.iter().position(|b| b == "moov") > boxes.iter().position(|b| b == "mdat"),
        "{boxes:?}"
    );

    let mkv = TempClip::new("streams-moov-out", "mkv");
    let report = render(source.path(), mkv.path(), MuxOptions::default());
    assert_eq!(report.moov, MoovLayout::NotApplicable);
}
