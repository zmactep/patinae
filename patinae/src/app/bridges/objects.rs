mod actions;
use actions::*;
mod models;
use models::*;
mod selection;
use selection::*;
mod recent;
use recent::*;
use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;

use slint::{ComponentHandle, Model, ModelRc, VecModel};

use patinae_cmd::{
    AnnotationRequest, LabelExpression, LabelRequest, LabelTarget, MeasurementRequest,
    MeasurementTarget,
};
use patinae_framework::kernel::AppKernel;
use patinae_framework::model::scene::{
    SceneColorContext, SceneEntry, SceneMapVisualKind, SceneModel, SceneObjectCapabilities,
    SceneObjectKind, SidebarColor,
};
use patinae_mol::RepMask;
use patinae_scene::{
    display_atom_path, MeasurementKind, MoleculeObject, ObjectRegistry, RecentAtomId, RecentAtoms,
};

use crate::native_file_actions::quote_command_arg;
use crate::{
    AnnotationTargetItem, AppWindow, ComponentMetrics, DockRow, LayoutState, ObjectItem,
    ObjectsState, OverflowMenuItem, RecentAtomItem, SelectionRow, SubchainItem, TopLevelRow,
};

/// Annotation objects accept at most four ordered atom operands.
const MAX_RECENT_OPERANDS: usize = 4;

// ---------------------------------------------------------------------------
// Selection level
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectionLevel {
    None,
    Groups,
    Objects,
    Subchains,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SelectedCapabilities {
    focus: bool,
    visibility: bool,
    color: bool,
    rename: bool,
    delete: bool,
    representations: bool,
    copy: bool,
    extract: bool,
    align: bool,
    orient: bool,
    remove_atoms: bool,
    all_annotations: bool,
}

impl SelectedCapabilities {
    const fn molecular_selection() -> Self {
        Self {
            focus: true,
            visibility: true,
            color: true,
            rename: true,
            delete: false,
            representations: true,
            copy: true,
            extract: true,
            align: true,
            orient: true,
            remove_atoms: true,
            all_annotations: false,
        }
    }

    const fn from_object(capabilities: SceneObjectCapabilities, kind: SceneObjectKind) -> Self {
        Self {
            focus: capabilities.focus,
            visibility: capabilities.visibility,
            color: capabilities.color,
            rename: capabilities.rename,
            delete: capabilities.delete,
            representations: capabilities.representations,
            copy: capabilities.copy,
            extract: capabilities.extract,
            align: capabilities.align,
            orient: capabilities.orient,
            remove_atoms: capabilities.remove_atoms,
            all_annotations: matches!(kind, SceneObjectKind::Measurement | SceneObjectKind::Label),
        }
    }

    fn intersect(&mut self, other: Self) {
        self.focus &= other.focus;
        self.visibility &= other.visibility;
        self.color &= other.color;
        self.rename &= other.rename;
        self.delete &= other.delete;
        self.representations &= other.representations;
        self.copy &= other.copy;
        self.extract &= other.extract;
        self.align &= other.align;
        self.orient &= other.orient;
        self.remove_atoms &= other.remove_atoms;
        self.all_annotations &= other.all_annotations;
    }
}

impl SelectionLevel {
    fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Groups => "groups",
            Self::Objects => "objects",
            Self::Subchains => "subchains",
        }
    }
}

// ---------------------------------------------------------------------------
// SubchainKey — full identity of a selected subchain row
// ---------------------------------------------------------------------------

/// Identity of a subchain selection. `selector_clause` and `entry_index`
/// come from `SceneSubchain` and are the *only* fields that drive command
/// generation / atom-set membership; `chain_id`/`label`/`kind` are kept for
/// anchor tracking and display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SubchainKey {
    pub obj_name: String,
    pub chain_id: String,
    pub label: String,
    pub kind: String,
    pub entry_index: u32,
    pub selector_clause: String,
}

// ---------------------------------------------------------------------------
// ObjectsBridge
// ---------------------------------------------------------------------------

pub struct ObjectsBridge {
    recent: RecentSelection,
    named_selection: NamedSelection,
    scene_selection: SceneSelection,
    dock_model: Rc<VecModel<DockRow>>,
    dock_dirty: Cell<bool>,
    dock_visible: [bool; 3],
    top_level_model: Rc<VecModel<TopLevelRow>>,
    selections_model: Rc<VecModel<SelectionRow>>,
    recent_atoms_model: Rc<VecModel<RecentAtomItem>>,
    // Selection state machine (groups/objects/subchains — mutually exclusive)

    // Shift-click anchor (for level-based selections)

    // Independent selection tracking (coexists with any level)

    // Last seen selection generation (for independent sync)

