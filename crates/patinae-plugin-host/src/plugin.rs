use libloading::Library;
use patinae_framework::atom_stream::AtomStreamPlan;
use std::collections::HashMap;
use std::sync::Arc;

use patinae_framework::plugin_ui::{PanelDescriptor, PanelSnapshot, PluginPanel};
use patinae_plugin::registrar::{MessageHandler, PluginKeyAction, PluginMetadata};
use patinae_scene::KeyBindings;

pub(crate) struct LoadedPanel {
    pub(crate) descriptor: PanelDescriptor,
    pub(crate) panel: Box<dyn PluginPanel>,
    pub(crate) visible: bool,
    pub(crate) active: bool,
    pub(crate) cached_snapshot_generation: Option<u64>,
    pub(crate) cached_snapshot: PanelSnapshot,
}

impl LoadedPanel {
    pub(crate) fn new(descriptor: PanelDescriptor, panel: Box<dyn PluginPanel>) -> Self {
        Self {
            visible: descriptor.default_visible,
            active: descriptor.default_visible,
            cached_snapshot_generation: None,
            cached_snapshot: PanelSnapshot::default(),
            descriptor,
            panel,
        }
    }
}

#[allow(dead_code)]
pub(crate) enum LibraryHandle {
    Dynamic(Library),
    #[cfg(test)]
    Static,
}

pub(crate) struct LoadedPlugin {
    pub(crate) registration_owner: u64,
    pub(crate) path: Option<std::path::PathBuf>,
    pub(crate) _library: Arc<LibraryHandle>,
    pub(crate) metadata: PluginMetadata,
    pub(crate) message_handler: Option<Box<dyn MessageHandler>>,
    pub(crate) panels: Vec<LoadedPanel>,
    pub(crate) hotkeys: KeyBindings<PluginKeyAction>,
    pub(crate) atom_streams: AtomStreams,
    pub(crate) faulted: bool,
}

#[derive(Default)]
pub(crate) struct AtomStreams {
    pub(crate) next_id: u64,
    pub(crate) streams: HashMap<u64, AtomStreamState>,
}

pub(crate) struct AtomStreamState {
    pub(crate) plan: AtomStreamPlan,
    pub(crate) position: usize,
    pub(crate) chunk_size: usize,
    pub(crate) idle_polls: u32,
}
