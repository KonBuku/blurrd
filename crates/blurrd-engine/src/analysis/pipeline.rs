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
//! surfaces of single frames in parallel; the dispatcher restores frame
//! order and gives every stage a shared handle to each frame's crops; each
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
use std::sync::{Arc, Mutex};
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
const POLL: Duration = Duration::from_millis(20);

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

/// The first error of any thread, and the flag that stops them all.
#[derive(Default)]
struct Control {
    stop: AtomicBool,
    error: Mutex<Option<AppError>>,
}

impl Control {
    /// Records `error` (the first one wins) and stops every thread.
    fn fail(&self, error: AppError) {
        self.stop.store(true, Ordering::Relaxed);
        if let Ok(mut slot) = self.error.lock() {
            slot.get_or_insert(error);
        }
    }

    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
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

/// Sends `value`, waiting while the queue is full, until the receiver is
/// gone or the pipeline stops; returns whether it was sent.
fn send<T>(tx: &Sender<T>, mut value: T, control: &Control) -> bool {
    loop {
        if control.stopped() {
            return false;
        }
        match tx.send_timeout(value, POLL) {
            Ok(()) => return true,
            Err(crossbeam_channel::SendTimeoutError::Timeout(back)) => value = back,
            Err(crossbeam_channel::SendTimeoutError::Disconnected(_)) => return false,
        }
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
        let control = Control::default();
        let workers = pools.crop_workers.max(1);
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
            let control = &control;
            let decoder = scope.spawn(move || {
                control
                    .guard(|| decode_frames(input, &decode, &frame_tx, job, control))
                    .unwrap_or_default()
            });
            for _ in 0..workers {
                let (frame_rx, crop_tx, mut cropper) =
                    (frame_rx.clone(), crop_tx.clone(), cropper.clone());
                scope.spawn(move || {
                    for frame in frame_rx {
                        let crops = cropper.crops(frame);
                        if !send(&crop_tx, crops, control) {
                            break;
                        }
                    }
                });
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
            let pass0 = pass0
                .map(|pass0| scope.spawn(move || pass0.run(&pass0_rx, &signal_rx, hud_stages)));
            let previews = previews.map(|previews| scope.spawn(move || previews.run(&previews_rx)));
            let collected = collect(&emit_rx, sources, progress, job, control, on_frame);
            let pts = decoder.join().unwrap_or_default();
            let pass0 = pass0.map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|_| Err(String::from("the OCR worker panicked")))
            });
            let previews = previews.map(|handle| match handle.join() {
                Ok(output) => output,
                Err(panic) => std::panic::resume_unwind(panic),
            });
            let (tracks, max_delay) =
                collected.finish(u32::try_from(pts.len()).unwrap_or(u32::MAX), on_frame);
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
    job: &Job<'_>,
    control: &Control,
) -> Result<Vec<i64>, AppError> {
    let mut decoder = Decoder::open(input, options)?;
    let mut pts = Vec::new();
    loop {
        job.check_canceled()?;
        if control.stopped() {
            return Ok(pts);
        }
        let Some(frame) = decoder.next_frame()? else {
            return Ok(pts);
        };
        pts.push(frame.pts());
        if !send(frames, frame, control) {
            return Ok(pts);
        }
    }
}

/// Restores frame order and hands every frame to each stage and side
/// output (pass 0, previews).
fn dispatch(
    crops: &Receiver<Result<FrameCrops, AppError>>,
    stages: &[Sender<Arc<FrameCrops>>],
    sides: &[&Sender<Arc<FrameCrops>>],
    control: &Control,
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
    control: &Control,
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
    control: &Control,
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
    if job.is_canceled() {
        control.fail(canceled());
    }
    collector
}
