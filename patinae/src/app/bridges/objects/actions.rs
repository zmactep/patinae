//! Actions for ObjectsBridge.

use super::*;

impl ObjectsBridge {
    /// Truncate a name to `max` characters with ellipsis for use in pre-filled inputs.
    pub(super) fn truncate_name(name: &str, max: usize) -> String {
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

    pub(super) fn selected_capabilities(&self, scene: &SceneModel) -> Option<SelectedCapabilities> {
        let mut selected: Option<SelectedCapabilities> = None;
        let mut include = |capabilities| match &mut selected {
            Some(current) => current.intersect(capabilities),
            None => selected = Some(capabilities),
        };

        for group_name in &self.scene_selection.selected_groups {
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
        for object_name in &self.scene_selection.selected_objects {
            if let Some(object) = scene.get(object_name) {
                include(SelectedCapabilities::from_object(
                    object.capabilities,
                    object.kind,
                ));
            }
        }
        if !self.scene_selection.selected_subchains.is_empty()
            || !self.named_selection.selected_selections.is_empty()
        {
            include(SelectedCapabilities::molecular_selection());
        }

        selected
    }

    pub(super) fn compute_overflow_menu(&self, scene: &SceneModel) -> Vec<OverflowMenuItem> {
        let total = self.scene_selection.selected_groups.len()
            + self.scene_selection.selected_objects.len()
            + self.scene_selection.selected_subchains.len()
            + self.named_selection.selected_selections.len();
        let is_multi = total > 1;
        let Some(capabilities) = self.selected_capabilities(scene) else {
            return Vec::new();
        };

        let is_chain_or_sel = matches!(
            self.scene_selection.selection_level,
            SelectionLevel::Subchains
        ) || (self.scene_selection.selection_level == SelectionLevel::None
            && !self.named_selection.selected_selections.is_empty());

        let mut items = Vec::new();

        if self.scene_selection.selection_level == SelectionLevel::Objects
            && self.scene_selection.selected_objects.len() == 1
            && scene
                .get(&self.scene_selection.selected_objects[0])
                .is_some_and(|object| object.can_materialize)
        {
            items.push(OverflowMenuItem {
                action: "materialize".into(),
                label: "Materialize copies".into(),
                disabled: false,
            });
        }

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
}
pub(super) fn object_icon_kind(obj: &patinae_framework::model::scene::SceneObject) -> &'static str {
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

pub(super) fn measurement_kind_name(kind: Option<MeasurementKind>) -> &'static str {
    match kind {
        Some(MeasurementKind::Distance) => "distance",
        Some(MeasurementKind::Angle) => "angle",
        Some(MeasurementKind::Dihedral) => "dihedral",
        None => "",
    }
}

/// Build the selection expression for the popover target.
///
/// `selector_clause` is precomputed by the scene model from the typed
/// `SubchainKind`/`SubchainLabel` (see
/// `patinae_framework::model::scene::build_selector_clause`). An empty
/// clause means "the whole object".
pub(super) fn build_popover_target(obj_name: &str, selector_clause: &str) -> String {
    if selector_clause.is_empty() {
        obj_name.to_string()
    } else {
        format!("{} and {}", obj_name, selector_clause)
    }
}

/// Build the human-readable display label for the popover header.
pub(super) fn build_popover_label(obj_name: &str, chain_id: &str, subchain_label: &str) -> String {
    if chain_id.is_empty() {
        obj_name.to_string()
    } else if subchain_label.is_empty() {
        format!("{} · {}", obj_name, chain_id)
    } else {
        format!("{} · {} ({})", obj_name, chain_id, subchain_label)
    }
}

/// Compute rep/preset state for an object or subchain and apply to ObjectsState.
///
/// `entry_index` < 0 selects the whole object; otherwise the partition
/// entry identifies the exact subchain.
pub(super) fn set_active_reps(
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
pub(super) fn set_active_reps_for_selection(
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
pub(super) fn set_active_reps_for_multi(
    os: &ObjectsState,
    kernel: &AppKernel,
    objects: &ObjectsBridge,
) {
    let mut summary = RepSummary::new();

    // Groups → children objects' atoms
    for group_name in &objects.scene_selection.selected_groups {
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
    for obj_name in &objects.scene_selection.selected_objects {
        if let Some(mol_obj) = kernel.session.registry.get_molecule(obj_name) {
            for atom in mol_obj.molecule().atoms() {
                summary.accumulate(mol_obj, atom);
            }
        }
    }

    // Subchains → partition view atoms
    for key in &objects.scene_selection.selected_subchains {
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
    for sel_name in &objects.named_selection.selected_selections {
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
pub(super) fn open_pill_popover(os: &ObjectsState, kind: &str, target: &str) {
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

pub(super) fn uses_solid_color_popover(scene: &SceneModel, object_name: &str) -> bool {
    scene
        .get(object_name)
        .is_some_and(|object| object.capabilities.color && !object.capabilities.representations)
}

/// Open the name-input popup (rename / copy / extract).
pub(super) fn open_name_popup(os: &ObjectsState, action: &str, target: &str, short: &str) {
    let (suffix, title_label) = match action {
        "rename" => ("_renamed", "Rename"),
        "copy" => ("_copy", "Copy"),
        "extract" => ("_extract", "Extract from"),
        _ => return,
    };
    os.set_name_input_action(action.into());
    os.set_name_input_target(target.into());
    os.set_name_input_text(if action == "rename" {
        target.into()
    } else {
        format!("{}{}", short, suffix).into()
    });
    os.set_name_input_title(format!("{} {}", title_label, target).into());
    os.set_popover_kind(if action == "rename" { "" } else { "N" }.into());
}

pub(super) struct RepSummary {
    pub(super) mask: RepMask,
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
    pub(super) fn new() -> Self {
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

    pub(super) fn from_mask(mask: RepMask) -> Self {
        Self {
            mask,
            ..Self::new()
        }
    }

    pub(super) fn accumulate(&mut self, mol_obj: &MoleculeObject, atom: &patinae_mol::Atom) {
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

    pub(super) fn apply(&self, os: &ObjectsState, resolved: &patinae_settings::ResolvedSettings) {
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
