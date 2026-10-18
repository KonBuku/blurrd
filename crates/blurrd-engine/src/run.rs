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
//! the output of an analysis followed by a render of its project. The
//! render reads the PTS table and opens its decoders and encoders on its
//! own thread, so the analysis starts right away.
//!
//! The verification runs once the analysis has every frame, when the
//! render has written all but the last frames. A streamed output is
//! written under its `*_UNVERIFIED` name; when the verification comes out
//! clean it gets its own name, and when the verification added fix boxes
//! the project is rendered again (rare: a fix means a detector missed a
//! name). That render encodes only the GOPs whose boxes changed and takes
//! every other frame from the streamed output, which the run remembers
//! the boxes of.

use std::collections::BTreeMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use blurrd_core::{
    AppError, BlurBox, BlurParams, Coverage, EncoderInfo, ErrorKind, ExportSettings, FrameBoxTable,
    JobEvent, JobId, JobKind, JobSummary, MediaInfo, Mode, Project, RegionSet, TrackSource,
    VerifyState,
};
use blurrd_video::{BoxSource, DecodePurpose, HwDecode, PtsTable, VideoError};
use serde::Serialize;

use crate::analysis::{AnalyzeReport, AnalyzeRequest, Prepared, expected_frames};
use crate::cache::{self, ProjectDir};
use crate::engine::Engine;
use crate::job::{CancellationToken, Job, canceled};
use crate::render::{RenderReport, RenderRequest, RenderTarget, Reuse, is_verified, transcode};

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
    /// The box geometry (see [`AnalyzeRequest::mode`]).
    pub mode: Mode,
    /// Blur parameters.
    pub blur: BlurParams,
    /// Codec, encoder, quality, audio and container settings.
    pub export: ExportSettings,
    /// Name the output normally even though it is not verified.
    pub accept_unverified: bool,
    /// Whether to verify (pass 0 of the OCR verification when OCR is
    /// installed).
    pub verify: bool,
    /// Which frames the verification reads.
    pub coverage: Coverage,
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
            mode: Mode::Tight,
            blur: BlurParams::default(),
            export: ExportSettings::default(),
            accept_unverified: false,
            verify: true,
            coverage: Coverage::Structure,
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
    /// Whether the streamed output was rendered again because the
    /// verification added fix boxes.
    pub rerendered: bool,
    /// Wall-clock time of the whole run in milliseconds.
    pub elapsed_ms: u64,
}

