//! The bounded frame pipeline of the analysis (plan 7.1 and 7.4).
//!
//! ```text
//! decode ─frames─► crop workers ─crops─► dispatch (reorder) ─┬─► kill-feed stage ──┐
//!                  (ROI copy, RGB,                           ├─► panel stage ──────┼─► collector
//!                   reference scale,                         ├─► screen stage ─────┘   (merge, tracks)
//!                   screen reads)                            └─► pass 0 (OCR, optional)
//! ```
//!
//! The decoder hands out reference-counted frames (no pixel copy); a fixed
//! number of crop workers cut and convert the HUD crops and read the screen
//! surfaces of single frames in parallel, at most [`REORDER_PER_WORKER`]
//! frames per worker ahead of the oldest frame not yet dispatched; the
//! dispatcher restores frame order and gives every stage a shared handle
//! to each frame's crops; each
//! stage runs its detectors and temporal models over the frames in order on
//! its own thread; the collector merges the stages' final boxes frame by
//! frame (each stage's output is delayed by its temporal model's
//! look-ahead) and builds the tracks. Every queue is bounded, so memory
//! does not depend on the clip length and a slow stage slows the decoder
//! down. The first error or a cancellation stops every thread at its next
//! frame.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

use blurrd_core::{AppError, Track};
use blurrd_video::{DecodeOptions, Decoder, VideoFrame};
use crossbeam_channel::{Receiver, Sender, bounded, unbounded};

use super::frame::{Cropper, FrameCrops};
use super::pass0::Pass0;
use super::previews::{PreviewOutput, Previews};
use super::stages::{Emitted, SOURCES, Stage};
use super::tracks::{RegionBox, TrackBuilder};
use crate::dictionary::Pass0Stats;
use crate::engine::PoolSizes;
use crate::job::{Job, canceled, internal};

/// How often a waiting thread looks at the stop flag.
pub(super) const POLL: Duration = Duration::from_millis(20);

/// How many frames per crop worker may be ahead of the oldest frame the
/// dispatcher still waits for. Bounds its reorder buffer, whose frames
/// hold decoder surfaces, when one worker is descheduled.
const REORDER_PER_WORKER: u32 = 4;

/// What the pipeline produced.
#[derive(Debug)]
pub(crate) struct PipelineOutput {
    /// The tracks, in id order.
    pub(crate) tracks: Vec<Track>,
    /// The PTS of every decoded frame, in presentation order.
    pub(crate) pts: Vec<i64>,
    /// The largest look-ahead of a temporal model, in frames.
    pub(crate) max_delay: u32,
    /// Pass 0 counters, if pass 0 ran to the end.
    pub(crate) pass0: Option<Pass0Stats>,
    /// Why pass 0 stopped, if it failed.
    pub(crate) pass0_error: Option<String>,
    /// What the previews produced, when they ran.
    pub(crate) previews: Option<PreviewOutput>,
}

/// The parts of one analysis pipeline.
pub(crate) struct Pipeline<'a> {
    /// The video.
    pub(crate) input: &'a Path,
    /// How it is decoded.
    pub(crate) decode: DecodeOptions,
    /// Makes the crops (cloned per worker).
    pub(crate) cropper: Cropper,
    /// The detector stages.
    pub(crate) stages: Vec<Box<dyn Stage>>,
    /// Which sources the stages feed.
    pub(crate) sources: [bool; SOURCES],
    /// Pass 0, when OCR runs.
    pub(crate) pass0: Option<Pass0>,
    /// Thumbnails and the proxy, when wanted.
    pub(crate) previews: Option<Previews>,
    /// Pool and queue sizes.
    pub(crate) pools: PoolSizes,
    /// Report progress, with the number of frames the video is expected
    /// to have; `None` when another part of the job reports it (the
    /// render of a streaming run).
    pub(crate) progress: Option<u32>,
}

/// How often the watcher looks at the job's cancellation.
const CANCEL_POLL: Duration = Duration::from_millis(2);

