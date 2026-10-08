//! The streams besides the video: audio, subtitles and data, and the
//! container metadata.
//!
//! The input is opened a second time, without the video stream, and its
//! packets are copied unchanged into the output (or, for audio set to AAC,
//! decoded, resampled and encoded with FFmpeg's native AAC encoder). The
//! muxer interleaves them with the video by timestamp.
//!
//! Metadata: by default only technical tags survive (stream languages, the
//! video's rotation, timecodes); recorder titles, comments, creation times
//! and account tags are dropped.
//!
//! Copied packets keep their timestamps, including packets the source
//! hides with an edit list (AAC encoder priming): the output's edit list
//! hides them the same way, and the decoder still gets the data it needs.
//! The one exception are the empty gap samples of MP4 timed text, which
//! the MP4 muxer writes again by itself.

use std::ffi::{CStr, c_int};
use std::path::Path;
use std::ptr;

use blurrd_core::{AudioMode, MetadataMode};
use crossbeam_channel::Sender;
use ffmpeg_the_third::codec::packet::Mut as _;
use ffmpeg_the_third::{codec, ffi, format, media};

use crate::mux::Muxer;
use crate::{VideoError, input};

/// Sample rates the native AAC encoder supports.
const AAC_RATES: [i32; 13] = [
    96_000, 88_200, 64_000, 48_000, 44_100, 32_000, 24_000, 22_050, 16_000, 12_000, 11_025, 8_000,
    7_350,
];
/// Channels the native AAC encoder supports at most (7.1).
const AAC_MAX_CHANNELS: i32 = 8;
/// `AVERROR(EAGAIN)`.
const EAGAIN: c_int = -11;

/// A stream of the input that the output does not contain.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SkippedStream {
    /// Stream index in the input.
    pub index: u32,
    /// FFmpeg codec name, for example `subrip`.
    pub codec: String,
    /// Why it was left out.
    pub reason: String,
}

/// A packet for the muxer: stream index and timestamps in the output
/// stream's time base, and its time in seconds for interleaving.
pub(crate) struct SidePacket {
    pub(crate) packet: codec::packet::Packet,
    pub(crate) seconds: f64,
}

/// How one input stream gets into the output.
enum Route {
    /// Packets are copied unchanged.
    Copy {
        output: usize,
        input_tb: ffi::AVRational,
        output_tb: ffi::AVRational,
        /// MP4 timed text, whose empty gap samples are left out.
        timed_text: bool,
    },
    /// Audio is re-encoded as AAC.
    Aac(Box<AacStream>),
}

impl Route {
    /// The output stream index.
    fn output(&self) -> usize {
        match self {
            Self::Copy { output, .. } => *output,
            Self::Aac(aac) => aac.output,
        }
    }
}

/// The non-video streams of the input and where they go.
pub(crate) struct SideStreams {
    input: format::context::Input,
    /// Indexed by input stream.
    routes: Vec<Option<Route>>,
    skipped: Vec<SkippedStream>,
    /// Upper bound of the packets this module writes, if every stream's
    /// packet count is known or can be estimated.
    packets: Option<u64>,
}

impl std::fmt::Debug for SideStreams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SideStreams")
            .field(
                "routes",
                &self.routes.iter().filter(|r| r.is_some()).count(),
            )
            .field("skipped", &self.skipped)
            .finish_non_exhaustive()
    }
}

/// Settings for the non-video streams.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SideOptions {
    pub(crate) audio: AudioMode,
    pub(crate) subtitles: bool,
    pub(crate) metadata: MetadataMode,
}

