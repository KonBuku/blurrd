//! What the installer and the CLI archive ship next to the executable.
//!
//! `stage-bundle` collects the files into `target/installer-files/`, which
//! `apps/desktop/src-tauri/tauri.conf.json` bundles as resources: Windows
//! installs them into the folder of `blurrd.exe`, where the executable
//! loads the FFmpeg DLLs from, and where blurrd looks for
//! `onnxruntime.dll` and the `models` folder. The folder holds:
//!
//! - the FFmpeg DLLs blurrd links (avcodec, avformat, avutil, swresample)
//!   and the DLLs they import from the same build;
//! - ONNX Runtime and DirectML;
//! - the Microsoft Visual C++ runtime DLLs those DLLs import, taken from
//!   the Visual Studio installation (app-local deployment), so blurrd runs
//!   on a PC without the Visual C++ Redistributable;
//! - the bundled OCR models of `models.toml` in `models/`, checked against
//!   their pinned size and SHA-256;
//! - `LICENSE`, `NOTICE`, `THIRD-PARTY-NOTICES.md` and the licence files of
//!   FFmpeg, ONNX Runtime and DirectML in `licenses/`.
//!
//! `check-bundle` proves that every DLL the shipped files import is shipped
//! or part of Windows, `check-ffmpeg` that the FFmpeg DLLs are the LGPL
//! build, and `package-cli` zips the CLI with the same files.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail, ensure};

use crate::ffmpeg::{self, target_dir};
use crate::models;

/// The FFmpeg libraries blurrd links (the DLL names carry the major
/// version, for example `avcodec-63.dll`).
const LINKED_FFMPEG: &[&str] = &["avcodec", "avformat", "avutil", "swresample"];

/// Configure flags the shipped FFmpeg must not have (plan 13.2).
const FORBIDDEN_FLAGS: &[&str] = &[
    "--enable-gpl",
    "--enable-nonfree",
    "--enable-version3",
    "--enable-libopenh264",
];

/// Configure flags the shipped FFmpeg must have (the same list as
/// `third_party/ffmpeg-build/build.ps1`).
const REQUIRED_FLAGS: &[&str] = &[
    "--enable-mediafoundation",
    "--enable-ffnvcodec",
    "--enable-nvenc",
    "--enable-nvdec",
    "--enable-cuda",
    "--enable-amf",
    "--enable-libvpl",
    "--enable-libdav1d",
    "--enable-d3d11va",
    "--enable-shared",
];

/// The licence string every FFmpeg library embeds (`av*_license()`).
const LGPL: &str = "license: LGPL version 2.1 or later";

/// The staging folder `target/installer-files/` of `root`. The path is
/// fixed (not below `CARGO_TARGET_DIR`) because `tauri.conf.json` names it.
pub(crate) fn staging_dir(root: &Path) -> PathBuf {
    root.join("target").join("installer-files")
}

/// Collects everything the installer ships next to `blurrd.exe` into a
/// fresh `target/installer-files/`.
pub(crate) fn stage(root: &Path) -> Result<()> {
    let ffmpeg_dir = ffmpeg::installed(root)?;
    let bin = ffmpeg_dir.join("bin");
    let ort = root.join("third_party").join("ort");
    ensure!(
        ort.join("onnxruntime.dll").is_file(),
        "{} has no onnxruntime.dll (run `cargo xtask fetch-ort` first)",
        ort.display()
    );
    let models = models::bundled_files(root)?;

    let out = staging_dir(root);
    if out.exists() {
        fs::remove_dir_all(&out).with_context(|| format!("removing {}", out.display()))?;
    }
    fs::create_dir_all(out.join("models"))
        .with_context(|| format!("creating {}", out.display()))?;

    let ffmpeg_dlls = ffmpeg_closure(&bin)?;
    let mut shipped = Vec::new();
    for name in &ffmpeg_dlls {
        shipped.push(copy(&bin.join(name), &out.join(name))?);
    }
    // ONNX Runtime and DirectML (loaded at run time, not imported).
    for name in dll_names(&ort)? {
        shipped.push(copy(&ort.join(&name), &out.join(&name))?);
    }
    let crt_count = stage_vc_runtime(root, &shipped, &out)?;
    for model in &models {
        let name = model
            .file_name()
            .context("model path without a file name")?;
        copy(model, &out.join("models").join(name))?;
    }
    stage_licenses(root, &ffmpeg_dir, &ort, &out)?;

    let (files, bytes) = folder_size(&out)?;
    eprintln!(
        "staged {} FFmpeg, {} ONNX Runtime and {crt_count} Visual C++ runtime DLLs, \
         {} models and the licences into {} ({files} files, {:.1} MB)",
        ffmpeg_dlls.len(),
        shipped.len() - ffmpeg_dlls.len(),
        models.len(),
        out.display(),
        bytes as f64 / 1e6,
    );
    check(root, &[])
}

