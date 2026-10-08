//! Random access to frames by presentation index, for the editor.
//!
//! A [`FrameSeeker`] owns a warm decoder on a worker thread. Requests are
//! answered from a cache of decoded frames around the last request; a miss
//! inside the next few hundred frames decodes forward, anything else seeks
//! to the keyframe before the frame. While no request is waiting, the
//! worker keeps decoding ahead (the rest of the current GOP and the next
//! one), so stepping and playback-like access hit the cache. Every decoded
//! frame is checked against the project's [`PtsTable`].

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::JoinHandle;

use crate::{DecodeOptions, Decoder, PixelLayout, PtsTable, VideoError, VideoFrame};

/// Frames ahead of the decoder position that are decoded forward instead
/// of seeking. Decoding is ~1 ms per 1080p frame, a seek plus a GOP refill
/// costs ~30–60 ms.
const FORWARD_LIMIT: u32 = 60;
/// Initial guess of the keyframe interval, replaced by the observed one.
const DEFAULT_GOP: u32 = 60;

/// Seeker settings.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SeekerOptions {
    /// Decoder settings.
    pub decode: DecodeOptions,
    /// Upper bound of the frame cache in bytes.
    pub cache_bytes: usize,
    /// Whether to decode ahead while idle.
    pub prefetch: bool,
}

impl Default for SeekerOptions {
    fn default() -> Self {
        Self {
            decode: DecodeOptions::default(),
            cache_bytes: 512 << 20,
            prefetch: true,
        }
    }
}

type Reply = Result<VideoFrame, VideoError>;

/// Random access to the frames of one video by presentation index.
#[derive(Debug)]
pub struct FrameSeeker {
    requests: Option<Sender<(u32, Sender<Reply>)>>,
    worker: Option<JoinHandle<()>>,
    count: u32,
}

impl FrameSeeker {
    /// Opens `path`, whose PTS table (from analysis) is `pts`.
    ///
    /// # Errors
    ///
    /// The errors of [`Decoder::open`].
    pub fn open(
        path: &Path,
        pts: Arc<PtsTable>,
        options: SeekerOptions,
    ) -> Result<Self, VideoError> {
        let decoder = Decoder::open(path, &options.decode)?;
        let count = pts.len();
        let (requests, inbox) = mpsc::channel();
        let worker = std::thread::Builder::new()
            .name(String::from("blurrd-seeker"))
            .spawn(move || Worker::new(decoder, pts, options).run(&inbox))
            .map_err(|source| VideoError::Io {
                context: "starting the seek thread for",
                path: path.to_path_buf(),
                source,
            })?;
        Ok(Self {
            requests: Some(requests),
            worker: Some(worker),
            count,
        })
    }

    /// Number of frames in the video.
    pub fn frame_count(&self) -> u32 {
        self.count
    }

    /// The frame with presentation index `index` (a reference to the cached
    /// pixels, no copy).
    ///
    /// # Errors
    ///
    /// [`VideoError::FrameOutOfRange`] past the end, [`VideoError::PtsMismatch`]
    /// or [`VideoError::FrameNotFound`] when the file does not match the PTS
    /// table, decoding errors otherwise.
    pub fn frame(&mut self, index: u32) -> Result<VideoFrame, VideoError> {
        if index >= self.count {
            return Err(VideoError::FrameOutOfRange {
                index,
                count: self.count,
            });
        }
        let (reply, answer) = mpsc::channel();
        let stopped = || VideoError::SeekerStopped;
        self.requests
            .as_ref()
            .ok_or_else(stopped)?
            .send((index, reply))
            .map_err(|_| stopped())?;
        answer.recv().map_err(|_| stopped())?
    }
}

