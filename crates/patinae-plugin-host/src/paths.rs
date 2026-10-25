use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use patinae_settings::{
    config::{initialize_config, read_config, AppConfig},
    paths::PathResolver,
};

/// A configured plugin allowlist, preserving entry order and duplicates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginConfigDocument {
    /// Unified configuration file.
    pub path: PathBuf,
    /// Entries as written in the document.
    pub entries: Vec<PluginConfigEntry>,
}

/// A library path before and after resolution against the configuration directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginConfigEntry {
    /// Original path text from TOML.
    pub path: String,
    /// Path resolved relative to the configuration directory.
    pub resolved_path: PathBuf,
}

/// Reads the allowlist from the unified configuration.
///
/// Returns `None` when the file or the `plugin` key is absent.
///
/// # Errors
/// Returns an error for unreadable or invalid configuration.
pub fn read_plugin_config(path: &Path) -> Result<Option<PluginConfigDocument>, String> {
    let config = read_config(path).map_err(|error| error.to_string())?;
    Ok(config.and_then(|config| plugin_document(path, config)))
}

fn plugin_document(path: &Path, config: AppConfig) -> Option<PluginConfigDocument> {
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    config.plugin.map(|entries| PluginConfigDocument {
        path: path.to_owned(),
        entries: entries
            .into_iter()
            .map(|entry| PluginConfigEntry {
                resolved_path: directory.join(&entry.path),
                path: entry.path,
            })
            .collect(),
    })
}

/// A bundled library identified independently of the installation's absolute path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundledPlugin {
    /// Layout-qualified filename, such as `adjacent/libhello_plugin.dylib`.
    pub id: String,
    /// Library path in this installation.
    pub path: PathBuf,
}

/// Discovered bundled libraries and the ordered user selection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginCatalog {
    /// Bundled libraries, including disabled ones.
    pub bundled: Vec<BundledPlugin>,
    /// User libraries, excluding aliases of bundled libraries.
    pub user: Vec<PathBuf>,
}

impl PluginCatalog {
    /// Selects enabled bundled libraries followed by user libraries.
    pub fn library_paths(&self, disabled: &[String]) -> Vec<PathBuf> {
        self.bundled
            .iter()
            .filter(|plugin| !disabled.contains(&plugin.id))
            .map(|plugin| plugin.path.clone())
            .chain(self.user.iter().cloned())
            .collect()
    }
}

pub(crate) fn scan_plugin_dirs(directories: &[PathBuf]) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for directory in directories {
        match std::fs::read_dir(directory) {
            Ok(entries) => {
                let mut libraries: Vec<_> = entries
                    .flatten()
                    .map(|entry| entry.path())
                    .filter(|path| is_plugin_library_path(path) && path.is_file())
                    .collect();
                libraries.sort();
                paths.extend(libraries);
            }
            Err(error) => log::debug!("Plugin directory {directory:?}: {error}"),
        }
    }
    paths
}

/// Discovers plugin directories from settings and executable layout.
#[derive(Debug, Clone)]
pub struct PluginDiscovery {
    paths: PathResolver,
    executable_path: Option<PathBuf>,
    config: Option<Result<AppConfig, String>>,
}

impl PluginDiscovery {
    /// Creates plugin discovery from deterministic path inputs.
    pub fn new(paths: PathResolver) -> Self {
        Self {
            paths,
            executable_path: None,
            config: None,
        }
    }

    /// Creates plugin discovery from the current process.
    pub fn from_process_env() -> Self {
        Self::new(PathResolver::from_process_env())
            .with_optional_executable_path(std::env::current_exe().ok())
    }

    /// Sets the executable path used for adjacent plugin directories.
    pub fn with_executable_path(mut self, executable_path: impl Into<PathBuf>) -> Self {
        self.executable_path = Some(executable_path.into());
        self
    }

    /// Sets or clears the executable path.
    pub fn with_optional_executable_path(mut self, executable_path: Option<PathBuf>) -> Self {
        self.executable_path = executable_path;
        self
    }

    /// Returns the single application configuration path.
    pub fn config_path(&self) -> PathBuf {
        self.paths.config_file()
    }

    /// Pins startup preferences so deferred discovery uses the same document as the GUI.
    pub fn with_config(mut self, config: Result<AppConfig, String>) -> Self {
        self.config = Some(config);
        self
    }

