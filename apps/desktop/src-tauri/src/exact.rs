//! Pixels the backend renders for the preview (architecture plan 9.2):
//! Exact patches, leak crops and thumbnails.
//!
//! An Exact patch is a part of a decoded frame blurred by the export's
//! own blur: the boxes of the frame are padded as the render pads them,
//! overlapping padded boxes are merged into one rectangle, and each
//! rectangle becomes a JPEG the preview draws over its WebGL blur. The
//! frame comes from a [`FrameSeeker`] that stays open on the last project
//! and keeps the frames around the last request decoded, so stepping
//! through frames is answered from memory ("warm"). The patches of the
//! last frame are kept until another frame or revision is asked for.

use std::sync::{Mutex, MutexGuard};

use blurrd_blur::{BlurBackend, CpuBlur, PixelParams, PreparedBox, prepare_boxes};
use blurrd_core::{
    AppError, BlurBox, BlurParams, ErrorKind, ExactFrame, ExactPatchInfo, Project, RectI, Region,
    Strength,
};
use blurrd_video::{FrameSeeker, RgbConverter, RgbImage, SeekerOptions, VideoColour, YuvRoi};
use jpeg_encoder::{ColorType, Encoder};

use crate::projects::{OpenProject, lock};

/// JPEG quality of patches and crops.
const JPEG_QUALITY: u8 = 90;
/// Margin around a leak's box in its crop, in source pixels at 1080p.
const CROP_MARGIN: f32 = 6.0;

/// One rendered patch.
#[derive(Debug, Clone)]
struct Patch {
    roi: String,
    rect: RectI,
    jpeg: Vec<u8>,
}

/// The seeker of the last project and the patches of its last frame.
struct Open {
    project: String,
    seeker: FrameSeeker,
    last: Option<(u32, u64, Vec<Patch>)>,
}

/// Renders Exact patches and leak crops.
pub(crate) struct ExactRenderer {
    open: Mutex<Option<Open>>,
}

impl std::fmt::Debug for ExactRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExactRenderer").finish_non_exhaustive()
    }
}

impl ExactRenderer {
    /// A renderer with no project open.
    pub(crate) fn new() -> Self {
        Self {
            open: Mutex::new(None),
        }
    }

    /// The patches of `frame` (clamped to the video) of `project`, which
    /// must be analysed.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidProject`] before the analysis, decoding errors.
    pub(crate) fn frame(&self, project: &OpenProject, frame: u32) -> Result<ExactFrame, AppError> {
        let analysed = project.analysed()?;
        let frame = frame.min(analysed.media.frame_count.saturating_sub(1));
        let patches = self.patches(project, &analysed, frame)?;
        Ok(ExactFrame {
            frame,
            revision: analysed.revision,
            patches: patches
                .iter()
                .map(|patch| ExactPatchInfo {
                    roi: patch.roi.clone(),
                    rect: patch.rect,
                })
                .collect(),
        })
    }

    /// The JPEG of patch `roi` of `frame`.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::NotFound`] for an unknown patch, otherwise as
    /// [`ExactRenderer::frame`].
    pub(crate) fn patch(
        &self,
        project: &OpenProject,
        frame: u32,
        roi: &str,
    ) -> Result<Vec<u8>, AppError> {
        let analysed = project.analysed()?;
        self.patches(project, &analysed, frame)?
            .into_iter()
            .find(|patch| patch.roi == roi)
            .map(|patch| patch.jpeg)
            .ok_or_else(|| {
                AppError::new(ErrorKind::NotFound, "The patch does not exist.")
                    .with_detail(format!("frame {frame}, {roi}"))
            })
    }

