//! Reader preferences and atomic, per-user settings storage.

use crate::i18n::Language;
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct PanelLayoutPreferences {
    /// Preferred sidebar width in logical pixels, limited to 216 through 320.
    pub sidebar_width: u32,
    /// Preferred inspector width in logical pixels, limited to 240 through 360.
    pub inspector_width: u32,
    pub inspector_visible: bool,
}

impl Default for PanelLayoutPreferences {
    fn default() -> Self {
        Self {
            sidebar_width: 224,
            inspector_width: 256,
            inspector_visible: true,
        }
    }
}

impl PanelLayoutPreferences {
    pub fn validate(&mut self) {
        self.sidebar_width = self.sidebar_width.clamp(216, 320);
        self.inspector_width = self.inspector_width.clamp(240, 360);
    }
}

/// Zoom, rotation, and image filters are runtime state, not preferences.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Preferences {
    pub language: Language,
    pub mode: ReadingMode,
    pub rtl: bool,
    pub fit: FitMode,
    pub sidebar: bool,
    pub sidebar_mode: SidebarMode,
    pub panel_layout: PanelLayoutPreferences,
    pub background: Background,
    pub recursive_folder: bool,
    pub remember_position: bool,
    /// Slideshow interval in seconds, limited to 1 through 120.
    pub interval: u64,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            language: Language::English,
            mode: ReadingMode::Single,
            rtl: true,
            fit: FitMode::Window,
            sidebar: true,
            sidebar_mode: SidebarMode::Thumbnails,
            panel_layout: PanelLayoutPreferences::default(),
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
        self.panel_layout.validate();
    }
}

/// Folder-image identity bound to the numeric bookmark saved with it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BookmarkImage {
    pub page: usize,
    #[serde(with = "stored_paths::path")]
    pub image: PathBuf,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppSettings {
    pub preferences: Preferences,
    /// Most recently opened book first. Paths are not required to still exist.
    #[serde(with = "stored_paths::recent")]
    pub recent: Vec<PathBuf>,
    /// Page indices are zero-based and are checked against a book when opened.
    #[serde(with = "stored_paths::map")]
    pub positions: HashMap<PathBuf, usize>,
    #[serde(with = "stored_paths::map")]
    pub bookmarks: HashMap<PathBuf, usize>,
    #[serde(with = "stored_paths::map", skip_serializing_if = "HashMap::is_empty")]
    pub bookmark_images: HashMap<PathBuf, BookmarkImage>,
}

// UTF-8 paths retain the legacy JSON format. Native paths use exactly one
// {"unix": [u8]} or {"windows": [u16]} encoding, with no reserved string prefixes.
// Each path map stays a JSON object unless it needs native keys; then it uses
// [path, value] pairs. A foreign native encoding is an error, never a lossy path.
mod stored_paths {
    use serde::de::{self, MapAccess, SeqAccess, Visitor};
    use serde::ser::{SerializeMap, SerializeSeq};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::collections::HashMap;
    use std::fmt;
    use std::path::{Path, PathBuf};

