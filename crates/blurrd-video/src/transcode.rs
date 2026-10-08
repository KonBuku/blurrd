//! The render: decode, blur, encode and mux on separate threads.
//!
//! ```text
//! session 1: decode thread ──frames──► blur+encode thread ──┐
//! session 2: decode thread ──frames──► blur+encode thread ──┴─packets──► mux (caller's thread)
//! ```
//!
//! Each encoder session has its own decode and blur+encode threads. With
//! one session it decodes the whole video; with two (H.264 on NVENC, whose
//! GPUs have two encoder engines) the video is split into keyframe-aligned
//! segments that the sessions take in turn (see [`crate::segments`]), and
//! the muxer stitches their packets back into frame order.
//!
//! The decode threads check every frame's PTS against the project's PTS
//! table, so a box is always drawn on the picture it was planned for. The
//! blur+encode threads copy a frame into an encoder-owned frame only when
//! it has boxes (or the encoder needs another layout), blur the copy and
//! send it; frames without boxes go to the encoder untouched. Every frame
//! that was a keyframe in the source (and the first frame of every segment)
//! becomes an IDR frame, so GOPs are closed, and the encoder adds no
//! keyframes of its own: the output has the source's keyframes and
//! timestamps. The muxer writes into `<name>.tmp.<ext>` and renames the
//! file when everything succeeded; the input is never written.
//!
//! Queues between the stages are bounded, so memory stays constant for any
//! clip length, and every stage stops at the next frame when another one
//! fails or the job is canceled.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use blurrd_blur::{BlurBackend, PixelParams, prepare_boxes};
use blurrd_core::{EncoderSetting, FrameBoxTable, KeyframePolicy, MediaInfo, NvencSessions};
use crossbeam_channel::{Receiver, Sender, bounded};
use ffmpeg_the_third::Dictionary;

use crate::frames::FramePool;
use crate::mux::Muxer;
use crate::segments::{Scheduler, Segment};
use crate::{
    DecodeOptions, Decoder, EncodeOptions, EncodedPacket, EncoderFallback, EncoderInput, PtsTable,
    VideoEncoder, VideoError, VideoFrame,
};

/// Decoded frames waiting for an encoder, per session.
const FRAME_QUEUE: usize = 8;
/// Encoded packets waiting for the muxer, per session.
const PACKET_QUEUE: usize = 64;
/// Default size of the output write buffer.
const DEFAULT_AVIO_BUFFER: usize = 4 * 1024 * 1024;
/// Containers the output may use, by file extension.
const CONTAINERS: [&str; 3] = ["mp4", "mov", "mkv"];
/// The encoder that runs as two sessions when allowed.
const SPLIT_ENCODER: &str = "h264_nvenc";

/// How the output file is written.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct MuxOptions {
    /// Size of the write buffer in bytes (clamped to 64 KB – 16 MB).
    pub avio_buffer: usize,
}

impl Default for MuxOptions {
    fn default() -> Self {
        Self {
            avio_buffer: DEFAULT_AVIO_BUFFER,
        }
    }
}

/// What a finished render did.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct TranscodeReport {
    /// The written file.
    pub output: PathBuf,
    /// Frames written.
    pub frames: u32,
    /// Frames that had at least one box.
    pub blurred_frames: u32,
    /// Time spent making those frames writable and blurring them (summed
    /// over the sessions).
    pub blur_time: Duration,
    /// The encoder used, for example `h264_nvenc`.
    pub encoder: String,
    /// Encoder sessions that ran in parallel (1 or 2).
    pub sessions: u8,
    /// Encoders that failed to open before it.
    pub fallbacks: Vec<EncoderFallback>,
    /// Wall-clock time of [`Transcoder::run`].
    pub elapsed: Duration,
}

/// One encoder session: a decoder and an encoder of its own.
struct Session {
    decoder: Decoder,
    /// Frame 0, decoded while opening (first session only).
    first: Option<VideoFrame>,
    encoder: VideoEncoder,
}

/// A render job: opened (decoders and encoders chosen), then run once.
pub struct Transcoder {
    pts: Arc<PtsTable>,
    output: PathBuf,
    temp: PathBuf,
    mux: MuxOptions,
    sessions: Vec<Session>,
    fallbacks: Vec<EncoderFallback>,
    /// Source keyframes as presentation indices, sorted (segment starts).
    source_keyframes: Vec<u32>,
    /// Whether every source keyframe becomes an IDR frame.
    match_keyframes: bool,
}

