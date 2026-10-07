//! The pinned FFmpeg build: download, verify, unpack and stage its DLLs.
//!
//! `third_party/ffmpeg.toml` pins one archive of a `deps-ffmpeg-*` release
//! (built by `.github/workflows/ffmpeg.yml`) by name and SHA-256. The archive
//! is unpacked into `third_party/ffmpeg/` (gitignored), which
//! `.cargo/config.toml` exposes to `ffmpeg-sys-the-third` as `FFMPEG_DIR`.
//! Test binaries and the CLI load the DLLs at start-up, so they are also
//! copied next to the binaries in `target/`.

use std::fmt::Write as _;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// Name of the stamp file inside the unpacked folder; holds the SHA-256 of
/// the archive it came from.
const STAMP: &str = ".blurrd-pin";

/// The pin in `third_party/ffmpeg.toml`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Pin {
    /// FFmpeg version, for example `9.0.2`.
    pub(crate) version: String,
    /// Build revision (the N in the release tag).
    pub(crate) revision: u32,
    /// GitHub repository that hosts the release, `owner/name`.
    pub(crate) repository: String,
    /// Release tag, `deps-ffmpeg-<version>-r<revision>`.
    pub(crate) tag: String,
    /// File name of the binary archive in the release.
    pub(crate) asset: String,
    /// SHA-256 of the binary archive, lowercase hex.
    pub(crate) sha256: String,
    /// File name of the corresponding source archive in the release.
    pub(crate) source_asset: String,
    /// SHA-256 of the source archive, lowercase hex.
    pub(crate) source_sha256: String,
}

impl Pin {
    /// Reads and checks `third_party/ffmpeg.toml`.
    pub(crate) fn load(root: &Path) -> Result<Self> {
        let path = root.join("third_party").join("ffmpeg.toml");
        let text =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("parsing {}", path.display()))
    }

    /// Parses and checks the text of a pin file.
    fn parse(text: &str) -> Result<Self> {
        let pin: Self = toml::from_str(text)?;
        let expected_tag = format!("deps-ffmpeg-{}-r{}", pin.version, pin.revision);
        ensure!(
            pin.tag == expected_tag,
            "tag `{}` does not match version and revision (expected `{expected_tag}`)",
            pin.tag
        );
        for (field, value) in [
            ("sha256", &pin.sha256),
            ("source_sha256", &pin.source_sha256),
        ] {
            ensure!(
                value.len() == 64
                    && value
                        .bytes()
                        .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')),
                "`{field}` must be 64 lowercase hex digits"
            );
        }
        Ok(pin)
    }

    /// The public download URL of an asset (works once the repository is
    /// public, and for anyone with access through the API path otherwise).
    fn public_url(&self, asset: &str) -> String {
        format!(
            "https://github.com/{}/releases/download/{}/{asset}",
            self.repository, self.tag
        )
    }
}

/// Where the archive comes from.
#[derive(Debug)]
pub(crate) enum Source {
    /// Download the pinned release asset.
    Release,
    /// Use a local copy of the archive (it must still match the pin).
    Archive(PathBuf),
}

/// Makes `third_party/ffmpeg/` match the pin, then stages the DLLs.
///
/// Does nothing but staging when the folder already holds the pinned build.
pub(crate) fn fetch(root: &Path, source: &Source) -> Result<()> {
    let pin = Pin::load(root)?;
    let dest = root.join("third_party").join("ffmpeg");
    let stamp = dest.join(STAMP);
    let current = fs::read_to_string(&stamp).unwrap_or_default();
    if current.trim() == pin.sha256 {
        eprintln!(
            "FFmpeg {} r{} is up to date in {}",
            pin.version,
            pin.revision,
            dest.display()
        );
    } else {
        let archive = match source {
            Source::Archive(path) => path.clone(),
            Source::Release => download(root, &pin)?,
        };
        let actual = sha256_file(&archive)?;
        ensure!(
            actual == pin.sha256,
            "{} has SHA-256 {actual}, but third_party/ffmpeg.toml pins {}",
            archive.display(),
            pin.sha256
        );
        unpack(&archive, &dest)?;
        fs::write(&stamp, format!("{}\n", pin.sha256))
            .with_context(|| format!("writing {}", stamp.display()))?;
        eprintln!(
            "installed FFmpeg {} r{} into {}",
            pin.version,
            pin.revision,
            dest.display()
        );
        eprintln!(
            "corresponding LGPL source: {} (SHA-256 {})",
            pin.public_url(&pin.source_asset),
            pin.source_sha256
        );
    }
    stage_dlls(root)
}

/// Downloads the pinned archive into `target/xtask/downloads/` (reusing a
/// copy with the right checksum) and returns its path.
fn download(root: &Path, pin: &Pin) -> Result<PathBuf> {
    let dir = target_dir(root).join("xtask").join("downloads");
    fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let path = dir.join(&pin.asset);
    if path.is_file() && sha256_file(&path)? == pin.sha256 {
        return Ok(path);
    }
    let partial = dir.join(format!("{}.part", pin.asset));
    match github_token() {
        Some(token) => {
            let url = release_asset_api_url(pin, &token)?;
            curl(&url, &partial, Some(&token), "application/octet-stream")?;
        }
        None => curl(&pin.public_url(&pin.asset), &partial, None, "*/*")?,
    }
    fs::rename(&partial, &path)
        .with_context(|| format!("moving {} to {}", partial.display(), path.display()))?;
    Ok(path)
}

