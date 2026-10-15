//! Presets (architecture plan 6.5): the built-ins of `Preset::builtins`
//! and the user's presets in `presets.json` (schema `blurrd-presets/1`)
//! in the app config folder. Exported files use the same schema and leave
//! out the names kept readable unless the user includes them.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use blurrd_core::{AppError, ErrorKind, Preset, PresetFile, schema};

use crate::projects::lock;

/// The user's presets.
#[derive(Debug)]
pub(crate) struct Presets {
    file: PathBuf,
    /// Held while the file is read and written.
    guard: Mutex<()>,
}

impl Presets {
    /// Presets stored in `file`.
    pub(crate) fn new(file: PathBuf) -> Self {
        Self {
            file,
            guard: Mutex::new(()),
        }
    }

    /// Every preset, built-ins first.
    ///
    /// # Errors
    ///
    /// The I/O and schema errors of reading the file (a missing file holds
    /// no user presets).
    pub(crate) fn list(&self) -> Result<Vec<Preset>, AppError> {
        let _guard = lock(&self.guard);
        self.read().map(with_builtins)
    }

    /// Adds `preset` or replaces the user preset with its id; answers the
    /// new list.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidProject`] for a built-in id or an empty name,
    /// I/O and schema errors of the file.
    pub(crate) fn save(&self, mut preset: Preset) -> Result<Vec<Preset>, AppError> {
        if is_builtin(&preset.id) {
            return Err(invalid("Built-in presets cannot be changed."));
        }
        if preset.name.trim().is_empty() {
            return Err(invalid("The preset needs a name."));
        }
        preset.builtin = false;
        let _guard = lock(&self.guard);
        let mut user = self.read()?;
        match user.iter_mut().find(|existing| existing.id == preset.id) {
            Some(existing) => *existing = preset,
            None => user.push(preset),
        }
        self.write(&user)?;
        Ok(with_builtins(user))
    }

    /// Deletes user preset `id`; answers the new list.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidProject`] for a built-in id, I/O and schema
    /// errors of the file.
    pub(crate) fn delete(&self, id: &str) -> Result<Vec<Preset>, AppError> {
        if is_builtin(id) {
            return Err(invalid("Built-in presets cannot be deleted."));
        }
        let _guard = lock(&self.guard);
        let mut user = self.read()?;
        user.retain(|preset| preset.id != id);
        self.write(&user)?;
        Ok(with_builtins(user))
    }

    /// Adds the presets of the preset file at `path` and answers them; ids
    /// that are taken get a numbered one.
    ///
    /// # Errors
    ///
    /// I/O errors, and [`ErrorKind::InvalidProject`] for a file that is not
    /// a blurrd preset file.
    pub(crate) fn import(&self, path: &Path) -> Result<Vec<Preset>, AppError> {
        let imported = read_file(path)?;
        let _guard = lock(&self.guard);
        let mut user = self.read()?;
        let mut taken: Vec<String> = with_builtins(user.clone())
            .into_iter()
            .map(|preset| preset.id)
            .collect();
        let mut added = Vec::with_capacity(imported.presets.len());
        for mut preset in imported.presets {
            let id = (1..=u32::MAX)
                .map(|n| {
                    if n == 1 {
                        preset.id.clone()
                    } else {
                        format!("{}-{n}", preset.id)
                    }
                })
                .find(|id| !taken.contains(id))
                .unwrap_or_default();
            taken.push(id.clone());
            preset.id = id;
            preset.builtin = false;
            added.push(preset);
        }
        user.extend(added.iter().cloned());
        self.write(&user)?;
        Ok(added)
    }

    /// Writes preset `id` to `path`, without the names kept readable unless
    /// `include_kept_names` is set.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::NotFound`] for an unknown id, I/O errors of the file.
    pub(crate) fn export(
        &self,
        id: &str,
        path: &Path,
        include_kept_names: bool,
    ) -> Result<(), AppError> {
        let mut preset = self
            .list()?
            .into_iter()
            .find(|preset| preset.id == id)
            .ok_or_else(|| {
                AppError::new(ErrorKind::NotFound, "The preset does not exist.")
                    .with_detail(id.to_owned())
            })?;
        if !include_kept_names {
            preset.regions.keep_names.names.clear();
        }
        write_file(path, &PresetFile::new(vec![preset]))
    }

