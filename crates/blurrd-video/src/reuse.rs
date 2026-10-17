//! Rendering a video again with only the parts whose boxes changed
//! encoded anew.
//!
//! When the verification of a streamed run adds fix boxes, most of the
//! output is already right. Every source keyframe is an IDR frame of the
//! output ([`blurrd_core::KeyframePolicy::MatchSource`]) and the encoder
//! adds no keyframes of its own, so the output's GOPs are closed and
//! start where the source's do. The render then encodes only the GOPs
//! that hold a changed frame ([`runs`]) and copies the packets of every
//! other frame from the earlier output ([`Previous`]): those parts of
//! the file are byte for byte the earlier ones. This needs codec headers
//! that match the earlier output's ([`same_headers`]); otherwise the
//! whole video is encoded again. Containers store H.264 samples with
//! length prefixes, encoders make Annex B packets, and the muxer turns
//! the latter into the former: copied H.264 packets go back to Annex B
//! ([`annex_b`]) so the muxer writes them as it wrote them before.

use std::ops::Range;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use crossbeam_channel::Sender;
use ffmpeg_the_third::{codec, format};

use crate::segments::Segment;
use crate::{EncodedPacket, PtsTable, VideoEncoder, VideoError, input};

/// Frames the copied packets may run ahead of the muxer at most, so the
/// packets waiting for an encoded part stay within some megabytes.
const COPY_LEAD: u32 = 600;
/// How often a copy that ran ahead looks whether it may go on.
const COPY_POLL: Duration = Duration::from_millis(2);

/// The keyframe-aligned runs of frames to encode again in a video of
/// `count` frames with the source keyframes `keyframes` (sorted): every
/// GOP that holds a frame of `changed`, neighbouring GOPs merged.
pub(crate) fn runs(changed: &[Range<u32>], keyframes: &[u32], count: u32) -> Vec<Segment> {
    let mut runs: Vec<Segment> = changed
        .iter()
        .map(|r| r.start.min(count)..r.end.min(count))
        .filter(|r| !r.is_empty())
        .map(|r| {
            let start = keyframes
                .iter()
                .rev()
                .copied()
                .find(|&k| k <= r.start)
                .unwrap_or(0);
            let end = keyframes
                .iter()
                .copied()
                .find(|&k| k >= r.end)
                .unwrap_or(count);
            Segment { start, end }
        })
        .collect();
    runs.sort_by_key(|run| run.start);
    let mut merged: Vec<Segment> = Vec::with_capacity(runs.len());
    for run in runs {
        match merged.last_mut() {
            Some(last) if run.start <= last.end => last.end = last.end.max(run.end),
            _ => merged.push(run),
        }
    }
    merged
}

/// Whether the codec headers `a` and `b` describe the same stream: equal
/// bytes, or for H.264 (`h264`) the same parameter sets, one of them in
/// the `avcC` form a container stores and the other as the Annex B
/// headers an encoder makes.
pub(crate) fn same_headers(a: &[u8], b: &[u8], h264: bool) -> bool {
    if a == b {
        return true;
    }
    h264 && matches!(
        (parameter_sets(a), parameter_sets(b)),
        (Some(a), Some(b)) if !a.is_empty() && a == b
    )
}

/// The SPS and PPS NAL units of H.264 codec headers in `avcC` or Annex B
/// form, in order; `None` when `data` is neither.
fn parameter_sets(data: &[u8]) -> Option<Vec<&[u8]>> {
    if data.first() == Some(&1) {
        return avcc_sets(data);
    }
    if !data.starts_with(&[0, 0, 1]) && !data.starts_with(&[0, 0, 0, 1]) {
        return None;
    }
    let mut units = Vec::new();
    let mut rest = data;
    while let Some(start) = find_start_code(rest) {
        rest = &rest[start + 3..];
        let end = find_start_code(rest).unwrap_or(rest.len());
        let mut unit = &rest[..end];
        // The zero before a four-byte start code is not part of the unit.
        while let [head @ .., 0] = unit {
            unit = head;
        }
        if !unit.is_empty() {
            units.push(unit);
        }
        rest = &rest[end..];
    }
    Some(units)
}

/// The position of the next `00 00 01` start code in `data`.
fn find_start_code(data: &[u8]) -> Option<usize> {
    data.windows(3).position(|w| w == [0, 0, 1])
}