/// The boxes of the frames the analysis made final, until the render
/// takes them. Frames without boxes are not stored.
struct BoxStore<'a> {
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
    fn new(cancel: &'a CancellationToken) -> Self {
        Self {
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

/// The box store as the render's box source, for a video of `frames`
/// frames (known once the render read the PTS table).
struct StoreSource<'s, 'a> {
    store: &'s BoxStore<'a>,
    frames: u32,
}

impl BoxSource for StoreSource<'_, '_> {
    fn frame_count(&self) -> u32 {
        self.frames
    }

    fn boxes_into(&self, frame: u32, out: &mut Vec<BlurBox>) -> Result<(), VideoError> {
        self.store.boxes_into(frame, out)
    }
}

impl BoxStore<'_> {
    /// The boxes of `frame`, once it is final; fails when the analysis
    /// ended before it or the job was canceled.
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
                // The project's state, whether or not the user accepted
                // an unverified result.
                verify: Some(if report.render.verified {
                    VerifyState::Clean
                } else {
                    match report.analysis.verify.map(|v| v.state) {
                        Some(VerifyState::LeaksRemain) => VerifyState::LeaksRemain,
                        _ => VerifyState::Unverified,
                    }
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
            mode: req.mode,
            blur: req.blur,
            verify: req.verify,
            coverage: req.coverage,
            // Thumbnails and the proxy serve the desktop app's preview; a
            // run goes straight to the output.
            previews: false,
        };
        let prepared = self.prepare_analysis(&analyze, true, job)?;
        let dir = ProjectDir::new(&self.config().cache_dir, &prepared.fingerprint);
        if dir.read_project().is_some_and(|p| !p.overrides.is_empty()) {
            return self.run_two_passes(req, &analyze, prepared, job);
        }

        let probe = self.start_encoder_probe(&req.export);
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
            // The verification ends after the render's last frames; the
            // output gets its own name afterwards if it comes out clean.
            verified: false,
            accept_unverified: req.accept_unverified,
            blur: &req.blur,
            sy: prepared.geometry.sy,
            decode,
        };
        let store = BoxStore::new(inner);
        // The boxes the streamed output got, frame by frame: a render
        // after verification fixes encodes only where they changed.
        let mut streamed = StreamedBoxes::default();
        let (analysis, rendered) = std::thread::scope(|scope| {
            // The render reads the PTS table and opens its decoders and
            // encoders while the analysis already runs.
            let render_thread = scope.spawn(|| {
                let rendered = self.render_streamed(&req.input, &target, &info, probe, &store, job);
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
                    streamed.push(&boxes);
                });
            store.close();
            let rendered = match render_thread.join() {
                Ok(rendered) => rendered,
                Err(panic) => std::panic::resume_unwind(panic),
            };
            (analysis, rendered)
        });
        let (analysis, (rendered, pts)) = match (analysis, rendered) {
            (Ok(analysis), Ok(rendered)) => (analysis, rendered),
            (analysis, rendered) => {
                if let Ok((rendered, _)) = &rendered {
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
        let (render, rerendered) =
            self.after_verify(req, &analysis.project, rendered, &streamed, job)?;
        Ok(RunReport {
            analysis: analysis.report,
            render,
            streamed: true,
            rerendered,
            elapsed_ms: elapsed_ms(job),
        })
    }

    /// Analyses first and renders the finished project afterwards (a video
    /// whose cached project has overrides, which need the finished
    /// tracks).
    fn run_two_passes(
        &self,
        req: &RunRequest,
        analyze: &AnalyzeRequest,
        prepared: Prepared,
        job: &Job<'_>,
    ) -> Result<RunReport, AppError> {
        let expected = expected_frames(&prepared.info);
        job.started(JobKind::Run, expected, 2);
        let analysis = self.run_analysis(analyze, prepared, job, Some(expected), &mut |_, _| {})?;
        let mut render = RenderRequest::new(
            req.job,
            req.input.clone(),
            analysis.project,
            req.output.clone(),
        );
        render.export = req.export.clone();
        render.accept_unverified = req.accept_unverified;
        job.next_pass();
        let rendered = self.render_project(&render, job, None, None)?;
        Ok(RunReport {
            analysis: analysis.report,
            render: rendered,
            streamed: false,
            rerendered: false,
            elapsed_ms: elapsed_ms(job),
        })
    }

    /// Settles a streamed output with the verified `project`: rendered
    /// again when the verification added fix boxes (only the GOPs whose
    /// boxes differ from `streamed`, the boxes the streamed output got,
    /// are encoded again), renamed to the requested name when it came out
    /// clean, left `*_UNVERIFIED` otherwise. Returns the final render and
    /// whether it was rendered again.
    fn after_verify(
        &self,
        req: &RunRequest,
        project: &Project,
        rendered: RenderReport,
        streamed: &StreamedBoxes,
        job: &Job<'_>,
    ) -> Result<(RenderReport, bool), AppError> {
        let fixed = project
            .tracks
            .iter()
            .any(|t| t.source == TrackSource::OcrFix);
        if fixed {
            job.next_pass();
            let rerendered = self.render_fixed(req, project, &rendered.output, streamed, job);
            // The streamed output goes unless the new render replaced it
            // under the same name.
            if rerendered
                .as_ref()
                .ok()
                .is_none_or(|report| report.output != rendered.output)
            {
                let _ = std::fs::remove_file(&rendered.output);
            }
            return Ok((rerendered?, true));
        }
        let verified = is_verified(project);
        let mut output = rendered.output.clone();
        if verified && output != req.output {
            std::fs::rename(&output, &req.output).map_err(|e| {
                cache::io_error(&e, "The verified output could not be renamed.", &req.output)
            })?;
            output.clone_from(&req.output);
        }
        Ok((
            RenderReport {
                output,
                verified,
                accepted_unverified: !verified && rendered.accepted_unverified,
                ..rendered
            },
            false,
        ))
    }

    /// Renders `project` again after the verification added fix boxes,
    /// taking the frames whose boxes did not change from the streamed
    /// output at `previous` (whose frames got `streamed`). Renders the
    /// whole video when that fails for another reason than a
    /// cancellation.
    fn render_fixed(
        &self,
        req: &RunRequest,
        project: &Project,
        previous: &Path,
        streamed: &StreamedBoxes,
        job: &Job<'_>,
    ) -> Result<RenderReport, AppError> {
        let mut render = RenderRequest::new(
            req.job,
            req.input.clone(),
            project.clone(),
            req.output.clone(),
        );
        render.export = req.export.clone();
        render.accept_unverified = req.accept_unverified;
        let table = project.plan().frame_table(&project.detector.regions);
        let changed = streamed.changed_frames(&table);
        let reuse = Reuse {
            previous,
            changed: &changed,
        };
        match self.render_project(&render, job, None, Some(reuse)) {
            Err(error) if error.kind != ErrorKind::Canceled => {
                tracing::warn!(%error, "rendering the whole video again");
                self.render_project(&render, job, None, None)
            }
            result => result,
        }
    }

    /// The render side of a streaming run. It reads the PTS table from
    /// the container, as the analysis has not decoded the video yet (the
    /// render checks every decoded frame against it, and the run compares
    /// it with the analysis' own table), starts the job with its frame
    /// count (the analysis reports no progress in a run), opens the render
    /// and blurs each frame as soon as its boxes are final.
    fn render_streamed(
        &self,
        input: &Path,
        target: &RenderTarget<'_>,
        info: &MediaInfo,
        probe: Option<std::thread::JoinHandle<Vec<EncoderInfo>>>,
        store: &BoxStore<'_>,
        job: &Job<'_>,
    ) -> Result<(RenderReport, Arc<PtsTable>), AppError> {
        let pts = Arc::new(blurrd_video::pts_table(input)?);
        job.started(JobKind::Run, pts.len(), 1);
        let render = self.prepare_render(target, info, Arc::clone(&pts), probe, job)?;
        let source = StoreSource {
            store,
            frames: pts.len(),
        };
        Ok((transcode(render, &source, job)?, pts))
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

/// The boxes each frame of a streamed output got, in order. Consecutive
/// frames with the same boxes share one entry, so a long run keeps about
/// one span per frame and the boxes of the frames where they changed.
#[derive(Debug, Default)]
struct StreamedBoxes {
    /// The boxes of every distinct entry, back to back.
    boxes: Vec<BlurBox>,
    /// Per frame: the range of `boxes` it got.
    frames: Vec<Range<usize>>,
}

impl StreamedBoxes {
    /// Adds the boxes of the next frame.
    fn push(&mut self, boxes: &[BlurBox]) {
        let same = self
            .frames
            .last()
            .is_some_and(|last| self.slice(last) == boxes);
        let span = match self.frames.last() {
            Some(last) if same => last.clone(),
            _ => {
                let start = self.boxes.len();
                self.boxes.extend_from_slice(boxes);
                start..self.boxes.len()
            }
        };
        self.frames.push(span);
    }

    /// The boxes of `frame`, `None` past the last frame.
    fn at(&self, frame: u32) -> Option<&[BlurBox]> {
        let span = self.frames.get(usize::try_from(frame).ok()?)?;
        Some(self.slice(span))
    }

    /// The boxes of an entry.
    fn slice(&self, span: &Range<usize>) -> &[BlurBox] {
        self.boxes.get(span.clone()).unwrap_or_default()
    }

    /// The frame ranges whose boxes in `table` differ from the streamed
    /// ones (frames past the streamed end count as changed).
    fn changed_frames(&self, table: &FrameBoxTable) -> Vec<Range<u32>> {
        let mut ranges: Vec<Range<u32>> = Vec::new();
        for frame in 0..table.frame_count() {
            if self.at(frame) == Some(table.boxes(frame)) {
                continue;
            }
            match ranges.last_mut() {
                Some(last) if last.end == frame => last.end = frame + 1,
                _ => ranges.push(frame..frame + 1),
            }
        }
        ranges
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

    use super::{BoxStore, StoreSource, StreamedBoxes, first_error};
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
        let store = BoxStore::new(&cancel);
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
        // The render reads it as a box source of the video's length.
        let source = StoreSource {
            store: &store,
            frames: 3,
        };
        assert_eq!(source.frame_count(), 3);
        // Taken frames are gone; frames after the end never come.
        source.boxes_into(0, &mut out).unwrap();
        assert_eq!(out, []);
        store.close();
        assert!(matches!(
            store.boxes_into(3, &mut out),
            Err(VideoError::Canceled)
        ));
        let cancel = CancellationToken::new();
        let store = BoxStore::new(&cancel);
        cancel.cancel();
        assert!(matches!(
            store.boxes_into(0, &mut out),
            Err(VideoError::Canceled)
        ));
    }

    #[test]
    fn streamed_boxes_keep_each_frame_and_share_repeats() {
        let mut streamed = StreamedBoxes::default();
        streamed.push(&[boxed(1)]);
        streamed.push(&[boxed(1)]);
        streamed.push(&[]);
        streamed.push(&[boxed(2), boxed(3)]);
        streamed.push(&[boxed(2), boxed(3)]);
        assert_eq!(streamed.at(0), Some([boxed(1)].as_slice()));
        assert_eq!(streamed.at(1), Some([boxed(1)].as_slice()));
        assert_eq!(streamed.at(2), Some([].as_slice()));
        assert_eq!(streamed.at(4), Some([boxed(2), boxed(3)].as_slice()));
        assert_eq!(streamed.at(5), None);
        // Repeated frames store their boxes once.
        assert_eq!(streamed.boxes.len(), 3);
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