impl SideStreams {
    /// Opens `path` again, adds every stream that goes into the output to
    /// `muxer` and copies the metadata the options keep. `video_output` is
    /// the output index of the video stream.
    ///
    /// # Errors
    ///
    /// [`VideoError::Unsupported`] when an audio stream cannot be copied
    /// into the container; the errors of opening the input or an AAC
    /// encoder.
    pub(crate) fn open(
        path: &Path,
        video_output: usize,
        options: SideOptions,
        muxer: &mut Muxer,
    ) -> Result<Self, VideoError> {
        let mut input = input::open(path)?;
        let video = input::video_stream(&input)?;
        copy_container_metadata(&input, video, video_output, options.metadata, muxer)?;
        let duration = container_seconds(&input);
        let mut routes = Vec::new();
        let mut skipped = Vec::new();
        let mut packets = Some(0u64);
        for mut stream in input.streams_mut() {
            let index = stream.index();
            let route = if index == video {
                None
            } else {
                // SAFETY: the stream belongs to the open input.
                let raw = unsafe { stream.as_mut_ptr() };
                let codec = codec_name(&stream);
                let decision = route_for(raw, &stream, &codec, options, muxer, duration)?;
                match decision {
                    Decision::Route(route, estimate) => {
                        packets = packets.zip(estimate).map(|(a, b)| a + b);
                        copy_stream_metadata(&stream, route.output(), options.metadata, muxer);
                        Some(route)
                    }
                    Decision::Skip(reason) => {
                        tracing::warn!(stream = index, %codec, %reason, "stream not copied");
                        skipped.push(SkippedStream {
                            index: u32::try_from(index).unwrap_or(u32::MAX),
                            codec,
                            reason,
                        });
                        None
                    }
                }
            };
            if route.is_none() {
                // SAFETY: setting the public `discard` field before reading
                // packets, so the demuxer skips the stream.
                unsafe { (*stream.as_mut_ptr()).discard = ffi::AVDiscard::ALL };
            }
            routes.push(route);
        }
        Ok(Self {
            input,
            routes,
            skipped,
            packets,
        })
    }

    /// Streams of the input the output does not contain.
    pub(crate) fn skipped(&self) -> &[SkippedStream] {
        &self.skipped
    }

    /// Number of output streams these streams add.
    pub(crate) fn track_count(&self) -> usize {
        self.routes.iter().flatten().count()
    }

    /// Upper bound of the packets these streams write, if known.
    pub(crate) fn packet_bound(&self) -> Option<u64> {
        self.packets
    }

    /// Records the output time bases the muxer chose when it wrote the
    /// header.
    pub(crate) fn header_written(&mut self, muxer: &Muxer) {
        for route in self.routes.iter_mut().flatten() {
            match route {
                Route::Copy {
                    output, output_tb, ..
                } => *output_tb = muxer.time_base(*output).into(),
                Route::Aac(aac) => aac.output_tb = muxer.time_base(aac.output).into(),
            }
        }
    }

    /// Reads every packet of the routed streams and passes it to the
    /// muxer, until the end of the input, `stop` returns `true` or the
    /// muxer goes away.
    pub(crate) fn run(
        mut self,
        packets: &Sender<SidePacket>,
        stop: impl Fn() -> bool,
    ) -> Result<(), VideoError> {
        let mut packet = codec::packet::Packet::empty();
        let mut send = |p: SidePacket| packets.send(p).is_ok();
        loop {
            if stop() {
                return Ok(());
            }
            match packet.read(&mut self.input) {
                Ok(()) => {}
                Err(ffmpeg_the_third::Error::Eof) => break,
                Err(error) => return Err(VideoError::ffmpeg("reading packets")(error)),
            }
            let Some(route) = self
                .routes
                .get_mut(packet.stream())
                .and_then(Option::as_mut)
            else {
                continue;
            };
            let delivered = match route {
                Route::Copy {
                    output,
                    input_tb,
                    output_tb,
                    timed_text,
                } => {
                    if *timed_text && packet.data() == Some(&[0, 0][..]) {
                        continue;
                    }
                    let mut out = std::mem::replace(&mut packet, codec::packet::Packet::empty());
                    out.set_stream(*output);
                    out.set_position(-1);
                    out.rescale_ts(*input_tb, *output_tb);
                    // The timestamps say what is hidden; the output's edit
                    // list hides it again.
                    // SAFETY: clearing a flag of the packet owned here.
                    unsafe { (*out.as_mut_ptr()).flags &= !ffi::AV_PKT_FLAG_DISCARD };
                    let seconds = seconds(&out, *output_tb);
                    send(SidePacket {
                        packet: out,
                        seconds,
                    })
                }
                Route::Aac(aac) => aac.push(Some(&packet), &mut send)?,
            };
            if !delivered {
                return Ok(());
            }
        }
        for route in self.routes.iter_mut().flatten() {
            if let Route::Aac(aac) = route
                && !aac.push(None, &mut send)?
            {
                return Ok(());
            }
        }
        Ok(())
    }
}

