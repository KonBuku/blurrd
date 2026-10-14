//! The engine's cache folder (architecture plan 6.2): one folder per
//! analysed video, named by the video's fingerprint, with the project, the
//! PTS table and the thumbnails.
//!
//! ```text
//! <cache>/projects/<fingerprint>/project.json
//! <cache>/projects/<fingerprint>/pts.bin
//! <cache>/projects/<fingerprint>/thumbs/00042.jpg
//! <cache>/caps/decoders.json  caps/encoders.json
//! ```

use std::fmt::Write as _;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use blurrd_core::{AppError, ErrorKind, Fingerprint, Project, schema};
use blurrd_video::PtsTable;
use sha2::{Digest, Sha256};

/// Bytes hashed at each end of the file for the fingerprint.
const FINGERPRINT_SPAN: u64 = 1 << 20;

/// Computes the fingerprint of the file at `path`: its size, modification
/// time and the BLAKE3 hash of its size (little-endian `u64`), its first
/// MiB and its last MiB (the whole file when it is smaller than 2 MiB).
///
/// The hash identifies the video cheaply: recorders write a header and an
/// index that change whenever the content does, so two files with the same
/// size, head and tail are the same recording.
///
/// # Errors
///
/// [`ErrorKind::NotFound`] or [`ErrorKind::Io`] when the file cannot be
/// read.
pub fn fingerprint(path: &Path) -> Result<Fingerprint, AppError> {
    let io = |e: std::io::Error| io_error(&e, "The video could not be read.", path);
    let mut file = File::open(path).map_err(io)?;
    let metadata = file.metadata().map_err(io)?;
    let size = metadata.len();
    let mtime = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
    let mut hasher = blake3::Hasher::new();
    hasher.update(&size.to_le_bytes());
    let mut buffer = Vec::new();
    if size <= 2 * FINGERPRINT_SPAN {
        file.read_to_end(&mut buffer).map_err(io)?;
        hasher.update(&buffer);
    } else {
        let span = FINGERPRINT_SPAN as usize;
        buffer.resize(span, 0);
        file.read_exact(&mut buffer).map_err(io)?;
        hasher.update(&buffer);
        file.seek(SeekFrom::End(-(FINGERPRINT_SPAN as i64)))
            .map_err(io)?;
        file.read_exact(&mut buffer).map_err(io)?;
        hasher.update(&buffer);
    }
    Ok(Fingerprint {
        size,
        mtime,
        blake3_head_tail: hasher.finalize().to_hex().to_string(),
    })
}

/// Whether `found` (the file now) is the file `expected` describes: same
/// size and hash. The modification time is not compared (copying a file
/// changes it).
pub fn same_file(expected: &Fingerprint, found: &Fingerprint) -> bool {
    expected.size == found.size
        && expected
            .blake3_head_tail
            .eq_ignore_ascii_case(&found.blake3_head_tail)
}

/// The hex SHA-256 of `bytes`.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

/// The project's `pts_sha256`: the hex SHA-256 of the PTS table in the
/// `pts.bin` format.
pub fn pts_sha256(pts: &PtsTable) -> String {
    sha256_hex(&pts.to_bytes())
}

/// The project's `keyframes_sha256`: the hex SHA-256 of the keyframes'
/// presentation indices, sorted, as little-endian `u32`s (an empty list
/// when the container lists none).
pub fn keyframes_sha256(keyframes: Option<&[u32]>) -> String {
    let mut sorted = keyframes.unwrap_or_default().to_vec();
    sorted.sort_unstable();
    let bytes: Vec<u8> = sorted.iter().flat_map(|k| k.to_le_bytes()).collect();
    sha256_hex(&bytes)
}

/// The cache folder of one video.
#[derive(Debug, Clone)]
pub(crate) struct ProjectDir {
    dir: PathBuf,
}

impl ProjectDir {
    /// The folder of the video with `fingerprint` inside `cache`.
    pub(crate) fn new(cache: &Path, fingerprint: &Fingerprint) -> Self {
        Self {
            dir: cache.join("projects").join(&fingerprint.blake3_head_tail),
        }
    }

    /// The project file.
    pub(crate) fn project(&self) -> PathBuf {
        self.dir.join("project.json")
    }

    /// The PTS table.
    pub(crate) fn pts(&self) -> PathBuf {
        self.dir.join("pts.bin")
    }

    /// Creates the folder.
    pub(crate) fn create(&self) -> Result<(), AppError> {
        std::fs::create_dir_all(&self.dir)
            .map_err(|e| io_error(&e, "The cache folder could not be created.", &self.dir))
    }

