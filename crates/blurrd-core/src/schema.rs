//! Versioned JSON documents.
//!
//! Every file blurrd writes starts with `"schema": "<name>/<major>"` and
//! `"version": <revision>`. The major number changes when a reader of the
//! previous major could misread the file; the revision counts compatible
//! additions within a major. Readers accept every revision of their own
//! major (unknown fields are ignored, missing optional fields take their
//! defaults), would migrate older majors (none exist before v0.1.0), and
//! refuse newer majors.

use serde::Serialize;
use serde::de::DeserializeOwned;

/// A top-level JSON document with a versioned schema.
pub trait Document: Serialize + DeserializeOwned {
    /// The document name, for example `blurrd-project`.
    const NAME: &'static str;
    /// The major version this build reads and writes.
    const MAJOR: u32;
    /// The revision within [`Document::MAJOR`] this build writes.
    const REVISION: u32;

    /// The schema identifier written into new documents, for example
    /// `blurrd-project/1`.
    fn schema_id() -> String {
        format!("{}/{}", Self::NAME, Self::MAJOR)
    }

    /// Checks invariants that serde cannot express.
    ///
    /// # Errors
    ///
    /// Returns [`SchemaError::Invalid`] describing the first violation.
    fn validate(&self) -> Result<(), SchemaError> {
        Ok(())
    }
}

/// Why a document could not be read or written.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SchemaError {
    /// The text is not valid JSON or does not match the document's shape.
    #[error("malformed document: {0}")]
    Malformed(String),
    /// The `schema` field is missing or names another kind of document.
    #[error("expected a {expected} document, found schema `{found}`")]
    WrongDocument {
        /// The expected document name.
        expected: &'static str,
        /// The `schema` value found.
        found: String,
    },
    /// The document was written by a newer, incompatible blurrd.
    #[error("{name} schema version {found} is newer than the supported version {supported}")]
    NewerMajor {
        /// The document name.
        name: &'static str,
        /// The major version found.
        found: u32,
        /// The major version this build supports.
        supported: u32,
    },
    /// The document uses an older major version that cannot be migrated.
    #[error("{name} schema version {found} is no longer supported (current version {supported})")]
    OlderMajor {
        /// The document name.
        name: &'static str,
        /// The major version found.
        found: u32,
        /// The major version this build supports.
        supported: u32,
    },
    /// The document parsed but violates an invariant.
    #[error("invalid document: {0}")]
    Invalid(String),
}

impl From<serde_json::Error> for SchemaError {
    fn from(error: serde_json::Error) -> Self {
        Self::Malformed(error.to_string())
    }
}

/// Parses and validates a document.
///
/// # Errors
///
/// Returns a [`SchemaError`] if the JSON is malformed, the schema names
/// another document or an unsupported major version, or validation fails.
pub fn from_json<D: Document>(json: &str) -> Result<D, SchemaError> {
    #[derive(serde::Deserialize)]
    struct Header {
        schema: Option<String>,
    }

    let header: Header = serde_json::from_str(json)?;
    let found = header.schema.unwrap_or_default();
    let major = found
        .strip_prefix(D::NAME)
        .and_then(|rest| rest.strip_prefix('/'))
        .and_then(|major| major.parse::<u32>().ok())
        .ok_or_else(|| SchemaError::WrongDocument {
            expected: D::NAME,
            found: found.clone(),
        })?;
    if major > D::MAJOR {
        return Err(SchemaError::NewerMajor {
            name: D::NAME,
            found: major,
            supported: D::MAJOR,
        });
    }
    if major < D::MAJOR {
        return Err(SchemaError::OlderMajor {
            name: D::NAME,
            found: major,
            supported: D::MAJOR,
        });
    }
    let document: D = serde_json::from_str(json)?;
    document.validate()?;
    Ok(document)
}

/// Validates a document and writes it as compact JSON.
///
/// # Errors
///
/// Returns a [`SchemaError`] if validation or serialisation fails.
pub fn to_json<D: Document>(document: &D) -> Result<String, SchemaError> {
    document.validate()?;
    Ok(serde_json::to_string(document)?)
}

/// Validates a document and writes it as indented JSON, for files that
/// users may read (exported projects and presets).
///
/// # Errors
///
/// Returns a [`SchemaError`] if validation or serialisation fails.
pub fn to_json_pretty<D: Document>(document: &D) -> Result<String, SchemaError> {
    document.validate()?;
    Ok(serde_json::to_string_pretty(document)?)
}
