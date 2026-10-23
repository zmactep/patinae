use super::*;
use lin_alg::f32::Vec3;
use patinae_cmd::{LabelExpression, LabelTarget, MeasurementTarget};
use patinae_mol::{Atom, AtomBuilder, CoordSet, Element, ObjectMolecule, RepMask};
use patinae_scene::{
    canonical_atom_path_for_hit, LabelObject, MeasurementObject, ObjectType, PickHit, RecentAtoms,
    Session,
};
use patinae_settings::groups::RecentPickLimit;
use std::collections::HashMap;

fn color_popover_kernel() -> AppKernel {
    let mut molecule = ObjectMolecule::new("mol");
    for (chain, residue) in [("A", 1), ("A", 2), ("B", 1), ("B", 2)] {
        molecule.add_atom(
            AtomBuilder::new()
                .name("CA")
                .element(Element::Carbon)
                .resn("GLY")
                .resv(residue)
                .chain(chain)
                .build(),
        );
    }
    molecule.add_coord_set(CoordSet::from_vec3(&[Vec3::new(0.0, 0.0, 0.0); 4]));
    let mut kernel = AppKernel::new();
    kernel
        .session
        .registry
        .add(MoleculeObject::with_name(molecule.clone(), "mol"));
    kernel
        .session
        .registry
        .add(MoleculeObject::with_name(molecule, "other"));
    kernel
}

fn assigned_colors(kernel: &mut AppKernel, target: &str) -> RepMask {
    kernel.mutate_viewer(None, (0, 0), |viewer| color_assignments(viewer, target))
}

fn color_command(kernel: &mut AppKernel, command: &str) {
    kernel
        .execute_command(command, false, None, (800, 600))
        .unwrap();
}

#[test]
fn color_popover_resets_each_representation_in_the_requested_scope() {
    for (setting, rep) in [
        ("line_color", RepMask::LINES),
        ("stick_color", RepMask::STICKS),
        ("sphere_color", RepMask::SPHERES),
        ("cartoon_color", RepMask::CARTOON),
        ("ribbon_color", RepMask::RIBBON),
        ("surface_color", RepMask::SURFACE),
        ("mesh_color", RepMask::MESH),
        ("dot_color", RepMask::DOTS),
    ] {
        let mut kernel = color_popover_kernel();
        assert_eq!(assigned_colors(&mut kernel, "mol"), RepMask::NONE);
        // A global representation color is inherited, not a local assignment.
        color_command(&mut kernel, &format!("set {setting}, red"));
        color_command(&mut kernel, "color red, mol");
        assert_eq!(assigned_colors(&mut kernel, "mol"), RepMask::NONE);
        color_command(&mut kernel, &format!("set {setting}, red, mol"));
        assert_eq!(assigned_colors(&mut kernel, "mol"), rep, "{setting}");
        assert_eq!(assigned_colors(&mut kernel, "mol and chain B"), rep);
        assert_eq!(assigned_colors(&mut kernel, "other"), RepMask::NONE);
        color_command(
            &mut kernel,
            &format!("set {setting}, blue, mol and chain A"),
        );
        color_command(&mut kernel, "set ellipsoid_color, green, mol");
        color_command(&mut kernel, &format!("unset {setting}, mol and chain A"));
        assert_eq!(
            assigned_colors(&mut kernel, "mol and chain A"),
            RepMask::NONE
        );
        assert_eq!(assigned_colors(&mut kernel, "mol and chain B"), rep);
        assert_eq!(assigned_colors(&mut kernel, "mol"), rep);
        let other_colors = kernel
            .session
            .registry
            .get_molecule("mol")
            .unwrap()
            .molecule()
            .atoms()
            .map(|atom| (atom.repr.colors.base, atom.repr.colors.ellipsoid))
            .collect::<Vec<_>>();
        color_command(&mut kernel, &format!("unset {setting}, mol"));
        assert_eq!(assigned_colors(&mut kernel, "mol"), RepMask::NONE);
        assert_eq!(
            kernel
                .session
                .registry
                .get_molecule("mol")
                .unwrap()
                .molecule()
                .atoms()
                .map(|atom| (atom.repr.colors.base, atom.repr.colors.ellipsoid))
                .collect::<Vec<_>>(),
            other_colors,
        );
    }
}

