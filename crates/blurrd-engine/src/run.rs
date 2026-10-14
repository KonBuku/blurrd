//! The streaming run: analysis and render of one video at the same time
//! (plan 7.4).
//!
//! ```text
//! analysis decoder ─► crops ─► detector stages ─► tracks ──boxes──┐
//!                                                                 ▼
//! render decoder (trailing) ─────────────────────────► blur + encode ─► mux
//! ```
//!
//! The analysis decoder leads. As soon as every temporal model made a
//! frame's boxes final (at most [`crate::LOOKAHEAD_MAX`] frames after the
//! analysis decoded it), the track builder gives the boxes the finished
//! plan will have on that frame, and they wait in a [`BoxStore`] until the
//! render reaches the frame. The render never blurs a frame whose boxes
//! are not final: it waits for them (back-pressure), so it trails the
//! analysis by at least the look-ahead. The output is therefore exactly
//! the output of an analysis followed by a render of its project.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use blurrd_core::{
    AppError, BlurBox, BlurParams, ErrorKind, ExportSettings, JobEvent, JobId, JobKind, JobSummary,
    RegionSet, VerifyState,
};
use blurrd_video::{BoxSource, DecodePurpose, HwDecode, VideoError};
use serde::Serialize;

use crate::analysis::{AnalyzeReport, AnalyzeRequest, expected_frames};
use crate::cache::{self, ProjectDir};
use crate::engine::Engine;
use crate::job::{CancellationToken, Job, canceled};
use crate::render::{RenderReport, RenderRequest, RenderTarget, transcode};

/// How often a render waiting for boxes looks at the cancellation.
const POLL: Duration = Duration::from_millis(20);

/// What to analyse and render.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct RunRequest {
    /// The job's id (its events carry it).
    pub job: JobId,
    /// The video.
    pub input: PathBuf,
    /// The output file; its extension picks the container.
    pub output: PathBuf,
    /// The regions to detect and blur.
    pub regions: RegionSet,
    /// Blur parameters.
    pub blur: BlurParams,
    /// Codec, encoder, quality, audio and container settings.
    pub export: ExportSettings,
    /// Name the output normally even though it is not verified.
    pub accept_unverified: bool,
    /// Whether to verify (pass 0 of the OCR verification when OCR is
    /// installed).
    pub verify: bool,
}

impl RunRequest {
    /// Every supported region of `input` into `output` with the default
    /// blur and export settings, with verification.
    pub fn new(job: JobId, input: PathBuf, output: PathBuf) -> Self {
        Self {
            job,
            input,
            output,
            regions: RegionSet::all(),
            blur: BlurParams::default(),
            export: ExportSettings::default(),
            accept_unverified: false,
            verify: true,
        }
    }
}

/// What a run did.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[non_exhaustive]
pub struct RunReport {
    /// The analysis.
    pub analysis: AnalyzeReport,
    /// The render.
    pub render: RenderReport,
    /// Whether the render ran while the analysis did (`false` when the
    /// video's cached project has overrides, which need the finished
    /// tracks: then the run analyses first and renders the project).
    pub streamed: bool,
    /// Wall-clock time of the whole run in milliseconds.
    pub elapsed_ms: u64,
}

/// The boxes of the frames the analysis made final, until the render
/// takes them. Frames without boxes are not stored.
struct BoxStore<'a> {
    frames: u32,
    state: Mutex<StoreState>,
    changed: Condvar,
    cancel: &'a CancellationToken,
}

#[derive(Default)]
struct StoreState {
    boxes: BTreeMap<u32, Vec<BlurBox>>,
    /// Every frame before this one is final.
    done: u32,
    /// The analysis ended (finished or failed).
    closed: bool,
}

impl<'a> BoxStore<'a> {
    fn new(frames: u32, cancel: &'a CancellationToken) -> Self {
        Self {
            frames,
            state: Mutex::new(StoreState::default()),
            changed: Condvar::new(),
            cancel,
        }
    }

    /// Stores the final boxes of `frame` (the frames come in order).
    fn push(&self, frame: u32, boxes: &[BlurBox]) {
        if let Ok(mut state) = self.state.lock() {
            if !boxes.is_empty() {
                state.boxes.insert(frame, boxes.to_vec());
            }
            state.done = frame + 1;
        }
        self.changed.notify_all();
    }

    /// No more frames will come.
    fn close(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.closed = true;
        }
        self.changed.notify_all();
    }
}

