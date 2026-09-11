//! Runtime host for dynamically loaded Patinae plugins.

mod actions;
mod host;
mod loader;
mod panel_events;
mod panels;
mod panic;
mod paths;
mod plugin;
mod runtime;

pub use host::PluginHost;
pub use loader::validate_declaration_versions;
pub use panels::{PanelFrame, PanelStatus};
pub use paths::{is_plugin_library_path, standard_plugin_dirs, PluginDiscovery};
pub use patinae_plugin::registrar::CommandResult;