#[test]
fn color_popover_finds_schemes_partial_colors_and_named_or_combined_targets() {
    let mut kernel = color_popover_kernel();
    color_command(
        &mut kernel,
        "set cartoon_color, ss, mol and chain A and resi 1",
    );
    color_command(
        &mut kernel,
        "set stick_color, blue, mol and chain A and resi 2",
    );
    color_command(
        &mut kernel,
        "set cartoon_color, green, mol and chain A and resi 2",
    );
    color_command(&mut kernel, "select chosen, mol and chain A");
    let expected = RepMask::CARTOON.union(RepMask::STICKS);
    assert_eq!(assigned_colors(&mut kernel, "chosen"), expected);
    assert_eq!(
        assigned_colors(&mut kernel, "mol and chain B"),
        RepMask::NONE
    );
    assert_eq!(
        assigned_colors(&mut kernel, "(chosen) or (other)"),
        expected
    );
    assert_eq!(
        assigned_colors(&mut kernel, "mol and resi 99"),
        RepMask::NONE
    );
    assert_eq!(assigned_colors(&mut kernel, "missing"), RepMask::NONE);
    color_command(&mut kernel, "unset cartoon_color, chosen");
    assert_eq!(assigned_colors(&mut kernel, "chosen"), RepMask::STICKS);
    color_command(&mut kernel, "select chosen, mol and chain B");
    assert_eq!(assigned_colors(&mut kernel, "chosen"), RepMask::NONE);
}

#[test]
fn color_popover_sync_refreshes_after_same_rgb_assignments_and_selection_changes() {
    i_slint_backend_testing::init_no_event_loop();
    let window = AppWindow::new().unwrap();
    let os = window.global::<ObjectsState>();
    let mut kernel = color_popover_kernel();
    let mut bridge = ObjectsBridge::new();
    bridge.attach(&window);
    color_command(&mut kernel, "color red, mol");
    color_command(&mut kernel, "select chosen, mol and chain A");
    os.set_popover_kind("C".into());
    os.set_popover_target("chosen".into());
    os.set_popover_scope("all-c".into());
    bridge.sync(&mut kernel, &window);
    assert!(!os.get_popover_color_cartoon());
    kernel.session.registry.clear_all_dirty_molecules();
    // The visible color stays red; the new stored assignment still needs a cross.
    color_command(&mut kernel, "set cartoon_color, red, chosen");
    bridge.sync(&mut kernel, &window);
    assert!(os.get_popover_color_cartoon());
    assert_eq!(os.get_popover_scope(), "all-c");
    kernel.session.registry.clear_all_dirty_molecules();
    color_command(&mut kernel, "select chosen, mol and chain B");
    bridge.sync(&mut kernel, &window);
    assert!(!os.get_popover_color_cartoon());
    os.set_popover_target("mol".into());
    sync_color_assignments(&os, &mut kernel);
    assert!(os.get_popover_color_cartoon());
    os.set_popover_target("other".into());
    sync_color_assignments(&os, &mut kernel);
    assert!(!os.get_popover_color_cartoon());
    os.set_popover_target("mol".into());
    sync_color_assignments(&os, &mut kernel);
    color_command(&mut kernel, "unset cartoon_color, mol");
    bridge.sync(&mut kernel, &window);
    assert!(!os.get_popover_color_cartoon());
    os.set_popover_color_cartoon(true);
    os.set_popover_solid_only(true);
    sync_color_assignments(&os, &mut kernel);
    assert!(!os.get_popover_color_cartoon());
}

fn recent_atoms(paths: &[&str]) -> RecentAtoms {
    let mut recent = RecentAtoms::new();
    for path in paths {
        recent.insert(*path, RecentPickLimit::Unlimited);
    }
    recent
}

fn recent_ids(recent: &RecentAtoms) -> Vec<RecentAtomId> {
    recent.rows().iter().map(|row| row.id()).collect()
}

