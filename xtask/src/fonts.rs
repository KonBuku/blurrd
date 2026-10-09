//! Fonts for the synthetic HUD fixtures that are too large to commit.
//!
//! `tests/fonts/fonts.toml` pins every file (a Noto Sans family or its
//! license text) by repository path and SHA-256 under one base URL (the
//! Google Fonts repository at a fixed commit). Files are downloaded into
//! `tests/fonts/noto/` (gitignored); files that are already there with the
//! right checksum are kept.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, ensure};
use serde::Deserialize;

use crate::ffmpeg::{curl, is_sha256_hex, sha256_file};

/// The pin file `tests/fonts/fonts.toml`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pins {
    /// URL that every `path` is relative to.
    base_url: String,
    /// The pinned files.
    font: Vec<FontPin>,
}

/// One pinned file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FontPin {
    /// File name in `tests/fonts/noto/`.
    file: String,
    /// Path below `base_url`.
    path: String,
    /// SHA-256 of the file, lowercase hex.
    sha256: String,
}

impl Pins {
    fn parse(text: &str) -> Result<Self> {
        let pins: Self = toml::from_str(text)?;
        ensure!(
            pins.base_url.starts_with("https://"),
            "`base_url` must be an https URL"
        );
        for pin in &pins.font {
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
        Ok(pins)
    }

    /// The download URL of a file. Square brackets (variable font names)
    /// are escaped so that curl does not treat them as a glob.
    fn url(&self, pin: &FontPin) -> String {
        let path = pin.path.replace('[', "%5B").replace(']', "%5D");
        format!("{}/{path}", self.base_url.trim_end_matches('/'))
    }
}

/// Downloads every pinned font that is missing or outdated.
pub(crate) fn fetch(root: &Path) -> Result<()> {
    let fonts = root.join("tests").join("fonts");
    let pin_path = fonts.join("fonts.toml");
    let text =
        fs::read_to_string(&pin_path).with_context(|| format!("reading {}", pin_path.display()))?;
    let pins = Pins::parse(&text).with_context(|| format!("parsing {}", pin_path.display()))?;
    let dest = fonts.join("noto");
    fs::create_dir_all(&dest).with_context(|| format!("creating {}", dest.display()))?;
    let mut downloaded = 0;
    for pin in &pins.font {
        let path = dest.join(&pin.file);
        if path.is_file() && sha256_file(&path)? == pin.sha256 {
            continue;
        }
        let partial = dest.join(format!("{}.part", pin.file));
        curl(&pins.url(pin), &partial, None, "*/*")?;
        let actual = sha256_file(&partial)?;
        if actual != pin.sha256 {
            // Best effort: a stale partial file is overwritten next time.
            let _ = fs::remove_file(&partial);
            anyhow::bail!(
                "{} has SHA-256 {actual}, but tests/fonts/fonts.toml pins {}",
                pin.file,
                pin.sha256
            );
        }
        fs::rename(&partial, &path)
            .with_context(|| format!("moving {} to {}", partial.display(), path.display()))?;
        downloaded += 1;
    }
    eprintln!(
        "{} font files up to date in {} ({downloaded} downloaded)",
        pins.font.len(),
        dest.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::Pins;

    const PINS: &str = r#"
base_url = "https://example.org/fonts/"

[[font]]
file = "NotoSansKR.ttf"
path = "ofl/notosanskr/NotoSansKR[wght].ttf"
sha256 = "194018e6b2b293a7964f037b25c0249ce1418bc9ab3c971060a03aa57861e252"
"#;

    #[test]
    fn parses_pins_and_escapes_brackets() {
        let pins = Pins::parse(PINS).unwrap();
        assert_eq!(
            pins.url(&pins.font[0]),
            "https://example.org/fonts/ofl/notosanskr/NotoSansKR%5Bwght%5D.ttf"
        );
    }

    #[test]
    fn rejects_bad_pins() {
        assert!(Pins::parse(&PINS.replace("194018e6", "194018E6")).is_err());
        assert!(Pins::parse(&PINS.replace("NotoSansKR.ttf\"", "../x.ttf\"")).is_err());
        assert!(Pins::parse(&PINS.replace("https://", "http://")).is_err());
    }

    #[test]
    fn repository_pins_are_valid() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let text = std::fs::read_to_string(root.join("tests/fonts/fonts.toml")).unwrap();
        let pins = Pins::parse(&text).unwrap();
        assert!(pins.font.len() >= 5);
    }
}
