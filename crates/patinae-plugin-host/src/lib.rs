//! Runtime host for dynamically loaded Patinae plugins.

mod actions;
mod host;
mod load_session;
mod loader;
mod panel_events;
mod panels;
mod panic;
mod paths;
mod plugin;
mod runtime;

pub use host::PluginHost;
pub use load_session::{PluginApplication, PluginLoadSession, PluginLoadUpdate};
pub use loader::{
    validate_declaration_versions, BackgroundPluginLoader, PluginLoadEvent, PreparedPlugin,
};
pub use panels::{PanelFrame, PanelStatus};
pub use paths::{is_plugin_library_path, library_identity, standard_plugin_dirs, PluginDiscovery};
#[doc(inline)]
pub use paths::{
    plugin_manifest_path, read_plugin_manifest, save_plugin_manifest, PluginManifestDocument,
    PluginManifestEntry,
};
pub use patinae_plugin::registrar::CommandResult;
