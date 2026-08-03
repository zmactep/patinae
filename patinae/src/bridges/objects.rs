use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, Model, ModelRc, VecModel};

use patinae_framework::kernel::AppKernel;
use patinae_framework::model::scene::{
    SceneColorContext, SceneEntry, SceneMapVisualKind, SceneModel, SceneObjectCapabilities,
    SceneObjectKind, SidebarColor,
};
use patinae_mol::RepMask;
use patinae_scene::{MeasurementKind, MoleculeObject, ObjectRegistry};

use crate::{
    AppWindow, ObjectItem, ObjectsState, OverflowMenuItem, SelectionRow, SubchainItem, TopLevelRow,
};

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
    top_level_model: Rc<VecModel<TopLevelRow>>,
    selections_model: Rc<VecModel<SelectionRow>>,

    // Selection state machine (groups/objects/subchains — mutually exclusive)
    selection_level: SelectionLevel,
    selected_groups: Vec<String>,
    selected_objects: Vec<String>,
    selected_subchains: Vec<SubchainKey>,

    // Shift-click anchor (for level-based selections)
    anchor: Option<String>, // name or "obj\0chain\0label" for subchains

    // Independent selection tracking (coexists with any level)
    selected_selections: Vec<String>,
    selection_anchor: Option<String>,

    // Last seen selection generation (for independent sync)
    last_selection_generation: u64,
}

impl ObjectsBridge {
    pub fn new() -> Self {
        Self {
            top_level_model: Rc::new(VecModel::default()),
            selections_model: Rc::new(VecModel::default()),
            selection_level: SelectionLevel::None,
            selected_groups: Vec::new(),
            selected_objects: Vec::new(),
            selected_subchains: Vec::new(),
            anchor: None,
            selected_selections: Vec::new(),
            selection_anchor: None,
            last_selection_generation: 0,
        }
    }

    /// Attach the VecModels to the Slint global (call once after window creation).
    pub fn attach(&self, window: &AppWindow) {
        let os = window.global::<ObjectsState>();
        os.set_top_level(ModelRc::from(self.top_level_model.clone()));
        os.set_selections(ModelRc::from(self.selections_model.clone()));
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
        let sel_changed = sel_gen != self.last_selection_generation;
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
            self.last_selection_generation = sel_gen;
            selection_state_changed |= self.rebuild_selections(kernel);
        }