/// What to do with one stream.
enum Decision {
    /// Put it into the output; the estimate bounds its packet count.
    Route(Route, Option<u64>),
    /// Leave it out, with the reason.
    Skip(String),
}

/// Decides and sets up the route of a non-video stream.
fn route_for(
    raw: *mut ffi::AVStream,
    stream: &format::stream::StreamMut<'_>,
    codec: &str,
    options: SideOptions,
    muxer: &mut Muxer,
    container_seconds: Option<f64>,
) -> Result<Decision, VideoError> {
    let parameters = stream.parameters();
    let medium = parameters.medium();
    // SAFETY: the parameters of a stream of the open input are valid.
    let codec_id = unsafe { (*parameters.as_ptr()).codec_id };
    let seconds = stream_seconds(stream).or(container_seconds);
    let copy = |muxer: &mut Muxer| -> Result<Route, VideoError> {
        // SAFETY: `raw` is a valid stream of the open input.
        let output = unsafe { muxer.add_copy(raw)? };
        Ok(Route::Copy {
            output,
            input_tb: stream.time_base().into(),
            output_tb: stream.time_base().into(),
            timed_text: codec_id == ffi::AVCodecID::MOV_TEXT,
        })
    };
    let frames = u64::try_from(stream.frames()).ok().filter(|&n| n > 0);
    Ok(match medium {
        media::Type::Audio => match options.audio {
            AudioMode::Remove => Decision::Skip(String::from("audio removed")),
            AudioMode::Aac(kbps) => {
                // SAFETY: `raw` is a valid stream of the open input.
                let aac = unsafe { AacStream::open(raw, kbps, muxer)? };
                let estimate = seconds.map(|s| estimate_packets(s, aac.sample_rate, 1024));
                Decision::Route(Route::Aac(Box::new(aac)), estimate)
            }
            _ => {
                if !muxer.supports(codec_id) {
                    return Err(VideoError::Unsupported(format!(
                        "{codec} audio cannot be copied into {}; choose AAC audio",
                        muxer.format_name()
                    )));
                }
                // SAFETY: as above.
                let (rate, frame_size) = unsafe {
                    let par = (*raw).codecpar;
                    ((*par).sample_rate, (*par).frame_size)
                };
                let estimate = frames.or_else(|| {
                    seconds.map(|s| {
                        estimate_packets(s, rate, if frame_size > 0 { frame_size } else { 1024 })
                    })
                });
                Decision::Route(copy(muxer)?, estimate)
            }
        },
        media::Type::Subtitle if !options.subtitles => {
            Decision::Skip(String::from("subtitles are not copied"))
        }
        media::Type::Subtitle | media::Type::Data if muxer.supports(codec_id) => {
            Decision::Route(copy(muxer)?, frames)
        }
        media::Type::Subtitle | media::Type::Data => {
            Decision::Skip(format!("{} cannot store {codec}", muxer.format_name()))
        }
        _ => Decision::Skip(String::from(
            "attachments and unknown streams are not copied",
        )),
    })
}

/// A generous upper bound of the packets of `seconds` of audio with
/// `frame_size` samples per packet: 10 % and 64 packets of margin.
fn estimate_packets(seconds: f64, rate: i32, frame_size: i32) -> u64 {
    let frames = seconds.max(0.0) * f64::from(rate.max(1)) / f64::from(frame_size.max(1));
    (frames * 1.1) as u64 + 64
}

