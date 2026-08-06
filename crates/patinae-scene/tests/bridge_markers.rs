use lin_alg::f32::Vec3;
use patinae_algos::surface::Grid3D;
use patinae_color::{NamedPalette, ThemedPalette};
use patinae_mol::{AtomBuilder, AtomIndex, CoordSet, DirtyFlags, MoleculeBuilder, ObjectMolecule};
use patinae_render::{MarkerUpdate, ObjectId, PickHit as RenderPickHit, RepKind};
use patinae_scene::{
    bridge::{
        resolve_pick, visit_render_objects, visit_render_scene, ResolvedSceneColors,
        ResolvedSceneMarkers, MARKER_RECENT,
    },
    HoverTarget, MapDisplayMode, MapObject, MoleculeObject, ObjectRegistry, SelectionManager,
    Session,
};
use patinae_select::SelectionResult;
use patinae_settings::Settings;
use std::cell::RefCell;

fn registry_with_two_atoms() -> ObjectRegistry {
    let mol = MoleculeBuilder::new("obj")
        .add_atom(
            AtomBuilder::new().name("A").element_symbol("C").build(),
            Vec3::new(0.0, 0.0, 0.0),
        )
        .add_atom(
            AtomBuilder::new().name("B").element_symbol("C").build(),
            Vec3::new(1.0, 0.0, 0.0),
        )
        .build();
    let mut registry = ObjectRegistry::new();
    registry.add(MoleculeObject::with_name(mol, "obj"));
    registry
}

fn registry_with_display_state(state: usize) -> ObjectRegistry {
    let mut mol = ObjectMolecule::new("obj");
    mol.add_atom(AtomBuilder::new().name("A").element_symbol("C").build());
    mol.add_coord_set(CoordSet::from_vec3(&[Vec3::new(0.0, 0.0, 0.0)]));
    mol.add_coord_set(CoordSet::from_vec3(&[Vec3::new(7.0, 0.0, 0.0)]));

    let mut obj = MoleculeObject::with_name(mol, "obj");
    assert!(obj.set_display_state(state));

    let mut registry = ObjectRegistry::new();
    registry.add(obj);
    registry
}

fn molecule_object(name: &str) -> MoleculeObject {
    let mol = MoleculeBuilder::new(name)
        .add_atom(
            AtomBuilder::new().name("A").element_symbol("C").build(),
            Vec3::new(0.0, 0.0, 0.0),
        )
        .build();
    MoleculeObject::with_name(mol, name)
}

fn collect_render_object_ids(registry: &ObjectRegistry) -> Vec<(String, u32)> {
    let settings = Settings::default();
    let colors = ResolvedSceneColors::build(
        registry,
        &settings,
        &NamedPalette::new(),
        &ThemedPalette::dark(),
    );
    let markers = ResolvedSceneMarkers::default();
    let mut ids = Vec::new();

    visit_render_scene(
        registry,
        &settings,
        &colors,
        &markers,
        &mut |name, obj| ids.push((name.to_string(), obj.object_id.0)),
        &mut |_name, _map| {},
    );

    ids
}

#[test]
fn marker_dirty_tracks_content_changes() {
    let registry = registry_with_two_atoms();
    let mut selections = SelectionManager::new();
    let mut markers = ResolvedSceneMarkers::default();

    markers.rebuild(&mut selections, &registry, None);
    assert!(markers.is_dirty("obj"));

    markers.rebuild(&mut selections, &registry, None);
    assert!(!markers.is_dirty("obj"));

    let selected = SelectionResult::from_indices(2, std::iter::once(AtomIndex(1)));
    selections.define_with_results("sel", "name B", vec![("obj".to_string(), selected)]);
    markers.rebuild(&mut selections, &registry, None);

    assert!(markers.is_dirty("obj"));
    assert!(markers.dirty_flags("obj").contains(DirtyFlags::SELECTION));
    assert!(!markers.dirty_flags("obj").contains(DirtyFlags::HOVER));
    assert_eq!(markers.get("obj"), Some([0, 1].as_slice()));
    assert_eq!(
        markers.updates("obj"),
        Some(
            [MarkerUpdate {
                atom_index: 1,
                bits: 1
            }]
            .as_slice()
        )
    );
}