        if selection_state_changed {
            let os = window.global::<ObjectsState>();
            self.update_slint_selection(&os, &kernel.scene);
        }
    }

    fn rebuild_top_level_model(&self, scene: &SceneModel) {
        let mut rows: Vec<TopLevelRow> = Vec::new();

        for entry in &scene.entries {
            match entry {
                SceneEntry::Group(group) => {
                    let children: Vec<ObjectItem> = group
                        .children
                        .iter()
                        .map(|obj| self.build_object_item(obj))
                        .collect();

                    rows.push(TopLevelRow {
                        is_group: true,
                        group_name: group.name.clone().into(),
                        group_open: group.open,
                        group_enabled: group.enabled,
                        group_selected: self.selected_groups.contains(&group.name),
                        group_child_count: children.len() as i32,
                        group_objects: ModelRc::from(Rc::new(VecModel::from(children))),
                        object: ObjectItem::default(),
                    });
                }
                SceneEntry::Object(obj) => {
                    rows.push(TopLevelRow {
                        is_group: false,
                        group_name: Default::default(),
                        group_open: false,
                        group_enabled: false,
                        group_selected: false,
                        group_child_count: 0,
                        group_objects: ModelRc::default(),
                        object: self.build_object_item(obj),
                    });
                }
            }
        }

        replace_model(&self.top_level_model, rows);
    }

    fn build_object_item(&self, obj: &patinae_framework::model::scene::SceneObject) -> ObjectItem {
        let subchains: Vec<SubchainItem> = obj
            .subchains
            .iter()
            .map(|sub| {
                let label = sub.display_label().to_string();
                let selected = self.selected_subchains.iter().any(|k| {
                    k.obj_name == obj.name
                        && k.chain_id == sub.chain_id
                        && k.label == label
                        && k.kind == sub.kind.as_str()
                });
                SubchainItem {
                    chain_id: sub.chain_id.clone().into(),
                    label: label.into(),
                    kind: sub.kind.as_str().into(),
                    atom_count: sub.atom_count as i32,
                    color: sidebar_color_to_slint(sub.color),
                    multicolor: matches!(sub.color, SidebarColor::Multicolor),
                    selected,
                    entry_index: sub.entry_index as i32,
                    selector_clause: sub.selector_clause.clone().into(),
                }
            })
            .collect();

        const MAX_VISIBLE_SUBCHAINS: usize = 100;
        let overflow_count = subchains.len().saturating_sub(MAX_VISIBLE_SUBCHAINS) as i32;
        let subchains: Vec<SubchainItem> =
            subchains.into_iter().take(MAX_VISIBLE_SUBCHAINS).collect();

        ObjectItem {
            name: obj.name.clone().into(),
            object_type: match obj.kind {
                SceneObjectKind::Molecule => "molecule".into(),
                SceneObjectKind::Map => "map".into(),
                SceneObjectKind::Measurement => "measurement".into(),
                SceneObjectKind::Label => "label".into(),
            },
            object_icon_kind: object_icon_kind(obj).into(),
            measurement_kind: measurement_kind_name(obj.measurement_kind).into(),
            entity_count: i32::try_from(obj.entity_count).unwrap_or(i32::MAX),
            has_unresolved_entities: obj.has_unresolved_entities,
            focus_disabled_reason: obj
                .focus_disabled_reason
                .as_deref()
                .unwrap_or_default()
                .into(),
            enabled: obj.enabled,
            expanded: obj.expanded,
            selected: self.selected_objects.contains(&obj.name),
            atom_count: obj.subchains.iter().map(|s| s.atom_count).sum::<usize>() as i32,
            color: sidebar_color_to_slint(obj.color),
            multicolor: matches!(obj.color, SidebarColor::Multicolor),
            has_representations: obj.capabilities.representations,
            can_color: obj.capabilities.color,
            can_copy: obj.capabilities.copy,
            can_extract: obj.capabilities.extract,
            can_align: obj.capabilities.align,
            can_orient: obj.capabilities.orient,
            can_remove_atoms: obj.capabilities.remove_atoms,
            can_delete: obj.capabilities.delete,
            can_rename: obj.capabilities.rename,
            can_group: obj.capabilities.grouping,
            can_focus: obj.capabilities.focus,
            can_toggle: obj.capabilities.visibility,
            subchains: ModelRc::from(Rc::new(VecModel::from(subchains))),
            overflow_count,
        }
    }

    fn rebuild_selections(&mut self, kernel: &AppKernel) -> bool {
        let sel_mgr = &kernel.session.selections;
        let mut rows: Vec<SelectionRow> = Vec::new();
        let mut surviving_names: Vec<String> = Vec::new();
        let before_selected = self.selected_selections.clone();
        let before_anchor = self.selection_anchor.clone();

        for name in sel_mgr.names() {
            let expr = sel_mgr.get_expression(&name).unwrap_or("").to_string();
            let entry = sel_mgr.get(&name);

            let atom_count = if let Some(entry) = entry {
                entry
                    .cached_results
                    .iter()
                    .filter_map(|(obj_name, result)| {
                        let mol = kernel.session.registry.get_molecule(obj_name)?;
                        (result.atom_count() == mol.molecule().atom_count()).then(|| result.count())
                    })
                    .sum::<usize>() as i32
            } else {
                0
            };

            let enabled = entry.map(|e| e.visible).unwrap_or(false);
            let selected = self.selected_selections.contains(&name);

            if selected {
                surviving_names.push(name.clone());
            }

            rows.push(SelectionRow {
                name: name.into(),
                expression: expr.into(),
                atom_count,
                residue_count: 0,
                enabled,
                selected,
            });
        }

        // Prune stale selections (deleted while selected)
        self.selected_selections
            .retain(|n| surviving_names.contains(n));
        if self.selected_selections.is_empty() {
            self.selection_anchor = None;
        }

        replace_model(&self.selections_model, rows);

        self.selected_selections != before_selected || self.selection_anchor != before_anchor
    }

    fn prune_scene_selection_state(&mut self, scene: &SceneModel) -> bool {
        let before_groups = self.selected_groups.clone();
        let before_objects = self.selected_objects.clone();
        let before_subchains = self.selected_subchains.clone();
        let before_level = self.selection_level;
        let before_anchor = self.anchor.clone();

        self.selected_groups
            .retain(|name| scene.get_group(name).is_some());
        self.selected_objects
            .retain(|name| scene.get(name).is_some());
        self.selected_subchains
            .retain(|key| scene_has_subchain(scene, key));

        let active_level_empty = match self.selection_level {
            SelectionLevel::None => false,
            SelectionLevel::Groups => self.selected_groups.is_empty(),
            SelectionLevel::Objects => self.selected_objects.is_empty(),
            SelectionLevel::Subchains => self.selected_subchains.is_empty(),
        };
        if active_level_empty {
            self.selection_level = SelectionLevel::None;
            self.anchor = None;
        } else if self.selected_groups != before_groups
            || self.selected_objects != before_objects
            || self.selected_subchains != before_subchains
        {
            self.anchor = None;
        }

        self.selected_groups != before_groups
            || self.selected_objects != before_objects
            || self.selected_subchains != before_subchains
            || self.selection_level != before_level
            || self.anchor != before_anchor
    }

    // --- Selection state machine ---

    fn switch_level(&mut self, level: SelectionLevel) {
        if self.selection_level != level {
            self.selected_groups.clear();
            self.selected_objects.clear();
            self.selected_subchains.clear();
            self.anchor = None;
            self.selection_level = level;
        }
    }

    fn selected_count(&self) -> i32 {
        let level_count = match self.selection_level {
            SelectionLevel::None => 0,
            SelectionLevel::Groups => self.selected_groups.len() as i32,
            SelectionLevel::Objects => self.selected_objects.len() as i32,
            SelectionLevel::Subchains => self.selected_subchains.len() as i32,
        };
        level_count + self.selected_selections.len() as i32
    }

    /// Return the single selected object that can receive an object movie
    /// keyframe. Named selections, groups, and subchains are intentionally
    /// excluded because `mview store, object=...` needs a concrete object.
    pub fn single_movie_keyframe_object(&self) -> Option<String> {
        if self.selection_level == SelectionLevel::Objects
            && self.selected_objects.len() == 1
            && self.selected_selections.is_empty()
        {
            Some(self.selected_objects[0].clone())
        } else {
            None
        }
    }

    fn update_slint_selection(&self, os: &ObjectsState, scene: &SceneModel) {
        os.set_selected_count(self.selected_count());
        os.set_selection_level(self.selection_level.as_str().into());
        os.set_selected_selection_count(self.selected_selections.len() as i32);
        let capabilities = self.selected_capabilities(scene);
        os.set_action_can_focus(capabilities.is_some_and(|value| value.focus));
        os.set_action_can_orient(capabilities.is_some_and(|value| value.orient));
        os.set_action_can_center(capabilities.is_some_and(|value| value.focus));
        os.set_action_can_toggle(capabilities.is_some_and(|value| value.visibility));
        os.set_action_can_overflow(capabilities.is_some());
        // Rebuild model to reflect selection flags
        // (we need to update selected flags in the existing model)
        self.update_selection_flags_in_model();
        self.update_selection_row_flags();
    }

    fn update_selection_flags_in_model(&self) {
        let count = self.top_level_model.row_count();
        for i in 0..count {
            let Some(mut row) = self.top_level_model.row_data(i) else {
                continue;
            };
            let mut changed = false;

            if row.is_group {
                let want = self.selected_groups.contains(&row.group_name.to_string());
                if row.group_selected != want {
                    row.group_selected = want;
                    changed = true;
                }
                // Update children selection flags
                let children = row.group_objects.clone();
                let child_count = children.row_count();
                for j in 0..child_count {
                    let Some(mut obj) = children.row_data(j) else {
                        continue;
                    };
                    let obj_name = obj.name.to_string();
                    let obj_want = self.selected_objects.contains(&obj_name);
                    if obj.selected != obj_want {
                        obj.selected = obj_want;
                        self.update_subchain_selection(&mut obj);
                        children.set_row_data(j, obj);
                    } else {
                        let sub_changed = self.update_subchain_selection(&mut obj);
                        if sub_changed {
                            children.set_row_data(j, obj);
                        }
                    }
                }
            } else {
                let obj_name = row.object.name.to_string();
                let want = self.selected_objects.contains(&obj_name);
                if row.object.selected != want {
                    row.object.selected = want;
                    changed = true;
                }
                let sub_changed = self.update_subchain_selection(&mut row.object);
                changed = changed || sub_changed;
            }

            if changed {
                self.top_level_model.set_row_data(i, row);
            }
        }
    }

    /// Update subchain selection flags on an ObjectItem. Returns true if any changed.
    fn update_subchain_selection(&self, obj: &mut ObjectItem) -> bool {
        let subchains = obj.subchains.clone();
        let count = subchains.row_count();
        let obj_name = obj.name.to_string();
        let mut any_changed = false;
        for k in 0..count {
            let Some(mut sub) = subchains.row_data(k) else {
                continue;
            };
            let want = self.selected_subchains.iter().any(|key| {
                key.obj_name == obj_name
                    && key.chain_id == sub.chain_id.as_str()
                    && key.label == sub.label.as_str()
                    && key.kind == sub.kind.as_str()
            });
            if sub.selected != want {
                sub.selected = want;
                subchains.set_row_data(k, sub);
                any_changed = true;
            }
        }
        any_changed
    }

    /// Update `selected` flags on each `SelectionRow` in the selections model.
    fn update_selection_row_flags(&self) {
        let count = self.selections_model.row_count();
        for i in 0..count {
            let Some(mut row) = self.selections_model.row_data(i) else {
                continue;
            };
            let name = row.name.to_string();
            let want = self.selected_selections.contains(&name);
            if row.selected != want {
                row.selected = want;
                self.selections_model.set_row_data(i, row);
            }
        }
    }

    // --- Selection helpers ---

    fn click_selections(&mut self, target: String, order: &[String], shift: bool, meta: bool) {
        handle_click(
            &mut self.selected_selections,
            target.clone(),
            target,
            |s| Some(s.to_string()),
            order,
            shift,
            meta,
            &mut self.selection_anchor,
        );
    }

    fn click_groups(&mut self, target: String, order: &[String], shift: bool, meta: bool) {
        self.switch_level(SelectionLevel::Groups);
        let cleared = handle_click(
            &mut self.selected_groups,
            target.clone(),
            target,
            |s| Some(s.to_string()),
            order,
            shift,
            meta,
            &mut self.anchor,
        );
        if cleared {
            self.selection_level = SelectionLevel::None;
        }
    }

    fn click_objects(&mut self, target: String, order: &[String], shift: bool, meta: bool) {
        self.switch_level(SelectionLevel::Objects);
        let cleared = handle_click(
            &mut self.selected_objects,
            target.clone(),
            target,
            |s| Some(s.to_string()),
            order,
            shift,
            meta,
            &mut self.anchor,
        );
        if cleared {
            self.selection_level = SelectionLevel::None;
        }
    }

    fn click_subchains(
        &mut self,
        target: SubchainKey,
        key: String,
        order: &[SubchainKey],
        shift: bool,
        meta: bool,
    ) {
        self.switch_level(SelectionLevel::Subchains);
        let cleared = handle_click(
            &mut self.selected_subchains,
            target,
            key,
            |s| {
                let parts: Vec<&str> = s.split('\0').collect();
                if parts.len() == 3 {
                    // Anchor key carries (obj, chain, label) — find the
                    // matching SubchainKey in the visible order so we can
                    // resolve full identity (entry_index/selector_clause).
                    order
                        .iter()
                        .find(|k| {
                            k.obj_name == parts[0] && k.chain_id == parts[1] && k.label == parts[2]
                        })
                        .cloned()
                } else {
                    None
                }
            },
            order,
            shift,
            meta,
            &mut self.anchor,
        );
        if cleared {
            self.selection_level = SelectionLevel::None;
        }
    }

    // --- Action pill helpers ---

    /// Truncate a name to `max` characters with ellipsis for use in pre-filled inputs.
    fn truncate_name(name: &str, max: usize) -> String {
        if name.len() > max {
            let mut end = max;
            while end > 0 && !name.is_char_boundary(end) {
                end -= 1;
            }
            format!("{}…", &name[..end])
        } else {
            name.to_string()
        }
    }

    fn collect_selected_target(&self) -> Option<String> {
        let level_expr = match self.selection_level {
            SelectionLevel::None => None,
            SelectionLevel::Groups => join_or(&self.selected_groups),
            SelectionLevel::Objects => join_or(&self.selected_objects),
            SelectionLevel::Subchains => build_subchains_expr(&self.selected_subchains),
        };

        let sel_expr = join_or(&self.selected_selections);

        match (level_expr, sel_expr) {
            (Some(l), Some(s)) => Some(format!("{} or {}", l, s)),
            (Some(l), None) => Some(l),
            (None, Some(s)) => Some(s),
            (None, None) => None,
        }
    }

    fn selected_capabilities(&self, scene: &SceneModel) -> Option<SelectedCapabilities> {
        let mut selected: Option<SelectedCapabilities> = None;
        let mut include = |capabilities| match &mut selected {
            Some(current) => current.intersect(capabilities),
            None => selected = Some(capabilities),
        };

        for group_name in &self.selected_groups {
            if let Some(group) = scene.get_group(group_name) {
                if group.children.is_empty() {
                    include(SelectedCapabilities::molecular_selection());
                } else {
                    for child in &group.children {
                        include(SelectedCapabilities::from_object(
                            child.capabilities,
                            child.kind,
                        ));
                    }
                }
            }
        }
        for object_name in &self.selected_objects {
            if let Some(object) = scene.get(object_name) {
                include(SelectedCapabilities::from_object(
                    object.capabilities,
                    object.kind,
                ));
            }
        }
        if !self.selected_subchains.is_empty() || !self.selected_selections.is_empty() {
            include(SelectedCapabilities::molecular_selection());
        }

        selected
    }

    fn compute_overflow_menu(&self, scene: &SceneModel) -> Vec<OverflowMenuItem> {
        let total = self.selected_groups.len()
            + self.selected_objects.len()
            + self.selected_subchains.len()
            + self.selected_selections.len();
        let is_multi = total > 1;
        let Some(capabilities) = self.selected_capabilities(scene) else {
            return Vec::new();
        };

        let is_chain_or_sel = matches!(self.selection_level, SelectionLevel::Subchains)
            || (self.selection_level == SelectionLevel::None
                && !self.selected_selections.is_empty());

        let mut items = Vec::new();

        if is_multi && capabilities.align {
            items.push(OverflowMenuItem {
                action: "align".into(),
                label: "Align".into(),
                disabled: false,
            });
        }

        if !is_multi && capabilities.rename {
            items.push(OverflowMenuItem {
                action: "rename".into(),
                label: "Rename".into(),
                disabled: false,
            });
        }

        if is_chain_or_sel && capabilities.copy {
            items.push(OverflowMenuItem {
                action: "copy".into(),
                label: "Copy".into(),
                disabled: false,
            });
            if capabilities.extract {
                items.push(OverflowMenuItem {
                    action: "extract".into(),
                    label: "Extract".into(),
                    disabled: false,
                });
            }
        } else if capabilities.copy {
            items.push(OverflowMenuItem {
                action: "separator".into(),
                label: "".into(),
                disabled: false,
            });
            items.push(OverflowMenuItem {
                action: "copy".into(),
                label: "Copy".into(),
                disabled: false,
            });
        }

        if capabilities.remove_atoms {
            items.push(OverflowMenuItem {
                action: "remove".into(),
                label: "Remove".into(),
                disabled: false,
            });
        } else if capabilities.all_annotations && capabilities.delete {
            items.push(OverflowMenuItem {
                action: "delete".into(),
                label: "Delete".into(),
                disabled: false,
            });
        }

        if capabilities.color || capabilities.representations {
            items.push(OverflowMenuItem {
                action: "separator".into(),
                label: "".into(),
                disabled: false,
            });
        }
        if capabilities.color {
            items.push(OverflowMenuItem {
                action: "color".into(),
                label: "Color".into(),
                disabled: false,
            });
        }
        if capabilities.representations {
            items.push(OverflowMenuItem {
                action: "representation".into(),
                label: "Representation".into(),
                disabled: false,
            });
        }

        items
    }

    /// Build (mobile, fixed) pair for align: mobile = all but last OR-joined,
    /// fixed = the very last selected item.
    fn collect_align_targets(&self) -> Option<(String, String)> {
        let mut parts: Vec<String> = Vec::new();

        match self.selection_level {
            SelectionLevel::Groups => {
                parts.extend(self.selected_groups.iter().cloned());
            }
            SelectionLevel::Objects => {
                parts.extend(self.selected_objects.iter().cloned());
            }
            SelectionLevel::Subchains => {
                // Build individual subchain expressions
                for key in &self.selected_subchains {
                    parts.push(build_single_subchain_expr(key));
                }
            }
            SelectionLevel::None => {}
        }

        parts.extend(self.selected_selections.iter().cloned());

        if parts.len() < 2 {
            return None;
        }

        let fixed = parts.pop().unwrap();
        let mobile = parts.join(" or ");

        Some((mobile, fixed))
    }
}