/// Duration of a stream in seconds, if the file states it.
fn stream_seconds(stream: &format::stream::StreamMut<'_>) -> Option<f64> {
    let tb = stream.time_base();
    (stream.duration() > 0 && tb.denominator() > 0)
        .then(|| stream.duration() as f64 * f64::from(tb.numerator()) / f64::from(tb.denominator()))
}

/// Duration of the file in seconds, if known.
fn container_seconds(input: &format::context::Input) -> Option<f64> {
    (input.duration() > 0).then(|| input.duration() as f64 / f64::from(ffi::AV_TIME_BASE))
}

/// FFmpeg's codec name of a stream.
fn codec_name(stream: &format::stream::StreamMut<'_>) -> String {
    let parameters = stream.parameters();
    // SAFETY: the parameters are valid; `avcodec_get_name` returns a static
    // string for every codec id.
    unsafe { CStr::from_ptr(ffi::avcodec_get_name((*parameters.as_ptr()).codec_id)) }
        .to_string_lossy()
        .into_owned()
}

/// The packet's decoding time (or presentation time) in seconds.
fn seconds(packet: &codec::packet::Packet, tb: ffi::AVRational) -> f64 {
    let ts = packet.dts().or(packet.pts()).unwrap_or(0);
    ts as f64 * f64::from(tb.num) / f64::from(tb.den.max(1))
}

/// Tags kept per stream with [`MetadataMode::Technical`].
const TECHNICAL_STREAM_TAGS: [&str; 2] = ["language", "timecode"];

/// Copies the stream's tags the mode keeps.
fn copy_stream_metadata(
    stream: &format::stream::StreamMut<'_>,
    output: usize,
    mode: MetadataMode,
    muxer: &mut Muxer,
) {
    for (key, value) in stream.metadata().iter() {
        if mode == MetadataMode::All || TECHNICAL_STREAM_TAGS.contains(&key) {
            muxer.set_tag(Some(output), key, value);
        }
    }
}

/// Copies container tags and the video stream's tags and rotation the
/// mode keeps.
fn copy_container_metadata(
    input: &format::context::Input,
    video: usize,
    video_output: usize,
    mode: MetadataMode,
    muxer: &mut Muxer,
) -> Result<(), VideoError> {
    for (key, value) in input.metadata().iter() {
        if mode == MetadataMode::All || key == "timecode" {
            muxer.set_tag(None, key, value);
        }
    }
    if let Some(stream) = input.stream(video) {
        for (key, value) in stream.metadata().iter() {
            if mode == MetadataMode::All || TECHNICAL_STREAM_TAGS.contains(&key) {
                muxer.set_tag(Some(video_output), key, value);
            }
        }
        // SAFETY: the parameters of a stream of the open input are valid
        // and `video_output` was added to the muxer before.
        unsafe {
            muxer.copy_side_data(
                video_output,
                stream.parameters().as_ptr(),
                ffi::AVPacketSideDataType::DISPLAYMATRIX,
            )?;
        }
    }
    Ok(())
}

/// An audio stream re-encoded as AAC: decoder, resampler to the encoder's
/// planar float format, a FIFO that cuts 1024-sample frames, and the
/// native AAC encoder.
struct AacStream {
    decoder: *mut ffi::AVCodecContext,
    encoder: *mut ffi::AVCodecContext,
    resampler: *mut ffi::SwrContext,
    fifo: *mut ffi::AVAudioFifo,
    input_tb: ffi::AVRational,
    output: usize,
    output_tb: ffi::AVRational,
    sample_rate: i32,
    /// PTS (in samples) of the next frame sent to the encoder.
    next_pts: Option<i64>,
}

// SAFETY: the FFmpeg contexts are owned exclusively by this value and
// only used by the thread that owns it.
unsafe impl Send for AacStream {}

