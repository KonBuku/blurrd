//! Synthetic test clips, written at test time with FFmpeg's built-in
//! encoders (no footage is ever committed).
//!
//! Every frame shows its own number: the top-left 16 blocks of 16×16 pixels
//! are the bits of the frame number (most significant first, white = 1),
//! so a test can check which picture a decoder returned, not only its PTS.

#![allow(
    dead_code,
    unreachable_pub,
    clippy::unwrap_used,
    reason = "test helpers shared through `mod support`: each test binary uses \
              a different subset, and a panic is a test failure"
)]

use std::path::{Path, PathBuf};
use std::ptr;

use ffmpeg_the_third::{Dictionary, Rational, codec, encoder, ffi, format, frame};

/// Side of one bit block in pixels.
pub const BLOCK: u32 = 16;
/// Number of bit blocks.
pub const BITS: u32 = 16;

/// What to write.
#[derive(Debug, Clone)]
pub struct ClipSpec {
    /// Encoder name, for example `mpeg4`.
    pub encoder: &'static str,
    /// Picture width (even).
    pub width: u32,
    /// Picture height (even).
    pub height: u32,
    /// Encoder time base.
    pub time_base: (i32, i32),
    /// Presentation timestamp of every frame in the encoder time base,
    /// strictly increasing.
    pub pts: Vec<i64>,
    /// Maximum consecutive B-frames (0 = none).
    pub b_frames: u32,
    /// Keyframe interval in frames.
    pub gop: u32,
    /// Extra encoder options.
    pub options: Vec<(&'static str, &'static str)>,
    /// Add a stereo 48 kHz AAC stream (language `deu`) covering the video.
    pub audio: bool,
    /// Add a subtitle stream (MP4 timed text or `SubRip`, by container) with
    /// a line every 30 frames.
    pub subtitles: bool,
    /// Container metadata tags.
    pub tags: Vec<(&'static str, &'static str)>,
}

impl ClipSpec {
    /// A constant-rate clip of `frames` frames at 60 fps with `mpeg4`.
    pub fn cfr(frames: u32) -> Self {
        Self {
            encoder: "mpeg4",
            width: 320,
            height: 240,
            time_base: (1, 60),
            pts: (0..i64::from(frames)).collect(),
            b_frames: 0,
            gop: 30,
            options: Vec::new(),
            audio: false,
            subtitles: false,
            tags: Vec::new(),
        }
    }
}

/// A unique path in the system temp folder (removed by [`TempClip`]).
pub struct TempClip(pub PathBuf);

