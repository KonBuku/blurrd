//! Calibration of a video before its analysis (plan 7.3): where the HUD
//! is, the kill feed's bar colours and, for HDR video, the HUD's paper
//! white.
//!
//! The frame size alone does not say where the HUD is (a 16:9 recording
//! can hold a 4:3 game stretched to fill it, a 4:3 game can sit between
//! black bars), and the crops of every frame depend on it, so the
//! geometry is decided before the pipeline starts. A few keyframes spread
//! over the video are decoded on their own (a seek and one decoded frame
//! each, on a few workers) and read by the detectors'
//! [`Calibrator`]. HDR video is sampled a second time to measure the
//! paper white in the calibrated HUD crops.
//!
//! The analysis covers a calibration it cannot trust with Safe boxes and a
//! warning: low confidence, a layout that changes within the video, or a
//! paper white that cannot be measured.

use std::path::Path;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

use blurrd_core::{AppError, HudGeometry, HudLayout, MediaInfo, RectI};
use blurrd_detect::calibration::{CalibratedSegment, Calibration, Calibrator, MIN_CONFIDENCE};
use blurrd_detect::chat::LEFT_PANEL_ROI;
use blurrd_detect::killfeed::{BarHues, FEED_ROI};
use blurrd_detect::screens::Anchor;
use blurrd_video::{
    DecodeOptions, Decoder, HwDecode, PaperWhite, RgbConverter, RgbImage, VideoColour, VideoError,
    VideoFrame, YuvRoi,
};

use crate::hud::{HudMap, rgb_view};
use crate::job::{Job, canceled};

/// Seconds of video per sampled keyframe.
const SAMPLE_SPACING_S: f64 = 2.0;
/// Keyframes sampled at least (short clips) ...
const MIN_SAMPLES: usize = 8;
/// ... and at most (long videos; the samples cost about 10 ms each).
const MAX_SAMPLES: usize = 32;
/// Workers decoding samples at the same time.
const SAMPLE_WORKERS: usize = 4;

/// What the calibration decided for one video.
#[derive(Debug, Clone)]
pub(crate) struct VideoCalibration {
    /// The HUD geometry the analysis maps its crops with.
    pub(crate) geometry: HudGeometry,
    /// The kill feed's bar colours, when they differ from the defaults.
    pub(crate) hues: Option<BarHues>,
    /// Why the geometry cannot be trusted (the analysis uses Safe boxes
    /// and warns), or `None`.
    pub(crate) doubt: Option<String>,
    /// The paper white of an HDR video in nits, when it was measured
    /// confidently.
    pub(crate) paper_white: Option<f32>,
}

/// Calibrates the video `input` described by `info` with colours
/// `colour`.
///
/// # Errors
///
/// [`blurrd_core::ErrorKind::Canceled`] when the job is canceled. A
/// sample that fails to decode is skipped: the analysis reports the
/// decoder's errors.
pub(crate) fn calibrate(
    input: &Path,
    info: &MediaInfo,
    colour: VideoColour,
    job: &Job<'_>,
) -> Result<VideoCalibration, AppError> {
    let started = std::time::Instant::now();
    let times = sample_times(info.duration_s);
    let mut read = 0u32;
    let reference = RgbConverter::new(colour);
    let fps = f64::from(info.fps.num) / f64::from(info.fps.den.max(1));
    let mut calibrator = Calibrator::new(info.width, info.height, HudLayout::Auto);
    let mut first_pts: Option<i64> = None;
    let mut last_pts: Option<i64> = None;
    sample(
        input,
        &times,
        job,
        |frame| {
            let mut roi = YuvRoi::new();
            frame.roi_yuv(whole_frame(frame), &mut roi)?;
            let mut image = RgbImage::default();
            reference.convert(&roi, &mut image)?;
            Ok(image)
        },
        |pts, seconds_per_tick, image| {
            // Keyframes shared by two sample times come twice.
            if last_pts.is_some_and(|last| pts <= last) {
                return;
            }
            last_pts = Some(pts);
            read += 1;
            let first = *first_pts.get_or_insert(pts);
            let index = ((pts - first) as f64 * seconds_per_tick * fps).round() as u32;
            if let Ok(view) = rgb_view(&image) {
                calibrator.push(index, &view);
            }
        },
    )?;
    let calibration = calibrator.finish();
    let frames = super::expected_frames(info);
    let (segment, mut doubt) = choose(&calibration, info.width, info.height, frames);
    let geometry = segment.geometry.clone();
    if doubt.is_none() && geometry.confidence < MIN_CONFIDENCE {
        doubt = Some(format!(
            "the HUD position was not confirmed (confidence {:.2}, layout {:?})",
            geometry.confidence, geometry.layout
        ));
    }
    let paper_white = if reference.is_tone_mapped() {
        measure_paper_white(input, info, &geometry, &reference, &times, job)?
    } else {
        None
    };
    tracing::info!(
        layout = ?geometry.layout,
        confidence = geometry.confidence,
        hues = segment.hues.is_some(),
        paper_white,
        samples = read,
        ms = started.elapsed().as_millis(),
        "calibrated"
    );
    Ok(VideoCalibration {
        geometry,
        hues: segment.hues,
        doubt,
        paper_white,
    })
}

