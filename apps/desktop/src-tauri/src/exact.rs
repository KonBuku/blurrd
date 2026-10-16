//! Pixels the backend renders for the preview (architecture plan 9.2):
//! Exact patches, leak crops and thumbnails.
//!
//! An Exact patch is a part of a decoded frame blurred by the export's
//! own blur: the boxes of the frame are padded as the render pads them,
//! overlapping padded boxes are merged into one rectangle, and each
//! rectangle becomes a JPEG the preview draws over its WebGL blur. The
//! request names the blur and the enabled regions of the export
//! ([`ExactSettings`]), which the inspector may have changed since the
//! analysis, so the patches show the pixels the export writes. The frame
//! comes from a [`FrameSeeker`] that stays open on the last project and
//! keeps the frames around the last request decoded, so stepping through
//! frames is answered from memory ("warm"). The patches of the last frame
//! are kept until another frame, revision or setting is asked for.

use std::sync::{Mutex, MutexGuard};

use blurrd_blur::{BlurBackend, CpuBlur, PixelParams, PreparedBox, prepare_boxes};
use blurrd_core::{
    AppError, BlurBox, BlurParams, ErrorKind, ExactFrame, ExactPatchInfo, ExactSettings, Project,
    RectI, Region, Strength,
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

/// The patches of one frame and what they were made with.
struct LastFrame {
    frame: u32,
    revision: u64,
    settings: ExactSettings,
    patches: Vec<Patch>,
}

/// The seeker of the last project and the patches of its last frame.
struct Open {
    project: String,
    seeker: FrameSeeker,
    last: Option<LastFrame>,
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
    /// must be analysed, blurred with `settings` (the project's own blur
    /// and regions without them).
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidProject`] before the analysis, decoding errors.
    pub(crate) fn frame(
        &self,
        project: &OpenProject,
        frame: u32,
        settings: Option<&ExactSettings>,
    ) -> Result<ExactFrame, AppError> {
        let analysed = project.analysed()?;
        let frame = frame.min(analysed.media.frame_count.saturating_sub(1));
        let settings = resolved(&analysed, settings);
        let patches = self.patches(project, &analysed, frame, &settings)?;
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
        settings: Option<&ExactSettings>,
    ) -> Result<Vec<u8>, AppError> {
        let analysed = project.analysed()?;
        let settings = resolved(&analysed, settings);
        self.patches(project, &analysed, frame, &settings)?
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
        let (_, roi) = blurred(project, &analysed, &blur, &decoded, crop, &boxes)?;
        encode(project, &roi)
    }

    /// The patches of `frame`, from the last request when it is the same.
    fn patches(
        &self,
        project: &OpenProject,
        analysed: &Project,
        frame: u32,
        settings: &ExactSettings,
    ) -> Result<Vec<Patch>, AppError> {
        let mut guard = self.seeker(project)?;
        let open = guard.as_mut().ok_or_else(|| internal("no seeker"))?;
        if let Some(last) = &open.last
            && last.frame == frame
            && last.revision == analysed.revision
            && last.settings == *settings
        {
            return Ok(last.patches.clone());
        }
        let patches = blurred_rects(project, analysed, settings, &mut open.seeker, frame)?
            .into_iter()
            .enumerate()
            .map(|(index, (rect, roi))| {
                Ok(Patch {
                    roi: format!("p{index}"),
                    rect,
                    jpeg: encode(project, &roi)?,
                })
            })
            .collect::<Result<Vec<_>, AppError>>()?;
        open.last = Some(LastFrame {
            frame,
            revision: analysed.revision,
            settings: *settings,
            patches: patches.clone(),
        });
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

    /// The blurred pixels of the patches of `frame`, before the JPEG.
    #[cfg(test)]
    fn pixels(
        &self,
        project: &OpenProject,
        frame: u32,
        settings: Option<&ExactSettings>,
    ) -> Result<Vec<(RectI, YuvRoi)>, AppError> {
        let analysed = project.analysed()?;
        let settings = resolved(&analysed, settings);
        let mut guard = self.seeker(project)?;
        let open = guard.as_mut().ok_or_else(|| internal("no seeker"))?;
        blurred_rects(project, &analysed, &settings, &mut open.seeker, frame)
    }
}

/// `settings`, or the blur and regions of `analysed` without them.
fn resolved(analysed: &Project, settings: Option<&ExactSettings>) -> ExactSettings {
    settings.copied().unwrap_or(ExactSettings {
        blur: analysed.blur,
        enabled: analysed.detector.regions,
    })
}

/// The blurred rectangles of `frame` with `settings`: the padded boxes,
/// merged until they are disjoint, each blurred like the render blurs it.
fn blurred_rects(
    project: &OpenProject,
    analysed: &Project,
    settings: &ExactSettings,
    seeker: &mut FrameSeeker,
    frame: u32,
) -> Result<Vec<(RectI, YuvRoi)>, AppError> {
    let mut boxes = Vec::new();
    analysed
        .plan()
        .boxes_at(frame, &settings.enabled, &mut boxes);
    if boxes.is_empty() {
        return Ok(Vec::new());
    }
    let decoded = seeker.frame(frame)?;
    let params = pixel_params(project, analysed, &settings.blur)?;
    let mut prepared = Vec::new();
    let info = project.info();
    prepare_boxes(&boxes, info.width, info.height, &params, &mut prepared);
    merged_rects(prepared.iter().map(PreparedBox::padded).collect())
        .into_iter()
        .map(|rect| {
            let inside: Vec<BlurBox> = boxes
                .iter()
                .zip(&prepared)
                .filter(|(_, p)| p.padded().intersection(&rect).is_some())
                .map(|(b, _)| *b)
                .collect();
            blurred(project, analysed, &settings.blur, &decoded, rect, &inside)
        })
        .collect()
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

/// Copies `rect` out of `decoded` and blurs `boxes` into it with `blur`;
/// answers the rectangle actually copied (aligned to even coordinates)
/// and its pixels.
fn blurred(
    project: &OpenProject,
    analysed: &Project,
    blur: &BlurParams,
    decoded: &blurrd_video::VideoFrame,
    rect: RectI,
    boxes: &[BlurBox],
) -> Result<(RectI, YuvRoi), AppError> {
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
    Ok((copied, roi))
}

/// The JPEG of `roi`, a part of a frame of `project`.
fn encode(project: &OpenProject, roi: &YuvRoi) -> Result<Vec<u8>, AppError> {
    let info = project.info();
    let converter = RgbConverter::new(VideoColour::from_info(&info.color, info.height));
    let mut rgb = RgbImage::default();
    converter.convert(roi, &mut rgb)?;
    jpeg(&rgb)
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

    /// Writes `frames` frames of deterministic noise (FFV1 in Matroska,
    /// 60 fps, BT.709 limited range, as the engine's synthetic clips).
    fn write_noise_clip(path: &std::path::Path, width: u32, height: u32, frames: u32) {
        use ffmpeg_the_third::util::color;
        use ffmpeg_the_third::{Rational, codec, encoder, format, frame};

        ffmpeg_the_third::init().unwrap();
        let mut out = format::output(path).unwrap();
        let codec = encoder::find_by_name("ffv1").unwrap();
        let mut ctx = codec::Context::new_with_codec(codec)
            .encoder()
            .video()
            .unwrap();
        ctx.set_width(width);
        ctx.set_height(height);
        ctx.set_format(format::Pixel::YUV420P);
        ctx.set_time_base(Rational::new(1, 60));
        ctx.set_colorspace(color::Space::BT709);
        ctx.set_color_range(color::Range::MPEG);
        ctx.set_flags(codec::Flags::GLOBAL_HEADER);
        let mut enc = ctx.open().unwrap();
        let mut stream = out.add_stream(codec).unwrap();
        stream.set_parameters(codec::Parameters::from(&enc));
        stream.set_time_base(Rational::new(1, 60));
        out.write_header().unwrap();
        let stream_tb = out.stream(0).unwrap().time_base();
        let mut picture = frame::Video::new(format::Pixel::YUV420P, width, height);
        let mut packet = codec::packet::Packet::empty();
        let mut write = |enc: &mut encoder::video::Encoder, out: &mut format::context::Output| {
            while enc.receive_packet(&mut packet).is_ok() {
                packet.set_stream(0);
                packet.rescale_ts(Rational::new(1, 60), stream_tb);
                packet.write_interleaved(out).unwrap();
            }
        };
        let mut state = 0x2545_f491_4f6c_dd1d_u64;
        for index in 0..frames {
            for plane in 0..3 {
                for byte in picture.data_mut(plane) {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    *byte = 16 + (state >> 56) as u8 % 220;
                }
            }
            picture.set_pts(Some(i64::from(index)));
            enc.send_frame(&picture).unwrap();
            write(&mut enc, &mut out);
        }
        enc.send_eof().unwrap();
        write(&mut enc, &mut out);
        out.write_trailer().unwrap();
    }

    /// Frame `index` of the video at `path`.
    fn decoded_frame(path: &std::path::Path, index: u32) -> blurrd_video::VideoFrame {
        let options = blurrd_video::DecodeOptions::default();
        let mut decoder = blurrd_video::Decoder::open(path, &options).unwrap();
        loop {
            let frame = decoder.next_frame().unwrap().unwrap();
            if frame.index() == index {
                return frame;
            }
        }
    }

    /// The pixels of `rect` of `frame`: luma, then both chroma planes.
    fn pixels_of(frame: &blurrd_video::VideoFrame, rect: RectI) -> Vec<u8> {
        let mut roi = blurrd_video::YuvRoi::new();
        frame.roi_yuv(rect, &mut roi).unwrap();
        roi_bytes(&roi)
    }

    fn roi_bytes(roi: &blurrd_video::YuvRoi) -> Vec<u8> {
        let (u, v) = roi.chroma();
        let mut bytes = roi.luma().to_vec();
        bytes.extend_from_slice(u);
        bytes.extend_from_slice(v.unwrap_or_default());
        bytes
    }

    /// Exports `project` with `settings` through the job queue as a
    /// lossless FFV1 file in `out`; answers the file.
    fn export(
        engine: &std::sync::Arc<blurrd_engine::Engine>,
        project: &std::sync::Arc<crate::projects::OpenProject>,
        out: &std::path::Path,
        settings: &blurrd_core::ExactSettings,
    ) -> std::path::PathBuf {
        use std::sync::Arc;

        use blurrd_core::{
            Container, EncoderSetting, ExportSettings, JobEvent, JobId, OutputFolder,
        };

        use crate::jobs::Jobs;
        use crate::jobs::tests::Recorder;
        use crate::tasks::{ExportRequest, render};

        std::fs::create_dir_all(out).unwrap();
        let mut export = ExportSettings {
            encoder: EncoderSetting::Named("ffv1".to_owned()),
            container: Container::Mkv,
            ..ExportSettings::default()
        };
        export.output.folder = OutputFolder::Folder(out.to_path_buf());
        let jobs = Jobs::start().unwrap();
        let recorder = Arc::new(Recorder::default());
        let job = JobId(uuid::Uuid::now_v7());
        let request = ExportRequest {
            export,
            blur: settings.blur,
            enabled: settings.enabled,
        };
        jobs.enqueue(
            job,
            recorder.clone(),
            render(Arc::clone(engine), Arc::clone(project), request),
        );
        recorder.wait_end(job);
        jobs.shutdown();
        assert!(matches!(
            recorder.events.lock().unwrap().last(),
            Some((_, JobEvent::Finished { .. }))
        ));
        std::fs::read_dir(out)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.extension().is_some_and(|ext| ext == "mkv"))
            .unwrap()
    }

    /// Exact shows the pixels of the export made with the same settings:
    /// a blur that is not the project's and a region switched off after
    /// the analysis (CPU only, lossless FFV1 output).
    #[test]
    fn exact_patches_equal_the_export_with_its_settings() {
        use blurrd_core::{
            BlurParams, BlurStyle, ExactSettings, JobEvent, JobId, Override, Region, RegionSet,
            Strength,
        };
        use blurrd_engine::{AnalyzeRequest, CancellationToken, ProgressSink};
        use uuid::Uuid;

        use super::ExactRenderer;
        use crate::tasks::tests::open;

        struct Quiet;
        impl ProgressSink for Quiet {
            fn emit(&self, _: JobEvent) {}
        }

        const FRAME: u32 = 12;
        let media = std::env::temp_dir().join(format!("blurrd-app-noise-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&media);
        std::fs::create_dir_all(&media).unwrap();
        let clip = media.join("noise.mkv");
        write_noise_clip(&clip, 1280, 720, 24);
        let (engine, project, work) = open(&clip, "exact-fidelity");
        let mut req = AnalyzeRequest::new(JobId(Uuid::now_v7()), project.source().to_path_buf());
        req.verify = false;
        req.previews = false;
        engine
            .analyze(&req, &Quiet, &CancellationToken::new())
            .unwrap();

        // One edit box per region kind; the kill feed is switched off.
        let add = |region, x0, y0, x1, y1| Override::Add {
            uid: Uuid::now_v7(),
            region,
            f0: 0,
            f1: 23,
            rect: rect(x0, y0, x1, y1),
        };
        let off = rect(900, 60, 1200, 92);
        let revision = project.tracks().revision;
        project
            .set_overrides(
                revision,
                vec![
                    add(Region::Manual, 100, 100, 301, 141),
                    add(Region::KillFeed, off.x0, off.y0, off.x1, off.y1),
                    add(Region::TextChat, 61, 500, 400, 531),
                ],
            )
            .unwrap();
        let mut enabled = RegionSet::all();
        enabled.remove(Region::KillFeed);
        let settings = ExactSettings {
            blur: BlurParams {
                style: BlurStyle::Gaussian,
                strength: Strength::High,
                pad: (7, 5),
                radius: 3,
                feather: 2.5,
                noise: true,
            },
            enabled,
        };
        assert_ne!(settings.blur, project.analysed().unwrap().blur);

        let exact = ExactRenderer::new();
        let patches = exact.pixels(&project, FRAME, Some(&settings)).unwrap();
        let listed = exact.frame(&project, FRAME, Some(&settings)).unwrap();
        assert_eq!(
            listed.patches.iter().map(|p| p.rect).collect::<Vec<_>>(),
            patches.iter().map(|(rect, _)| *rect).collect::<Vec<_>>()
        );
        assert!(patches.len() >= 2, "{}", patches.len());
        assert!(
            patches
                .iter()
                .all(|(rect, _)| rect.intersection(&off).is_none()),
            "a patch covers the switched-off region"
        );

        // The export with the same settings.
        let output = export(&engine, &project, &work.join("out"), &settings);

        let exported = decoded_frame(&output, FRAME);
        let source = decoded_frame(&clip, FRAME);
        for (rect, roi) in &patches {
            assert_eq!(pixels_of(&exported, *rect), roi_bytes(roi), "{rect:?}");
            assert_ne!(pixels_of(&source, *rect), roi_bytes(roi), "{rect:?}");
        }
        assert_eq!(pixels_of(&exported, off), pixels_of(&source, off));

        // The project's own settings would show other pixels.
        let stale = exact.pixels(&project, FRAME, None).unwrap();
        assert!(
            stale
                .iter()
                .any(|(rect, roi)| pixels_of(&exported, *rect) != roi_bytes(roi))
        );
        drop(exact);
        std::fs::remove_dir_all(&work).unwrap();
        std::fs::remove_dir_all(&media).unwrap();
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
        let first = exact.frame(&project, start, None).unwrap();
        let cold = cold.elapsed();
        assert_eq!(first.frame, start);
        assert_ne!(first.patches, vec![]);
        // The JPEG requests that follow are answered from the frame's patches.
        let jpeg = Instant::now();
        for patch in &first.patches {
            let bytes = exact.patch(&project, start, &patch.roi, None).unwrap();
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
            let answer = exact.frame(&project, frame, None).unwrap();
            for patch in &answer.patches {
                exact.patch(&project, frame, &patch.roi, None).unwrap();
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