    struct PathRef<'a>(&'a Path);

    impl Serialize for PathRef<'_> {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            if let Some(path) = self.0.to_str() {
                return serializer.serialize_str(path);
            }
            #[cfg(any(unix, windows))]
            {
                let mut map = serializer.serialize_map(Some(1))?;
                #[cfg(unix)]
                {
                    use std::os::unix::ffi::OsStrExt;
                    map.serialize_entry("unix", self.0.as_os_str().as_bytes())?;
                }
                #[cfg(windows)]
                {
                    use std::os::windows::ffi::OsStrExt;
                    let units: Vec<u16> = self.0.as_os_str().encode_wide().collect();
                    map.serialize_entry("windows", &units)?;
                }
                map.end()
            }
            #[cfg(not(any(unix, windows)))]
            Err(serde::ser::Error::custom(
                "Native paths are supported only on Unix and Windows",
            ))
        }
    }

    struct PathValue(PathBuf);

    impl<'de> Deserialize<'de> for PathValue {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            struct PathVisitor;

            impl<'de> Visitor<'de> for PathVisitor {
                type Value = PathValue;

                fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                    formatter.write_str("a UTF-8 path string or a native path object")
                }

                fn visit_str<E: de::Error>(self, path: &str) -> Result<Self::Value, E> {
                    Ok(PathValue(path.into()))
                }

                fn visit_string<E: de::Error>(self, path: String) -> Result<Self::Value, E> {
                    Ok(PathValue(path.into()))
                }

                fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                    let encoding = map.next_key::<String>()?.ok_or_else(|| {
                        de::Error::custom("Native path must contain one unix or windows encoding")
                    })?;
                    let path = match encoding.as_str() {
                        "unix" => {
                            let bytes = map.next_value::<Vec<u8>>()?;
                            #[cfg(unix)]
                            {
                                use std::os::unix::ffi::OsStringExt;
                                std::ffi::OsString::from_vec(bytes).into()
                            }
                            #[cfg(not(unix))]
                            {
                                let _ = bytes;
                                return Err(de::Error::custom(
                                    "Unix native paths cannot be loaded on this platform",
                                ));
                            }
                        }
                        "windows" => {
                            let units = map.next_value::<Vec<u16>>()?;
                            #[cfg(windows)]
                            {
                                use std::os::windows::ffi::OsStringExt;
                                std::ffi::OsString::from_wide(&units).into()
                            }
                            #[cfg(not(windows))]
                            {
                                let _ = units;
                                return Err(de::Error::custom(
                                    "Windows native paths cannot be loaded on this platform",
                                ));
                            }
                        }
                        _ => {
                            return Err(de::Error::unknown_field(&encoding, &["unix", "windows"]));
                        }
                    };
                    if map.next_key::<de::IgnoredAny>()?.is_some() {
                        return Err(de::Error::custom(
                            "Native path must contain exactly one encoding and no other fields",
                        ));
                    }
                    Ok(PathValue(path))
                }
            }

            deserializer.deserialize_any(PathVisitor)
        }
    }

    pub(super) mod recent {
        use super::*;

        pub fn serialize<S: Serializer>(
            paths: &[PathBuf],
            serializer: S,
        ) -> Result<S::Ok, S::Error> {
            let mut sequence = serializer.serialize_seq(Some(paths.len()))?;
            for path in paths {
                sequence.serialize_element(&PathRef(path))?;
            }
            sequence.end()
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(
            deserializer: D,
        ) -> Result<Vec<PathBuf>, D::Error> {
            Vec::<PathValue>::deserialize(deserializer)
                .map(|paths| paths.into_iter().map(|path| path.0).collect())
        }
    }

    pub(super) mod path {
        use super::*;

        pub fn serialize<S: Serializer>(path: &Path, serializer: S) -> Result<S::Ok, S::Error> {
            PathRef(path).serialize(serializer)
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(
            deserializer: D,
        ) -> Result<PathBuf, D::Error> {
            PathValue::deserialize(deserializer).map(|path| path.0)
        }
    }

    pub(super) mod map {
        use super::*;
        use std::marker::PhantomData;

        pub fn serialize<S: Serializer, T: Serialize>(
            values: &HashMap<PathBuf, T>,
            serializer: S,
        ) -> Result<S::Ok, S::Error> {
            if values.keys().all(|path| path.to_str().is_some()) {
                return values.serialize(serializer);
            }
            let mut sequence = serializer.serialize_seq(Some(values.len()))?;
            for (path, value) in values {
                sequence.serialize_element(&(PathRef(path), value))?;
            }
            sequence.end()
        }

        pub fn deserialize<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
            deserializer: D,
        ) -> Result<HashMap<PathBuf, T>, D::Error> {
            struct PathMapVisitor<T>(PhantomData<T>);

            impl<'de, T: Deserialize<'de>> Visitor<'de> for PathMapVisitor<T> {
                type Value = HashMap<PathBuf, T>;

                fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                    formatter.write_str("a path-to-value object or an array of [path, value] pairs")
                }

                fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
                    HashMap::deserialize(de::value::MapAccessDeserializer::new(map))
                }

                fn visit_seq<A: SeqAccess<'de>>(
                    self,
                    mut sequence: A,
                ) -> Result<Self::Value, A::Error> {
                    let mut values = HashMap::new();
                    while let Some((PathValue(path), value)) = sequence.next_element()? {
                        values.insert(path, value);
                    }
                    Ok(values)
                }
            }

            deserializer.deserialize_any(PathMapVisitor(PhantomData))
        }
    }
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
            || value.get("preferences").is_some_and(|preferences| {
                !preferences.is_object()
                    || preferences
                        .get("panel_layout")
                        .is_some_and(|layout| !layout.is_object())
            })
        {
            return Err(invalid(
                "Settings, preferences, and panel layout must be JSON objects".to_owned(),
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
        self.bookmark_images
            .retain(|path, bookmark| self.bookmarks.get(path) == Some(&bookmark.page));
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
        assert_eq!(preferences.language, Language::English);
        assert_eq!(preferences.mode, ReadingMode::Single);
        assert!(preferences.rtl);
        assert_eq!(preferences.fit, FitMode::Window);
        assert!(preferences.sidebar);
        assert_eq!(preferences.sidebar_mode, SidebarMode::Thumbnails);
        assert_eq!(
            preferences.panel_layout,
            PanelLayoutPreferences {
                sidebar_width: 224,
                inspector_width: 256,
                inspector_visible: true,
            }
        );
        assert_eq!(preferences.background, Background::Black);
        assert!(preferences.recursive_folder);
        assert!(preferences.remember_position);
        assert_eq!(preferences.interval, 5);
    }

    #[test]
    fn panel_layout_defaults_apply_to_missing_and_partial_settings() {
        assert_eq!(
            serde_json::from_str::<PanelLayoutPreferences>("{}").unwrap(),
            PanelLayoutPreferences::default()
        );
        assert_eq!(
            serde_json::from_str::<AppSettings>("{}").unwrap(),
            AppSettings::default()
        );
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let book = PathBuf::from("books/日本語.cbz");
        for (preferences, sidebar_width, inspector_width, inspector_visible, sidebar) in [
            (r#"{}"#, 224, 256, true, true),
            (r#"{"sidebar":false}"#, 224, 256, true, false),
            (r#"{"panel_layout":{}}"#, 224, 256, true, true),
            (
                r#"{"panel_layout":{"sidebar_width":280}}"#,
                280,
                256,
                true,
                true,
            ),
            (
                r#"{"panel_layout":{"inspector_width":300}}"#,
                224,
                300,
                true,
                true,
            ),
            (
                r#"{"panel_layout":{"inspector_visible":false}}"#,
                224,
                256,
                false,
                true,
            ),
            (
                r#"{"sidebar":false,"panel_layout":{"sidebar_width":320,"inspector_width":360,"inspector_visible":true}}"#,
                320,
                360,
                true,
                false,
            ),
            (
                r#"{"sidebar":true,"panel_layout":{"sidebar_width":216,"inspector_width":240,"inspector_visible":false}}"#,
                216,
                240,
                false,
                true,
            ),
        ] {
            let source = serde_json::to_vec(&serde_json::json!({
                "preferences": serde_json::from_str::<serde_json::Value>(preferences).unwrap(),
                "recent": ["books/日本語.cbz"],
                "positions": {"books/日本語.cbz": 12},
                "bookmarks": {"books/日本語.cbz": 8}
            }))
            .unwrap();
            fs::write(&path, &source).unwrap();
            let expected = AppSettings {
                preferences: Preferences {
                    sidebar,
                    panel_layout: PanelLayoutPreferences {
                        sidebar_width,
                        inspector_width,
                        inspector_visible,
                    },
                    ..Preferences::default()
                },
                recent: vec![book.clone()],
                positions: HashMap::from([(book.clone(), 12)]),
                bookmarks: HashMap::from([(book.clone(), 8)]),
                ..AppSettings::default()
            };
            assert_eq!(AppSettings::load_from(&path).unwrap(), expected);
            assert_eq!(fs::read(&path).unwrap(), source);
            expected.save_to(&path).unwrap();
            assert_eq!(AppSettings::load_from(&path).unwrap(), expected);
            let saved: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            assert_eq!(
                saved["preferences"]["panel_layout"],
                serde_json::json!({
                    "sidebar_width": sidebar_width,
                    "inspector_width": inspector_width,
                    "inspector_visible": inspector_visible
                })
            );
            assert_eq!(saved["preferences"]["sidebar"], sidebar);
        }
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn panel_layout_widths_are_clamped_on_load_and_save() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let book = PathBuf::from("books/日本語.cbz");
        for (sidebar_width, inspector_width, expected_sidebar, expected_inspector) in [
            (0, 0, 216, 240),
            (215, 239, 216, 240),
            (216, 240, 216, 240),
            (217, 241, 217, 241),
            (224, 256, 224, 256),
            (319, 359, 319, 359),
            (320, 360, 320, 360),
            (321, 361, 320, 360),
            (u32::MAX, u32::MAX, 320, 360),
            (0, u32::MAX, 216, 360),
            (u32::MAX, 0, 320, 240),
        ] {
            let settings = AppSettings {
                preferences: Preferences {
                    sidebar: false,
                    panel_layout: PanelLayoutPreferences {
                        sidebar_width,
                        inspector_width,
                        inspector_visible: false,
                    },
                    ..Preferences::default()
                },
                recent: vec![book.clone()],
                positions: HashMap::from([(book.clone(), 12)]),
                bookmarks: HashMap::from([(book.clone(), 8)]),
                ..AppSettings::default()
            };
            let mut expected = settings.clone();
            expected.preferences.panel_layout.sidebar_width = expected_sidebar;
            expected.preferences.panel_layout.inspector_width = expected_inspector;
            let mut validated = settings.clone();
            validated.validate();
            assert_eq!(validated, expected);

            let source = serde_json::to_vec(&settings).unwrap();
            fs::write(&path, &source).unwrap();
            assert_eq!(AppSettings::load_from(&path).unwrap(), expected);
            assert_eq!(fs::read(&path).unwrap(), source);
            settings.save_to(&path).unwrap();
            assert_eq!(AppSettings::load_from(&path).unwrap(), expected);
            // Read without validation to check that saving stored clamped values.
            let saved: AppSettings = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            assert_eq!(saved, expected);
            // Saving validation must not change the caller's preferences or reading data.
            assert_eq!(serde_json::to_vec(&settings).unwrap(), source);
        }
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    fn assert_invalid_panel_layout_is_preserved(path: &Path, layout: &str) {
        let source = format!(
            r#"{{"preferences":{{"panel_layout":{layout}}},"recent":["book.cbz"],"positions":{{"book.cbz":12}},"bookmarks":{{"book.cbz":8}}}}"#
        );
        fs::write(path, &source).unwrap();
        let error = AppSettings::load_from(path).unwrap_err();
        assert!(error.contains("Invalid settings"), "{source}: {error}");
        assert!(
            error.contains("The file was not changed"),
            "{source}: {error}"
        );
        assert!(AppSettings::default().save_to(path).is_err(), "{source}");
        assert_eq!(fs::read(path).unwrap(), source.as_bytes());
    }

    #[test]
    fn invalid_panel_width_types_preserve_the_original_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        for field in ["sidebar_width", "inspector_width"] {
            for value in [
                "-1",
                "4294967296",
                "18446744073709551615",
                "0.0",
                "224.0",
                "1.5",
                "2.24e2",
                r#""224""#,
                "null",
                "true",
                "false",
                "[]",
                "{}",
            ] {
                let layout = format!(r#"{{"{field}":{value}}}"#);
                assert_invalid_panel_layout_is_preserved(&path, &layout);
            }
        }
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn invalid_panel_visibility_types_preserve_the_original_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        for value in ["null", "0", "1", "-1", "1.0", r#""true""#, "[]", "{}"] {
            let layout = format!(r#"{{"inspector_visible":{value}}}"#);
            assert_invalid_panel_layout_is_preserved(&path, &layout);
        }
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn non_object_panel_layout_preserves_the_original_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        for layout in [
            "null",
            "true",
            "false",
            "224",
            "224.0",
            r#""layout""#,
            "[]",
            "[224]",
            "[224,256,true]",
        ] {
            assert_invalid_panel_layout_is_preserved(&path, layout);
        }
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn unknown_and_duplicate_panel_fields_preserve_the_original_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        for layout in [
            r#"{"future_width":300}"#,
            r#"{"sidebar":false}"#,
            r#"{"sidebar_width":280,"inspector_width":300,"inspector_visible":false,"future_option":{"preserve":true}}"#,
            r#"{"sidebar_width":224,"sidebar_width":280}"#,
            r#"{"inspector_width":256,"inspector_width":300}"#,
            r#"{"inspector_visible":true,"inspector_visible":false}"#,
        ] {
            assert_invalid_panel_layout_is_preserved(&path, layout);
        }
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn language_defaults_for_legacy_settings_and_round_trips() {
        let legacy: AppSettings = serde_json::from_str(
            r#"{"preferences":{"interval":17},"recent":["books/日本語.cbz"]}"#,
        )
        .unwrap();
        assert_eq!(legacy.preferences.language, Language::English);
        assert_eq!(legacy.preferences.interval, 17);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        for language in Language::ALL {
            let mut settings = legacy.clone();
            settings.preferences.language = language;
            settings.save_to(&path).unwrap();
            assert_eq!(AppSettings::load_from(&path).unwrap(), settings);
        }
    }

    #[test]
    fn unsupported_language_is_not_overwritten() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let bytes = br#"{"preferences":{"language":"unknown"}}"#;
        fs::write(&path, bytes).unwrap();
        assert!(AppSettings::load_from(&path).is_err());
        assert!(AppSettings::default().save_to(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
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
        settings.preferences.sidebar = false;
        settings.preferences.panel_layout = PanelLayoutPreferences {
            sidebar_width: 288,
            inspector_width: 312,
            inspector_visible: true,
        };
        settings.add_recent(&book);
        settings.positions.insert(book.clone(), 12);
        settings.bookmarks.insert(book, 8);
        settings.save_to(&path).unwrap();
        assert_eq!(AppSettings::load_from(&path).unwrap(), settings);
        settings.preferences.mode = ReadingMode::Spread;
        settings.preferences.sidebar = true;
        settings.preferences.panel_layout.inspector_visible = false;
        settings.save_to(&path).unwrap();
        assert_eq!(AppSettings::load_from(&path).unwrap(), settings);
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
    }

    #[test]
    fn legacy_utf8_settings_keep_their_format_and_literal_path_strings() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let legacy = serde_json::json!({
            "preferences": {
                "mode": "spread",
                "rtl": false,
                "fit": "width",
                "sidebar": false,
                "sidebar_mode": "files",
                "background": "paper",
                "recursive_folder": false,
                "remember_position": false,
                "interval": 17
            },
            "recent": ["books/日本語.cbz", "unix:[255]", "windows:[55296]", "{\"unix\":[255]}"],
            "positions": {"books/日本語.cbz": 12, "unix:[255]": 3},
            "bookmarks": {"windows:[55296]": 8, "{\"unix\":[255]}": 4}
        });
        fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        let settings = AppSettings::load_from(&path).unwrap();
        assert_eq!(settings.preferences.mode, ReadingMode::Spread);
        assert_eq!(settings.recent[0], Path::new("books/日本語.cbz"));
        assert_eq!(settings.recent[1], Path::new("unix:[255]"));
        assert_eq!(settings.recent[2], Path::new("windows:[55296]"));
        assert_eq!(settings.recent[3], Path::new(r#"{"unix":[255]}"#));
        assert_eq!(settings.positions[Path::new("books/日本語.cbz")], 12);
        assert_eq!(settings.bookmarks[Path::new("windows:[55296]")], 8);
        settings.save_to(&path).unwrap();
        let saved: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let mut expected = legacy;
        expected["preferences"]["language"] = serde_json::json!("en");
        expected["preferences"]["panel_layout"] = serde_json::json!({
            "sidebar_width": 224,
            "inspector_width": 256,
            "inspector_visible": true
        });
        assert_eq!(saved, expected);
        assert!(settings.bookmark_images.is_empty());
        assert!(saved.get("bookmark_images").is_none());
        assert_eq!(AppSettings::load_from(&path).unwrap(), settings);
    }

    #[test]
    fn utf8_pair_maps_load_and_save_as_legacy_objects() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        fs::write(
            &path,
            r#"{"positions":[["books/日本語.cbz",12]],"bookmarks":[["book.cbz",8]]}"#,
        )
        .unwrap();
        let settings = AppSettings::load_from(&path).unwrap();
        assert_eq!(settings.positions[Path::new("books/日本語.cbz")], 12);
        assert_eq!(settings.bookmarks[Path::new("book.cbz")], 8);
        settings.save_to(&path).unwrap();
        let saved: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            saved["positions"],
            serde_json::json!({"books/日本語.cbz": 12})
        );
        assert_eq!(saved["bookmarks"], serde_json::json!({"book.cbz": 8}));
        assert!(settings.bookmark_images.is_empty());
        assert!(saved.get("bookmark_images").is_none());
        assert_eq!(AppSettings::load_from(&path).unwrap(), settings);
    }

    #[test]
    fn bookmark_images_utf8_round_trip_in_objects_and_pair_maps() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let book = PathBuf::from("books/日本語");
        let bookmark = BookmarkImage {
            page: 8,
            image: PathBuf::from("books/日本語/画像.png"),
        };
        for metadata in [
            serde_json::json!({"books/日本語": bookmark}),
            serde_json::json!([["books/日本語", bookmark]]),
        ] {
            let source = serde_json::to_vec(&serde_json::json!({
                "bookmarks": {"books/日本語": 8},
                "bookmark_images": metadata,
            }))
            .unwrap();
            fs::write(&path, &source).unwrap();
            let settings = AppSettings::load_from(&path).unwrap();
            assert_eq!(settings.bookmark_images[&book], bookmark);
            assert_eq!(fs::read(&path).unwrap(), source);
            settings.save_to(&path).unwrap();
            let saved: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            assert_eq!(saved["bookmarks"], serde_json::json!({"books/日本語": 8}));
            assert_eq!(
                saved["bookmark_images"],
                serde_json::json!({"books/日本語": {"page": 8, "image": "books/日本語/画像.png"}})
            );
            assert_eq!(AppSettings::load_from(&path).unwrap(), settings);
        }
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn bookmark_images_keep_only_matching_nonempty_bookmark_keys() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let mut settings = AppSettings::default();
        for (book, page) in [("", 2), ("missing/../book", 3), ("stale", 4), ("orphan", 5)] {
            let key = PathBuf::from(book);
            settings.bookmark_images.insert(
                key.clone(),
                BookmarkImage {
                    page,
                    image: PathBuf::from("absent/../image.png"),
                },
            );
            if book != "orphan" {
                settings
                    .bookmarks
                    .insert(key, if book == "stale" { 0 } else { page });
            }
        }
        let source = serde_json::to_vec(&settings).unwrap();
        fs::write(&path, &source).unwrap();
        let loaded = AppSettings::load_from(&path).unwrap();
        assert_eq!(fs::read(&path).unwrap(), source);
        settings.validate();
        assert_eq!(loaded, settings);
        assert_eq!(settings.bookmark_images.len(), 1);
        assert_eq!(
            settings.bookmark_images[Path::new("missing/../book")].page,
            3
        );
        assert_eq!(
            settings.bookmark_images[Path::new("missing/../book")].image,
            Path::new("absent/../image.png")
        );
        assert_eq!(settings.bookmarks[Path::new("stale")], 0);
        settings.bookmarks.remove(Path::new("missing/../book"));
        settings.save_to(&path).unwrap();
        let saved = AppSettings::load_from(&path).unwrap();
        assert!(saved.bookmark_images.is_empty());
        let json: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(json.get("bookmark_images").is_none());
        // Save validation does not change the caller's metadata.
        assert_eq!(settings.bookmark_images.len(), 1);
    }

    fn assert_invalid_bookmark_images_is_preserved(path: &Path, metadata: &str) {
        let source = format!(
            r#"{{"recent":["book"],"positions":{{"book":12}},"bookmarks":{{"book":8}},"bookmark_images":{metadata}}}"#
        );
        fs::write(path, &source).unwrap();
        let error = AppSettings::load_from(path).unwrap_err();
        assert!(error.contains("Invalid settings"), "{source}: {error}");
        assert!(
            error.contains("The file was not changed"),
            "{source}: {error}"
        );
        assert!(AppSettings::default().save_to(path).is_err(), "{source}");
        assert_eq!(fs::read(path).unwrap(), source.as_bytes());
    }

    #[test]
    fn malformed_bookmark_metadata_preserves_the_original_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        for page in [
            "-1",
            "18446744073709551616",
            "0.0",
            "8.0",
            "1.5",
            "8e0",
            r#""8""#,
            "null",
            "true",
            "false",
            "[]",
            "{}",
        ] {
            let metadata = format!(r#"{{"book":{{"page":{page},"image":"image.png"}}}}"#);
            assert_invalid_bookmark_images_is_preserved(&path, &metadata);
        }
        for metadata in [
            "null",
            "true",
            "8",
            r#""metadata""#,
            "[null]",
            r#"{"book":null}"#,
            r#"{"book":{}}"#,
            r#"{"book":{"page":8}}"#,
            r#"{"book":{"image":"image.png"}}"#,
            r#"{"book":{"page":8,"image":"image.png","unknown":true}}"#,
            r#"{"book":{"page":8,"page":8,"image":"image.png"}}"#,
            r#"{"book":{"page":8,"image":"a.png","image":"b.png"}}"#,
            r#"[["book"]]"#,
            r#"[["book",{"page":8,"image":"image.png"},0]]"#,
        ] {
            assert_invalid_bookmark_images_is_preserved(&path, metadata);
        }
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
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
            br#"{"preferences":{"panel_layout":{},"panel_layout":{}}}"#.as_slice(),
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

    #[cfg(any(unix, windows))]
    fn assert_native_bookmark_images_round_trip(native: &Path, encoded: &serde_json::Value) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let utf8 = Path::new("books/日本語/image.png");
        for (book, image, expected) in [
            (
                native,
                native,
                serde_json::json!([[encoded, {"page": 8, "image": encoded}]]),
            ),
            (
                native,
                utf8,
                serde_json::json!([[encoded, {"page": 8, "image": "books/日本語/image.png"}]]),
            ),
            (
                utf8,
                native,
                serde_json::json!({"books/日本語/image.png": {"page": 8, "image": encoded}}),
            ),
        ] {
            let settings = AppSettings {
                bookmarks: HashMap::from([(book.to_path_buf(), 8)]),
                bookmark_images: HashMap::from([(
                    book.to_path_buf(),
                    BookmarkImage {
                        page: 8,
                        image: image.to_path_buf(),
                    },
                )]),
                ..AppSettings::default()
            };
            settings.save_to(&path).unwrap();
            let saved: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            assert_eq!(saved["bookmark_images"], expected);
            assert_eq!(AppSettings::load_from(&path).unwrap(), settings);
            settings.save_to(&path).unwrap();
            assert_eq!(AppSettings::load_from(&path).unwrap(), settings);
        }
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[cfg(any(unix, windows))]
    fn assert_native_path_round_trip(native: PathBuf, encoded: serde_json::Value) {
        assert!(native.to_str().is_none());
        assert_native_bookmark_images_round_trip(&native, &encoded);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let utf8 = PathBuf::from("books/日本語.cbz");
        let mut settings = AppSettings {
            preferences: Preferences {
                language: Language::Japanese,
                mode: ReadingMode::Continuous,
                rtl: false,
                fit: FitMode::Width,
                sidebar: false,
                sidebar_mode: SidebarMode::Recent,
                panel_layout: PanelLayoutPreferences {
                    sidebar_width: 280,
                    inspector_width: 300,
                    inspector_visible: false,
                },
                background: Background::Paper,
                recursive_folder: false,
                remember_position: true,
                interval: 17,
            },
            ..AppSettings::default()
        };
        settings.add_recent(&utf8);
        settings.add_recent(&native);
        settings.positions.insert(utf8.clone(), 12);
        settings.positions.insert(native.clone(), 20);
        settings.bookmarks.insert(utf8.clone(), 8);
        settings.bookmarks.insert(native.clone(), 15);
        settings.save_to(&path).unwrap();
        assert_eq!(AppSettings::load_from(&path).unwrap(), settings);

        let saved: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            saved["recent"],
            serde_json::json!([encoded, "books/日本語.cbz"])
        );
        for (field, native_page, utf8_page) in [("positions", 20, 12), ("bookmarks", 15, 8)] {
            let pairs = saved[field].as_array().unwrap();
            assert_eq!(pairs.len(), 2);
            assert!(pairs.contains(&serde_json::json!([encoded, native_page])));
            assert!(pairs.contains(&serde_json::json!(["books/日本語.cbz", utf8_page])));
        }

        // Later preference edits and the final position save must also succeed.
        let mut reopened = AppSettings::load_from(&path).unwrap();
        reopened.preferences.fit = FitMode::Original;
        reopened.preferences.interval = 23;
        reopened.positions.insert(utf8.clone(), 13);
        reopened.bookmarks.insert(native.clone(), 16);
        reopened.save_to(&path).unwrap();
        assert_eq!(AppSettings::load_from(&path).unwrap(), reopened);
        let mut closing = AppSettings::load_from(&path).unwrap();
        closing.positions.insert(native.clone(), 21);
        closing.save_to(&path).unwrap();
        let expected = closing.clone();
        drop(closing);
        let mut settings = AppSettings::load_from(&path).unwrap();
        assert_eq!(settings, expected);

        // Each map selects its format independently, even with native recents.
        settings.positions.remove(&native);
        settings.save_to(&path).unwrap();
        let saved: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(saved["positions"].is_object());
        assert!(saved["bookmarks"].is_array());
        assert_eq!(AppSettings::load_from(&path).unwrap(), settings);
        settings.positions.insert(native.clone(), 22);
        settings.bookmarks.remove(&native);
        settings.save_to(&path).unwrap();
        let saved: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(saved["positions"].is_array());
        assert!(saved["bookmarks"].is_object());
        assert_eq!(AppSettings::load_from(&path).unwrap(), settings);
        settings.positions.remove(&native);
        settings.save_to(&path).unwrap();
        let saved: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(saved["positions"].is_object());
        assert!(saved["bookmarks"].is_object());
        assert_eq!(AppSettings::load_from(&path).unwrap(), settings);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_unix_paths_preserve_all_settings_across_saves() {
        use std::os::unix::ffi::OsStringExt;
        let bytes = b"books/chapter-\xff-\xfe.cbz".to_vec();
        let native = std::ffi::OsString::from_vec(bytes.clone()).into();
        assert_native_path_round_trip(native, serde_json::json!({"unix": bytes}));
    }

    #[cfg(windows)]
    #[test]
    fn unpaired_utf16_windows_paths_preserve_all_settings_across_saves() {
        use std::os::windows::ffi::OsStringExt;
        let mut units: Vec<u16> = "books/book-".encode_utf16().collect();
        units.extend([0xd800, u16::from(b'-'), 0xdc00]);
        units.extend(".cbz".encode_utf16());
        let native = std::ffi::OsString::from_wide(&units).into();
        assert_native_path_round_trip(native, serde_json::json!({"windows": units}));
    }

    #[test]
    fn malformed_native_paths_preserve_the_original_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        for encoding in [
            "null",
            "123",
            "[]",
            "{}",
            r#"{"unknown":[255]}"#,
            r#"{"unix":null}"#,
            r#"{"unix":"255"}"#,
            r#"{"unix":[-1]}"#,
            r#"{"unix":[256]}"#,
            r#"{"unix":[1.5]}"#,
            r#"{"unix":["255"]}"#,
            r#"{"windows":null}"#,
            r#"{"windows":"55296"}"#,
            r#"{"windows":[-1]}"#,
            r#"{"windows":[65536]}"#,
            r#"{"windows":[1.5]}"#,
            r#"{"windows":["55296"]}"#,
            r#"{"unix":[255],"extra":true}"#,
            r#"{"windows":[55296],"extra":true}"#,
            r#"{"unix":[255],"windows":[55296]}"#,
            r#"{"unix":[255],"unix":[254]}"#,
            r#"{"windows":[55296],"windows":[56320]}"#,
        ] {
            for source in [
                format!(r#"{{"recent":[{encoding}]}}"#),
                format!(r#"{{"positions":[[{encoding},1]]}}"#),
                format!(r#"{{"bookmarks":[[{encoding},2]]}}"#),
            ] {
                fs::write(&path, &source).unwrap();
                assert!(AppSettings::load_from(&path).is_err(), "{source}");
                assert!(AppSettings::default().save_to(&path).is_err(), "{source}");
                assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
            }
            for metadata in [
                format!(r#"{{"book":{{"page":8,"image":{encoding}}}}}"#),
                format!(r#"[[{encoding},{{"page":8,"image":"image.png"}}]]"#),
            ] {
                assert_invalid_bookmark_images_is_preserved(&path, &metadata);
            }
        }
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn malformed_pair_maps_preserve_the_original_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        for pairs in [
            "null",
            "[null]",
            r#"["book.cbz",1]"#,
            r#"[["book.cbz"]]"#,
            r#"[["book.cbz",1,2]]"#,
            r#"[["book.cbz",-1]]"#,
            r#"[["book.cbz","one"]]"#,
            r#"[["book.cbz",1.5]]"#,
            r#"[["book.cbz",null]]"#,
            r#"[["book.cbz",{}]]"#,
        ] {
            for field in ["positions", "bookmarks"] {
                let source = format!(r#"{{"{field}":{pairs}}}"#);
                fs::write(&path, &source).unwrap();
                assert!(AppSettings::load_from(&path).is_err(), "{source}");
                assert!(AppSettings::default().save_to(&path).is_err(), "{source}");
                assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
            }
        }
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn foreign_native_paths_report_the_platform_error_and_preserve_the_file() {
        #[cfg(unix)]
        let encodings = [r#"{"windows":[98,111,111,107]}"#, r#"{"windows":[55296]}"#];
        #[cfg(windows)]
        let encodings = [r#"{"unix":[98,111,111,107]}"#, r#"{"unix":[255]}"#];
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        for encoding in encodings {
            for source in [
                format!(r#"{{"recent":[{encoding}]}}"#),
                format!(r#"{{"positions":[[{encoding},1]]}}"#),
                format!(r#"{{"bookmarks":[[{encoding},2]]}}"#),
                format!(
                    r#"{{"bookmarks":{{"book":8}},"bookmark_images":{{"book":{{"page":8,"image":{encoding}}}}}}}"#
                ),
                format!(r#"{{"bookmark_images":[[{encoding},{{"page":8,"image":"image.png"}}]]}}"#),
            ] {
                fs::write(&path, &source).unwrap();
                let error = AppSettings::load_from(&path).unwrap_err();
                assert!(error.contains("native paths cannot be loaded on this platform"));
                #[cfg(unix)]
                assert!(error.contains("Windows native paths"));
                #[cfg(windows)]
                assert!(error.contains("Unix native paths"));
                assert!(AppSettings::default().save_to(&path).is_err());
                assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
            }
        }
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
