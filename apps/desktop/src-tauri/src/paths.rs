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
        // The scope matches globs on the text of the path and does not
        // resolve `..`: `<chosen>\..\x` would match `<chosen>\**`.
        !has_dot_segments(path) && self.is_allowed(path)
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
        self.check(canonical(&absolute(path)?)?)
    }

    /// The canonical form of `path`, a file that may not exist yet (a file
    /// chosen in a save dialog) but must lie in a place the user chose.
    ///
    /// The folder of the file must exist and is resolved by the system
    /// (links, `8.3` names); the file name is joined to it as it is. Paths
    /// with `.` or `..` segments, names the system would rewrite (trailing
    /// dots or spaces, `:` streams), reserved device names (`NUL`, `CON`,
    /// `COM1`, also with an extension) and device or other special
    /// prefixes are refused, so the path that is checked is the path that
    /// is written. An existing file is resolved like the folder, so a link
    /// cannot lead out of the chosen place.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::NotFound`] when the folder does not exist,
    /// [`ErrorKind::Io`] when the path is relative, not plain, names a
    /// folder or was not chosen.
    pub(crate) fn target(&self, path: &Path) -> Result<PathBuf, AppError> {
        let absolute = absolute(path)?;
        let (folder, name) = plain_parts(&absolute)?;
        let joined = canonical(folder)?.join(name);
        let resolved = match std::fs::symlink_metadata(&joined) {
            // Something exists there (a file, a folder, a link): check
            // where it really is; a dangling link has no answer.
            Ok(_) => canonical(&joined)?,
            Err(_) => joined,
        };
        if resolved.is_dir() {
            return Err(
                AppError::new(ErrorKind::Io, "This path names a folder, not a file.")
                    .with_detail(resolved.display().to_string()),
            );
        }
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

/// The canonical form of `path` without the verbatim prefix.
fn canonical(path: &Path) -> Result<PathBuf, AppError> {
    std::fs::canonicalize(path)
        .map(|canonical| simplified(&canonical))
        .map_err(|error| {
            let kind = if error.kind() == std::io::ErrorKind::NotFound {
                ErrorKind::NotFound
            } else {
                ErrorKind::Io
            };
            AppError::new(kind, "The file could not be found.")
                .with_detail(format!("{}: {error}", path.display()))
        })
}

/// The folder and the file name of `path` (absolute), when every part of
/// it is plain: a drive or `\\server\share` prefix (also in its verbatim
/// form), then only names that the system keeps as they are.
fn plain_parts(path: &Path) -> Result<(&Path, &std::ffi::OsStr), AppError> {
    let not_plain = |why: &str| {
        AppError::new(ErrorKind::Io, "This path cannot be used.")
            .with_detail(format!("{}: {why}", path.display()))
    };
    if has_dot_segments(path) {
        return Err(not_plain("it has . or .. segments"));
    }
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => match prefix.kind() {
                Prefix::Disk(_)
                | Prefix::VerbatimDisk(_)
                | Prefix::UNC(..)
                | Prefix::VerbatimUNC(..) => {}
                Prefix::DeviceNS(_) | Prefix::Verbatim(_) => {
                    return Err(not_plain("device paths are not accepted"));
                }
            },
            Component::RootDir => {}
            Component::CurDir | Component::ParentDir => {
                return Err(not_plain("it has . or .. segments"));
            }
            Component::Normal(name) => {
                let name = name.to_string_lossy();
                if name.ends_with(['.', ' ']) || name.contains(':') {
                    return Err(not_plain(
                        "a name ends with a dot or space or names a stream",
                    ));
                }
                if is_reserved_name(&name) {
                    return Err(not_plain("a name is reserved for a device"));
                }
            }
        }
    }
    match (path.parent(), path.file_name()) {
        (Some(folder), Some(name)) => Ok((folder, name)),
        _ => Err(not_plain("it does not name a file")),
    }
}

