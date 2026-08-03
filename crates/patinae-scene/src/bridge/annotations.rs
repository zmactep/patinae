//! Reconstructible cache for renderer-neutral annotation bundles.

use std::hash::{DefaultHasher, Hash, Hasher};

use ahash::AHashMap;
use patinae_color::NamedPalette;
use patinae_settings::Settings;

use crate::{
    resolve_annotation_bundles, Camera, LabelAlignment, ObjectRegistry, RenderObjectId,
    ResolvedAnnotationBundle,
};

/// Per-context cache of authoritative annotation bundles.
///
/// The cache owns only reconstructible derived data; semantic state remains in
/// the object registry and no cache entry is serialized.
#[derive(Debug, Default)]
pub struct ResolvedSceneAnnotations {
    bundles: Vec<ResolvedAnnotationBundle>,
    by_owner: AHashMap<RenderObjectId, usize>,
}

impl ResolvedSceneAnnotations {
    /// Rebuilds all bundles from current registry state and global settings.
    pub fn rebuild(
        &mut self,
        registry: &ObjectRegistry,
        settings: &Settings,
        named: &NamedPalette,
    ) {
        let bundles = resolve_annotation_bundles(registry, settings, named);
        self.by_owner.clear();
        self.by_owner.reserve(bundles.len());
        for (index, bundle) in bundles.iter().enumerate() {
            self.by_owner.insert(bundle.owner_id, index);
        }
        self.bundles = bundles;
    }

    /// Returns bundles in deterministic registry render order.
    pub fn bundles(&self) -> &[ResolvedAnnotationBundle] {
        &self.bundles
    }

    /// Returns one cached bundle by stable semantic-owner identity.
    pub fn bundle(&self, owner_id: RenderObjectId) -> Option<&ResolvedAnnotationBundle> {
        self.by_owner
            .get(&owner_id)
            .and_then(|&index| self.bundles.get(index))
    }
}

/// One label after camera projection, ready for a native or web overlay.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectedAnnotationLabel {
    /// Physical viewport x coordinate.
    pub x: f32,
    /// Physical viewport y coordinate.
    pub y: f32,
    /// Display text copied from the semantic bundle.
    pub text: String,
    /// Resolved RGBA color.
    pub color: [f32; 4],
    /// Font size in device-independent UI pixels.
    pub size: f32,
    /// Text alignment around the projected anchor.
    pub alignment: LabelAlignment,
    /// Stable identity of the semantic owner.
    pub owner_id: RenderObjectId,
    /// Position of the entity inside its owner collection.
    pub insertion_ordinal: usize,
    /// Deterministic paint order; larger values paint later.
    pub display_order: u64,
}

/// Camera-dependent label cache owned independently by each render context.
#[derive(Debug, Default)]
pub struct ProjectedSceneLabels {
    fingerprint: Option<u64>,
    labels: Vec<ProjectedAnnotationLabel>,
}

impl ProjectedSceneLabels {
    /// Reprojects only when camera, viewport, or semantic labels changed.
    ///
    /// Returns whether the projected payload was rebuilt.
    pub fn rebuild(
        &mut self,
        camera: &Camera,
        viewport: (f32, f32, f32, f32),
        bundles: &[ResolvedAnnotationBundle],
    ) -> bool {
        let fingerprint = projection_fingerprint(camera, viewport, bundles);
        if self.fingerprint == Some(fingerprint) {
            return false;
        }
        self.labels = project_annotation_labels(camera, viewport, bundles);
        self.fingerprint = Some(fingerprint);
        true
    }

    /// Returns onscreen labels in deterministic paint order.
    pub fn labels(&self) -> &[ProjectedAnnotationLabel] {
        &self.labels
    }

    /// Clears the cache, for example while a captured viewport image replaces live rendering.
    pub fn clear(&mut self) {
        self.fingerprint = None;
        self.labels.clear();
    }
}