    /// The crop around leak `index` of the project's report: blurred
    /// strongly unless `clear` asks for the readable pixels.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::NotFound`] without such a leak, decoding errors.
    pub(crate) fn leak_crop(
        &self,
        project: &OpenProject,
        index: usize,
        clear: bool,
    ) -> Result<Vec<u8>, AppError> {
        let analysed = project.analysed()?;
        let leak = project
            .verify_report()?
            .and_then(|report| report.leaks.get(index).cloned())
            .ok_or_else(|| {
                AppError::new(ErrorKind::NotFound, "The leak does not exist.")
                    .with_detail(format!("leak {index}"))
            })?;
        let margin = (CROP_MARGIN * analysed.geometry.sy).ceil() as i32;
        let info = project.info();
        let width = i32::try_from(info.width).unwrap_or(i32::MAX);
        let height = i32::try_from(info.height).unwrap_or(i32::MAX);
        let crop = RectI::new(
            (leak.rect.x0 - margin).max(0),
            (leak.rect.y0 - margin).max(0),
            (leak.rect.x1 + margin).min(width),
            (leak.rect.y1 + margin).min(height),
        )
        .map_err(|e| internal(e.to_string()))?;
        let boxes = if clear {
            Vec::new()
        } else {
            vec![BlurBox {
                rect: leak.rect,
                region: leak.region.unwrap_or(Region::Manual),
                seed: 0,
                grid_origin: (leak.rect.x0, leak.rect.y0),
                block_scale: 1.0,
            }]
        };
        let blur = BlurParams {
            strength: Strength::High,
            ..analysed.blur
        };
        let mut guard = self.seeker(project)?;
        let open = guard.as_mut().ok_or_else(|| internal("no seeker"))?;
        let decoded = open.seeker.frame(leak.frame_range.f0)?;
        render(project, &analysed, &blur, &decoded, crop, &boxes).map(|(_, jpeg)| jpeg)
    }

    /// The patches of `frame`, from the last request when it is the same.
    fn patches(
        &self,
        project: &OpenProject,
        analysed: &Project,
        frame: u32,
    ) -> Result<Vec<Patch>, AppError> {
        let mut guard = self.seeker(project)?;
        let open = guard.as_mut().ok_or_else(|| internal("no seeker"))?;
        if let Some((last_frame, revision, patches)) = &open.last
            && *last_frame == frame
            && *revision == analysed.revision
        {
            return Ok(patches.clone());
        }
        let mut boxes = Vec::new();
        analysed
            .plan()
            .boxes_at(frame, &analysed.detector.regions, &mut boxes);
        let patches = if boxes.is_empty() {
            Vec::new()
        } else {
            let decoded = open.seeker.frame(frame)?;
            let params = pixel_params(project, analysed, &analysed.blur)?;
            let mut prepared = Vec::new();
            let info = project.info();
            prepare_boxes(&boxes, info.width, info.height, &params, &mut prepared);
            merged_rects(prepared.iter().map(PreparedBox::padded).collect())
                .into_iter()
                .enumerate()
                .map(|(index, rect)| {
                    let inside: Vec<BlurBox> = boxes
                        .iter()
                        .zip(&prepared)
                        .filter(|(_, p)| p.padded().intersection(&rect).is_some())
                        .map(|(b, _)| *b)
                        .collect();
                    render(project, analysed, &analysed.blur, &decoded, rect, &inside).map(
                        |(rect, jpeg)| Patch {
                            roi: format!("p{index}"),
                            rect,
                            jpeg,
                        },
                    )
                })
                .collect::<Result<Vec<_>, _>>()?
        };
        open.last = Some((frame, analysed.revision, patches.clone()));
        Ok(patches)
    }

    /// The seeker of `project`, opened when another project (or none) was
    /// open.
    fn seeker(&self, project: &OpenProject) -> Result<MutexGuard<'_, Option<Open>>, AppError> {
        let mut guard = lock(&self.open);
        if guard
            .as_ref()
            .is_none_or(|open| open.project != project.id())
        {
            // Close the previous decoder before opening the next one.
            *guard = None;
            let seeker =
                FrameSeeker::open(project.source(), project.pts()?, SeekerOptions::default())?;
            *guard = Some(Open {
                project: project.id().to_owned(),
                seeker,
                last: None,
            });
        }
        Ok(guard)
    }
}