impl BoxSource for BoxStore<'_> {
    fn frame_count(&self) -> u32 {
        self.frames
    }

    fn boxes_into(&self, frame: u32, out: &mut Vec<BlurBox>) -> Result<(), VideoError> {
        out.clear();
        let mut state = self.state.lock().map_err(|_| VideoError::Canceled)?;
        loop {
            if frame < state.done {
                if let Some(boxes) = state.boxes.remove(&frame) {
                    *out = boxes;
                }
                return Ok(());
            }
            if state.closed || self.cancel.is_cancelled() {
                return Err(VideoError::Canceled);
            }
            state = self
                .changed
                .wait_timeout(state, POLL)
                .map_err(|_| VideoError::Canceled)?
                .0;
        }
    }
}

impl Engine {
    /// Analyses a video and renders it in one job, the render trailing the
    /// analysis (plan 7.4). The project and the PTS table are saved in the
    /// cache as by [`Engine::analyze`], and the output is identical to a
    /// render of that project. When the video's cached project has
    /// overrides, they need the finished tracks: the run then analyses
    /// first and renders the project afterwards.
    ///
    /// # Errors
    ///
    /// The errors of [`Engine::analyze`] and [`Engine::render`];
    /// [`ErrorKind::Canceled`] when `cancel` fires (no output is left
    /// behind).
    pub fn run(
        &self,
        req: &RunRequest,
        sink: &dyn crate::ProgressSink,
        cancel: &CancellationToken,
    ) -> Result<RunReport, AppError> {
        // The job's own token: a failing render stops the analysis
        // without canceling the caller's token.
        let inner = cancel.child_token();
        let job = Job::new(req.job, sink, &inner);
        let result = self.run_job(req, &job, &inner).map_err(|error| {
            if cancel.is_cancelled() {
                canceled()
            } else {
                error
            }
        });
        job.end(result, |report| JobEvent::Finished {
            job: req.job,
            summary: JobSummary {
                frames: report.render.frames,
                elapsed_ms: report.elapsed_ms,
                output: Some(report.render.output.clone()),
                verify: Some(if report.render.verified {
                    VerifyState::Clean
                } else {
                    VerifyState::Unverified
                }),
                warnings: job.warnings(),
            },
        })
    }

    fn run_job(
        &self,
        req: &RunRequest,
        job: &Job<'_>,
        inner: &CancellationToken,
    ) -> Result<RunReport, AppError> {
        let analyze = AnalyzeRequest {
            job: req.job,
            input: req.input.clone(),
            regions: req.regions,
            blur: req.blur,
            verify: req.verify,
            // Thumbnails and the proxy serve the desktop app's preview; a
            // run goes straight to the output.
            previews: false,
        };
        let prepared = self.prepare_analysis(&req.input, req.verify, job)?;
        let dir = ProjectDir::new(&self.config().cache_dir, &prepared.fingerprint);
        if dir.read_project().is_some_and(|p| !p.overrides.is_empty()) {
            let expected = expected_frames(&prepared.info);
            job.started(JobKind::Run, expected);
            let analysis =
                self.run_analysis(&analyze, prepared, job, Some(expected), &mut |_, _| {})?;
            let mut render = RenderRequest::new(
                req.job,
                req.input.clone(),
                analysis.project,
                req.output.clone(),
            );
            render.export = req.export.clone();
            render.accept_unverified = req.accept_unverified;
            let rendered = self.render_project(&render, job, None)?;
            return Ok(RunReport {
                analysis: analysis.report,
                render: rendered,
                streamed: false,
                elapsed_ms: elapsed_ms(job),
            });
        }

        let probe = self.start_encoder_probe(&req.export);
        // The render needs the PTS table before the analysis has decoded
        // the video: one pass over the container's packets. The render
        // checks every decoded frame against it, and the analysis' own
        // table must equal it.
        let pts = Arc::new(blurrd_video::pts_table(&req.input)?);
        job.started(JobKind::Run, pts.len());
        let info = prepared.info.clone();
        // The render decoder takes the other decoder than the analysis
        // where that is not much slower, to share the work between CPU and
        // GPU.
        let decode = if prepared.decode.hw == HwDecode::Software {
            DecodePurpose::SpareCpu
        } else {
            DecodePurpose::Throughput
        };
        let target = RenderTarget {
            output: &req.output,
            export: &req.export,
            // A fresh analysis is not verified yet (verification arrives
            // with M17).
            verified: req.accept_unverified,
            blur: &req.blur,
            sy: prepared.geometry.sy,
            decode,
        };
        let render = self.prepare_render(&target, &info, Arc::clone(&pts), probe, job)?;
        let store = BoxStore::new(pts.len(), inner);
        let (analysis, rendered) = std::thread::scope(|scope| {
            let render_thread = scope.spawn(|| {
                let rendered = transcode(render, &store, job);
                if rendered.is_err() {
                    inner.cancel();
                }
                rendered
            });
            let mut boxes = Vec::new();
            let analysis =
                self.run_analysis(&analyze, prepared, job, None, &mut |frame, builder| {
                    builder.boxes_at(req.regions, &mut boxes);
                    store.push(frame, &boxes);
                });
            store.close();
            let rendered = match render_thread.join() {
                Ok(rendered) => rendered,
                Err(panic) => std::panic::resume_unwind(panic),
            };
            (analysis, rendered)
        });
        let (analysis, rendered) = match (analysis, rendered) {
            (Ok(analysis), Ok(rendered)) => (analysis, rendered),
            (analysis, rendered) => {
                if let Ok(rendered) = &rendered {
                    // The analysis failed after the last frame (saving the
                    // project): no output without its project.
                    let _ = std::fs::remove_file(&rendered.output);
                }
                return Err(first_error(analysis.err(), rendered.err()));
            }
        };
        if !analysis
            .project
            .media
            .pts_sha256
            .eq_ignore_ascii_case(&cache::pts_sha256(&pts))
        {
            let _ = std::fs::remove_file(&rendered.output);
            return Err(AppError::new(
                ErrorKind::UnsupportedMedia,
                "The video's frames do not match its timestamps.",
            )
            .with_detail("the decoded timestamps differ from the container's"));
        }
        Ok(RunReport {
            analysis: analysis.report,
            render: rendered,
            streamed: true,
            elapsed_ms: elapsed_ms(job),
        })
    }
}

