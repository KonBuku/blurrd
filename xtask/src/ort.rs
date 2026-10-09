//! The pinned ONNX Runtime build: download, verify and install it.
//!
//! `third_party/ort.toml` pins NuGet packages (the DirectML build of ONNX
//! Runtime and the DirectML redistributable) by version and SHA-256 and
//! lists the files to take from each. They are copied into
//! `third_party/ort/` (gitignored), where blurrd-ocr loads `onnxruntime.dll`
//! from at run time (`ORT_DYLIB_PATH` in `.cargo/config.toml`).

use std::fs;
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::ffmpeg::{curl, is_sha256_hex, sha256_file, target_dir, to_hex};

/// Name of the stamp file inside the install folder; holds a digest of the
/// pin it was installed from.
const STAMP: &str = ".blurrd-pin";

/// The pin file `third_party/ort.toml`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pin {
    /// ONNX Runtime version, for example `1.24.4`.
    version: String,
    /// The C API level of that version (the minor version number); the
    /// `api-NN` feature of the ort crate must match it.
    api: u32,
    /// The pinned packages.
    package: Vec<Package>,
}

/// One pinned NuGet package.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Package {
    /// Package id on nuget.org.
    id: String,
    /// Exact package version.
    version: String,
    /// SHA-256 of the `.nupkg`, lowercase hex.
    sha256: String,
    /// License of the package contents (informational).
    license: String,
    /// Files to install from the package.
    files: Vec<FileMap>,
}

/// A file inside a package and where it goes in `third_party/ort/`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileMap {
    /// Path inside the package.
    from: String,
    /// Path below the install folder.
    to: String,
}

impl Pin {
    fn parse(text: &str) -> Result<Self> {
        let pin: Self = toml::from_str(text)?;
        let minor = pin.version.split('.').nth(1).and_then(|m| m.parse().ok());
        ensure!(
            minor == Some(pin.api),
            "`api` ({}) must equal the minor version of `version` ({})",
            pin.api,
            pin.version
        );
        ensure!(!pin.package.is_empty(), "no packages pinned");
        for package in &pin.package {
            ensure!(
                is_sha256_hex(&package.sha256),
                "`sha256` of {} must be 64 lowercase hex digits",
                package.id
            );
            ensure!(!package.license.is_empty(), "{} has no license", package.id);
            for file in &package.files {
                let to = Path::new(&file.to);
                ensure!(
                    !file.to.is_empty()
                        && to.is_relative()
                        && to
                            .components()
                            .all(|c| matches!(c, std::path::Component::Normal(_))),
                    "invalid target path `{}` in {}",
                    file.to,
                    package.id
                );
            }
        }
        Ok(pin)
    }

    /// A digest over the whole pin, stored in the stamp file: any change to
    /// a package or its file list reinstalls.
    fn digest(text: &str) -> String {
        to_hex(&Sha256::digest(text.as_bytes()))
    }
}

impl Package {
    /// The download URL on nuget.org (the flat container API; ids are
    /// lowercase there).
    fn url(&self) -> String {
        let id = self.id.to_lowercase();
        let version = self.version.to_lowercase();
        format!("https://api.nuget.org/v3-flatcontainer/{id}/{version}/{id}.{version}.nupkg")
    }

    fn file_name(&self) -> String {
        format!("{}.{}.nupkg", self.id.to_lowercase(), self.version)
    }
}

/// Makes `third_party/ort/` match the pin.
///
/// Does nothing when the folder already holds the pinned files.
pub(crate) fn fetch(root: &Path) -> Result<()> {
    let pin_path = root.join("third_party").join("ort.toml");
    let text =
        fs::read_to_string(&pin_path).with_context(|| format!("reading {}", pin_path.display()))?;
    let pin = Pin::parse(&text).with_context(|| format!("parsing {}", pin_path.display()))?;
    let digest = Pin::digest(&text);
    let dest = root.join("third_party").join("ort");
    let stamp = dest.join(STAMP);
    if fs::read_to_string(&stamp).unwrap_or_default().trim() == digest {
        eprintln!(
            "ONNX Runtime {} is up to date in {}",
            pin.version,
            dest.display()
        );
        return Ok(());
    }
    let staging = dest.with_extension("partial");
    if staging.exists() {
        fs::remove_dir_all(&staging).with_context(|| format!("removing {}", staging.display()))?;
    }
    for package in &pin.package {
        let archive = download(root, package)?;
        extract(&archive, package, &staging)?;
    }
    fs::write(staging.join(STAMP), format!("{digest}\n"))
        .with_context(|| format!("writing the stamp in {}", staging.display()))?;
    if dest.exists() {
        fs::remove_dir_all(&dest).with_context(|| format!("removing {}", dest.display()))?;
    }
    fs::rename(&staging, &dest)
        .with_context(|| format!("moving {} to {}", staging.display(), dest.display()))?;
    eprintln!(
        "installed ONNX Runtime {} (C API {}) into {}",
        pin.version,
        pin.api,
        dest.display()
    );
    Ok(())
}

