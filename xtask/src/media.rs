//! Checks the local recordings against the committed media manifest.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use blurrd_testkit::media::{ClipStatus, MEDIA_ENV, Manifest, MediaDir};

/// Verifies every clip of `tests/media/manifest.toml` in `media` (or the
/// folder in `BLURRD_TEST_MEDIA`): present and matching its SHA-256.
pub(crate) fn verify(media: Option<PathBuf>) -> Result<()> {
    let manifest = Manifest::load_repository().context("loading tests/media/manifest.toml")?;
    let media = match media {
        Some(root) => MediaDir::new(root),
        None => MediaDir::from_env()
            .with_context(|| format!("set {MEDIA_ENV} or pass --media <DIR>"))?,
    };
    let mut ok = Vec::new();
    for clip in &manifest.clips {
        let status = media.verify(clip);
        let split = if clip.held_out { "held-out" } else { "tuning" };
        eprintln!("{:<12} {split:<8} {status}", clip.id);
        if status == ClipStatus::Ok {
            ok.push(clip.id.as_str());
        }
    }
    let failed = manifest.clips.len() - ok.len();
    let excerpts = manifest
        .excerpts
        .iter()
        .filter(|e| ok.contains(&e.clip.as_str()))
        .count();
    if failed > 0 {
        bail!(
            "{failed} of {} clips are missing or differ in {}",
            manifest.clips.len(),
            media.root().display()
        );
    }
    eprintln!(
        "all {} clips match; {excerpts} of {} excerpts available",
        manifest.clips.len(),
        manifest.excerpts.len()
    );
    Ok(())
}
