//! Native application configuration.
//!
//! Missing `plugin` enables user discovery; an empty list disables user plugins.
//! Bundled plugins are discovered independently and can be disabled by stable ID.
//! Updates preserve unrelated TOML values and comments and replace files atomically.

use crate::{SettingEnum as _, ThemeMode};
use serde::Deserialize;
use std::{
    io::Write,
    path::{Path, PathBuf},
};
use toml_edit::{value, Array, ArrayOfTables, DocumentMut, Item, Table};

/// Vertical placement of the command input and its attached drawers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReplPosition {
    /// Input at the top, with output below it.
    Top,
    /// Input at the bottom, with output above it.
    #[default]
    Bottom,
}

/// A literal library path, relative to the configuration file or absolute.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginEntry {
    /// Library path without shell or environment expansion.
    pub path: String,
}

/// Application preferences, separate from molecular session settings.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct AppConfig {
    /// Appearance preference, following the operating system by default.
    #[serde(default, deserialize_with = "deserialize_theme")]
    pub theme: ThemeMode,
    /// Input placement, defaulting to the bottom.
    #[serde(default)]
    pub repl_position: ReplPosition,
    /// Ordered allowlist; `None` means automatic discovery.
    pub plugin: Option<Vec<PluginEntry>>,
    /// Disabled bundled library IDs, relative to their installation layout.
    #[serde(default)]
    pub disabled_bundled_plugins: Vec<String>,
}

// Config strings are lowercase; session serialization retains its existing enum format.
fn deserialize_theme<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<ThemeMode, D::Error> {
    let name = String::deserialize(deserializer)?;
    match name.as_str() {
        "dark" => Ok(ThemeMode::Dark),
        "light" => Ok(ThemeMode::Light),
        "system" => Ok(ThemeMode::System),
        _ => Err(serde::de::Error::unknown_variant(
            &name,
            &["dark", "light", "system"],
        )),
    }
}

/// A configuration failure with its file and original cause.
#[derive(Debug, thiserror::Error)]
#[error("{operation} {path}: {source}")]
pub struct ConfigError {
    path: PathBuf,
    operation: &'static str,
    #[source]
    source: Box<dyn std::error::Error + Send + Sync>,
}

fn error(
    path: &Path,
    operation: &'static str,
    source: impl std::error::Error + Send + Sync + 'static,
) -> ConfigError {
    ConfigError {
        path: path.to_owned(),
        operation,
        source: Box::new(source),
    }
}

fn invalid(path: &Path, message: &str) -> ConfigError {
    error(
        path,
        "Invalid",
        std::io::Error::new(std::io::ErrorKind::InvalidData, message),
    )
}

fn validate_entries(path: &Path, entries: &[PluginEntry]) -> Result<(), ConfigError> {
    for (index, entry) in entries.iter().enumerate() {
        if entry.path.trim().is_empty() {
            return Err(invalid(
                path,
                &format!("plugin entry {} requires a nonempty path", index + 1),
            ));
        }
    }
    Ok(())
}

fn parse(path: &Path, content: &str) -> Result<AppConfig, ConfigError> {
    let table: toml::Table = toml::from_str(content).map_err(|e| error(path, "Invalid", e))?;
    if table.contains_key("plugins") {
        return Err(invalid(path, "Use [[plugin]], not [[plugins]]"));
    }
    let config: AppConfig = toml::from_str(content).map_err(|e| error(path, "Invalid", e))?;
    if let Some(entries) = &config.plugin {
        validate_entries(path, entries)?;
    }
    Ok(config)
}

fn read_text(path: &Path) -> Result<Option<String>, ConfigError> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(error(path, "Cannot inspect", e)),
        Ok(_) => {}
    }
    // A dangling symlink is an error, not an absent configuration.
    std::fs::read_to_string(path)
        .map(Some)
        .map_err(|e| error(path, "Cannot read", e))
}