/// Projects all resolved labels with shared clipping and ordering semantics.
pub fn project_annotation_labels(
    camera: &Camera,
    viewport: (f32, f32, f32, f32),
    bundles: &[ResolvedAnnotationBundle],
) -> Vec<ProjectedAnnotationLabel> {
    let (viewport_x, viewport_y, viewport_width, viewport_height) = viewport;
    if !viewport_x.is_finite()
        || !viewport_y.is_finite()
        || !viewport_width.is_finite()
        || !viewport_height.is_finite()
        || viewport_width <= 0.0
        || viewport_height <= 0.0
    {
        return Vec::new();
    }
    let max_x = viewport_x + viewport_width;
    let max_y = viewport_y + viewport_height;
    let mut projected = bundles
        .iter()
        .flat_map(|bundle| &bundle.labels)
        .filter_map(|label| {
            let (x, y) = camera.project_to_screen(label.position, viewport)?;
            if !(viewport_x..=max_x).contains(&x) || !(viewport_y..=max_y).contains(&y) {
                return None;
            }
            Some(ProjectedAnnotationLabel {
                x,
                y,
                text: label.text.clone(),
                color: label.color,
                size: label.size,
                alignment: label.alignment,
                owner_id: label.owner_id,
                insertion_ordinal: label.insertion_ordinal,
                display_order: label.display_order,
            })
        })
        .collect::<Vec<_>>();
    projected.sort_by_key(|label| label.display_order);
    projected
}

