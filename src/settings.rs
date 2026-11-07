//! Reader preferences and atomic, per-user settings storage.

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

pub const MAX_RECENT_BOOKS: usize = 15;
const MAX_SETTINGS_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FitMode {
    #[default]
    Window,
    Width,
    Height,
    Original,
    Custom,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadingMode {
    #[default]
    Single,
    Spread,
    Continuous,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SidebarMode {
    Files,
    #[default]
    Thumbnails,
    Recent,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Background {
    #[default]
    Black,
    Gray,
    Paper,
}

/// Zoom, rotation, and image filters are runtime state, not preferences.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Preferences {
    pub mode: ReadingMode,
    pub rtl: bool,
    pub fit: FitMode,
    pub sidebar: bool,
    pub sidebar_mode: SidebarMode,
    pub background: Background,
    pub recursive_folder: bool,
    pub remember_position: bool,
    /// Slideshow interval in seconds, limited to 1 through 120.
    pub interval: u64,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            mode: ReadingMode::Single,
            rtl: true,
            fit: FitMode::Window,
            sidebar: true,
            sidebar_mode: SidebarMode::Thumbnails,
            background: Background::Black,
            recursive_folder: true,
            remember_position: true,
            interval: 5,
        }
    }
}

impl Preferences {
    pub fn validate(&mut self) {
        self.interval = self.interval.clamp(1, 120);
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppSettings {
    pub preferences: Preferences,
    /// Most recently opened book first. Paths are not required to still exist.
    pub recent: Vec<PathBuf>,
    /// Page indices are zero-based and are checked against a book when opened.
    pub positions: HashMap<PathBuf, usize>,
    pub bookmarks: HashMap<PathBuf, usize>,
}

/// Return the current user's configuration file, without creating it.
pub fn settings_path() -> Result<PathBuf, String> {
    ProjectDirs::from("org", "MangaMeeya", "MangaMeeya Rust")
        .map(|dirs| dirs.config_dir().join("settings.json"))
        .ok_or_else(|| "Cannot find the user configuration directory".to_owned())
}

impl AppSettings {
    pub fn config_path() -> Result<PathBuf, String> {
        settings_path()
    }

    pub fn load() -> Result<Self, String> {
        Self::load_from(&settings_path()?)
    }

    /// A missing file uses defaults. Other errors leave the file unchanged.
    /// The caller must report errors instead of silently discarding them.
    pub fn load_from(path: &Path) -> Result<Self, String> {
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(format!("Cannot read settings {}: {error}", path.display())),
        };
        let mut bytes = Vec::new();
        file.take(MAX_SETTINGS_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("Cannot read settings {}: {error}", path.display()))?;
        if bytes.len() as u64 > MAX_SETTINGS_BYTES {
            return Err(format!(
                "Settings {} exceed the 8 MiB limit; the file was not changed",
                path.display()
            ));
        }
        let invalid = |error: String| {
            format!(
                "Invalid settings {}: {error}. The file was not changed; use an explicit backup and reset to recover it",
                path.display()
            )
        };
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|error| invalid(error.to_string()))?;
        if !value.is_object()
            || value
                .get("preferences")
                .is_some_and(|preferences| !preferences.is_object())
        {
            return Err(invalid(
                "Settings and preferences must be JSON objects".to_owned(),
            ));
        }
        let mut settings: Self =
            serde_json::from_slice(&bytes).map_err(|error| invalid(error.to_string()))?;
        settings.validate();
        Ok(settings)
    }

    pub fn save(&self) -> Result<(), String> {
        self.save_to(&settings_path()?)
    }

