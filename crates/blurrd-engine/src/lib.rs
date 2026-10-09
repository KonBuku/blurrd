//! The jobs of blurrd, shared by the CLI and the desktop app.
//!
//! The engine connects decoding, detection, OCR, blurring and encoding into
//! jobs (analysis with verification, render, run, audit). It grows with the
//! milestones that need it; so far it holds:
//!
//! - [`dictionary`]: pass 0 of the verification, the in-memory name
//!   dictionary built by OCR over the protected zones of the source.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_possible_wrap,
    reason = "frame sizes are at most a few thousand pixels, so coordinates never wrap in i32"
)]

pub mod dictionary;

pub use dictionary::{DictionaryBuilder, Pass0Stats, ProtectedZone, ZoneKind, protected_zones};