fn object_icon_kind(obj: &patinae_framework::model::scene::SceneObject) -> &'static str {
    if let Some(kind) = obj.measurement_kind {
        return match kind {
            MeasurementKind::Distance => "measurement-distance",
            MeasurementKind::Angle => "measurement-angle",
            MeasurementKind::Dihedral => "measurement-dihedral",
        };
    }
    if obj.kind == SceneObjectKind::Label {
        return "label";
    }
    match obj.map_visual_kind {
        Some(SceneMapVisualKind::Source) => "map-source",
        Some(SceneMapVisualKind::Isomesh) => "map-isomesh",
        Some(SceneMapVisualKind::Isosurface) => "map-isosurface",
        None => "",
    }
}

fn measurement_kind_name(kind: Option<MeasurementKind>) -> &'static str {
    match kind {
        Some(MeasurementKind::Distance) => "distance",
        Some(MeasurementKind::Angle) => "angle",
        Some(MeasurementKind::Dihedral) => "dihedral",
        None => "",
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn replace_model<T: Clone + 'static>(model: &Rc<VecModel<T>>, rows: Vec<T>) {
    model.set_vec(rows);
}

/// Map semantic SidebarColor to a concrete Slint color.
fn sidebar_color_to_slint(sc: SidebarColor) -> slint::Color {
    match sc {
        SidebarColor::Solvent => slint::Color::from_rgb_u8(102, 153, 255),
        SidebarColor::Other => slint::Color::from_rgb_u8(128, 128, 128),
        SidebarColor::Multicolor => slint::Color::from_rgb_u8(128, 128, 128),
        SidebarColor::Color(c) => slint::Color::from_rgb_u8(
            (c.r * 255.0) as u8,
            (c.g * 255.0) as u8,
            (c.b * 255.0) as u8,
        ),
    }
}

/// Build the selection expression for the popover target.
///
/// `selector_clause` is precomputed by the scene model from the typed
/// `SubchainKind`/`SubchainLabel` (see
/// `patinae_framework::model::scene::build_selector_clause`). An empty
/// clause means "the whole object".
fn build_popover_target(obj_name: &str, selector_clause: &str) -> String {
    if selector_clause.is_empty() {
        obj_name.to_string()
    } else {
        format!("{} and {}", obj_name, selector_clause)
    }
}

/// Build the human-readable display label for the popover header.
fn build_popover_label(obj_name: &str, chain_id: &str, subchain_label: &str) -> String {
    if chain_id.is_empty() {
        obj_name.to_string()
    } else if subchain_label.is_empty() {
        format!("{} · {}", obj_name, chain_id)
    } else {
        format!("{} · {} ({})", obj_name, chain_id, subchain_label)
    }
}

/// Encode a subchain selection key for anchor tracking.
fn subchain_key(obj: &str, chain: &str, label: &str) -> String {
    format!("{}\0{}\0{}", obj, chain, label)
}

fn scene_has_subchain(scene: &SceneModel, key: &SubchainKey) -> bool {
    scene.get(&key.obj_name).is_some_and(|obj| {
        obj.subchains.iter().any(|sub| {
            sub.chain_id.as_str() == key.chain_id.as_str()
                && sub.display_label() == key.label.as_str()
                && sub.kind.as_str() == key.kind.as_str()
                && sub.entry_index == key.entry_index
                && sub.selector_clause.as_str() == key.selector_clause.as_str()
        })
    })
}

// ---------------------------------------------------------------------------
// RepSummary — shared accumulator for popover rep/preset/transparency state
// ---------------------------------------------------------------------------

/// Accumulated representation state across a set of atoms.
struct RepSummary {
    mask: RepMask,
    preset_sidechain: bool,
    preset_backbone: bool,
    preset_organic: bool,
    preset_solvent: bool,
    preset_inorganic: bool,
    preset_polymer: bool,
    stick_trans: Option<f32>,
    sphere_trans: Option<f32>,
    cartoon_trans: Option<f32>,
    surface_trans: Option<f32>,
}

impl RepSummary {
    fn new() -> Self {
        Self {
            mask: RepMask::NONE,
            preset_sidechain: false,
            preset_backbone: false,
            preset_organic: false,
            preset_solvent: false,
            preset_inorganic: false,
            preset_polymer: false,
            stick_trans: None,
            sphere_trans: None,
            cartoon_trans: None,
            surface_trans: None,
        }
    }

    fn from_mask(mask: RepMask) -> Self {
        Self {
            mask,
            ..Self::new()
        }
    }

    fn accumulate(&mut self, mol_obj: &MoleculeObject, atom: &patinae_mol::Atom) {
        use patinae_mol::AtomFlags;

        let vis = mol_obj.effective_atom_reps(atom);
        self.mask = self.mask.union(vis);

        let has_sticks = vis.is_visible(RepMask::STICKS);
        let has_spheres = vis.is_visible(RepMask::SPHERES);

        if has_sticks && (atom.is_sidechain() || atom.is_ca()) {
            self.preset_sidechain = true;
        }
        if has_sticks && atom.is_backbone() && matches!(&*atom.name, "N" | "C") {
            self.preset_backbone = true;
        }
        if has_sticks && atom.state.flags.contains(AtomFlags::ORGANIC) {
            self.preset_organic = true;
        }
        if vis.is_visible(RepMask::DOTS) && atom.state.flags.contains(AtomFlags::SOLVENT) {
            self.preset_solvent = true;
        }
        if has_spheres && atom.state.flags.contains(AtomFlags::INORGANIC) {
            self.preset_inorganic = true;
        }
        if has_sticks && atom.state.flags.is_biomolecule() {
            self.preset_polymer = true;
        }

        if self.stick_trans.is_none() {
            self.stick_trans = atom.repr.stick_transparency;
        }
        if self.sphere_trans.is_none() {
            self.sphere_trans = atom.repr.sphere_transparency;
        }
        if self.cartoon_trans.is_none() {
            self.cartoon_trans = atom.repr.cartoon_transparency;
        }
        if self.surface_trans.is_none() {
            self.surface_trans = atom.repr.surface_transparency;
        }
    }

