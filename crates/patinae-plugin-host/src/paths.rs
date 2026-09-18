use std::path::{Path, PathBuf};

use patinae_settings::paths::PathResolver;

/// Optional allowlist filename shared by every native plugin loader.
const PLUGIN_MANIFEST_FILE: &str = "plugins.toml";

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PluginManifest {
    #[serde(default)]
    plugin: Vec<PluginEntry>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PluginEntry {
    path: String,
}

/// Selects libraries from the first manifest, or scans all directories.
///
/// Errors reading or parsing a manifest never fall back to directory scanning.
pub(crate) fn plugin_library_paths(directories: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    for directory in directories {
        let manifest_path = directory.join(PLUGIN_MANIFEST_FILE);
        // Unlike exists(), this distinguishes absence from an inaccessible file
        // and recognizes dangling symlinks as manifests that failed to load.
        match std::fs::symlink_metadata(&manifest_path) {
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                ) =>
            {
                continue;
            }
            Err(error) => {
                return Err(format!(
                    "Cannot inspect {}: {error}",
                    manifest_path.display()
                ));
            }
        }
        let content = std::fs::read_to_string(&manifest_path)
            .map_err(|error| format!("Cannot read {}: {error}", manifest_path.display()))?;
        let manifest: PluginManifest = toml::from_str(&content)
            .map_err(|error| format!("Invalid {}: {error}", manifest_path.display()))?;
        let mut paths = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for (index, entry) in manifest.plugin.into_iter().enumerate() {
            if entry.path.trim().is_empty() {
                return Err(format!(
                    "Invalid {}: plugin entry {} requires a nonempty path",
                    manifest_path.display(),
                    index + 1
                ));
            }
            let path = directory.join(entry.path);
            // Keep unresolved paths for per-library errors; one missing library
            // must not prevent subsequent entries from loading.
            let identity = path.canonicalize().unwrap_or_else(|_| path.clone());
            if seen.insert(identity) {
                paths.push(path);
            }
        }
        return Ok(paths);
    }

    let mut paths = Vec::new();
    for directory in directories {
        match std::fs::read_dir(directory) {
            Ok(entries) => paths.extend(
                entries
                    .flatten()
                    .map(|entry| entry.path())
                    .filter(|path| is_plugin_library_path(path)),
            ),
            Err(error) => log::debug!("Plugin directory {directory:?}: {error}"),
        }
    }
    Ok(paths)
}

/// Discovers plugin directories from settings and executable layout.
#[derive(Debug, Clone)]
pub struct PluginDiscovery {
    paths: PathResolver,
    executable_path: Option<PathBuf>,
}

