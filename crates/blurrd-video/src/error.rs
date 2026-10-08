//! The error type of `blurrd-video` and its mapping to [`AppError`].

use std::path::PathBuf;

use blurrd_core::{AppError, ErrorKind};

/// Why a video operation failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum VideoError {
    /// An FFmpeg call failed. The message comes from `av_strerror`.
    #[error("{context}: {source}")]
    Ffmpeg {
        /// What blurrd was doing.
        context: &'static str,
        /// The FFmpeg error.
        #[source]
        source: ffmpeg_the_third::Error,
    },
    /// Reading a file failed.
    #[error("{context} {path}: {source}")]
    Io {
        /// What blurrd was doing.
        context: &'static str,
        /// The file.
        path: PathBuf,
        /// The I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The FFmpeg libraries loaded at run time are not the ones blurrd was
    /// built against.
    #[error("FFmpeg library {library} has version {found}, blurrd needs major version {expected}")]
    LibraryVersion {
        /// Library name, for example `avcodec`.
        library: &'static str,
        /// Major version blurrd was compiled against.
        expected: u32,
        /// Version found at run time (`major.minor.micro`).
        found: String,
    },
    /// The file has no video stream.
    #[error("the file has no video stream")]
    NoVideoStream,
    /// The video uses a codec, pixel format or layout blurrd does not
    /// process.
    #[error("unsupported video: {0}")]
    Unsupported(String),
    /// A decoded frame has no usable presentation timestamp, or timestamps
    /// do not increase.
    #[error("invalid timestamps: {0}")]
    Timestamps(String),
    /// A `pts.bin` file is malformed.
    #[error("invalid PTS table {path}: {reason}")]
    InvalidPtsTable {
        /// The file.
        path: PathBuf,
        /// What is wrong with it.
        reason: String,
    },
    /// The decoded video does not match the PTS table of the project (the
    /// file changed after analysis, or a decoder behaves differently).
    #[error("frame {index} has PTS {found}, the project expects {expected}")]
    PtsMismatch {
        /// Presentation frame index.
        index: u32,
        /// PTS stored in the project's PTS table.
        expected: i64,
        /// PTS of the decoded frame.
        found: i64,
    },
    /// Seeking found no decoded frame with the PTS the project's table
    /// expects for this index.
    #[error("frame {index} (PTS {pts}) was not found in the video")]
    FrameNotFound {
        /// Presentation frame index.
        index: u32,
        /// PTS stored in the project's PTS table.
        pts: i64,
    },
    /// The seek thread of a `FrameSeeker` is gone (it panicked).
    #[error("the frame seeker's decoder thread stopped")]
    SeekerStopped,
    /// A requested frame index is past the end of the video.
    #[error("frame {index} is past the end of the video ({count} frames)")]
    FrameOutOfRange {
        /// Requested presentation frame index.
        index: u32,
        /// Number of frames in the video.
        count: u32,
    },
}

impl VideoError {
    /// Wraps an FFmpeg error with what blurrd was doing.
    pub(crate) fn ffmpeg(context: &'static str) -> impl FnOnce(ffmpeg_the_third::Error) -> Self {
        move |source| Self::Ffmpeg { context, source }
    }

    /// The category shown to the user.
    pub fn kind(&self) -> ErrorKind {
        match self {
            Self::Io { source, .. } if source.kind() == std::io::ErrorKind::NotFound => {
                ErrorKind::NotFound
            }
            Self::Io { .. } => ErrorKind::Io,
            Self::Ffmpeg { source, .. } => match source {
                ffmpeg_the_third::Error::InvalidData
                | ffmpeg_the_third::Error::DecoderNotFound
                | ffmpeg_the_third::Error::DemuxerNotFound
                | ffmpeg_the_third::Error::StreamNotFound
                | ffmpeg_the_third::Error::ProtocolNotFound
                | ffmpeg_the_third::Error::PatchWelcome => ErrorKind::UnsupportedMedia,
                ffmpeg_the_third::Error::Other { errno } if *errno == libc_enoent() => {
                    ErrorKind::NotFound
                }
                _ => ErrorKind::Decode,
            },
            Self::NoVideoStream | Self::Unsupported(_) => ErrorKind::UnsupportedMedia,
            Self::Timestamps(_) | Self::FrameOutOfRange { .. } => ErrorKind::Decode,
            Self::InvalidPtsTable { .. }
            | Self::PtsMismatch { .. }
            | Self::FrameNotFound { .. } => ErrorKind::ProjectMismatch,
            Self::LibraryVersion { .. } | Self::SeekerStopped => ErrorKind::Internal,
        }
    }
}

/// `ENOENT`, which FFmpeg returns as `AVERROR(ENOENT)` for a missing file.
const fn libc_enoent() -> i32 {
    2
}

impl From<VideoError> for AppError {
    fn from(error: VideoError) -> Self {
        let message = match error.kind() {
            ErrorKind::NotFound => "The file does not exist.",
            ErrorKind::UnsupportedMedia => "This file is not a video blurrd can process.",
            ErrorKind::ProjectMismatch => "The video changed since it was analysed.",
            ErrorKind::Io => "The file could not be read.",
            ErrorKind::Internal => "blurrd is installed incorrectly; reinstall it.",
            _ => "The video could not be decoded.",
        };
        Self::new(error.kind(), message).with_detail(error.to_string())
    }
}
