//! The OCR model files and their pinned checksums.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::error::OcrError;

/// One bundled model file with the size and SHA-256 pinned in `models.toml`
/// (a test keeps the two in sync).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PinnedFile {
    /// File name inside the model folder.
    pub file: &'static str,
    /// Size in bytes.
    pub size: u64,
    /// SHA-256, lowercase hex.
    pub sha256: &'static str,
}

/// The PP-OCRv5 mobile text detection model.
pub const DET_MODEL: PinnedFile = PinnedFile {
    file: "ch_PP-OCRv5_det_mobile.onnx",
    size: 4_819_576,
    sha256: "4d97c44a20d30a81aad087d6a396b08f786c4635742afc391f6621f5c6ae78ae",
};

/// The PP-OCRv5 mobile text recognition model.
pub const REC_MODEL: PinnedFile = PinnedFile {
    file: "ch_PP-OCRv5_rec_mobile.onnx",
    size: 16_631_306,
    sha256: "5825fc7ebf84ae7a412be049820b4d86d77620f204a041697b0494669b1742c5",
};

/// The character list of [`REC_MODEL`].
pub const REC_DICT: PinnedFile = PinnedFile {
    file: "ppocrv5_dict.txt",
    size: 74_012,
    sha256: "d1979e9f794c464c0d2e0b70a7fe14dd978e9dc644c0e71f14158cdf8342af1b",
};

/// The models of one OCR configuration, held in memory so that every
/// worker can create its sessions without reading the files again.
#[derive(Debug, Clone)]
pub struct ModelSet {
    det: Arc<[u8]>,
    rec: Arc<[u8]>,
    charset: Arc<[String]>,
}

impl ModelSet {
    /// Loads the bundled models from `dir` and checks every file against
    /// its pinned size and SHA-256.
    ///
    /// # Errors
    ///
    /// [`OcrError::ModelMissing`] or [`OcrError::ModelCorrupt`] when a file
    /// is absent or differs from the pin ("reinstall to repair").
    pub fn load(dir: &Path) -> Result<Self, OcrError> {
        let det = read_pinned(dir, &DET_MODEL)?;
        let rec = read_pinned(dir, &REC_MODEL)?;
        let dict = read_pinned(dir, &REC_DICT)?;
        let charset = parse_charset(&dir.join(REC_DICT.file), &dict)?;
        Ok(Self {
            det: det.into(),
            rec: rec.into(),
            charset: charset.into(),
        })
    }

    /// Loads an unpinned combination of files, for benchmarking other
    /// models (`models/candidates/`). No checksum is checked.
    ///
    /// # Errors
    ///
    /// [`OcrError::ModelMissing`] or [`OcrError::Io`] when a file cannot be
    /// read, [`OcrError::ModelCorrupt`] when the character list is empty.
    pub fn from_files(det: &Path, rec: &Path, dict: &Path) -> Result<Self, OcrError> {
        let det_bytes = read(det)?;
        let rec_bytes = read(rec)?;
        let dict_bytes = read(dict)?;
        let charset = parse_charset(dict, &dict_bytes)?;
        Ok(Self {
            det: det_bytes.into(),
            rec: rec_bytes.into(),
            charset: charset.into(),
        })
    }

    /// The detection model (ONNX bytes).
    pub(crate) fn det(&self) -> &[u8] {
        &self.det
    }

    /// The recognition model (ONNX bytes).
    pub(crate) fn rec(&self) -> &[u8] {
        &self.rec
    }

    /// The recogniser's characters; class `i + 1` of the model output is
    /// `charset[i]` (class 0 is the CTC blank).
    pub(crate) fn charset(&self) -> &Arc<[String]> {
        &self.charset
    }
}

/// Reads a file, mapping "not found" to [`OcrError::ModelMissing`].
fn read(path: &Path) -> Result<Vec<u8>, OcrError> {
    fs::read(path).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            OcrError::ModelMissing {
                path: path.to_path_buf(),
            }
        } else {
            OcrError::Io {
                path: path.to_path_buf(),
                source,
            }
        }
    })
}

