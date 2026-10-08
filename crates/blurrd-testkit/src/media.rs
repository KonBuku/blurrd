//! The media manifest: which real recordings the local tests use, how they
//! are split into tuning and held-out sets, and where they live.
//!
//! `tests/media/manifest.toml` lists every recording by id, file name,
//! SHA-256 and frame count, the excerpts E1–E18 (frame ranges and case
//! tags) and the held-out flag (architecture plan 11.2). The recordings
//! themselves stay outside the repository, in the folder named by
//! [`MEDIA_ENV`]; tests that need them call [`MediaDir::from_env`] and skip
//! with a message when it is not set.

use std::collections::BTreeSet;
use std::fmt::{self, Write as _};
use std::io::Read;
use std::path::{Path, PathBuf};

use blurrd_core::FrameRange;
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// Environment variable naming the folder with the recordings.
pub const MEDIA_ENV: &str = "BLURRD_TEST_MEDIA";

/// Schema identifier of the manifest.
pub const MANIFEST_SCHEMA: &str = "blurrd-media/1";

/// Number of held-out clips (plan 11.2: 10 of the 31 clips).
pub const HELD_OUT_CLIPS: usize = 10;

/// The case tags an excerpt may carry.
pub const CASE_TAGS: [&str; 30] = [
    "banner",
    "banner-bright",
    "banner-feed-overlap",
    "banner-fx",
    "banner-settle",
    "banner-two-word",
    "chat",
    "chat-broadcast",
    "chat-fade",
    "chat-low-contrast",
    "chat-push-up",
    "chat-stack",
    "chat-system",
    "chat-team",
    "chat-two-word",
    "chat-wrap",
    "killfeed",
    "killfeed-flash",
    "killfeed-multikill",
    "killfeed-self-kill",
    "killfeed-stack",
    "killfeed-wall",
    "killfeed-wipe-in",
    "negative-location",
    "negative-ult-ready",
    "voice",
    "voice-blink",
    "voice-pop",
    "voice-stack",
    "world-tags",
];

/// One recording.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Clip {
    /// Short identifier (the recording's date and time).
    pub id: String,
    /// File name inside the media folder.
    pub file: String,
    /// SHA-256 of the file, lowercase hex.
    pub sha256: String,
    /// Number of decoded frames.
    pub frames: u32,
    /// Whether the clip belongs to the held-out set.
    pub held_out: bool,
    /// Whether the clip is run in full as a soak test.
    pub soak: bool,
}

/// A short range of a recording that covers specific cases.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Excerpt {
    /// Identifier, `E1` to `E18`.
    pub id: String,
    /// Id of the clip it is cut from.
    pub clip: String,
    /// Inclusive frame ranges of the clip.
    pub frames: Vec<FrameRange>,
    /// What the excerpt covers (from [`CASE_TAGS`]).
    pub cases: Vec<String>,
}

/// The parsed and validated manifest.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Must be [`MANIFEST_SCHEMA`].
    pub schema: String,
    /// Every recording.
    #[serde(rename = "clip")]
    pub clips: Vec<Clip>,
    /// The excerpts.
    #[serde(rename = "excerpt")]
    pub excerpts: Vec<Excerpt>,
}

/// Why a manifest was rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ManifestError {
    /// The file could not be read.
    #[error("reading {path}: {message}")]
    Io {
        /// The manifest path.
        path: PathBuf,
        /// The I/O error.
        message: String,
    },
    /// The TOML does not have the manifest's shape.
    #[error("parsing the manifest: {0}")]
    Parse(String),
    /// The content breaks a rule of the manifest.
    #[error("invalid manifest: {0}")]
    Invalid(String),
}