/// Reads configuration without creating files.
///
/// # Errors
/// Returns an error for unreadable files, invalid TOML, or invalid known fields.
pub fn read_config(path: &Path) -> Result<Option<AppConfig>, ConfigError> {
    read_text(path)?.map(|text| parse(path, &text)).transpose()
}

fn document(path: &Path) -> Result<(DocumentMut, bool), ConfigError> {
    match read_text(path)? {
        Some(text) => {
            parse(path, &text)?;
            Ok((text.parse().map_err(|e| error(path, "Invalid", e))?, true))
        }
        None => {
            let mut doc = DocumentMut::new();
            doc["repl_position"] = value("bottom");
            doc["theme"] = value("system");
            Ok((doc, false))
        }
    }
}

fn replace_plugins(doc: &mut DocumentMut, entries: &[PluginEntry]) {
    if entries.is_empty() {
        doc["plugin"] = value(Array::new());
    } else {
        let mut tables = ArrayOfTables::new();
        for entry in entries {
            let mut table = Table::new();
            table["path"] = value(entry.path.as_str());
            tables.push(table);
        }
        doc["plugin"] = Item::ArrayOfTables(tables);
    }
}

fn save(path: &Path, doc: &DocumentMut, replace: bool) -> Result<(), ConfigError> {
    let text = doc.to_string();
    parse(path, &text)?;
    let target = if path.is_symlink() {
        path.canonicalize()
            .map_err(|e| error(path, "Cannot resolve", e))?
    } else {
        path.to_owned()
    };
    let parent = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let write = || -> std::io::Result<()> {
        std::fs::create_dir_all(parent)?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        if let Ok(metadata) = std::fs::metadata(&target) {
            temporary
                .as_file()
                .set_permissions(metadata.permissions())?;
        }
        temporary.write_all(text.as_bytes())?;
        temporary.as_file().sync_all()?;
        if replace {
            temporary.persist(&target).map_err(|e| e.error)?;
        } else {
            temporary.persist_noclobber(&target).map_err(|e| e.error)?;
        }
        Ok(())
    };
    write().map_err(|e| error(path, "Cannot save", e))
}

/// Replaces only the plugin allowlist in the latest valid configuration document.
///
/// Existing symlinks, unrelated values, comments, and file permissions are retained.
///
/// # Errors
/// Returns an error for invalid paths, damaged existing documents, or filesystem failures.
pub fn save_plugins(path: &Path, libraries: &[PathBuf]) -> Result<(), ConfigError> {
    save_plugin_preferences(path, libraries, None)
}

