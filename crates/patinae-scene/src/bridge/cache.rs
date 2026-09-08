//! Cached render-scene bridge state.
//!
//! Hosts keep this cache alive across frames so marker bits,
//! and picking name lookups are rebuilt only when scene state changes.

use std::cell::RefCell;

use patinae_render::{RenderInput, RenderMapInput, RenderObjectInput, RenderStrokeInput, SceneLod};
use patinae_settings::ResolvedSettings;

use crate::{session::Session, ResolvedAnnotationBundle};

use super::{
    objects::visit_render_scene_deferred, picking::render_id_slot_index, ResolvedSceneMarkers,
    ResolvedSceneStrokes,
};

/// Persistent host-side cache for renderer input.
///
/// The renderer input itself borrows from the current [`Session`], so each
/// frame still owns short-lived input vectors. Colors resolve directly into
/// renderer storage on dirty updates. The expensive marker buffers
/// and the sparse render-id-to-name picking lookup persist here.
#[derive(Default)]
pub struct CachedRenderScene {
    markers: ResolvedSceneMarkers,
    recent_atom_targets: Vec<(String, patinae_mol::AtomIndex)>,
    recent_atom_markers: std::collections::HashMap<String, Vec<patinae_render::RecentAtomMarker>>,
    recent_atom_target_key: Option<(u64, u64, u64)>,
    annotation_strokes: ResolvedSceneStrokes,
    object_names: Vec<Option<String>>,
}

impl CachedRenderScene {
    /// Builds a frame with deferred colors and cached marker buffers.
    pub fn prepare<'a>(&'a mut self, session: &'a mut Session) -> CachedRenderFrame<'a> {
        let recent_observation = (
            session.recent_atoms.incarnation(),
            session.recent_atoms.generation(),
        );
        let recent_target_key = (
            session.registry.generation(),
            recent_observation.0,
            recent_observation.1,
        );
        if self.recent_atom_target_key != Some(recent_target_key) {
            self.recent_atom_targets.clear();
            self.recent_atom_markers.clear();
            for anchor in session.resolved_recent_atom_anchors() {
                self.recent_atom_markers
                    .entry(anchor.object_name.clone())
                    .or_default()
                    .push(patinae_render::RecentAtomMarker {
                        atom_index: anchor.atom_index.as_u32(),
                        instance: anchor.instance,
                    });
                self.recent_atom_targets
                    .push((anchor.object_name, anchor.atom_index));
            }
            self.recent_atom_target_key = Some(recent_target_key);
        }
        self.markers.rebuild_with_recent(
            &mut session.selections,
            &session.registry,
            &self.recent_atom_targets,
            recent_observation,
            session.hover_target.as_ref(),
        );
        if self.annotation_strokes.needs_rebuild(
            &session.registry,
            &session.settings,
            &session.named_palette,
        ) {
            self.annotation_strokes.rebuild(
                &session.registry,
                &session.settings,
                &session.named_palette,
            );
        }

        let mut objects = Vec::new();
        let mut maps = Vec::new();
        {
            let names = RefCell::new(&mut self.object_names);
            names.borrow_mut().clear();
            visit_render_scene_deferred(
                &session.registry,
                &session.settings,
                &session.named_palette,
                &session.palette,
                &self.markers,
                &mut |name, mut obj| {
                    obj.recent_atom_markers = Some(
                        self.recent_atom_markers
                            .get(name)
                            .map_or(&[], Vec::as_slice),
                    );
                    record_object_name(&mut names.borrow_mut(), obj.object_id.0, name);
                    objects.push(obj);
                },
                &mut |name, map| {
                    record_object_name(&mut names.borrow_mut(), map.object_id.0, name);
                    maps.push(map);
                },
            );
        }

        let settings = ResolvedSettings::resolve(&session.settings, None);
        let lod = objects.first().map(|o| o.lod).unwrap_or(SceneLod::Auto);
        let strokes = self.annotation_strokes.render_inputs();

        CachedRenderFrame {
            objects,
            maps,
            strokes,
            settings,
            lod,
        }
    }

    /// Returns sparse names indexed by `RenderObjectId::slot_index()`.
    pub fn object_names(&self) -> &[Option<String>] {
        &self.object_names
    }

    /// Returns authoritative annotation bundles from the latest prepared frame.
    pub fn annotation_bundles(&self) -> &[ResolvedAnnotationBundle] {
        self.annotation_strokes.annotation_bundles()
    }
}