impl std::fmt::Debug for Transcoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Transcoder")
            .field("output", &self.output)
            .field("encoder", &self.encoder())
            .field("sessions", &self.sessions.len())
            .finish_non_exhaustive()
    }
}

impl Transcoder {
    /// Prepares the render of the video `info` describes (as returned by
    /// [`crate::probe()`]) into `output`, whose extension picks the
    /// container (`.mp4`, `.mov` or `.mkv`).
    ///
    /// Opens the decoder, decodes the first frame and opens the encoder, so
    /// the encoder (and any fallback) is known before the render starts.
    /// For H.264 on NVENC (unless [`EncodeOptions::sessions`] says one), a
    /// second session is opened; it is used only if both sessions produce
    /// byte-identical codec headers, because the stitched stream has one.
    /// `pts` is the project's PTS table of the video; the render checks
    /// every decoded frame against it.
    ///
    /// # Errors
    ///
    /// [`VideoError::Unsupported`] when `output` is the input, has no
    /// supported extension, or the video has no frames;
    /// [`VideoError::PtsMismatch`] when the first frame does not match
    /// `pts`; the errors of [`Decoder::open`] and [`VideoEncoder::open`].
    pub fn open(
        info: &MediaInfo,
        pts: Arc<PtsTable>,
        output: &Path,
        decode: &DecodeOptions,
        encode: &EncodeOptions,
        mux: MuxOptions,
    ) -> Result<Self, VideoError> {
        let temp = temp_path(&info.path, output)?;
        let mut decoder = Decoder::open(&info.path, decode)?;
        let first = decoder
            .next_frame()?
            .ok_or_else(|| VideoError::Unsupported(String::from("the video has no frames")))?;
        check_frame(&first, &pts)?;

        let mut input = EncoderInput::from_media(info, first.layout());
        let aspect = first.raw().aspect_ratio();
        input.sample_aspect = blurrd_core::Rational::new(
            u32::try_from(aspect.numerator()).unwrap_or(0),
            u32::try_from(aspect.denominator()).unwrap_or(0),
        )
        .ok()
        .filter(|r| r.num > 0);
        let (encoder, fallbacks) = VideoEncoder::open(&input, encode)?;
        let mut source_keyframes = info.keyframes.clone().unwrap_or_default();
        source_keyframes.sort_unstable();

        let split = runs_two_sessions(encoder.name(), encode.sessions)
            && Scheduler::worth_splitting(pts.len());
        let mut sessions = vec![Session {
            decoder,
            first: Some(first),
            encoder,
        }];
        if split {
            match second_session(info, &input, decode, encode, &sessions[0].encoder) {
                Ok(session) => sessions.push(session),
                Err(reason) => {
                    tracing::info!(%reason, "encoding with one NVENC session");
                }
            }
        }
        Ok(Self {
            pts,
            output: output.to_path_buf(),
            temp,
            mux,
            sessions,
            fallbacks,
            source_keyframes,
            match_keyframes: encode.keyframes == KeyframePolicy::MatchSource,
        })
    }

    /// FFmpeg's name of the encoder that will be used.
    pub fn encoder(&self) -> &str {
        self.sessions
            .first()
            .map_or("", |session| session.encoder.name())
    }

    /// Number of encoder sessions that will run in parallel (1 or 2).
    pub fn sessions(&self) -> u8 {
        u8::try_from(self.sessions.len()).unwrap_or(u8::MAX)
    }

    /// Encoders of the chain that failed to open before the one used.
    pub fn fallbacks(&self) -> &[EncoderFallback] {
        &self.fallbacks
    }

