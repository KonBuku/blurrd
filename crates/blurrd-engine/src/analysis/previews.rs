//! Thumbnails and the preview proxy, made from the frames the analysis
//! decodes anyway (plan 6.2 and 7.1 side outputs).
//!
//! A thumbnail of [`THUMB_HEIGHT`] lines is written every
//! [`THUMB_INTERVAL`] seconds of presentation time as
//! [`ProjectDir::thumbnail`] `k` (`k` counts the intervals), from the
//! first frame at or after the interval's start. Videos the desktop app's
//! player cannot play also get `proxy.mp4` (see
//! [`blurrd_video::ProxyWriter`]). Neither is needed for blurring, so a
//! failure only becomes a warning.

use std::path::PathBuf;
use std::sync::Arc;

use blurrd_core::{MediaInfo, Rational};
use blurrd_video::{EncodeOptions, ProxyWriter, RgbConverter, RgbImage, VideoFrame};
use crossbeam_channel::Receiver;
use jpeg_encoder::{ColorType, Encoder};

use super::frame::FrameCrops;
use crate::cache::{ProjectDir, THUMBNAIL_INTERVAL_S as THUMB_INTERVAL};

/// Height of a thumbnail in pixels (160 × 90 for 16:9 videos).
pub(crate) const THUMB_HEIGHT: u32 = 90;
/// JPEG quality of the thumbnails.
const JPEG_QUALITY: u8 = 80;

/// What the previews produced.
#[derive(Debug, Default)]
pub(crate) struct PreviewOutput {
    /// Thumbnails written.
    pub(crate) thumbnails: u32,
    /// The proxy, when the video needed one and it was made.
    pub(crate) proxy: Option<PathBuf>,
    /// What went wrong (no text from the video, only causes).
    pub(crate) problems: Vec<String>,
}

/// A proxy to make: the writer is created on the previews' own thread
/// (the muxer stays on the thread that opened it).
struct ProxyPlan {
    info: MediaInfo,
    path: PathBuf,
    options: EncodeOptions,
}

/// Makes the previews of one video.
pub(crate) struct Previews {
    dir: ProjectDir,
    converter: RgbConverter,
    time_base: Rational,
    size: (u32, u32),
    proxy: Option<ProxyPlan>,
    first_pts: Option<i64>,
    /// The next thumbnail to write.
    next: u32,
    rgb: RgbImage,
    output: PreviewOutput,
}

impl std::fmt::Debug for Previews {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Previews")
            .field("thumbs", &self.dir.thumbs())
            .field("proxy", &self.proxy.as_ref().map(|plan| &plan.path))
            .finish_non_exhaustive()
    }
}

impl Previews {
    /// Prepares the previews of the video `info` describes in the project
    /// folder `dir`. `proxy` holds the encoder options when a proxy may be
    /// made (a proxy already in the cache is kept).
    pub(crate) fn open(
        dir: &ProjectDir,
        info: &MediaInfo,
        converter: RgbConverter,
        proxy: &EncodeOptions,
    ) -> Self {
        let mut output = PreviewOutput::default();
        let thumbs = dir.thumbs();
        if let Err(error) = std::fs::create_dir_all(&thumbs) {
            output.problems.push(format!(
                "the thumbnail folder could not be created: {error}"
            ));
        }
        let path = dir.proxy();
        let mut plan = None;
        if ProxyWriter::needed(info) {
            if path.is_file() {
                output.proxy = Some(path);
            } else {
                plan = Some(ProxyPlan {
                    info: info.clone(),
                    path,
                    options: proxy.clone(),
                });
            }
        }
        Self {
            dir: dir.clone(),
            converter,
            time_base: info.time_base,
            size: thumb_size(info.width, info.height),
            proxy: plan,
            first_pts: None,
            next: 0,
            rgb: RgbImage::default(),
            output,
        }
    }

