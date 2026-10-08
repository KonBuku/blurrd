//! Writing the output container.
//!
//! FFmpeg writes through an AVIO context whose callbacks go to a Rust
//! [`File`] with a large write buffer (1–4 MB instead of FFmpeg's 32 KB),
//! so a 40 Mbit/s output is written in a few hundred large writes per
//! minute instead of tens of thousands of small ones. The container format
//! follows the output file's extension.

use std::ffi::{CString, c_int, c_void};
use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::ptr;

use ffmpeg_the_third::codec::packet::Mut as _;
use ffmpeg_the_third::{Dictionary, codec, ffi};

use crate::{VideoEncoder, VideoError};

/// `AVERROR(EIO)`.
const AVERROR_EIO: c_int = -5;

/// The file behind the AVIO context.
struct Sink {
    file: File,
    /// The first I/O error, reported instead of FFmpeg's generic EIO.
    error: Option<std::io::Error>,
}

/// AVIO write callback: writes the whole buffer to the file.
unsafe extern "C" fn write_packet(opaque: *mut c_void, buf: *const u8, size: c_int) -> c_int {
    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `opaque` is the `Sink` owned by the `Muxer`, which
        // outlives the AVIO context; FFmpeg calls the callbacks from the
        // thread that writes, never concurrently. `buf` holds `size` bytes.
        let (sink, data) = unsafe {
            (
                &mut *opaque.cast::<Sink>(),
                std::slice::from_raw_parts(buf, usize::try_from(size).unwrap_or(0)),
            )
        };
        match sink.file.write_all(data) {
            Ok(()) => size,
            Err(error) => {
                sink.error.get_or_insert(error);
                AVERROR_EIO
            }
        }
    }));
    result.unwrap_or(AVERROR_EIO)
}

/// AVIO seek callback, including FFmpeg's "report the size" request.
unsafe extern "C" fn seek(opaque: *mut c_void, offset: i64, whence: c_int) -> i64 {
    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: as in `write_packet`.
        let sink = unsafe { &mut *opaque.cast::<Sink>() };
        let whence = whence & !(ffi::AVSEEK_FORCE as c_int);
        let outcome = if whence == ffi::AVSEEK_SIZE as c_int {
            sink.file.metadata().map(|m| m.len())
        } else {
            let target = match whence {
                0 => u64::try_from(offset).map(SeekFrom::Start).ok(),
                1 => Some(SeekFrom::Current(offset)),
                2 => Some(SeekFrom::End(offset)),
                _ => None,
            };
            match target {
                Some(target) => sink.file.seek(target),
                None => return i64::from(AVERROR_EIO),
            }
        };
        match outcome {
            Ok(position) => i64::try_from(position).unwrap_or(i64::MAX),
            Err(error) => {
                sink.error.get_or_insert(error);
                i64::from(AVERROR_EIO)
            }
        }
    }));
    result.unwrap_or(i64::from(AVERROR_EIO))
}

/// An output container being written.
pub(crate) struct Muxer {
    ctx: *mut ffi::AVFormatContext,
    sink: *mut Sink,
    path: PathBuf,
}

