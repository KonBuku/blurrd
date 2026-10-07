//! Probing an input: streams, colour, B-frames, edit lists and keyframes.

use std::ffi::{CStr, c_char};
use std::path::Path;

use blurrd_core::{AudioStreamInfo, MediaInfo, Rational};
use ffmpeg_the_third::codec::packet::Ref as _;
use ffmpeg_the_third::{ffi, format, media};

use crate::colour::{color_info, hdr_kind};
use crate::{VideoError, input, mp4};

/// Reads everything blurrd needs to know about a video file.
///
/// Demuxes the whole video stream once (no decoding) to find the keyframes
/// as presentation frame indices, whether frames are reordered (B-frames)
/// and whether the frame rate varies. Packets that the container marks as
/// discarded (cut by an edit list) are not counted.
///
/// # Errors
///
/// [`VideoError::Io`] when the file cannot be read,
/// [`VideoError::NoVideoStream`] or [`VideoError::Unsupported`] for files
/// blurrd cannot process, [`VideoError::Ffmpeg`] when demuxing fails.
pub fn probe(path: &Path) -> Result<MediaInfo, VideoError> {
    crate::init()?;
    let size = std::fs::metadata(path)
        .map_err(|source| VideoError::Io {
            context: "reading",
            path: path.to_path_buf(),
            source,
        })?
        .len();
    let mut input = input::open(path)?;
    let video = input::video_stream(&input)?;

    let streams = describe_streams(&input, video)?;
    let scan = scan_packets(&mut input, video)?;

    let demuxer = input.format().name().to_owned();
    let mov_family = demuxer
        .split(',')
        .any(|name| name == "mov" || name == "mp4");
    let elst = if mov_family {
        let mut file = std::fs::File::open(path).map_err(|source| VideoError::Io {
            context: "reading",
            path: path.to_path_buf(),
            source,
        })?;
        mp4::has_edit_list(&mut file).map_err(|source| VideoError::Io {
            context: "reading",
            path: path.to_path_buf(),
            source,
        })?
    } else {
        false
    };

    let duration_s = if input.duration() > 0 {
        input.duration() as f64 / f64::from(ffi::AV_TIME_BASE)
    } else {
        streams.duration_s
    };
    let bitrate = u64::try_from(input.bit_rate()).ok().filter(|&b| b > 0);

    Ok(MediaInfo {
        path: path.to_path_buf(),
        size,
        duration_s,
        width: streams.width,
        height: streams.height,
        // A constant-rate video reports its nominal rate; FFmpeg's average
        // is skewed by the missing duration of the last frame. A variable
        // rate video reports the average.
        fps: if scan.variable_rate {
            streams.avg_fps.or(streams.base_fps)
        } else {
            streams.base_fps.or(streams.avg_fps)
        }
        .ok_or_else(|| VideoError::Unsupported(String::from("the video has no frame rate")))?,
        time_base: streams.time_base,
        frame_count: streams.frame_count,
        video_codec: streams.codec,
        profile: streams.profile,
        bit_depth: streams.bit_depth,
        hdr: hdr_kind(&streams.color.transfer),
        color: streams.color,
        has_b_frames: streams.reorder || scan.reordered,
        has_edit_list: elst || scan.discarded > 0,
        keyframes: Some(scan.keyframes),
        bitrate,
        audio: streams.audio,
        has_subtitles: streams.has_subtitles,
        is_vfr: scan.variable_rate,
    })
}

/// Facts read from the stream headers.
struct Streams {
    width: u32,
    height: u32,
    /// Average frame rate (frames / duration), if known.
    avg_fps: Option<Rational>,
    /// Lowest rate that represents every timestamp exactly (`r_frame_rate`).
    base_fps: Option<Rational>,
    time_base: Rational,
    frame_count: Option<u32>,
    duration_s: f64,
    codec: String,
    profile: String,
    bit_depth: u8,
    color: blurrd_core::ColorInfo,
    reorder: bool,
    audio: Vec<AudioStreamInfo>,
    has_subtitles: bool,
}

/// Converts an FFmpeg rational to a positive [`Rational`], if it is one.
pub(crate) fn positive_rational(r: ffmpeg_the_third::Rational) -> Option<Rational> {
    let (num, den) = (
        u32::try_from(r.numerator()).ok()?,
        u32::try_from(r.denominator()).ok()?,
    );
    if num == 0 {
        return None;
    }
    Rational::new(num, den).ok()
}

fn c_str(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: FFmpeg name functions return null or a static NUL-terminated
    // string.
    Some(
        unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned(),
    )
}

/// Bits per luma sample of a pixel format from the codec parameters.
fn pixel_format_depth(format: i32) -> Option<u8> {
    // SAFETY: `av_pix_fmt_desc_get` accepts any value and returns null or a
    // pointer to a static descriptor.
    let desc = unsafe { ffi::av_pix_fmt_desc_get(ffi::AVPixelFormat(format)) };
    if desc.is_null() {
        return None;
    }
    // SAFETY: `desc` points to a valid static descriptor.
    u8::try_from(unsafe { (*desc).comp[0].depth }).ok()
}