    // Recent atoms use their own selection domain and preserve runtime IDs.
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecentAtomBinding {
    key: String,
    id: RecentAtomId,
    path: String,
}

impl ObjectsBridge {
    pub fn new() -> Self {
        Self {
            named_selection: NamedSelection {
                selected_selections: Vec::new(),
                selection_anchor: None,
                last_selection_generation: 0,
            },
            recent: RecentSelection {
                recent_bindings: Vec::new(),
                recent_operands: Vec::new(),
                recent_anchor: None,
                last_recent_observation: None,
                next_recent_key: 1,
            },
            scene_selection: SceneSelection {
                selection_level: SelectionLevel::None,
                selected_groups: Vec::new(),
                selected_objects: Vec::new(),
                selected_subchains: Vec::new(),
                anchor: None,
            },
            dock_model: Rc::new(VecModel::default()),
            dock_dirty: Cell::new(true),
            dock_visible: [false; 3],
            top_level_model: Rc::new(VecModel::default()),
            selections_model: Rc::new(VecModel::default()),
            recent_atoms_model: Rc::new(VecModel::default()),
        }
    }

    /// Attach the VecModels to the Slint global (call once after window creation).
    pub fn attach(&self, window: &AppWindow) {
        let os = window.global::<ObjectsState>();
        os.set_dock_rows(ModelRc::from(self.dock_model.clone()));
        os.set_top_level(ModelRc::from(self.top_level_model.clone()));
        os.set_selections(ModelRc::from(self.selections_model.clone()));
        os.set_recent_atoms(ModelRc::from(self.recent_atoms_model.clone()));
    }

    /// Sync scene data → Slint models. Called each frame; rebuilds only when
    /// the SceneModel or SelectionManager reports changes.
    pub fn sync(&mut self, kernel: &mut AppKernel, window: &AppWindow) {
        let color_ctx = SceneColorContext {
            named_palette: &kernel.session.named_palette,
            palette: &kernel.session.palette,
            settings: &kernel.session.settings,
        };

        let scene_changed = kernel.scene.sync(&kernel.session.registry, &color_ctx);
        let sel_gen = kernel.session.selections.generation();
        let sel_changed = sel_gen != self.named_selection.last_selection_generation;
        let recent_observation = (
            kernel.session.recent_atoms.incarnation(),
            kernel.session.recent_atoms.generation(),
        );
        let recent_changed = self.recent.last_recent_observation != Some(recent_observation);
        let mut selection_state_changed = false;

        if scene_changed {
            selection_state_changed |= self.prune_scene_selection_state(&kernel.scene);
            self.rebuild_top_level_model(&kernel.scene);

            let os = window.global::<ObjectsState>();
            let obj_count: i32 = kernel
                .scene
                .entries
                .iter()
                .map(|e| match e {
                    SceneEntry::Group(g) => g.children.len() as i32,
                    SceneEntry::Object(_) => 1,
                })
                .sum();
            os.set_object_count(obj_count);
        }

        if sel_changed || scene_changed {
            self.named_selection.last_selection_generation = sel_gen;
            selection_state_changed |= self.rebuild_selections(kernel);
        }

        if recent_changed {
            selection_state_changed |= self.sync_recent_atoms(&kernel.session.recent_atoms);
        }

        if selection_state_changed || recent_changed || scene_changed {
            let os = window.global::<ObjectsState>();
            self.update_slint_selection(&os, &kernel.scene, &kernel.session.registry);
        }

        let os = window.global::<ObjectsState>();
        self.sync_dock(window);
        let popover_kind = os.get_popover_kind();
        if recent_changed && matches!(popover_kind.as_str(), "AM" | "AL") {
            os.set_popover_kind("".into());
        } else if scene_changed && popover_kind == "AM" {
            let targets = self.compatible_measurement_targets(&kernel.session.registry);
            if targets.is_empty() {
                os.set_popover_kind("".into());
            } else {
                os.set_measurement_targets(annotation_target_model(targets));
            }
        } else if scene_changed && popover_kind == "AL" {
            let live_targets = self.label_targets(&kernel.session.registry);
            if label_popover_targets_invalidated(&os.get_label_targets(), &live_targets) {
                // LabelPopover owns its selected target locally. Recreate it only
                // when a removed or renamed label could remain selected.
                os.set_popover_kind("".into());
            }
        }
    }

    // --- Selection state machine ---

    // --- Selection helpers ---

    // --- Action pill helpers ---
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// RepSummary — shared accumulator for popover rep/preset/transparency state
// ---------------------------------------------------------------------------

pub(crate) mod events;
pub(crate) use events::setup_callbacks;

#[cfg(test)]
mod tests;
