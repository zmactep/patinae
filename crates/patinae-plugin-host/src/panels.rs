use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

use patinae_framework::component::SharedContext;
use patinae_framework::plugin_ui::{PanelDescriptor, PanelEvent, PanelPlacement, PanelSnapshot};

use crate::host::PluginHost;
use crate::panic::panic_payload_to_string;
use crate::plugin::LoadedPanel;

#[derive(Debug, Clone)]
pub struct PanelStatus {
    pub descriptor: PanelDescriptor,
    pub visible: bool,
    pub active: bool,
}

#[derive(Debug, Clone)]
pub struct PanelFrame {
    pub status: PanelStatus,
    pub snapshot: PanelSnapshot,
}

/// Selection is stored once per placement, never on individual panels.
#[derive(Default)]
pub(crate) struct PanelSelection {
    right: Option<String>,
    bottom: Option<String>,
}

impl PanelSelection {
    fn get(&self, placement: PanelPlacement) -> Option<&str> {
        match placement {
            PanelPlacement::Right => self.right.as_deref(),
            PanelPlacement::Bottom => self.bottom.as_deref(),
        }
    }

    fn set(&mut self, placement: PanelPlacement, id: Option<String>) -> bool {
        let selected = match placement {
            PanelPlacement::Right => &mut self.right,
            PanelPlacement::Bottom => &mut self.bottom,
        };
        if *selected == id {
            return false;
        }
        *selected = id;
        true
    }

    fn is_active(&self, panel: &LoadedPanel) -> bool {
        panel.visible && self.get(panel.descriptor.placement) == Some(panel.descriptor.id.as_str())
    }
}

impl PluginHost {
    pub fn panel_frames(
        &mut self,
        shared: &SharedContext<'_>,
        snapshot_generation: u64,
    ) -> Vec<PanelFrame> {
        self.ensure_single_active_per_placement();
        let mut frames = Vec::new();
        for plugin in &mut self.plugins {
            if plugin.faulted {
                continue;
            }
            let frame_start = frames.len();
            for panel in &mut plugin.panels {
                let status = PanelStatus {
                    descriptor: panel.descriptor.clone(),
                    visible: panel.visible,
                    active: self.panel_selection.is_active(panel),
                };
                let snapshot = if status.active {
                    if panel.cached_snapshot_generation != Some(snapshot_generation) {
                        let result =
                            catch_unwind(AssertUnwindSafe(|| panel.panel.snapshot(shared)));
                        match result {
                            Ok(snapshot) => {
                                panel.cached_snapshot = snapshot;
                                panel.cached_snapshot_generation = Some(snapshot_generation);
                            }
                            Err(panic_info) => {
                                log::error!(
                                    "Plugin '{}' panel '{}' panicked during snapshot: {}. Plugin disabled.",
                                    plugin.metadata.name,
                                    status.descriptor.id,
                                    panic_payload_to_string(&panic_info),
                                );
                                plugin.faulted = true;
                                panel.cached_snapshot = PanelSnapshot::default();
                                panel.cached_snapshot_generation = Some(snapshot_generation);
                                break;
                            }
                        }
                    };
                    panel.cached_snapshot.clone()
                } else {
                    PanelSnapshot::default()
                };
                frames.push(PanelFrame { status, snapshot });
            }
            if plugin.faulted {
                frames.truncate(frame_start);
            }
        }
        self.ensure_single_active_per_placement();
        frames
    }

    pub fn panel_statuses(&self) -> Vec<PanelStatus> {
        self.plugins
            .iter()
            .filter(|p| !p.faulted)
            .flat_map(|plugin| {
                plugin.panels.iter().map(|panel| PanelStatus {
                    descriptor: panel.descriptor.clone(),
                    visible: panel.visible,
                    active: self.panel_selection.is_active(panel),
                })
            })
            .collect()
    }

    pub fn has_panel(&self, id: &str) -> bool {
        self.panel_exists(id)
    }

    pub fn panel_ui_generation(&self) -> u64 {
        self.panel_ui_generation
    }

    pub fn invalidate_panel_ui(&mut self) {
        self.bump_panel_ui_generation();
    }