    fn read(&self) -> Result<Vec<Preset>, AppError> {
        if !self.file.is_file() {
            return Ok(Vec::new());
        }
        Ok(read_file(&self.file)?.presets)
    }

    fn write(&self, user: &[Preset]) -> Result<(), AppError> {
        if let Some(folder) = self.file.parent() {
            std::fs::create_dir_all(folder).map_err(|e| io(&e, folder))?;
        }
        write_file(&self.file, &PresetFile::new(user.to_vec()))
    }
}

/// The built-ins followed by `user`.
fn with_builtins(user: Vec<Preset>) -> Vec<Preset> {
    let mut all = Preset::builtins();
    all.extend(user);
    all
}

fn is_builtin(id: &str) -> bool {
    Preset::builtins().iter().any(|preset| preset.id == id)
}

fn read_file(path: &Path) -> Result<PresetFile, AppError> {
    let text = std::fs::read_to_string(path).map_err(|e| io(&e, path))?;
    schema::from_json(&text).map_err(|e| {
        AppError::new(
            ErrorKind::InvalidProject,
            "This is not a blurrd preset file.",
        )
        .with_detail(e.to_string())
    })
}

/// Writes `file` to `path` through a temporary file, so a reader never
/// sees half a file.
fn write_file(path: &Path, file: &PresetFile) -> Result<(), AppError> {
    let text = schema::to_json_pretty(file)?;
    let mut temp = path.as_os_str().to_owned();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);
    std::fs::write(&temp, text)
        .and_then(|()| std::fs::rename(&temp, path))
        .map_err(|e| {
            let _ = std::fs::remove_file(&temp);
            io(&e, path)
        })
}

fn io(error: &std::io::Error, path: &Path) -> AppError {
    let kind = if error.kind() == std::io::ErrorKind::NotFound {
        ErrorKind::NotFound
    } else {
        ErrorKind::Io
    };
    AppError::new(kind, "The preset file could not be read or written.")
        .with_detail(format!("{}: {error}", path.display()))
}

fn invalid(message: &str) -> AppError {
    AppError::new(ErrorKind::InvalidProject, message)
}

#[cfg(test)]
mod tests {
    use blurrd_core::{ErrorKind, Preset};

    use super::Presets;

    fn temp(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("blurrd-presets-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn user(id: &str) -> Preset {
        let mut preset = Preset::builtins().remove(0);
        preset.id = id.to_owned();
        preset.name = format!("Mine {id}");
        preset.builtin = true;
        preset.regions.keep_names.names = vec!["SomeName".to_owned()];
        preset
    }

    #[test]
    fn user_presets_follow_the_builtins_and_persist() {
        let dir = temp("persist");
        let presets = Presets::new(dir.join("presets.json"));
        let builtins = Preset::builtins().len();
        assert_eq!(presets.list().unwrap().len(), builtins);
        let list = presets.save(user("a")).unwrap();
        assert_eq!(list.len(), builtins + 1);
        assert!(!list[builtins].builtin);
        let reopened = Presets::new(dir.join("presets.json"));
        assert_eq!(reopened.list().unwrap(), list);
        assert_eq!(reopened.delete("a").unwrap().len(), builtins);
        assert_eq!(
            presets.save(Preset::builtins().remove(0)).unwrap_err().kind,
            ErrorKind::InvalidProject
        );
        assert_eq!(
            presets.delete("default").unwrap_err().kind,
            ErrorKind::InvalidProject
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn exports_leave_out_kept_names_and_imports_rename_taken_ids() {
        let dir = temp("exchange");
        let presets = Presets::new(dir.join("presets.json"));
        presets.save(user("a")).unwrap();
        let file = dir.join("a.blurrd-preset.json");
        presets.export("a", &file, false).unwrap();
        let added = presets.import(&file).unwrap();
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].id, "a-2");
        assert_eq!(added[0].regions.keep_names.names, Vec::<String>::new());
        presets.export("a", &file, true).unwrap();
        let again = presets.import(&file).unwrap();
        assert_eq!(again[0].id, "a-3");
        assert_eq!(
            again[0].regions.keep_names.names,
            vec!["SomeName".to_owned()]
        );
        std::fs::write(&file, "{}").unwrap();
        assert_eq!(
            presets.import(&file).unwrap_err().kind,
            ErrorKind::InvalidProject
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