/// The times sampled in a video of `duration_s` seconds: one every
/// [`SAMPLE_SPACING_S`], at least [`MIN_SAMPLES`] and at most
/// [`MAX_SAMPLES`], spread evenly from the start.
fn sample_times(duration_s: f64) -> Vec<f64> {
    let duration = if duration_s.is_finite() {
        duration_s.max(0.0)
    } else {
        0.0
    };
    let n = ((duration / SAMPLE_SPACING_S).ceil() as usize).clamp(MIN_SAMPLES, MAX_SAMPLES);
    (0..n).map(|i| duration * i as f64 / n as f64).collect()
}

/// The segment whose geometry the analysis uses and, when the layout
/// changes within the video, why it cannot be trusted. The analysis maps
/// every frame with one geometry, so a video with several layouts gets
/// that of its longest segment (a segment ends where the next one starts,
/// the last one at frame `frames`).
fn choose(
    calibration: &Calibration,
    width: u32,
    height: u32,
    frames: u32,
) -> (CalibratedSegment, Option<String>) {
    let segments = &calibration.segments;
    let Some(first) = segments.first() else {
        return (
            CalibratedSegment {
                first_frame: 0,
                geometry: HudGeometry::for_layout(width, height, HudLayout::Auto),
                hues: None,
            },
            None,
        );
    };
    if segments
        .iter()
        .all(|s| s.geometry.layout == first.geometry.layout)
    {
        return (first.clone(), None);
    }
    let span = |k: usize| {
        let end = segments.get(k + 1).map_or(frames, |s| s.first_frame);
        end.saturating_sub(segments[k].first_frame)
    };
    let longest = (0..segments.len()).max_by_key(|&k| span(k)).unwrap_or(0);
    let changes: Vec<String> = segments
        .iter()
        .skip(1)
        .map(|s| format!("{:?} from about frame {}", s.geometry.layout, s.first_frame))
        .collect();
    let doubt = format!(
        "the HUD layout changes within the video ({:?} at the start, {}); every frame is mapped \
         as {:?}",
        first.geometry.layout,
        changes.join(", "),
        segments[longest].geometry.layout
    );
    (segments[longest].clone(), Some(doubt))
}

/// The paper white of an HDR video, measured in the HUD crops of
/// `geometry` (the kill feed and the left panel) of the sampled frames.
fn measure_paper_white(
    input: &Path,
    info: &MediaInfo,
    geometry: &HudGeometry,
    converter: &RgbConverter,
    times: &[f64],
    job: &Job<'_>,
) -> Result<Option<f32>, AppError> {
    let map = HudMap::new(geometry, info.width, info.height);
    let rects: Vec<RectI> = [(FEED_ROI, Anchor::Right), (LEFT_PANEL_ROI, Anchor::Left)]
        .into_iter()
        .filter_map(|(roi, anchor)| map.source_rect(roi, anchor))
        .collect();
    let mut measured = PaperWhite::new();
    let mut last_pts: Option<i64> = None;
    sample(
        input,
        times,
        job,
        |frame| {
            rects
                .iter()
                .map(|&rect| {
                    let mut roi = YuvRoi::new();
                    frame.roi_yuv(rect, &mut roi)?;
                    Ok(roi)
                })
                .collect::<Result<Vec<YuvRoi>, VideoError>>()
        },
        |pts, _, crops| {
            if last_pts.is_some_and(|last| pts <= last) {
                return;
            }
            last_pts = Some(pts);
            for roi in &crops {
                if let Err(error) = converter.measure_paper_white(roi, &mut measured) {
                    tracing::debug!(%error, "paper white sample skipped");
                }
            }
        },
    )?;
    Ok(measured.estimate())
}

/// The whole frame, for [`VideoFrame::roi_yuv`].
fn whole_frame(frame: &VideoFrame) -> RectI {
    RectI {
        x0: 0,
        y0: 0,
        x1: frame.width() as i32,
        y1: frame.height() as i32,
    }
}

/// A sample decoded by a worker: its PTS, the seconds per PTS tick and
/// what `read` made of it; `None` for a sample that could not be read.
type Sampled<T> = Option<(i64, f64, T)>;