impl Manifest {
    /// The path of the committed manifest, `tests/media/manifest.toml`.
    pub fn repository_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("tests")
            .join("media")
            .join("manifest.toml")
    }

    /// Reads and validates the committed manifest.
    ///
    /// # Errors
    ///
    /// [`ManifestError`] if it cannot be read, parsed or validated.
    pub fn load_repository() -> Result<Self, ManifestError> {
        Self::load(&Self::repository_path())
    }

    /// Reads and validates a manifest file.
    ///
    /// # Errors
    ///
    /// [`ManifestError`] if it cannot be read, parsed or validated.
    pub fn load(path: &Path) -> Result<Self, ManifestError> {
        let text = std::fs::read_to_string(path).map_err(|e| ManifestError::Io {
            path: path.to_path_buf(),
            message: e.to_string(),
        })?;
        Self::parse(&text)
    }

    /// Parses and validates manifest text.
    ///
    /// # Errors
    ///
    /// [`ManifestError::Parse`] or [`ManifestError::Invalid`].
    pub fn parse(text: &str) -> Result<Self, ManifestError> {
        let manifest: Self =
            toml::from_str(text).map_err(|e| ManifestError::Parse(e.to_string()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    fn validate(&self) -> Result<(), ManifestError> {
        let invalid = |msg: String| Err(ManifestError::Invalid(msg));
        if self.schema != MANIFEST_SCHEMA {
            return invalid(format!(
                "schema `{}`, expected `{MANIFEST_SCHEMA}`",
                self.schema
            ));
        }
        let mut ids = BTreeSet::new();
        for clip in &self.clips {
            if !ids.insert(clip.id.as_str()) {
                return invalid(format!("duplicate clip id {}", clip.id));
            }
            if !is_sha256(&clip.sha256) {
                return invalid(format!(
                    "clip {}: sha256 must be 64 lowercase hex digits",
                    clip.id
                ));
            }
            if clip.file.is_empty() || clip.file.contains(['/', '\\']) || clip.frames == 0 {
                return invalid(format!(
                    "clip {}: invalid file name or frame count",
                    clip.id
                ));
            }
        }
        let held_out = self.clips.iter().filter(|c| c.held_out).count();
        if held_out != HELD_OUT_CLIPS {
            return invalid(format!(
                "{held_out} held-out clips, expected {HELD_OUT_CLIPS}"
            ));
        }
        let mut excerpt_ids = BTreeSet::new();
        for e in &self.excerpts {
            if !excerpt_ids.insert(e.id.as_str()) {
                return invalid(format!("duplicate excerpt id {}", e.id));
            }
            let Some(clip) = self.clip(&e.clip) else {
                return invalid(format!("excerpt {}: unknown clip {}", e.id, e.clip));
            };
            if e.frames.is_empty() {
                return invalid(format!("excerpt {}: no frame ranges", e.id));
            }
            for pair in e.frames.windows(2) {
                if pair[1].f0 <= pair[0].f1 {
                    return invalid(format!("excerpt {}: ranges overlap or are unsorted", e.id));
                }
            }
            if e.frames.iter().any(|r| r.f1 >= clip.frames) {
                return invalid(format!(
                    "excerpt {}: range beyond the clip's {} frames",
                    e.id, clip.frames
                ));
            }
            if let Some(tag) = e.cases.iter().find(|t| !CASE_TAGS.contains(&t.as_str())) {
                return invalid(format!("excerpt {}: unknown case tag `{tag}`", e.id));
            }
        }
        Ok(())
    }

    /// The clip with id `id`.
    pub fn clip(&self, id: &str) -> Option<&Clip> {
        self.clips.iter().find(|c| c.id == id)
    }

    /// Whether an excerpt is held out (its clip is).
    pub fn is_held_out(&self, excerpt: &Excerpt) -> bool {
        self.clip(&excerpt.clip).is_some_and(|c| c.held_out)
    }

    /// The excerpts covering a case tag.
    pub fn excerpts_with<'a>(&'a self, case: &'a str) -> impl Iterator<Item = &'a Excerpt> + 'a {
        self.excerpts
            .iter()
            .filter(move |e| e.cases.iter().any(|c| c == case))
    }
}

fn is_sha256(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The folder with the recordings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaDir {
    root: PathBuf,
}

/// The result of checking one clip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipStatus {
    /// The file exists and its SHA-256 matches.
    Ok,
    /// The file does not exist.
    Missing,
    /// The file's SHA-256 differs.
    Mismatch {
        /// The actual SHA-256.
        actual: String,
    },
    /// The file could not be read.
    Unreadable(String),
}

impl fmt::Display for ClipStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ok => f.write_str("ok"),
            Self::Missing => f.write_str("missing"),
            Self::Mismatch { actual } => write!(f, "SHA-256 mismatch (got {actual})"),
            Self::Unreadable(e) => write!(f, "unreadable: {e}"),
        }
    }
}