    pub fn queue_panel_event(&mut self, event: PanelEvent) {
        self.pending_panel_events.push(event);
    }

    pub fn toggle_panel(&mut self, id: &str) -> bool {
        match self.find_panel(id).map(|panel| panel.visible) {
            Some(true) => self.hide_panel(id),
            Some(false) => self.activate_panel(id),
            None => false,
        }
    }

    pub fn show_panel(&mut self, id: &str) -> bool {
        self.activate_panel(id)
    }

    pub fn hide_panel(&mut self, id: &str) -> bool {
        let Some(panel) = self.find_panel_mut(id) else {
            return false;
        };
        let placement = panel.descriptor.placement;
        let changed = std::mem::replace(&mut panel.visible, false);
        let selected = self.first_visible(placement);
        let changed = self.panel_selection.set(placement, selected) || changed;
        self.panel_change(changed)
    }

    pub fn deactivate_placement(&mut self, placement: PanelPlacement) -> bool {
        let mut changed = self.panel_selection.set(placement, None);
        for plugin in &mut self.plugins {
            for panel in &mut plugin.panels {
                if panel.descriptor.placement == placement {
                    changed |= std::mem::replace(&mut panel.visible, false);
                }
            }
        }
        self.panel_change(changed)
    }

    pub fn activate_panel(&mut self, id: &str) -> bool {
        let Some(panel) = self.find_panel_mut(id) else {
            return false;
        };
        let placement = panel.descriptor.placement;
        let changed = !std::mem::replace(&mut panel.visible, true);
        let changed = self.panel_selection.set(placement, Some(id.to_owned())) || changed;
        self.panel_change(changed)
    }

    /// Retains a discovered directory for plugin resource lookup.
    pub fn add_plugin_dir(&mut self, dir: &Path) {
        if !self.plugin_dirs.iter().any(|p| p == dir) {
            self.plugin_dirs.push(dir.to_path_buf());
        }
    }

    pub(crate) fn bump_panel_ui_generation(&mut self) {
        self.panel_ui_generation = self.panel_ui_generation.wrapping_add(1);
    }

    pub(crate) fn panel_exists(&self, id: &str) -> bool {
        self.plugins
            .iter()
            .flat_map(|p| &p.panels)
            .any(|p| p.descriptor.id == id)
    }

    pub(crate) fn find_panel_indices(&self, id: &str) -> Option<(usize, usize)> {
        for (plugin_idx, plugin) in self.plugins.iter().enumerate() {
            for (panel_idx, panel) in plugin.panels.iter().enumerate() {
                if panel.descriptor.id == id {
                    return Some((plugin_idx, panel_idx));
                }
            }
        }
        None
    }

    pub(crate) fn ensure_single_active_per_placement(&mut self) {
        let mut changed = false;
        for placement in [PanelPlacement::Right, PanelPlacement::Bottom] {
            let valid = self
                .panel_selection
                .get(placement)
                .and_then(|id| self.find_panel(id))
                .is_some_and(|panel| panel.visible);
            if !valid {
                let selected = self.first_visible(placement);
                changed |= self.panel_selection.set(placement, selected);
            }
        }
        self.panel_change(changed);
    }

    fn panel_change(&mut self, changed: bool) -> bool {
        if changed {
            self.bump_panel_ui_generation();
        }
        changed
    }

    fn first_visible(&self, placement: PanelPlacement) -> Option<String> {
        self.plugins
            .iter()
            .filter(|plugin| !plugin.faulted)
            .flat_map(|plugin| &plugin.panels)
            .find(|panel| panel.visible && panel.descriptor.placement == placement)
            .map(|panel| panel.descriptor.id.clone())
    }

    fn find_panel(&self, id: &str) -> Option<&LoadedPanel> {
        self.plugins
            .iter()
            .filter(|plugin| !plugin.faulted)
            .flat_map(|plugin| &plugin.panels)
            .find(|panel| panel.descriptor.id == id)
    }

    fn find_panel_mut(&mut self, id: &str) -> Option<&mut LoadedPanel> {
        self.plugins
            .iter_mut()
            .filter(|plugin| !plugin.faulted)
            .flat_map(|plugin| &mut plugin.panels)
            .find(|panel| panel.descriptor.id == id)
    }
}