    /// Renders the video: blurs the boxes of `table` with `blur` and
    /// `params` and writes the output.
    ///
    /// `cancel` is polled once per frame; `progress` is called on the
    /// calling thread with the number of frames written so far.
    ///
    /// # Errors
    ///
    /// [`VideoError::FrameCountMismatch`], [`VideoError::PtsMismatch`]
    /// when the video does not match the project; [`VideoError::Canceled`];
    /// [`VideoError::Blur`], [`VideoError::Encode`] or [`VideoError::Io`]
    /// when a stage fails. The temporary file is removed on every error.
    pub fn run(
        self,
        table: &FrameBoxTable,
        blur: &dyn BlurBackend,
        params: &PixelParams,
        cancel: &(dyn Fn() -> bool + Sync),
        mut progress: impl FnMut(u32),
    ) -> Result<TranscodeReport, VideoError> {
        let started = Instant::now();
        let count = self.pts.len();
        if table.frame_count() != count {
            return Err(VideoError::FrameCountMismatch {
                expected: count,
                found: table.frame_count(),
            });
        }
        let temp = self.temp.clone();
        let output = self.output.clone();
        let encoder = self.encoder().to_owned();
        let sessions = self.sessions();
        let fallbacks = self.fallbacks.clone();
        let mut muxer = Muxer::create(&temp, self.mux.avio_buffer)?;
        let outcome = self
            .transcode(&mut muxer, table, blur, params, cancel, &mut progress)
            .and_then(|stats| muxer.finish().map(|()| stats));
        let stats = match outcome {
            Ok(stats) => stats,
            Err(error) => {
                remove_temp(&temp);
                return Err(error);
            }
        };
        std::fs::rename(&temp, &output).map_err(|source| {
            remove_temp(&temp);
            VideoError::Io {
                context: "renaming the output to",
                path: output.clone(),
                source,
            }
        })?;
        Ok(TranscodeReport {
            output,
            frames: stats.frames,
            blurred_frames: stats.blurred.frames,
            blur_time: stats.blurred.time,
            encoder,
            sessions,
            fallbacks,
            elapsed: started.elapsed(),
        })
    }

    /// Runs the stages of every session into `muxer`.
    fn transcode(
        self,
        muxer: &mut Muxer,
        table: &FrameBoxTable,
        blur: &dyn BlurBackend,
        params: &PixelParams,
        cancel: &(dyn Fn() -> bool + Sync),
        progress: &mut dyn FnMut(u32),
    ) -> Result<Stats, VideoError> {
        let Self {
            pts,
            sessions,
            source_keyframes,
            match_keyframes,
            ..
        } = self;
        let Some(lead) = sessions.first() else {
            return Err(VideoError::Unsupported(String::from("no encoder session")));
        };
        let stream = muxer.add_video(&lead.encoder)?;
        muxer.write_header(Dictionary::new())?;
        let (encoder_tb, stream_tb) = (lead.encoder.time_base(), muxer.time_base(stream));

        let session_count = u32::try_from(sessions.len()).unwrap_or(1);
        let scheduler = Mutex::new(Scheduler::new(pts.len(), &source_keyframes, session_count));
        let keyframes = Keyframes {
            source: &source_keyframes,
            match_source: match_keyframes,
        };
        let control = Control::default();
        let (packet_tx, packet_rx) = bounded(PACKET_QUEUE * sessions.len());
        let pts: &PtsTable = &pts;
        let mut written = 0u32;
        let blurred = std::thread::scope(|scope| {
            let (control, scheduler) = (&control, &scheduler);
            // Each stage owns its channel ends, so a stage that stops
            // (finished or failed) disconnects its neighbours.
            let encoders: Vec<_> = sessions
                .into_iter()
                .map(|session| {
                    let (frame_tx, frame_rx) = bounded(FRAME_QUEUE);
                    let packet_tx = packet_tx.clone();
                    let Session {
                        decoder,
                        first,
                        encoder,
                    } = session;
                    let decode = DecodeStage {
                        decoder,
                        pending: first,
                        pts,
                        keyframes,
                        scheduler,
                    };
                    scope.spawn(move || control.guard(decode.run(&frame_tx, cancel, control)));
                    scope.spawn(move || {
                        let stage = EncodeStage {
                            encoder,
                            pool: FramePool::default(),
                            table,
                            blur,
                            params,
                        };
                        control.guard(stage.run(&frame_rx, &packet_tx, control))
                    })
                })
                .collect();
            drop(packet_tx);
            let mux_result =
                mux_packets(&packet_rx, muxer, pts, stream, encoder_tb, stream_tb, |n| {
                    written = n;
                    progress(n);
                });
            control.guard(mux_result);
            drop(packet_rx);
            let mut blurred = BlurStats::default();
            for encoder in encoders {
                match encoder.join() {
                    Ok(stats) => blurred.add(stats.unwrap_or_default()),
                    Err(panic) => std::panic::resume_unwind(panic),
                }
            }
            blurred
        });
        control.into_result()?;
        if written != pts.len() {
            return Err(VideoError::FrameCountMismatch {
                expected: pts.len(),
                found: written,
            });
        }
        Ok(Stats {
            frames: written,
            blurred,
        })
    }
}