impl AacStream {
    /// Opens decoder, resampler and encoder for the input stream `stream`
    /// and adds the output stream to `muxer`.
    ///
    /// # Safety
    ///
    /// `stream` must point to a valid audio stream of an open input.
    unsafe fn open(
        stream: *mut ffi::AVStream,
        kbps: u16,
        muxer: &mut Muxer,
    ) -> Result<Self, VideoError> {
        let mut aac = Self {
            decoder: ptr::null_mut(),
            encoder: ptr::null_mut(),
            resampler: ptr::null_mut(),
            fifo: ptr::null_mut(),
            // SAFETY: valid stream by the caller's contract.
            input_tb: unsafe { (*stream).time_base },
            output: 0,
            output_tb: ffi::AVRational { num: 1, den: 1 },
            sample_rate: 48_000,
            next_pts: None,
        };
        let fail = |context| {
            move |ret: c_int| VideoError::encode(context)(ffmpeg_the_third::Error::from(ret))
        };
        // SAFETY: every pointer is checked before use; contexts are owned by
        // `aac`, whose `Drop` frees them on every early return.
        unsafe {
            let par = (*stream).codecpar;
            let decoder = ffi::avcodec_find_decoder((*par).codec_id);
            if decoder.is_null() {
                return Err(VideoError::Unsupported(String::from(
                    "no decoder for the audio stream",
                )));
            }
            aac.decoder = ffi::avcodec_alloc_context3(decoder);
            if aac.decoder.is_null() {
                return Err(out_of_memory());
            }
            check(ffi::avcodec_parameters_to_context(aac.decoder, par))
                .map_err(fail("reading audio parameters"))?;
            (*aac.decoder).pkt_timebase = aac.input_tb;
            check(ffi::avcodec_open2(aac.decoder, decoder, ptr::null_mut()))
                .map_err(fail("opening the audio decoder"))?;

            let encoder = ffi::avcodec_find_encoder_by_name(c"aac".as_ptr());
            if encoder.is_null() {
                return Err(VideoError::Unsupported(String::from(
                    "this FFmpeg build has no AAC encoder",
                )));
            }
            aac.encoder = ffi::avcodec_alloc_context3(encoder);
            if aac.encoder.is_null() {
                return Err(out_of_memory());
            }
            let source_rate = (*aac.decoder).sample_rate;
            aac.sample_rate = if AAC_RATES.contains(&source_rate) {
                source_rate
            } else {
                48_000
            };
            let enc = aac.encoder;
            (*enc).sample_fmt = ffi::AVSampleFormat::FLTP;
            (*enc).sample_rate = aac.sample_rate;
            (*enc).bit_rate = i64::from(kbps) * 1000;
            (*enc).time_base = ffi::AVRational {
                num: 1,
                den: aac.sample_rate,
            };
            (*enc).flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER.cast_signed();
            let channels = (*aac.decoder).ch_layout.nb_channels;
            if channels > 0 && channels <= AAC_MAX_CHANNELS {
                check(ffi::av_channel_layout_copy(
                    &raw mut (*enc).ch_layout,
                    &raw const (*aac.decoder).ch_layout,
                ))
                .map_err(fail("copying the channel layout"))?;
            } else {
                ffi::av_channel_layout_default(&raw mut (*enc).ch_layout, 2);
            }
            check(ffi::avcodec_open2(enc, encoder, ptr::null_mut()))
                .map_err(fail("opening the AAC encoder"))?;

            aac.resampler = ffi::swr_alloc();
            if aac.resampler.is_null() {
                return Err(out_of_memory());
            }
            aac.fifo = ffi::av_audio_fifo_alloc(
                ffi::AVSampleFormat::FLTP,
                (*enc).ch_layout.nb_channels,
                (*enc).frame_size.max(1024) * 4,
            );
            if aac.fifo.is_null() {
                return Err(out_of_memory());
            }
            aac.output = muxer.add_encoded(enc)?;
            aac.output_tb = (*enc).time_base;
        }
        Ok(aac)
    }