impl Drop for FrameSeeker {
    fn drop(&mut self) {
        // Closing the channel ends the worker loop.
        self.requests = None;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// The decoder thread's state.
struct Worker {
    decoder: Decoder,
    table: Arc<PtsTable>,
    options: SeekerOptions,
    cache: BTreeMap<u32, VideoFrame>,
    cache_bytes: usize,
    /// Index the decoder returns next, if known.
    position: Option<u32>,
    /// Last requested index; the cache is kept around it.
    focus: u32,
    /// Prefetch until the decoder reaches this index.
    prefetch_until: u32,
    gop: u32,
    last_key: Option<u32>,
}

impl Worker {
    fn new(decoder: Decoder, table: Arc<PtsTable>, options: SeekerOptions) -> Self {
        Self {
            decoder,
            table,
            options,
            cache: BTreeMap::new(),
            cache_bytes: 0,
            position: Some(0),
            focus: 0,
            prefetch_until: 0,
            gop: DEFAULT_GOP,
            last_key: None,
        }
    }

    fn run(mut self, inbox: &Receiver<(u32, Sender<Reply>)>) {
        loop {
            let request = if self.wants_prefetch() {
                match inbox.try_recv() {
                    Ok(request) => request,
                    Err(TryRecvError::Empty) => {
                        if self.decode_next().is_err() {
                            // Stop prefetching; the next request reports it.
                            self.prefetch_until = 0;
                        }
                        continue;
                    }
                    Err(TryRecvError::Disconnected) => return,
                }
            } else {
                match inbox.recv() {
                    Ok(request) => request,
                    Err(_) => return,
                }
            };
            let (index, reply) = request;
            let answer = self.serve(index);
            // The requester may have given up; nothing to do then.
            let _ = reply.send(answer);
        }
    }

    fn wants_prefetch(&self) -> bool {
        self.options.prefetch && self.position.is_some_and(|p| p < self.prefetch_until)
    }

    fn serve(&mut self, index: u32) -> Reply {
        self.focus = index;
        if !self.cache.contains_key(&index) {
            let forward = self
                .position
                .is_some_and(|p| p <= index && index - p <= FORWARD_LIMIT);
            if !forward {
                self.decoder.seek(index, &self.table)?;
                self.position = Some(index);
            }
            while !self.cache.contains_key(&index) {
                self.decode_next()?;
            }
        }
        // Keep the rest of this GOP and the next one warm.
        self.prefetch_until = index.saturating_add(2 * self.gop).min(self.table.len());
        self.evict();
        self.cache.get(&index).map_or_else(
            || {
                Err(VideoError::FrameNotFound {
                    index,
                    pts: self.table.pts(index).unwrap_or_default(),
                })
            },
            VideoFrame::share,
        )
    }

    /// Decodes one frame into the cache, checking it against the table.
    fn decode_next(&mut self) -> Result<(), VideoError> {
        let Some(frame) = self.decoder.next_frame()? else {
            self.position = None;
            return Err(VideoError::FrameOutOfRange {
                index: self.table.len(),
                count: self.table.len(),
            });
        };
        let index = frame.index();
        let expected = self.table.pts(index).ok_or(VideoError::FrameOutOfRange {
            index,
            count: self.table.len(),
        })?;
        if frame.pts() != expected {
            self.position = None;
            return Err(VideoError::PtsMismatch {
                index,
                expected,
                found: frame.pts(),
            });
        }
        if frame.is_key() {
            if let Some(last) = self.last_key
                && index > last
            {
                self.gop = (index - last).clamp(1, 600);
            }
            self.last_key = Some(index);
        }
        self.position = Some(index + 1);
        self.cache_bytes += frame_bytes(&frame);
        self.cache.insert(index, frame);
        Ok(())
    }

    /// Drops the cached frames farthest from the focus until the cache fits.
    fn evict(&mut self) {
        while self.cache_bytes > self.options.cache_bytes && self.cache.len() > 1 {
            let (Some(&first), Some(&last)) =
                (self.cache.keys().next(), self.cache.keys().next_back())
            else {
                break;
            };
            // Frames behind the focus go first: forward access is the norm.
            let victim = if self.focus.saturating_sub(first) >= last.saturating_sub(self.focus) / 2
            {
                first
            } else {
                last
            };
            if victim == self.focus {
                break;
            }
            if let Some(frame) = self.cache.remove(&victim) {
                self.cache_bytes -= frame_bytes(&frame);
            }
        }
    }
}

/// Approximate memory of a decoded frame.
fn frame_bytes(frame: &VideoFrame) -> usize {
    let pixels = frame.width() as usize * frame.height() as usize;
    match frame.layout() {
        PixelLayout::I420 | PixelLayout::Nv12 => pixels * 3 / 2,
        _ => pixels * 3,
    }
}