/// A token for the GitHub API while the repository is private: `GH_TOKEN`
/// or `GITHUB_TOKEN` (CI), otherwise the login of the GitHub CLI.
fn github_token() -> Option<String> {
    for var in ["GH_TOKEN", "GITHUB_TOKEN"] {
        if let Ok(token) = std::env::var(var)
            && !token.trim().is_empty()
        {
            return Some(token.trim().to_owned());
        }
    }
    let output = Command::new("gh")
        .args(["auth", "token"])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let token = String::from_utf8(output.stdout).ok()?;
    (output.status.success() && !token.trim().is_empty()).then(|| token.trim().to_owned())
}

/// Looks up the API URL of the pinned asset (`/releases/assets/<id>`), which
/// serves private release assets to authenticated requests.
fn release_asset_api_url(pin: &Pin, token: &str) -> Result<String> {
    #[derive(Deserialize)]
    struct Release {
        assets: Vec<Asset>,
    }
    #[derive(Deserialize)]
    struct Asset {
        name: String,
        url: String,
    }

    let api = format!(
        "https://api.github.com/repos/{}/releases/tags/{}",
        pin.repository, pin.tag
    );
    let body = curl_to_memory(&api, token).with_context(|| {
        format!(
            "looking up release {} of {} (does it exist, and can the token read \
             the repository? A local copy can be installed with --archive)",
            pin.tag, pin.repository
        )
    })?;
    let release: Release =
        serde_json::from_slice(&body).with_context(|| format!("parsing the response of {api}"))?;
    release
        .assets
        .into_iter()
        .find(|asset| asset.name == pin.asset)
        .map(|asset| asset.url)
        .with_context(|| format!("release {} has no asset {}", pin.tag, pin.asset))
}

/// Runs curl with the authorization header on stdin, so the token never
/// appears on a command line.
fn curl_command(url: &str, token: Option<&str>, accept: &str) -> Command {
    let mut cmd = Command::new("curl");
    cmd.args([
        "--fail",
        "--silent",
        "--show-error",
        "--location",
        "--retry",
        "3",
    ])
    .args(["--header", &format!("Accept: {accept}")])
    .args(["--header", "X-GitHub-Api-Version: 2022-11-28"]);
    if token.is_some() {
        cmd.args(["--header", "@-"]);
    }
    cmd.arg(url);
    cmd
}

/// Feeds the authorization header to a spawned curl and waits for it.
fn run_curl(mut cmd: Command, token: Option<&str>, url: &str) -> Result<std::process::Output> {
    cmd.stdin(if token.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    })
    .stdout(Stdio::piped())
    .stderr(Stdio::inherit());
    let mut child = cmd.spawn().context("running curl (is it on PATH?)")?;
    if let (Some(token), Some(mut stdin)) = (token, child.stdin.take()) {
        writeln!(stdin, "Authorization: Bearer {token}").context("writing to curl")?;
    }
    let output = child.wait_with_output().context("waiting for curl")?;
    if !output.status.success() {
        bail!("downloading {url} failed ({})", output.status);
    }
    Ok(output)
}

fn curl(url: &str, dest: &Path, token: Option<&str>, accept: &str) -> Result<()> {
    eprintln!("downloading {url}");
    let mut cmd = curl_command(url, token, accept);
    cmd.arg("--output").arg(dest);
    run_curl(cmd, token, url).map(drop)
}

fn curl_to_memory(url: &str, token: &str) -> Result<Vec<u8>> {
    let cmd = curl_command(url, Some(token), "application/vnd.github+json");
    run_curl(cmd, Some(token), url).map(|output| output.stdout)
}

/// Lowercase hex SHA-256 of a file.
pub(crate) fn sha256_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1 << 20];
    loop {
        let read = file
            .read(&mut buffer)
            .with_context(|| format!("reading {}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let mut hex = String::with_capacity(64);
    for byte in hasher.finalize() {
        // Writing into a String cannot fail.
        let _ = write!(hex, "{byte:02x}");
    }
    Ok(hex)
}

/// Unpacks the archive's single top-level folder into `dest`, replacing it.
fn unpack(archive: &Path, dest: &Path) -> Result<()> {
    let file = fs::File::open(archive).with_context(|| format!("opening {}", archive.display()))?;
    let mut zip = zip::ZipArchive::new(io::BufReader::new(file))
        .with_context(|| format!("reading {}", archive.display()))?;
    let staging = dest.with_extension("partial");
    if staging.exists() {
        fs::remove_dir_all(&staging).with_context(|| format!("removing {}", staging.display()))?;
    }
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        // `enclosed_name` rejects absolute paths and `..` components.
        let name = entry
            .enclosed_name()
            .with_context(|| format!("unsafe path {}", entry.name()))?;
        // Strip the archive's top-level folder (`ffmpeg-<version>-r<N>-...`).
        let relative: PathBuf = name.components().skip(1).collect();
        if relative.as_os_str().is_empty() {
            continue;
        }
        let out = staging.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&out)?;
        } else {
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut writer =
                fs::File::create(&out).with_context(|| format!("creating {}", out.display()))?;
            io::copy(&mut entry, &mut writer)
                .with_context(|| format!("writing {}", out.display()))?;
        }
    }
    ensure!(
        staging.join("include").join("libavcodec").is_dir() && staging.join("bin").is_dir(),
        "{} does not look like an FFmpeg build (no include/libavcodec or bin)",
        archive.display()
    );
    if dest.exists() {
        fs::remove_dir_all(dest).with_context(|| format!("removing {}", dest.display()))?;
    }
    fs::rename(&staging, dest)
        .with_context(|| format!("moving {} to {}", staging.display(), dest.display()))?;
    Ok(())
}