fn projection_fingerprint(
    camera: &Camera,
    viewport: (f32, f32, f32, f32),
    bundles: &[ResolvedAnnotationBundle],
) -> u64 {
    let mut hasher = DefaultHasher::new();
    for value in camera
        .view_matrix()
        .data
        .into_iter()
        .chain(camera.projection_matrix().data)
        .chain([viewport.0, viewport.1, viewport.2, viewport.3])
    {
        value.to_bits().hash(&mut hasher);
    }
    bundles.len().hash(&mut hasher);
    for bundle in bundles {
        bundle.owner_id.get().hash(&mut hasher);
        bundle.label_revision.hash(&mut hasher);
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use lin_alg::f32::Vec3;
    use patinae_color::{ColorIndex, NamedPalette};
    use patinae_mol::{Atom, AtomIndex, CoordSet, DirtyFlags, Element, ObjectMolecule};
    use patinae_settings::Settings;

    use crate::{
        AtomAnchor, GroupObject, LabelAlignment, LabelEntity, LabelEntityPresentation, LabelObject,
        MeasurementEntity, MeasurementKind, MeasurementObject, MoleculeObject, Object,
        ObjectRegistry, ResolvedAnnotationBundle, StrokePath,
    };

    use super::{project_annotation_labels, ProjectedSceneLabels, ResolvedSceneAnnotations};

    fn molecule(name: &str, points: &[Vec3]) -> (MoleculeObject, Vec<AtomIndex>) {
        let mut molecule = ObjectMolecule::new(name);
        for _ in points {
            molecule.add_atom(Atom::new("C", Element::Carbon));
        }
        molecule.add_coord_set(CoordSet::from_vec3(points));
        let indices = (0..points.len())
            .map(|index| AtomIndex(u32::try_from(index).expect("atom count must fit AtomIndex")))
            .collect();
        (MoleculeObject::with_name(molecule, name), indices)
    }

    fn add_measurement(
        registry: &mut ObjectRegistry,
        name: &str,
        kind: MeasurementKind,
        anchors: Vec<AtomAnchor>,
    ) {
        let mut measurement = MeasurementObject::new(name, kind);
        measurement
            .add_entry(MeasurementEntity::new(anchors))
            .expect("valid measurement cardinality");
        registry.add(measurement);
    }

    fn rebuild(registry: &ObjectRegistry, settings: &Settings) -> ResolvedSceneAnnotations {
        let mut cache = ResolvedSceneAnnotations::default();
        cache.rebuild(registry, settings, &NamedPalette::default());
        cache
    }

    fn bundle<'a>(cache: &'a ResolvedSceneAnnotations, name: &str) -> &'a ResolvedAnnotationBundle {
        cache
            .bundles()
            .iter()
            .find(|bundle| bundle.owner_name == name)
            .expect("annotation bundle")
    }

    #[test]
    fn resolves_one_high_level_bundle_without_tessellating_its_path() {
        let mut molecule = ObjectMolecule::new("source");
        molecule.add_atom(Atom::new("A", Element::Carbon));
        molecule.add_atom(Atom::new("B", Element::Carbon));
        molecule.add_coord_set(CoordSet::from_vec3(&[
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(2.0, 0.0, 0.0),
        ]));
        let mut registry = ObjectRegistry::new();
        registry.add(MoleculeObject::with_name(molecule, "source"));
        let mut measurement = MeasurementObject::new("distance", MeasurementKind::Distance);
        measurement
            .add_entry(MeasurementEntity::new(vec![
                AtomAnchor::new("source", AtomIndex(0)),
                AtomAnchor::new("source", AtomIndex(1)),
            ]))
            .expect("distance entity");
        registry.add(measurement);

        let mut cache = ResolvedSceneAnnotations::default();
        cache.rebuild(&registry, &Settings::default(), &NamedPalette::default());

        let bundle = &cache.bundles()[0];
        assert_eq!(bundle.owner_name, "distance");
        assert_eq!(bundle.paths.len(), 1);
        assert!(matches!(bundle.paths[0].path, StrokePath::Segment { .. }));
        assert_eq!(bundle.labels[0].text, "2.0 Å");
    }

    #[test]
    fn cross_molecule_anchors_ignore_hidden_sources_and_apply_world_transforms() {
        let (mut first, first_keys) = molecule("first", &[Vec3::new(0.0, 0.0, 0.0)]);
        let (mut second, second_keys) = molecule("second", &[Vec3::new(1.0, 0.0, 0.0)]);
        first.state_mut().transform = lin_alg::f32::Mat4::new_translation(Vec3::new(1.0, 0.0, 0.0));
        second.state_mut().transform =
            lin_alg::f32::Mat4::new_translation(Vec3::new(4.0, 0.0, 0.0));
        first.state_mut().enabled = false;
        second.state_mut().enabled = false;
        let mut registry = ObjectRegistry::new();
        registry.add(first);
        registry.add(second);
        add_measurement(
            &mut registry,
            "distance",
            MeasurementKind::Distance,
            vec![
                AtomAnchor::new("first", first_keys[0]),
                AtomAnchor::new("second", second_keys[0]),
            ],
        );

        let cache = rebuild(&registry, &Settings::default());
        let distance = bundle(&cache, "distance");

        assert_eq!(distance.labels[0].text, "4.0 Å");
        let StrokePath::Segment { start, end } = distance.paths[0].path else {
            panic!("distance must remain one high-level segment")
        };
        assert_eq!(start, Vec3::new(1.0, 0.0, 0.0));
        assert_eq!(end, Vec3::new(5.0, 0.0, 0.0));
    }

    #[test]
    fn resolves_distance_angle_and_signed_dihedral_paths_and_labels() {
        let points = [
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, 1.0, 1.0),
        ];
        let (source, keys) = molecule("source", &points);
        let mut registry = ObjectRegistry::new();
        registry.add(source);
        add_measurement(
            &mut registry,
            "distance",
            MeasurementKind::Distance,
            vec![
                AtomAnchor::new("source", keys[0]),
                AtomAnchor::new("source", keys[1]),
            ],
        );
        add_measurement(
            &mut registry,
            "angle",
            MeasurementKind::Angle,
            vec![
                AtomAnchor::new("source", keys[0]),
                AtomAnchor::new("source", keys[1]),
                AtomAnchor::new("source", keys[2]),
            ],
        );
        add_measurement(
            &mut registry,
            "dihedral",
            MeasurementKind::Dihedral,
            keys.iter()
                .map(|&key| AtomAnchor::new("source", key))
                .collect(),
        );

        let cache = rebuild(&registry, &Settings::default());
        assert_eq!(bundle(&cache, "distance").paths.len(), 1);
        assert_eq!(bundle(&cache, "angle").paths.len(), 3);
        let dihedral = bundle(&cache, "dihedral");
        assert_eq!(dihedral.paths.len(), 6);
        assert_eq!(bundle(&cache, "angle").labels[0].text, "90.0°");
        assert_eq!(dihedral.labels[0].text, "-90.0°");

        let StrokePath::Arc {
            center,
            x_axis,
            y_axis,
            sweep_radians,
        } = dihedral.paths[3].path
        else {
            panic!("fourth dihedral path must remain an arc")
        };
        assert!(sweep_radians < 0.0);
        let endpoint = center + x_axis * sweep_radians.cos() + y_axis * sweep_radians.sin();
        let expected = center + Vec3::new(0.0, 0.0, x_axis.magnitude());
        assert!((endpoint - expected).magnitude() < 1.0e-5);
    }

    #[test]
    fn degenerate_measurements_recover_and_orphans_remain_counted() {
        let (source, keys) = molecule(
            "source",
            &[Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)],
        );
        let mut registry = ObjectRegistry::new();
        registry.add(source);
        add_measurement(
            &mut registry,
            "distance",
            MeasurementKind::Distance,
            vec![
                AtomAnchor::new("source", keys[0]),
                AtomAnchor::new("source", keys[1]),
            ],
        );

        let initial = rebuild(&registry, &Settings::default());
        assert_eq!(bundle(&initial, "distance").unresolved_count, 1);
        assert!(bundle(&initial, "distance").paths.is_empty());

        registry
            .get_molecule_mut("source")
            .unwrap()
            .molecule_mut_with_dirty(DirtyFlags::COORDS)
            .set_coord(AtomIndex(1), 0, Vec3::new(2.0, 0.0, 0.0));
        let recovered = rebuild(&registry, &Settings::default());
        assert_eq!(bundle(&recovered, "distance").unresolved_count, 0);
        assert_eq!(bundle(&recovered, "distance").paths.len(), 1);

        registry
            .remove_molecule_atoms("source", &[keys[1]])
            .expect("remove source atom");
        let orphaned = rebuild(&registry, &Settings::default());
        assert_eq!(bundle(&orphaned, "distance").unresolved_count, 1);
        assert!(bundle(&orphaned, "distance").paths.is_empty());
        assert!(bundle(&orphaned, "distance").warning.is_some());
    }

    #[test]
    fn hidden_measurement_label_keeps_strokes_and_drops_only_text() {
        let (source, keys) = molecule(
            "source",
            &[Vec3::new(0.0, 0.0, 0.0), Vec3::new(2.0, 0.0, 0.0)],
        );
        let mut registry = ObjectRegistry::new();
        registry.add(source);
        add_measurement(
            &mut registry,
            "distance",
            MeasurementKind::Distance,
            vec![
                AtomAnchor::new("source", keys[0]),
                AtomAnchor::new("source", keys[1]),
            ],
        );
        registry
            .get_measurement_mut("distance")
            .unwrap()
            .entity_presentation_mut(0)
            .unwrap()
            .set_label_visible(false);

        let cache = rebuild(&registry, &Settings::default());
        let distance = bundle(&cache, "distance");
        assert_eq!(distance.paths.len(), 1);
        assert!(distance.labels.is_empty());
        assert!(distance.bounds.is_some());
    }

    #[test]
    fn standalone_labels_resolve_entity_object_global_style_and_anchor_bounds() {
        let named = NamedPalette::default();
        let (red_index, red) = named.get_by_name("red").unwrap();
        let (green_index, green) = named.get_by_name("green").unwrap();
        let (source, keys) = molecule(
            "source",
            &[
                Vec3::new(1.0, 2.0, 3.0),
                Vec3::new(4.0, 5.0, 6.0),
                Vec3::new(100.0, 100.0, 100.0),
            ],
        );
        let mut first = LabelEntityPresentation::default();
        first
            .set_color(ColorIndex::Named(red_index))
            .expect("named color");
        first.set_size(11.0).expect("valid size");
        let mut hidden = LabelEntityPresentation::default();
        hidden.set_visible(false);
        let mut labels = LabelObject::with_entities(
            "labels",
            vec![
                LabelEntity::with_presentation(AtomAnchor::new("source", keys[0]), "entity", first),
                LabelEntity::new(AtomAnchor::new("source", keys[1]), "object"),
                LabelEntity::with_presentation(
                    AtomAnchor::new("source", keys[2]),
                    "hidden",
                    hidden,
                ),
            ],
        );
        labels
            .presentation_mut()
            .set_color(ColorIndex::Named(green_index))
            .expect("named color");
        labels
            .presentation_mut()
            .set_size(22.0)
            .expect("valid size");
        let mut registry = ObjectRegistry::new();
        registry.add(source);
        registry.add(labels);
        let mut cache = ResolvedSceneAnnotations::default();
        cache.rebuild(&registry, &Settings::default(), &named);
        let labels = bundle(&cache, "labels");

        assert!(labels.paths.is_empty());
        assert_eq!(labels.labels.len(), 2);
        assert_eq!(labels.labels[0].color, red.to_rgba(1.0));
        assert_eq!(labels.labels[0].size, 11.0);
        assert_eq!(labels.labels[1].color, green.to_rgba(1.0));
        assert_eq!(labels.labels[1].size, 22.0);
        assert_eq!(
            labels.bounds,
            Some((Vec3::new(1.0, 2.0, 3.0), Vec3::new(4.0, 5.0, 6.0)))
        );
        assert_eq!(registry.object_extent("labels"), labels.bounds);
        let initial_geometry_revision = labels.geometry_revision;
        let initial_label_revision = labels.label_revision;

        registry
            .get_molecule_mut("source")
            .unwrap()
            .molecule_mut_with_dirty(DirtyFlags::COORDS)
            .set_coord(AtomIndex(0), 0, Vec3::new(-2.0, -3.0, -4.0));
        let moved = rebuild(&registry, &Settings::default());
        let labels = bundle(&moved, "labels");
        assert_eq!(
            labels.bounds,
            Some((Vec3::new(-2.0, -3.0, -4.0), Vec3::new(4.0, 5.0, 6.0)))
        );
        assert_ne!(labels.geometry_revision, initial_geometry_revision);
        assert_ne!(labels.label_revision, initial_label_revision);
    }

    #[test]
    fn disabled_containing_group_gates_output_but_keeps_owner_bundle() {
        let (source, keys) = molecule(
            "source",
            &[Vec3::new(1.0, 2.0, 3.0), Vec3::new(4.0, 5.0, 6.0)],
        );
        let mut registry = ObjectRegistry::new();
        registry.add(source);
        registry.add(LabelObject::with_entities(
            "labels",
            vec![LabelEntity::new(AtomAnchor::new("source", keys[0]), "CA")],
        ));
        add_measurement(
            &mut registry,
            "distance",
            MeasurementKind::Distance,
            vec![
                AtomAnchor::new("source", keys[0]),
                AtomAnchor::new("source", keys[1]),
            ],
        );
        registry.add(GroupObject::new("annotations"));
        assert!(registry.add_to_group("annotations", "labels"));
        assert!(registry.add_to_group("annotations", "distance"));
        registry.enable("annotations", false).unwrap();

        let cache = rebuild(&registry, &Settings::default());
        let labels = bundle(&cache, "labels");
        assert!(labels.paths.is_empty());
        assert!(labels.labels.is_empty());
        assert_eq!(
            labels.bounds,
            Some((Vec3::new(1.0, 2.0, 3.0), Vec3::new(1.0, 2.0, 3.0)))
        );
        assert_eq!(labels.unresolved_count, 0);
        assert_eq!(registry.object_extent("labels"), labels.bounds);

        let distance = bundle(&cache, "distance");
        assert!(distance.paths.is_empty());
        assert!(distance.labels.is_empty());
        assert_eq!(
            distance.bounds,
            Some((Vec3::new(1.0, 2.0, 3.0), Vec3::new(4.0, 5.0, 6.0)))
        );
        assert_eq!(distance.unresolved_count, 0);
        assert_eq!(registry.object_extent("distance"), distance.bounds);
    }

    #[test]
    fn owner_and_entity_order_produce_stable_label_display_order() {
        let (source, keys) = molecule(
            "source",
            &[Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)],
        );
        let mut registry = ObjectRegistry::new();
        registry.add(source);
        registry.add(LabelObject::with_entities(
            "first",
            vec![
                LabelEntity::new(AtomAnchor::new("source", keys[0]), "a"),
                LabelEntity::new(AtomAnchor::new("source", keys[1]), "b"),
            ],
        ));
        registry.add(LabelObject::with_entities(
            "second",
            vec![LabelEntity::new(AtomAnchor::new("source", keys[0]), "c")],
        ));

        let first_pass = rebuild(&registry, &Settings::default());
        let second_pass = rebuild(&registry, &Settings::default());
        let first = bundle(&first_pass, "first");
        let second = bundle(&first_pass, "second");
        assert!(first.owner_order < second.owner_order);
        assert_eq!(first.labels[0].insertion_ordinal, 0);
        assert_eq!(first.labels[1].insertion_ordinal, 1);
        assert!(first.labels[1].display_order < second.labels[0].display_order);
        assert_eq!(first_pass.bundles(), second_pass.bundles());
    }

    #[test]
    fn revision_tokens_are_split_by_geometry_material_and_labels() {
        let (source, keys) = molecule(
            "source",
            &[Vec3::new(0.0, 0.0, 0.0), Vec3::new(2.0, 0.0, 0.0)],
        );
        let mut registry = ObjectRegistry::new();
        registry.add(source);
        add_measurement(
            &mut registry,
            "distance",
            MeasurementKind::Distance,
            vec![
                AtomAnchor::new("source", keys[0]),
                AtomAnchor::new("source", keys[1]),
            ],
        );
        let baseline_settings = Settings::default();
        let baseline_cache = rebuild(&registry, &baseline_settings);
        let baseline = bundle(&baseline_cache, "distance").clone();

        let mut label_settings = Settings::default();
        label_settings.measurement.label_size = 18.0;
        let label_cache = rebuild(&registry, &label_settings);
        let label_changed = bundle(&label_cache, "distance");
        assert_eq!(label_changed.geometry_revision, baseline.geometry_revision);
        assert_eq!(label_changed.material_revision, baseline.material_revision);
        assert_ne!(label_changed.label_revision, baseline.label_revision);

        let mut material_settings = Settings::default();
        material_settings.measurement.dash_width = 5.0;
        let material_cache = rebuild(&registry, &material_settings);
        let material_changed = bundle(&material_cache, "distance");
        assert_eq!(
            material_changed.geometry_revision,
            baseline.geometry_revision
        );
        assert_ne!(
            material_changed.material_revision,
            baseline.material_revision
        );
        assert_eq!(material_changed.label_revision, baseline.label_revision);

        let mut geometry_settings = Settings::default();
        geometry_settings.measurement.dash_length = 0.3;
        let geometry_cache = rebuild(&registry, &geometry_settings);
        let geometry_changed = bundle(&geometry_cache, "distance");
        assert_ne!(
            geometry_changed.geometry_revision,
            baseline.geometry_revision
        );
        assert_eq!(
            geometry_changed.material_revision,
            baseline.material_revision
        );
        assert_eq!(geometry_changed.label_revision, baseline.label_revision);

        registry
            .get_measurement_mut("distance")
            .unwrap()
            .entity_presentation_mut(0)
            .unwrap()
            .set_label_visible(false);
        let hidden_cache = rebuild(&registry, &Settings::default());
        let hidden = bundle(&hidden_cache, "distance");
        assert_eq!(hidden.geometry_revision, baseline.geometry_revision);
        assert_eq!(hidden.material_revision, baseline.material_revision);
        assert_ne!(hidden.label_revision, baseline.label_revision);
    }

    #[test]
    fn large_label_collection_is_not_truncated() {
        let (source, keys) = molecule("source", &[Vec3::new(0.0, 0.0, 0.0)]);
        let entities = (0..65_537)
            .map(|index| {
                LabelEntity::new(AtomAnchor::new("source", keys[0]), format!("label-{index}"))
            })
            .collect();
        let mut registry = ObjectRegistry::new();
        registry.add(source);
        registry.add(LabelObject::with_entities("labels", entities));

        let cache = rebuild(&registry, &Settings::default());
        assert_eq!(bundle(&cache, "labels").labels.len(), 65_537);
    }

    #[test]
    fn shared_projection_culls_depth_and_viewport_but_preserves_overlaps_and_order() {
        let (source, keys) = molecule(
            "source",
            &[
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1_000.0, 0.0, 0.0),
                Vec3::new(0.0, 0.0, 49.95),
            ],
        );
        let mut registry = ObjectRegistry::new();
        registry.add(source);
        registry.add(LabelObject::with_entities(
            "labels",
            vec![
                LabelEntity::new(AtomAnchor::new("source", keys[0]), "first"),
                LabelEntity::new(AtomAnchor::new("source", keys[0]), "second"),
                LabelEntity::new(AtomAnchor::new("source", keys[0]), "third"),
                LabelEntity::new(AtomAnchor::new("source", keys[1]), "offscreen"),
                LabelEntity::new(AtomAnchor::new("source", keys[2]), "near"),
            ],
        ));
        let cache = rebuild(&registry, &Settings::default());
        let camera = crate::Camera::new();
        let viewport = (0.0, 0.0, 200.0, 100.0);

        let projected = project_annotation_labels(&camera, viewport, cache.bundles());

        assert_eq!(projected.len(), 3);
        assert_eq!(
            projected
                .iter()
                .map(|label| label.text.as_str())
                .collect::<Vec<_>>(),
            ["first", "second", "third"]
        );
        assert!(projected
            .windows(2)
            .all(|pair| pair[0].x == pair[1].x && pair[0].y == pair[1].y));
        assert!(projected
            .windows(2)
            .all(|pair| pair[0].display_order < pair[1].display_order));
        assert_eq!(projected[0].alignment, LabelAlignment::BottomLeft);
        assert!(projected[0].size.is_finite() && projected[0].size > 0.0);
        assert_eq!(projected[0].owner_id, bundle(&cache, "labels").owner_id);
        assert_eq!(projected[0].insertion_ordinal, 0);
    }

    #[test]
    fn projected_label_cache_tracks_camera_viewport_and_label_revisions() {
        let (source, keys) = molecule("source", &[Vec3::new(0.0, 0.0, 0.0)]);
        let mut registry = ObjectRegistry::new();
        registry.add(source);
        registry.add(LabelObject::with_entities(
            "labels",
            vec![LabelEntity::new(AtomAnchor::new("source", keys[0]), "CA")],
        ));
        let resolved = rebuild(&registry, &Settings::default());
        let mut camera = crate::Camera::new();
        let mut projected = ProjectedSceneLabels::default();

        assert!(projected.rebuild(&camera, (0.0, 0.0, 200.0, 100.0), resolved.bundles()));
        assert!(!projected.rebuild(&camera, (0.0, 0.0, 200.0, 100.0), resolved.bundles()));
        assert!(projected.rebuild(&camera, (0.0, 0.0, 300.0, 100.0), resolved.bundles()));
        camera.view_mut().origin.x = 1.0;
        assert!(projected.rebuild(&camera, (0.0, 0.0, 300.0, 100.0), resolved.bundles()));

        registry
            .get_label_mut("labels")
            .expect("label object")
            .entity_mut(0)
            .expect("label entity")
            .set_text("CB");
        let changed = rebuild(&registry, &Settings::default());
        assert!(projected.rebuild(&camera, (0.0, 0.0, 300.0, 100.0), changed.bundles()));
        assert_eq!(projected.labels()[0].text, "CB");
    }
}
