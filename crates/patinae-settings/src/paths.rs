//! Resolves standard Patinae paths.
//!
//! Each path can be overridden via an environment variable.  When the variable
//! is unset (or empty), the default under `~/.patinae` is used.
//!
//! | Variable                 | Default                    |
//! |--------------------------|----------------------------|
//! | `PATINAE_CONFIG_DIR`    | `~/.patinae`              |
//! | `PATINAE_PLUGIN_DIR`    | `<config_dir>/plugins`     |
//! | `PATINAERC`             | `<config_dir>/patinaerc`   |
//! | `PATINAE_RESOURCES_DIR` | `<config_dir>/resources`   |
//! | recent files            | `<config_dir>/recent-files.json` |
//! | thumbnails              | `<config_dir>/thumbnails`  |
//! | application config      | `<config_dir>/config.toml` |

use std::path::PathBuf;

/// Default directory name under `$HOME`.
#[cfg(not(target_arch = "wasm32"))]
const DEFAULT_CONFIG_DIR_NAME: &str = ".patinae";

/// Identifies how the startup rc path was selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatinaercSource {
    /// The path came from the `PATINAERC` environment variable.
    Env,
    /// The path is the default under the configuration directory.
    Default,
}

/// Resolved startup rc path plus its source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatinaercPath {
    /// Path to the startup rc script.
    pub path: PathBuf,
    /// Source used to resolve `path`.
    pub source: PatinaercSource,
}

/// Deterministic inputs for [`PathResolver`].
///
/// Use [`PathResolverInput::from_process_env`] for production process
/// environment values. Tests can construct this type directly instead of
/// mutating global environment variables.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PathResolverInput {
    /// User home directory used for the default configuration directory.
    pub home_dir: Option<PathBuf>,
    /// Value of `PATINAE_CONFIG_DIR`, if configured and non-empty.
    pub config_dir: Option<PathBuf>,
    /// Value of `PATINAE_PLUGIN_DIR`, if configured and non-empty.
    pub plugin_dir: Option<PathBuf>,
    /// Value of `PATINAE_RESOURCES_DIR`, if configured and non-empty.
    pub resources_dir: Option<PathBuf>,
    /// Value of `PATINAERC`, if configured and non-empty.
    pub patinaerc: Option<PathBuf>,
}

impl PathResolverInput {
    /// Captures path inputs from the current process.
    pub fn from_process_env() -> Self {
        Self {
            home_dir: process_home_dir(),
            config_dir: env_path("PATINAE_CONFIG_DIR"),
            plugin_dir: env_path("PATINAE_PLUGIN_DIR"),
            resources_dir: env_path("PATINAE_RESOURCES_DIR"),
            patinaerc: env_path("PATINAERC"),
        }
    }
}

/// Resolves configuration, plugin, resource, and startup-script paths.
///
/// The resolver itself is deterministic: it only reads values supplied through
/// [`PathResolverInput`]. Use [`PathResolver::from_process_env`] at the process
/// boundary when production behavior should honor environment variables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathResolver {
    input: PathResolverInput,
}

impl PathResolver {
    /// Creates a resolver from deterministic path inputs.
    pub fn new(input: PathResolverInput) -> Self {
        Self { input }
    }

    /// Creates a resolver from the current process environment.
    pub fn from_process_env() -> Self {
        Self::new(PathResolverInput::from_process_env())
    }

    /// Returns the application configuration directory.
    pub fn config_dir(&self) -> PathBuf {
        self.input
            .config_dir
            .clone()
            .unwrap_or_else(|| default_config_dir_from(self.input.home_dir.as_ref()))
    }

    /// Returns the unified application configuration file.
    pub fn config_file(&self) -> PathBuf {
        self.config_dir().join("config.toml")
    }

    /// Returns the plugin directory.
    pub fn plugin_dir(&self) -> PathBuf {
        self.input
            .plugin_dir
            .clone()
            .unwrap_or_else(|| self.config_dir().join("plugins"))
    }

    /// Returns the resources directory.
    pub fn resources_dir(&self) -> PathBuf {
        self.input
            .resources_dir
            .clone()
            .unwrap_or_else(|| self.config_dir().join("resources"))
    }

    /// Returns the CCD cache path.
    pub fn ccd_cache_path(&self) -> PathBuf {
        self.resources_dir().join("components.bin")
    }

