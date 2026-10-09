//! Loading ONNX Runtime at run time.
//!
//! blurrd links ONNX Runtime dynamically through the ort crate's
//! `load-dynamic` feature: nothing is loaded until OCR is first needed, so
//! the app starts, analyses and renders on a machine where the library is
//! missing or broken; OCR is then reported as unavailable. A process can
//! hold only one ONNX Runtime, so the first successful [`OrtRuntime::load`]
//! decides which file it is.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use crate::error::OcrError;

/// File name of the DirectML redistributable next to `onnxruntime.dll`.
const DIRECTML_DLL: &str = "DirectML.dll";

/// The loaded runtime of this process, set by the first successful load.
static LOADED: Mutex<Option<Loaded>> = Mutex::new(None);

/// What [`LOADED`] remembers about the loaded library.
#[derive(Debug, Clone)]
struct Loaded {
    path: PathBuf,
    commit: String,
    directml: bool,
}

/// Proof that ONNX Runtime is loaded in this process.
///
/// Every ort call needs a loaded runtime (ort panics otherwise), so the
/// engine can only be created from an `OrtRuntime`.
#[derive(Debug, Clone)]
pub struct OrtRuntime {
    loaded: Loaded,
}

impl OrtRuntime {
    /// Loads `onnxruntime.dll` from `dylib` (an absolute path).
    ///
    /// The DirectML redistributable next to it is loaded first, so the
    /// runtime uses the DirectML version blurrd ships rather than the older
    /// copy in the Windows system folder. Loading again with the same path
    /// returns the runtime that is already loaded.
    ///
    /// # Errors
    ///
    /// - [`OcrError::RuntimeUnavailable`] if the file is missing, cannot be
    ///   loaded or is older than the C API level blurrd needs
    ///   ([`ort::MINOR_VERSION`]).
    /// - [`OcrError::RuntimeConflict`] if another file is already loaded.
    pub fn load(dylib: &Path) -> Result<Self, OcrError> {
        let mut loaded = LOADED.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(existing) = loaded.as_ref() {
            return if same_file(&existing.path, dylib) {
                Ok(Self {
                    loaded: existing.clone(),
                })
            } else {
                Err(OcrError::RuntimeConflict {
                    loaded: existing.path.clone(),
                    requested: dylib.to_path_buf(),
                })
            };
        }
        let unavailable = |reason: String| OcrError::RuntimeUnavailable {
            path: dylib.to_path_buf(),
            reason,
        };
        if !dylib.is_absolute() {
            return Err(unavailable("the path must be absolute".to_owned()));
        }
        if !dylib.is_file() {
            return Err(unavailable("the file does not exist".to_owned()));
        }
        if let Some(directml) = dylib.parent().map(|dir| dir.join(DIRECTML_DLL))
            && directml.is_file()
        {
            ort::util::preload_dylib(&directml)
                .map_err(|e| unavailable(format!("loading {DIRECTML_DLL} failed: {e}")))?;
        }
        let committed = ort::init_from(dylib)
            .map_err(|e| unavailable(e.to_string()))?
            .with_name("blurrd")
            .with_telemetry(false)
            .commit();
        if !committed {
            tracing::debug!("an ONNX Runtime environment already existed");
        }
        let info = Loaded {
            path: dylib.to_path_buf(),
            commit: commit_of(ort::info()),
            directml: ort::ep::directml::api().is_some(),
        };
        tracing::info!(
            commit = %info.commit,
            directml = info.directml,
            "loaded ONNX Runtime"
        );
        *loaded = Some(info.clone());
        Ok(Self { loaded: info })
    }

    /// The loaded library.
    pub fn path(&self) -> &Path {
        &self.loaded.path
    }

    /// The ONNX Runtime source commit the library was built from (the
    /// abbreviated hash in its build information, or `unknown`). Official
    /// builds report no version number, so this identifies the release;
    /// `third_party/ort.toml` records which commit the pinned version is.
    pub fn commit(&self) -> &str {
        &self.loaded.commit
    }

    /// Whether this build of ONNX Runtime contains the DirectML execution
    /// provider.
    pub fn has_directml(&self) -> bool {
        self.loaded.directml
    }

    /// The build information of the loaded library (branch, commit and
    /// build type), as reported by ONNX Runtime.
    pub fn build_info(&self) -> &'static str {
        ort::info()
    }
}

/// Whether two paths name the same file (compared after canonicalising,
/// falling back to the paths themselves).
fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Extracts the commit from ONNX Runtime's build string
/// (`ORT Build Info: git-branch=HEAD, git-commit-id=2d924974ef, ...`), or
/// `unknown`.
fn commit_of(build_info: &str) -> String {
    build_info
        .split([',', ' '])
        .find_map(|part| part.strip_prefix("git-commit-id="))
        .filter(|commit| !commit.is_empty())
        .unwrap_or("unknown")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{OrtRuntime, commit_of};
    use crate::error::OcrError;

    #[test]
    fn reads_the_commit_from_the_build_string() {
        assert_eq!(
            commit_of(
                "ORT Build Info: git-branch=HEAD, git-commit-id=2d924974ef, build type=RelWithDebInfo"
            ),
            "2d924974ef"
        );
        assert_eq!(commit_of("something else"), "unknown");
    }

    #[test]
    fn missing_runtime_is_reported_not_fatal() {
        let path = std::env::temp_dir()
            .join("blurrd-no-such-dir")
            .join("onnxruntime.dll");
        let error = OrtRuntime::load(&path).unwrap_err();
        assert!(
            matches!(error, OcrError::RuntimeUnavailable { .. }),
            "{error}"
        );
        let relative = OrtRuntime::load(Path::new("onnxruntime.dll")).unwrap_err();
        assert!(matches!(relative, OcrError::RuntimeUnavailable { .. }));
    }
}
