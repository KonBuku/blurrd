//! Poster frames of the queue (`blurrd://poster.jpg?path=<file>`): a small
//! JPEG of one frame of a video the user chose, so a queue row shows the
//! clip before any analysis has run (the timeline thumbnails only exist
//! after one).
//!
//! The frame is the first one decoded after a seek to
//! [`POSTER_POSITION`] of the duration: recordings often open on a black
//! or loading screen. Software decoding of one GOP takes a few dozen
//! milliseconds at 1080p; the frontend caches the answer for the session.

use std::path::Path;

use blurrd_core::{AppError, ErrorKind};
use blurrd_video::{DecodeOptions, Decoder, RgbConverter, RgbImage, VideoColour};

use crate::exact::jpeg;

/// Where in the video the poster frame is taken, as a fraction of the
/// duration.
const POSTER_POSITION: f64 = 0.1;
/// Height of a poster in pixels (the width keeps the aspect ratio): twice
/// the queue tile, sharp on high-DPI screens.
const POSTER_HEIGHT: u32 = 90;
/// Decoder threads for one poster; several posters decode in parallel on
/// the scheme's workers.
const POSTER_THREADS: u32 = 2;

/// The poster JPEG of the video at `path` (an absolute, checked path).
///
/// # Errors
///
/// The probe's and the decoder's errors; [`ErrorKind::UnsupportedMedia`]
/// when the video has no frame.
pub(crate) fn poster(path: &Path) -> Result<Vec<u8>, AppError> {
    let info = blurrd_video::probe(path)?;
    let mut options = DecodeOptions::default();
    options.threads = Some(POSTER_THREADS);
    let mut decoder = Decoder::open(path, &options)?;
    let target = seek_target(
        info.duration_s,
        decoder.time_base().num,
        decoder.time_base().den,
    );
    if target > 0 {
        let start = decoder.start_pts().unwrap_or(0);
        // A failed seek still leaves the first frame to show.
        if decoder.seek_keyframe(start.saturating_add(target)).is_err() {
            decoder = Decoder::open(path, &options)?;
        }
    }
    let frame = decoder.next_frame()?.ok_or_else(|| {
        AppError::new(ErrorKind::UnsupportedMedia, "The video has no frames.")
            .with_detail(path.display().to_string())
    })?;
    let (width, height) = poster_size(info.width, info.height);
    let converter = RgbConverter::new(VideoColour::from_info(&info.color, info.height));
    let mut rgb = RgbImage::default();
    frame.thumbnail(width, height, &converter, &mut rgb)?;
    jpeg(&rgb)
}

/// The poster position of a video of `duration_s` seconds in its stream
/// time base `num / den`, relative to the stream start.
fn seek_target(duration_s: f64, num: u32, den: u32) -> i64 {
    if num == 0 || !duration_s.is_finite() || duration_s <= 0.0 {
        return 0;
    }
    let seconds = duration_s * POSTER_POSITION;
    (seconds * f64::from(den) / f64::from(num)) as i64
}

/// The poster size of a `width × height` video: [`POSTER_HEIGHT`] lines,
/// an even width with the video's aspect ratio.
fn poster_size(width: u32, height: u32) -> (u32, u32) {
    let w = (u64::from(width) * u64::from(POSTER_HEIGHT) / u64::from(height.max(1))) as u32;
    ((w.max(2) + 1) & !1, POSTER_HEIGHT)
}

#[cfg(test)]
mod tests {
    use super::{poster, poster_size, seek_target};
    use crate::exact::tests::write_noise_clip;

    /// A poster of a real (synthetic) video is a JPEG of the poster size.
    #[test]
    fn a_video_gets_a_jpeg_poster() {
        let dir = std::env::temp_dir().join(format!("blurrd-app-poster-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let clip = dir.join("noise.mkv");
        write_noise_clip(&clip, 640, 360, 120);
        let jpeg = poster(&clip).unwrap();
        assert_eq!(&jpeg[..2], &[0xFF, 0xD8], "JPEG start of image");
        // The SOF0 header holds the height and the width.
        let sof = jpeg.windows(2).position(|w| w == [0xFF, 0xC0]).unwrap();
        let size = |at: usize| u16::from_be_bytes([jpeg[sof + at], jpeg[sof + at + 1]]);
        assert_eq!((size(7), size(5)), (160, 90));
        assert!(poster(&dir.join("missing.mkv")).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn posters_keep_the_aspect_ratio() {
        assert_eq!(poster_size(1920, 1080), (160, 90));
        assert_eq!(poster_size(2560, 1080), (214, 90));
        assert_eq!(poster_size(1440, 1080), (120, 90));
        assert_eq!(poster_size(0, 0), (2, 90));
    }

    #[test]
    fn the_poster_is_taken_a_tenth_into_the_video() {
        // 60 s at a 1/15360 time base: 6 s in.
        assert_eq!(seek_target(60.0, 1, 15_360), 92_160);
        assert_eq!(seek_target(10.0, 1, 1000), 1000);
        assert_eq!(seek_target(0.0, 1, 1000), 0);
        assert_eq!(seek_target(f64::NAN, 1, 1000), 0);
        assert_eq!(seek_target(10.0, 0, 1000), 0);
    }
}