    /// The cached project, if there is a valid one.
    pub(crate) fn read_project(&self) -> Option<Project> {
        let path = self.project();
        path.is_file()
            .then(|| read_project(&path))
            .and_then(|project| match project {
                Ok(project) => Some(project),
                Err(error) => {
                    tracing::warn!(%error, "ignoring an unreadable cached project");
                    None
                }
            })
    }

    /// The cached PTS table, if there is a valid one.
    pub(crate) fn read_pts(&self) -> Option<PtsTable> {
        let path = self.pts();
        path.is_file()
            .then(|| PtsTable::read(&path))
            .and_then(|table| match table {
                Ok(table) => Some(table),
                Err(error) => {
                    tracing::warn!(%error, "ignoring an unreadable cached PTS table");
                    None
                }
            })
    }
}

/// Writes `project` to `path` atomically (a temporary file next to it,
/// then a rename), so a reader never sees half a file.
///
/// # Errors
///
/// [`ErrorKind::Io`] when the file cannot be written.
pub fn write_project(project: &Project, path: &Path) -> Result<(), AppError> {
    let text = schema::to_json(project)?;
    let mut temp = path.as_os_str().to_owned();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);
    let written = std::fs::write(&temp, text).and_then(|()| std::fs::rename(&temp, path));
    written.map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        io_error(&e, "The project could not be saved.", path)
    })
}

/// Reads and validates the project file at `path`.
///
/// # Errors
///
/// [`ErrorKind::NotFound`] or [`ErrorKind::Io`] when it cannot be read,
/// [`ErrorKind::InvalidProject`] when it is malformed or from a newer
/// version.
pub fn read_project(path: &Path) -> Result<Project, AppError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| io_error(&e, "The project file could not be read.", path))?;
    Ok(schema::from_json::<Project>(&text)?)
}

/// An I/O error about `path` with `message`.
pub(crate) fn io_error(error: &std::io::Error, message: &str, path: &Path) -> AppError {
    let kind = if error.kind() == std::io::ErrorKind::NotFound {
        ErrorKind::NotFound
    } else {
        ErrorKind::Io
    };
    AppError::new(kind, message).with_detail(format!("{}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use blurrd_core::{Fingerprint, Rational};
    use blurrd_video::PtsTable;

    use super::{FINGERPRINT_SPAN, fingerprint, keyframes_sha256, pts_sha256, same_file};

    #[test]
    fn fingerprints_follow_head_tail_and_size() {
        let dir = std::env::temp_dir().join(format!("blurrd-fp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.bin");
        let mut bytes = vec![7u8; 3 * FINGERPRINT_SPAN as usize];
        std::fs::write(&path, &bytes).unwrap();
        let first = fingerprint(&path).unwrap();
        assert_eq!(first.size, bytes.len() as u64);
        assert_eq!(first.blake3_head_tail.len(), 64);
        // A change in the middle is not seen (by design), one in the tail is.
        bytes[FINGERPRINT_SPAN as usize + 10] = 1;
        std::fs::write(&path, &bytes).unwrap();
        assert!(same_file(&first, &fingerprint(&path).unwrap()));
        *bytes.last_mut().unwrap() = 1;
        std::fs::write(&path, &bytes).unwrap();
        assert!(!same_file(&first, &fingerprint(&path).unwrap()));
        // Small files are hashed whole.
        std::fs::write(&path, b"abc").unwrap();
        let small = fingerprint(&path).unwrap();
        std::fs::write(&path, b"xyz").unwrap();
        assert!(!same_file(&small, &fingerprint(&path).unwrap()));
        let other = Fingerprint {
            size: 3,
            mtime: 0,
            blake3_head_tail: small.blake3_head_tail.to_uppercase(),
        };
        assert!(same_file(&small, &other), "hash case and mtime are ignored");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn table_hashes_depend_on_the_content() {
        let a = PtsTable::new(Rational::new(1, 60).unwrap(), vec![0, 1, 2]).unwrap();
        let b = PtsTable::new(Rational::new(1, 60).unwrap(), vec![0, 1, 3]).unwrap();
        assert_ne!(pts_sha256(&a), pts_sha256(&b));
        assert_eq!(
            keyframes_sha256(Some(&[60, 0])),
            keyframes_sha256(Some(&[0, 60]))
        );
        assert_ne!(keyframes_sha256(Some(&[0])), keyframes_sha256(None));
    }
}
