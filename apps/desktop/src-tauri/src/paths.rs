//! The path rule of the IPC commands (architecture plan 10): a command
//! only takes paths the user chose in a dialog or dropped onto the window.
//!
//! The dialog plugin and the window's drop handler add every chosen file
//! and folder to the asset protocol's scope (folders with everything
//! inside them). That scope is therefore the list of user-chosen paths:
//! the commands accept a path only when the scope allows it, after making
//! it absolute and canonical. Paths the backend wrote itself (the saved
//! queue, the preview proxy) are added to the scope when they are handed
//! to the frontend.

use std::path::{Component, Path, PathBuf, Prefix};

use blurrd_core::{AppError, ErrorKind};

/// The set of paths the user chose.
pub(crate) trait PathScope: Send + Sync {
    /// Whether `path` (absolute, canonical) is a chosen file or inside a
    /// chosen folder.
    fn allows(&self, path: &Path) -> bool;
    /// Adds a file the backend hands to the frontend.
    fn add_file(&self, path: &Path);
}

impl PathScope for tauri::scope::fs::Scope {
    fn allows(&self, path: &Path) -> bool {
        self.is_allowed(path)
    }

    fn add_file(&self, path: &Path) {
        if let Err(error) = self.allow_file(path) {
            tauri_plugin_log::log::warn!("could not allow a file for the preview: {error}");
        }
    }
}

/// Checks the paths of IPC commands against the user's choices.
pub(crate) struct UserPaths {
    scope: Box<dyn PathScope>,
}

impl std::fmt::Debug for UserPaths {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UserPaths").finish_non_exhaustive()
    }
}

impl UserPaths {
    /// Paths checked against `scope`.
    pub(crate) fn new(scope: Box<dyn PathScope>) -> Self {
        Self { scope }
    }

    /// The canonical form of `path`, which must exist and have been chosen
    /// by the user.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::NotFound`] when it does not exist, [`ErrorKind::Io`]
    /// when it is relative or was not chosen by the user.
    pub(crate) fn existing(&self, path: &Path) -> Result<PathBuf, AppError> {
        let absolute = absolute(path)?;
        let canonical = std::fs::canonicalize(&absolute).map_err(|error| {
            let kind = if error.kind() == std::io::ErrorKind::NotFound {
                ErrorKind::NotFound
            } else {
                ErrorKind::Io
            };
            AppError::new(kind, "The file could not be found.")
                .with_detail(format!("{}: {error}", absolute.display()))
        })?;
        self.check(simplified(&canonical))
    }

    /// The absolute form of `path`, which may not exist yet (a file chosen
    /// in a save dialog) but must have been chosen by the user.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Io`] when it is relative or was not chosen.
    pub(crate) fn target(&self, path: &Path) -> Result<PathBuf, AppError> {
        let absolute = absolute(path)?;
        let resolved = match std::fs::canonicalize(&absolute) {
            Ok(canonical) => simplified(&canonical),
            Err(_) => absolute,
        };
        self.check(resolved)
    }

    /// Lets the frontend use `path`, a file the backend chose (a restored
    /// queue entry, a preview proxy).
    pub(crate) fn add_file(&self, path: &Path) {
        self.scope.add_file(path);
    }

    fn check(&self, path: PathBuf) -> Result<PathBuf, AppError> {
        if self.scope.allows(&path) {
            Ok(path)
        } else {
            Err(AppError::new(
                ErrorKind::Io,
                "blurrd only opens files you chose or dropped onto it.",
            )
            .with_detail(path.display().to_string()))
        }
    }
}

/// `path` if it is absolute.
fn absolute(path: &Path) -> Result<PathBuf, AppError> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Err(
            AppError::new(ErrorKind::Io, "Only absolute paths are accepted.")
                .with_detail(path.display().to_string()),
        )
    }
}

/// `path` without the verbatim prefix `canonicalize` adds on Windows
/// (`\\?\C:\a` becomes `C:\a`, `\\?\UNC\host\share` becomes
/// `\\host\share`), the form the dialogs and the drop handler report.
pub(crate) fn simplified(path: &Path) -> PathBuf {
    let mut components = path.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return path.to_path_buf();
    };
    let rest: PathBuf = components.collect();
    match prefix.kind() {
        Prefix::VerbatimDisk(drive) => {
            PathBuf::from(format!("{}:\\", char::from(drive))).join(rest)
        }
        Prefix::VerbatimUNC(server, share) => PathBuf::from(format!(
            r"\\{}\{}\",
            server.to_string_lossy(),
            share.to_string_lossy()
        ))
        .join(rest),
        _ => path.to_path_buf(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use blurrd_core::ErrorKind;

    use super::{PathScope, UserPaths, simplified};

    /// A scope of explicitly allowed paths and folders, for tests.
    #[derive(Debug, Default)]
    pub(crate) struct TestScope {
        pub(crate) allowed: Mutex<Vec<PathBuf>>,
    }

    impl PathScope for TestScope {
        fn allows(&self, path: &Path) -> bool {
            self.allowed
                .lock()
                .unwrap()
                .iter()
                .any(|allowed| path.starts_with(allowed))
        }

        fn add_file(&self, path: &Path) {
            self.allowed.lock().unwrap().push(path.to_path_buf());
        }
    }

    /// User paths that allow everything below `root`.
    pub(crate) fn paths_below(root: &Path) -> UserPaths {
        let scope = TestScope::default();
        scope.add_file(&simplified(&root.canonicalize().unwrap()));
        UserPaths::new(Box::new(scope))
    }

    #[test]
    fn verbatim_prefixes_are_removed() {
        assert_eq!(
            simplified(Path::new(r"\\?\C:\clips\a.mp4")),
            PathBuf::from(r"C:\clips\a.mp4")
        );
        assert_eq!(
            simplified(Path::new(r"\\?\UNC\nas\share\a.mp4")),
            PathBuf::from(r"\\nas\share\a.mp4")
        );
        assert_eq!(
            simplified(Path::new(r"C:\clips\a.mp4")),
            PathBuf::from(r"C:\clips\a.mp4")
        );
    }

    #[test]
    fn only_chosen_paths_pass() {
        let dir = std::env::temp_dir().join(format!("blurrd-paths-{}", std::process::id()));
        let chosen = dir.join("chosen");
        let other = dir.join("other");
        std::fs::create_dir_all(&chosen).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(chosen.join("a.mp4"), b"x").unwrap();
        std::fs::write(other.join("b.mp4"), b"x").unwrap();
        let paths = paths_below(&chosen);

        let found = paths.existing(&chosen.join("a.mp4")).unwrap();
        assert!(found.ends_with("a.mp4"));
        assert!(!found.to_string_lossy().starts_with(r"\\?\"));
        // `..` cannot leave the chosen folder.
        let escape = chosen.join("..").join("other").join("b.mp4");
        assert_eq!(paths.existing(&escape).unwrap_err().kind, ErrorKind::Io);
        assert_eq!(
            paths
                .existing(&chosen.join("missing.mp4"))
                .unwrap_err()
                .kind,
            ErrorKind::NotFound
        );
        assert_eq!(
            paths.existing(Path::new("a.mp4")).unwrap_err().kind,
            ErrorKind::Io
        );
        // A file chosen in a save dialog does not exist yet.
        assert!(paths.target(&chosen.join("new.json")).is_ok());
        assert!(paths.target(&other.join("new.json")).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