impl std::fmt::Debug for Muxer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Muxer")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl Muxer {
    /// Creates `path` (replacing an existing file) for the container its
    /// extension names, writing through a buffer of `buffer` bytes.
    pub(crate) fn create(path: &Path, buffer: usize) -> Result<Self, VideoError> {
        let name = CString::new(path.to_string_lossy().as_bytes())
            .map_err(|_| VideoError::Unsupported(format!("invalid path {}", path.display())))?;
        let mut ctx = ptr::null_mut();
        // SAFETY: `ctx` receives a new context on success; the format is
        // guessed from the file name.
        let ret = unsafe {
            ffi::avformat_alloc_output_context2(
                &raw mut ctx,
                ptr::null(),
                ptr::null(),
                name.as_ptr(),
            )
        };
        if ret < 0 || ctx.is_null() {
            return Err(VideoError::Unsupported(format!(
                "no container format for {}",
                path.display()
            )));
        }
        let mut muxer = Self {
            ctx,
            sink: ptr::null_mut(),
            path: path.to_path_buf(),
        };
        let file = File::create(path).map_err(|source| VideoError::Io {
            context: "creating",
            path: path.to_path_buf(),
            source,
        })?;
        muxer.sink = Box::into_raw(Box::new(Sink { file, error: None }));
        let size = c_int::try_from(buffer.clamp(64 * 1024, 16 * 1024 * 1024)).unwrap_or(c_int::MAX);
        // SAFETY: `av_malloc` returns null or a buffer of `size` bytes,
        // which the AVIO context takes over; the callbacks get the sink,
        // which lives until `Drop` frees the context first.
        unsafe {
            let data = ffi::av_malloc(size as usize).cast::<u8>();
            if data.is_null() {
                return Err(out_of_memory());
            }
            let io = ffi::avio_alloc_context(
                data,
                size,
                1,
                muxer.sink.cast(),
                None,
                Some(write_packet),
                Some(seek),
            );
            if io.is_null() {
                ffi::av_free(data.cast());
                return Err(out_of_memory());
            }
            (*ctx).pb = io;
            (*ctx).flags |= ffi::AVFMT_FLAG_CUSTOM_IO as c_int;
        }
        Ok(muxer)
    }

    /// Adds the video stream described by an open encoder. Returns the
    /// stream index.
    pub(crate) fn add_video(&mut self, encoder: &VideoEncoder) -> Result<usize, VideoError> {
        let stream = self.new_stream()?;
        // SAFETY: `stream` was just created in this context; its codec
        // parameters are valid.
        unsafe {
            encoder.fill_parameters((*stream).codecpar)?;
            (*(*stream).codecpar).codec_tag = 0;
            (*stream).time_base = encoder.time_base().into();
            Ok(usize::try_from((*stream).index).unwrap_or(0))
        }
    }

    /// The muxer's short name, for example `mp4`, `mov` or `matroska`.
    pub(crate) fn format_name(&self) -> String {
        // SAFETY: `oformat` of a valid output context points to a static
        // format description whose name is a static C string.
        unsafe { std::ffi::CStr::from_ptr((*(*self.ctx).oformat).name) }
            .to_string_lossy()
            .into_owned()
    }

    /// Whether the container can store a stream of `codec`.
    pub(crate) fn supports(&self, codec: ffi::AVCodecID) -> bool {
        // SAFETY: `oformat` is valid; the query only reads static tables.
        unsafe { ffi::avformat_query_codec((*self.ctx).oformat, codec, 0) == 1 }
    }

    /// Adds a stream that copies the input stream `input` (codec
    /// parameters, including coded side data such as the display matrix,
    /// and time base). Returns the stream index.
    ///
    /// # Safety
    ///
    /// `input` must point to a valid stream of an open input.
    pub(crate) unsafe fn add_copy(
        &mut self,
        input: *const ffi::AVStream,
    ) -> Result<usize, VideoError> {
        let stream = self.new_stream()?;
        // SAFETY: both streams and their parameters are valid.
        unsafe {
            let ret = ffi::avcodec_parameters_copy((*stream).codecpar, (*input).codecpar);
            if ret < 0 {
                return Err(VideoError::encode("copying stream parameters")(
                    ffmpeg_the_third::Error::from(ret),
                ));
            }
            (*(*stream).codecpar).codec_tag = 0;
            (*stream).time_base = (*input).time_base;
            (*stream).disposition = (*input).disposition;
            Ok(usize::try_from((*stream).index).unwrap_or(0))
        }
    }

    /// Adds a stream described by an open encoder context. Returns the
    /// stream index.
    ///
    /// # Safety
    ///
    /// `encoder` must point to a valid, open encoder context.
    pub(crate) unsafe fn add_encoded(
        &mut self,
        encoder: *const ffi::AVCodecContext,
    ) -> Result<usize, VideoError> {
        let stream = self.new_stream()?;
        // SAFETY: the stream and the encoder context are valid.
        unsafe {
            let ret = ffi::avcodec_parameters_from_context((*stream).codecpar, encoder);
            if ret < 0 {
                return Err(VideoError::encode("copying encoder parameters")(
                    ffmpeg_the_third::Error::from(ret),
                ));
            }
            (*(*stream).codecpar).codec_tag = 0;
            (*stream).time_base = (*encoder).time_base;
            Ok(usize::try_from((*stream).index).unwrap_or(0))
        }
    }

    /// Copies coded side data of `kind` (for example the display matrix,
    /// which holds the rotation) from input stream parameters to a stream.
    ///
    /// # Safety
    ///
    /// `input` must point to valid codec parameters; `stream` must be an
    /// index returned by this muxer before the header was written.
    pub(crate) unsafe fn copy_side_data(
        &mut self,
        stream: usize,
        input: *const ffi::AVCodecParameters,
        kind: ffi::AVPacketSideDataType,
    ) -> Result<(), VideoError> {
        // SAFETY: both parameter sets are valid; the side data entry and its
        // buffer are owned by `input` and copied by `av_packet_side_data_new`.
        unsafe {
            let found = ffi::av_packet_side_data_get(
                (*input).coded_side_data,
                (*input).nb_coded_side_data,
                kind,
            );
            if found.is_null() {
                return Ok(());
            }
            let par = (**(*self.ctx).streams.add(stream)).codecpar;
            let entry = ffi::av_packet_side_data_new(
                &raw mut (*par).coded_side_data,
                &raw mut (*par).nb_coded_side_data,
                kind,
                (*found).size,
                0,
            );
            if entry.is_null() {
                return Err(out_of_memory());
            }
            ptr::copy_nonoverlapping((*found).data, (*entry).data, (*found).size);
        }
        Ok(())
    }

    /// Sets a metadata tag of the container (`stream: None`) or a stream.
    pub(crate) fn set_tag(&mut self, stream: Option<usize>, key: &str, value: &str) {
        let (Ok(key), Ok(value)) = (CString::new(key), CString::new(value)) else {
            return;
        };
        // SAFETY: the dictionary belongs to the valid context or one of its
        // streams; `av_dict_set` copies key and value.
        unsafe {
            let dict = match stream {
                None => &raw mut (*self.ctx).metadata,
                Some(index) => &raw mut (**(*self.ctx).streams.add(index)).metadata,
            };
            ffi::av_dict_set(dict, key.as_ptr(), value.as_ptr(), 0);
        }
    }

    /// A new stream in the context (before the header).
    fn new_stream(&mut self) -> Result<*mut ffi::AVStream, VideoError> {
        // SAFETY: the context is valid and its header is not written yet.
        let stream = unsafe { ffi::avformat_new_stream(self.ctx, ptr::null()) };
        if stream.is_null() {
            return Err(out_of_memory());
        }
        Ok(stream)
    }

    /// Writes the container header with muxer options.
    pub(crate) fn write_header(&mut self, mut options: Dictionary) -> Result<(), VideoError> {
        // SAFETY: the context is valid with all streams added.
        let ret = unsafe { ffi::avformat_write_header(self.ctx, options.as_mut_ptr()) };
        if ret < 0 {
            return Err(self.failure("writing the container header", ret));
        }
        for (key, value) in options.iter() {
            tracing::debug!(key, value, "muxer ignored an option");
        }
        Ok(())
    }

    /// Time base of a stream after the header was written (the muxer may
    /// change it).
    pub(crate) fn time_base(&self, stream: usize) -> ffmpeg_the_third::Rational {
        // SAFETY: `stream` is an index returned by `add_video`; the stream
        // array of the valid context is not modified after the header.
        unsafe { (**(*self.ctx).streams.add(stream)).time_base.into() }
    }

    /// Writes a packet (its stream index and timestamps already set in the
    /// stream's time base); the muxer interleaves streams by timestamp.
    pub(crate) fn write(&mut self, packet: &mut codec::packet::Packet) -> Result<(), VideoError> {
        // SAFETY: the context is valid and its header written; the muxer
        // takes the packet's data reference and leaves it blank.
        let ret = unsafe { ffi::av_interleaved_write_frame(self.ctx, packet.as_mut_ptr()) };
        if ret < 0 {
            return Err(self.failure("writing a packet", ret));
        }
        Ok(())
    }

    /// Writes the trailer and flushes the buffer to the file.
    pub(crate) fn finish(self) -> Result<(), VideoError> {
        // SAFETY: the context is valid and its header written; `pb` is our
        // AVIO context.
        let ret = unsafe {
            let ret = ffi::av_write_trailer(self.ctx);
            ffi::avio_flush((*self.ctx).pb);
            ret
        };
        if ret < 0 {
            return Err(self.failure("finishing the container", ret));
        }
        // SAFETY: the sink is valid until `Drop`.
        if let Some(source) = unsafe { (*self.sink).error.take() } {
            return Err(VideoError::Io {
                context: "writing",
                path: self.path.clone(),
                source,
            });
        }
        Ok(())
    }

    /// The error for a failed FFmpeg call: the file's I/O error if there was
    /// one, FFmpeg's otherwise.
    fn failure(&self, context: &'static str, ret: c_int) -> VideoError {
        // SAFETY: the sink is valid until `Drop`.
        match unsafe { (*self.sink).error.take() } {
            Some(source) => VideoError::Io {
                context: "writing",
                path: self.path.clone(),
                source,
            },
            None => VideoError::encode(context)(ffmpeg_the_third::Error::from(ret)),
        }
    }
}

impl Drop for Muxer {
    fn drop(&mut self) {
        // SAFETY: the context and AVIO context were allocated in `create`
        // and are freed exactly once here, the AVIO context (and its
        // current buffer) separately because it is custom I/O; the sink is
        // freed last because the callbacks use it.
        unsafe {
            let mut io = (*self.ctx).pb;
            (*self.ctx).pb = ptr::null_mut();
            ffi::avformat_free_context(self.ctx);
            if !io.is_null() {
                ffi::av_freep((&raw mut (*io).buffer).cast());
                ffi::avio_context_free(&raw mut io);
            }
            if !self.sink.is_null() {
                drop(Box::from_raw(self.sink));
            }
        }
    }
}

fn out_of_memory() -> VideoError {
    VideoError::encode("allocating the output")(ffmpeg_the_third::Error::Other { errno: 12 })
}