impl TempClip {
    pub fn new(name: &str, ext: &str) -> Self {
        let dir = std::env::temp_dir().join("blurrd-video-tests");
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir.join(format!("{name}-{}.{ext}", std::process::id())))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempClip {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Luma value of the bit blocks and the background.
pub const WHITE: u8 = 235;
pub const BLACK: u8 = 16;
pub const GREY: u8 = 128;

/// Fills `frame` (yuv420p) with the pattern for `number`.
fn draw(frame: &mut frame::Video, number: u32) {
    let (w, h) = (frame.width() as usize, frame.height() as usize);
    let stride = frame.stride(0);
    let luma = frame.data_mut(0);
    for y in 0..h {
        for x in 0..w {
            let block = (x / BLOCK as usize) as u32;
            let value = if y < BLOCK as usize && block < BITS {
                if number >> (BITS - 1 - block) & 1 == 1 {
                    WHITE
                } else {
                    BLACK
                }
            } else {
                // A gentle moving gradient so frames differ everywhere.
                GREY.wrapping_add(((x + y + number as usize) % 32) as u8)
            };
            luma[y * stride + x] = value;
        }
    }
    for plane in 1..3 {
        let stride = frame.stride(plane);
        let rows = h.div_ceil(2);
        let data = frame.data_mut(plane);
        for y in 0..rows {
            data[y * stride..y * stride + w.div_ceil(2)].fill(128);
        }
    }
}

/// Reads the frame number from luma rows: `sample(x, y)` returns the 8-bit
/// luma value at a pixel.
pub fn read_number(sample: impl Fn(u32, u32) -> u8) -> u32 {
    (0..BITS).fold(0, |acc, bit| {
        let value = sample(bit * BLOCK + BLOCK / 2, BLOCK / 2);
        (acc << 1) | u32::from(value > 128)
    })
}

/// Writes a clip. The container follows the file extension.
pub fn write_clip(path: &Path, spec: &ClipSpec) {
    let mut out = format::output(path).unwrap();
    let codec = encoder::find_by_name(spec.encoder).unwrap();
    let global_header = out.format().flags().contains(format::Flags::GLOBAL_HEADER);

    let mut ctx = codec::Context::new_with_codec(codec)
        .encoder()
        .video()
        .unwrap();
    ctx.set_width(spec.width);
    ctx.set_height(spec.height);
    ctx.set_format(format::Pixel::YUV420P);
    ctx.set_time_base(Rational::new(spec.time_base.0, spec.time_base.1));
    ctx.set_gop(spec.gop);
    ctx.set_max_b_frames(spec.b_frames as usize);
    if global_header {
        ctx.set_flags(codec::Flags::GLOBAL_HEADER);
    }
    let mut options = Dictionary::new();
    for (key, value) in &spec.options {
        options.set(key, value);
    }
    let mut enc = ctx.open_with(options).unwrap();

    let mut stream = out.add_stream(codec).unwrap();
    // SAFETY: copies the opened encoder's parameters into the new stream;
    // both are valid for the duration of the call.
    unsafe {
        let ret =
            ffi::avcodec_parameters_from_context((*stream.as_mut_ptr()).codecpar, enc.as_ptr());
        assert!(ret >= 0);
    }
    stream.set_time_base(Rational::new(spec.time_base.0, spec.time_base.1));
    let mut audio = spec.audio.then(|| SineAudio::add(&mut out, global_header));
    let subtitles = spec.subtitles.then(|| add_subtitles(&mut out, path));
    for (key, value) in &spec.tags {
        out.metadata_mut().set(key, value);
    }
    out.write_header().unwrap();
    let stream_tb = out.stream(0).unwrap().time_base();
    let enc_tb = Rational::new(spec.time_base.0, spec.time_base.1);
    let seconds =
        |pts: i64| pts as f64 * f64::from(enc_tb.numerator()) / f64::from(enc_tb.denominator());

    let mut packet = codec::packet::Packet::empty();
    let mut drain = |enc: &mut encoder::video::Encoder, out: &mut format::context::Output| {
        while enc.receive_packet(&mut packet).is_ok() {
            packet.set_stream(0);
            packet.rescale_ts(enc_tb, stream_tb);
            packet.write_interleaved(out).unwrap();
        }
    };
    let mut picture = frame::Video::new(format::Pixel::YUV420P, spec.width, spec.height);
    for (number, &pts) in spec.pts.iter().enumerate() {
        draw(&mut picture, number as u32);
        picture.set_pts(Some(pts));
        // Each frame lasts until the next one (the last as long as the one
        // before), so the container knows the duration of every frame.
        let next = spec.pts.get(number + 1).copied();
        let previous = number.checked_sub(1).map(|i| spec.pts[i]);
        let duration = match (next, previous) {
            (Some(next), _) => next - pts,
            (None, Some(previous)) => pts - previous,
            (None, None) => 1,
        };
        // SAFETY: setting the duration field of the frame owned here.
        unsafe { (*picture.as_mut_ptr()).duration = duration };
        enc.send_frame(&picture).unwrap();
        drain(&mut enc, &mut out);
        if let Some(audio) = audio.as_mut() {
            audio.write_until(&mut out, seconds(pts + duration));
        }
        if let Some((index, codec_id)) = subtitles
            && number % 30 == 0
        {
            write_subtitle(&mut out, index, codec_id, seconds(pts), number);
        }
    }
    enc.send_eof().unwrap();
    drain(&mut enc, &mut out);
    if let Some(audio) = audio.as_mut() {
        audio.finish(&mut out);
    }
    out.write_trailer().unwrap();
}

/// A 440 Hz stereo tone encoded with FFmpeg's native AAC encoder.
struct SineAudio {
    encoder: *mut ffi::AVCodecContext,
    stream: usize,
    /// Samples encoded so far.
    samples: i64,
}

impl SineAudio {
    const RATE: i32 = 48_000;

    fn add(out: &mut format::context::Output, global_header: bool) -> Self {
        // SAFETY: every FFmpeg object is checked and owned by the returned
        // value (the context) or by the output (the stream).
        unsafe {
            let codec = ffi::avcodec_find_encoder_by_name(c"aac".as_ptr());
            assert!(!codec.is_null());
            let ctx = ffi::avcodec_alloc_context3(codec);
            (*ctx).sample_fmt = ffi::AVSampleFormat::FLTP;
            (*ctx).sample_rate = Self::RATE;
            (*ctx).bit_rate = 128_000;
            (*ctx).time_base = ffi::AVRational {
                num: 1,
                den: Self::RATE,
            };
            ffi::av_channel_layout_default(&raw mut (*ctx).ch_layout, 2);
            if global_header {
                (*ctx).flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER.cast_signed();
            }
            assert!(ffi::avcodec_open2(ctx, codec, ptr::null_mut()) >= 0);
            let stream = ffi::avformat_new_stream(out.as_mut_ptr(), ptr::null());
            assert!(ffi::avcodec_parameters_from_context((*stream).codecpar, ctx) >= 0);
            (*stream).time_base = (*ctx).time_base;
            ffi::av_dict_set(
                &raw mut (*stream).metadata,
                c"language".as_ptr(),
                c"deu".as_ptr(),
                0,
            );
            Self {
                encoder: ctx,
                stream: usize::try_from((*stream).index).unwrap(),
                samples: 0,
            }
        }
    }

    /// Encodes the tone up to `seconds`.
    fn write_until(&mut self, out: &mut format::context::Output, seconds: f64) {
        // SAFETY: reading a field of the open encoder.
        let frame_size = i64::from(unsafe { (*self.encoder).frame_size });
        let target = (seconds * f64::from(Self::RATE)) as i64;
        while self.samples + frame_size <= target {
            self.encode(out, frame_size);
        }
    }

    fn encode(&mut self, out: &mut format::context::Output, count: i64) {
        // SAFETY: the frame gets the encoder's format and a buffer for
        // `count` samples per channel, filled with the tone, sent and freed.
        unsafe {
            let mut frame = ffi::av_frame_alloc();
            (*frame).format = ffi::AVSampleFormat::FLTP.0;
            (*frame).sample_rate = Self::RATE;
            (*frame).nb_samples = i32::try_from(count).unwrap();
            ffi::av_channel_layout_default(&raw mut (*frame).ch_layout, 2);
            assert!(ffi::av_frame_get_buffer(frame, 0) >= 0);
            for channel in 0..2 {
                let data = (*frame).extended_data.add(channel).read();
                for i in 0..count {
                    let t = (self.samples + i) as f32 / Self::RATE as f32;
                    let value = (t * 440.0 * std::f32::consts::TAU).sin() * 0.2;
                    ptr::copy_nonoverlapping(
                        value.to_ne_bytes().as_ptr(),
                        data.add(4 * i as usize),
                        4,
                    );
                }
            }
            (*frame).pts = self.samples;
            self.samples += count;
            assert!(ffi::avcodec_send_frame(self.encoder, frame) >= 0);
            ffi::av_frame_free(&raw mut frame);
        }
        self.drain(out);
    }

    fn drain(&mut self, out: &mut format::context::Output) {
        let mut packet = codec::packet::Packet::empty();
        let stream_tb = out.stream(self.stream).unwrap().time_base();
        loop {
            // SAFETY: the encoder is open; the packet is valid.
            let ret = unsafe {
                ffi::avcodec_receive_packet(
                    self.encoder,
                    ffmpeg_the_third::codec::packet::Mut::as_mut_ptr(&mut packet),
                )
            };
            if ret < 0 {
                break;
            }
            packet.set_stream(self.stream);
            packet.rescale_ts(Rational::new(1, Self::RATE), stream_tb);
            packet.write_interleaved(out).unwrap();
        }
    }

    fn finish(&mut self, out: &mut format::context::Output) {
        // SAFETY: a null frame drains the open encoder.
        unsafe { ffi::avcodec_send_frame(self.encoder, ptr::null()) };
        self.drain(out);
        // SAFETY: the context is owned here and freed once.
        unsafe { ffi::avcodec_free_context(&raw mut self.encoder) };
    }
}

/// Adds a subtitle stream: MP4 timed text for MP4/MOV, `SubRip` otherwise.
/// Returns its index and codec.
fn add_subtitles(out: &mut format::context::Output, path: &Path) -> (usize, ffi::AVCodecID) {
    let mp4 = matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("mp4" | "mov")
    );
    let codec_id = if mp4 {
        ffi::AVCodecID::MOV_TEXT
    } else {
        ffi::AVCodecID::SUBRIP
    };
    // SAFETY: the new stream belongs to the output; plain fields of its
    // parameters are set before the header.
    unsafe {
        let stream = ffi::avformat_new_stream(out.as_mut_ptr(), ptr::null());
        (*(*stream).codecpar).codec_type = ffi::AVMediaType::SUBTITLE;
        (*(*stream).codecpar).codec_id = codec_id;
        (*stream).time_base = ffi::AVRational { num: 1, den: 1000 };
        (usize::try_from((*stream).index).unwrap(), codec_id)
    }
}

/// Writes one subtitle line at `seconds`, shown for half a second.
fn write_subtitle(
    out: &mut format::context::Output,
    stream: usize,
    codec_id: ffi::AVCodecID,
    seconds: f64,
    number: usize,
) {
    let text = format!("line {number}");
    let mut data = Vec::new();
    if codec_id == ffi::AVCodecID::MOV_TEXT {
        data.extend_from_slice(&u16::try_from(text.len()).unwrap().to_be_bytes());
    }
    data.extend_from_slice(text.as_bytes());
    let mut packet = codec::packet::Packet::copy(&data);
    let tb = out.stream(stream).unwrap().time_base();
    let ticks = |s: f64| (s * f64::from(tb.denominator()) / f64::from(tb.numerator())) as i64;
    packet.set_pts(Some(ticks(seconds)));
    packet.set_dts(Some(ticks(seconds)));
    packet.set_duration(ticks(0.5));
    packet.set_stream(stream);
    packet.set_flags(codec::packet::Flags::KEY);
    packet.write_interleaved(out).unwrap();
}

/// One packet as `(PTS, DTS, duration, data)`.
pub type PacketRecord = (Option<i64>, Option<i64>, i64, Vec<u8>);

/// The packets of the first stream of `medium` in file order, including
/// packets the container hides with an edit list.
pub fn stream_packets(path: &Path, medium: ffmpeg_the_third::media::Type) -> Vec<PacketRecord> {
    let mut input = format::input(path).unwrap();
    let Some(index) = input.streams().best(medium).map(|s| s.index()) else {
        return Vec::new();
    };
    let mut packets = Vec::new();
    for item in input.packets() {
        let (stream, packet) = item.unwrap();
        if stream.index() == index {
            packets.push((
                packet.pts(),
                packet.dts(),
                packet.duration(),
                packet.data().unwrap_or_default().to_vec(),
            ));
        }
    }
    packets
}

/// The top-level boxes of an MP4/MOV file, in file order.
pub fn top_level_boxes(path: &Path) -> Vec<String> {
    let data = std::fs::read(path).unwrap();
    let mut boxes = Vec::new();
    let mut at = 0usize;
    while at + 8 <= data.len() {
        let size32 = u32::from_be_bytes(data[at..at + 4].try_into().unwrap());
        boxes.push(String::from_utf8_lossy(&data[at + 4..at + 8]).into_owned());
        let size = match size32 {
            1 => usize::try_from(u64::from_be_bytes(
                data[at + 8..at + 16].try_into().unwrap(),
            ))
            .unwrap(),
            0 => data.len() - at,
            n => n as usize,
        };
        at += size.max(8);
    }
    boxes
}

/// Number of video packets the demuxer presents (not marked as discarded by
/// an edit list), counted independently of blurrd's probe and decoder.
pub fn presented_packets(path: &Path) -> u32 {
    let mut input = format::input(path).unwrap();
    let video = input
        .streams()
        .best(ffmpeg_the_third::media::Type::Video)
        .unwrap()
        .index();
    let mut count = 0;
    for item in input.packets() {
        let (stream, packet) = item.unwrap();
        // SAFETY: reading the flags of a valid packet.
        let flags = unsafe { (*ffmpeg_the_third::codec::packet::Ref::as_ptr(&packet)).flags };
        if stream.index() == video && flags & ffi::AV_PKT_FLAG_DISCARD == 0 {
            count += 1;
        }
    }
    count
}

/// Decodes a file in software and returns its PTS table and keyframe
/// indices (what an analysis would store).
pub fn decode_table(path: &Path) -> (blurrd_video::PtsTable, Vec<u32>) {
    let mut decoder =
        blurrd_video::Decoder::open(path, &blurrd_video::DecodeOptions::default()).unwrap();
    let (mut pts, mut keys) = (Vec::new(), Vec::new());
    while let Some(frame) = decoder.next_frame().unwrap() {
        if frame.is_key() {
            keys.push(frame.index());
        }
        pts.push(frame.pts());
    }
    (
        blurrd_video::PtsTable::new(decoder.time_base(), pts).unwrap(),
        keys,
    )
}

/// A box table with one manual box `rect` on frames `f0..=f1` of a video
/// with `frames` frames (no boxes when `f0 > f1`).
pub fn box_table(
    frames: u32,
    f0: u32,
    f1: u32,
    rect: blurrd_core::RectI,
) -> blurrd_core::FrameBoxTable {
    let overrides = if f0 <= f1 {
        vec![blurrd_core::Override::Add {
            uid: uuid::Uuid::from_u128(1),
            region: blurrd_core::Region::Manual,
            f0,
            f1,
            rect,
        }]
    } else {
        Vec::new()
    };
    blurrd_core::BlurPlan {
        frame_count: frames,
        tracks: Vec::new(),
        overrides,
    }
    .frame_table(&blurrd_core::RegionSet::all())
}

/// Blur parameters for a source with the given colour tags and height.
pub fn pixel_params(info: &blurrd_core::MediaInfo) -> blurrd_blur::PixelParams {
    let colour = blurrd_video::VideoColour::from_info(&info.color, info.height);
    blurrd_blur::PixelParams::new(
        &blurrd_core::BlurParams::default(),
        (info.height as f32 / 1080.0).max(blurrd_blur::MIN_SCALE),
        colour.yuv_colour(),
    )
    .unwrap()
}

/// Every picture of `path`, decoded in software: luma, then the U and V
/// planes, without padding (8-bit sources).
pub fn pictures(path: &Path) -> Vec<Vec<u8>> {
    let mut decoder =
        blurrd_video::Decoder::open(path, &blurrd_video::DecodeOptions::default()).unwrap();
    let mut out = Vec::new();
    let mut roi = blurrd_video::YuvRoi::new();
    while let Some(frame) = decoder.next_frame().unwrap() {
        let full = blurrd_core::RectI::new(
            0,
            0,
            frame.width().cast_signed(),
            frame.height().cast_signed(),
        )
        .unwrap();
        frame.roi_yuv(full, &mut roi).unwrap();
        let (u, v) = roi.chroma();
        let mut picture = roi.luma().to_vec();
        picture.extend_from_slice(u);
        picture.extend_from_slice(v.unwrap_or_default());
        out.push(picture);
    }
    out
}