    /// Decodes a packet (or flushes everything with `None`) and passes the
    /// encoded packets to `send`. Returns `false` when `send` failed.
    fn push(
        &mut self,
        packet: Option<&codec::packet::Packet>,
        send: &mut impl FnMut(SidePacket) -> bool,
    ) -> Result<bool, VideoError> {
        // SAFETY: the decoder is open; a null packet starts draining.
        let ret = unsafe {
            ffi::avcodec_send_packet(
                self.decoder,
                packet.map_or(ptr::null(), codec::packet::Ref::as_ptr),
            )
        };
        if ret < 0 && ret != ffi::AVERROR_EOF {
            return Err(VideoError::ffmpeg("decoding audio")(
                ffmpeg_the_third::Error::from(ret),
            ));
        }
        let mut decoded = ffmpeg_the_third::frame::Audio::empty();
        loop {
            // SAFETY: the decoder is open; `decoded` is a valid frame.
            let ret = unsafe { ffi::avcodec_receive_frame(self.decoder, decoded.as_mut_ptr()) };
            if ret == EAGAIN || ret == ffi::AVERROR_EOF {
                break;
            }
            if ret < 0 {
                return Err(VideoError::ffmpeg("decoding audio")(
                    ffmpeg_the_third::Error::from(ret),
                ));
            }
            if self.next_pts.is_none() {
                // SAFETY: reading fields of the decoded frame.
                let first = unsafe { (*decoded.as_ptr()).best_effort_timestamp };
                let first = if first == ffi::AV_NOPTS_VALUE {
                    0
                } else {
                    first
                };
                // SAFETY: plain arithmetic on rationals.
                self.next_pts = Some(unsafe {
                    ffi::av_rescale_q(
                        first,
                        self.input_tb,
                        ffi::AVRational {
                            num: 1,
                            den: self.sample_rate,
                        },
                    )
                });
            }
            self.resample(Some(&decoded))?;
            if !self.encode_ready(false, send)? {
                return Ok(false);
            }
        }
        if packet.is_none() {
            self.resample(None)?;
            if !self.encode_ready(true, send)? {
                return Ok(false);
            }
            // SAFETY: a null frame starts draining the open encoder.
            unsafe { ffi::avcodec_send_frame(self.encoder, ptr::null()) };
            return self.receive(send);
        }
        Ok(true)
    }

    /// Converts a decoded frame (or the resampler's remaining samples) into
    /// the FIFO.
    fn resample(
        &mut self,
        frame: Option<&ffmpeg_the_third::frame::Audio>,
    ) -> Result<(), VideoError> {
        let mut converted = ffmpeg_the_third::frame::Audio::empty();
        // SAFETY: the encoder, resampler and FIFO are valid; the output
        // frame gets the encoder's format and FFmpeg allocates its buffer;
        // the resampler configures itself from the first frame.
        unsafe {
            let out = converted.as_mut_ptr();
            (*out).format = ffi::AVSampleFormat::FLTP.0;
            (*out).sample_rate = self.sample_rate;
            check(ffi::av_channel_layout_copy(
                &raw mut (*out).ch_layout,
                &raw const (*self.encoder).ch_layout,
            ))
            .map_err(|ret| {
                VideoError::encode("resampling audio")(ffmpeg_the_third::Error::from(ret))
            })?;
            if frame.is_none() && ffi::swr_is_initialized(self.resampler) == 0 {
                return Ok(());
            }
            let input = frame.map_or(ptr::null(), |f| f.as_ptr());
            check(ffi::swr_convert_frame(self.resampler, out, input)).map_err(|ret| {
                VideoError::encode("resampling audio")(ffmpeg_the_third::Error::from(ret))
            })?;
            if (*out).nb_samples > 0 {
                let written = ffi::av_audio_fifo_write(
                    self.fifo,
                    (*out)
                        .extended_data
                        .cast::<*mut std::ffi::c_void>()
                        .cast_const(),
                    (*out).nb_samples,
                );
                if written < (*out).nb_samples {
                    return Err(out_of_memory());
                }
            }
        }
        Ok(())
    }