/// The FFmpeg DLLs blurrd links and every DLL of the same build they
/// import, directly or indirectly.
fn ffmpeg_closure(bin: &Path) -> Result<BTreeSet<String>> {
    let available = dll_names(bin)?;
    let mut pending: Vec<String> = available
        .iter()
        .filter(|name| LINKED_FFMPEG.contains(&name.split('-').next().unwrap_or_default()))
        .cloned()
        .collect();
    ensure!(
        pending.len() == LINKED_FFMPEG.len(),
        "{} does not hold one DLL for each of {LINKED_FFMPEG:?}",
        bin.display()
    );
    let mut closure = BTreeSet::new();
    while let Some(name) = pending.pop() {
        if !closure.insert(name.clone()) {
            continue;
        }
        for import in pe_imports(&bin.join(&name))? {
            if let Some(found) = available.iter().find(|a| a.eq_ignore_ascii_case(&import)) {
                pending.push(found.clone());
            }
        }
    }
    Ok(closure)
}

/// Copies the Visual C++ runtime DLLs that any of `shipped` imports into
/// `out` and returns how many.
fn stage_vc_runtime(root: &Path, shipped: &[PathBuf], out: &Path) -> Result<usize> {
    let mut crt = BTreeSet::new();
    for dll in shipped {
        for import in pe_imports(dll)? {
            if is_vc_runtime(&import) {
                crt.insert(import.to_ascii_lowercase());
            }
        }
    }
    if crt.is_empty() {
        return Ok(0);
    }
    // THIRD-PARTY-NOTICES.md must list exactly what ships.
    let notice_path = root
        .join("scripts")
        .join("license-texts")
        .join("msvc-runtime.txt");
    let notice = fs::read_to_string(&notice_path)
        .with_context(|| format!("reading {}", notice_path.display()))?;
    for name in &crt {
        ensure!(
            notice.contains(name.as_str()),
            "{} does not name {name}; add it and run `pnpm notices`",
            notice_path.display()
        );
    }
    let redist = vc_redist_dir()?;
    for name in &crt {
        let source = redist.join(name);
        ensure!(
            source.is_file(),
            "{name} is not in the Visual C++ redistributable folder {}",
            redist.display()
        );
        copy(&source, &out.join(name))?;
    }
    eprintln!("Visual C++ runtime from {}", redist.display());
    Ok(crt.len())
}