/// Whether `encoder` runs as two sessions with the `sessions` setting:
/// H.264 NVENC unless one session is asked for, and the software test
/// encoders (`mpeg4`, `ffv1`) only when two are asked for, so CI covers
/// segmenting and stitching without a GPU.
fn runs_two_sessions(encoder: &str, sessions: NvencSessions) -> bool {
    match encoder {
        SPLIT_ENCODER => sessions != NvencSessions::One,
        "mpeg4" | "ffv1" => sessions == NvencSessions::Two,
        _ => false,
    }
}

/// Opens the second NVENC session (decoder and encoder). Fails, with the
/// reason, when the encoder does not open or its codec headers differ
/// from the first session's.
fn second_session(
    info: &MediaInfo,
    input: &EncoderInput,
    decode: &DecodeOptions,
    encode: &EncodeOptions,
    lead: &VideoEncoder,
) -> Result<Session, String> {
    let mut same = encode.clone();
    same.encoder = EncoderSetting::Named(lead.name().to_owned());
    let (encoder, _) = VideoEncoder::open(input, &same).map_err(|e| e.to_string())?;
    if encoder.extradata() != lead.extradata() {
        return Err(String::from(
            "the two sessions produce different codec headers",
        ));
    }
    let decoder = Decoder::open(&info.path, decode).map_err(|e| e.to_string())?;
    Ok(Session {
        decoder,
        first: None,
        encoder,
    })
}

/// Counters of a finished transcode.
struct Stats {
    frames: u32,
    blurred: BlurStats,
}

/// Frames an encode stage blurred and the time it spent on them (copy
/// included).
#[derive(Debug, Default, Clone, Copy)]
struct BlurStats {
    frames: u32,
    time: Duration,
}

impl BlurStats {
    fn add(&mut self, other: Self) {
        self.frames += other.frames;
        self.time += other.time;
    }
}

/// Shared state of the stages: the first error, and the flag that tells
/// every stage to stop.
#[derive(Default)]
struct Control {
    abort: AtomicBool,
    error: Mutex<Option<VideoError>>,
}

impl Control {
    /// Records the error of a stage (the first one wins) and stops the
    /// others. Returns the stage's value on success.
    fn guard<T>(&self, result: Result<T, VideoError>) -> Option<T> {
        match result {
            Ok(value) => Some(value),
            Err(error) => {
                self.abort.store(true, Ordering::Relaxed);
                if let Ok(mut slot) = self.error.lock() {
                    slot.get_or_insert(error);
                }
                None
            }
        }
    }

    fn aborted(&self) -> bool {
        self.abort.load(Ordering::Relaxed)
    }

    fn into_result(self) -> Result<(), VideoError> {
        match self.error.into_inner() {
            Ok(Some(error)) => Err(error),
            Ok(None) => Ok(()),
            Err(poisoned) => poisoned.into_inner().map_or(Ok(()), Err),
        }
    }
}

/// Which frames become keyframes.
#[derive(Clone, Copy)]
struct Keyframes<'a> {
    /// Source keyframes, sorted.
    source: &'a [u32],
    /// Whether every source keyframe becomes an IDR frame (otherwise only
    /// segment starts do).
    match_source: bool,
}

impl Keyframes<'_> {
    fn is_key(&self, index: u32, segment: Segment) -> bool {
        index == segment.start || (self.match_source && self.source.binary_search(&index).is_ok())
    }
}

/// Checks a decoded frame against the PTS table.
fn check_frame(frame: &VideoFrame, pts: &PtsTable) -> Result<(), VideoError> {
    match pts.pts(frame.index()) {
        None => Err(VideoError::FrameCountMismatch {
            expected: pts.len(),
            found: frame.index() + 1,
        }),
        Some(expected) if expected != frame.pts() => Err(VideoError::PtsMismatch {
            index: frame.index(),
            expected,
            found: frame.pts(),
        }),
        Some(_) => Ok(()),
    }
}

/// A frame for the encoder, and whether it must become a keyframe.
struct Work {
    frame: VideoFrame,
    keyframe: bool,
}

/// The decode stage of a session: decodes the segments it gets from the
/// scheduler, checks every frame and passes it on.
struct DecodeStage<'a> {
    decoder: Decoder,
    /// A decoded frame not passed on yet (frame 0 of the first session).
    pending: Option<VideoFrame>,
    pts: &'a PtsTable,
    keyframes: Keyframes<'a>,
    scheduler: &'a Mutex<Scheduler>,
}

