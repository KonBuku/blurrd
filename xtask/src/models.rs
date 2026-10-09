//! The OCR models pinned in `models.toml`: download and verify them.
//!
//! Bundled models go into `models/` (gitignored), benchmark candidates into
//! `models/candidates/`. Files that are already there with the right size
//! and checksum are kept.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;

use crate::ffmpeg::{curl, is_sha256_hex, sha256_file};

/// The pin file `models.toml`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pins {
    /// Where the ONNX files come from (informational, checked for presence).
    conversion: Conversion,
    /// URL that every `path` is relative to.
    base_url: String,
    /// The pinned files.
    model: Vec<ModelPin>,
}

/// Provenance of the ONNX conversion.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Conversion {
    by: String,
    repository: String,
    revision: String,
    tool: String,
    license: String,
    upstream: String,
}

/// What a file is used for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Role {
    /// Text detection model.
    Det,
    /// Text recognition model.
    Rec,
    /// Character list of a recognition model.
    Dict,
}

/// One pinned file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelPin {
    /// File name in `models/` (or `models/candidates/`).
    file: String,
    /// What it is.
    role: Role,
    /// Whether the installer bundles it (otherwise a benchmark candidate).
    bundled: bool,
    /// Path below `base_url`.
    path: String,
    /// Size in bytes.
    size: u64,
    /// SHA-256, lowercase hex.
    sha256: String,
}

impl Pins {
    fn parse(text: &str) -> Result<Self> {
        let pins: Self = toml::from_str(text)?;
        ensure!(
            pins.base_url.starts_with("https://"),
            "`base_url` must be an https URL"
        );
        let c = &pins.conversion;
        ensure!(
            [
                &c.by,
                &c.repository,
                &c.revision,
                &c.tool,
                &c.license,
                &c.upstream
            ]
            .iter()
            .all(|v| !v.is_empty()),
            "every field of [conversion] must be filled in"
        );
        for pin in &pins.model {
            ensure!(
                is_sha256_hex(&pin.sha256),
                "`sha256` of {} must be 64 lowercase hex digits",
                pin.file
            );
            ensure!(
                !pin.file.is_empty()
                    && !pin.file.contains(['/', '\\'])
                    && pin.file != ".."
                    && !pin.path.starts_with('/'),
                "invalid file name or path for {}",
                pin.file
            );
        }
        for role in [Role::Det, Role::Rec, Role::Dict] {
            let bundled = pins
                .model
                .iter()
                .filter(|m| m.bundled && m.role == role)
                .count();
            ensure!(
                bundled == 1,
                "exactly one bundled {role:?} file is required, found {bundled}"
            );
        }
        Ok(pins)
    }

    fn url(&self, pin: &ModelPin) -> String {
        format!("{}/{}", self.base_url.trim_end_matches('/'), pin.path)
    }
}

/// Downloads every pinned bundled model (and with `candidates` every
/// benchmark candidate) that is missing or outdated.
pub(crate) fn fetch(root: &Path, candidates: bool) -> Result<()> {
    let pin_path = root.join("models.toml");
    let text =
        fs::read_to_string(&pin_path).with_context(|| format!("reading {}", pin_path.display()))?;
    let pins = Pins::parse(&text).with_context(|| format!("parsing {}", pin_path.display()))?;
    let models = root.join("models");
    let mut downloaded = 0;
    let mut checked = 0;
    for pin in pins.model.iter().filter(|m| m.bundled || candidates) {
        let dir = if pin.bundled {
            models.clone()
        } else {
            models.join("candidates")
        };
        fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let path = dir.join(&pin.file);
        checked += 1;
        if is_current(&path, pin)? {
            continue;
        }
        let partial = dir.join(format!("{}.part", pin.file));
        curl(&pins.url(pin), &partial, None, "*/*")?;
        if !is_current(&partial, pin)? {
            // Best effort: a stale partial file is overwritten next time.
            let _ = fs::remove_file(&partial);
            bail!(
                "{} does not match the size and SHA-256 pinned in models.toml",
                pin.file
            );
        }
        fs::rename(&partial, &path)
            .with_context(|| format!("moving {} to {}", partial.display(), path.display()))?;
        downloaded += 1;
    }
    eprintln!(
        "{checked} model files up to date in {} ({downloaded} downloaded)",
        models.display()
    );
    Ok(())
}

/// Whether `path` exists with the pinned size and checksum.
fn is_current(path: &Path, pin: &ModelPin) -> Result<bool> {
    match fs::metadata(path) {
        Ok(meta) if meta.is_file() && meta.len() == pin.size => {
            Ok(sha256_file(path)? == pin.sha256)
        }
        _ => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::{Pins, Role};

    const PINS: &str = r#"
base_url = "https://example.org/models/v1"

[conversion]
by = "someone"
repository = "https://example.org/models"
revision = "v1"
tool = "paddle2onnx"
license = "Apache-2.0"
upstream = "PaddleOCR"

[[model]]
file = "det.onnx"
role = "det"
bundled = true
path = "onnx/det.onnx"
size = 3
sha256 = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"

[[model]]
file = "rec.onnx"
role = "rec"
bundled = true
path = "onnx/rec.onnx"
size = 3
sha256 = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"

[[model]]
file = "dict.txt"
role = "dict"
bundled = true
path = "dict.txt"
size = 3
sha256 = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
"#;

    #[test]
    fn parses_pins() {
        let pins = Pins::parse(PINS).unwrap();
        assert_eq!(
            pins.url(&pins.model[0]),
            "https://example.org/models/v1/onnx/det.onnx"
        );
        assert_eq!(pins.model[2].role, Role::Dict);
    }

    #[test]
    fn rejects_bad_pins() {
        assert!(Pins::parse(&PINS.replace("414140de5dae", "414140DE5DAE")).is_err());
        assert!(Pins::parse(&PINS.replace("\"det.onnx\"", "\"../det.onnx\"")).is_err());
        assert!(Pins::parse(&PINS.replace("tool = \"paddle2onnx\"", "tool = \"\"")).is_err());
        // Exactly one bundled file per role.
        assert!(Pins::parse(&PINS.replacen("bundled = true", "bundled = false", 1)).is_err());
    }

    #[test]
    fn repository_pins_are_valid() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let text = std::fs::read_to_string(root.join("models.toml")).unwrap();
        Pins::parse(&text).unwrap();
    }
}