/// Copies the licences and notices into `out`: blurrd's own at the top,
/// FFmpeg's (with its build information and where its source is) in
/// `licenses/ffmpeg/`, ONNX Runtime's and DirectML's in `licenses/`.
fn stage_licenses(root: &Path, ffmpeg_dir: &Path, ort: &Path, out: &Path) -> Result<()> {
    for name in ["LICENSE", "NOTICE", "THIRD-PARTY-NOTICES.md"] {
        copy(&root.join(name), &out.join(name))?;
    }
    let licenses = out.join("licenses");
    let ffmpeg_licenses = licenses.join("ffmpeg");
    fs::create_dir_all(&ffmpeg_licenses)
        .with_context(|| format!("creating {}", ffmpeg_licenses.display()))?;
    for entry in fs::read_dir(ffmpeg_dir.join("licenses"))? {
        let entry = entry?;
        let port = entry.file_name();
        let copyright = entry.path().join("copyright");
        if copyright.is_file() {
            let name = format!("{}.txt", port.to_string_lossy());
            copy(&copyright, &ffmpeg_licenses.join(name))?;
        }
    }
    copy(
        &ffmpeg_dir.join("BUILDINFO.txt"),
        &ffmpeg_licenses.join("BUILDINFO.txt"),
    )?;
    let pin = ffmpeg::Pin::load(root)?;
    fs::write(
        ffmpeg_licenses.join("SOURCE.txt"),
        format!(
            "The FFmpeg DLLs next to blurrd.exe are FFmpeg {} (blurrd build revision {}),\n\
             licensed under the GNU Lesser General Public License version 2.1 or later\n\
             and linked dynamically: they can be replaced with a compatible build.\n\
             \n\
             The corresponding source code (the FFmpeg source tarball, the build recipe\n\
             and the patches) is published at\n\
             {}\n\
             SHA-256 {}\n",
            pin.version,
            pin.revision,
            pin.public_url(&pin.source_asset),
            pin.source_sha256,
        ),
    )
    .context("writing the FFmpeg source note")?;
    for entry in fs::read_dir(ort.join("licenses"))
        .with_context(|| format!("reading {}", ort.join("licenses").display()))?
    {
        let path = entry?.path();
        if path.is_file()
            && let Some(name) = path.file_name()
        {
            copy(&path, &licenses.join(name))?;
        }
    }
    Ok(())
}

/// Checks that every executable and DLL in the staging folder, and the
/// executables in `exes`, import only DLLs that are staged or part of
/// Windows (System32, except the Visual C++ runtime, which Windows does
/// not guarantee).
pub(crate) fn check(root: &Path, exes: &[PathBuf]) -> Result<()> {
    let out = staging_dir(root);
    ensure!(
        out.is_dir(),
        "{} does not exist (run `cargo xtask stage-bundle` first)",
        out.display()
    );
    let staged = dll_names(&out)?;
    let system = system_dir()?;
    let mut binaries: Vec<PathBuf> = staged.iter().map(|name| out.join(name)).collect();
    binaries.extend(exes.iter().cloned());
    let mut missing = Vec::new();
    for binary in &binaries {
        for import in pe_imports(binary)? {
            let lower = import.to_ascii_lowercase();
            let ok = staged.iter().any(|s| s.eq_ignore_ascii_case(&import))
                || lower.starts_with("api-ms-win-")
                || lower.starts_with("ext-ms-")
                || (!is_vc_runtime(&lower) && system.join(&import).is_file());
            if !ok {
                missing.push(format!("{} imports {import}", binary.display()));
            }
        }
    }
    if !missing.is_empty() {
        bail!(
            "DLLs that are neither shipped nor part of Windows:\n  {}",
            missing.join("\n  ")
        );
    }
    eprintln!(
        "{} binaries import only shipped DLLs and Windows system DLLs",
        binaries.len()
    );
    Ok(())
}