    /// Takes the frames from `frames` until it closes, or until `stopped`
    /// says the analysis stops (then no proxy is written); returns what
    /// was made.
    pub(crate) fn run(
        mut self,
        frames: &Receiver<Arc<FrameCrops>>,
        stopped: &dyn Fn() -> bool,
    ) -> PreviewOutput {
        let mut proxy = self.proxy.take().and_then(|plan| {
            match ProxyWriter::create(&plan.info, &plan.path, &plan.options) {
                Ok((writer, fallbacks)) => {
                    for fallback in fallbacks {
                        tracing::info!(from = %fallback.from, to = %fallback.to,
                            reason = %fallback.reason, "proxy encoder fallback");
                    }
                    Some(writer)
                }
                Err(error) => {
                    self.problem(format!("the preview proxy could not be started: {error}"));
                    None
                }
            }
        });
        for crops in frames {
            if stopped() {
                break;
            }
            self.push(&crops.frame, &mut proxy);
        }
        if stopped() {
            // The analysis stops: no proxy (an unfinished one leaves no
            // file).
            return self.output;
        }
        if let Some(proxy) = proxy {
            match proxy.finish() {
                Ok(path) => self.output.proxy = Some(path),
                Err(error) => self
                    .output
                    .problems
                    .push(format!("the preview proxy could not be finished: {error}")),
            }
        }
        self.output
    }

    /// Adds the next frame (frames in presentation order) to the
    /// thumbnails and to `proxy`.
    fn push(&mut self, frame: &VideoFrame, proxy: &mut Option<ProxyWriter>) {
        let first = *self.first_pts.get_or_insert(frame.pts());
        let seconds = (frame.pts() - first) as f64 * self.time_base.as_f64();
        // Every interval that started by this frame gets this frame (a
        // gap in a variable-rate video repeats it).
        let due = (seconds / THUMB_INTERVAL).floor().max(0.0) as u32;
        if self.next <= due {
            match self.thumbnail(frame) {
                Ok(jpeg) => {
                    for k in self.next..=due {
                        let path = self.dir.thumbnail(k);
                        if let Err(error) = std::fs::write(&path, &jpeg) {
                            self.problem(format!("a thumbnail could not be written: {error}"));
                            break;
                        }
                        self.output.thumbnails += 1;
                    }
                }
                Err(problem) => self.problem(problem),
            }
            self.next = due + 1;
        }
        if let Some(writer) = proxy.as_mut()
            && let Err(error) = writer.push(frame)
        {
            *proxy = None;
            self.problem(format!("the preview proxy stopped: {error}"));
        }
    }

    /// The JPEG thumbnail of `frame`.
    fn thumbnail(&mut self, frame: &VideoFrame) -> Result<Vec<u8>, String> {
        frame
            .thumbnail(self.size.0, self.size.1, &self.converter, &mut self.rgb)
            .map_err(|e| format!("a thumbnail could not be made: {e}"))?;
        let mut jpeg = Vec::new();
        let (width, height) = (
            u16::try_from(self.rgb.width).map_err(|e| e.to_string())?,
            u16::try_from(self.rgb.height).map_err(|e| e.to_string())?,
        );
        Encoder::new(&mut jpeg, JPEG_QUALITY)
            .encode(&self.rgb.data, width, height, ColorType::Rgb)
            .map_err(|e| format!("a thumbnail could not be encoded: {e}"))?;
        Ok(jpeg)
    }

    /// Records a problem once per cause.
    fn problem(&mut self, problem: String) {
        if !self.output.problems.contains(&problem) {
            tracing::warn!(%problem, "preview");
            self.output.problems.push(problem);
        }
    }
}

/// The thumbnail size of a `width × height` video: [`THUMB_HEIGHT`] lines,
/// the video's aspect ratio, even sizes.
fn thumb_size(width: u32, height: u32) -> (u32, u32) {
    let w = (u64::from(width) * u64::from(THUMB_HEIGHT) / u64::from(height.max(1))) as u32;
    ((w.max(2) + 1) & !1, THUMB_HEIGHT)
}

#[cfg(test)]
mod tests {
    use super::thumb_size;

    #[test]
    fn thumbnails_keep_the_aspect_ratio() {
        assert_eq!(thumb_size(1920, 1080), (160, 90));
        assert_eq!(thumb_size(1280, 720), (160, 90));
        assert_eq!(thumb_size(1440, 1080), (120, 90));
        assert_eq!(thumb_size(1920, 1200), (144, 90));
    }
}