/// Reads a pinned file and checks its size and SHA-256.
fn read_pinned(dir: &Path, pin: &PinnedFile) -> Result<Vec<u8>, OcrError> {
    let path: PathBuf = dir.join(pin.file);
    let bytes = read(&path)?;
    if bytes.len() as u64 != pin.size {
        return Err(OcrError::ModelCorrupt {
            path,
            reason: format!("{} bytes instead of {}", bytes.len(), pin.size),
        });
    }
    let digest = Sha256::digest(&bytes)
        .iter()
        .fold(String::with_capacity(64), |mut hex, b| {
            // Writing into a String cannot fail.
            let _ = write!(hex, "{b:02x}");
            hex
        });
    if digest != pin.sha256 {
        return Err(OcrError::ModelCorrupt {
            path,
            reason: "the SHA-256 does not match the pinned value".to_owned(),
        });
    }
    Ok(bytes)
}

/// Parses a PaddleOCR character list: one character (or short sequence)
/// per line, in class order. The space class that PaddleOCR appends after
/// the list (`use_space_char`) is added here.
fn parse_charset(path: &Path, bytes: &[u8]) -> Result<Vec<String>, OcrError> {
    let corrupt = |reason: &str| OcrError::ModelCorrupt {
        path: path.to_path_buf(),
        reason: reason.to_owned(),
    };
    let text = std::str::from_utf8(bytes).map_err(|_| corrupt("not UTF-8"))?;
    let mut charset: Vec<String> = text
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line).to_owned())
        .collect();
    // A trailing newline ends the last line; it is not an empty class.
    if charset.last().is_some_and(String::is_empty) {
        charset.pop();
    }
    if charset.is_empty() || charset.iter().any(String::is_empty) {
        return Err(corrupt("empty character list or empty line"));
    }
    charset.push(" ".to_owned());
    Ok(charset)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{DET_MODEL, ModelSet, REC_DICT, REC_MODEL, parse_charset};
    use crate::error::OcrError;

    #[test]
    fn parses_character_lists() {
        let path = Path::new("dict.txt");
        assert_eq!(
            parse_charset(path, b"a\nb\r\nc\n").unwrap(),
            ["a", "b", "c", " "]
        );
        assert!(parse_charset(path, b"a\n\nb\n").is_err());
        assert!(parse_charset(path, b"").is_err());
        assert!(parse_charset(path, &[0xff, b'\n']).is_err());
    }

    #[test]
    fn missing_models_are_reported() {
        let dir = std::env::temp_dir().join("blurrd-no-models-here");
        let error = ModelSet::load(&dir).unwrap_err();
        assert!(matches!(error, OcrError::ModelMissing { .. }), "{error}");
    }

    #[test]
    fn corrupt_models_are_rejected() {
        let dir =
            std::env::temp_dir().join(format!("blurrd-corrupt-models-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(DET_MODEL.file), vec![0u8; DET_MODEL.size as usize]).unwrap();
        let error = ModelSet::load(&dir).unwrap_err();
        assert!(matches!(error, OcrError::ModelCorrupt { .. }), "{error}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The pins compiled into this crate must equal the bundled entries of
    /// `models.toml`, which `cargo xtask fetch-models` downloads.
    #[test]
    fn pins_match_models_toml() {
        #[derive(serde::Deserialize)]
        struct Pins {
            model: Vec<Model>,
        }
        #[derive(serde::Deserialize)]
        struct Model {
            file: String,
            role: String,
            bundled: bool,
            size: u64,
            sha256: String,
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let text = std::fs::read_to_string(root.join("models.toml")).unwrap();
        let pins: Pins = toml::from_str(&text).unwrap();
        for (role, pin) in [("det", DET_MODEL), ("rec", REC_MODEL), ("dict", REC_DICT)] {
            let entry = pins
                .model
                .iter()
                .find(|m| m.bundled && m.role == role)
                .unwrap();
            assert_eq!(
                (entry.file.as_str(), entry.size, entry.sha256.as_str()),
                (pin.file, pin.size, pin.sha256),
                "{role}"
            );
        }
    }
}