impl DecodeStage<'_> {
    fn run(
        mut self,
        frames: &Sender<Work>,
        cancel: &(dyn Fn() -> bool + Sync),
        control: &Control,
    ) -> Result<(), VideoError> {
        // Presentation index of the next frame the decoder outputs.
        let mut position = self.pending.as_ref().map_or(0, VideoFrame::index);
        while let Some(segment) = self.next_segment()? {
            if position != segment.start {
                self.pending = None;
                self.decoder.seek(segment.start, self.pts)?;
            }
            for index in segment.start..segment.end {
                if control.aborted() {
                    return Ok(());
                }
                if cancel() {
                    return Err(VideoError::Canceled);
                }
                let frame = match self.pending.take() {
                    Some(frame) => frame,
                    None => self
                        .decoder
                        .next_frame()?
                        .ok_or(VideoError::FrameCountMismatch {
                            expected: self.pts.len(),
                            found: index,
                        })?,
                };
                check_frame(&frame, self.pts)?;
                let keyframe = self.keyframes.is_key(index, segment);
                if frames.send(Work { frame, keyframe }).is_err() {
                    // The encoder stopped; its error is already recorded.
                    return Ok(());
                }
            }
            position = segment.end;
            if segment.end == self.pts.len() && self.decoder.next_frame()?.is_some() {
                return Err(VideoError::FrameCountMismatch {
                    expected: self.pts.len(),
                    found: self.pts.len() + 1,
                });
            }
        }
        Ok(())
    }

    fn next_segment(&self) -> Result<Option<Segment>, VideoError> {
        self.scheduler
            .lock()
            .map(|mut scheduler| scheduler.next_segment())
            .map_err(|_| VideoError::Unsupported(String::from("segment scheduler poisoned")))
    }
}

/// The blur+encode stage of a session.
struct EncodeStage<'a> {
    encoder: VideoEncoder,
    pool: FramePool,
    table: &'a FrameBoxTable,
    blur: &'a dyn BlurBackend,
    params: &'a PixelParams,
}