fn recent_model_rows(bridge: &ObjectsBridge) -> Vec<RecentAtomItem> {
    (0..bridge.recent_atoms_model.row_count())
        .filter_map(|index| bridge.recent_atoms_model.row_data(index))
        .collect()
}

fn single_atom_session(object_name: &str) -> (Session, String) {
    let mut molecule = ObjectMolecule::new(object_name);
    molecule.add_atom(
        AtomBuilder::new()
            .name("CA")
            .element(Element::Carbon)
            .resn("GLY")
            .resv(1)
            .chain("A")
            .build(),
    );
    molecule.add_coord_set(CoordSet::from_vec3(&[Vec3::new(0.0, 0.0, 0.0)]));
    let mut session = Session::new();
    session
        .registry
        .add(MoleculeObject::with_name(molecule, object_name));
    let path = canonical_atom_path_for_hit(
        &PickHit {
            instance: None,
            object_name: object_name.to_string(),
            object_type: ObjectType::Molecule,
            atom_index: Some(patinae_mol::AtomIndex(0)),
            position: Vec3::new(0.0, 0.0, 0.0),
            distance: 0.0,
        },
        session
            .registry
            .get_molecule(object_name)
            .expect("test molecule")
            .molecule(),
    )
    .expect("canonical atom path");
    session
        .recent_atoms
        .insert(path.clone(), RecentPickLimit::Unlimited);
    (session, path)
}

#[test]
fn recent_rows_refresh_on_generation_and_preserve_operand_on_rewrite() {
    let (mut session, _) = single_atom_session("old");
    let ids = recent_ids(&session.recent_atoms);
    let mut bridge = ObjectsBridge::new();
    assert!(!bridge.sync_recent_atoms(&session.recent_atoms));
    let first_key = recent_model_rows(&bridge)[0].key.clone();

    bridge.click_recent_atom(ids[0], false, false);
    session.rename_object("old", "renamed").unwrap();

    assert!(!bridge.sync_recent_atoms(&session.recent_atoms));
    assert_eq!(bridge.recent.recent_operands, [ids[0]]);
    let rows = recent_model_rows(&bridge);
    assert_eq!(rows[0].key, first_key);
    assert!(rows[0].path.starts_with("/renamed/"));
    assert_eq!(rows[0].operand_position, 1);
    assert!(rows[0].selected);
}

#[test]
fn recent_model_reconciles_large_history_without_rekeying_survivors() {
    let mut recent = RecentAtoms::new();
    for index in 0..2_000 {
        recent.insert(
            format!("/object/chain/residue/{index}/atom"),
            RecentPickLimit::Unlimited,
        );
    }
    let mut bridge = ObjectsBridge::new();
    bridge.sync_recent_atoms(&recent);
    let original_keys = bridge
        .recent
        .recent_bindings
        .iter()
        .map(|binding| (binding.id, binding.key.clone()))
        .collect::<HashMap<_, _>>();

    recent.insert("/object/chain/residue/new/atom", RecentPickLimit::Unlimited);
    bridge.sync_recent_atoms(&recent);
    let removed_path = recent.rows()[10].path().to_string();
    let removed_id = recent.row_id(&removed_path).unwrap();
    recent.remove_path(&removed_path);
    bridge.sync_recent_atoms(&recent);

    assert_eq!(bridge.recent_atoms_model.row_count(), 2_000);
    assert!(bridge.recent.recent_bindings.iter().all(|binding| {
        original_keys
            .get(&binding.id)
            .is_none_or(|key| key == &binding.key)
    }));
    assert!(bridge
        .recent
        .recent_bindings
        .iter()
        .all(|binding| binding.id != removed_id));
}

#[test]
fn literal_labels_require_trimmed_nonempty_text() {
    assert!(label_expression_from_key("literal", " \n\t ").is_none());
    assert_eq!(
        label_expression_from_key("literal", "  active site  "),
        Some(LabelExpression::Literal("active site".to_string()))
    );
}

#[test]
fn label_target_picker_is_invalidated_only_when_live_names_change() {
    let shown = annotation_target_model(vec!["labels-a".to_string(), "labels-b".to_string()]);

    assert!(!label_popover_targets_invalidated(
        &shown,
        &["labels-b".to_string(), "labels-a".to_string()]
    ));
    assert!(label_popover_targets_invalidated(
        &shown,
        &["labels-a".to_string()]
    ));
    assert!(label_popover_targets_invalidated(
        &shown,
        &["labels-a".to_string(), "labels-c".to_string()]
    ));
}

