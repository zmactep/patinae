use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use patinae_cmd::DynamicCommandInvocation;
use patinae_framework::plugin_ui::PanelEvent;
use patinae_plugin::registrar::{
    CommandExecRequest, DynCmdRegistration, KeyBinding, PluginKeyAction, ViewerMutation,
};
use patinae_plugin::wire::WireHostQueryResult;

use crate::plugin::LoadedPlugin;
use crate::CommandResult;

/// Dynamic plugin loader and runtime coordinator.
pub struct PluginHost {
    pub(crate) plugins: Vec<LoadedPlugin>,
    pub(crate) plugin_dirs: Vec<PathBuf>,
    pub(crate) panel_ui_generation: u64,
    pub(crate) pending_executions: Vec<CommandExecRequest>,
    pub(crate) command_results: Vec<Vec<CommandResult>>,
    pub(crate) command_owners: ahash::AHashMap<u64, (usize, u64)>,
    pub(crate) next_command_id: u64,
    pub(crate) host_query_results: Vec<Vec<WireHostQueryResult>>,
    pub(crate) dynamic_invocations: Arc<Mutex<Vec<DynamicCommandInvocation>>>,
    pub(crate) pending_registrations: Vec<DynCmdRegistration>,
    pub(crate) pending_unregistrations: Vec<String>,
    pub(crate) notification_messages: Vec<String>,
    pub(crate) triggered_hotkeys: Vec<TriggeredHotkey>,
    pub(crate) pending_hotkey_registrations: Vec<(String, PluginKeyAction)>,
    pub(crate) pending_hotkey_unregistrations: Vec<String>,
    pub(crate) pending_mutations: Vec<ViewerMutation>,
    pub(crate) pending_panel_events: Vec<PanelEvent>,
}

#[derive(Clone)]
pub(crate) struct TriggeredHotkey {
    pub(crate) plugin_index: usize,
    pub(crate) binding: KeyBinding,
}

impl PluginHost {
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
            plugin_dirs: Vec::new(),
            panel_ui_generation: 0,
            pending_executions: Vec::new(),
            command_results: Vec::new(),
            command_owners: ahash::AHashMap::new(),
            next_command_id: 0,
            host_query_results: Vec::new(),
            dynamic_invocations: Arc::new(Mutex::new(Vec::new())),
            pending_registrations: Vec::new(),
            pending_unregistrations: Vec::new(),
            notification_messages: Vec::new(),
            triggered_hotkeys: Vec::new(),
            pending_hotkey_registrations: Vec::new(),
            pending_hotkey_unregistrations: Vec::new(),
            pending_mutations: Vec::new(),
            pending_panel_events: Vec::new(),
        }
    }

    pub fn invocations_handle(&self) -> Arc<Mutex<Vec<DynamicCommandInvocation>>> {
        self.dynamic_invocations.clone()
    }

    pub fn plugin_count(&self) -> usize {
        self.plugins.len()
    }
}

impl Default for PluginHost {
    fn default() -> Self {
        Self::new()
    }
}