/// The NAL units of a sample with `length`-byte length prefixes, each
/// behind an Annex B start code; `None` when a prefix runs past the end.
fn annex_b(sample: &[u8], length: usize) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(sample.len() + 16);
    let mut rest = sample;
    while !rest.is_empty() {
        let (prefix, tail) = rest.split_at_checked(length)?;
        let size = prefix
            .iter()
            .fold(0usize, |size, &byte| (size << 8) | usize::from(byte));
        let (unit, tail) = tail.split_at_checked(size)?;
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(unit);
        rest = tail;
    }
    Some(out)
}

/// The SPS and PPS NAL units of an `avcC` record.
fn avcc_sets(data: &[u8]) -> Option<Vec<&[u8]>> {
    let mut units = Vec::new();
    let mut pos = 5;
    for mask in [0x1f, 0xff] {
        let count = data.get(pos)? & mask;
        pos += 1;
        for _ in 0..count {
            let len = usize::from(u16::from_be_bytes([*data.get(pos)?, *data.get(pos + 1)?]));
            pos += 2;
            units.push(data.get(pos..pos + len)?);
            pos += len;
        }
    }
    Some(units)
}

/// The video stream of an earlier render of the same video with the same
/// encoder settings, whose packets are copied.
pub(crate) struct Previous {
    input: format::context::Input,
    stream: usize,
    /// The size of the length prefixes of its H.264 samples, when they
    /// have them (`avcC` codec headers).
    nal_length: Option<usize>,
}

impl Previous {
    /// Opens the earlier output at `path` for a render of `frames` frames
    /// with `encoder`. Fails with the reason when its packets cannot be
    /// used: no video stream, another frame count, or codec headers that
    /// differ from `encoder`'s.
    pub(crate) fn open(path: &Path, encoder: &VideoEncoder, frames: u32) -> Result<Self, String> {
        let input = input::open(path).map_err(|e| e.to_string())?;
        let stream = input::video_stream(&input).map_err(|e| e.to_string())?;
        let video = input
            .stream(stream)
            .ok_or_else(|| String::from("the earlier output has no video stream"))?;
        let counted = video.frames();
        if counted > 0 && counted != i64::from(frames) {
            return Err(format!(
                "the earlier output has {counted} frames, the video {frames}"
            ));
        }
        let parameters = video.parameters();
        // SAFETY: `extradata` and `extradata_size` of a stream of an open
        // input describe its codec headers (or are null and zero).
        let headers = unsafe {
            let par = parameters.as_ptr();
            let size = usize::try_from((*par).extradata_size).unwrap_or(0);
            if (*par).extradata.is_null() || size == 0 {
                &[][..]
            } else {
                std::slice::from_raw_parts((*par).extradata, size)
            }
        };
        let h264 = encoder.codec() == blurrd_core::Codec::H264;
        if !same_headers(headers, encoder.extradata(), h264) {
            return Err(String::from(
                "the earlier output's codec headers differ from the encoder's",
            ));
        }
        let nal_length = match headers {
            [1, _, _, _, size, ..] if h264 => Some(usize::from(size & 3) + 1),
            _ => None,
        };
        Ok(Self {
            input,
            stream,
            nal_length,
        })
    }