    /// Returns the recent-files store path.
    pub fn recent_files_path(&self) -> PathBuf {
        self.config_dir().join("recent-files.json")
    }

    /// Returns the thumbnail cache directory.
    pub fn thumbnail_cache_dir(&self) -> PathBuf {
        self.config_dir().join("thumbnails")
    }

    /// Returns the path to the startup rc script.
    pub fn patinaerc_path(&self) -> PatinaercPath {
        if let Some(path) = self.input.patinaerc.clone() {
            PatinaercPath {
                path,
                source: PatinaercSource::Env,
            }
        } else {
            PatinaercPath {
                path: self.config_dir().join("patinaerc"),
                source: PatinaercSource::Default,
            }
        }
    }
}

/// Returns the application configuration directory.
///
/// Checks `PATINAE_CONFIG_DIR` first, then falls back to `~/.patinae`.
pub fn config_dir() -> PathBuf {
    PathResolver::from_process_env().config_dir()
}

/// Returns the plugin directory.
///
/// Checks `PATINAE_PLUGIN_DIR` first, then falls back to `<config_dir>/plugins`.
pub fn plugin_dir() -> PathBuf {
    PathResolver::from_process_env().plugin_dir()
}

/// Returns the resources directory (for CCD cache, etc.).
///
/// Checks `PATINAE_RESOURCES_DIR` first, then falls back to `<config_dir>/resources`.
pub fn resources_dir() -> PathBuf {
    PathResolver::from_process_env().resources_dir()
}

/// Returns the CCD cache path.
///
/// Defaults to `<resources_dir>/components.bin`.
pub fn ccd_cache_path() -> PathBuf {
    PathResolver::from_process_env().ccd_cache_path()
}

/// Returns the recent-files store path.
///
/// Defaults to `<config_dir>/recent-files.json`.
pub fn recent_files_path() -> PathBuf {
    PathResolver::from_process_env().recent_files_path()
}

/// Returns the thumbnail cache directory.
///
/// Defaults to `<config_dir>/thumbnails`.
pub fn thumbnail_cache_dir() -> PathBuf {
    PathResolver::from_process_env().thumbnail_cache_dir()
}

/// Returns the path to the startup rc script.
///
/// Checks `PATINAERC` first, then falls back to `<config_dir>/patinaerc`.
pub fn patinaerc_path() -> PatinaercPath {
    PathResolver::from_process_env().patinaerc_path()
}

