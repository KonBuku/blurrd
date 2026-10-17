//! Job execution and the job channels (architecture plan 9.1 and 9.2).
//!
//! Every job (analysis, render, re-verification, the files of a run)
//! waits in one queue and runs on the queue's worker thread, one after
//! another: parallelism lives inside a job, and two jobs would only fight
//! over the decoder, the NVENC sessions and the CPU (plan 8).
//!
//! A job reports through its IPC channel. [`JobSink`] forwards the
//! engine's events and keeps `Progress` ordered and at most 20 per second
//! whatever thread the engine reports from: a progress event is dropped
//! when its frame is behind the last one sent in the same pass, or when
//! one was sent less than 50 ms ago (the last frame always passes).
//! `Started`, and a `Stage` of a pass the engine has not reported before
//! (a run of an edited file analyses and then renders under one job),
//! begin a pass whose frames start again from 0. The sink
//! holds its lock while it sends, so the channel's order is the order of
//! the checks.
//!
//! Pausing stops the queue from starting jobs and holds the running job
//! at its next progress report; cancelling a waiting job ends it at once,
//! a running one at the engine's next frame.

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use blurrd_core::{AppError, ErrorKind, JobEvent, JobId};
use blurrd_engine::{CancellationToken, ProgressSink};
use tauri::ipc::Channel;

use crate::projects::lock;

/// Shortest time between two `Progress` events of a job (20 per second).
pub(crate) const PROGRESS_INTERVAL: Duration = Duration::from_millis(50);

/// How often a paused job looks at its cancellation.
const PAUSE_POLL: Duration = Duration::from_millis(50);

/// Where the events of a job go.
pub(crate) trait EventOut: Send + Sync {
    /// Delivers one event.
    fn send(&self, event: JobEvent);
}

impl EventOut for Channel<JobEvent> {
    fn send(&self, event: JobEvent) {
        if let Err(error) = Channel::send(self, event) {
            tauri_plugin_log::log::warn!("could not send a job event: {error}");
        }
    }
}

/// Decides which events reach the channel.
#[derive(Debug, Default)]
struct ProgressFilter {
    pass: u32,
    last_frame: Option<u32>,
    last_sent: Option<Instant>,
}

impl ProgressFilter {
    /// Whether `event` is sent at `now`; remembers the progress it lets
    /// through.
    fn admit(&mut self, event: &JobEvent, now: Instant) -> bool {
        let (frame, total) = match *event {
            JobEvent::Progress { frame, total, .. } => (frame, total),
            // A new pass of the job (a run analyses, then renders the
            // project): its progress starts again from frame 0.
            JobEvent::Started { .. } => {
                *self = Self::default();
                return true;
            }
            JobEvent::Stage { pass, .. } => {
                if pass != self.pass {
                    *self = Self {
                        pass,
                        ..Self::default()
                    };
                }
                return true;
            }
            _ => return true,
        };
        if self.last_frame.is_some_and(|last| frame < last) {
            return false;
        }
        let due = self
            .last_sent
            .is_none_or(|sent| now.duration_since(sent) >= PROGRESS_INTERVAL);
        if !due && frame < total {
            return false;
        }
        self.last_frame = Some(frame);
        self.last_sent = Some(now);
        true
    }
}

/// The engine's progress sink for one job.
pub(crate) struct JobSink {
    job: JobId,
    out: Arc<dyn EventOut>,
    filter: Mutex<ProgressFilter>,
    pause: Arc<PauseGate>,
    cancel: CancellationToken,
}

impl std::fmt::Debug for JobSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JobSink")
            .field("job", &self.job)
            .finish_non_exhaustive()
    }
}

impl JobSink {
    /// The job's id.
    pub(crate) fn job(&self) -> JobId {
        self.job
    }

    /// The job's cancellation.
    pub(crate) fn cancel(&self) -> &CancellationToken {
        &self.cancel
    }

    /// Ends a job that failed before the engine took it.
    pub(crate) fn failed(&self, error: AppError) {
        self.emit(JobEvent::Failed {
            job: self.job,
            error,
        });
    }
}