/// The first error of any thread, and what stops them all: a flag that
/// working threads check, and a channel whose closing wakes the threads
/// that wait on a full queue at once.
struct Control<'a> {
    /// Shared with the OCR worker, which may outlive a stopped pipeline.
    stop: Arc<AtomicBool>,
    error: Mutex<Option<AppError>>,
    /// Dropped on the first error: closes `halted`.
    halt: Mutex<Option<Sender<()>>>,
    /// Never receives a value; disconnects when the threads stop.
    halted: Receiver<()>,
    /// The job, whose cancellation stops every thread too.
    job: &'a Job<'a>,
    /// How far crop workers may run ahead of the dispatcher.
    window: Window,
}

impl<'a> Control<'a> {
    /// The control of a pipeline of `job` with `workers` crop workers.
    fn new(job: &'a Job<'a>, workers: usize) -> Self {
        let (halt, halted) = bounded(0);
        let window = u32::try_from(workers)
            .unwrap_or(u32::MAX)
            .saturating_mul(REORDER_PER_WORKER);
        Self {
            stop: Arc::new(AtomicBool::new(false)),
            error: Mutex::new(None),
            halt: Mutex::new(Some(halt)),
            halted,
            job,
            window: Window::new(window),
        }
    }

    /// Records `error` (the first one wins) and stops every thread.
    fn fail(&self, error: AppError) {
        self.stop.store(true, Ordering::Relaxed);
        if let Ok(mut slot) = self.error.lock() {
            slot.get_or_insert(error);
        }
        drop(
            self.halt
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take(),
        );
        self.window.wake();
    }

    /// Whether the threads stop: after an error, or once the job was
    /// canceled (then the cancellation is the error).
    fn stopped(&self) -> bool {
        if self.stop.load(Ordering::Relaxed) {
            return true;
        }
        if self.job.is_canceled() {
            self.fail(canceled());
            return true;
        }
        false
    }

    /// Turns a cancellation of the job into a stop of every thread, even
    /// those that wait, until the threads stop or `finished` is set.
    fn watch(&self, finished: &AtomicBool) {
        while !finished.load(Ordering::Relaxed) && !self.stopped() {
            std::thread::sleep(CANCEL_POLL);
        }
    }

    /// Runs `work`, recording its error.
    fn guard<T>(&self, work: impl FnOnce() -> Result<T, AppError>) -> Option<T> {
        match work() {
            Ok(value) => Some(value),
            Err(error) => {
                self.fail(error);
                None
            }
        }
    }

