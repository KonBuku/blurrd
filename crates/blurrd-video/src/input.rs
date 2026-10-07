//! Opening inputs safely.

use std::path::Path;

use ffmpeg_the_third::{Dictionary, format, media};

use crate::VideoError;

/// Opens a local media file with FFmpeg.
///
/// The protocol whitelist is `file`: playlists, reference movies or concat
/// scripts inside a crafted file cannot make FFmpeg open other protocols
/// (no network access, no pipes). The path is checked first so a missing
/// file reports [`VideoError::Io`] with the path.
pub(crate) fn open(path: &Path) -> Result<format::context::Input, VideoError> {
    let meta = std::fs::metadata(path).map_err(|source| VideoError::Io {
        context: "opening",
        path: path.to_path_buf(),
        source,
    })?;
    if !meta.is_file() {
        return Err(VideoError::Unsupported(format!(
            "{} is not a file",
            path.display()
        )));
    }
    let mut options = Dictionary::new();
    options.set("protocol_whitelist", "file");
    format::input_with_dictionary(path, options).map_err(VideoError::ffmpeg("opening the file"))
}

/// Index of the video stream FFmpeg considers the main one.
pub(crate) fn video_stream(input: &format::context::Input) -> Result<usize, VideoError> {
    input
        .streams()
        .best(media::Type::Video)
        .map(|stream| stream.index())
        .ok_or(VideoError::NoVideoStream)
}
