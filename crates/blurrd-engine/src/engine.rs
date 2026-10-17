//! The engine: its configuration and the state shared by its jobs.

use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};
use std::thread::JoinHandle;

use blurrd_core::{AppError, EncoderSetting, Luid};

/// Where the OCR runtime and models are.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct OcrSetup {
    /// The ONNX Runtime library (`onnxruntime.dll`).
    pub ort_dylib: PathBuf,
    /// The folder with the OCR models (`models.toml`'s files).
    pub model_dir: PathBuf,
    /// Whether OCR may use the GPU (DirectML); the CPU otherwise.
    pub gpu: bool,
}

impl OcrSetup {
    /// OCR with the runtime at `ort_dylib` and the models in `model_dir`,
    /// on the GPU when one works.
    pub fn new(ort_dylib: PathBuf, model_dir: PathBuf) -> Self {
        Self {
            ort_dylib,
            model_dir,
            gpu: true,
        }
    }
}

/// Explicit sizes of the analysis pools and queues (plan 7.4). Memory
/// depends on these, never on the clip length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct PoolSizes {
    /// Decoded frames waiting for a crop worker (reference-counted, so a
    /// queued frame holds one decoder surface).
    pub frame_queue: usize,
    /// Threads that bring frames to the reference scale and cut the crops.
    pub crop_workers: usize,
    /// Frames of crops waiting for each detector stage.
    pub stage_queue: usize,
    /// Frames of crops waiting for the OCR worker.
    pub ocr_queue: usize,
    /// OCR engines that read pass 0's zones at the same time, each on a
    /// thread of its own (at most four: the kill feed and banner, the left
    /// panel, the screen surfaces, the full frame).
    pub ocr_readers: usize,
}

impl PoolSizes {
    /// The sizes for a machine with `threads` hardware threads: a crop
    /// worker per four of them (two to six; the crops of a 1080p frame
    /// take about a quarter of the detectors' time, and the detector
    /// stages, the decoder and OCR need the rest), and four OCR readers
    /// from eight threads on (two below: their host work would take the
    /// detectors' cores).
    pub fn for_threads(threads: usize) -> Self {
        Self {
            frame_queue: 8,
            crop_workers: (threads / 4).clamp(2, 6),
            stage_queue: 16,
            ocr_queue: 16,
            ocr_readers: if threads >= 8 { 4 } else { 2 },
        }
    }
}

impl Default for PoolSizes {
    /// The sizes for this machine ([`PoolSizes::for_threads`] with its
    /// hardware threads): eight queued frames (plan 7.1), sixteen frames
    /// per stage and per OCR queue.
    fn default() -> Self {
        Self::for_threads(std::thread::available_parallelism().map_or(4, usize::from))
    }
}

/// How an [`Engine`] runs.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct EngineConfig {
    /// The cache folder (projects, PTS tables, thumbnails, capability
    /// caches).
    pub cache_dir: PathBuf,
    /// OCR, when installed.
    pub ocr: Option<OcrSetup>,
    /// The GPU to use, by LUID (`None`: the high-performance GPU).
    pub adapter: Option<Luid>,
    /// Whether the GPU may decode and encode. Without it, decoding is
    /// software and encoding uses Media Foundation.
    pub gpu: bool,
    /// Decoder threads for software decoding (`None`: FFmpeg's choice).
    pub decoder_threads: Option<u32>,
    /// Pool and queue sizes.
    pub pools: PoolSizes,
    /// The encoder of the preview proxy (`Auto`: the export chain).
    pub proxy_encoder: EncoderSetting,
}

impl EngineConfig {
    /// The defaults with the cache in `cache_dir`: GPU on, no OCR.
    pub fn new(cache_dir: PathBuf) -> Self {
        Self {
            cache_dir,
            ocr: None,
            adapter: None,
            gpu: true,
            decoder_threads: None,
            pools: PoolSizes::default(),
            proxy_encoder: EncoderSetting::Auto,
        }
    }
}

/// blurrd's default cache folder: `%LOCALAPPDATA%\blurrd` (the system's
/// temporary folder when that variable is not set).
pub fn default_cache_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map_or_else(std::env::temp_dir, PathBuf::from)
        .join("blurrd")
}

/// Runs blurrd's jobs. Cheap to create; jobs take `&self`, so one engine
/// can serve several jobs at once.
#[derive(Debug)]
pub struct Engine {
    config: EngineConfig,
    /// Threads releasing what finished jobs held (see [`Engine::release`]).
    releases: Mutex<Vec<JoinHandle<()>>>,
}

impl Engine {
    /// An engine with `config`.
    ///
    /// # Errors
    ///
    /// The error of [`blurrd_video::init`] when the FFmpeg libraries next to
    /// the program do not match the ones blurrd was built for.
    pub fn new(config: EngineConfig) -> Result<Self, AppError> {
        blurrd_video::init()?;
        Ok(Self {
            config,
            releases: Mutex::new(Vec::new()),
        })
    }

    /// Drops `held` on a thread of its own: closing OCR sessions takes
    /// from a few hundred milliseconds to seconds (DirectML tears its
    /// devices down), and a job is done before that. Dropping the engine
    /// waits for these threads, so nothing is still closing when the
    /// process ends.
    pub(crate) fn release<T: Send + 'static>(&self, held: T) {
        let mut releases = self.releases.lock().unwrap_or_else(PoisonError::into_inner);
        releases.retain(|thread| !thread.is_finished());
        match std::thread::Builder::new()
            .name(String::from("blurrd-release"))
            .spawn(move || drop(held))
        {
            Ok(thread) => releases.push(thread),
            Err(error) => tracing::debug!(%error, "released on the job's thread"),
        }
    }

    /// The configuration.
    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    /// The folder of the cached capability probes.
    pub(crate) fn caps_dir(&self) -> PathBuf {
        self.config.cache_dir.join("caps")
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let releases = self
            .releases
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner);
        for thread in releases.drain(..) {
            // A panic while dropping has nothing left to report.
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PoolSizes;

    #[test]
    fn pools_follow_the_hardware_threads() {
        let small = PoolSizes::for_threads(4);
        assert_eq!((small.crop_workers, small.ocr_readers), (2, 2));
        let desktop = PoolSizes::for_threads(16);
        assert_eq!((desktop.crop_workers, desktop.ocr_readers), (4, 4));
        let workstation = PoolSizes::for_threads(64);
        assert_eq!((workstation.crop_workers, workstation.ocr_readers), (6, 4));
    }
}