/// Downloads a package into `target/xtask/downloads/` (reusing a copy with
/// the right checksum) and returns its path.
fn download(root: &Path, package: &Package) -> Result<PathBuf> {
    let dir = target_dir(root).join("xtask").join("downloads");
    fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let path = dir.join(package.file_name());
    if path.is_file() && sha256_file(&path)? == package.sha256 {
        return Ok(path);
    }
    let partial = dir.join(format!("{}.part", package.file_name()));
    curl(&package.url(), &partial, None, "*/*")?;
    let actual = sha256_file(&partial)?;
    if actual != package.sha256 {
        // Best effort: a stale partial file is overwritten next time.
        let _ = fs::remove_file(&partial);
        anyhow::bail!(
            "{} {} has SHA-256 {actual}, but third_party/ort.toml pins {}",
            package.id,
            package.version,
            package.sha256
        );
    }
    fs::rename(&partial, &path)
        .with_context(|| format!("moving {} to {}", partial.display(), path.display()))?;
    Ok(path)
}

/// Copies the listed files of a package into `dest`.
fn extract(archive: &Path, package: &Package, dest: &Path) -> Result<()> {
    let file = fs::File::open(archive).with_context(|| format!("opening {}", archive.display()))?;
    let mut zip = zip::ZipArchive::new(BufReader::new(file))
        .with_context(|| format!("reading {}", archive.display()))?;
    for map in &package.files {
        let mut entry = zip
            .by_name(&map.from)
            .with_context(|| format!("{} has no file {}", package.id, map.from))?;
        let out = dest.join(&map.to);
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        let mut writer =
            fs::File::create(&out).with_context(|| format!("creating {}", out.display()))?;
        io::copy(&mut entry, &mut writer).with_context(|| format!("writing {}", out.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::Pin;

    const PIN: &str = r#"
version = "1.24.4"
api = 24

[[package]]
id = "Microsoft.ML.OnnxRuntime.DirectML"
version = "1.24.4"
sha256 = "57e9f11b73437bef7a309496135d4c1f96b1a8e9ddba60013fa27bfc1d788681"
license = "MIT"
files = [{ from = "runtimes/win-x64/native/onnxruntime.dll", to = "onnxruntime.dll" }]
"#;

    #[test]
    fn parses_pins_and_builds_nuget_urls() {
        let pin = Pin::parse(PIN).unwrap();
        assert_eq!(
            pin.package[0].url(),
            "https://api.nuget.org/v3-flatcontainer/microsoft.ml.onnxruntime.directml/1.24.4/\
             microsoft.ml.onnxruntime.directml.1.24.4.nupkg"
        );
    }

    #[test]
    fn rejects_bad_pins() {
        assert!(Pin::parse(&PIN.replace("api = 24", "api = 27")).is_err());
        assert!(Pin::parse(&PIN.replace("57e9f11b", "57E9F11B")).is_err());
        assert!(Pin::parse(&PIN.replace("to = \"onnxruntime.dll\"", "to = \"../x.dll\"")).is_err());
        assert!(Pin::parse(&format!("{PIN}extra = 1\n")).is_err());
    }

    #[test]
    fn repository_pin_is_valid() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let text = std::fs::read_to_string(root.join("third_party/ort.toml")).unwrap();
        let pin = Pin::parse(&text).unwrap();
        assert!(
            pin.package
                .iter()
                .flat_map(|p| &p.files)
                .any(|f| f.to == "onnxruntime.dll")
        );
    }
}