/// Checks the shipped FFmpeg DLLs themselves: each library embeds its
/// configure line (what `ffprobe -buildconf` prints) and its licence.
pub(crate) fn check_ffmpeg(root: &Path) -> Result<()> {
    let bin = ffmpeg::installed(root)?.join("bin");
    let mut failures = Vec::new();
    let mut checked = 0;
    for name in dll_names(&bin)? {
        let stem = name.split('-').next().unwrap_or_default();
        if !(stem.starts_with("av") || stem.starts_with("sw")) {
            continue;
        }
        checked += 1;
        let bytes = fs::read(bin.join(&name)).with_context(|| format!("reading {name}"))?;
        let Some(config) = embedded_configuration(&bytes) else {
            failures.push(format!("{name}: no embedded configure line"));
            continue;
        };
        for flag in FORBIDDEN_FLAGS {
            if config.split_whitespace().any(|word| word == *flag) {
                failures.push(format!("{name}: forbidden configure flag {flag}"));
            }
        }
        for flag in REQUIRED_FLAGS {
            if !config.split_whitespace().any(|word| word == *flag) {
                failures.push(format!("{name}: missing configure flag {flag}"));
            }
        }
        if !contains(&bytes, LGPL.as_bytes()) {
            failures.push(format!("{name}: does not report \"{LGPL}\""));
        }
    }
    ensure!(
        checked >= LINKED_FFMPEG.len(),
        "found only {checked} FFmpeg libraries in {}",
        bin.display()
    );
    if !failures.is_empty() {
        bail!(
            "the FFmpeg build is not the LGPL build blurrd ships:\n  {}",
            failures.join("\n  ")
        );
    }
    eprintln!(
        "{checked} FFmpeg libraries: LGPL-2.1-or-later, no gpl/nonfree/version3/openh264, \
         all required components"
    );
    Ok(())
}

/// Zips the CLI (`target/dist-cli/blurrd.exe`, built with `cargo build
/// --profile dist-cli -p blurrd-cli`) with the staged files into
/// `target/dist/blurrd-cli_<version>_x64.zip`.
pub(crate) fn package_cli(root: &Path) -> Result<PathBuf> {
    let exe = target_dir(root).join("dist-cli").join("blurrd.exe");
    ensure!(
        exe.is_file(),
        "{} does not exist (run `cargo build --profile dist-cli -p blurrd-cli --locked`)",
        exe.display()
    );
    check(root, std::slice::from_ref(&exe))?;
    let staged = staging_dir(root);
    let name = format!("blurrd-cli_{}_x64", env!("CARGO_PKG_VERSION"));
    let dist = target_dir(root).join("dist");
    fs::create_dir_all(&dist).with_context(|| format!("creating {}", dist.display()))?;
    let path = dist.join(format!("{name}.zip"));
    let file = fs::File::create(&path).with_context(|| format!("creating {}", path.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    // A fixed time stamp (1980-01-01) keeps the archive reproducible.
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default());
    let mut entries = vec![(exe.clone(), "blurrd.exe".to_owned())];
    for file in files_below(&staged)? {
        let relative = file
            .strip_prefix(&staged)?
            .to_string_lossy()
            .replace('\\', "/");
        entries.push((file, relative));
    }
    entries.sort_by(|a, b| a.1.cmp(&b.1));
    for (source, relative) in &entries {
        zip.start_file(format!("{name}/{relative}"), options)?;
        let mut reader =
            fs::File::open(source).with_context(|| format!("opening {}", source.display()))?;
        io::copy(&mut reader, &mut zip).with_context(|| format!("zipping {relative}"))?;
    }
    zip.finish()?;
    let size = fs::metadata(&path)?.len();
    eprintln!(
        "wrote {} ({} files, {:.1} MB)",
        path.display(),
        entries.len(),
        size as f64 / 1e6
    );
    Ok(path)
}

/// Copies `from` to `to` and returns `to`.
fn copy(from: &Path, to: &Path) -> Result<PathBuf> {
    fs::copy(from, to)
        .with_context(|| format!("copying {} to {}", from.display(), to.display()))?;
    Ok(to.to_path_buf())
}

/// The names of the DLLs directly inside `dir`, sorted.
fn dll_names(dir: &Path) -> Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.path().is_file() && name.to_ascii_lowercase().ends_with(".dll") {
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}

/// Every file below `dir`, recursively.
fn files_below(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut dirs = vec![dir.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))? {
            let path = entry?.path();
            if path.is_dir() {
                dirs.push(path);
            } else {
                files.push(path);
            }
        }
    }
    Ok(files)
}

/// Number of files below `dir` and their total size in bytes.
fn folder_size(dir: &Path) -> Result<(usize, u64)> {
    let files = files_below(dir)?;
    let mut bytes = 0;
    for file in &files {
        bytes += fs::metadata(file)?.len();
    }
    Ok((files.len(), bytes))
}