/// Whether `name` is reserved for a device by Win32 (`NUL`, `CON`,
/// `PRN`, `AUX`, `COM0`-`COM9`, `LPT0`-`LPT9`, `CONIN$`, `CONOUT$`, the
/// superscript `COM¹`-`LPT³` too): case-insensitive, with any extension
/// and spaces before it, since `nul.txt` and `CON .json` open the device
/// too. Behind a verbatim `\\?\` prefix such a name is a plain file
/// that most programs cannot open or delete, so it is refused as well.
fn is_reserved_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).trim_end_matches(' ');
    let upper = stem.to_ascii_uppercase();
    match upper.as_str() {
        "NUL" | "CON" | "PRN" | "AUX" | "CONIN$" | "CONOUT$" => true,
        _ => upper
            .strip_prefix("COM")
            .or_else(|| upper.strip_prefix("LPT"))
            .is_some_and(|digit| {
                matches!(
                    digit,
                    "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            }),
    }
}

/// Whether `path` has a `.` or `..` segment, between any separators
/// (`std::path` drops some `.` segments before they could be seen).
pub(crate) fn has_dot_segments(path: &Path) -> bool {
    path.components()
        .any(|c| matches!(c, Component::CurDir | Component::ParentDir))
        || path
            .to_string_lossy()
            .split(['\\', '/'])
            .any(|segment| segment == "." || segment == "..")
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

    use super::{PathScope, UserPaths, has_dot_segments, is_reserved_name, simplified};

    /// A scope of explicitly allowed paths and folders, for tests.
    #[derive(Debug, Default)]
    pub(crate) struct TestScope {
        pub(crate) allowed: Mutex<Vec<PathBuf>>,
    }

    impl PathScope for TestScope {
        fn allows(&self, path: &Path) -> bool {
            // Like the asset scope: `Path::starts_with` compares the text
            // of the components, so `<chosen>\..\x` would start with
            // `<chosen>`.
            !has_dot_segments(path)
                && self
                    .allowed
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

    #[test]
    fn the_test_scope_refuses_dot_segments() {
        let scope = TestScope::default();
        scope.add_file(Path::new(r"C:\chosen"));
        assert!(scope.allows(Path::new(r"C:\chosen\new.json")));
        assert!(!scope.allows(Path::new(r"C:\chosen\..\other\new.json")));
        assert!(!scope.allows(Path::new(r"C:\chosen/../other/new.json")));
        assert!(!scope.allows(Path::new(r"C:\other\new.json")));
    }

    /// Save targets stay inside the chosen folder whatever the spelling.
    #[test]
    fn save_targets_cannot_leave_the_chosen_folder() {
        let dir = std::env::temp_dir().join(format!("blurrd-targets-{}", std::process::id()));
        let chosen = dir.join("chosen");
        let other = dir.join("other");
        std::fs::create_dir_all(chosen.join("sub")).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        let paths = paths_below(&chosen);
        let chosen = simplified(&chosen.canonicalize().unwrap());
        let other = simplified(&other.canonicalize().unwrap());
        let text = chosen.to_string_lossy().into_owned();

        // Plain targets inside the folder, also with forward slashes, in
        // the verbatim form and over an existing file.
        for good in [
            format!(r"{text}\new.json"),
            format!(r"{text}\sub\new.json"),
            format!("{}/sub/new.json", text.replace('\\', "/")),
            format!(r"\\?\{text}\new.json"),
        ] {
            let target = paths
                .target(Path::new(&good))
                .unwrap_or_else(|e| panic!("{good}: {e}"));
            assert!(target.starts_with(&chosen), "{good}: {}", target.display());
        }
        std::fs::write(chosen.join("old.json"), b"{}").unwrap();
        assert_eq!(
            paths.target(&chosen.join("old.json")).unwrap(),
            chosen.join("old.json")
        );

        let escapes = [
            format!(r"{text}\..\other\new.json"),
            format!(r"{text}\..\..\new.json"),
            format!("{text}/../other/new.json"),
            format!(r"{text}/..\other/new.json"),
            format!(r"{text}\sub\..\..\other\new.json"),
            format!(r"{text}\.\new.json"),
            format!(r"{text}\.. \other\new.json"),
            format!(r"{text}\..\other.\new.json"),
            format!(r"{text}\...\new.json"),
            format!(r"{text}\sub.\new.json"),
            format!(r"{text}\new.json."),
            format!(r"{text}\new.json "),
            format!(r"{text}\new.json:stream"),
            format!(r"\\?\{text}\..\other\new.json"),
            format!(r"\\.\{text}\new.json"),
            r"\\server\share\x\..\new.json".to_owned(),
            r"\\?\UNC\server\share\x\..\new.json".to_owned(),
            format!(r"{text}\missing\new.json"),
            format!(r"{}\new.json", other.display()),
            "new.json".to_owned(),
            format!(r"{text}\"),
        ];
        for bad in &escapes {
            assert!(paths.target(Path::new(bad)).is_err(), "{bad} was accepted");
        }
        assert!(!other.join("new.json").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn device_names_are_reserved_in_any_spelling() {
        for reserved in [
            "NUL",
            "nul",
            "Nul.json",
            "CON",
            "con.txt",
            "CON .json",
            "PRN.mp4",
            "aux.tar.gz",
            "COM0",
            "com1",
            "COM9.json",
            "LPT0",
            "lpt5.mkv",
            "LPT9",
            "CONIN$",
            "conout$.json",
            "COM¹",
            "lpt³.json",
        ] {
            assert!(is_reserved_name(reserved), "{reserved}");
        }
        for plain in [
            "NULL.json",
            "nul_preset.json",
            "console.json",
            "CON-1.json",
            "auxiliary.json",
            "COM",
            "COM10.json",
            "LPT.json",
            "com1x.json",
            "xCON.json",
            "CONIN.json",
            "é.json",
        ] {
            assert!(!is_reserved_name(plain), "{plain}");
        }
    }

    /// A save target never names a device, also behind `\?\`.
    #[test]
    fn save_targets_cannot_name_a_device() {
        let dir = std::env::temp_dir().join(format!("blurrd-devices-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let paths = paths_below(&dir);
        let text = simplified(&dir.canonicalize().unwrap())
            .to_string_lossy()
            .into_owned();
        for name in [
            "NUL",
            "nul.json",
            "Con.json",
            "CON .json",
            "COM1.json",
            "lpt9",
            "CONIN$",
        ] {
            for spelling in [format!(r"{text}\{name}"), format!(r"\?\{text}\{name}")] {
                let refused = paths.target(Path::new(&spelling)).unwrap_err();
                assert_eq!(refused.kind, ErrorKind::Io, "{spelling}");
            }
        }
        assert!(paths.target(&dir.join("null.json")).is_ok());
        assert!(paths.target(&dir.join("COM10.json")).is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
