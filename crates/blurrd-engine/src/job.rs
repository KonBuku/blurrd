//! What every job shares: its id, the sink its events go to, the
//! cancellation token and the warning count.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use blurrd_core::{
    AppError, ErrorKind, JobEvent, JobId, JobKind, LeakStatus, Region, Stage, WarningCode,
};
use blurrd_video::sys::power::JobPower;
pub use tokio_util::sync::CancellationToken;

/// Receives the events of a job (the desktop app forwards them over IPC,
/// the CLI prints them). Called from the job's threads.
pub trait ProgressSink: Send + Sync {
    /// Handles one event.
    fn emit(&self, event: JobEvent);
}

/// `Progress` events are sent at most this often (20 per second).
const PROGRESS_INTERVAL: Duration = Duration::from_millis(50);

/// A running job.
pub(crate) struct Job<'a> {
    id: JobId,
    sink: &'a dyn ProgressSink,
    cancel: &'a CancellationToken,
    started: Instant,
    last_progress: Mutex<Option<Instant>>,
    warnings: AtomicU32,
    /// The current pass over the video, from 0.
    pass: AtomicU32,
    /// Keeps the machine awake and the process unthrottled while the job
    /// runs (plan 7.4).
    _power: JobPower,
}

impl<'a> Job<'a> {
    /// A job `id` reporting to `sink`, canceled by `cancel`, running on
    /// the calling thread.
    pub(crate) fn new(
        id: JobId,
        sink: &'a dyn ProgressSink,
        cancel: &'a CancellationToken,
    ) -> Self {
        Self {
            id,
            sink,
            cancel,
            started: Instant::now(),
            last_progress: Mutex::new(None),
            warnings: AtomicU32::new(0),
            pass: AtomicU32::new(0),
            _power: JobPower::begin(),
        }
    }

    /// Whether the job was canceled.
    pub(crate) fn is_canceled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// Time since the job started.
    pub(crate) fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// Number of warnings emitted so far.
    pub(crate) fn warnings(&self) -> u32 {
        self.warnings.load(Ordering::Relaxed)
    }

    /// Emits `Started` for a job of `passes` passes over `total_frames`
    /// frames each.
    pub(crate) fn started(&self, kind: JobKind, total_frames: u32, passes: u32) {
        self.sink.emit(JobEvent::Started {
            job: self.id,
            kind,
            total_frames,
            passes,
        });
    }

    /// Begins the next pass over the video: the stages from now on carry
    /// its index and its progress counts from frame 0.
    pub(crate) fn next_pass(&self) {
        self.pass.fetch_add(1, Ordering::Relaxed);
    }

    /// Emits `Stage` in the current pass.
    pub(crate) fn stage(&self, stage: Stage) {
        self.sink.emit(JobEvent::Stage {
            job: self.id,
            stage,
            pass: self.pass.load(Ordering::Relaxed),
        });
    }

    /// Emits `LeakFound` for a leak from `frame` in `region`.
    pub(crate) fn leak(&self, frame: u32, region: Region, status: LeakStatus) {
        self.sink.emit(JobEvent::LeakFound {
            job: self.id,
            frame,
            region,
            status,
        });
    }

    /// Emits a warning.
    pub(crate) fn warning(&self, code: WarningCode, message: String, frames: Option<(u32, u32)>) {
        self.warnings.fetch_add(1, Ordering::Relaxed);
        self.sink.emit(JobEvent::Warning {
            job: self.id,
            code,
            message,
            frames,
        });
    }

    /// Reports that `frame` of `total` frames are done, at most 20 times
    /// per second (always for the last frame).
    pub(crate) fn progress(&self, frame: u32, total: u32) {
        let now = Instant::now();
        {
            let Ok(mut last) = self.last_progress.lock() else {
                return;
            };
            let due = last.is_none_or(|t| now.duration_since(t) >= PROGRESS_INTERVAL);
            if !due && frame < total {
                return;
            }
            *last = Some(now);
        }
        let seconds = self.started.elapsed().as_secs_f64();
        let fps = if seconds > 0.0 {
            f64::from(frame) / seconds
        } else {
            0.0
        };
        let eta_ms = if fps > 0.0 {
            (f64::from(total.saturating_sub(frame)) / fps * 1000.0) as u64
        } else {
            0
        };
        self.sink.emit(JobEvent::Progress {
            job: self.id,
            frame,
            total,
            fps: fps as f32,
            eta_ms,
        });
    }

    /// Emits the event that ends the job for `result`: `Canceled` for a
    /// [`ErrorKind::Canceled`] error, `Failed` for other errors; `finish`
    /// builds `Finished` for a success.
    pub(crate) fn end<T>(
        &self,
        result: Result<T, AppError>,
        finish: impl FnOnce(&T) -> JobEvent,
    ) -> Result<T, AppError> {
        match &result {
            Ok(value) => self.sink.emit(finish(value)),
            Err(error) if error.kind == ErrorKind::Canceled => {
                self.sink.emit(JobEvent::Canceled { job: self.id });
            }
            Err(error) => self.sink.emit(JobEvent::Failed {
                job: self.id,
                error: error.clone(),
            }),
        }
        result
    }
}

/// The error of a canceled job.
pub(crate) fn canceled() -> AppError {
    AppError::new(ErrorKind::Canceled, "The job was canceled.")
}

/// An internal error (a bug or a broken invariant) with `detail`.
pub(crate) fn internal(detail: impl Into<String>) -> AppError {
    AppError::new(ErrorKind::Internal, "blurrd ran into an internal error.").with_detail(detail)
}
