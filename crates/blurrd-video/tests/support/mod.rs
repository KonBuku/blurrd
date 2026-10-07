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
    out.write_header().unwrap();
    let stream_tb = out.stream(0).unwrap().time_base();
    let enc_tb = Rational::new(spec.time_base.0, spec.time_base.1);

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
        enc.send_frame(&picture).unwrap();
        drain(&mut enc, &mut out);
    }
    enc.send_eof().unwrap();
    drain(&mut enc, &mut out);
    out.write_trailer().unwrap();
}