impl ProgressSink for JobSink {
    fn emit(&self, event: JobEvent) {
        if matches!(event, JobEvent::Progress { .. }) {
            self.pause.wait(&self.cancel);
        }
        let mut filter = lock(&self.filter);
        if filter.admit(&event, Instant::now()) {
            self.out.send(event);
        }
    }
}

/// Holds jobs while the queue is paused.
#[derive(Debug, Default)]
pub(crate) struct PauseGate {
    paused: Mutex<bool>,
    changed: Condvar,
}

impl PauseGate {
    fn set(&self, paused: bool) {
        *lock(&self.paused) = paused;
        self.changed.notify_all();
    }

    fn is_paused(&self) -> bool {
        *lock(&self.paused)
    }

    /// Returns once the queue runs or `cancel` fired.
    fn wait(&self, cancel: &CancellationToken) {
        let mut paused = lock(&self.paused);
        while *paused && !cancel.is_cancelled() {
            paused = self
                .changed
                .wait_timeout(paused, PAUSE_POLL)
                .map_or_else(|e| e.into_inner().0, |(guard, _)| guard);
        }
    }
}

/// The work of a job: runs with the job's sink, which it must end with
/// `Finished`, `Failed` or `Canceled` (the engine's jobs do).
pub(crate) type Work = Box<dyn FnOnce(&JobSink) + Send>;

/// A job waiting in the queue.
struct Waiting {
    job: JobId,
    out: Arc<dyn EventOut>,
    cancel: CancellationToken,
    work: Work,
}

#[derive(Default)]
struct QueueState {
    waiting: VecDeque<Waiting>,
    running: Option<(JobId, CancellationToken)>,
    closed: bool,
}

#[derive(Default)]
struct Shared {
    state: Mutex<QueueState>,
    wake: Condvar,
    pause: Arc<PauseGate>,
}

/// The job queue and its worker thread.
pub(crate) struct Jobs {
    shared: Arc<Shared>,
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl std::fmt::Debug for Jobs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Jobs").finish_non_exhaustive()
    }
}

impl Jobs {
    /// An empty queue with its worker thread.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Internal`] when the thread cannot be started.
    pub(crate) fn start() -> Result<Self, AppError> {
        let shared = Arc::new(Shared::default());
        let worker_shared = Arc::clone(&shared);
        let worker = std::thread::Builder::new()
            .name("blurrd-jobs".to_owned())
            .spawn(move || run_queue(&worker_shared))
            .map_err(|e| {
                AppError::new(ErrorKind::Internal, "The job queue could not start.")
                    .with_detail(e.to_string())
            })?;
        Ok(Self {
            shared,
            worker: Mutex::new(Some(worker)),
        })
    }

    /// Queues `work` as job `job`, reporting to `out`.
    pub(crate) fn enqueue(&self, job: JobId, out: Arc<dyn EventOut>, work: Work) {
        let mut state = lock(&self.shared.state);
        state.waiting.push_back(Waiting {
            job,
            out,
            cancel: CancellationToken::new(),
            work,
        });
        self.shared.wake.notify_all();
    }

    /// Cancels job `job`: a waiting job ends at once with `Canceled`, the
    /// running one at the engine's next frame. Unknown and finished jobs
    /// are ignored.
    pub(crate) fn cancel(&self, job: JobId) {
        let removed = {
            let mut state = lock(&self.shared.state);
            if let Some((running, cancel)) = &state.running
                && *running == job
            {
                cancel.cancel();
                None
            } else {
                let index = state.waiting.iter().position(|waiting| waiting.job == job);
                index.and_then(|index| state.waiting.remove(index))
            }
        };
        if let Some(waiting) = removed {
            waiting.out.send(JobEvent::Canceled { job });
        }
    }

    /// Pauses the queue: no job starts, the running job waits at its next
    /// progress report.
    pub(crate) fn pause(&self) {
        self.shared.pause.set(true);
    }

    /// Resumes the queue.
    pub(crate) fn resume(&self) {
        self.shared.pause.set(false);
        // Take the queue lock so the worker cannot miss the wake-up.
        let _state = lock(&self.shared.state);
        self.shared.wake.notify_all();
    }