#[test]
fn recent_atom_display_path_hides_canonical_blank_sentinels() {
    assert_eq!(
        display_atom_path(r#"/1fsd/""/A/LYS`"16 "/HZ2`" ""#),
        "/1fsd//A/LYS`16/HZ2"
    );
    assert_eq!(
        display_atom_path(r#"/ordinary/""/A/GLY`"42 "/CA`" ""#),
        "/ordinary//A/GLY`42/CA"
    );
    assert_eq!(
        display_atom_path(r#"/ordinary/""/""/GLY`42/CA`"""#),
        "/ordinary///GLY`42/CA"
    );
    assert_eq!(
        display_atom_path(r#"/"model/\"quoted\""/"*"/A/GLY`42/CA`" ""#),
        r#"/"model/\"quoted\""/"*"/A/GLY`42/CA"#
    );
    assert_eq!(
        display_atom_path(r#"/ordinary/""/A/GLY`"42A"/CA`B"#),
        "/ordinary//A/GLY`42A/CA`B"
    );
}

#[test]
fn shift_click_with_stale_anchor_starts_a_new_range() {
    let mut selected = vec!["missing"];
    let mut anchor = Some("missing");

    handle_click(
        &mut selected,
        "second",
        "second",
        |key| ["first", "second"].contains(key).then_some(*key),
        &["first", "second"],
        true,
        false,
        &mut anchor,
    );

    assert_eq!(selected, ["second"]);
    assert_eq!(anchor, Some("second"));
}

#[test]
fn recent_clicks_use_unique_meta_and_range_selection_semantics() {
    let recent = recent_atoms(&["/1", "/2", "/3"]);
    let ids = recent_ids(&recent);
    let mut bridge = ObjectsBridge::new();
    bridge.sync_recent_atoms(&recent);

    bridge.click_recent_atom(ids[0], false, false);
    bridge.click_recent_atom(ids[1], false, false);
    assert_eq!(bridge.recent.recent_operands, [ids[1]]);

    bridge.click_recent_atom(ids[0], false, true);
    assert_eq!(bridge.recent.recent_operands, [ids[1], ids[0]]);

    bridge.click_recent_atom(ids[2], true, false);
    assert_eq!(bridge.recent.recent_operands, ids);

    bridge.clear_recent_operands();
    assert!(bridge.recent.recent_operands.is_empty());
    assert!(bridge.recent.recent_anchor.is_none());
    assert!(recent_model_rows(&bridge)
        .iter()
        .all(|row| !row.selected && row.operand_position == 0));
}

#[test]
fn panel_background_clicks_clear_only_their_selection_domain() {
    let recent = recent_atoms(&["/1"]);
    let ids = recent_ids(&recent);
    let mut bridge = ObjectsBridge::new();
    bridge.sync_recent_atoms(&recent);
    bridge.scene_selection.selection_level = SelectionLevel::Objects;
    bridge
        .scene_selection
        .selected_objects
        .push("object".to_string());
    bridge
        .named_selection
        .selected_selections
        .push("selection".to_string());
    bridge.click_recent_atom(ids[0], false, false);

    bridge.clear_scene_selection();
    assert_eq!(bridge.scene_selection.selection_level, SelectionLevel::None);
    assert!(bridge.scene_selection.selected_objects.is_empty());
    assert_eq!(bridge.named_selection.selected_selections, ["selection"]);
    assert_eq!(bridge.recent.recent_operands, ids);

    bridge.clear_named_selection();
    assert!(bridge.named_selection.selected_selections.is_empty());
    assert_eq!(bridge.recent.recent_operands, ids);

    bridge.clear_recent_operands();
    assert!(bridge.recent.recent_operands.is_empty());
}

#[test]
fn recent_clicks_enforce_four_operand_limit_for_meta_and_shift_ranges() {
    let recent = recent_atoms(&["/1", "/2", "/3", "/4", "/5", "/6"]);
    let ids = recent_ids(&recent);
    let mut bridge = ObjectsBridge::new();
    bridge.sync_recent_atoms(&recent);

    assert!(bridge.click_recent_atom(ids[0], false, false));
    assert_eq!(bridge.recent.recent_operands, [ids[0]]);
    assert!(!bridge.click_recent_atom(ids[0], false, false));
    assert_eq!(bridge.recent.recent_operands, [ids[0]]);
    assert!(bridge.clear_recent_operands());
    assert!(!bridge.clear_recent_operands());

    for id in &ids[..5] {
        bridge.click_recent_atom(*id, false, true);
    }
    assert_eq!(bridge.recent.recent_operands, ids[1..5]);
    bridge.click_recent_atom(ids[2], false, true);
    assert_eq!(bridge.recent.recent_operands, [ids[1], ids[3], ids[4]]);

    bridge.click_recent_atom(ids[0], false, false);
    bridge.click_recent_atom(ids[5], true, false);
    assert_eq!(bridge.recent.recent_operands, ids[2..6]);
    bridge.click_recent_atom(ids[4], true, false);
    assert_eq!(bridge.recent.recent_operands, ids[1..5]);
}

#[test]
fn large_recent_shift_range_keeps_only_the_last_four_visible_ids() {
    let mut recent = RecentAtoms::new();
    for index in 0..10_000 {
        recent.insert(format!("/{index}"), RecentPickLimit::Unlimited);
    }
    let ids = recent_ids(&recent);
    let mut bridge = ObjectsBridge::new();
    bridge.sync_recent_atoms(&recent);

    bridge.click_recent_atom(ids[0], false, false);
    bridge.click_recent_atom(ids[9_999], true, false);

    assert_eq!(bridge.recent.recent_operands, ids[9_996..]);
}

#[test]
fn recent_mutations_prune_and_compact_operands_without_touching_general_selection() {
    let mut recent = recent_atoms(&["/1", "/2", "/3", "/4", "/5"]);
    let ids = recent_ids(&recent);
    let mut bridge = ObjectsBridge::new();
    bridge
        .scene_selection
        .selected_objects
        .push("object".to_string());
    bridge
        .named_selection
        .selected_selections
        .push("named".to_string());
    bridge.sync_recent_atoms(&recent);
    for id in &ids[..4] {
        bridge.click_recent_atom(*id, false, true);
    }

    recent.remove_path("/2");
    assert!(bridge.sync_recent_atoms(&recent));
    assert_eq!(bridge.recent.recent_operands, [ids[0], ids[2], ids[3]]);
    assert_eq!(
        recent_model_rows(&bridge)
            .iter()
            .map(|row| row.operand_position)
            .collect::<Vec<_>>(),
        [1, 2, 3, 0]
    );

    recent.remove_path("/3");
    bridge.sync_recent_atoms(&recent);
    recent.insert("/6", RecentPickLimit::Bounded(1));
    bridge.sync_recent_atoms(&recent);
    assert!(bridge.recent.recent_operands.is_empty());
    assert_eq!(bridge.scene_selection.selected_objects, ["object"]);
    assert_eq!(bridge.named_selection.selected_selections, ["named"]);

    recent.clear();
    bridge.sync_recent_atoms(&recent);
    assert!(recent_model_rows(&bridge).is_empty());
}

#[test]
fn recent_row_key_resolves_its_durable_path_without_changing_operands() {
    let recent = recent_atoms(&["/1", "/2", "/3"]);
    let ids = recent_ids(&recent);
    let mut bridge = ObjectsBridge::new();
    bridge.sync_recent_atoms(&recent);
    bridge.click_recent_atom(ids[0], false, false);
    let second_key = bridge.recent.recent_bindings[1].key.clone();

    assert_eq!(bridge.recent.recent_operands, [ids[0]]);
    assert_eq!(bridge.recent_path_for_key(&second_key), Some("/2"));
}

#[test]
fn replacement_clears_operands_when_generation_and_row_ids_collide() {
    let (mut current, _) = single_atom_session("old");
    let old_id = current.recent_atoms.rows()[0].id();
    let mut bridge = ObjectsBridge::new();
    bridge.sync_recent_atoms(&current.recent_atoms);
    bridge.click_recent_atom(old_id, false, false);

    let (replacement, replacement_path) = single_atom_session("new");
    assert_eq!(replacement.recent_atoms.rows()[0].id(), old_id);
    assert_eq!(
        replacement.recent_atoms.generation(),
        current.recent_atoms.generation()
    );
    current.replace_contents(replacement);

    assert!(bridge.sync_recent_atoms(&current.recent_atoms));
    assert!(bridge.recent.recent_operands.is_empty());
    assert_eq!(
        current.recent_atoms.paths().collect::<Vec<_>>(),
        [replacement_path]
    );
    assert_eq!(recent_model_rows(&bridge)[0].operand_position, 0);
}

#[test]
fn recent_annotation_requests_use_only_numbered_operands_in_display_order() {
    let recent = recent_atoms(&["/1", "/2", "/3", "/4"]);
    let ids = recent_ids(&recent);
    let mut bridge = ObjectsBridge::new();
    bridge
        .scene_selection
        .selected_objects
        .push("ordinary-object".to_string());
    bridge
        .named_selection
        .selected_selections
        .push("ordinary-selection".to_string());
    bridge.sync_recent_atoms(&recent);
    for id in [ids[2], ids[0], ids[3]] {
        bridge.click_recent_atom(id, false, true);
    }

    let measurement = bridge
        .measurement_request(MeasurementTarget::New)
        .expect("three selected operands");
    assert_eq!(measurement.operands, ["/3", "/1", "/4"]);
    assert_eq!(measurement.inferred_kind().unwrap(), MeasurementKind::Angle);
    let label = bridge
        .label_request(LabelExpression::Name, LabelTarget::New)
        .expect("three selected operands");
    assert_eq!(label.operands, ["/3", "/1", "/4"]);
    assert_eq!(bridge.recent.recent_operands, [ids[2], ids[0], ids[3]]);
    assert_eq!(bridge.scene_selection.selected_objects, ["ordinary-object"]);
    assert_eq!(
        bridge.named_selection.selected_selections,
        ["ordinary-selection"]
    );
}

#[test]
fn recent_annotation_target_models_filter_measurement_kind_and_labels() {
    let recent = recent_atoms(&["/1", "/2", "/3"]);
    let ids = recent_ids(&recent);
    let mut bridge = ObjectsBridge::new();
    bridge.sync_recent_atoms(&recent);
    for id in ids {
        bridge.click_recent_atom(id, false, true);
    }
    let mut registry = ObjectRegistry::new();
    registry.add(MeasurementObject::new(
        "distance",
        MeasurementKind::Distance,
    ));
    registry.add(MeasurementObject::new("angle", MeasurementKind::Angle));
    registry.add(MeasurementObject::new(
        "dihedral",
        MeasurementKind::Dihedral,
    ));
    registry.add(LabelObject::new("labels"));

    assert_eq!(bridge.compatible_measurement_targets(&registry), ["angle"]);
    assert_eq!(bridge.label_targets(&registry), ["labels"]);

    bridge.recent.recent_operands.pop();
    assert_eq!(
        bridge.compatible_measurement_targets(&registry),
        ["distance"]
    );
}

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

fn scene_object(name: &str, kind: SceneObjectKind) -> patinae_framework::model::scene::SceneObject {
    patinae_framework::model::scene::SceneObject {
        storage_mode: patinae_mol::StorageMode::Explicit,
        stored_atom_count: 0,
        displayed_atom_count: 0,
        can_materialize: false,
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
    bridge.scene_selection.selection_level = SelectionLevel::Objects;
    bridge
        .scene_selection
        .selected_objects
        .push("annotation".to_string());

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
        bridge.scene_selection.selection_level = SelectionLevel::Objects;
        bridge
            .scene_selection
            .selected_objects
            .push("annotation".to_string());
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
    bridge.scene_selection.selection_level = SelectionLevel::Objects;
    bridge
        .scene_selection
        .selected_objects
        .push("mol".to_string());

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
    // Neither row subsumes the other: preserve both typed clauses.
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