    /// Replace the file atomically. An invalid existing file is never replaced.
    /// This also protects data if a caller uses defaults after a load error.
    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        Self::load_from(path)?;
        self.write_atomic(path)
    }

    /// Call only after an explicit user request. Preserve the complete invalid
    /// file in a unique same-directory backup, then atomically install defaults.
    /// Return the backup path. Valid or missing settings cannot be reset here.
    pub fn reset_corrupt_at(path: &Path) -> Result<PathBuf, String> {
        let mut original = File::open(path).map_err(|error| {
            format!(
                "Cannot open settings {} for backup: {error}",
                path.display()
            )
        })?;
        if !original
            .metadata()
            .map_err(|error| error.to_string())?
            .is_file()
        {
            return Err("Only a regular settings file can be backed up and reset".to_owned());
        }
        if Self::load_from(path).is_ok() {
            return Err("Settings are valid; a corruption reset is not required".to_owned());
        }
        let parent = settings_parent(path);
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("settings.json");
        let mut backup = tempfile::Builder::new()
            .prefix(&format!("{name}.corrupt-"))
            .tempfile_in(parent)
            .map_err(|error| format!("Cannot create settings backup: {error}"))?;
        std::io::copy(&mut original, &mut backup)
            .and_then(|_| backup.as_file().sync_all())
            .map_err(|error| format!("Cannot write settings backup: {error}"))?;
        // Windows cannot replace the settings path while its read handle is open.
        drop(original);
        // Keep even oversized corrupt files intact without loading them into RAM.
        let (_, backup_path) = backup
            .keep()
            .map_err(|error| format!("Cannot preserve settings backup: {error}"))?;
        sync_directory(parent)?;
        if Self::load_from(&backup_path).is_ok() {
            return Err(format!(
                "Settings changed during backup; reset was canceled. Backup: {}",
                backup_path.display()
            ));
        }
        Self::default().write_atomic(path).map_err(|error| {
            format!(
                "{error}. Original settings backup: {}",
                backup_path.display()
            )
        })?;
        Ok(backup_path)
    }

    fn write_atomic(&self, path: &Path) -> Result<(), String> {
        let mut settings = self.clone();
        settings.validate();
        let mut bytes = serde_json::to_vec_pretty(&settings)
            .map_err(|error| format!("Cannot encode settings: {error}"))?;
        bytes.push(b'\n');
        if bytes.len() as u64 > MAX_SETTINGS_BYTES {
            return Err("Settings exceed the 8 MiB limit".to_owned());
        }

        let parent = settings_parent(path);
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Cannot create settings directory {}: {error}",
                parent.display()
            )
        })?;
        let mut temporary = NamedTempFile::new_in(parent)
            .map_err(|error| format!("Cannot create temporary settings file: {error}"))?;
        temporary
            .write_all(&bytes)
            .and_then(|()| temporary.as_file().sync_all())
            .map_err(|error| format!("Cannot write settings {}: {error}", path.display()))?;
        temporary
            .persist(path)
            .map_err(|error| format!("Cannot replace settings {}: {error}", path.display()))?;
        sync_directory(parent)
    }

    pub fn validate(&mut self) {
        self.preferences.validate();
        let mut seen = HashSet::new();
        self.recent
            .retain(|path| !path.as_os_str().is_empty() && seen.insert(path.clone()));
        self.recent.truncate(MAX_RECENT_BOOKS);
        self.positions
            .retain(|path, _| !path.as_os_str().is_empty());
        self.bookmarks
            .retain(|path, _| !path.as_os_str().is_empty());
    }

    pub fn add_recent(&mut self, path: &Path) {
        if path.as_os_str().is_empty() {
            return;
        }
        self.recent.retain(|existing| existing != path);
        self.recent.insert(0, path.to_path_buf());
        self.recent.truncate(MAX_RECENT_BOOKS);
    }
}