impl MediaDir {
    /// The folder named by [`MEDIA_ENV`], or `None` if the variable is not
    /// set or empty.
    pub fn from_env() -> Option<Self> {
        std::env::var_os(MEDIA_ENV)
            .filter(|v| !v.is_empty())
            .map(|root| Self::new(PathBuf::from(root)))
    }

    /// A media folder at `root`.
    pub const fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// The folder.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The path of a clip's file.
    pub fn clip_path(&self, clip: &Clip) -> PathBuf {
        self.root.join(&clip.file)
    }

    /// Where the golden of an excerpt is stored: `goldens/<id>.golden.json`
    /// in the media folder (never in the repository).
    pub fn golden_path(&self, excerpt: &Excerpt) -> PathBuf {
        self.root
            .join("goldens")
            .join(format!("{}.golden.json", excerpt.id))
    }

    /// Checks that a clip is present with the pinned SHA-256.
    pub fn verify(&self, clip: &Clip) -> ClipStatus {
        let path = self.clip_path(clip);
        if !path.is_file() {
            return ClipStatus::Missing;
        }
        match sha256_file(&path) {
            Ok(actual) if actual == clip.sha256 => ClipStatus::Ok,
            Ok(actual) => ClipStatus::Mismatch { actual },
            Err(e) => ClipStatus::Unreadable(e.to_string()),
        }
    }
}

/// Lowercase hex SHA-256 of a file.
///
/// # Errors
///
/// Any I/O error while reading.
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let read = file.read(&mut buffer)?;
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

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::{ClipStatus, Manifest, ManifestError, MediaDir, sha256_file};

    fn manifest(held_out: usize, extra: &str) -> String {
        let mut s = String::from("schema = \"blurrd-media/1\"\n");
        for i in 0..12 {
            writeln!(
                s,
                "[[clip]]\nid = \"c{i}\"\nfile = \"c{i}.mp4\"\nsha256 = \"{}\"\nframes = 100\n\
                 held_out = {}\nsoak = false",
                "a".repeat(64),
                i < held_out
            )
            .unwrap();
        }
        s.push_str("[[excerpt]]\nid = \"E1\"\nclip = \"c11\"\nframes = [[10, 20], [30, 40]]\n");
        s.push_str("cases = [\"killfeed\"]\n");
        s.push_str(extra);
        s
    }

    #[test]
    fn accepts_a_valid_manifest() {
        let m = Manifest::parse(&manifest(10, "")).unwrap();
        assert_eq!(m.clips.len(), 12);
        assert!(!m.is_held_out(&m.excerpts[0]));
        assert_eq!(m.excerpts_with("killfeed").count(), 1);
    }

    #[test]
    fn rejects_broken_manifests() {
        let invalid = |text: &str| matches!(Manifest::parse(text), Err(ManifestError::Invalid(_)));
        assert!(invalid(&manifest(9, "")));
        assert!(invalid(&manifest(10, "").replace("[30, 40]", "[30, 400]")));
        assert!(invalid(
            &manifest(10, "").replace("[[10, 20], [30, 40]]", "[[30, 40], [10, 20]]")
        ));
        assert!(invalid(
            &manifest(10, "").replace("\"killfeed\"", "\"kill feed\"")
        ));
        assert!(invalid(
            &manifest(10, "").replace("clip = \"c11\"", "clip = \"nope\"")
        ));
        assert!(invalid(&manifest(10, "").replacen(
            &"a".repeat(64),
            &"A".repeat(64),
            1
        )));
        assert!(matches!(
            Manifest::parse(&manifest(10, "unknown = 1\n")),
            Err(ManifestError::Parse(_))
        ));
    }

    #[test]
    fn verifies_files() {
        let dir = std::env::temp_dir().join(format!("blurrd-media-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("c0.mp4"), b"abc").unwrap();
        let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(sha256_file(&dir.join("c0.mp4")).unwrap(), abc);
        let mut m = Manifest::parse(&manifest(10, "")).unwrap();
        let media = MediaDir::new(dir.clone());
        assert_eq!(media.verify(&m.clips[1]), ClipStatus::Missing);
        assert!(matches!(
            media.verify(&m.clips[0]),
            ClipStatus::Mismatch { .. }
        ));
        m.clips[0].sha256 = abc.to_owned();
        assert_eq!(media.verify(&m.clips[0]), ClipStatus::Ok);
        assert!(
            media
                .golden_path(&m.excerpts[0])
                .ends_with("goldens/E1.golden.json")
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