/// The blur of `project` at its HUD scale and colour.
fn pixel_params(
    project: &OpenProject,
    analysed: &Project,
    blur: &BlurParams,
) -> Result<PixelParams, AppError> {
    let info = project.info();
    let colour = VideoColour::from_info(&info.color, info.height);
    PixelParams::new(blur, analysed.geometry.sy, colour.yuv_colour()).map_err(|e| {
        AppError::new(
            ErrorKind::InvalidProject,
            "The project's blur settings are invalid.",
        )
        .with_detail(e.to_string())
    })
}

/// Copies `rect` out of `decoded`, blurs `boxes` into it with `blur` and
/// encodes it; answers the rectangle actually copied (aligned to even
/// coordinates) and the JPEG.
fn render(
    project: &OpenProject,
    analysed: &Project,
    blur: &BlurParams,
    decoded: &blurrd_video::VideoFrame,
    rect: RectI,
    boxes: &[BlurBox],
) -> Result<(RectI, Vec<u8>), AppError> {
    let info = project.info();
    let mut roi = YuvRoi::new();
    decoded.roi_yuv(rect, &mut roi)?;
    let copied = roi.rect().ok_or_else(|| internal("empty patch"))?;
    if !boxes.is_empty() {
        let params = pixel_params(project, analysed, blur)?;
        let mut prepared = Vec::new();
        prepare_boxes(boxes, info.width, info.height, &params, &mut prepared);
        prepared.retain(|p| inside(p.padded(), copied));
        let mut view = roi.as_frame_mut().ok_or_else(|| internal("empty patch"))?;
        CpuBlur
            .blur(&mut view, (copied.x0, copied.y0), &prepared, &params)
            .map_err(|e| internal(e.to_string()))?;
    }
    let converter = RgbConverter::new(VideoColour::from_info(&info.color, info.height));
    let mut rgb = RgbImage::default();
    converter.convert(&roi, &mut rgb)?;
    Ok((copied, jpeg(&rgb)?))
}

/// Whether `inner` lies completely inside `outer`.
fn inside(inner: RectI, outer: RectI) -> bool {
    inner.x0 >= outer.x0 && inner.y0 >= outer.y0 && inner.x1 <= outer.x1 && inner.y1 <= outer.y1
}

/// `rects` merged into disjoint rectangles: overlapping ones become their
/// bounding rectangle until no two overlap.
fn merged_rects(mut rects: Vec<RectI>) -> Vec<RectI> {
    let mut changed = true;
    while changed {
        changed = false;
        'outer: for i in 0..rects.len() {
            for j in i + 1..rects.len() {
                if rects[i].intersection(&rects[j]).is_some() {
                    let (a, b) = (rects[i], rects.swap_remove(j));
                    rects[i] = bounding(a, b);
                    changed = true;
                    break 'outer;
                }
            }
        }
    }
    rects.sort_unstable_by_key(|rect| (rect.y0, rect.x0));
    rects
}

fn bounding(a: RectI, b: RectI) -> RectI {
    RectI {
        x0: a.x0.min(b.x0),
        y0: a.y0.min(b.y0),
        x1: a.x1.max(b.x1),
        y1: a.y1.max(b.y1),
    }
}

/// The JPEG of `rgb`.
fn jpeg(rgb: &RgbImage) -> Result<Vec<u8>, AppError> {
    let (Ok(width), Ok(height)) = (u16::try_from(rgb.width), u16::try_from(rgb.height)) else {
        return Err(internal("the image is too large for a JPEG"));
    };
    let mut out = Vec::new();
    Encoder::new(&mut out, JPEG_QUALITY)
        .encode(&rgb.data, width, height, ColorType::Rgb)
        .map_err(|e| internal(e.to_string()))?;
    Ok(out)
}

fn internal(detail: impl Into<String>) -> AppError {
    AppError::new(ErrorKind::Internal, "The preview image could not be made.").with_detail(detail)
}

#[cfg(test)]
mod tests {
    use blurrd_core::RectI;

    use super::merged_rects;

    fn rect(x0: i32, y0: i32, x1: i32, y1: i32) -> RectI {
        RectI::new(x0, y0, x1, y1).unwrap()
    }