/// Cargo's target directory (`CARGO_TARGET_DIR` or `<root>/target`).
fn target_dir(root: &Path) -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| root.join("target"), PathBuf::from)
}

/// Copies the FFmpeg DLLs next to the binaries of every build profile.
///
/// Windows looks for import-linked DLLs in the folder of the executable, so
/// they go into `target/<profile>/` (CLI, app) and `target/<profile>/deps/`
/// (test binaries) for `debug`, `release` and any other profile folder that
/// exists. Unchanged files are skipped.
pub(crate) fn stage_dlls(root: &Path) -> Result<()> {
    let bin = root.join("third_party").join("ffmpeg").join("bin");
    let mut dlls = Vec::new();
    for entry in fs::read_dir(&bin).with_context(|| {
        format!(
            "reading {} (run `cargo xtask fetch-ffmpeg` first)",
            bin.display()
        )
    })? {
        let path = entry?.path();
        if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("dll"))
        {
            dlls.push(path);
        }
    }
    ensure!(!dlls.is_empty(), "no DLLs in {}", bin.display());

    let target = target_dir(root);
    let mut profiles = vec![target.join("debug"), target.join("release")];
    if let Ok(entries) = fs::read_dir(&target) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.join(".fingerprint").is_dir() && !profiles.contains(&path) {
                profiles.push(path);
            }
        }
    }
    let mut copied = 0;
    for profile in &profiles {
        for dir in [profile.clone(), profile.join("deps")] {
            fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
            for dll in &dlls {
                let Some(name) = dll.file_name() else {
                    continue;
                };
                let out = dir.join(name);
                if !same_file_contents(dll, &out) {
                    fs::copy(dll, &out).with_context(|| {
                        format!("copying {} to {}", dll.display(), out.display())
                    })?;
                    copied += 1;
                }
            }
        }
    }
    eprintln!(
        "staged {} FFmpeg DLLs into {} profile folders ({copied} copied)",
        dlls.len(),
        profiles.len()
    );
    Ok(())
}

/// Cheap equality check: same size and the copy is not older than the source.
fn same_file_contents(source: &Path, copy: &Path) -> bool {
    let (Ok(a), Ok(b)) = (fs::metadata(source), fs::metadata(copy)) else {
        return false;
    };
    match (a.modified(), b.modified()) {
        (Ok(src), Ok(dst)) => a.len() == b.len() && dst >= src,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{Pin, sha256_file};

    const PIN: &str = r#"
version = "9.0.2"
revision = 3
repository = "owner/repo"
tag = "deps-ffmpeg-9.0.2-r3"
asset = "ffmpeg-9.0.2-r3-win64-lgpl-shared.zip"
sha256 = "107d3a9452c7216613a1c003c09f87b6a7036e324700b5a2d2e770f77d4362ea"
source_asset = "ffmpeg-9.0.2-r3-source.zip"
source_sha256 = "234dc2f8aacf8bd19a03d2d7075ca116c4e99e2fc6316f718a7d22a04c4a4af6"
"#;

    #[test]
    fn parses_a_valid_pin() {
        let pin = Pin::parse(PIN).unwrap();
        assert_eq!(pin.revision, 3);
        assert_eq!(
            pin.public_url(&pin.asset),
            "https://github.com/owner/repo/releases/download/deps-ffmpeg-9.0.2-r3/\
             ffmpeg-9.0.2-r3-win64-lgpl-shared.zip"
        );
    }

    #[test]
    fn rejects_inconsistent_pins() {
        // Tag no longer matches the revision.
        assert!(Pin::parse(&PIN.replace("revision = 3", "revision = 4")).is_err());
        assert!(Pin::parse(&PIN.replace("107d3a94", "107D3A94")).is_err());
        assert!(Pin::parse(&PIN.replace("234dc2f8", "234dc2")).is_err());
        assert!(Pin::parse(&format!("{PIN}extra = 1\n")).is_err());
    }

    #[test]
    fn hashes_files() {
        let path = std::env::temp_dir().join(format!("blurrd-xtask-sha-{}", std::process::id()));
        std::fs::write(&path, b"abc").unwrap();
        assert_eq!(
            sha256_file(&path).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        std::fs::remove_file(&path).unwrap();
    }
}