    fn apply(&self, os: &ObjectsState, resolved: &patinae_settings::ResolvedSettings) {
        os.set_popover_rep_lines(self.mask.is_visible(RepMask::LINES));
        os.set_popover_rep_sticks(self.mask.is_visible(RepMask::STICKS));
        os.set_popover_rep_spheres(self.mask.is_visible(RepMask::SPHERES));
        os.set_popover_rep_cartoon(self.mask.is_visible(RepMask::CARTOON));
        os.set_popover_rep_ribbon(self.mask.is_visible(RepMask::RIBBON));
        os.set_popover_rep_surface(self.mask.is_visible(RepMask::SURFACE));
        os.set_popover_rep_mesh(self.mask.is_visible(RepMask::MESH));
        os.set_popover_rep_dots(self.mask.is_visible(RepMask::DOTS));

        os.set_popover_preset_sidechain(self.preset_sidechain);
        os.set_popover_preset_backbone(self.preset_backbone);
        os.set_popover_preset_organic(self.preset_organic);
        os.set_popover_preset_solvent(self.preset_solvent);
        os.set_popover_preset_inorganic(self.preset_inorganic);
        os.set_popover_preset_polymer(self.preset_polymer);

        os.set_popover_transparency_sticks(self.stick_trans.unwrap_or(resolved.stick.transparency));
        os.set_popover_transparency_spheres(
            self.sphere_trans.unwrap_or(resolved.sphere.transparency),
        );
        os.set_popover_transparency_cartoon(
            self.cartoon_trans.unwrap_or(resolved.cartoon.transparency),
        );
        os.set_popover_transparency_surface(
            self.surface_trans.unwrap_or(resolved.surface.transparency),
        );
    }
}

/// Compute rep/preset state for an object or subchain and apply to ObjectsState.
///
/// `entry_index` < 0 selects the whole object; otherwise the partition
/// entry identifies the exact subchain.
fn set_active_reps(
    os: &ObjectsState,
    registry: &ObjectRegistry,
    settings: &patinae_settings::Settings,
    obj_name: &str,
    entry_index: i32,
) {
    let Some(mol_obj) = registry.get_molecule(obj_name) else {
        RepSummary::new().apply(
            os,
            &patinae_settings::ResolvedSettings::resolve(settings, None),
        );
        return;
    };

    let mol = mol_obj.molecule();

    let summary = if entry_index < 0 {
        RepSummary::from_mask(mol_obj.effective_reps())
    } else {
        let mut summary = RepSummary::new();
        let partition = mol.subchain_partition();
        if let Some(view) = partition.view_for(entry_index as u32, mol.atoms_slice()) {
            for atom in view.iter() {
                summary.accumulate(mol_obj, atom);
            }
        }
        summary
    };

    use patinae_scene::Object;
    let resolved = patinae_settings::ResolvedSettings::resolve(settings, mol_obj.overrides());
    summary.apply(os, &resolved);
}

/// Compute rep/preset state for a named selection and apply to ObjectsState.
fn set_active_reps_for_selection(
    os: &ObjectsState,
    registry: &ObjectRegistry,
    settings: &patinae_settings::Settings,
    selections: &patinae_scene::SelectionManager,
    selection_name: &str,
) {
    let Some(entry) = selections.get(selection_name) else {
        RepSummary::new().apply(
            os,
            &patinae_settings::ResolvedSettings::resolve(settings, None),
        );
        return;
    };

    let mut summary = RepSummary::new();

    for (obj_name, sel_result) in &entry.cached_results {
        let Some(mol_obj) = registry.get_molecule(obj_name) else {
            continue;
        };
        for (i, atom) in mol_obj.molecule().atoms().enumerate() {
            if !sel_result.contains_index(i) {
                continue;
            }
            summary.accumulate(mol_obj, atom);
        }
    }

    let resolved = patinae_settings::ResolvedSettings::resolve(settings, None);
    summary.apply(os, &resolved);
}

/// Compute rep/preset state across all currently selected
/// groups/objects/subchains/selections and apply to ObjectsState.
fn set_active_reps_for_multi(os: &ObjectsState, kernel: &AppKernel, objects: &ObjectsBridge) {
    let mut summary = RepSummary::new();

    // Groups → children objects' atoms
    for group_name in &objects.selected_groups {
        if let Some(group) = kernel.scene.get_group(group_name) {
            for child in &group.children {
                if let Some(mol_obj) = kernel.session.registry.get_molecule(&child.name) {
                    for atom in mol_obj.molecule().atoms() {
                        summary.accumulate(mol_obj, atom);
                    }
                }
            }
        }
    }

    // Objects → all atoms
    for obj_name in &objects.selected_objects {
        if let Some(mol_obj) = kernel.session.registry.get_molecule(obj_name) {
            for atom in mol_obj.molecule().atoms() {
                summary.accumulate(mol_obj, atom);
            }
        }
    }

    // Subchains → partition view atoms
    for key in &objects.selected_subchains {
        if let Some(mol_obj) = kernel.session.registry.get_molecule(&key.obj_name) {
            let mol = mol_obj.molecule();
            let partition = mol.subchain_partition();
            if let Some(view) = partition.view_for(key.entry_index, mol.atoms_slice()) {
                for atom in view.iter() {
                    summary.accumulate(mol_obj, atom);
                }
            }
        }
    }

    // Selections → cached results
    for sel_name in &objects.selected_selections {
        if let Some(entry) = kernel.session.selections.get(sel_name) {
            for (obj_name, sel_result) in &entry.cached_results {
                if let Some(mol_obj) = kernel.session.registry.get_molecule(obj_name) {
                    for (i, atom) in mol_obj.molecule().atoms().enumerate() {
                        if !sel_result.contains_index(i) {
                            continue;
                        }
                        summary.accumulate(mol_obj, atom);
                    }
                }
            }
        }
    }

    let resolved = patinae_settings::ResolvedSettings::resolve(&kernel.session.settings, None);
    summary.apply(os, &resolved);
}

/// Open a popover from the action pill — resets all per-row fields.
fn open_pill_popover(os: &ObjectsState, kind: &str, target: &str) {
    os.set_popover_source("pill".into());
    os.set_popover_obj_name("".into());
    os.set_popover_chain_id("".into());
    os.set_popover_subchain_label("".into());
    os.set_popover_subchain_kind("".into());
    os.set_popover_entry_index(-1);
    os.set_popover_selector_clause("".into());
    os.set_popover_target(target.into());
    os.set_popover_display_label("".into());
    os.set_popover_cmd_preview("".into());
    os.set_popover_kind(kind.into());
}

fn uses_solid_color_popover(scene: &SceneModel, object_name: &str) -> bool {
    scene
        .get(object_name)
        .is_some_and(|object| object.capabilities.color && !object.capabilities.representations)
}

/// Open the name-input popup (rename / copy / extract).
fn open_name_popup(os: &ObjectsState, action: &str, target: &str, short: &str) {
    let (suffix, title_label) = match action {
        "rename" => ("_renamed", "Rename"),
        "copy" => ("_copy", "Copy"),
        "extract" => ("_extract", "Extract from"),
        _ => return,
    };
    os.set_name_input_action(action.into());
    os.set_name_input_target(target.into());
    os.set_name_input_text(format!("{}{}", short, suffix).into());
    os.set_name_input_title(format!("{} {}", title_label, target).into());
    os.set_popover_kind("N".into());
}

/// Join names with ` or `, returning `None` if empty.
fn join_or(names: &[String]) -> Option<String> {
    if names.is_empty() {
        None
    } else {
        Some(names.join(" or "))
    }
}