#[test]
fn recent_atom_changes_emit_sparse_marker_updates() {
    let registry = registry_with_two_atoms();
    let mut selections = SelectionManager::new();
    let mut markers = ResolvedSceneMarkers::default();

    markers.rebuild_with_recent(&mut selections, &registry, &[], (0, 0), None);
    markers.rebuild_with_recent(
        &mut selections,
        &registry,
        &[("obj".to_string(), AtomIndex(1))],
        (0, 1),
        None,
    );

    assert_eq!(markers.get("obj"), Some([0, MARKER_RECENT].as_slice()));
    assert!(!markers.has_markers("obj"));
    assert!(markers.dirty_flags("obj").contains(DirtyFlags::SELECTION));
    assert_eq!(
        markers.updates("obj"),
        Some(
            [MarkerUpdate {
                atom_index: 1,
                bits: MARKER_RECENT,
            }]
            .as_slice()
        )
    );

    markers.rebuild_with_recent(
        &mut selections,
        &registry,
        &[
            ("obj".to_string(), AtomIndex(1)),
            ("obj".to_string(), AtomIndex(0)),
        ],
        (0, 2),
        None,
    );
    assert_eq!(markers.get("obj"), Some([MARKER_RECENT; 2].as_slice()));
    assert_eq!(
        markers.updates("obj"),
        Some(
            [MarkerUpdate {
                atom_index: 0,
                bits: MARKER_RECENT,
            }]
            .as_slice()
        )
    );

    markers.rebuild_with_recent(&mut selections, &registry, &[], (0, 3), None);
    assert_eq!(markers.get("obj"), Some([0, 0].as_slice()));
    assert_eq!(
        markers.updates("obj"),
        Some(
            [
                MarkerUpdate {
                    atom_index: 1,
                    bits: 0,
                },
                MarkerUpdate {
                    atom_index: 0,
                    bits: 0,
                },
            ]
            .as_slice()
        )
    );
}

#[test]
fn recent_updates_preserve_selected_and_hover_bits() {
    const SELECTED: u32 = 1 << 0;
    const HOVER: u32 = 1 << 1;

    let registry = registry_with_two_atoms();
    let mut selections = SelectionManager::new();
    let selected = SelectionResult::from_indices(2, std::iter::once(AtomIndex(1)));
    selections.define_with_results("sel", "name B", vec![("obj".to_string(), selected)]);
    let hover = HoverTarget {
        object: "obj".to_string(),
        selection: SelectionResult::from_indices(2, std::iter::once(AtomIndex(1))),
    };
    let mut markers = ResolvedSceneMarkers::default();

    markers.rebuild_with_recent(&mut selections, &registry, &[], (0, 0), Some(&hover));
    markers.rebuild_with_recent(
        &mut selections,
        &registry,
        &[("obj".to_string(), AtomIndex(1))],
        (0, 1),
        Some(&hover),
    );
    assert_eq!(
        markers.updates("obj"),
        Some(
            [MarkerUpdate {
                atom_index: 1,
                bits: SELECTED | HOVER | MARKER_RECENT,
            }]
            .as_slice()
        )
    );

    markers.rebuild_with_recent(&mut selections, &registry, &[], (0, 2), Some(&hover));
    assert_eq!(
        markers.updates("obj"),
        Some(
            [MarkerUpdate {
                atom_index: 1,
                bits: SELECTED | HOVER,
            }]
            .as_slice()
        )
    );
    assert!(markers.has_markers("obj"));
}

#[test]
fn selection_only_rebuild_emits_sparse_marker_updates() {
    let registry = registry_with_two_atoms();
    let mut selections = SelectionManager::new();
    let mut markers = ResolvedSceneMarkers::default();

    markers.rebuild(&mut selections, &registry, None);

    let selected_b = SelectionResult::from_indices(2, std::iter::once(AtomIndex(1)));
    selections.define_with_results("sel", "name B", vec![("obj".to_string(), selected_b)]);
    markers.rebuild(&mut selections, &registry, None);
    assert_eq!(markers.get("obj"), Some([0, 1].as_slice()));
    assert_eq!(
        markers.updates("obj"),
        Some(
            [MarkerUpdate {
                atom_index: 1,
                bits: 1
            }]
            .as_slice()
        )
    );

    let selected_a = SelectionResult::from_indices(2, std::iter::once(AtomIndex(0)));
    selections.define_with_results("sel", "name A", vec![("obj".to_string(), selected_a)]);
    markers.rebuild(&mut selections, &registry, None);
    assert_eq!(markers.get("obj"), Some([1, 0].as_slice()));
    assert_eq!(
        markers.updates("obj"),
        Some(
            [
                MarkerUpdate {
                    atom_index: 1,
                    bits: 0
                },
                MarkerUpdate {
                    atom_index: 0,
                    bits: 1
                },
            ]
            .as_slice()
        )
    );

    assert!(selections.remove("sel"));
    markers.rebuild(&mut selections, &registry, None);
    assert_eq!(markers.get("obj"), Some([0, 0].as_slice()));
    assert_eq!(
        markers.updates("obj"),
        Some(
            [MarkerUpdate {
                atom_index: 0,
                bits: 0
            }]
            .as_slice()
        )
    );
    assert!(!markers.has_markers("obj"));
}