/// The error to report when the analysis or the render failed: a real
/// failure before a cancellation it caused on the other side.
fn first_error(analysis: Option<AppError>, render: Option<AppError>) -> AppError {
    let real = |e: &AppError| e.kind != ErrorKind::Canceled;
    match (analysis, render) {
        (Some(a), _) if real(&a) => a,
        (_, Some(r)) if real(&r) => r,
        (Some(e), _) | (None, Some(e)) => e,
        (None, None) => canceled(),
    }
}

/// The job's wall-clock time in milliseconds.
fn elapsed_ms(job: &Job<'_>) -> u64 {
    u64::try_from(job.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use blurrd_core::{AppError, BlurBox, ErrorKind, RectI, Region};
    use blurrd_video::{BoxSource, VideoError};

    use super::{BoxStore, first_error};
    use crate::CancellationToken;

    fn boxed(x: i32) -> BlurBox {
        BlurBox {
            rect: RectI::new(x, 0, x + 10, 10).unwrap(),
            region: Region::Manual,
            seed: 1,
            grid_origin: (x, 0),
            block_scale: 1.0,
        }
    }

    #[test]
    fn the_store_hands_out_final_frames_and_waits_for_the_rest() {
        let cancel = CancellationToken::new();
        let store = BoxStore::new(3, &cancel);
        let mut out = vec![boxed(99)];
        std::thread::scope(|scope| {
            let render = scope.spawn(|| {
                let mut got = Vec::new();
                for frame in 0..3 {
                    let mut out = Vec::new();
                    store.boxes_into(frame, &mut out).unwrap();
                    got.push(out);
                }
                got
            });
            store.push(0, &[boxed(1)]);
            store.push(1, &[]);
            store.push(2, &[boxed(2), boxed(3)]);
            let got = render.join().unwrap();
            assert_eq!(got, [vec![boxed(1)], vec![], vec![boxed(2), boxed(3)]]);
        });
        // Taken frames are gone; frames after the end never come.
        store.boxes_into(0, &mut out).unwrap();
        assert_eq!(out, []);
        store.close();
        assert!(matches!(
            store.boxes_into(3, &mut out),
            Err(VideoError::Canceled)
        ));
        let cancel = CancellationToken::new();
        let store = BoxStore::new(3, &cancel);
        cancel.cancel();
        assert!(matches!(
            store.boxes_into(0, &mut out),
            Err(VideoError::Canceled)
        ));
    }

    #[test]
    fn a_real_failure_wins_over_the_cancellation_it_caused() {
        let canceled = || AppError::new(ErrorKind::Canceled, "c");
        let io = || AppError::new(ErrorKind::Io, "i");
        let encode = || AppError::new(ErrorKind::Encode, "e");
        assert_eq!(
            first_error(Some(canceled()), Some(encode())).kind,
            ErrorKind::Encode
        );
        assert_eq!(
            first_error(Some(io()), Some(canceled())).kind,
            ErrorKind::Io
        );
        assert_eq!(first_error(Some(io()), Some(encode())).kind, ErrorKind::Io);
        assert_eq!(
            first_error(Some(canceled()), None).kind,
            ErrorKind::Canceled
        );
    }
}