    /// Reads configuration, creating defaults when the file is absent.
    ///
    /// # Errors
    /// Returns configuration errors without falling back to scanning.
    pub fn load_config(&self) -> Result<AppConfig, String> {
        self.config.clone().unwrap_or_else(|| {
            initialize_config(&self.config_path()).map_err(|error| error.to_string())
        })
    }

    pub(crate) fn library_paths(&self) -> Result<Vec<PathBuf>, String> {
        let config = self.load_config()?;
        Ok(self
            .catalog(&config)
            .library_paths(&config.disabled_bundled_plugins))
    }

    /// Discovers both groups without loading libraries or modifying configuration.
    pub fn catalog(&self, config: &AppConfig) -> PluginCatalog {
        let mut seen = HashSet::new();
        let mut bundled = Vec::new();
        if let Some(exe) = &self.executable_path {
            for (kind, directory) in executable_plugin_dirs(exe, cfg!(target_os = "macos")) {
                for path in scan_plugin_dirs(&[directory]) {
                    let Some(filename) = path.file_name().and_then(|name| name.to_str()) else {
                        continue;
                    };
                    if seen.insert(library_identity(&path)) {
                        bundled.push(BundledPlugin {
                            id: format!("{kind}/{filename}"),
                            path,
                        });
                    }
                }
            }
        }
        let user = match &config.plugin {
            Some(entries) => {
                let config_path = self.config_path();
                let directory = config_path.parent().unwrap_or_else(|| Path::new("."));
                entries
                    .iter()
                    .map(|entry| directory.join(&entry.path))
                    .collect()
            }
            None => scan_plugin_dirs(&[self.paths.plugin_dir()]),
        };
        PluginCatalog {
            bundled,
            user: user
                .into_iter()
                .filter(|path| seen.insert(library_identity(path)))
                .collect(),
        }
    }

    /// Returns the standard plugin search directories.
    pub fn standard_plugin_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        push_unique(&mut dirs, self.paths.plugin_dir());

        if let Some(exe) = &self.executable_path {
            push_executable_plugin_dirs(&mut dirs, exe, cfg!(target_os = "macos"));
        }

        dirs
    }
}

pub fn is_plugin_library_path(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    match std::env::consts::OS {
        "macos" => ext == "dylib",
        "linux" => ext == "so",
        "windows" => ext == "dll",
        _ => false,
    }
}

pub fn standard_plugin_dirs() -> Vec<PathBuf> {
    PluginDiscovery::from_process_env().standard_plugin_dirs()
}

fn push_unique(dirs: &mut Vec<PathBuf>, dir: PathBuf) {
    if !dirs.iter().any(|p| p == &dir) {
        dirs.push(dir);
    }
}

fn push_executable_plugin_dirs(dirs: &mut Vec<PathBuf>, exe: &Path, include_bundle_plugins: bool) {
    for (_, directory) in executable_plugin_dirs(exe, include_bundle_plugins) {
        push_unique(dirs, directory);
    }
}

fn executable_plugin_dirs(
    exe: &Path,
    include_bundle_plugins: bool,
) -> Vec<(&'static str, PathBuf)> {
    let mut dirs = Vec::new();
    let Some(parent) = exe.parent() else {
        return dirs;
    };
    dirs.push(("adjacent", parent.join("plugins")));

    if include_bundle_plugins && parent.file_name().and_then(|s| s.to_str()) == Some("MacOS") {
        if let Some(contents) = parent.parent() {
            dirs.push(("macos", contents.join("PlugIns")));
        }
    }
    dirs
}

#[cfg(target_os = "windows")]
pub(crate) fn apply_deps_search_paths(plugin_path: &Path) {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let deps_path = plugin_path.with_extension("deps");
    let content = match std::fs::read_to_string(&deps_path) {
        Ok(c) => c,
        Err(_) => return,
    };

    let current_path = std::env::var("PATH").unwrap_or_default();
    let deps_dir = deps_path.parent().unwrap_or_else(|| Path::new("."));
    let updates = deps_search_path_updates(
        &content,
        deps_dir,
        &current_path,
        |path| path.is_dir(),
        |line, args| {
            let output = match std::process::Command::new(&args[0])
                .args(&args[1..])
                .creation_flags(CREATE_NO_WINDOW)
                .output()
            {
                Ok(o) if o.status.success() => o,
                Ok(o) => {
                    log::debug!("Plugin deps: command exited {}: {}", o.status, line);
                    return None;
                }
                Err(e) => {
                    log::debug!("Plugin deps: failed to run: {} ({})", line, e);
                    return None;
                }
            };

            Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
        },
    );

    for (key, value) in &updates.env_updates {
        std::env::set_var(key, value);
        log::debug!(
            "Plugin deps ({:?}): set {}={}",
            deps_path.file_name().unwrap_or_default(),
            key,
            value
        );
    }

    if !updates.path_prepend.is_empty() {
        let prepend = updates.path_prepend.join(";");
        std::env::set_var("PATH", format!("{prepend};{current_path}"));
        log::debug!(
            "Plugin deps ({:?}): added to PATH: {}",
            deps_path.file_name().unwrap_or_default(),
            prepend
        );
    }
}