fn env_path(key: &str) -> Option<PathBuf> {
    let path = std::env::var_os(key).map(PathBuf::from)?;
    if path.as_os_str().is_empty() {
        None
    } else {
        Some(path)
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn process_home_dir() -> Option<PathBuf> {
    dirs::home_dir()
}

#[cfg(target_arch = "wasm32")]
fn process_home_dir() -> Option<PathBuf> {
    None
}

#[cfg(not(target_arch = "wasm32"))]
fn default_config_dir_from(home_dir: Option<&PathBuf>) -> PathBuf {
    home_dir
        .cloned()
        .unwrap_or_default()
        .join(DEFAULT_CONFIG_DIR_NAME)
}

#[cfg(target_arch = "wasm32")]
fn default_config_dir_from(_home_dir: Option<&PathBuf>) -> PathBuf {
    PathBuf::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolver(input: PathResolverInput) -> PathResolver {
        PathResolver::new(PathResolverInput {
            home_dir: Some(PathBuf::from("/home/patinae")),
            ..input
        })
    }

    #[test]
    fn config_dir_env_override() {
        let paths = resolver(PathResolverInput {
            config_dir: Some(PathBuf::from("/tmp/test-patinae")),
            ..PathResolverInput::default()
        });
        assert_eq!(paths.config_dir(), PathBuf::from("/tmp/test-patinae"));
    }

    #[test]
    fn config_dir_derives_from_home_dir() {
        let paths = resolver(PathResolverInput::default());
        assert_eq!(paths.config_dir(), PathBuf::from("/home/patinae/.patinae"));
    }

    #[test]
    fn config_file_uses_config_dir_independently_of_plugin_dir() {
        let paths = resolver(PathResolverInput {
            config_dir: Some(PathBuf::from("/custom/config")),
            plugin_dir: Some(PathBuf::from("/external/plugins")),
            ..Default::default()
        });
        assert_eq!(
            paths.config_file(),
            PathBuf::from("/custom/config/config.toml")
        );
    }

    #[test]
    fn plugin_dir_derives_from_config_dir() {
        let paths = resolver(PathResolverInput {
            config_dir: Some(PathBuf::from("/tmp/test-patinae")),
            ..PathResolverInput::default()
        });
        assert_eq!(
            paths.plugin_dir(),
            PathBuf::from("/tmp/test-patinae/plugins")
        );
    }

    #[test]
    fn plugin_dir_env_override_independent() {
        let paths = resolver(PathResolverInput {
            config_dir: Some(PathBuf::from("/tmp/test-patinae")),
            plugin_dir: Some(PathBuf::from("/opt/plugins")),
            ..PathResolverInput::default()
        });
        assert_eq!(paths.plugin_dir(), PathBuf::from("/opt/plugins"));
    }

    #[test]
    fn resources_dir_env_override_independent() {
        let paths = resolver(PathResolverInput {
            config_dir: Some(PathBuf::from("/tmp/test-patinae")),
            resources_dir: Some(PathBuf::from("/opt/resources")),
            ..PathResolverInput::default()
        });
        assert_eq!(paths.resources_dir(), PathBuf::from("/opt/resources"));
        assert_eq!(
            paths.ccd_cache_path(),
            PathBuf::from("/opt/resources/components.bin")
        );
    }

    #[test]
    fn patinaerc_path_derives_from_config_dir() {
        let paths = resolver(PathResolverInput {
            config_dir: Some(PathBuf::from("/tmp/test-patinae")),
            ..PathResolverInput::default()
        });

        assert_eq!(
            paths.patinaerc_path(),
            PatinaercPath {
                path: PathBuf::from("/tmp/test-patinae/patinaerc"),
                source: PatinaercSource::Default,
            }
        );
    }

    #[test]
    fn recent_files_path_derives_from_config_dir() {
        let paths = resolver(PathResolverInput {
            config_dir: Some(PathBuf::from("/tmp/test-patinae")),
            ..PathResolverInput::default()
        });

        assert_eq!(
            paths.recent_files_path(),
            PathBuf::from("/tmp/test-patinae/recent-files.json")
        );
    }

    #[test]
    fn thumbnail_cache_dir_derives_from_config_dir() {
        let paths = resolver(PathResolverInput {
            config_dir: Some(PathBuf::from("/tmp/test-patinae")),
            ..PathResolverInput::default()
        });

        assert_eq!(
            paths.thumbnail_cache_dir(),
            PathBuf::from("/tmp/test-patinae/thumbnails")
        );
    }

    #[test]
    fn patinaerc_env_override_independent() {
        let paths = resolver(PathResolverInput {
            config_dir: Some(PathBuf::from("/tmp/test-patinae")),
            patinaerc: Some(PathBuf::from("/tmp/custom-patinaerc")),
            ..PathResolverInput::default()
        });

        assert_eq!(
            paths.patinaerc_path(),
            PatinaercPath {
                path: PathBuf::from("/tmp/custom-patinaerc"),
                source: PatinaercSource::Env,
            }
        );
    }

    #[test]
    fn patinaerc_env_override_has_priority_over_config_dir() {
        let paths = resolver(PathResolverInput {
            config_dir: Some(PathBuf::from("/tmp/test-patinae")),
            patinaerc: Some(PathBuf::from("/tmp/env-patinaerc")),
            ..PathResolverInput::default()
        });

        assert_eq!(
            paths.patinaerc_path().path,
            PathBuf::from("/tmp/env-patinaerc")
        );
    }

    #[test]
    fn patinaerc_env_path_treats_empty_value_as_unset() {
        let key = "PATINAE_TEST_EMPTY_PATH";
        std::env::set_var(key, "");

        assert_eq!(env_path(key), None);

        std::env::remove_var(key);
    }

    #[test]
    fn patinaerc_default_filename_is_not_pymolrc() {
        let paths = resolver(PathResolverInput {
            config_dir: Some(PathBuf::from("/tmp/test-patinae")),
            ..PathResolverInput::default()
        });

        assert_ne!(
            paths.patinaerc_path().path,
            PathBuf::from("/tmp/test-patinae/pymolrc")
        );
    }
}
