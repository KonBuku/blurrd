//! The saved queue (`session.json` in the cache folder, architecture plan
//! 6.2): the files of the queue in order, with their resting status, their
//! last output and, for files whose project was opened, the project
//! fingerprint. The status is the frontend's and is stored as it comes.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use blurrd_core::{AppError, ErrorKind, Fingerprint};
use serde::{Deserialize, Serialize};

use crate::projects::lock;

/// One file of the saved queue as the frontend sends and receives it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct SessionEntry {
    /// The file.
    pub(crate) path: PathBuf,
    /// Its resting status (a frontend type, kept as it is).
    pub(crate) status: serde_json::Value,
    /// The exported file, once there is one.
    pub(crate) output: Option<PathBuf>,
}

/// One file of `session.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct StoredEntry {
    #[serde(flatten)]
    entry: SessionEntry,
    /// The fingerprint of the file's project, when it was opened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fingerprint: Option<Fingerprint>,
}

/// The saved queue.
#[derive(Debug)]
pub(crate) struct Session {
    file: PathBuf,
    guard: Mutex<()>,
}

impl Session {
    /// The queue saved in `file`.
    pub(crate) fn new(file: PathBuf) -> Self {
        Self {
            file,
            guard: Mutex::new(()),
        }
    }

    /// The saved entries in queue order; files that no longer exist are
    /// left out. A missing or unreadable file is an empty queue (logged).
    pub(crate) fn load(&self) -> Vec<SessionEntry> {
        let _guard = lock(&self.guard);
        let Ok(text) = std::fs::read_to_string(&self.file) else {
            return Vec::new();
        };
        match serde_json::from_str::<Vec<StoredEntry>>(&text) {
            Ok(entries) => entries
                .into_iter()
                .map(|stored| stored.entry)
                .filter(|entry| entry.path.is_file())
                .collect(),
            Err(error) => {
                tauri_plugin_log::log::warn!("ignoring an unreadable saved queue: {error}");
                Vec::new()
            }
        }
    }

    /// Replaces the saved queue; `fingerprint` names the project of a file
    /// when it was opened.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Io`] when the file cannot be written.
    pub(crate) fn save(
        &self,
        entries: Vec<SessionEntry>,
        fingerprint: impl Fn(&Path) -> Option<Fingerprint>,
    ) -> Result<(), AppError> {
        let stored: Vec<StoredEntry> = entries
            .into_iter()
            .map(|entry| StoredEntry {
                fingerprint: fingerprint(&entry.path),
                entry,
            })
            .collect();
        let text = serde_json::to_string(&stored).map_err(|e| {
            AppError::new(ErrorKind::Internal, "The queue could not be saved.")
                .with_detail(e.to_string())
        })?;
        let _guard = lock(&self.guard);
        let io = |e: std::io::Error| {
            AppError::new(ErrorKind::Io, "The queue could not be saved.")
                .with_detail(format!("{}: {e}", self.file.display()))
        };
        if let Some(folder) = self.file.parent() {
            std::fs::create_dir_all(folder).map_err(io)?;
        }
        let mut temp = self.file.as_os_str().to_owned();
        temp.push(".tmp");
        let temp = PathBuf::from(temp);
        std::fs::write(&temp, text)
            .and_then(|()| std::fs::rename(&temp, &self.file))
            .map_err(|e| {
                let _ = std::fs::remove_file(&temp);
                io(e)
            })
    }
}

#[cfg(test)]
mod tests {
    use blurrd_core::Fingerprint;
    use serde_json::json;

    use super::{Session, SessionEntry};

    #[test]
    fn the_queue_round_trips_with_fingerprints_and_drops_missing_files() {
        let dir = std::env::temp_dir().join(format!("blurrd-session-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let kept = dir.join("kept.mp4");
        std::fs::write(&kept, b"x").unwrap();
        let entries = vec![
            SessionEntry {
                path: kept.clone(),
                status: json!({ "kind": "done", "verified": true }),
                output: Some(dir.join("kept_blurrd.mp4")),
            },
            SessionEntry {
                path: dir.join("gone.mp4"),
                status: json!({ "kind": "queued" }),
                output: None,
            },
        ];
        let session = Session::new(dir.join("session.json"));
        assert_eq!(session.load(), vec![]);
        session
            .save(entries.clone(), |path| {
                (path == kept).then(|| Fingerprint {
                    size: 1,
                    mtime: 0,
                    blake3_head_tail: "ab".to_owned(),
                })
            })
            .unwrap();
        let text = std::fs::read_to_string(dir.join("session.json")).unwrap();
        assert!(text.contains("\"fingerprint\""));
        assert_eq!(session.load(), entries[..1].to_vec());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