#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Default, PartialEq, Eq)]
struct DepsSearchPathUpdates {
    path_prepend: Vec<String>,
    env_updates: Vec<(String, String)>,
}

#[cfg(any(target_os = "windows", test))]
fn deps_search_path_updates(
    content: &str,
    deps_dir: &Path,
    current_path: &str,
    mut path_exists: impl FnMut(&Path) -> bool,
    mut run_command: impl FnMut(&str, &[String]) -> Option<String>,
) -> DepsSearchPathUpdates {
    let mut updates = DepsSearchPathUpdates::default();

    for line in content.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let args = parse_shell_words(line);
        if args.is_empty() {
            continue;
        }

        match args.first().map(String::as_str) {
            Some("path") => {
                let Some(raw_path) = args.get(1).filter(|_| args.len() == 2) else {
                    log::debug!("Plugin deps: invalid path directive: {}", line);
                    continue;
                };
                let path = resolve_deps_path(deps_dir, raw_path);
                if path_exists(&path) {
                    add_unique_path(&mut updates.path_prepend, current_path, path_string(&path));
                } else {
                    log::debug!("Plugin deps: path does not exist: {}", path.display());
                }
            }
            Some("env") => {
                let (Some(key), Some(raw_path)) =
                    (args.get(1), args.get(2).filter(|_| args.len() == 3))
                else {
                    log::debug!("Plugin deps: invalid env directive: {}", line);
                    continue;
                };
                let path = resolve_deps_path(deps_dir, raw_path);
                if path_exists(&path) {
                    updates.env_updates.push((key.clone(), path_string(&path)));
                } else {
                    log::debug!("Plugin deps: env path does not exist: {}", path.display());
                }
            }
            _ => {
                let Some(dir) = run_command(line, &args) else {
                    continue;
                };
                add_unique_path(
                    &mut updates.path_prepend,
                    current_path,
                    dir.trim().to_string(),
                );
            }
        }
    }

    updates
}

#[cfg(any(target_os = "windows", test))]
fn add_unique_path(added: &mut Vec<String>, current_path: &str, dir: String) {
    if !dir.is_empty()
        && !current_path
            .split(';')
            .any(|p| p.eq_ignore_ascii_case(&dir))
        && !added.iter().any(|p| p.eq_ignore_ascii_case(&dir))
    {
        added.push(dir);
    }
}

#[cfg(any(target_os = "windows", test))]
fn resolve_deps_path(deps_dir: &Path, raw_path: &str) -> PathBuf {
    let path = PathBuf::from(raw_path);
    if path.is_absolute() {
        normalize_deps_path(&path)
    } else {
        normalize_deps_path(&deps_dir.join(path))
    }
}

#[cfg(any(target_os = "windows", test))]
fn normalize_deps_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if !normalized.pop() {
                    normalized.push("..");
                }
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

#[cfg(any(target_os = "windows", test))]
fn path_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