impl EncodeStage<'_> {
    /// Encodes every frame from `frames`.
    fn run(
        mut self,
        frames: &Receiver<Work>,
        packets: &Sender<EncodedPacket>,
        control: &Control,
    ) -> Result<BlurStats, VideoError> {
        let mut prepared = Vec::new();
        let mut blurred = BlurStats::default();
        for Work {
            mut frame,
            keyframe,
        } in frames
        {
            if control.aborted() {
                return Ok(blurred);
            }
            let boxes = self.table.boxes(frame.index());
            let target = self.encoder.input_layout();
            let started = Instant::now();
            // Frames the encoder cannot take directly are converted into a
            // pool frame; a converted frame is already private.
            let mut owned = false;
            if frame.layout() != target {
                frame = self.pool.copy(&frame, target)?;
                owned = true;
            }
            if !boxes.is_empty() {
                owned |= !frame.is_writable();
                let (width, height) = (frame.width(), frame.height());
                prepare_boxes(boxes, width, height, self.params, &mut prepared);
                let mut view = frame.make_writable(&mut self.pool)?;
                self.blur.blur(&mut view, (0, 0), &prepared, self.params)?;
                blurred.frames += 1;
                blurred.time += started.elapsed();
            }
            self.encoder.send(&mut frame, keyframe)?;
            if owned {
                self.pool.give_back(frame);
            }
            if !self.drain(packets)? {
                return Ok(blurred);
            }
        }
        self.encoder.flush()?;
        self.drain(packets)?;
        Ok(blurred)
    }

    /// Passes every packet the encoder has ready to the muxer. Returns
    /// `false` when the muxer stopped.
    fn drain(&mut self, packets: &Sender<EncodedPacket>) -> Result<bool, VideoError> {
        while let Some(packet) = self.encoder.receive()? {
            if packets.send(packet).is_err() {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

/// Writes the video packets in frame order with their durations from the
/// PTS table. Packets of a segment that is ahead of the one being written
/// wait in a reorder buffer. Calls `written` with the number of packets
/// written after each one.
fn mux_packets(
    packets: &Receiver<EncodedPacket>,
    muxer: &mut Muxer,
    pts: &PtsTable,
    stream: usize,
    encoder_tb: ffmpeg_the_third::Rational,
    stream_tb: ffmpeg_the_third::Rational,
    mut written: impl FnMut(u32),
) -> Result<(), VideoError> {
    let mut waiting = BTreeMap::new();
    let mut next = 0u32;
    for packet in packets {
        let index = packet.pts().and_then(|p| pts.index_of(p)).ok_or_else(|| {
            VideoError::Timestamps(format!(
                "the encoder returned a packet with PTS {:?}, which is no frame of the video",
                packet.pts()
            ))
        })?;
        waiting.insert(index, packet.into_inner());
        while let Some(mut packet) = waiting.remove(&next) {
            packet.set_duration(frame_duration(pts, next));
            // Without B-frames, decoding order is presentation order.
            packet.set_dts(packet.pts());
            packet.set_stream(stream);
            packet.rescale_ts(encoder_tb, stream_tb);
            muxer.write(&mut packet)?;
            next += 1;
            written(next);
        }
    }
    if waiting.is_empty() {
        Ok(())
    } else {
        Err(VideoError::FrameCountMismatch {
            expected: pts.len(),
            found: next,
        })
    }
}

/// Duration of frame `index`: until the next frame, or as long as the
/// previous one for the last frame.
fn frame_duration(pts: &PtsTable, index: u32) -> i64 {
    match (pts.pts(index), pts.pts(index + 1)) {
        (Some(this), Some(next)) => next - this,
        (Some(this), None) => index
            .checked_sub(1)
            .and_then(|previous| pts.pts(previous))
            .map_or(1, |previous| this - previous),
        _ => 0,
    }
}

/// `<stem>.tmp.<ext>` next to `output`, after checking that the output is
/// a supported container and not the input.
fn temp_path(input: &Path, output: &Path) -> Result<PathBuf, VideoError> {
    let extension = output
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .filter(|e| CONTAINERS.contains(&e.as_str()))
        .ok_or_else(|| {
            VideoError::Unsupported(format!(
                "the output {} must end in .mp4, .mov or .mkv",
                output.display()
            ))
        })?;
    let stem = output
        .file_stem()
        .ok_or_else(|| VideoError::Unsupported(format!("invalid output {}", output.display())))?;
    let mut name = stem.to_os_string();
    name.push(".tmp.");
    name.push(&extension);
    let temp = output.with_file_name(name);
    let resolved = |path: &Path| -> Option<PathBuf> {
        let parent = path.parent().filter(|p| !p.as_os_str().is_empty());
        let dir = parent.map_or_else(|| Path::new(".").canonicalize(), Path::canonicalize);
        Some(dir.ok()?.join(path.file_name()?))
    };
    let source = input.canonicalize().map_err(|source| VideoError::Io {
        context: "reading",
        path: input.to_path_buf(),
        source,
    })?;
    for path in [output, temp.as_path()] {
        if resolved(path).is_some_and(|p| same_path(&p, &source)) {
            return Err(VideoError::Unsupported(String::from(
                "the output must not replace the input",
            )));
        }
    }
    Ok(temp)
}

/// Whether two resolved paths name the same file (Windows file systems
/// ignore case).
fn same_path(a: &Path, b: &Path) -> bool {
    a.as_os_str()
        .to_string_lossy()
        .eq_ignore_ascii_case(&b.as_os_str().to_string_lossy())
}

/// Removes a temporary output, logging (not failing) when that fails.
fn remove_temp(temp: &Path) {
    if let Err(error) = std::fs::remove_file(temp)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(path = %temp.display(), %error, "could not remove the temporary output");
    }
}

#[cfg(test)]
mod tests {
    use blurrd_core::Rational;

    use super::{frame_duration, temp_path};
    use crate::PtsTable;

    #[test]
    fn durations_come_from_the_pts_table() {
        let pts = PtsTable::new(Rational::new(1, 1000).unwrap(), vec![0, 16, 33, 50, 70]).unwrap();
        let durations: Vec<i64> = (0..5).map(|i| frame_duration(&pts, i)).collect();
        assert_eq!(durations, [16, 17, 17, 20, 20]);
        assert_eq!(frame_duration(&pts, 9), 0);
        let single = PtsTable::new(Rational::new(1, 1000).unwrap(), vec![5]).unwrap();
        assert_eq!(frame_duration(&single, 0), 1);
    }

    #[test]
    fn temporary_names_and_input_protection() {
        let dir = std::env::temp_dir().join(format!("blurrd-temp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("Clip.mp4");
        std::fs::write(&input, b"x").unwrap();
        assert_eq!(
            temp_path(&input, &dir.join("out.MKV")).unwrap(),
            dir.join("out.tmp.mkv")
        );
        assert!(
            temp_path(&input, &dir.join("clip.mp4")).is_err(),
            "same file"
        );
        assert!(
            temp_path(&input, &dir.join("out.avi")).is_err(),
            "container"
        );
        assert!(temp_path(&input, &dir.join("out")).is_err(), "no extension");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