    /// Cancels every job and waits for the worker to stop (app exit), so
    /// no job leaves a partial output behind.
    pub(crate) fn shutdown(&self) {
        let waiting: Vec<Waiting> = {
            let mut state = lock(&self.shared.state);
            state.closed = true;
            if let Some((_, cancel)) = &state.running {
                cancel.cancel();
            }
            state.waiting.drain(..).collect()
        };
        for waiting in waiting {
            waiting.out.send(JobEvent::Canceled { job: waiting.job });
        }
        self.shared.pause.set(false);
        self.shared.wake.notify_all();
        if let Some(worker) = lock(&self.worker).take()
            && worker.join().is_err()
        {
            tauri_plugin_log::log::warn!("the job queue stopped with a panic");
        }
    }
}

/// The worker loop: takes the next job whenever the queue runs.
fn run_queue(shared: &Shared) {
    loop {
        let next = {
            let mut state = lock(&shared.state);
            loop {
                if state.closed {
                    return;
                }
                if !shared.pause.is_paused()
                    && let Some(next) = state.waiting.pop_front()
                {
                    state.running = Some((next.job, next.cancel.clone()));
                    break next;
                }
                state = shared
                    .wake
                    .wait_timeout(state, PAUSE_POLL)
                    .map_or_else(|e| e.into_inner().0, |(guard, _)| guard);
            }
        };
        let sink = JobSink {
            job: next.job,
            out: next.out,
            filter: Mutex::new(ProgressFilter::default()),
            pause: Arc::clone(&shared.pause),
            cancel: next.cancel,
        };
        let work = next.work;
        if catch_unwind(AssertUnwindSafe(|| work(&sink))).is_err() {
            sink.failed(
                AppError::new(ErrorKind::Internal, "blurrd ran into an internal error.")
                    .with_detail("the job panicked"),
            );
        }
        lock(&shared.state).running = None;
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use blurrd_core::{JobEvent, JobId, JobKind, JobSummary, Stage};
    use blurrd_engine::ProgressSink;
    use uuid::Uuid;

    use super::{EventOut, Jobs, PROGRESS_INTERVAL, ProgressFilter};

    /// The recorder reads the clock a moment after the sink did, so two
    /// arrival times may be closer than the sink's by a few microseconds.
    pub(crate) const CLOCK_SLACK: Duration = Duration::from_millis(1);

    /// Records the events of jobs with their arrival times.
    #[derive(Debug, Default)]
    pub(crate) struct Recorder {
        pub(crate) events: Mutex<Vec<(Instant, JobEvent)>>,
    }

    impl EventOut for Recorder {
        fn send(&self, event: JobEvent) {
            self.events.lock().unwrap().push((Instant::now(), event));
        }
    }

    impl Recorder {
        pub(crate) fn kinds(&self) -> Vec<&'static str> {
            self.events
                .lock()
                .unwrap()
                .iter()
                .map(|(_, event)| match event {
                    JobEvent::Started { .. } => "Started",
                    JobEvent::Stage { .. } => "Stage",
                    JobEvent::Progress { .. } => "Progress",
                    JobEvent::Warning { .. } => "Warning",
                    JobEvent::Finished { .. } => "Finished",
                    JobEvent::Failed { .. } => "Failed",
                    JobEvent::Canceled { .. } => "Canceled",
                    _ => "other",
                })
                .collect()
        }

        /// Waits until an event ends job `job`.
        pub(crate) fn wait_end(&self, job: JobId) {
            let deadline = Instant::now() + Duration::from_secs(600);
            while Instant::now() < deadline {
                let ended = self.events.lock().unwrap().iter().any(|(_, event)| {
                    event.job() == job
                        && matches!(
                            event,
                            JobEvent::Finished { .. }
                                | JobEvent::Failed { .. }
                                | JobEvent::Canceled { .. }
                        )
                });
                if ended {
                    return;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            panic!("job {job} did not end");
        }
    }

    fn progress(frame: u32) -> JobEvent {
        JobEvent::Progress {
            job: JobId(Uuid::nil()),
            frame,
            total: 100,
            fps: 1.0,
            eta_ms: 0,
        }
    }

    fn finished(job: JobId) -> JobEvent {
        JobEvent::Finished {
            job,
            summary: JobSummary {
                frames: 100,
                elapsed_ms: 1,
                output: None,
                verify: None,
                warnings: 0,
            },
        }
    }

    #[test]
    fn progress_is_ordered_and_rate_limited() {
        let mut filter = ProgressFilter::default();
        let t0 = Instant::now();
        assert!(filter.admit(&progress(10), t0));
        // Too soon, and out of order.
        assert!(!filter.admit(&progress(11), t0 + Duration::from_millis(10)));
        assert!(!filter.admit(&progress(9), t0 + PROGRESS_INTERVAL * 2));
        assert!(filter.admit(&progress(12), t0 + PROGRESS_INTERVAL));
        // The last frame always passes; other events too.
        assert!(filter.admit(&progress(100), t0 + PROGRESS_INTERVAL));
        assert!(filter.admit(&finished(JobId(Uuid::nil())), t0 + PROGRESS_INTERVAL));
    }

    #[test]
    fn a_new_pass_starts_its_progress_again() {
        let mut filter = ProgressFilter::default();
        let t0 = Instant::now();
        let stage = |stage, pass| JobEvent::Stage {
            job: JobId(Uuid::nil()),
            stage,
            pass,
        };
        assert!(filter.admit(&progress(100), t0));
        // Another stage of the same pass keeps its progress.
        assert!(filter.admit(&stage(Stage::Verify, 0), t0));
        assert!(!filter.admit(&progress(50), t0 + PROGRESS_INTERVAL));
        // The render's first frames right after the analysis's last.
        assert!(filter.admit(&stage(Stage::BlurEncode, 1), t0));
        assert!(filter.admit(&progress(0), t0));
        assert!(!filter.admit(&progress(1), t0 + Duration::from_millis(10)));
        assert!(filter.admit(&progress(30), t0 + PROGRESS_INTERVAL));
    }

    fn job() -> JobId {
        JobId(Uuid::now_v7())
    }

    /// A job of two passes like a run of an edited file: the analysis
    /// reports frames 0 to 199, the render reports them again.
    #[test]
    fn both_passes_of_a_two_pass_job_report_progress() {
        let jobs = Jobs::start().unwrap();
        let recorder = Arc::new(Recorder::default());
        let id = job();
        let pass = |sink: &super::JobSink, stage, pass| {
            sink.emit(JobEvent::Stage {
                job: sink.job(),
                stage,
                pass,
            });
            for frame in 0..200 {
                sink.emit(JobEvent::Progress {
                    job: sink.job(),
                    frame,
                    total: 199,
                    fps: 1.0,
                    eta_ms: 0,
                });
                std::thread::sleep(Duration::from_millis(2));
            }
        };
        jobs.enqueue(
            id,
            recorder.clone(),
            Box::new(move |sink| {
                sink.emit(JobEvent::Started {
                    job: sink.job(),
                    kind: JobKind::Run,
                    total_frames: 200,
                    passes: 2,
                });
                pass(sink, Stage::Detect, 0);
                pass(sink, Stage::BlurEncode, 1);
                sink.emit(finished(sink.job()));
            }),
        );
        recorder.wait_end(id);
        let events = recorder.events.lock().unwrap();
        // The progress of each pass, split at the stage events.
        let mut passes: Vec<Vec<u32>> = Vec::new();
        for (_, event) in events.iter() {
            match *event {
                JobEvent::Stage { .. } => passes.push(Vec::new()),
                JobEvent::Progress { frame, .. } => passes.last_mut().unwrap().push(frame),
                _ => {}
            }
        }
        assert_eq!(passes.len(), 2);
        for frames in &passes {
            assert!(frames.len() > 3, "{frames:?}");
            assert!(frames.windows(2).all(|pair| pair[1] >= pair[0]));
            assert!(frames[0] < 50, "{frames:?}");
            assert_eq!(frames.last(), Some(&199));
        }
        drop(events);
        jobs.shutdown();
    }

    /// A job that reports `frames` progress events from four threads at
    /// once, as the engine's pipeline does, then finishes.
    fn noisy_work(frames: u32) -> super::Work {
        Box::new(move |sink| {
            std::thread::scope(|scope| {
                for part in 0..4 {
                    scope.spawn(move || {
                        for frame in (part..frames).step_by(4) {
                            sink.emit(JobEvent::Progress {
                                job: sink.job(),
                                frame,
                                total: frames - 1,
                                fps: 1.0,
                                eta_ms: 0,
                            });
                            std::thread::sleep(Duration::from_micros(200));
                        }
                    });
                }
            });
            sink.emit(finished(sink.job()));
        })
    }

    #[test]
    fn channels_get_ordered_progress_at_most_twenty_times_a_second() {
        let jobs = Jobs::start().unwrap();
        let recorder = Arc::new(Recorder::default());
        let id = job();
        jobs.enqueue(id, recorder.clone(), noisy_work(4000));
        recorder.wait_end(id);
        let events = recorder.events.lock().unwrap();
        let progress: Vec<(Instant, u32)> = events
            .iter()
            .filter_map(|(at, event)| match *event {
                JobEvent::Progress { frame, .. } => Some((*at, frame)),
                _ => None,
            })
            .collect();
        assert!(progress.len() > 2);
        for pair in progress.windows(2) {
            assert!(pair[1].1 >= pair[0].1, "progress went backwards");
            let gap = pair[1].0.duration_since(pair[0].0);
            // The last event may follow sooner.
            assert!(
                gap + CLOCK_SLACK >= PROGRESS_INTERVAL || pair[1].1 == 3999,
                "{gap:?}"
            );
        }
        assert_eq!(progress.last().map(|p| p.1), Some(3999));
        jobs.shutdown();
    }

    #[test]
    fn waiting_jobs_cancel_at_once_and_pausing_holds_the_queue() {
        let jobs = Jobs::start().unwrap();
        let recorder = Arc::new(Recorder::default());
        jobs.pause();
        let (first, second) = (job(), job());
        let work = |kind| -> super::Work {
            Box::new(move |sink| {
                sink.emit(JobEvent::Started {
                    job: sink.job(),
                    kind,
                    total_frames: 1,
                    passes: 1,
                });
                sink.emit(finished(sink.job()));
            })
        };
        jobs.enqueue(first, recorder.clone(), work(JobKind::Analyze));
        jobs.enqueue(second, recorder.clone(), work(JobKind::Render));
        std::thread::sleep(Duration::from_millis(150));
        assert!(recorder.kinds().is_empty(), "a paused queue starts nothing");
        jobs.cancel(second);
        assert_eq!(recorder.kinds(), vec!["Canceled"]);
        jobs.resume();
        recorder.wait_end(first);
        assert_eq!(recorder.kinds(), vec!["Canceled", "Started", "Finished"]);
        jobs.shutdown();
    }

    #[test]
    fn a_panicking_job_fails_and_the_queue_goes_on() {
        let jobs = Jobs::start().unwrap();
        let recorder = Arc::new(Recorder::default());
        let (bad, good) = (job(), job());
        jobs.enqueue(bad, recorder.clone(), Box::new(|_| panic!("boom")));
        jobs.enqueue(good, recorder.clone(), noisy_work(8));
        recorder.wait_end(good);
        let kinds = recorder.kinds();
        assert_eq!(kinds.first(), Some(&"Failed"));
        assert_eq!(kinds.last(), Some(&"Finished"));
        jobs.shutdown();
    }

    #[test]
    fn cancelling_the_running_job_fires_its_token() {
        let jobs = Jobs::start().unwrap();
        let recorder = Arc::new(Recorder::default());
        let id = job();
        jobs.enqueue(
            id,
            recorder.clone(),
            Box::new(|sink| {
                while !sink.cancel().is_cancelled() {
                    std::thread::sleep(Duration::from_millis(1));
                }
                sink.emit(JobEvent::Canceled { job: sink.job() });
            }),
        );
        std::thread::sleep(Duration::from_millis(50));
        let asked = Instant::now();
        jobs.cancel(id);
        recorder.wait_end(id);
        assert!(asked.elapsed() < Duration::from_millis(100));
        jobs.shutdown();
    }
}