#[cfg(any(target_os = "windows", test))]
fn parse_shell_words(line: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;

    for ch in line.chars() {
        match ch {
            '"' => in_quotes = !in_quotes,
            c if c.is_ascii_whitespace() && !in_quotes => {
                if !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        args.push(current);
    }
    args
}

/// Resolves a library identity, retaining absolute paths for missing files.
pub fn library_identity(path: &Path) -> PathBuf {
    path.canonicalize()
        .or_else(|_| std::path::absolute(path))
        .unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use patinae_settings::paths::PathResolverInput;

    fn fixture_discovery(root: &Path) -> PluginDiscovery {
        PluginDiscovery::new(PathResolver::new(PathResolverInput {
            config_dir: Some(root.to_owned()),
            plugin_dir: Some(root.join("plugins")),
            ..Default::default()
        }))
        .with_executable_path(root.join("bin/patinae"))
    }

    #[test]
    fn missing_allowlist_scans_all_dirs_but_empty_list_disables_only_user() {
        let root = tempfile::tempdir().unwrap();
        let discovery = fixture_discovery(root.path());
        let dirs = discovery.standard_plugin_dirs();
        let mut expected = Vec::new();
        for dir in &dirs {
            std::fs::create_dir_all(dir).unwrap();
            let library = dir.join(format!("fixture.{}", std::env::consts::DLL_EXTENSION));
            std::fs::write(&library, "fixture").unwrap();
            std::fs::write(dir.join("readme.txt"), "ignore").unwrap();
            expected.push(library);
        }
        expected.reverse(); // Bundled libraries precede user libraries.
        assert_eq!(discovery.library_paths().unwrap(), expected);
        assert!(discovery.config_path().is_file());
        std::fs::write(discovery.config_path(), "repl_position='top'\nplugin=[]").unwrap();
        assert_eq!(discovery.library_paths().unwrap(), expected[..1]);
    }

    #[test]
    fn bundled_selection_survives_relocation_and_new_libraries_default_to_enabled() {
        let root = tempfile::tempdir().unwrap();
        let extension = std::env::consts::DLL_EXTENSION;
        let disabled = format!("adjacent/a.{extension}");
        for location in ["first", "moved"] {
            let location = root.path().join(location);
            let discovery = fixture_discovery(&location);
            let bundled = location.join("bin/plugins");
            std::fs::create_dir_all(&bundled).unwrap();
            for name in ["z", "a", "b"] {
                std::fs::write(bundled.join(format!("{name}.{extension}")), "fixture").unwrap();
            }
            std::fs::create_dir(bundled.join(format!("directory.{extension}"))).unwrap();
            let text = format!(
                "disabled_bundled_plugins = ['{disabled}', 'adjacent/future']\n[[plugin]]\npath='bin/plugins/a.{extension}'\n[[plugin]]\npath='missing-user'"
            );
            std::fs::write(discovery.config_path(), text).unwrap();
            let config = discovery.load_config().unwrap();
            let catalog = discovery.catalog(&config);
            assert_eq!(catalog.bundled.len(), 3);
            assert_eq!(catalog.bundled[0].id, disabled);
            assert_eq!(catalog.user, vec![location.join("missing-user")]);
            assert_eq!(
                discovery.library_paths().unwrap(),
                vec![
                    bundled.join(format!("b.{extension}")),
                    bundled.join(format!("z.{extension}")),
                    location.join("missing-user"),
                ]
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn bundled_aliases_are_deduplicated_and_cannot_be_enabled_through_user_paths() {
        let root = tempfile::tempdir().unwrap();
        let discovery = fixture_discovery(root.path());
        let bundled = root.path().join("bin/plugins");
        std::fs::create_dir_all(&bundled).unwrap();
        let filename = format!("a.{}", std::env::consts::DLL_EXTENSION);
        let library = bundled.join(&filename);
        std::fs::write(&library, "fixture").unwrap();
        std::os::unix::fs::symlink(
            &library,
            bundled.join(format!("z.{}", std::env::consts::DLL_EXTENSION)),
        )
        .unwrap();
        std::os::unix::fs::symlink(&library, root.path().join("alias")).unwrap();
        std::fs::write(
            discovery.config_path(),
            format!("disabled_bundled_plugins=['adjacent/{filename}']\n[[plugin]]\npath='alias'"),
        )
        .unwrap();
        let config = discovery.load_config().unwrap();
        let catalog = discovery.catalog(&config);
        assert_eq!(catalog.bundled.len(), 1);
        assert!(catalog.user.is_empty());
        assert!(discovery.library_paths().unwrap().is_empty());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_bundle_uses_distinct_layout_ids_and_stable_directory_order() {
        let root = tempfile::tempdir().unwrap();
        let contents = root.path().join("Patinae.app/Contents");
        let adjacent = contents.join("MacOS/plugins");
        let plugins = contents.join("PlugIns");
        for directory in [&adjacent, &plugins] {
            std::fs::create_dir_all(directory).unwrap();
            std::fs::write(directory.join("same.dylib"), "fixture").unwrap();
        }
        let discovery =
            fixture_discovery(root.path()).with_executable_path(contents.join("MacOS/patinae"));
        let catalog = discovery.catalog(&AppConfig::default());
        assert_eq!(
            catalog
                .bundled
                .iter()
                .map(|plugin| plugin.id.as_str())
                .collect::<Vec<_>>(),
            vec!["adjacent/same.dylib", "macos/same.dylib"]
        );
        assert_eq!(
            catalog.library_paths(&["macos/same.dylib".into()]),
            vec![adjacent.join("same.dylib")]
        );
    }

    #[test]
    fn configured_paths_resolve_from_config_dir_and_deduplicate_in_order() {
        let root = tempfile::tempdir().unwrap();
        let discovery = fixture_discovery(root.path());
        let library = root.path().join("library");
        std::fs::write(&library, "fixture").unwrap();
        let text = format!("[[plugin]]\npath='./library'\n[[plugin]]\npath='{}'\n[[plugin]]\npath='missing'\n[[plugin]]\npath='missing'", library.display());
        std::fs::write(discovery.config_path(), text).unwrap();
        let document = read_plugin_config(&discovery.config_path())
            .unwrap()
            .unwrap();
        assert_eq!(document.entries.len(), 4);
        assert_eq!(document.entries[0].path, "./library");
        assert_eq!(
            discovery.library_paths().unwrap(),
            vec![root.path().join("./library"), root.path().join("missing")]
        );
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&library, root.path().join("alias")).unwrap();
            std::fs::write(
                discovery.config_path(),
                "[[plugin]]\npath='alias'\n[[plugin]]\npath='library'",
            )
            .unwrap();
            assert_eq!(
                discovery.library_paths().unwrap(),
                vec![root.path().join("alias")]
            );
        }
    }

    #[test]
    fn config_errors_never_fall_back_to_scanning() {
        let root = tempfile::tempdir().unwrap();
        let discovery = fixture_discovery(root.path());
        let plugins = root.path().join("plugins");
        std::fs::create_dir(&plugins).unwrap();
        std::fs::write(discovery.config_path(), "repl_position='left'").unwrap();
        assert!(discovery
            .library_paths()
            .unwrap_err()
            .contains("config.toml"));
    }

    #[test]
    fn startup_snapshot_survives_external_edits_and_pins_failures() {
        let root = tempfile::tempdir().unwrap();
        let discovery = fixture_discovery(root.path());
        std::fs::write(discovery.config_path(), "[[plugin]]\npath='initial'").unwrap();
        let config = discovery.load_config();
        let snapshot = discovery.clone().with_config(config);
        std::fs::write(discovery.config_path(), "plugin=[]").unwrap();
        assert_eq!(
            snapshot.library_paths().unwrap(),
            vec![root.path().join("initial")]
        );
        let failed = discovery.with_config(Err("Cannot read startup config".into()));
        assert!(failed.library_paths().is_err());
    }

    fn discovery(config_dir: &str, plugin_dir: Option<&str>, exe: Option<&str>) -> PluginDiscovery {
        let input = PathResolverInput {
            home_dir: Some(PathBuf::from("/home/patinae")),
            config_dir: Some(PathBuf::from(config_dir)),
            plugin_dir: plugin_dir.map(PathBuf::from),
            ..PathResolverInput::default()
        };
        PluginDiscovery::new(PathResolver::new(input))
            .with_optional_executable_path(exe.map(PathBuf::from))
    }

    #[test]
    fn discovery_uses_plugin_dir_override() {
        let dirs = discovery("/cfg", Some("/plugins"), None).standard_plugin_dirs();
        assert_eq!(dirs, vec![PathBuf::from("/plugins")]);
    }

    #[test]
    fn discovery_derives_plugin_dir_from_config_dir() {
        let dirs = discovery("/cfg", None, None).standard_plugin_dirs();
        assert_eq!(dirs, vec![PathBuf::from("/cfg/plugins")]);
    }

    #[test]
    fn discovery_includes_executable_adjacent_plugins() {
        let dirs = discovery("/cfg", Some("/plugins"), Some("/opt/patinae/bin/patinae"))
            .standard_plugin_dirs();
        assert_eq!(
            dirs,
            vec![
                PathBuf::from("/plugins"),
                PathBuf::from("/opt/patinae/bin/plugins"),
            ]
        );
    }

    #[test]
    fn discovery_deduplicates_dirs() {
        let dirs = discovery(
            "/cfg",
            Some("/opt/patinae/bin/plugins"),
            Some("/opt/patinae/bin/patinae"),
        )
        .standard_plugin_dirs();
        assert_eq!(dirs, vec![PathBuf::from("/opt/patinae/bin/plugins")]);
    }

    #[test]
    fn executable_dirs_can_include_macos_bundle_plugins() {
        let mut dirs = Vec::new();
        push_executable_plugin_dirs(
            &mut dirs,
            Path::new("/Applications/Patinae.app/Contents/MacOS/patinae"),
            true,
        );
        assert_eq!(
            dirs,
            vec![
                PathBuf::from("/Applications/Patinae.app/Contents/MacOS/plugins"),
                PathBuf::from("/Applications/Patinae.app/Contents/PlugIns"),
            ]
        );
    }

    #[test]
    fn deps_search_path_updates_parse_commands_and_deduplicate() {
        let content = "\
# comment
runtime-tool -c \"print('C:\\\\Dep A')\"
tool --dir
tool --dir
existing
";
        let mut calls = Vec::new();
        let updates = deps_search_path_updates(
            content,
            Path::new("/bundle/plugins"),
            "C:\\Existing;C:\\Other",
            |_| false,
            |_, args| {
                calls.push(args.to_vec());
                match args.first().map(String::as_str) {
                    Some("runtime-tool") => Some("C:\\Dep A".to_string()),
                    Some("tool") => Some("C:\\Dep B".to_string()),
                    Some("existing") => Some("c:\\existing".to_string()),
                    _ => None,
                }
            },
        );

        assert_eq!(
            calls,
            vec![
                vec![
                    "runtime-tool".to_string(),
                    "-c".to_string(),
                    "print('C:\\\\Dep A')".to_string(),
                ],
                vec!["tool".to_string(), "--dir".to_string()],
                vec!["tool".to_string(), "--dir".to_string()],
                vec!["existing".to_string()],
            ]
        );
        assert_eq!(
            updates.path_prepend,
            vec!["C:\\Dep A".to_string(), "C:\\Dep B".to_string()]
        );
        assert!(updates.env_updates.is_empty());
    }

    #[test]
    fn deps_search_path_updates_resolve_relative_path_and_env_directives() {
        let content = "\
path ../runtime
path ../runtime-env/Scripts
env FIXTURE_RUNTIME_HOME ../runtime-env
";
        let existing = [
            PathBuf::from("/bundle/runtime"),
            PathBuf::from("/bundle/runtime-env"),
            PathBuf::from("/bundle/runtime-env/Scripts"),
        ];

        let updates = deps_search_path_updates(
            content,
            Path::new("/bundle/plugins"),
            "",
            |path| existing.iter().any(|existing_path| existing_path == path),
            |_, _| None,
        );

        assert_eq!(
            updates.path_prepend,
            vec![
                "/bundle/runtime".to_string(),
                "/bundle/runtime-env/Scripts".to_string(),
            ]
        );
        assert_eq!(
            updates.env_updates,
            vec![(
                "FIXTURE_RUNTIME_HOME".to_string(),
                "/bundle/runtime-env".to_string()
            )]
        );
    }

    #[test]
    fn deps_search_path_updates_skip_missing_declarative_paths() {
        let content = "\
path ../missing-runtime
env FIXTURE_RUNTIME_HOME ../missing-venv
";
        let updates = deps_search_path_updates(
            content,
            Path::new("/bundle/plugins"),
            "",
            |_| false,
            |_, _| None,
        );

        assert!(updates.path_prepend.is_empty());
        assert!(updates.env_updates.is_empty());
    }

    #[test]
    fn deps_search_path_updates_deduplicate_declarative_paths_against_current_path() {
        let content = "\
path ../runtime
path ../runtime
";
        let updates = deps_search_path_updates(
            content,
            Path::new("/bundle/plugins"),
            "/bundle/runtime;/other",
            |path| path == Path::new("/bundle/runtime"),
            |_, _| None,
        );

        assert!(updates.path_prepend.is_empty());
    }

    #[test]
    fn deps_search_path_updates_skip_failed_or_empty_commands() {
        let content = "missing\nempty\n";
        let updates = deps_search_path_updates(
            content,
            Path::new("/bundle/plugins"),
            "",
            |_| false,
            |_, args| match args[0].as_str() {
                "empty" => Some(String::new()),
                _ => None,
            },
        );
        assert!(updates.path_prepend.is_empty());
        assert!(updates.env_updates.is_empty());
    }
}