fn record_object_name(names: &mut Vec<Option<String>>, object_id: u32, name: &str) {
    let Some(idx) = render_id_slot_index(object_id) else {
        return;
    };
    if names.len() <= idx {
        names.resize_with(idx + 1, || None);
    }
    names[idx] = Some(name.to_string());
}

/// Short-lived renderer input for a single frame.
pub struct CachedRenderFrame<'a> {
    objects: Vec<RenderObjectInput<'a>>,
    maps: Vec<RenderMapInput<'a>>,
    strokes: Vec<RenderStrokeInput<'a>>,
    settings: ResolvedSettings,
    lod: SceneLod,
}

impl<'a> CachedRenderFrame<'a> {
    /// Returns borrowed render input for [`patinae_render::RenderState::sync`].
    pub fn render_input(&self) -> RenderInput<'_> {
        RenderInput {
            objects: &self.objects,
            maps: &self.maps,
            strokes: &self.strokes,
            settings: &self.settings,
            lod: self.lod,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::bridge::ResolvedSceneColors;
    use lin_alg::f32::Vec3;
    use patinae_color::{Color, ColorIndex, ThemedPalette};
    use patinae_mol::{Atom, AtomIndex, CoordSet, DirtyFlags, Element, ObjectMolecule};
    use patinae_render::{scene_store::SceneStore, ColorLutEntry};
    use patinae_settings::{groups::RecentPickLimit, Color as SettingColor};

    use crate::{AtomAnchor, LabelEntity, LabelObject, MoleculeObject};

    use super::{CachedRenderScene, Session};

    #[test]
    fn recent_markers_keep_copy_identity_through_cache_changes_and_materialization() {
        use crate::object::Object;
        use patinae_mol::{InstanceGroup, InstanceTable, ObjectInstance};
        use patinae_render::{RecentAtomMarker, IDENTITY_TRANSFORM};

        let mut mol = ObjectMolecule::new("capsid");
        mol.add_atom(Atom::new("CA", Element::Carbon));
        mol.add_coord_set(CoordSet::from_vec3(&[Vec3::new(0.0, 0.0, 0.0)]));
        let path = crate::canonical_atom_path_for_atom("capsid", &mol, AtomIndex(0)).unwrap();
        let mut object = MoleculeObject::with_name(mol, "capsid");
        object.state_mut().instances = Some(InstanceTable {
            groups: vec![InstanceGroup { indices: vec![] }],
            copies: (0..2)
                .map(|copy| {
                    let mut transform = IDENTITY_TRANSFORM;
                    transform[3][0] = copy as f32 * 10.0;
                    ObjectInstance {
                        group: 0,
                        transform,
                    }
                })
                .collect(),
        });
        let mut session = Session::new();
        session.registry.add(object);
        let paths = [
            format!("instance 1 and {path}"),
            format!("instance 2 and {path}"),
        ];
        let mut cache = CachedRenderScene::default();
        let marker = |copy| RecentAtomMarker {
            atom_index: 0,
            instance: Some(copy),
        };
        session
            .recent_atoms
            .insert(paths[0].clone(), RecentPickLimit::Unlimited);
        {
            let frame = cache.prepare(&mut session);
            assert_eq!(
                frame.render_input().objects[0].recent_atom_markers,
                Some([marker(0)].as_slice())
            );
        }
        session.registry.clear_all_dirty_objects();
        session.recent_atoms.remove_path(&paths[0]);
        session
            .recent_atoms
            .insert(paths[1].clone(), RecentPickLimit::Unlimited);
        {
            let frame = cache.prepare(&mut session);
            let input = frame.render_input();
            assert_eq!(input.objects[0].atom_markers, [super::super::MARKER_RECENT]);
            assert_eq!(
                input.objects[0].recent_atom_markers,
                Some([marker(1)].as_slice())
            );
        }
        session
            .recent_atoms
            .insert(paths[0].clone(), RecentPickLimit::Unlimited);
        {
            let frame = cache.prepare(&mut session);
            assert_eq!(
                frame.render_input().objects[0].recent_atom_markers,
                Some([marker(1), marker(0)].as_slice())
            );
        }
        session.recent_atoms.remove_path(&paths[0]);
        session.materialize_object("capsid").unwrap();
        {
            let frame = cache.prepare(&mut session);
            assert_eq!(
                frame.render_input().objects[0].recent_atom_markers,
                Some(
                    [RecentAtomMarker {
                        atom_index: 1,
                        instance: None
                    }]
                    .as_slice()
                )
            );
        }
        let remaining = session.recent_atoms.paths().next().unwrap().to_string();
        session.recent_atoms.remove_path(&remaining);
        let frame = cache.prepare(&mut session);
        assert_eq!(
            frame.render_input().objects[0].recent_atom_markers,
            Some([].as_slice())
        );
    }

    #[test]
    fn prepared_frame_marks_recent_atoms_without_enabling_selection_overlay() {
        let mut molecule = ObjectMolecule::new("source");
        molecule.add_atom(Atom::new("CA", Element::Carbon));
        molecule.add_coord_set(CoordSet::from_vec3(&[Vec3::new(1.0, 2.0, 3.0)]));
        let mut session = Session::new();
        session
            .registry
            .add(MoleculeObject::with_name(molecule, "source"));
        let source = session.registry.get_molecule("source").unwrap();
        let path =
            crate::canonical_atom_path_for_atom("source", source.molecule(), AtomIndex(0)).unwrap();
        session
            .recent_atoms
            .insert(path, RecentPickLimit::Unlimited);

        let mut cache = CachedRenderScene::default();
        let frame = cache.prepare(&mut session);
        let input = frame.render_input();

        assert_eq!(input.objects[0].atom_markers, [super::super::MARKER_RECENT]);
        assert!(!input.objects[0].has_markers);
    }

    #[test]
    fn prepared_frame_clears_removed_recent_atom_from_reused_cache() {
        let mut molecule = ObjectMolecule::new("source");
        molecule.add_atom(Atom::new("CA", Element::Carbon));
        molecule.add_coord_set(CoordSet::from_vec3(&[Vec3::new(1.0, 2.0, 3.0)]));
        let mut session = Session::new();
        session
            .registry
            .add(MoleculeObject::with_name(molecule, "source"));
        let source = session.registry.get_molecule("source").unwrap();
        let path =
            crate::canonical_atom_path_for_atom("source", source.molecule(), AtomIndex(0)).unwrap();
        session
            .recent_atoms
            .insert(path.clone(), RecentPickLimit::Unlimited);
        let mut cache = CachedRenderScene::default();

        drop(cache.prepare(&mut session));
        assert!(session.recent_atoms.remove_path(&path));
        let frame = cache.prepare(&mut session);
        let input = frame.render_input();

        assert_eq!(input.objects[0].atom_markers, [0]);
        assert_eq!(
            input.objects[0].marker_updates,
            [patinae_render::MarkerUpdate {
                atom_index: 0,
                bits: 0,
            }]
        );
    }

    #[test]
    fn prepared_frame_re_resolves_recent_atom_after_index_remap() {
        let mut molecule = ObjectMolecule::new("source");
        molecule.add_atom(Atom::new("first", Element::Carbon));
        molecule.add_atom(Atom::new("picked", Element::Nitrogen));
        molecule.add_coord_set(CoordSet::from_vec3(&[
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
        ]));
        let mut session = Session::new();
        session
            .registry
            .add(MoleculeObject::with_name(molecule, "source"));
        let source = session.registry.get_molecule("source").unwrap();
        let path =
            crate::canonical_atom_path_for_atom("source", source.molecule(), AtomIndex(1)).unwrap();
        session
            .recent_atoms
            .insert(path, RecentPickLimit::Unlimited);
        let mut cache = CachedRenderScene::default();

        let first = cache.prepare(&mut session);
        assert_eq!(
            first.render_input().objects[0].atom_markers,
            [0, super::super::MARKER_RECENT]
        );
        drop(first);
        session
            .remove_molecule_atoms("source", &[AtomIndex(0)])
            .unwrap();
        let second = cache.prepare(&mut session);

        assert_eq!(
            second.render_input().objects[0].atom_markers,
            [super::super::MARKER_RECENT]
        );
    }

    #[test]
    fn unchanged_bulk_labels_reuse_resolved_strokes() {
        let mut molecule = ObjectMolecule::new("source");
        molecule.add_atom(Atom::new("CA", Element::Carbon));
        molecule.add_coord_set(CoordSet::from_vec3(&[Vec3::new(1.0, 2.0, 3.0)]));
        let mut session = Session::new();
        session
            .registry
            .add(MoleculeObject::with_name(molecule, "source"));
        session.registry.add(LabelObject::with_entities(
            "labels",
            (0..2_048)
                .map(|index| {
                    LabelEntity::new(
                        AtomAnchor::new("source", AtomIndex(0)),
                        format!("label-{index}"),
                    )
                })
                .collect(),
        ));

        let mut cache = CachedRenderScene::default();
        drop(cache.prepare(&mut session));
        assert_eq!(cache.annotation_strokes.rebuild_count(), 1);
        session.registry.clear_all_dirty_objects();

        drop(cache.prepare(&mut session));
        assert_eq!(cache.annotation_strokes.rebuild_count(), 1);

        session
            .registry
            .get_label_mut("labels")
            .unwrap()
            .set_color(ColorIndex::Named(1));
        drop(cache.prepare(&mut session));
        assert_eq!(cache.annotation_strokes.rebuild_count(), 2);
        session.registry.clear_all_dirty_objects();

        session.settings.measurement.label_size = 18.0;
        drop(cache.prepare(&mut session));
        assert_eq!(cache.annotation_strokes.rebuild_count(), 3);
        session.registry.clear_all_dirty_objects();

        session
            .registry
            .get_molecule_mut("source")
            .unwrap()
            .invalidate(DirtyFlags::COORDS);
        drop(cache.prepare(&mut session));
        assert_eq!(cache.annotation_strokes.rebuild_count(), 4);
    }

    fn molecule(name: &str, count: usize) -> MoleculeObject {
        let mut mol = ObjectMolecule::new(name);
        for _ in 0..count {
            mol.add_atom(Atom::new("CA", Element::Carbon));
        }
        mol.add_coord_set(CoordSet::from_coords(vec![0.0; count * 3]));
        MoleculeObject::with_name(mol, name)
    }

    fn clear(session: &mut Session) {
        for name in ["one", "two"] {
            if let Some(mol) = session.registry.get_molecule_mut(name) {
                mol.clear_dirty();
            }
        }
    }

    fn check(session: &mut Session, cache: &mut CachedRenderScene, store: &mut SceneStore) {
        let expected = ResolvedSceneColors::build(
            &session.registry,
            &session.settings,
            &session.named_palette,
            &session.palette,
        );
        let expected: std::collections::HashMap<_, _> = ["one", "two"]
            .into_iter()
            .filter_map(|name| {
                let bases = expected.get(name)?;
                let reps = expected.get_rep(name).unwrap();
                Some((
                    session.registry.render_id(name).unwrap().get(),
                    bases
                        .iter()
                        .zip(reps)
                        .map(|(&base, &rep)| ColorLutEntry::new(base, rep))
                        .collect::<Vec<_>>(),
                ))
            })
            .collect();
        let frame = cache.prepare(session);
        let input = frame.render_input();
        assert_eq!(input.objects.len(), expected.len());
        for object in input.objects {
            let expected = &expected[&object.object_id.0];
            for (index, color) in expected.iter().enumerate() {
                assert_eq!(object.colors.get(index), Some(*color));
            }
            let dirty = if store.has_slot(object.object_id) {
                object.dirty
            } else {
                DirtyFlags::ALL
            };
            let slot = store.sync_object(object, dirty);
            let start = slot.atom_offset as usize;
            assert_eq!(
                &store.color_lut.cpu()[start..start + slot.atom_count as usize],
                expected
            );
        }
    }

    #[test]
    fn cached_colors_follow_atom_palette_settings_and_object_changes() {
        let mut session = Session::new();
        session.registry.add(molecule("one", 3));
        session.registry.add(molecule("two", 2));
        let mut cache = CachedRenderScene::default();
        let mut store = SceneStore::new();
        check(&mut session, &mut cache, &mut store);
        clear(&mut session);
        check(&mut session, &mut cache, &mut store);
        let red = session.named_palette.get_by_name("red").unwrap().0 as i32;
        let atom = session
            .registry
            .get_molecule_mut("one")
            .unwrap()
            .molecule_mut_with_dirty(DirtyFlags::COLOR)
            .get_atom_mut(AtomIndex(1))
            .unwrap();
        atom.repr.colors.base = red;
        atom.repr.colors.cartoon = red;
        atom.repr.colors.ribbon = red;
        atom.repr.colors.surface = red;
        check(&mut session, &mut cache, &mut store);
        clear(&mut session);
        session
            .named_palette
            .set("red", Color::new(0.15, 0.25, 0.35));
        session.registry.mark_all_dirty();
        check(&mut session, &mut cache, &mut store);
        clear(&mut session);
        session.palette = ThemedPalette::light();
        session.settings.sphere.color = SettingColor(red);
        session.registry.mark_all_dirty();
        check(&mut session, &mut cache, &mut store);
        clear(&mut session);
        let atom = session
            .registry
            .get_molecule_mut("two")
            .unwrap()
            .molecule_mut_with_dirty(DirtyFlags::COLOR | DirtyFlags::TRANSPARENCY)
            .get_atom_mut(AtomIndex(0))
            .unwrap();
        atom.repr.colors.base = ColorIndex::ByBFactor.into();
        atom.b_factor = 37.0;
        atom.repr.sphere_transparency = Some(0.4);
        check(&mut session, &mut cache, &mut store);
        session.registry.remove("one");
        session.registry.add(molecule("one", 3));
        check(&mut session, &mut cache, &mut store);
        session.registry.enable("two", false).unwrap();
        check(&mut session, &mut cache, &mut store);
        session.registry.enable("two", true).unwrap();
        check(&mut session, &mut cache, &mut store);
        clear(&mut session);
        let object = session.registry.get_molecule_mut("two").unwrap();
        object.get_or_create_overrides().sphere.color = Some(SettingColor(red));
        object.get_or_create_overrides().ribbon.color = Some(SettingColor(red));
        object.invalidate(DirtyFlags::COLOR);
        check(&mut session, &mut cache, &mut store);
    }

    #[test]
    fn deferred_colors_preserve_polymer_spectrum_and_all_representation_overrides() {
        let mut session = Session::new();
        let mut mol = ObjectMolecule::new("one");
        for index in 0..8 {
            let mut atom = Atom::new("CA", Element::Carbon);
            atom.residue = std::sync::Arc::new(patinae_mol::AtomResidue::from_parts(
                if index < 4 { "A" } else { "B" },
                "ALA",
                index,
                ' ',
                "",
            ));
            atom.state.flags = patinae_mol::AtomFlags::PROTEIN | patinae_mol::AtomFlags::POLYMER;
            atom.repr.colors.base = ColorIndex::ByResidueIndex.into();
            atom.repr.colors.sphere = ColorIndex::ByResidueIndex.into();
            atom.repr.colors.stick = ColorIndex::ByChain.into();
            atom.repr.colors.line = ColorIndex::ByBFactor.into();
            atom.repr.colors.dot = ColorIndex::ByElement.into();
            atom.repr.colors.cartoon = ColorIndex::ByResidueIndex.into();
            atom.repr.colors.ribbon = ColorIndex::BySS.into();
            atom.repr.colors.surface = ColorIndex::ByResidueType.into();
            atom.repr.colors.mesh = ColorIndex::ByResidueIndex.into();
            atom.repr.colors.ellipsoid = ColorIndex::ByResidueIndex.into();
            mol.add_atom(atom);
        }
        mol.add_coord_set(CoordSet::from_coords(vec![0.0; 24]));
        session.registry.add(MoleculeObject::with_name(mol, "one"));
        let mut cache = CachedRenderScene::default();
        let mut store = SceneStore::new();
        check(&mut session, &mut cache, &mut store);
        session
            .remove_molecule_atoms("one", &[AtomIndex(0)])
            .unwrap();
        check(&mut session, &mut cache, &mut store);
    }

    #[test]
    fn independent_hosts_resolve_current_colors_after_another_host_clears_dirty_flags() {
        let mut session = Session::new();
        session.registry.add(molecule("one", 2));
        let mut first = CachedRenderScene::default();
        let mut second = CachedRenderScene::default();
        check(&mut session, &mut first, &mut SceneStore::new());
        clear(&mut session);
        check(&mut session, &mut second, &mut SceneStore::new());
        session
            .registry
            .get_molecule_mut("one")
            .unwrap()
            .molecule_mut()
            .get_atom_mut(AtomIndex(0))
            .unwrap()
            .b_factor = 84.0;
        session
            .registry
            .get_molecule_mut("one")
            .unwrap()
            .molecule_mut()
            .get_atom_mut(AtomIndex(0))
            .unwrap()
            .repr
            .colors
            .base = ColorIndex::ByBFactor.into();
        check(&mut session, &mut first, &mut SceneStore::new());
        clear(&mut session);
        check(&mut session, &mut second, &mut SceneStore::new());
    }
}
