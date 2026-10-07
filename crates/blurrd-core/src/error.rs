//! The error type that crosses the engine boundary (IPC, CLI JSON output).

use std::fmt;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::schema::SchemaError;

/// Category of an [`AppError`], for the UI to choose a message and actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum ErrorKind {
    /// Reading or writing a file failed.
    Io,
    /// A file does not exist.
    NotFound,
    /// The file is not a video blurrd can process.
    UnsupportedMedia,
    /// Decoding failed.
    Decode,
    /// Encoding failed.
    Encode,
    /// No working encoder for the requested codec.
    EncoderUnavailable,
    /// The GPU driver is too old for the encoder or decoder.
    DriverTooOld,
    /// OCR cannot run (runtime or GPU problem).
    OcrUnavailable,
    /// An OCR model file is missing or corrupt.
    ModelMissing,
    /// A project or settings file is malformed or from a newer version.
    InvalidProject,
    /// The video changed since the project was analysed.
    ProjectMismatch,
    /// An edit was based on an older project revision.
    StaleRevision,
    /// The job was canceled.
    Canceled,
    /// A bug in blurrd.
    Internal,
}

/// An error as shown to the user: a category, a one-line message and
/// optional technical detail.
///
/// Messages and details never contain OCR text.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct AppError {
    /// Category.
    pub kind: ErrorKind,
    /// One-line message for the user.
    pub message: String,
    /// Technical detail for bug reports.
    pub detail: Option<String>,
}

impl AppError {
    /// An error without detail.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            detail: None,
        }
    }

    /// The same error with technical detail.
    #[must_use]
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.detail {
            Some(detail) => write!(f, "{} ({detail})", self.message),
            None => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for AppError {}

impl From<SchemaError> for AppError {
    fn from(error: SchemaError) -> Self {
        Self::new(ErrorKind::InvalidProject, "The file could not be read.")
            .with_detail(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::{AppError, ErrorKind};
    use crate::schema::SchemaError;

    #[test]
    fn displays_message_and_detail() {
        let error = AppError::new(ErrorKind::Encode, "Encoding failed.");
        assert_eq!(error.to_string(), "Encoding failed.");
        assert_eq!(
            error.with_detail("code -22").to_string(),
            "Encoding failed. (code -22)"
        );
    }

    #[test]
    fn schema_errors_map_to_invalid_project() {
        let error = AppError::from(SchemaError::Invalid("bad".to_owned()));
        assert_eq!(error.kind, ErrorKind::InvalidProject);
        assert_eq!(error.detail.as_deref(), Some("invalid document: bad"));
    }
}