/// Build a single selection expression from multiple selected
/// subchains. Each `SubchainKey` carries its own pre-baked
/// `selector_clause` (synthesized in the scene model from typed
/// `SubchainKind`/`SubchainLabel`); this function only groups by
/// `obj_name` and joins.
fn build_subchains_expr(subchains: &[SubchainKey]) -> Option<String> {
    if subchains.is_empty() {
        return None;
    }

    // Group by object name, preserving insertion order; collect distinct
    // selector clauses per object (empty clause = whole-object scope).
    let mut obj_groups: Vec<(&str, Vec<&str>)> = Vec::new();
    for key in subchains {
        let clause = key.selector_clause.as_str();
        if let Some(entry) = obj_groups
            .iter_mut()
            .find(|(k, _)| *k == key.obj_name.as_str())
        {
            if !entry.1.contains(&clause) {
                entry.1.push(clause);
            }
        } else {
            obj_groups.push((key.obj_name.as_str(), vec![clause]));
        }
    }

    let mut obj_exprs: Vec<String> = Vec::new();
    for (obj, clauses) in &obj_groups {
        // Whole-object selection wins: if any selected row covers the
        // whole object, all other clauses for that object are redundant.
        if clauses.iter().any(|c| c.is_empty()) {
            obj_exprs.push((*obj).to_string());
            continue;
        }
        let expr = if clauses.len() == 1 {
            format!("{} and {}", obj, clauses[0])
        } else {
            let joined = clauses.to_vec().join(" or ");
            format!("{} and ({})", obj, joined)
        };
        obj_exprs.push(expr);
    }

    if obj_exprs.len() == 1 {
        Some(obj_exprs.into_iter().next().unwrap())
    } else {
        let parts: Vec<String> = obj_exprs.into_iter().map(|e| format!("({})", e)).collect();
        Some(parts.join(" or "))
    }
}

/// Build a selection expression for a single subchain.
fn build_single_subchain_expr(key: &SubchainKey) -> String {
    if key.selector_clause.is_empty() {
        key.obj_name.clone()
    } else {
        format!("{} and {}", key.obj_name, key.selector_clause)
    }
}

fn visible_order_groups(scene: &SceneModel) -> Vec<String> {
    scene
        .entries
        .iter()
        .filter_map(|e| match e {
            SceneEntry::Group(g) => Some(g.name.clone()),
            _ => None,
        })
        .collect()
}

fn visible_order_objects(scene: &SceneModel) -> Vec<String> {
    let mut order = Vec::new();
    for entry in &scene.entries {
        match entry {
            SceneEntry::Object(obj) => order.push(obj.name.clone()),
            SceneEntry::Group(g) if g.open => {
                for child in &g.children {
                    order.push(child.name.clone());
                }
            }
            _ => {}
        }
    }
    order
}

fn visible_order_subchains(scene: &SceneModel) -> Vec<SubchainKey> {
    fn push_obj(order: &mut Vec<SubchainKey>, obj: &patinae_framework::model::scene::SceneObject) {
        for sub in &obj.subchains {
            order.push(SubchainKey {
                obj_name: obj.name.clone(),
                chain_id: sub.chain_id.clone(),
                label: sub.display_label().to_string(),
                kind: sub.kind.as_str().to_string(),
                entry_index: sub.entry_index,
                selector_clause: sub.selector_clause.clone(),
            });
        }
    }

    let mut order = Vec::new();
    for entry in &scene.entries {
        match entry {
            SceneEntry::Object(obj) if obj.expanded => push_obj(&mut order, obj),
            SceneEntry::Group(g) if g.open => {
                for child in &g.children {
                    if child.expanded {
                        push_obj(&mut order, child);
                    }
                }
            }
            _ => {}
        }
    }
    order
}

fn visible_order_selections(bridge: &ObjectsBridge) -> Vec<String> {
    let count = bridge.selections_model.row_count();
    let mut order = Vec::with_capacity(count);
    for i in 0..count {
        if let Some(row) = bridge.selections_model.row_data(i) {
            order.push(row.name.to_string());
        }
    }
    order
}

/// Select range between two items in an ordered list.
fn select_range<T: PartialEq + Clone>(order: &[T], anchor: &T, target: &T) -> Vec<T> {
    let a_pos = order.iter().position(|x| x == anchor);
    let t_pos = order.iter().position(|x| x == target);
    match (a_pos, t_pos) {
        (Some(a), Some(t)) => {
            let (lo, hi) = if a <= t { (a, t) } else { (t, a) };
            order[lo..=hi].to_vec()
        }
        _ => vec![target.clone()],
    }
}