    /// Encodes every full frame in the FIFO (and the rest with `last`).
    fn encode_ready(
        &mut self,
        last: bool,
        send: &mut impl FnMut(SidePacket) -> bool,
    ) -> Result<bool, VideoError> {
        // SAFETY: the encoder is open.
        let frame_size = unsafe { (*self.encoder).frame_size.max(1) };
        loop {
            // SAFETY: the FIFO is valid.
            let available = unsafe { ffi::av_audio_fifo_size(self.fifo) };
            let take = if available >= frame_size {
                frame_size
            } else if last && available > 0 {
                available
            } else {
                return Ok(true);
            };
            let mut frame = ffmpeg_the_third::frame::Audio::empty();
            let pts = self.next_pts.unwrap_or(0);
            // SAFETY: the frame gets the encoder's format and a buffer for
            // `take` samples, filled from the FIFO, then sent.
            unsafe {
                let f = frame.as_mut_ptr();
                (*f).format = ffi::AVSampleFormat::FLTP.0;
                (*f).sample_rate = self.sample_rate;
                (*f).nb_samples = take;
                check(ffi::av_channel_layout_copy(
                    &raw mut (*f).ch_layout,
                    &raw const (*self.encoder).ch_layout,
                ))
                .and_then(|()| check(ffi::av_frame_get_buffer(f, 0)))
                .map_err(|ret| {
                    VideoError::encode("encoding audio")(ffmpeg_the_third::Error::from(ret))
                })?;
                ffi::av_audio_fifo_read(
                    self.fifo,
                    (*f).extended_data
                        .cast::<*mut std::ffi::c_void>()
                        .cast_const(),
                    take,
                );
                (*f).pts = pts;
                check(ffi::avcodec_send_frame(self.encoder, f)).map_err(|ret| {
                    VideoError::encode("encoding audio")(ffmpeg_the_third::Error::from(ret))
                })?;
            }
            self.next_pts = Some(pts + i64::from(take));
            if !self.receive(send)? {
                return Ok(false);
            }
        }
    }

    /// Passes every packet the encoder has ready to `send`.
    fn receive(&mut self, send: &mut impl FnMut(SidePacket) -> bool) -> Result<bool, VideoError> {
        loop {
            let mut packet = codec::packet::Packet::empty();
            // SAFETY: the encoder is open; the packet is valid.
            let ret = unsafe { ffi::avcodec_receive_packet(self.encoder, packet.as_mut_ptr()) };
            if ret == EAGAIN || ret == ffi::AVERROR_EOF {
                return Ok(true);
            }
            if ret < 0 {
                return Err(VideoError::encode("encoding audio")(
                    ffmpeg_the_third::Error::from(ret),
                ));
            }
            packet.set_stream(self.output);
            // SAFETY: reading the encoder's time base.
            let encoder_tb = unsafe { (*self.encoder).time_base };
            packet.rescale_ts(encoder_tb, self.output_tb);
            let seconds = seconds(&packet, self.output_tb);
            if !send(SidePacket { packet, seconds }) {
                return Ok(false);
            }
        }
    }
}

impl Drop for AacStream {
    fn drop(&mut self) {
        // SAFETY: each pointer is null or owned by `self`; the free
        // functions accept null.
        unsafe {
            ffi::avcodec_free_context(&raw mut self.decoder);
            ffi::avcodec_free_context(&raw mut self.encoder);
            ffi::swr_free(&raw mut self.resampler);
            if !self.fifo.is_null() {
                ffi::av_audio_fifo_free(self.fifo);
            }
        }
    }
}

/// `Ok` for non-negative FFmpeg return values.
fn check(ret: c_int) -> Result<(), c_int> {
    if ret < 0 { Err(ret) } else { Ok(()) }
}

fn out_of_memory() -> VideoError {
    VideoError::encode("allocating audio buffers")(ffmpeg_the_third::Error::Other { errno: 12 })
}

#[cfg(test)]
mod tests {
    use super::estimate_packets;

    #[test]
    fn packet_estimates_have_a_margin() {
        // 60 s of 48 kHz AAC: 2813 packets; the estimate stays above.
        let estimate = estimate_packets(60.0, 48_000, 1024);
        assert!(estimate > 2813 && estimate < 3300, "{estimate}");
        assert_eq!(estimate_packets(0.0, 48_000, 1024), 64);
    }
}