    #[test]
    fn overlapping_boxes_merge_until_the_patches_are_disjoint() {
        // a and c overlap only through b's bounding rectangle with a.
        let a = rect(0, 0, 10, 10);
        let b = rect(5, 5, 30, 12);
        let c = rect(25, 11, 40, 20);
        let far = rect(100, 100, 110, 110);
        assert_eq!(
            merged_rects(vec![far, c, a, b]),
            vec![rect(0, 0, 40, 20), far]
        );
        assert_eq!(merged_rects(vec![a, far]), vec![a, far]);
    }

    /// Exact patches on a real recording: analysed once, then the frames
    /// with boxes are stepped through as the preview does. Prints the
    /// latency of a frame's patches (decode, blur, JPEG); warm requests,
    /// whose frame the seeker holds, must take less than 50 ms.
    #[test]
    #[ignore = "needs BLURRD_TEST_MEDIA; run in release"]
    fn warm_exact_patches_take_less_than_50_ms() {
        use std::sync::Arc;
        use std::time::{Duration, Instant};

        use blurrd_core::{JobId, RegionSet};
        use blurrd_engine::{AnalyzeRequest, CancellationToken, ProgressSink};

        use super::ExactRenderer;
        use crate::tasks::tests::{open, test_clip};

        struct Quiet;
        impl ProgressSink for Quiet {
            fn emit(&self, _: blurrd_core::JobEvent) {}
        }

        let Some(clip) = test_clip() else {
            eprintln!("skipped: BLURRD_TEST_MEDIA is not set");
            return;
        };
        let (engine, project, work) = open(&clip, "exact");
        let mut req = AnalyzeRequest::new(JobId(uuid::Uuid::now_v7()), clip.clone());
        req.regions = RegionSet::all();
        req.verify = false;
        req.previews = false;
        engine
            .analyze(&req, &Quiet, &CancellationToken::new())
            .unwrap();
        let analysed = project.analysed().unwrap();
        let table = analysed.plan().frame_table(&analysed.detector.regions);
        let with_boxes: Vec<u32> = (0..table.frame_count())
            .filter(|&f| !table.boxes(f).is_empty())
            .collect();
        assert!(with_boxes.len() > 100, "{}", with_boxes.len());

        let exact = Arc::new(ExactRenderer::new());
        let start = with_boxes[with_boxes.len() / 3];
        let cold = Instant::now();
        let first = exact.frame(&project, start).unwrap();
        let cold = cold.elapsed();
        assert_eq!(first.frame, start);
        assert_ne!(first.patches, vec![]);
        // The JPEG requests that follow are answered from the frame's patches.
        let jpeg = Instant::now();
        for patch in &first.patches {
            let bytes = exact.patch(&project, start, &patch.roi).unwrap();
            assert_eq!(&bytes[..2], &[0xFF, 0xD8]);
        }
        let jpeg = jpeg.elapsed();

        // Step forward frame by frame, as the transport does; the seeker
        // decodes ahead between requests.
        let mut warm: Vec<Duration> = Vec::new();
        let mut patches = 0;
        for frame in start + 1..start + 120 {
            std::thread::sleep(Duration::from_millis(16));
            let t = Instant::now();
            let answer = exact.frame(&project, frame).unwrap();
            for patch in &answer.patches {
                exact.patch(&project, frame, &patch.roi).unwrap();
            }
            warm.push(t.elapsed());
            patches += answer.patches.len();
        }
        warm.sort_unstable();
        let median = warm[warm.len() / 2];
        let p95 = warm[warm.len() * 95 / 100];
        let max = warm[warm.len() - 1];
        eprintln!(
            "exact: cold {:.1} ms, jpeg reads {:.2} ms; warm over {} frames \
             ({patches} patches): median {:.1} ms, p95 {:.1} ms, max {:.1} ms",
            cold.as_secs_f64() * 1e3,
            jpeg.as_secs_f64() * 1e3,
            warm.len(),
            median.as_secs_f64() * 1e3,
            p95.as_secs_f64() * 1e3,
            max.as_secs_f64() * 1e3,
        );
        assert!(max < Duration::from_millis(50), "{max:?}");
        drop(exact);
        std::fs::remove_dir_all(&work).unwrap();
    }
}