/// Whether `name` is a Visual C++ runtime DLL (redistributable, not part
/// of Windows).
fn is_vc_runtime(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [
        "vcruntime140",
        "msvcp140",
        "concrt140",
        "vccorlib140",
        "vcomp140",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
}

/// The Windows system folder (`%SystemRoot%\System32`).
fn system_dir() -> Result<PathBuf> {
    let root = std::env::var_os("SystemRoot").context("SystemRoot is not set")?;
    Ok(PathBuf::from(root).join("System32"))
}

/// The x64 Visual C++ runtime folder of the newest Visual Studio with the
/// C++ tools (`VC\Redist\MSVC\<version>\x64\Microsoft.VC<toolset>.CRT`).
fn vc_redist_dir() -> Result<PathBuf> {
    let program_files = std::env::var_os("ProgramFiles(x86)")
        .or_else(|| std::env::var_os("ProgramFiles"))
        .context("ProgramFiles is not set")?;
    let vswhere = PathBuf::from(program_files)
        .join("Microsoft Visual Studio")
        .join("Installer")
        .join("vswhere.exe");
    let output = Command::new(&vswhere)
        .args([
            "-latest",
            "-products",
            "*",
            "-requires",
            "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
            "-property",
            "installationPath",
        ])
        .output()
        .with_context(|| format!("running {}", vswhere.display()))?;
    ensure!(
        output.status.success(),
        "vswhere failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let install = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    ensure!(
        install.is_dir(),
        "no Visual Studio installation with the C++ tools found"
    );
    let version_file = install
        .join("VC")
        .join("Auxiliary")
        .join("Build")
        .join("Microsoft.VCRedistVersion.default.txt");
    let version = fs::read_to_string(&version_file)
        .with_context(|| format!("reading {}", version_file.display()))?;
    let x64 = install
        .join("VC")
        .join("Redist")
        .join("MSVC")
        .join(version.trim())
        .join("x64");
    for entry in fs::read_dir(&x64).with_context(|| format!("reading {}", x64.display()))? {
        let path = entry?.path();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if path.is_dir()
            && name.starts_with("Microsoft.VC")
            && path.extension().is_some_and(|ext| ext == "CRT")
        {
            return Ok(path);
        }
    }
    bail!("no Microsoft.VC*.CRT folder in {}", x64.display())
}

/// Whether `haystack` contains `needle`.
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// The configure line FFmpeg embeds in each library (from `--prefix=` to
/// the terminating NUL), if there is one.
fn embedded_configuration(bytes: &[u8]) -> Option<String> {
    let start = bytes.windows(9).position(|w| w == b"--prefix=")?;
    let len = bytes[start..].iter().position(|&b| b == 0)?;
    Some(String::from_utf8_lossy(&bytes[start..start + len]).into_owned())
}

/// The DLL names a PE file imports (import table and delay-load table).
fn pe_imports(path: &Path) -> Result<Vec<String>> {
    let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    parse_imports(&bytes).with_context(|| format!("reading the imports of {}", path.display()))
}

/// Parses the import and delay-load tables of a PE32+ image.
fn parse_imports(bytes: &[u8]) -> Result<Vec<String>> {
    let pe = read_u32(bytes, 0x3c)? as usize;
    ensure!(
        bytes.get(pe..pe + 4) == Some(b"PE\0\0".as_slice()),
        "not a PE file"
    );
    let coff = pe + 4;
    let sections = usize::from(read_u16(bytes, coff + 2)?);
    let optional_size = usize::from(read_u16(bytes, coff + 16)?);
    let optional = coff + 20;
    ensure!(read_u16(bytes, optional)? == 0x20b, "not a 64-bit image");
    let directory_count = read_u32(bytes, optional + 108)? as usize;
    let directory = |index: usize| -> Result<u32> {
        if index < directory_count {
            read_u32(bytes, optional + 112 + 8 * index)
        } else {
            Ok(0)
        }
    };
    let table = optional + optional_size;
    let mut ranges = Vec::with_capacity(sections);
    for i in 0..sections {
        let s = table + 40 * i;
        let virtual_size = read_u32(bytes, s + 8)?;
        let virtual_address = read_u32(bytes, s + 12)?;
        let raw_size = read_u32(bytes, s + 16)?;
        let raw_pointer = read_u32(bytes, s + 20)?;
        ranges.push((virtual_address, virtual_size.max(raw_size), raw_pointer));
    }
    let offset = |rva: u32| -> Result<usize> {
        ranges
            .iter()
            .find(|(va, size, _)| rva >= *va && rva - va < *size)
            .map(|(va, _, raw)| (rva - va + raw) as usize)
            .context("address outside every section")
    };
    let name_at = |rva: u32| -> Result<String> {
        let start = offset(rva)?;
        let len = bytes
            .get(start..)
            .and_then(|rest| rest.iter().position(|&b| b == 0))
            .context("unterminated DLL name")?;
        Ok(String::from_utf8_lossy(&bytes[start..start + len]).into_owned())
    };
    let mut names = Vec::new();
    // Import descriptors: 20 bytes each, the name address at +12.
    let imports = directory(1)?;
    if imports != 0 {
        let mut at = offset(imports)?;
        loop {
            let name = read_u32(bytes, at + 12)?;
            if name == 0 {
                break;
            }
            names.push(name_at(name)?);
            at += 20;
        }
    }
    // Delay-load descriptors: 32 bytes each, attributes at +0 (bit 0: the
    // fields are addresses relative to the image), the name at +4.
    let delayed = directory(13)?;
    if delayed != 0 {
        let mut at = offset(delayed)?;
        loop {
            let attributes = read_u32(bytes, at)?;
            let name = read_u32(bytes, at + 4)?;
            if name == 0 {
                break;
            }
            ensure!(attributes & 1 == 1, "old-style delay-load descriptor");
            names.push(name_at(name)?);
            at += 32;
        }
    }
    Ok(names)
}

fn read_u16(bytes: &[u8], at: usize) -> Result<u16> {
    let b = bytes.get(at..at + 2).context("truncated PE file")?;
    Ok(u16::from_le_bytes([b[0], b[1]]))
}

fn read_u32(bytes: &[u8], at: usize) -> Result<u32> {
    let b = bytes.get(at..at + 4).context("truncated PE file")?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

#[cfg(test)]
mod tests {
    use super::{embedded_configuration, is_vc_runtime, parse_imports};

    #[test]
    fn reads_the_imports_of_a_system_dll() {
        let Some(root) = std::env::var_os("SystemRoot") else {
            return;
        };
        let dll = std::path::PathBuf::from(root)
            .join("System32")
            .join("dxgi.dll");
        let bytes = std::fs::read(dll).unwrap();
        let imports = parse_imports(&bytes).unwrap();
        assert!(imports.iter().any(|name| {
            name.eq_ignore_ascii_case("kernel32.dll")
                || name.to_ascii_lowercase().starts_with("api-ms-win-")
        }));
    }

    #[test]
    fn rejects_files_that_are_not_images() {
        assert!(parse_imports(b"MZ").is_err());
        assert!(parse_imports(&[0_u8; 256]).is_err());
    }

    #[test]
    fn finds_the_embedded_configure_line() {
        let bytes = b"xx\0--prefix=/x --enable-gpl --disable-doc\0yy";
        assert_eq!(
            embedded_configuration(bytes).as_deref(),
            Some("--prefix=/x --enable-gpl --disable-doc")
        );
        assert_eq!(embedded_configuration(b"nothing here"), None);
    }

    #[test]
    fn knows_the_visual_cpp_runtime() {
        assert!(is_vc_runtime("VCRUNTIME140.dll"));
        assert!(is_vc_runtime("msvcp140_1.dll"));
        assert!(!is_vc_runtime("kernel32.dll"));
        assert!(!is_vc_runtime("ucrtbase.dll"));
    }
}