impl PluginDiscovery {
    /// Creates plugin discovery from deterministic path inputs.
    pub fn new(paths: PathResolver) -> Self {
        Self {
            paths,
            executable_path: None,
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
    let Some(parent) = exe.parent() else {
        return;
    };
    push_unique(dirs, parent.join("plugins"));

    if include_bundle_plugins && parent.file_name().and_then(|s| s.to_str()) == Some("MacOS") {
        if let Some(contents) = parent.parent() {
            push_unique(dirs, contents.join("PlugIns"));
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use patinae_settings::paths::PathResolverInput;

    struct ManifestFixture(PathBuf);

    impl ManifestFixture {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "patinae-manifest-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            for dir in ["user", "app", "external"] {
                std::fs::create_dir_all(root.join(dir)).unwrap();
            }
            Self(root)
        }

        fn dirs(&self) -> Vec<PathBuf> {
            vec![self.0.join("user"), self.0.join("app")]
        }

        fn library(&self, dir: &str, name: &str) -> PathBuf {
            let path = self
                .0
                .join(dir)
                .join(format!("{name}.{}", std::env::consts::DLL_EXTENSION));
            std::fs::write(&path, "fixture").unwrap();
            path
        }

        fn manifest(&self, dir: &str, content: &str) {
            std::fs::write(self.0.join(dir).join(PLUGIN_MANIFEST_FILE), content).unwrap();
        }
    }

    impl Drop for ManifestFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn absent_manifest_preserves_scanning_all_directories() {
        let fixture = ManifestFixture::new();
        let user = fixture.library("user", "user");
        let app = fixture.library("app", "app");
        fixture.library("external", "external");
        std::fs::write(fixture.0.join("user/readme.txt"), "ignored").unwrap();
        assert_eq!(
            plugin_library_paths(&fixture.dirs()).unwrap(),
            vec![user, app]
        );
    }

    #[test]
    fn absent_or_non_directory_search_paths_do_not_block_other_directories() {
        let fixture = ManifestFixture::new();
        let library = fixture.library("app", "library");
        let not_directory = fixture.0.join("file");
        std::fs::write(&not_directory, "not a directory").unwrap();
        let directories = vec![
            fixture.0.join("missing"),
            not_directory,
            fixture.0.join("app"),
        ];
        assert_eq!(plugin_library_paths(&directories).unwrap(), vec![library]);
    }

    #[test]
    fn first_manifest_wins_before_any_directory_is_scanned() {
        let fixture = ManifestFixture::new();
        fixture.library("user", "unlisted");
        fixture.library("app", "unlisted");
        fixture.manifest("app", "plugin = []");
        assert!(plugin_library_paths(&fixture.dirs()).unwrap().is_empty());
        fixture.manifest("user", "[[plugin]]\npath = 'selected'");
        fixture.manifest("app", "invalid TOML");
        assert_eq!(
            plugin_library_paths(&fixture.dirs()).unwrap(),
            vec![fixture.0.join("user/selected")]
        );
    }

    #[test]
    fn manifest_resolves_paths_in_order_and_deduplicates_canonical_paths() {
        let fixture = ManifestFixture::new();
        let local = fixture.library("user", "local");
        let external = fixture.library("external", "external");
        let absolute = fixture.library("external", "absolute");
        fixture.manifest("user", &format!(
            "[[plugin]]\npath = '../external/{}'\n[[plugin]]\npath = '{}'\n[[plugin]]\npath = '{}'\n[[plugin]]\npath = './{}'\n[[plugin]]\npath = 'missing'\n",
            external.file_name().unwrap().to_str().unwrap(),
            local.file_name().unwrap().to_str().unwrap(),
            absolute.display(),
            local.file_name().unwrap().to_str().unwrap(),
        ));
        let paths = plugin_library_paths(&fixture.dirs()).unwrap();
        assert_eq!(paths.len(), 4);
        assert_eq!(
            paths[0].canonicalize().unwrap(),
            external.canonicalize().unwrap()
        );
        assert_eq!(paths[1], local);
        assert_eq!(paths[2], absolute);
        assert_eq!(paths[3], fixture.0.join("user/missing"));
    }

    #[cfg(unix)]
    #[test]
    fn manifest_deduplicates_library_symlinks() {
        let fixture = ManifestFixture::new();
        let library = fixture.library("external", "library");
        std::os::unix::fs::symlink(&library, fixture.0.join("user/link")).unwrap();
        fixture.manifest(
            "user",
            &format!(
                "[[plugin]]\npath = 'link'\n[[plugin]]\npath = '{}'\n",
                library.display()
            ),
        );
        assert_eq!(
            plugin_library_paths(&fixture.dirs()).unwrap(),
            vec![fixture.0.join("user/link")]
        );
    }

    #[test]
    fn empty_manifest_disables_loading() {
        let fixture = ManifestFixture::new();
        fixture.library("user", "unlisted");
        for content in ["", "# no plugins", "plugin = []"] {
            fixture.manifest("user", content);
            assert!(plugin_library_paths(&fixture.dirs()).unwrap().is_empty());
        }
    }

    #[test]
    fn invalid_manifest_never_falls_back() {
        let fixture = ManifestFixture::new();
        fixture.library("user", "unlisted");
        fixture.manifest("app", "plugin = []");
        for content in [
            "[",
            "[[plugins]]\npath = 'x'",
            "unknown = 1",
            "[[plugin]]",
            "[[plugin]]\npath = 42",
            "[[plugin]]\npath = ''",
            "[[plugin]]\npath = '  '",
            "[[plugin]]\npath = 'x'\nenabled = true",
            "[[plugin]]\npath = 'valid'\n[[plugin]]\npath = ''",
        ] {
            fixture.manifest("user", content);
            let error = plugin_library_paths(&fixture.dirs()).unwrap_err();
            assert!(
                error.contains(&fixture.0.join("user/plugins.toml").display().to_string()),
                "{error}"
            );
        }
    }

    #[test]
    fn unreadable_manifest_never_falls_back() {
        let fixture = ManifestFixture::new();
        fixture.manifest("app", "plugin = []");
        let manifest = fixture.0.join("user/plugins.toml");
        std::fs::create_dir(&manifest).unwrap();
        let error = plugin_library_paths(&fixture.dirs()).unwrap_err();
        assert!(error.contains("Cannot read"));
        assert!(error.contains(&manifest.display().to_string()));
    }

    #[cfg(unix)]
    #[test]
    fn dangling_manifest_symlink_never_falls_back() {
        let fixture = ManifestFixture::new();
        std::os::unix::fs::symlink("missing", fixture.0.join("user/plugins.toml")).unwrap();
        assert!(plugin_library_paths(&fixture.dirs())
            .unwrap_err()
            .contains("Cannot read"));
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
python -c \"print('C:\\\\Dep A')\"
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
                    Some("python") => Some("C:\\Dep A".to_string()),
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
                    "python".to_string(),
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
path ../python
path ../python-venv/Scripts
env VIRTUAL_ENV ../python-venv
";
        let existing = [
            PathBuf::from("/bundle/python"),
            PathBuf::from("/bundle/python-venv"),
            PathBuf::from("/bundle/python-venv/Scripts"),
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
                "/bundle/python".to_string(),
                "/bundle/python-venv/Scripts".to_string(),
            ]
        );
        assert_eq!(
            updates.env_updates,
            vec![("VIRTUAL_ENV".to_string(), "/bundle/python-venv".to_string())]
        );
    }

    #[test]
    fn deps_search_path_updates_skip_missing_declarative_paths() {
        let content = "\
path ../missing-python
env VIRTUAL_ENV ../missing-venv
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
path ../python
path ../python
";
        let updates = deps_search_path_updates(
            content,
            Path::new("/bundle/plugins"),
            "/bundle/python;/other",
            |path| path == Path::new("/bundle/python"),
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