    /// Sends the packet of every frame outside `runs` (sorted) to
    /// `packets`, with the timestamps of `pts` (in the encoder's time
    /// base, as the encoder's packets have them), never more than
    /// [`COPY_LEAD`] frames ahead of `muxed`, the frames written so far.
    /// Stops early when `aborted` says so or the muxer went away.
    ///
    /// # Errors
    ///
    /// [`VideoError::FrameCountMismatch`] when the earlier output has
    /// other frames; [`VideoError::Unsupported`] when a copied part does
    /// not start with a keyframe; reading errors.
    pub(crate) fn copy(
        mut self,
        runs: &[Segment],
        pts: &PtsTable,
        packets: &Sender<EncodedPacket>,
        muxed: &AtomicU32,
        aborted: &dyn Fn() -> bool,
    ) -> Result<(), VideoError> {
        let mut packet = codec::packet::Packet::empty();
        let mut index = 0u32;
        let mut run = 0usize;
        let mut last_copied: Option<u32> = None;
        loop {
            if aborted() {
                return Ok(());
            }
            match packet.read(&mut self.input) {
                Ok(()) => {}
                Err(ffmpeg_the_third::Error::Eof) => break,
                Err(error) => return Err(VideoError::ffmpeg("reading the earlier output")(error)),
            }
            if packet.stream() != self.stream {
                continue;
            }
            let this = index;
            index += 1;
            while runs.get(run).is_some_and(|r| r.end <= this) {
                run += 1;
            }
            if runs.get(run).is_some_and(|r| r.start <= this) {
                continue;
            }
            let Some(stamp) = pts.pts(this) else {
                return Err(VideoError::FrameCountMismatch {
                    expected: pts.len(),
                    found: index,
                });
            };
            if last_copied != this.checked_sub(1) && !packet.is_key() {
                return Err(VideoError::Unsupported(format!(
                    "frame {this} of the earlier output is no keyframe"
                )));
            }
            last_copied = Some(this);
            while this > muxed.load(Ordering::Relaxed).saturating_add(COPY_LEAD) {
                if aborted() {
                    return Ok(());
                }
                std::thread::sleep(COPY_POLL);
            }
            let mut out = match self.nal_length {
                Some(length) => {
                    let data =
                        annex_b(packet.data().unwrap_or_default(), length).ok_or_else(|| {
                            VideoError::Unsupported(format!(
                                "frame {this} of the earlier output is malformed"
                            ))
                        })?;
                    let mut converted = codec::packet::Packet::copy(&data);
                    converted.set_flags(packet.flags());
                    converted
                }
                None => std::mem::replace(&mut packet, codec::packet::Packet::empty()),
            };
            out.set_pts(Some(stamp));
            out.set_dts(Some(stamp));
            out.set_position(-1);
            if packets.send(EncodedPacket::copied(out)).is_err() {
                return Ok(());
            }
        }
        if index != pts.len() {
            return Err(VideoError::FrameCountMismatch {
                expected: pts.len(),
                found: index,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{annex_b, parameter_sets, runs, same_headers};
    use crate::segments::Segment;

    const SPS: [u8; 4] = [0x67, 0x64, 0x00, 0x28];
    const PPS: [u8; 3] = [0x68, 0xee, 0x3c];

    fn annex_b_headers() -> Vec<u8> {
        [&[0, 0, 0, 1][..], &SPS, &[0, 0, 0, 1], &PPS].concat()
    }

    fn avcc() -> Vec<u8> {
        [
            &[1, 0x64, 0x00, 0x28, 0xff, 0xe1, 0, 4][..],
            &SPS,
            &[1, 0, 3],
            &PPS,
        ]
        .concat()
    }

    #[test]
    fn runs_cover_whole_gops_and_merge() {
        let keys = [0, 60, 120, 180, 240];
        assert_eq!(runs(&[], &keys, 300), []);
        assert_eq!(
            runs(std::slice::from_ref(&(70..75)), &keys, 300),
            [Segment {
                start: 60,
                end: 120
            }]
        );
        // A range ending on a keyframe does not take the next GOP.
        assert_eq!(
            runs(std::slice::from_ref(&(60..120)), &keys, 300),
            [Segment {
                start: 60,
                end: 120
            }]
        );
        // Neighbouring and overlapping GOPs merge; the last GOP ends with
        // the video.
        assert_eq!(
            runs(&[130..140, 100..125, 250..400], &keys, 300),
            [
                Segment {
                    start: 60,
                    end: 180
                },
                Segment {
                    start: 240,
                    end: 300
                }
            ]
        );
        // Without keyframes the whole video is one GOP.
        assert_eq!(
            runs(std::slice::from_ref(&(5..6)), &[], 50),
            [Segment { start: 0, end: 50 }]
        );
    }

    #[test]
    fn length_prefixed_samples_become_annex_b() {
        let sample = [&[0, 0, 0, 4][..], &SPS, &[0, 0, 0, 3], &PPS].concat();
        assert_eq!(annex_b(&sample, 4), Some(annex_b_headers()));
        assert_eq!(
            annex_b(&[0, 2, 9, 9, 0, 1, 7], 2),
            Some(vec![0, 0, 0, 1, 9, 9, 0, 0, 0, 1, 7])
        );
        assert_eq!(annex_b(&[0, 0, 0, 9, 1], 4), None);
        assert_eq!(annex_b(&[], 4), Some(Vec::new()));
    }

    #[test]
    fn h264_headers_match_in_both_forms() {
        assert_eq!(
            parameter_sets(&annex_b_headers()),
            Some(vec![&SPS[..], &PPS[..]])
        );
        assert_eq!(parameter_sets(&avcc()), Some(vec![&SPS[..], &PPS[..]]));
        assert!(same_headers(&avcc(), &annex_b_headers(), true));
        assert!(!same_headers(&avcc(), &annex_b_headers(), false));
        let mut other = annex_b_headers();
        *other.last_mut().unwrap() ^= 1;
        assert!(!same_headers(&avcc(), &other, true));
        assert!(!same_headers(&[], &[2, 3], true));
        assert!(same_headers(&[2, 3], &[2, 3], false));
    }
}