/// Decodes the keyframe at or before each of `times` (seconds) on up to
/// [`SAMPLE_WORKERS`] software decoders and hands what `read` makes of
/// each to `take`, in the order of `times`, with its PTS and the seconds
/// per PTS tick. Samples that fail are skipped (logged).
fn sample<T: Send>(
    input: &Path,
    times: &[f64],
    job: &Job<'_>,
    read: impl Fn(&VideoFrame) -> Result<T, VideoError> + Sync,
    mut take: impl FnMut(i64, f64, T),
) -> Result<(), AppError> {
    let workers = SAMPLE_WORKERS.min(times.len()).max(1);
    let read = &read;
    std::thread::scope(|scope| {
        let mut queues: Vec<Receiver<Sampled<T>>> = Vec::with_capacity(workers);
        for worker in 0..workers {
            // One sample in flight per worker bounds the memory to a few
            // decoded frames.
            let (tx, rx) = sync_channel::<Sampled<T>>(1);
            queues.push(rx);
            let mine: Vec<f64> = times
                .iter()
                .copied()
                .skip(worker)
                .step_by(workers)
                .collect();
            scope.spawn(move || decode_samples(input, &mine, read, &tx, job));
        }
        for k in 0..times.len() {
            if job.is_canceled() {
                return Err(canceled());
            }
            // A worker that stopped (open failed) leaves its samples out.
            if let Ok(Some((pts, tick, value))) = queues[k % workers].recv() {
                take(pts, tick, value);
            }
        }
        Ok(())
    })
}

/// One sampling worker: decodes the keyframe at or before each of `times`
/// and sends what `read` makes of it, until the receiver stops listening
/// or the job is canceled.
fn decode_samples<T>(
    input: &Path,
    times: &[f64],
    read: &(impl Fn(&VideoFrame) -> Result<T, VideoError> + Sync),
    tx: &SyncSender<Sampled<T>>,
    job: &Job<'_>,
) {
    // One thread per decoder: a keyframe decodes on its own, and frame
    // threads would decode the frames after it as well.
    let mut options = DecodeOptions::with_hw(HwDecode::Software);
    options.threads = Some(1);
    let mut decoder = match Decoder::open(input, &options) {
        Ok(decoder) => decoder,
        Err(error) => {
            tracing::debug!(%error, "calibration decoder not opened");
            return;
        }
    };
    let tb = decoder.time_base();
    let tick = f64::from(tb.num) / f64::from(tb.den.max(1));
    for &t in times {
        if job.is_canceled() {
            return;
        }
        let sampled = (|| -> Result<Option<(i64, T)>, VideoError> {
            decoder.seek_keyframe((t / tick.max(f64::MIN_POSITIVE)).round() as i64)?;
            let Some(frame) = decoder.next_frame()? else {
                return Ok(None);
            };
            Ok(Some((frame.pts(), read(&frame)?)))
        })();
        let message = match sampled {
            Ok(Some((pts, value))) => Some((pts, tick, value)),
            Ok(None) => None,
            Err(error) => {
                tracing::debug!(%error, seconds = t, "calibration sample skipped");
                None
            }
        };
        if tx.send(message).is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use blurrd_core::{HudGeometry, HudLayout};
    use blurrd_detect::calibration::{CalibratedSegment, Calibration};

    use super::{MAX_SAMPLES, MIN_SAMPLES, choose, sample_times};

    #[test]
    fn samples_are_spread_and_bounded() {
        assert_eq!(sample_times(1.5).len(), MIN_SAMPLES);
        assert_eq!(sample_times(30.0).len(), 15);
        assert_eq!(sample_times(3600.0).len(), MAX_SAMPLES);
        let times = sample_times(60.0);
        assert_eq!(times[0], 0.0);
        assert!(times.windows(2).all(|w| w[0] < w[1]));
        assert!(times.iter().all(|&t| t < 60.0));
        assert_eq!(sample_times(f64::NAN).len(), MIN_SAMPLES);
    }

    fn segment(first_frame: u32, layout: HudLayout) -> CalibratedSegment {
        let mut geometry = HudGeometry::for_layout(1920, 1080, layout);
        geometry.confidence = 1.0;
        CalibratedSegment {
            first_frame,
            geometry,
            hues: None,
        }
    }

    #[test]
    fn one_layout_is_trusted_and_a_change_is_doubted() {
        let same = Calibration {
            segments: vec![segment(0, HudLayout::Wide16x9)],
        };
        let (chosen, doubt) = choose(&same, 1920, 1080, 3000);
        assert_eq!(chosen.geometry.layout, HudLayout::Wide16x9);
        assert_eq!(doubt, None);
        // The pillarboxed part is the longest: it maps every frame.
        let mixed = Calibration {
            segments: vec![
                segment(0, HudLayout::Wide16x9),
                segment(100, HudLayout::Pillarbox { bar_px: 240 }),
                segment(2000, HudLayout::Wide16x9),
            ],
        };
        let (chosen, doubt) = choose(&mixed, 1920, 1080, 3000);
        assert_eq!(chosen.geometry.layout, HudLayout::Pillarbox { bar_px: 240 });
        assert!(doubt.is_some_and(|d| d.contains("changes")));
    }
}
