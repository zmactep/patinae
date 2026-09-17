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

impl PluginHost {
    pub fn panel_frames(
        &mut self,
        shared: &SharedContext<'_>,
        snapshot_generation: u64,
    ) -> Vec<PanelFrame> {
        let mut frames = Vec::new();
        for plugin in &mut self.plugins {
            if plugin.faulted {
                continue;
            }
            for panel in &mut plugin.panels {
                let status = PanelStatus {
                    descriptor: panel.descriptor.clone(),
                    visible: panel.visible,
                    active: panel.active,
                };
                let snapshot = if panel.visible && panel.active {
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
                            }
                        }
                    };
                    panel.cached_snapshot.clone()
                } else {
                    PanelSnapshot::default()
                };
                frames.push(PanelFrame { status, snapshot });
            }
        }
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
                    active: panel.active,
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
        let before = self.panel_state_signature();
        let Some((placement, visible)) = self
            .find_panel(id)
            .map(|p| (p.descriptor.placement, p.visible))
        else {
            return false;
        };

        if visible {
            if let Some(panel) = self.find_panel_mut(id) {
                panel.visible = false;
                panel.active = false;
            }
            self.activate_first_visible(placement);
        } else {
            self.activate_panel_raw(id, placement);
        }

        self.bump_panel_ui_generation_if_changed(before)
    }

    pub fn show_panel(&mut self, id: &str) -> bool {
        self.activate_panel(id)
    }

    pub fn hide_panel(&mut self, id: &str) -> bool {
        let before = self.panel_state_signature();
        let Some(placement) = self.find_panel(id).map(|p| p.descriptor.placement) else {
            return false;
        };
        if let Some(panel) = self.find_panel_mut(id) {
            panel.visible = false;
            panel.active = false;
        }
        self.activate_first_visible(placement);
        self.bump_panel_ui_generation_if_changed(before)
    }

    pub fn deactivate_placement(&mut self, placement: PanelPlacement) -> bool {
        let before = self.panel_state_signature();
        for plugin in &mut self.plugins {
            for panel in &mut plugin.panels {
                if panel.descriptor.placement == placement {
                    panel.visible = false;
                    panel.active = false;
                }
            }
        }
        self.bump_panel_ui_generation_if_changed(before)
    }

    pub fn activate_panel(&mut self, id: &str) -> bool {
        let before = self.panel_state_signature();
        if id.is_empty() {
            return false;
        }

        let Some(placement) = self.find_panel(id).map(|p| p.descriptor.placement) else {
            return false;
        };

        self.activate_panel_raw(id, placement);
        self.bump_panel_ui_generation_if_changed(before)
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
        for placement in [PanelPlacement::Right, PanelPlacement::Bottom] {
            let mut seen_active = false;
            for plugin in &mut self.plugins {
                for panel in &mut plugin.panels {
                    if panel.descriptor.placement != placement {
                        continue;
                    }
                    if panel.visible && panel.active && !seen_active {
                        seen_active = true;
                    } else if panel.descriptor.placement == placement {
                        panel.active = false;
                    }
                }
            }
            if !seen_active {
                self.activate_first_visible(placement);
            }
        }
    }

    fn bump_panel_ui_generation_if_changed(&mut self, before: Vec<(String, bool, bool)>) -> bool {
        if self.panel_state_signature() == before {
            return false;
        }
        self.bump_panel_ui_generation();
        true
    }

    fn panel_state_signature(&self) -> Vec<(String, bool, bool)> {
        self.plugins
            .iter()
            .filter(|p| !p.faulted)
            .flat_map(|plugin| {
                plugin
                    .panels
                    .iter()
                    .map(|panel| (panel.descriptor.id.clone(), panel.visible, panel.active))
            })
            .collect()
    }

    fn find_panel(&self, id: &str) -> Option<&LoadedPanel> {
        self.plugins
            .iter()
            .flat_map(|p| &p.panels)
            .find(|p| p.descriptor.id == id)
    }

    fn find_panel_mut(&mut self, id: &str) -> Option<&mut LoadedPanel> {
        self.plugins
            .iter_mut()
            .flat_map(|p| &mut p.panels)
            .find(|p| p.descriptor.id == id)
    }

    fn activate_panel_raw(&mut self, id: &str, placement: PanelPlacement) {
        for plugin in &mut self.plugins {
            for panel in &mut plugin.panels {
                if panel.descriptor.placement == placement {
                    panel.active = panel.descriptor.id == id;
                    if panel.active {
                        panel.visible = true;
                    }
                }
            }
        }
    }

    fn activate_first_visible(&mut self, placement: PanelPlacement) {
        let mut activated = false;
        for plugin in &mut self.plugins {
            for panel in &mut plugin.panels {
                if panel.descriptor.placement == placement {
                    panel.active = !activated && panel.visible;
                    activated |= panel.active;
                }
            }
        }
    }
}