/// Saves user libraries and bundled preferences together, preserving unrelated settings.
///
/// `None` preserves the existing bundled preferences.
///
/// # Errors
/// Returns an error for invalid paths, damaged documents, or filesystem failures.
pub fn save_plugin_preferences(
    path: &Path,
    libraries: &[PathBuf],
    disabled_bundled_plugins: Option<&[String]>,
) -> Result<(), ConfigError> {
    let entries = libraries
        .iter()
        .map(|library| {
            library
                .to_str()
                .filter(|text| !text.trim().is_empty())
                .map(|path| PluginEntry {
                    path: path.to_owned(),
                })
                .ok_or_else(|| invalid(path, "Plugin paths must be nonempty Unicode strings"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let (mut doc, exists) = document(path)?;
    replace_plugins(&mut doc, &entries);
    if let Some(disabled) = disabled_bundled_plugins {
        doc["disabled_bundled_plugins"] =
            value(disabled.iter().map(String::as_str).collect::<Array>());
    }
    save(path, &doc, exists)
}

/// Reads configuration, creating the default document when it is absent.
///
/// # Errors
/// Returns an error for invalid documents or filesystem failures.
pub fn initialize_config(path: &Path) -> Result<AppConfig, ConfigError> {
    let (doc, exists) = document(path)?;
    if !exists {
        save(path, &doc, false)?;
    }
    parse(path, &doc.to_string())
}

/// Saves the theme preference while preserving unrelated configuration and comments.
///
/// # Errors
/// Returns an error for invalid documents, invalid paths, or filesystem failures.
pub fn save_theme(path: &Path, theme: ThemeMode) -> Result<(), ConfigError> {
    let (mut doc, exists) = document(path)?;
    let decor = doc
        .get("theme")
        .and_then(Item::as_value)
        .map(|v| v.decor().clone());
    let mut replacement = toml_edit::Value::from(theme.name());
    if let Some(decor) = decor {
        *replacement.decor_mut() = decor;
    }
    doc["theme"] = Item::Value(replacement);
    save(path, &doc, exists)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_preferences_save_atomically_and_preserve_other_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "# keep this\ntheme='dark'\nrepl_position='top'\nplugin=[]\ndisabled_bundled_plugins=['adjacent/unknown']\n[future]\nanswer=42\n").unwrap();
        let disabled = vec!["adjacent/unknown".into(), "macos/test.dylib".into()];
        save_plugin_preferences(&path, &[PathBuf::from("custom")], Some(&disabled)).unwrap();
        let config = read_config(&path).unwrap().unwrap();
        assert_eq!(config.disabled_bundled_plugins, disabled);
        assert_eq!(config.plugin.unwrap()[0].path, "custom");
        assert_eq!(config.theme, ThemeMode::Dark);
        assert_eq!(config.repl_position, ReplPosition::Top);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# keep this"));
        assert!(text.contains("answer=42"));
        assert!(save_plugin_preferences(&path, &[PathBuf::new()], Some(&[])).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        save_plugins(&path, &[]).unwrap();
        assert_eq!(
            read_config(&path)
                .unwrap()
                .unwrap()
                .disabled_bundled_plugins,
            disabled
        );
        std::fs::write(&path, "disabled_bundled_plugins = false").unwrap();
        assert!(save_plugin_preferences(&path, &[], Some(&[])).is_err());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "disabled_bundled_plugins = false"
        );
    }

    #[test]
    fn theme_round_trips_preserve_comments_plugins_and_session_encoding() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "# preferences\ntheme = 'system' # appearance\nrepl_position = 'top'\n\n[[plugin]]\npath = 'custom/plugin'\n\n[future]\nanswer = 42\n").unwrap();
        for mode in [ThemeMode::Dark, ThemeMode::Light, ThemeMode::System] {
            save_theme(&path, mode).unwrap();
            let config = read_config(&path).unwrap().unwrap();
            assert_eq!(config.theme, mode);
            assert_eq!(config.repl_position, ReplPosition::Top);
            assert_eq!(config.plugin.unwrap()[0].path, "custom/plugin");
            let text = std::fs::read_to_string(&path).unwrap();
            assert!(text.contains("# preferences"));
            assert!(text.contains("# appearance"));
            assert!(text.contains("answer = 42"));
        }
        save_plugins(&path, &[]).unwrap();
        assert_eq!(
            read_config(&path).unwrap().unwrap().theme,
            ThemeMode::System
        );
        assert_eq!(serde_json::to_string(&ThemeMode::Dark).unwrap(), "\"Dark\"");
        assert_eq!(
            serde_json::from_str::<ThemeMode>("\"Light\"").unwrap(),
            ThemeMode::Light
        );
        assert_eq!(ThemeMode::Dark.to_i32(), 0);
        assert_eq!(ThemeMode::Light.to_i32(), 1);
        assert_eq!(ThemeMode::System.to_i32(), 2);
    }

    #[test]
    fn theme_save_creates_missing_config_and_extends_legacy_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/config.toml");
        save_theme(&path, ThemeMode::Light).unwrap();
        assert_eq!(read_config(&path).unwrap().unwrap().theme, ThemeMode::Light);
        std::fs::write(&path, "repl_position = 'top'\nplugin = []\n").unwrap();
        assert_eq!(
            read_config(&path).unwrap().unwrap().theme,
            ThemeMode::System
        );
        save_theme(&path, ThemeMode::Dark).unwrap();
        assert_eq!(read_config(&path).unwrap().unwrap().theme, ThemeMode::Dark);
        assert_eq!(read_config(&path).unwrap().unwrap().plugin, Some(vec![]));
    }

    #[test]
    fn defaults_and_positions_preserve_the_missing_empty_distinction() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        assert_eq!(read_config(&path).unwrap(), None);
        let config = initialize_config(&path).unwrap();
        assert_eq!(config, AppConfig::default());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "repl_position = \"bottom\"\ntheme = \"system\"\n"
        );
        for (text, position, entries) in [
            ("", ReplPosition::Bottom, None),
            ("repl_position = 'top'", ReplPosition::Top, None),
            (
                "repl_position = 'bottom'\nplugin = []",
                ReplPosition::Bottom,
                Some(vec![]),
            ),
        ] {
            std::fs::write(&path, text).unwrap();
            assert_eq!(
                read_config(&path).unwrap().unwrap(),
                AppConfig {
                    theme: ThemeMode::System,
                    repl_position: position,
                    plugin: entries,
                    disabled_bundled_plugins: Vec::new(),
                }
            );
        }
    }

    #[test]
    fn invalid_documents_are_reported_with_paths_and_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        for text in [
            "[",
            "repl_position = 'left'",
            "repl_position = 1",
            "theme = 'auto'",
            "theme = 2",
            "[[plugins]]\npath='x'",
            "[[plugin]]",
            "[[plugin]]\npath = 42",
            "[[plugin]]\npath = ''",
            "[[plugin]]\npath = '  '",
            "[[plugin]]\npath = 'x'\nenabled = true",
        ] {
            std::fs::write(&path, text).unwrap();
            assert!(
                read_config(&path)
                    .unwrap_err()
                    .to_string()
                    .contains(&path.display().to_string()),
                "{text}"
            );
            assert!(save_plugins(&path, &[]).is_err(), "{text}");
            assert!(save_theme(&path, ThemeMode::Dark).is_err(), "{text}");
            assert!(initialize_config(&path).is_err(), "{text}");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        }
    }

    #[test]
    fn plugin_updates_reread_and_preserve_preferences_comments_and_other_sections() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        initialize_config(&path).unwrap();
        let text = "# My layout\nrepl_position = 'top' # keep above\n\n[future]\nanswer = 42 # keep this too\n";
        std::fs::write(&path, text).unwrap();
        let entries = vec![
            PathBuf::from("relative/quoted\"name"),
            PathBuf::from("back\\slash"),
            dir.path().join("absolute"),
        ];
        save_plugins(&path, &entries).unwrap();
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(saved.starts_with(text));
        let config = read_config(&path).unwrap().unwrap();
        assert_eq!(config.repl_position, ReplPosition::Top);
        assert_eq!(
            config
                .plugin
                .unwrap()
                .iter()
                .map(|entry| PathBuf::from(&entry.path))
                .collect::<Vec<_>>(),
            entries
        );
        assert!(save_plugins(&path, &[PathBuf::new()]).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), saved);
        save_plugins(&path, &[]).unwrap();
        assert_eq!(read_config(&path).unwrap().unwrap().plugin, Some(vec![]));
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(saved.contains("repl_position = 'top' # keep above"));
        assert!(saved.contains("answer = 42 # keep this too"));
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_preserve_target_and_dangling_links_are_errors() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.toml");
        let path = dir.path().join("config.toml");
        std::fs::write(&target, "repl_position='top'\n").unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
        save_plugins(&path, &[]).unwrap();
        assert!(path.is_symlink());
        save_theme(&path, ThemeMode::Light).unwrap();
        assert!(path.is_symlink());
        assert_eq!(
            read_config(&target).unwrap().unwrap().theme,
            ThemeMode::Light
        );
        assert_eq!(
            read_config(&target).unwrap().unwrap().repl_position,
            ReplPosition::Top
        );
        assert_eq!(read_config(&target).unwrap().unwrap().plugin, Some(vec![]));
        std::fs::remove_file(&target).unwrap();
        assert!(read_config(&path).is_err());
        assert!(initialize_config(&path).is_err());
        assert!(save_plugins(&path, &[]).is_err());
        assert!(save_theme(&path, ThemeMode::Dark).is_err());
    }
}
