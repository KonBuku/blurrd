//! Errors of the OCR crate.

use std::path::PathBuf;

use blurrd_core::{AppError, ErrorKind};

/// Why OCR could not be set up or run.
///
/// No variant ever carries recognised text: messages describe files,
/// shapes and runtime failures only (architecture plan 10).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum OcrError {
    /// The ONNX Runtime library could not be loaded (missing file, missing
    /// dependency, or a build older than the C API level blurrd needs).
    #[error("ONNX Runtime could not be loaded from {path}: {reason}")]
    RuntimeUnavailable {
        /// The library blurrd tried to load.
        path: PathBuf,
        /// What went wrong.
        reason: String,
    },
    /// ONNX Runtime was already loaded from another file in this process; a
    /// process can hold only one ONNX Runtime.
    #[error("ONNX Runtime is already loaded from {loaded}, not {requested}")]
    RuntimeConflict {
        /// The library that is loaded.
        loaded: PathBuf,
        /// The library that was requested.
        requested: PathBuf,
    },
    /// A model file is missing.
    #[error("OCR model file {path} is missing")]
    ModelMissing {
        /// The missing file.
        path: PathBuf,
    },
    /// A model file does not match its pinned checksum, or is unusable.
    #[error("OCR model file {path} is corrupt: {reason}")]
    ModelCorrupt {
        /// The file.
        path: PathBuf,
        /// What is wrong with it.
        reason: String,
    },
    /// Reading a model file failed.
    #[error("reading {path} failed")]
    Io {
        /// The file.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },
    /// The requested execution provider is not available on this machine.
    #[error("the {provider} execution provider is unavailable: {reason}")]
    ProviderUnavailable {
        /// The provider, for example `DirectML`.
        provider: &'static str,
        /// What went wrong.
        reason: String,
    },
    /// Creating or running an ONNX Runtime session failed.
    #[error("ONNX Runtime failed while {action}")]
    Session {
        /// What blurrd was doing, for example `running the detection model`.
        action: &'static str,
        /// The underlying error.
        #[source]
        source: ort::Error,
    },
    /// The model's inputs or outputs are not what blurrd expects.
    #[error("unexpected model signature: {0}")]
    Signature(String),
    /// An image or zone passed to the engine is invalid.
    #[error("invalid OCR input: {0}")]
    InvalidInput(String),
}

impl OcrError {
    /// Wraps an ONNX Runtime error with what blurrd was doing.
    pub(crate) fn session<R>(action: &'static str) -> impl FnOnce(ort::Error<R>) -> Self
    where
        ort::Error: From<ort::Error<R>>,
    {
        move |error| Self::Session {
            action,
            source: error.into(),
        }
    }
}

impl From<OcrError> for AppError {
    fn from(error: OcrError) -> Self {
        let (kind, message) = match &error {
            OcrError::ModelMissing { .. } | OcrError::ModelCorrupt { .. } | OcrError::Io { .. } => {
                (
                    ErrorKind::ModelMissing,
                    "An OCR model file is missing or damaged. Reinstall blurrd to repair it.",
                )
            }
            OcrError::InvalidInput(_) => (ErrorKind::Internal, "OCR received an invalid image."),
            _ => (
                ErrorKind::OcrUnavailable,
                "OCR is unavailable, so the result cannot be verified.",
            ),
        };
        Self::new(kind, message).with_detail(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use blurrd_core::{AppError, ErrorKind};

    use super::OcrError;

    #[test]
    fn maps_to_app_errors() {
        let missing = AppError::from(OcrError::ModelMissing {
            path: PathBuf::from("models/x.onnx"),
        });
        assert_eq!(missing.kind, ErrorKind::ModelMissing);
        let runtime = AppError::from(OcrError::RuntimeUnavailable {
            path: PathBuf::from("onnxruntime.dll"),
            reason: "not found".to_owned(),
        });
        assert_eq!(runtime.kind, ErrorKind::OcrUnavailable);
        assert!(runtime.detail.unwrap().contains("onnxruntime.dll"));
    }
}