#[test]
fn hover_only_rebuild_emits_sparse_marker_updates() {
    let registry = registry_with_two_atoms();
    let mut selections = SelectionManager::new();
    let mut markers = ResolvedSceneMarkers::default();

    markers.rebuild(&mut selections, &registry, None);
    assert!(markers.updates("obj").is_none());

    let hover_a = HoverTarget {
        object: "obj".to_string(),
        selection: SelectionResult::from_indices(2, std::iter::once(AtomIndex(0))),
    };
    markers.rebuild(&mut selections, &registry, Some(&hover_a));
    assert!(markers.dirty_flags("obj").contains(DirtyFlags::HOVER));
    assert!(!markers.dirty_flags("obj").contains(DirtyFlags::SELECTION));
    assert_eq!(markers.get("obj"), Some([2, 0].as_slice()));
    assert_eq!(
        markers.updates("obj"),
        Some(
            [MarkerUpdate {
                atom_index: 0,
                bits: 2
            }]
            .as_slice()
        )
    );
    assert!(markers.has_markers("obj"));

    let hover_b = HoverTarget {
        object: "obj".to_string(),
        selection: SelectionResult::from_indices(2, std::iter::once(AtomIndex(1))),
    };
    markers.rebuild(&mut selections, &registry, Some(&hover_b));
    assert_eq!(markers.get("obj"), Some([0, 2].as_slice()));
    assert_eq!(
        markers.updates("obj"),
        Some(
            [
                MarkerUpdate {
                    atom_index: 0,
                    bits: 0
                },
                MarkerUpdate {
                    atom_index: 1,
                    bits: 2
                },
            ]
            .as_slice()
        )
    );
}

#[test]
fn visit_render_objects_marks_marker_only_dirty() {
    let mut registry = registry_with_two_atoms();
    registry.clear_all_dirty_molecules();

    let settings = Settings::default();
    let colors = ResolvedSceneColors::build(
        &registry,
        &settings,
        &NamedPalette::new(),
        &ThemedPalette::dark(),
    );
    let mut selections = SelectionManager::new();
    let mut markers = ResolvedSceneMarkers::default();
    markers.rebuild(&mut selections, &registry, None);

    let selected = SelectionResult::from_indices(2, std::iter::once(AtomIndex(0)));
    selections.define_with_results("sel", "name A", vec![("obj".to_string(), selected)]);
    markers.rebuild(&mut selections, &registry, None);

    let mut dirty = DirtyFlags::empty();
    visit_render_objects(
        &registry,
        &settings,
        &colors,
        &markers,
        &mut |_name, obj| {
            dirty = obj.dirty;
        },
    );

    assert!(dirty.contains(DirtyFlags::SELECTION));
    assert!(!dirty.contains(DirtyFlags::HOVER));
    assert!(dirty.is_lut_only());
}

#[test]
fn visit_render_objects_marks_hover_only_dirty_precisely() {
    let mut registry = registry_with_two_atoms();
    registry.clear_all_dirty_molecules();

    let settings = Settings::default();
    let colors = ResolvedSceneColors::build(
        &registry,
        &settings,
        &NamedPalette::new(),
        &ThemedPalette::dark(),
    );
    let mut selections = SelectionManager::new();
    let hover = HoverTarget {
        object: "obj".to_string(),
        selection: SelectionResult::from_indices(2, std::iter::once(AtomIndex(0))),
    };
    let mut markers = ResolvedSceneMarkers::default();
    markers.rebuild(&mut selections, &registry, None);
    markers.rebuild(&mut selections, &registry, Some(&hover));

    let mut dirty = DirtyFlags::empty();
    visit_render_objects(
        &registry,
        &settings,
        &colors,
        &markers,
        &mut |_name, obj| {
            dirty = obj.dirty;
        },
    );

    assert!(!dirty.contains(DirtyFlags::SELECTION));
    assert!(dirty.contains(DirtyFlags::HOVER));
    assert!(dirty.is_lut_only());
}