fn describe_streams(input: &format::context::Input, video: usize) -> Result<Streams, VideoError> {
    let stream = input.stream(video).ok_or(VideoError::NoVideoStream)?;
    let params = stream.parameters();
    // SAFETY: the parameters pointer is valid while `stream` (borrowed from
    // `input`) lives; it is only read here.
    let par = unsafe { &*params.as_ptr() };
    let (width, height) = (
        u32::try_from(par.width).unwrap_or(0),
        u32::try_from(par.height).unwrap_or(0),
    );
    if width == 0 || height == 0 {
        return Err(VideoError::Unsupported(String::from(
            "the video has no picture size",
        )));
    }
    let time_base = positive_rational(stream.time_base()).ok_or_else(|| {
        VideoError::Unsupported(String::from("the video stream has no time base"))
    })?;
    let avg_fps = positive_rational(stream.avg_frame_rate());
    let base_fps = positive_rational(stream.rate());
    if avg_fps.is_none() && base_fps.is_none() {
        return Err(VideoError::Unsupported(String::from(
            "the video has no frame rate",
        )));
    }

    // SAFETY: plain lookups into FFmpeg's static codec table; every codec id
    // is accepted and unknown values return null.
    let (codec, profile) = unsafe {
        (
            c_str(ffi::avcodec_get_name(par.codec_id)).unwrap_or_default(),
            c_str(ffi::avcodec_profile_name(par.codec_id, par.profile)).unwrap_or_default(),
        )
    };
    let bit_depth = pixel_format_depth(par.format).unwrap_or(8);

    let duration_s = if stream.duration() > 0 {
        stream.duration() as f64 * f64::from(time_base.num) / f64::from(time_base.den)
    } else {
        0.0
    };

    let mut audio = Vec::new();
    let mut has_subtitles = false;
    for other in input.streams() {
        let p = other.parameters();
        match p.medium() {
            media::Type::Audio => {
                // SAFETY: as above, read-only access while `input` lives.
                let ap = unsafe { &*p.as_ptr() };
                audio.push(AudioStreamInfo {
                    index: other.index() as u32,
                    // SAFETY: static codec name table.
                    codec: c_str(unsafe { ffi::avcodec_get_name(ap.codec_id) }).unwrap_or_default(),
                    channels: u16::try_from(ap.ch_layout.nb_channels).unwrap_or(0),
                    sample_rate: u32::try_from(ap.sample_rate).unwrap_or(0),
                    bitrate: u64::try_from(ap.bit_rate).ok().filter(|&b| b > 0),
                    language: other.metadata().get("language").map(str::to_owned),
                });
            }
            media::Type::Subtitle => has_subtitles = true,
            _ => {}
        }
    }

    Ok(Streams {
        width,
        height,
        avg_fps,
        base_fps,
        time_base,
        frame_count: u32::try_from(stream.frames()).ok().filter(|&n| n > 0),
        duration_s,
        codec,
        profile,
        bit_depth,
        color: color_info(par),
        reorder: par.video_delay > 0,
        audio,
        has_subtitles,
    })
}

/// Facts found by demuxing every video packet.
struct Scan {
    keyframes: Vec<u32>,
    reordered: bool,
    discarded: u32,
    variable_rate: bool,
}

fn scan_packets(input: &mut format::context::Input, video: usize) -> Result<Scan, VideoError> {
    // (pts, keyframe) of every presented packet, in decode order.
    let mut packets: Vec<(i64, bool)> = Vec::new();
    let mut discarded = 0;
    let mut reordered = false;
    let mut max_pts = i64::MIN;
    for item in input.packets() {
        let (stream, packet) = item.map_err(VideoError::ffmpeg("reading packets"))?;
        if stream.index() != video {
            continue;
        }
        // SAFETY: reading the flags field of a valid packet.
        let flags = unsafe { (*packet.as_ptr()).flags };
        if flags & ffi::AV_PKT_FLAG_DISCARD != 0 {
            discarded += 1;
            continue;
        }
        let Some(pts) = packet.pts().or(packet.dts()) else {
            continue;
        };
        if pts < max_pts {
            reordered = true;
        }
        max_pts = max_pts.max(pts);
        packets.push((pts, packet.is_key()));
    }
    let mut order: Vec<i64> = packets.iter().map(|&(pts, _)| pts).collect();
    order.sort_unstable();
    order.dedup();
    let mut keyframes: Vec<u32> = packets
        .iter()
        .filter(|&&(_, key)| key)
        .filter_map(|&(pts, _)| order.binary_search(&pts).ok().map(|i| i as u32))
        .collect();
    keyframes.sort_unstable();
    keyframes.dedup();
    Ok(Scan {
        keyframes,
        reordered,
        discarded,
        variable_rate: is_variable_rate(&order),
    })
}

/// Whether frame durations vary: any gap between consecutive presentation
/// timestamps differs from the median gap by more than one tick and more
/// than 10 %. One-tick jitter (16/17 ms at a 1 ms time base) is constant
/// rate.
pub(crate) fn is_variable_rate(sorted_pts: &[i64]) -> bool {
    let mut gaps: Vec<i64> = sorted_pts.windows(2).map(|w| w[1] - w[0]).collect();
    if gaps.len() < 2 {
        return false;
    }
    gaps.sort_unstable();
    let median = gaps[gaps.len() / 2];
    let tolerance = (median / 10).max(1);
    gaps.iter().any(|&gap| (gap - median).abs() > tolerance)
}

#[cfg(test)]
mod tests {
    use super::is_variable_rate;

    #[test]
    fn detects_variable_frame_rates() {
        let cfr: Vec<i64> = (0..100).map(|i| i * 256).collect();
        assert!(!is_variable_rate(&cfr));
        let jitter: Vec<i64> = (0..100).map(|i| i * 50 / 3).collect();
        assert!(!is_variable_rate(&jitter));
        let mut dropped = cfr.clone();
        dropped.remove(50);
        assert!(is_variable_rate(&dropped));
        assert!(!is_variable_rate(&[0]));
    }
}