    fn into_error(self) -> Option<AppError> {
        match self.error.into_inner() {
            Ok(error) => error,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

/// The frame the dispatcher hands out next, so crop workers do not run
/// too far ahead of it.
struct Window {
    next: Mutex<u32>,
    moved: Condvar,
    size: u32,
}

impl Window {
    fn new(size: u32) -> Self {
        Self {
            next: Mutex::new(0),
            moved: Condvar::new(),
            size: size.max(1),
        }
    }

    /// Waits until frame `index` is inside the window; false when
    /// `stopped` says the pipeline stops first.
    fn enter(&self, index: u32, stopped: impl Fn() -> bool) -> bool {
        let mut next = self.next.lock().unwrap_or_else(PoisonError::into_inner);
        while index >= next.saturating_add(self.size) {
            if stopped() {
                return false;
            }
            next = self
                .moved
                .wait_timeout(next, POLL)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        true
    }

    /// Moves the window to start at frame `next`.
    fn advance(&self, next: u32) {
        *self.next.lock().unwrap_or_else(PoisonError::into_inner) = next;
        self.moved.notify_all();
    }

    /// Wakes the waiting workers (to see that the pipeline stops).
    fn wake(&self) {
        let _next = self.next.lock().unwrap_or_else(PoisonError::into_inner);
        self.moved.notify_all();
    }
}

/// Sends `value`, waiting while the queue is full, until the receiver is
/// gone or the pipeline stops; returns whether it was sent.
fn send<T>(tx: &Sender<T>, value: T, control: &Control<'_>) -> bool {
    if control.stopped() {
        return false;
    }
    crossbeam_channel::select! {
        send(tx, value) -> sent => sent.is_ok(),
        recv(control.halted) -> _ => false,
    }
}

impl Pipeline<'_> {
    /// Runs the pipeline to the end of the video. `on_frame` is called on
    /// the calling thread for every frame, in order, as soon as its boxes
    /// are final, with the track builder that holds them.
    pub(crate) fn run(
        self,
        job: &Job<'_>,
        on_frame: &mut dyn FnMut(u32, &TrackBuilder),
    ) -> Result<PipelineOutput, AppError> {
        let Self {
            input,
            decode,
            cropper,
            stages,
            sources,
            pass0,
            previews,
            pools,
            progress,
        } = self;
        let workers = pools.crop_workers.max(1);
        let control = Control::new(job, workers);
        let finished = AtomicBool::new(false);
        let (frame_tx, frame_rx) = bounded::<VideoFrame>(pools.frame_queue.max(1));
        let (crop_tx, crop_rx) = bounded::<Result<FrameCrops, AppError>>(workers * 2);
        let (emit_tx, emit_rx) = unbounded::<Emitted>();
        let hud_stages = stages.iter().filter(|s| s.is_hud()).count();
        let (signal_tx, signal_rx) = unbounded::<(u32, bool)>();
        let (to_stages, stage_inputs): (Vec<_>, Vec<_>) = stages
            .iter()
            .map(|_| bounded::<Arc<FrameCrops>>(pools.stage_queue.max(1)))
            .unzip();
        let (pass0_tx, pass0_rx) = bounded::<Arc<FrameCrops>>(pools.ocr_queue.max(1));
        let pass0_tx = pass0.is_some().then_some(pass0_tx);
        let (previews_tx, previews_rx) = bounded::<Arc<FrameCrops>>(pools.stage_queue.max(1));
        let previews_tx = previews.is_some().then_some(previews_tx);

        let (pts, tracks, max_delay, pass0_result, previews) = std::thread::scope(|scope| {
            let (control, finished) = (&control, &finished);
            scope.spawn(move || control.watch(finished));
            let decoder = scope.spawn(move || {
                control
                    .guard(|| decode_frames(input, &decode, &frame_tx, control))
                    .unwrap_or_default()
            });
            for _ in 0..workers {
                let (frame_rx, crop_tx, mut cropper) =
                    (frame_rx.clone(), crop_tx.clone(), cropper.clone());
                scope.spawn(move || crop_frames(&frame_rx, &crop_tx, &mut cropper, control));
            }
            drop((frame_rx, crop_tx));
            scope.spawn(move || {
                let sides: Vec<_> = pass0_tx.iter().chain(&previews_tx).collect();
                control.guard(|| dispatch(&crop_rx, &to_stages, &sides, control));
            });
            for (stage, rx) in stages.into_iter().zip(stage_inputs) {
                let (emit_tx, signal_tx) = (emit_tx.clone(), signal_tx.clone());
                let signals = (hud_stages > 0 && stage.is_hud()).then_some(signal_tx);
                scope.spawn(move || {
                    control.guard(|| run_stage(stage, &rx, &emit_tx, signals.as_ref(), control));
                });
            }
            drop((emit_tx, signal_tx));
            let pass0 =
                pass0.map(|pass0| spawn_pass0(pass0, pass0_rx, signal_rx, hud_stages, control));
            let previews = previews.map(|previews| {
                scope.spawn(move || previews.run(&previews_rx, &|| control.stopped()))
            });
            let collected = collect(&emit_rx, sources, progress, job, control, on_frame);
            let pts = decoder.join().unwrap_or_default();
            let pass0 = pass0.map(|worker| finish_pass0(worker, control));
            let previews = previews.map(|handle| match handle.join() {
                Ok(output) => output,
                Err(panic) => std::panic::resume_unwind(panic),
            });
            let (tracks, max_delay) =
                collected.finish(u32::try_from(pts.len()).unwrap_or(u32::MAX), on_frame);
            finished.store(true, Ordering::Relaxed);
            (pts, tracks, max_delay, pass0, previews)
        });
        if let Some(error) = control.into_error() {
            return Err(error);
        }
        if progress.is_some() {
            let frames = u32::try_from(pts.len()).unwrap_or(u32::MAX);
            job.progress(frames, frames);
        }
        let (pass0, pass0_error) = match pass0_result {
            Some(Ok(stats)) => (Some(stats), None),
            Some(Err(reason)) => (None, Some(reason)),
            None => (None, None),
        };
        Ok(PipelineOutput {
            tracks,
            pts,
            max_delay,
            pass0,
            pass0_error,
            previews,
        })
    }
}

/// Decodes every frame into `frames`; returns the PTS table.
fn decode_frames(
    input: &Path,
    options: &DecodeOptions,
    frames: &Sender<VideoFrame>,
    control: &Control<'_>,
) -> Result<Vec<i64>, AppError> {
    let mut decoder = Decoder::open(input, options)?;
    let mut pts = Vec::new();
    loop {
        if control.stopped() {
            break;
        }
        let Some(frame) = decoder.next_frame()? else {
            return Ok(pts);
        };
        pts.push(frame.pts());
        if !send(frames, frame, control) {
            break;
        }
    }
    // Stopped early: closing a GPU decoder takes tens of milliseconds,
    // and the job ends as soon as its threads stopped (plan M14: cancel
    // in under 100 ms), so the decoder closes on a thread of its own.
    let spawned = std::thread::Builder::new()
        .name(String::from("blurrd-release"))
        .spawn(move || drop(decoder));
    if let Err(error) = spawned {
        tracing::debug!(%error, "closed the decoder on the job's thread");
    }
    Ok(pts)
}

/// The OCR worker of pass 0, or why it could not start.
type Pass0Worker = Result<std::thread::JoinHandle<Result<Pass0Stats, String>>, String>;

/// Starts pass 0 on its own thread, outside the pipeline's scope: an OCR
/// call cannot be interrupted, and closing the OCR sessions takes a few
/// hundred milliseconds, so a stopping pipeline does not wait for them.
fn spawn_pass0(
    pass0: Pass0,
    frames: Receiver<Arc<FrameCrops>>,
    structure: Receiver<(u32, bool)>,
    hud_stages: usize,
    control: &Control<'_>,
) -> Pass0Worker {
    let stop = Arc::clone(&control.stop);
    std::thread::Builder::new()
        .name(String::from("blurrd-pass0"))
        .spawn(move || {
            pass0.run(&frames, &structure, hud_stages, &|| {
                stop.load(Ordering::Relaxed)
            })
        })
        .map_err(|e| format!("the OCR worker could not start: {e}"))
}

/// The result of pass 0 once the pipeline ended: waits for the worker,
/// unless the pipeline stopped (then the worker ends on its own).
fn finish_pass0(worker: Pass0Worker, control: &Control<'_>) -> Result<Pass0Stats, String> {
    let worker = worker?;
    if control.stopped() {
        return Err(String::from("the analysis stopped"));
    }
    worker
        .join()
        .unwrap_or_else(|_| Err(String::from("the OCR worker panicked")))
}

/// Cuts the crops of the frames from `frames` into `crops`, inside the
/// reorder window, until the frames end or the pipeline stops.
fn crop_frames(
    frames: &Receiver<VideoFrame>,
    crops: &Sender<Result<FrameCrops, AppError>>,
    cropper: &mut Cropper,
    control: &Control<'_>,
) {
    for frame in frames {
        if !control.window.enter(frame.index(), || control.stopped()) {
            return;
        }
        if !send(crops, cropper.crops(frame), control) {
            return;
        }
    }
}

/// Restores frame order and hands every frame to each stage and side
/// output (pass 0, previews).
fn dispatch(
    crops: &Receiver<Result<FrameCrops, AppError>>,
    stages: &[Sender<Arc<FrameCrops>>],
    sides: &[&Sender<Arc<FrameCrops>>],
    control: &Control<'_>,
) -> Result<(), AppError> {
    let mut waiting: BTreeMap<u32, FrameCrops> = BTreeMap::new();
    let mut next = 0u32;
    for crop in crops {
        let crop = crop?;
        waiting.insert(crop.index, crop);
        while let Some(crop) = waiting.remove(&next) {
            let shared = Arc::new(crop);
            for tx in stages {
                if !send(tx, Arc::clone(&shared), control) {
                    return Ok(());
                }
            }
            // A side output stopping (OCR failed) does not stop the
            // analysis.
            for tx in sides {
                let _ = send(tx, Arc::clone(&shared), control);
            }
            next += 1;
        }
        control.window.advance(next);
    }
    if waiting.is_empty() || control.stopped() {
        Ok(())
    } else {
        Err(internal(format!(
            "frame {next} never reached the detectors"
        )))
    }
}

/// Runs one stage over its frames; sends what became final to the
/// collector and the structure signals to pass 0.
fn run_stage(
    mut stage: Box<dyn Stage>,
    frames: &Receiver<Arc<FrameCrops>>,
    emitted: &Sender<Emitted>,
    signals: Option<&Sender<(u32, bool)>>,
    control: &Control<'_>,
) -> Result<(), AppError> {
    for crops in frames {
        if control.stopped() {
            return Ok(());
        }
        let mut out = Emitted::default();
        let structure = stage.push(&crops, &mut out)?;
        if let Some(tx) = signals {
            let _ = tx.send((crops.index, structure));
        }
        if emitted.send(out).is_err() {
            return Ok(());
        }
    }
    if control.stopped() {
        return Ok(());
    }
    let mut out = Emitted::default();
    stage.finish(&mut out);
    let _ = emitted.send(out);
    Ok(())
}

/// The collector's state: per-frame boxes of each source until every
/// source made the frame final.
struct Collector {
    sources: [bool; SOURCES],
    pending: BTreeMap<u32, [Vec<RegionBox>; SOURCES]>,
    ready: [u32; SOURCES],
    next: u32,
    builder: TrackBuilder,
    max_delay: u32,
    boxes: Vec<RegionBox>,
}

impl Collector {
    fn add(&mut self, emitted: Emitted) {
        self.max_delay = self.max_delay.max(emitted.delay);
        for (source, frames) in emitted.frames.into_iter().enumerate() {
            for (frame, boxes) in frames {
                self.pending.entry(frame).or_default()[source] = boxes;
            }
            self.ready[source] = self.ready[source].max(emitted.ready[source]);
        }
    }

    /// Hands every frame before `until` to the builder and `on_frame`.
    fn advance(&mut self, until: u32, on_frame: &mut dyn FnMut(u32, &TrackBuilder)) {
        while self.next < until {
            let frame = self.next;
            self.boxes.clear();
            if let Some(per_source) = self.pending.remove(&frame) {
                for boxes in per_source {
                    self.boxes.extend(boxes);
                }
            }
            self.builder.push(frame, &self.boxes);
            on_frame(frame, &self.builder);
            self.next += 1;
        }
    }

    /// The frame every enabled source has made final up to.
    fn until(&self) -> u32 {
        self.ready
            .iter()
            .zip(self.sources)
            .filter(|(_, on)| *on)
            .map(|(&r, _)| r)
            .min()
            .unwrap_or(u32::MAX)
    }

    /// Flushes the remaining frames of a `frames`-frame video.
    fn finish(
        mut self,
        frames: u32,
        on_frame: &mut dyn FnMut(u32, &TrackBuilder),
    ) -> (Vec<Track>, u32) {
        self.advance(frames, on_frame);
        (self.builder.finish(), self.max_delay)
    }
}

/// Receives the stages' output until they all finished, handing frames to
/// the builder as soon as every source made them final.
fn collect(
    emitted: &Receiver<Emitted>,
    sources: [bool; SOURCES],
    progress: Option<u32>,
    job: &Job<'_>,
    control: &Control<'_>,
    on_frame: &mut dyn FnMut(u32, &TrackBuilder),
) -> Collector {
    let mut collector = Collector {
        sources,
        pending: BTreeMap::new(),
        ready: [0; SOURCES],
        next: 0,
        builder: TrackBuilder::new(),
        max_delay: 0,
        boxes: Vec::new(),
    };
    for out in emitted {
        if control.stopped() {
            break;
        }
        collector.add(out);
        // Without any source, frames are final as soon as they are decoded;
        // the end of the video flushes them.
        let until = collector.until();
        if until != u32::MAX {
            collector.advance(until, on_frame);
            if let Some(expected) = progress {
                job.progress(collector.next, expected.max(collector.next));
            }
        }
    }
    // Records a cancellation that came after the last frame.
    control.stopped();
    collector
}
