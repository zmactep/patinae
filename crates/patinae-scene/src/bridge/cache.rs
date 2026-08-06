//! Cached render-scene bridge state.
//!
//! Hosts keep this cache alive across frames so per-atom colors, marker bits,
//! and picking name lookups are rebuilt only when scene state changes.

use std::cell::RefCell;

use patinae_render::{RenderInput, RenderMapInput, RenderObjectInput, RenderStrokeInput, SceneLod};
use patinae_settings::ResolvedSettings;

use crate::{session::Session, ResolvedAnnotationBundle};

use super::{
    picking::render_id_slot_index, visit_render_scene, ResolvedSceneColors, ResolvedSceneMarkers,
    ResolvedSceneStrokes,
};

/// Persistent host-side cache for renderer input.
///
/// The renderer input itself borrows from the current [`Session`], so each
/// frame still owns short-lived input vectors. The expensive per-atom buffers
/// and the sparse render-id-to-name picking lookup persist here.
#[derive(Default)]
pub struct CachedRenderScene {
    colors: ResolvedSceneColors,
    markers: ResolvedSceneMarkers,
    recent_atom_targets: Vec<(String, patinae_mol::AtomIndex)>,
    recent_atom_target_key: Option<(u64, u64, u64)>,
    annotation_strokes: ResolvedSceneStrokes,
    object_names: Vec<Option<String>>,
}

impl CachedRenderScene {
    /// Builds a frame input using cached color and marker buffers.
    pub fn prepare<'a>(&'a mut self, session: &'a mut Session) -> CachedRenderFrame<'a> {
        if self.colors.needs_rebuild(&session.registry) {
            self.colors.rebuild(
                &session.registry,
                &session.settings,
                &session.named_palette,
                &session.palette,
            );
        }

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
            self.recent_atom_targets = session.resolved_recent_atoms();
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
            visit_render_scene(
                &session.registry,
                &session.settings,
                &self.colors,
                &self.markers,
                &mut |name, obj| {
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
    use lin_alg::f32::Vec3;
    use patinae_color::ColorIndex;
    use patinae_mol::{Atom, AtomIndex, CoordSet, DirtyFlags, Element, ObjectMolecule};
    use patinae_settings::groups::RecentPickLimit;

    use crate::{AtomAnchor, LabelEntity, LabelObject, MoleculeObject};

    use super::{CachedRenderScene, Session};

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
}