fn settings_parent(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn sync_directory(_parent: &Path) -> Result<(), String> {
    // A directory sync makes renames durable on Unix. Windows cannot open a
    // directory with File::open; the temporary file itself is synced above.
    #[cfg(unix)]
    File::open(_parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("Cannot sync settings directory: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_reader_behavior() {
        let preferences = Preferences::default();
        assert_eq!(preferences.mode, ReadingMode::Single);
        assert!(preferences.rtl);
        assert_eq!(preferences.fit, FitMode::Window);
        assert!(preferences.sidebar);
        assert_eq!(preferences.sidebar_mode, SidebarMode::Thumbnails);
        assert_eq!(preferences.background, Background::Black);
        assert!(preferences.recursive_folder);
        assert!(preferences.remember_position);
        assert_eq!(preferences.interval, 5);
    }

    #[test]
    fn missing_file_does_not_create_configuration() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("missing/settings.json");
        assert_eq!(
            AppSettings::load_from(&path).unwrap(),
            AppSettings::default()
        );
        assert!(!path.parent().unwrap().exists());
    }

    #[test]
    fn settings_round_trip_and_replace_atomically() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config/settings.json");
        let book = PathBuf::from("books/日本語/book.cbz");
        let mut settings = AppSettings::default();
        settings.preferences.mode = ReadingMode::Continuous;
        settings.preferences.background = Background::Paper;
        settings.add_recent(&book);
        settings.positions.insert(book.clone(), 12);
        settings.bookmarks.insert(book, 8);
        settings.save_to(&path).unwrap();
        assert_eq!(AppSettings::load_from(&path).unwrap(), settings);
        settings.preferences.mode = ReadingMode::Spread;
        settings.save_to(&path).unwrap();
        assert_eq!(AppSettings::load_from(&path).unwrap(), settings);
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
    }

    #[test]
    fn partial_settings_use_defaults_and_clamp_known_values() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        fs::write(&path, r#"{"preferences":{"interval":0,"rtl":false}}"#).unwrap();
        let settings = AppSettings::load_from(&path).unwrap();
        assert_eq!(settings.preferences.interval, 1);
        assert!(!settings.preferences.rtl);
        assert_eq!(settings.preferences.sidebar_mode, SidebarMode::Thumbnails);
        let mut settings = settings;
        settings.preferences.interval = u64::MAX;
        settings.save_to(&path).unwrap();
        assert_eq!(
            AppSettings::load_from(&path).unwrap().preferences.interval,
            120
        );
        // Saving validation does not change the caller's in-memory state.
        assert_eq!(settings.preferences.interval, u64::MAX);
    }

    #[test]
    fn recent_books_are_deduplicated_and_limited() {
        let mut settings = AppSettings::default();
        for index in 0..20 {
            settings.add_recent(Path::new(&format!("book-{index}.cbz")));
        }
        settings.add_recent(Path::new("book-10.cbz"));
        assert_eq!(settings.recent.len(), MAX_RECENT_BOOKS);
        assert_eq!(settings.recent[0], Path::new("book-10.cbz"));
        assert_eq!(settings.recent[1], Path::new("book-19.cbz"));
        settings.recent.push(PathBuf::from("book-10.cbz"));
        settings.recent.insert(0, PathBuf::new());
        settings.positions.insert(PathBuf::new(), 1);
        settings.bookmarks.insert(PathBuf::new(), 1);
        settings.validate();
        assert_eq!(settings.recent.len(), MAX_RECENT_BOOKS);
        assert_eq!(settings.recent[0], Path::new("book-10.cbz"));
        assert!(settings.positions.is_empty());
        assert!(settings.bookmarks.is_empty());
    }

    #[test]
    fn corrupt_data_is_preserved_even_after_default_fallback() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        for bytes in [
            b"{".as_slice(),
            b"null".as_slice(),
            b"[]".as_slice(),
            br#"{"preferences":[]}"#.as_slice(),
            br#"{"preferences":{},"preferences":{}}"#.as_slice(),
            br#"{"unknown_user_data":"preserve"}"#.as_slice(),
            br#"{"preferences":{"future_option":true}}"#.as_slice(),
            br#"{"preferences":{"fit":"unknown"}}"#.as_slice(),
            br#"{"positions":{"book.cbz":-1}}"#.as_slice(),
            br#"{"preferences":{"interval":"five"}}"#.as_slice(),
            &[0xff, 0xfe],
        ] {
            fs::write(&path, bytes).unwrap();
            assert!(AppSettings::load_from(&path).is_err());
            assert!(AppSettings::default().save_to(&path).is_err());
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn oversized_settings_are_not_overwritten() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let bytes = vec![b' '; MAX_SETTINGS_BYTES as usize + 1];
        fs::write(&path, &bytes).unwrap();
        assert!(AppSettings::load_from(&path).unwrap_err().contains("8 MiB"));
        assert!(AppSettings::default().save_to(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }

    #[test]
    fn invalid_settings_location_reports_an_error() {
        let directory = tempfile::tempdir().unwrap();
        assert!(AppSettings::load_from(directory.path()).is_err());
        assert!(AppSettings::default().save_to(directory.path()).is_err());
        let file = directory.path().join("not-a-directory");
        fs::write(&file, b"content").unwrap();
        assert!(
            AppSettings::default()
                .save_to(&file.join("settings.json"))
                .is_err()
        );
        assert_eq!(fs::read(file).unwrap(), b"content");
    }

    #[test]
    fn explicit_corruption_reset_keeps_unique_exact_backups() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        assert!(AppSettings::reset_corrupt_at(&path).is_err());
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
        fs::write(&path, b"{broken settings").unwrap();
        let first = AppSettings::reset_corrupt_at(&path).unwrap();
        assert_eq!(first.parent(), path.parent());
        assert!(
            first
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("settings.json.corrupt-")
        );
        assert_eq!(fs::read(&first).unwrap(), b"{broken settings");
        assert_eq!(
            AppSettings::load_from(&path).unwrap(),
            AppSettings::default()
        );
        assert!(AppSettings::reset_corrupt_at(&path).is_err());
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
        fs::write(&path, b"second corrupt file").unwrap();
        let second = AppSettings::reset_corrupt_at(&path).unwrap();
        assert_ne!(first, second);
        assert_eq!(fs::read(&first).unwrap(), b"{broken settings");
        assert_eq!(fs::read(second).unwrap(), b"second corrupt file");
        assert_eq!(
            AppSettings::load_from(&path).unwrap(),
            AppSettings::default()
        );
    }

    #[test]
    fn explicit_reset_preserves_oversized_data_without_truncating_it() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let bytes = vec![b'x'; MAX_SETTINGS_BYTES as usize + 1];
        fs::write(&path, &bytes).unwrap();
        let backup = AppSettings::reset_corrupt_at(&path).unwrap();
        assert_eq!(fs::read(backup).unwrap(), bytes);
        assert_eq!(
            AppSettings::load_from(&path).unwrap(),
            AppSettings::default()
        );
    }

    #[cfg(unix)]
    #[test]
    fn non_unicode_path_encoding_failure_preserves_previous_file() {
        use std::os::unix::ffi::OsStringExt;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        AppSettings::default().save_to(&path).unwrap();
        let before = fs::read(&path).unwrap();
        let mut settings = AppSettings::default();
        settings
            .positions
            .insert(PathBuf::from(std::ffi::OsString::from_vec(vec![0xff])), 10);
        assert!(settings.save_to(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