/// Unified shift / meta / plain click selection logic.
#[expect(
    clippy::too_many_arguments,
    reason = "UI selection helper keeps each click input explicit at call sites"
)]
fn handle_click<T: PartialEq + Clone>(
    selected: &mut Vec<T>,
    target: T,
    anchor_key: String,
    resolve_anchor: impl FnOnce(&str) -> Option<T>,
    order: &[T],
    shift: bool,
    meta: bool,
    anchor: &mut Option<String>,
) -> bool {
    if shift {
        if let Some(ref ak) = *anchor {
            if let Some(anchor_item) = resolve_anchor(ak) {
                let range = select_range(order, &anchor_item, &target);
                if selected.iter().any(|x| x == &target) {
                    selected.retain(|x| !range.contains(x));
                    if selected.is_empty() {
                        *anchor = None;
                        return true; // cleared
                    } else {
                        *anchor = Some(anchor_key);
                    }
                } else {
                    *selected = range;
                    *anchor = Some(anchor_key);
                }
            }
        } else {
            *selected = vec![target];
            *anchor = Some(anchor_key);
        }
    } else if meta {
        if let Some(pos) = selected.iter().position(|x| x == &target) {
            selected.remove(pos);
        } else {
            selected.push(target);
        }
        *anchor = Some(anchor_key);
    } else {
        let is_sole = selected.len() == 1 && selected[0] == target;
        if is_sole {
            selected.clear();
            *anchor = None;
            return true; // cleared
        } else {
            *selected = vec![target];
            *anchor = Some(anchor_key);
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Callback wiring
// ---------------------------------------------------------------------------

pub fn setup_callbacks(app: Rc<RefCell<crate::app::App>>, window: &AppWindow) {
    let os = window.global::<ObjectsState>();

    // --- Group clicked ---
    {
        let app = app.clone();
        let weak = window.as_weak();
        os.on_group_clicked(move |name, shift, meta| {
            let mut a = app.borrow_mut();
            let Some(w) = weak.upgrade() else { return };
            let os = w.global::<ObjectsState>();
            let name = name.to_string();

            let order = visible_order_groups(&a.kernel.scene);
            a.objects.click_groups(name, &order, shift, meta);

            a.objects.update_slint_selection(&os, &a.kernel.scene);
        });
    }

    // --- Object clicked ---
    {
        let app = app.clone();
        let weak = window.as_weak();
        os.on_object_clicked(move |name, shift, meta| {
            let mut a = app.borrow_mut();
            let Some(w) = weak.upgrade() else { return };
            let os = w.global::<ObjectsState>();
            let name = name.to_string();

            let order = visible_order_objects(&a.kernel.scene);
            a.objects.click_objects(name, &order, shift, meta);

            a.objects.update_slint_selection(&os, &a.kernel.scene);
        });
    }

    // --- Subchain clicked ---
    {
        let app = app.clone();
        let weak = window.as_weak();
        os.on_subchain_clicked(
            move |obj_name, chain_id, label, kind, entry_index, selector_clause, shift, meta| {
                let mut a = app.borrow_mut();
                let Some(w) = weak.upgrade() else { return };
                let os = w.global::<ObjectsState>();
                let obj_name = obj_name.to_string();
                let chain_id = chain_id.to_string();
                let label = label.to_string();
                let kind = kind.to_string();
                let key = subchain_key(&obj_name, &chain_id, &label);
                let target = SubchainKey {
                    obj_name: obj_name.clone(),
                    chain_id: chain_id.clone(),
                    label: label.clone(),
                    kind: kind.clone(),
                    entry_index: entry_index.max(0) as u32,
                    selector_clause: selector_clause.to_string(),
                };

                let order = visible_order_subchains(&a.kernel.scene);
                a.objects.click_subchains(target, key, &order, shift, meta);

                a.objects.update_slint_selection(&os, &a.kernel.scene);
            },
        );
    }

    // --- Toggle group open ---
    {
        let app = app.clone();
        os.on_toggle_group_open(move |name| {
            let mut a = app.borrow_mut();
            let name = name.to_string();
            a.kernel.scene.toggle_group_open(&name);
            a.kernel.scene.invalidate();
        });
    }

    // --- Toggle object expand ---
    {
        let app = app.clone();
        os.on_toggle_object_expand(move |name| {
            let mut a = app.borrow_mut();
            let name = name.to_string();
            a.kernel.scene.toggle_expanded(&name);
            a.kernel.scene.invalidate();
        });
    }

    // --- Selection clicked (select, not toggle visibility) ---
    {
        let app = app.clone();
        let weak = window.as_weak();
        os.on_selection_clicked(move |name, shift, meta| {
            let mut a = app.borrow_mut();
            let Some(w) = weak.upgrade() else { return };
            let os = w.global::<ObjectsState>();
            let name = name.to_string();

            let order = visible_order_selections(&a.objects);
            a.objects.click_selections(name, &order, shift, meta);

            a.objects.update_slint_selection(&os, &a.kernel.scene);
        });
    }

    // --- Row button clicked (opens popover) ---
    {
        let app = app.clone();
        let weak = window.as_weak();
        os.on_row_btn_clicked(
            move |kind,
                  source,
                  obj_name,
                  chain_id,
                  subchain_label,
                  subchain_kind,
                  entry_index,
                  selector_clause| {
                let Some(w) = weak.upgrade() else { return };
                let os = w.global::<ObjectsState>();
                let kind = kind.to_string();
                let source = source.to_string();
                let obj_name = obj_name.to_string();
                let chain_id = chain_id.to_string();
                let subchain_label = subchain_label.to_string();
                let subchain_kind = subchain_kind.to_string();
                let selector_clause = selector_clause.to_string();

                // Toggle: if same popover already open, close it
                if os.get_popover_kind() == kind.as_str()
                    && os.get_popover_source() == source.as_str()
                    && os.get_popover_obj_name() == obj_name.as_str()
                    && os.get_popover_chain_id() == chain_id.as_str()
                    && os.get_popover_subchain_label() == subchain_label.as_str()
                {
                    os.set_popover_kind("".into());
                    return;
                }

                if source == "selection" {
                    os.set_popover_solid_only(false);
                    // Selection-sourced popover: target is the selection name
                    if kind == "R" {
                        let a = app.borrow();
                        set_active_reps_for_selection(
                            &os,
                            &a.kernel.session.registry,
                            &a.kernel.session.settings,
                            &a.kernel.session.selections,
                            &obj_name,
                        );
                    }

                    os.set_popover_kind(kind.into());
                    os.set_popover_source("selection".into());
                    os.set_popover_obj_name(obj_name.clone().into());
                    os.set_popover_chain_id("".into());
                    os.set_popover_subchain_label("".into());
                    os.set_popover_subchain_kind("".into());
                    os.set_popover_entry_index(-1);
                    os.set_popover_selector_clause("".into());
                    os.set_popover_target(obj_name.clone().into());
                    os.set_popover_display_label(obj_name.into());
                    os.set_popover_cmd_preview("".into());
                } else {
                    // Object/subchain-sourced popover
                    let target = build_popover_target(&obj_name, &selector_clause);
                    let label = build_popover_label(&obj_name, &chain_id, &subchain_label);
                    let solid_only = {
                        let a = app.borrow();
                        uses_solid_color_popover(&a.kernel.scene, &obj_name)
                    };
                    os.set_popover_solid_only(solid_only);

                    // Reset scope to "all" when landing on a disabled scope button.
                    let current_scope = os.get_popover_scope().to_string();
                    let is_bio = subchain_kind.is_empty() || subchain_kind == "biopolymer";
                    let is_bio_or_organic = is_bio || subchain_kind == "organic";
                    if solid_only
                        || (!is_bio && current_scope == "cartoon")
                        || (!is_bio_or_organic && current_scope == "all-c")
                    {
                        os.set_popover_scope("all".into());
                    }

                    if kind == "R" {
                        let a = app.borrow();
                        set_active_reps(
                            &os,
                            &a.kernel.session.registry,
                            &a.kernel.session.settings,
                            &obj_name,
                            entry_index,
                        );
                    }

                    os.set_popover_kind(kind.into());
                    os.set_popover_source("object".into());
                    os.set_popover_obj_name(obj_name.into());
                    os.set_popover_chain_id(chain_id.into());
                    os.set_popover_subchain_label(subchain_label.into());
                    os.set_popover_subchain_kind(subchain_kind.into());
                    os.set_popover_entry_index(entry_index);
                    os.set_popover_selector_clause(selector_clause.into());
                    os.set_popover_target(target.into());
                    os.set_popover_display_label(label.into());
                    os.set_popover_cmd_preview("".into());
                }
            },
        );
    }

    // --- Popover close ---
    {
        let weak = window.as_weak();
        os.on_popover_close(move || {
            let Some(w) = weak.upgrade() else { return };
            let os = w.global::<ObjectsState>();
            os.set_popover_kind("".into());
        });
    }

    // --- Popover execute (reads cmd-preview, executes as command) ---
    {
        let app = app.clone();
        let weak = window.as_weak();
        os.on_popover_execute(move || {
            let mut a = app.borrow_mut();
            let Some(w) = weak.upgrade() else { return };
            let os = w.global::<ObjectsState>();
            let cmd = os.get_popover_cmd_preview().to_string();
            if !cmd.is_empty() {
                a.kernel.bus.execute_command(&cmd);
            }

            // Optimistic update: the command is queued (async), so we can't
            // recompute from atoms yet. Instead, parse the command to flip
            // the corresponding boolean immediately.
            if os.get_popover_kind() == "R" && !cmd.is_empty() {
                let showing = cmd.starts_with("show ");
                let is_show_or_hide = showing || cmd.starts_with("hide ");
                if is_show_or_hide {
                    // Detect preset commands by the suffix BEYOND the known
                    // target. The target itself may contain "and polymer" etc.
                    // for bio chains, so `cmd.contains(...)` would false-match.
                    let selection = cmd.split_once(',').map(|(_, s)| s.trim()).unwrap_or("");
                    let target = os.get_popover_target().to_string();
                    let suffix = selection
                        .strip_prefix(target.as_str())
                        .unwrap_or(selection)
                        .trim();

                    // `ends_with` (with leading space) avoids the substring
                    // collision where "inorganic" matches the "organic"
                    // branch. `inorganic` is checked before `organic` for
                    // belt-and-braces.
                    if suffix.contains("(sidechain") {
                        os.set_popover_preset_sidechain(showing);
                    } else if suffix.ends_with(" backbone") {
                        os.set_popover_preset_backbone(showing);
                    } else if suffix.ends_with(" inorganic") {
                        os.set_popover_preset_inorganic(showing);
                    } else if suffix.ends_with(" organic") {
                        os.set_popover_preset_organic(showing);
                    } else if suffix.ends_with(" solvent") {
                        os.set_popover_preset_solvent(showing);
                    } else if suffix.ends_with(" polymer") {
                        os.set_popover_preset_polymer(showing);
                    } else {
                        // Plain rep toggle: "show/hide <rep>, <target>"
                        let rep = cmd[5..].split(',').next().unwrap_or("").trim();
                        match rep {
                            "lines" => os.set_popover_rep_lines(showing),
                            "sticks" => os.set_popover_rep_sticks(showing),
                            "spheres" => os.set_popover_rep_spheres(showing),
                            "cartoon" => os.set_popover_rep_cartoon(showing),
                            "ribbon" => os.set_popover_rep_ribbon(showing),
                            "surface" => os.set_popover_rep_surface(showing),
                            "mesh" => os.set_popover_rep_mesh(showing),
                            "dots" => os.set_popover_rep_dots(showing),
                            _ => {}
                        }
                    }
                }
            }
        });
    }

    // --- Action pill: zoom ---
    {
        let app = app.clone();
        os.on_action_zoom(move || {
            let mut a = app.borrow_mut();
            let can_focus = a
                .objects
                .selected_capabilities(&a.kernel.scene)
                .is_some_and(|capabilities| capabilities.focus);
            if can_focus {
                let Some(target) = a.objects.collect_selected_target() else {
                    return;
                };
                a.kernel.bus.execute_command(format!("zoom {}", target));
            }
        });
    }

    // --- Action pill: orient ---
    {
        let app = app.clone();
        os.on_action_orient(move || {
            let mut a = app.borrow_mut();
            let can_orient = a
                .objects
                .selected_capabilities(&a.kernel.scene)
                .is_some_and(|capabilities| capabilities.orient);
            if can_orient {
                let Some(target) = a.objects.collect_selected_target() else {
                    return;
                };
                a.kernel.bus.execute_command(format!("orient {}", target));
            }
        });
    }

    // --- Action pill: center ---
    {
        let app = app.clone();
        os.on_action_center(move || {
            let mut a = app.borrow_mut();
            let can_focus = a
                .objects
                .selected_capabilities(&a.kernel.scene)
                .is_some_and(|capabilities| capabilities.focus);
            if can_focus {
                let Some(target) = a.objects.collect_selected_target() else {
                    return;
                };
                a.kernel.bus.execute_command(format!("center {}", target));
            }
        });
    }

    // --- Action pill: toggle visibility ---
    {
        let app = app.clone();
        os.on_action_toggle(move || {
            let mut a = app.borrow_mut();
            let can_toggle = a
                .objects
                .selected_capabilities(&a.kernel.scene)
                .is_some_and(|capabilities| capabilities.visibility);
            if !can_toggle {
                return;
            }

            // Toggle level-based selections (groups/objects)
            match a.objects.selection_level {
                SelectionLevel::Groups => {
                    let groups = a.objects.selected_groups.clone();
                    for group_name in groups {
                        if let Some(group) = a.kernel.scene.get_group(&group_name) {
                            let children: Vec<String> =
                                group.children.iter().map(|o| o.name.clone()).collect();
                            for child in children {
                                a.kernel.bus.execute_command(format!("toggle {}", child));
                            }
                        }
                    }
                }
                SelectionLevel::Objects => {
                    let objects = a.objects.selected_objects.clone();
                    for name in objects {
                        a.kernel.bus.execute_command(format!("toggle {}", name));
                    }
                }
                _ => {} // Subchains can't be toggled
            }

            // Toggle selected named selections' visibility indicators
            let sel_names = a.objects.selected_selections.clone();
            for name in sel_names {
                let sel_mgr = &mut a.kernel.session.selections;
                let visible = sel_mgr.is_visible(&name);
                sel_mgr.set_visible(&name, !visible);
            }
        });
    }

    // --- Action pill: overflow menu (three-dots) ---
    {
        let app = app.clone();
        let weak = window.as_weak();
        os.on_action_overflow(move || {
            let Some(w) = weak.upgrade() else { return };
            let os = w.global::<ObjectsState>();
            let a = app.borrow();

            // Toggle: close if already open
            if os.get_popover_kind() == "M" {
                os.set_popover_kind("".into());
                return;
            }

            let items = a.objects.compute_overflow_menu(&a.kernel.scene);
            let model: Rc<VecModel<OverflowMenuItem>> = Rc::new(VecModel::from(items));
            os.set_overflow_menu_items(ModelRc::from(model));
            os.set_popover_kind("M".into());
        });
    }

    // --- Overflow menu: execute action ---
    {
        let app = app.clone();
        let weak = window.as_weak();
        os.on_overflow_action(move |action| {
            let mut a = app.borrow_mut();
            let Some(w) = weak.upgrade() else { return };
            let os = w.global::<ObjectsState>();
            let action = action.to_string();
            let Some(capabilities) = a.objects.selected_capabilities(&a.kernel.scene) else {
                return;
            };
            let Some(target) = a.objects.collect_selected_target() else {
                return;
            };
            let short = ObjectsBridge::truncate_name(&target, 28);

            match action.as_str() {
                "rename" if capabilities.rename => open_name_popup(&os, "rename", &target, &short),
                "copy" if capabilities.copy => open_name_popup(&os, "copy", &target, &short),
                "extract" if capabilities.extract => {
                    open_name_popup(&os, "extract", &target, &short)
                }
                "remove" if capabilities.remove_atoms => {
                    let cmd = format!("remove {}", target);
                    a.kernel.bus.execute_command(&cmd);
                    os.set_popover_kind("".into());
                }
                "delete" if capabilities.all_annotations && capabilities.delete => {
                    let targets: Vec<String> = a
                        .objects
                        .selected_groups
                        .iter()
                        .chain(&a.objects.selected_objects)
                        .cloned()
                        .collect();
                    for name in targets {
                        a.kernel.bus.execute_command(format!("delete {}", name));
                    }
                    os.set_popover_kind("".into());
                }
                "align" if capabilities.align => {
                    if let Some((mobile, fixed)) = a.objects.collect_align_targets() {
                        let cmd = format!("align {}, {}", mobile, fixed);
                        a.kernel.bus.execute_command(&cmd);
                    }
                    os.set_popover_kind("".into());
                }
                "color" if capabilities.color => {
                    if os.get_popover_kind() == "C" {
                        os.set_popover_kind("".into());
                        return;
                    }
                    os.set_popover_scope("all".into());
                    os.set_popover_solid_only(!capabilities.representations);
                    open_pill_popover(&os, "C", &target);
                }
                "representation" if capabilities.representations => {
                    if os.get_popover_kind() == "R" {
                        os.set_popover_kind("".into());
                        return;
                    }
                    os.set_popover_solid_only(false);
                    set_active_reps_for_multi(&os, &a.kernel, &a.objects);
                    open_pill_popover(&os, "R", &target);
                }
                _ => {}
            }
        });
    }

    // --- Name input confirm (rename / copy / extract from action pill) ---
    {
        let app = app.clone();
        let weak = window.as_weak();
        os.on_name_input_confirm(move || {
            let mut a = app.borrow_mut();
            let Some(w) = weak.upgrade() else { return };
            let os = w.global::<ObjectsState>();
            let action = os.get_name_input_action().to_string();
            let target = os.get_name_input_target().to_string();
            let new_name = os.get_name_input_text().to_string();

            if action.is_empty() || target.is_empty() || new_name.is_empty() {
                os.set_popover_kind("".into());
                return;
            }

            let cmd = match action.as_str() {
                "rename" => format!("set_name {}, {}", target, new_name),
                "copy" => format!("copy {}, {}", new_name, target),
                "extract" => format!("extract {}, {}", new_name, target),
                _ => return,
            };
            a.kernel.bus.execute_command(&cmd);

            if action == "rename" {
                a.objects.selected_groups.clear();
                a.objects.selected_objects.clear();
                a.objects.selected_subchains.clear();
                a.objects.selected_selections.clear();
                a.objects.selection_level = SelectionLevel::None;
                a.objects.anchor = None;
                a.objects.selection_anchor = None;
                a.objects.update_slint_selection(&os, &a.kernel.scene);
            }

            os.set_popover_kind("".into());
        });
    }

    // --- Right-click: toggle object visibility ---
    {
        let app = app.clone();
        os.on_object_right_clicked(move |name| {
            let mut a = app.borrow_mut();
            a.kernel.bus.execute_command(format!("toggle {}", name));
        });
    }

    // --- Right-click: toggle selection visibility ---
    {
        let app = app.clone();
        os.on_selection_right_clicked(move |name| {
            let mut a = app.borrow_mut();
            let sel_mgr = &mut a.kernel.session.selections;
            let visible = sel_mgr.is_visible(&name);
            sel_mgr.set_visible(&name, !visible);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use patinae_mol::{Atom, Element, ObjectMolecule, RepMask};

    /// Build a `SubchainKey` for tests. `entry_index` is irrelevant to
    /// `build_subchains_expr` (which only consumes `selector_clause` and
    /// `obj_name`), so we use `0` as a placeholder.
    fn key(obj: &str, chain: &str, label: &str, kind: &str, clause: &str) -> SubchainKey {
        SubchainKey {
            obj_name: obj.into(),
            chain_id: chain.into(),
            label: label.into(),
            kind: kind.into(),
            entry_index: 0,
            selector_clause: clause.into(),
        }
    }

    fn capabilities_for(kind: SceneObjectKind) -> SceneObjectCapabilities {
        let annotation = matches!(kind, SceneObjectKind::Measurement | SceneObjectKind::Label);
        SceneObjectCapabilities {
            focus: true,
            visibility: true,
            color: true,
            rename: true,
            delete: true,
            grouping: true,
            representations: kind == SceneObjectKind::Molecule,
            copy: !annotation,
            extract: !annotation,
            align: !annotation,
            orient: !annotation,
            remove_atoms: !annotation,
        }
    }

    fn scene_object(
        name: &str,
        kind: SceneObjectKind,
    ) -> patinae_framework::model::scene::SceneObject {
        patinae_framework::model::scene::SceneObject {
            name: name.to_string(),
            kind,
            map_visual_kind: None,
            measurement_kind: (kind == SceneObjectKind::Measurement)
                .then_some(MeasurementKind::Distance),
            entity_count: usize::from(matches!(
                kind,
                SceneObjectKind::Measurement | SceneObjectKind::Label
            )),
            has_unresolved_entities: false,
            focus_disabled_reason: None,
            capabilities: capabilities_for(kind),
            color: SidebarColor::Other,
            enabled: true,
            expanded: false,
            subchains: Vec::new(),
        }
    }

    fn unresolved_annotation(
        name: &str,
        kind: SceneObjectKind,
    ) -> patinae_framework::model::scene::SceneObject {
        let mut object = scene_object(name, kind);
        object.has_unresolved_entities = true;
        object.focus_disabled_reason = Some("No resolvable anchors".to_string());
        object.capabilities.focus = false;
        object
    }

    fn annotation_kinds() -> [SceneObjectKind; 2] {
        [SceneObjectKind::Measurement, SceneObjectKind::Label]
    }

    fn assert_annotation_overflow(kind: SceneObjectKind) {
        let mut scene = SceneModel::new();
        scene
            .entries
            .push(SceneEntry::Object(scene_object("annotation", kind)));
        let mut bridge = ObjectsBridge::new();
        bridge.selection_level = SelectionLevel::Objects;
        bridge.selected_objects.push("annotation".to_string());

        let capabilities = bridge
            .selected_capabilities(&scene)
            .expect("annotation capabilities");
        assert!(capabilities.focus);
        assert!(capabilities.visibility);
        assert!(capabilities.color);
        assert!(!capabilities.orient);
        assert!(!capabilities.remove_atoms);

        let actions = action_names(&bridge.compute_overflow_menu(&scene));
        assert!(actions.iter().any(|action| action == "rename"));
        assert!(actions.iter().any(|action| action == "delete"));
        assert!(actions.iter().any(|action| action == "color"));
        for hidden in ["remove", "copy", "extract", "align", "representation"] {
            assert!(!actions.iter().any(|action| action == hidden), "{hidden}");
        }
    }

    #[test]
    fn annotation_overflow_uses_delete_and_hides_molecular_actions() {
        for kind in annotation_kinds() {
            assert_annotation_overflow(kind);
        }
    }

    #[test]
    fn unresolved_annotations_disable_focus_but_keep_management_actions() {
        for kind in annotation_kinds() {
            let mut scene = SceneModel::new();
            scene.entries.push(SceneEntry::Object(unresolved_annotation(
                "annotation",
                kind,
            )));
            let mut bridge = ObjectsBridge::new();
            bridge.selection_level = SelectionLevel::Objects;
            bridge.selected_objects.push("annotation".to_string());
            let capabilities = bridge
                .selected_capabilities(&scene)
                .expect("annotation capabilities");
            assert!(!capabilities.focus);
            assert!(capabilities.visibility);
            assert!(capabilities.color);
            assert!(capabilities.delete);
        }
    }

    #[test]
    fn label_icon_uses_annotation_key() {
        let object = scene_object("labels", SceneObjectKind::Label);
        assert_eq!(object_icon_kind(&object), "label");
    }

    fn action_names(items: &[OverflowMenuItem]) -> Vec<String> {
        items.iter().map(|item| item.action.to_string()).collect()
    }

    #[test]
    fn molecule_overflow_preserves_atomic_remove_and_representation_actions() {
        let mut scene = SceneModel::new();
        scene.entries.push(SceneEntry::Object(scene_object(
            "mol",
            SceneObjectKind::Molecule,
        )));
        let mut bridge = ObjectsBridge::new();
        bridge.selection_level = SelectionLevel::Objects;
        bridge.selected_objects.push("mol".to_string());

        let actions = action_names(&bridge.compute_overflow_menu(&scene));
        assert!(actions.iter().any(|action| action == "remove"));
        assert!(actions.iter().any(|action| action == "copy"));
        assert!(actions.iter().any(|action| action == "representation"));
        assert!(!actions.iter().any(|action| action == "delete"));
    }

    #[test]
    fn measurement_icon_keys_distinguish_all_three_kinds() {
        let mut object = scene_object("m", SceneObjectKind::Measurement);
        object.measurement_kind = Some(MeasurementKind::Distance);
        assert_eq!(object_icon_kind(&object), "measurement-distance");
        object.measurement_kind = Some(MeasurementKind::Angle);
        assert_eq!(object_icon_kind(&object), "measurement-angle");
        object.measurement_kind = Some(MeasurementKind::Dihedral);
        assert_eq!(object_icon_kind(&object), "measurement-dihedral");
    }

    #[test]
    fn measurement_color_popover_uses_only_solid_colors() {
        let mut scene = SceneModel::new();
        scene.entries.push(SceneEntry::Object(scene_object(
            "distance",
            SceneObjectKind::Measurement,
        )));
        scene.entries.push(SceneEntry::Object(scene_object(
            "molecule",
            SceneObjectKind::Molecule,
        )));

        assert!(uses_solid_color_popover(&scene, "distance"));
        assert!(!uses_solid_color_popover(&scene, "molecule"));
    }

    #[test]
    fn build_popover_target_empty_clause_is_obj_only() {
        assert_eq!(build_popover_target("1abc", ""), "1abc");
    }

    #[test]
    fn build_popover_target_with_clause() {
        assert_eq!(
            build_popover_target("1abc", "chain A and polymer"),
            "1abc and chain A and polymer"
        );
    }

    #[test]
    fn rep_summary_applies_object_draw_mask_to_atom_bits() {
        let mut atom = Atom::new("CA", Element::Carbon);
        atom.repr.visible_reps = RepMask::CARTOON.union(RepMask::STICKS);
        let mut mol_obj = MoleculeObject::from_raw(ObjectMolecule::new("mask"));
        mol_obj.set_visible_reps(RepMask::CARTOON.union(RepMask::STICKS));
        mol_obj.set_draw_reps(RepMask::STICKS);

        let mut summary = RepSummary::new();
        summary.accumulate(&mol_obj, &atom);

        assert!(!summary.mask.is_visible(RepMask::CARTOON));
        assert!(summary.mask.is_visible(RepMask::STICKS));
    }

    #[test]
    fn single_bio_subchain() {
        let subs = vec![key("1abc", "A", "", "biopolymer", "chain A and polymer")];
        assert_eq!(
            build_subchains_expr(&subs).unwrap(),
            "1abc and chain A and polymer"
        );
    }

    #[test]
    fn single_labeled_het_subchain() {
        let subs = vec![key(
            "1abc",
            "A",
            "HEM",
            "organic",
            "chain A and organic and resi 200",
        )];
        assert_eq!(
            build_subchains_expr(&subs).unwrap(),
            "1abc and chain A and organic and resi 200"
        );
    }

    #[test]
    fn single_subchain_in_chain() {
        // Multi-chain object where chain E is a single subchain — clause
        // is just "chain E", no kind qualifier.
        let subs = vec![key("1abc", "E", "NAG+8", "organic", "chain E")];
        assert_eq!(build_subchains_expr(&subs).unwrap(), "1abc and chain E");
    }

    #[test]
    fn single_subchain_object() {
        // Object containing a single subchain — empty clause covers the
        // whole object.
        let subs = vec![key("1abc", "A", "HEM", "organic", "")];
        assert_eq!(build_subchains_expr(&subs).unwrap(), "1abc");
    }

    #[test]
    fn composite_glycan_clause() {
        let subs = vec![key(
            "1abc",
            "E",
            "NAG+8",
            "organic",
            "chain E and organic and resi 1001-1009",
        )];
        assert_eq!(
            build_subchains_expr(&subs).unwrap(),
            "1abc and chain E and organic and resi 1001-1009"
        );
    }

    #[test]
    fn solvent_clause() {
        let subs = vec![key("1abc", "S", "HOH", "solvent", "chain S and solvent")];
        assert_eq!(
            build_subchains_expr(&subs).unwrap(),
            "1abc and chain S and solvent"
        );
    }

    #[test]
    fn inorganic_clause() {
        let subs = vec![key(
            "1abc",
            "A",
            "ZN+CL",
            "inorganic",
            "chain A and resn ZN+CL",
        )];
        assert_eq!(
            build_subchains_expr(&subs).unwrap(),
            "1abc and chain A and resn ZN+CL"
        );
    }

    #[test]
    fn two_bio_subchains_same_object() {
        let subs = vec![
            key("1abc", "A", "", "biopolymer", "chain A and polymer"),
            key("1abc", "B", "", "biopolymer", "chain B and polymer"),
        ];
        assert_eq!(
            build_subchains_expr(&subs).unwrap(),
            "1abc and (chain A and polymer or chain B and polymer)"
        );
    }

    #[test]
    fn same_chain_polymer_and_organic() {
        let subs = vec![
            key("1abc", "A", "", "biopolymer", "chain A and polymer"),
            key(
                "1abc",
                "A",
                "HEM",
                "organic",
                "chain A and organic and resi 200",
            ),
        ];
        assert_eq!(
            build_subchains_expr(&subs).unwrap(),
            "1abc and (chain A and polymer or chain A and organic and resi 200)"
        );
    }

    #[test]
    fn bio_and_het_same_chain() {
        // The old "bio subsumes het" optimisation is gone — both rows
        // produce a clean OR of typed clauses.
        let subs = vec![
            key("1abc", "A", "", "biopolymer", "chain A and polymer"),
            key(
                "1abc",
                "A",
                "HEM",
                "organic",
                "chain A and organic and resi 200",
            ),
        ];
        assert_eq!(
            build_subchains_expr(&subs).unwrap(),
            "1abc and (chain A and polymer or chain A and organic and resi 200)"
        );
    }

    #[test]
    fn cross_object() {
        let subs = vec![
            key("1abc", "A", "", "biopolymer", "chain A and polymer"),
            key("2def", "B", "", "biopolymer", "chain B and polymer"),
        ];
        assert_eq!(
            build_subchains_expr(&subs).unwrap(),
            "(1abc and chain A and polymer) or (2def and chain B and polymer)"
        );
    }

    #[test]
    fn two_objects_multi_subchain() {
        let subs = vec![
            key("1abc", "A", "", "biopolymer", "chain A and polymer"),
            key("1abc", "B", "", "biopolymer", "chain B and polymer"),
            key("2def", "C", "", "biopolymer", "chain C and polymer"),
        ];
        assert_eq!(
            build_subchains_expr(&subs).unwrap(),
            "(1abc and (chain A and polymer or chain B and polymer)) or (2def and chain C and polymer)"
        );
    }

    #[test]
    fn whole_object_subsumes_other_clauses() {
        // If one selected row covers the whole object (empty clause),
        // it absorbs any sibling sub-clauses for that object.
        let subs = vec![
            key("1abc", "A", "HEM", "organic", ""),
            key(
                "1abc",
                "A",
                "HEM",
                "organic",
                "chain A and organic and resi 200",
            ),
        ];
        assert_eq!(build_subchains_expr(&subs).unwrap(), "1abc");
    }

    #[test]
    fn empty_returns_none() {
        assert!(build_subchains_expr(&[]).is_none());
    }
}
