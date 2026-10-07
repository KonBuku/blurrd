//! Shared types of blurrd.
//!
//! `blurrd-core` holds the data model that every other crate and the desktop
//! app agree on: regions, box tracks, blur plans, the versioned project,
//! export and settings schemas, events and errors. It performs no I/O and
//! contains no `unsafe` code, so it builds and tests without FFmpeg, ONNX
//! Runtime or a GPU.

#![forbid(unsafe_code)]

/// The blurrd version, shared by every crate of the workspace.
///
/// It is written into project files and reports so that a file can be traced
/// back to the build that produced it.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