#[test]
fn visit_render_objects_uses_displayed_coord_set() {
    let registry = registry_with_display_state(1);
    let settings = Settings::default();
    let colors = ResolvedSceneColors::build(
        &registry,
        &settings,
        &NamedPalette::new(),
        &ThemedPalette::dark(),
    );
    let markers = ResolvedSceneMarkers::default();
    let mut displayed_x = None;

    visit_render_objects(
        &registry,
        &settings,
        &colors,
        &markers,
        &mut |_name, obj| {
            displayed_x = obj
                .coord_set
                .get_atom_coord(AtomIndex(0))
                .map(|coord| coord.x);
        },
    );

    assert_eq!(displayed_x, Some(7.0));
    assert_eq!(
        registry
            .get_molecule("obj")
            .unwrap()
            .molecule()
            .current_state,
        0
    );
}

#[test]
fn visit_render_objects_includes_object_overrides() {
    let mut registry = registry_with_two_atoms();
    let settings = Settings::default();
    registry
        .get_molecule_mut("obj")
        .unwrap()
        .get_or_create_overrides()
        .cartoon
        .smooth_loops = Some(true);

    let colors = ResolvedSceneColors::build(
        &registry,
        &settings,
        &NamedPalette::new(),
        &ThemedPalette::dark(),
    );
    let markers = ResolvedSceneMarkers::default();
    let mut smooth_loops = None;

    visit_render_objects(
        &registry,
        &settings,
        &colors,
        &markers,
        &mut |_name, obj| {
            smooth_loops = obj.object_settings.map(|s| s.cartoon.smooth_loops);
        },
    );

    assert_eq!(smooth_loops, Some(true));
}

#[test]
fn visit_render_scene_emits_renderable_maps_in_order() {
    let mut registry = registry_with_two_atoms();
    let grid = Grid3D::from_dims([0.0; 3], [1.0; 3], [1, 1, 1], vec![0.0; 8]);
    let mut map = MapObject::new("mesh1", grid);
    map.set_display_mode(MapDisplayMode::Isomesh);
    registry.add(map);

    let settings = Settings::default();
    let colors = ResolvedSceneColors::build(
        &registry,
        &settings,
        &NamedPalette::new(),
        &ThemedPalette::dark(),
    );
    let markers = ResolvedSceneMarkers::default();
    let names = RefCell::new(Vec::new());
    let mut map_ids = Vec::new();

    visit_render_scene(
        &registry,
        &settings,
        &colors,
        &markers,
        &mut |name, obj| {
            names.borrow_mut().push(name.to_string());
            assert_eq!(obj.object_id.0, 1);
        },
        &mut |name, map| {
            names.borrow_mut().push(name.to_string());
            map_ids.push(map.object_id.0);
        },
    );

    assert_eq!(names.into_inner(), ["obj", "mesh1"]);
    assert_eq!(map_ids, [2]);
}

#[test]
fn visit_render_scene_keeps_object_ids_stable_when_middle_object_is_disabled() {
    let mut registry = ObjectRegistry::new();
    registry.add(molecule_object("a"));
    registry.add(molecule_object("b"));
    registry.add(molecule_object("c"));

    assert_eq!(
        collect_render_object_ids(&registry),
        [
            ("a".to_string(), 1),
            ("b".to_string(), 2),
            ("c".to_string(), 3),
        ]
    );

    registry.enable("b", false).unwrap();

    assert_eq!(
        collect_render_object_ids(&registry),
        [("a".to_string(), 1), ("c".to_string(), 3)]
    );
}

#[test]
fn resolve_pick_uses_sparse_object_id_lookup() {
    let mut session = Session::new();
    session.registry.add(molecule_object("obj"));
    let obj_id = session.registry.render_id("obj").unwrap();
    let mut names = vec![None; obj_id.slot_index() + 1];
    names[obj_id.slot_index()] = Some("obj".to_string());

    let hit = RenderPickHit {
        rep_kind: RepKind::Sphere,
        object_id: ObjectId(obj_id.get()),
        atom_id: 0,
    };
    let resolved = resolve_pick(hit, &names, &session).unwrap();
    assert_eq!(resolved.object_name, "obj");
    assert_eq!(resolved.atom_index, Some(AtomIndex::from(0usize)));

    names[obj_id.slot_index()] = None;
    assert!(resolve_pick(hit, &names, &session).is_none());

    names[obj_id.slot_index()] = Some("obj".to_string());
    session.registry.enable("obj", false).unwrap();
    assert!(resolve_pick(hit, &names, &session).is_none());
}
