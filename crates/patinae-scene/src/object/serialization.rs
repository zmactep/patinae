//! Borrowed registry serialization with the existing snapshot wire format.

use serde::{Serialize, Serializer};

use super::*;

// Keep field names and order identical to the owned snapshot types. The owned
// snapshots remain available for callers that need an independent scene copy.
#[derive(Serialize)]
#[serde(rename = "ObjectRegistrySnapshot")]
struct RegistryRef<'a> {
    molecules: Vec<(&'a str, MoleculeRef<'a>)>,
    groups: Vec<(&'a str, &'a GroupObject)>,
    maps: Vec<(&'a str, MapRef<'a>)>,
    render_order: &'a [String],
    object_states: Vec<(&'a str, &'a ObjectState)>,
    render_ids: Vec<(&'a str, u32)>,
    next_render_id: u32,
    next_id: u32,
    generation: u64,
    measurements: Vec<(&'a str, MeasurementObjectSnapshot)>,
    labels: Vec<(&'a str, LabelObjectSnapshot)>,
}

#[derive(Serialize)]
#[serde(rename = "MoleculeObjectSnapshot")]
struct MoleculeRef<'a> {
    molecule: &'a patinae_mol::ObjectMolecule,
    state: &'a ObjectState,
    display_state: usize,
    overrides: Option<&'a ObjectOverrides>,
    surface_quality: i32,
}

#[derive(Serialize)]
#[serde(rename = "MapObjectSnapshot")]
struct MapRef<'a> {
    states: &'a [MapData],
    state: &'a ObjectState,
    current_state: usize,
    level: f32,
    display_mode: MapDisplayMode,
    mesh_color: [f32; 4],
    carve_radius: f32,
    carve_positions: Option<&'a Vec<[f32; 3]>>,
    overrides: Option<&'a ObjectOverrides>,
}

impl Serialize for ObjectRegistry {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut snapshot = RegistryRef {
            molecules: Vec::new(),
            groups: Vec::new(),
            maps: Vec::new(),
            render_order: &self.render_order,
            object_states: Vec::new(),
            render_ids: self
                .render_order
                .iter()
                .filter_map(|name| {
                    self.render_ids
                        .get(name)
                        .map(|id| (name.as_str(), id.get()))
                })
                .collect(),
            next_render_id: u32::from(self.next_render_id),
            next_id: self.next_id,
            generation: self.generation,
            measurements: Vec::new(),
            labels: Vec::new(),
        };
        for name in &self.render_order {
            let Some(obj) = self.objects.get(name) else {
                continue;
            };
            snapshot.object_states.push((name, obj.state()));
            if let Some(mol) = Self::object_as::<MoleculeObject>(obj.as_ref()) {
                snapshot.molecules.push((
                    name,
                    MoleculeRef {
                        molecule: mol.molecule(),
                        state: mol.state(),
                        display_state: mol.display_state(),
                        overrides: mol.overrides(),
                        surface_quality: mol.surface_quality(),
                    },
                ));
            } else if let Some(group) = Self::object_as::<GroupObject>(obj.as_ref()) {
                snapshot.groups.push((name, group));
            } else if let Some(map) = Self::object_as::<MapObject>(obj.as_ref()) {
                snapshot.maps.push((
                    name,
                    MapRef {
                        states: map.states(),
                        state: map.state(),
                        current_state: map.current_state(),
                        level: map.level(),
                        display_mode: map.display_mode(),
                        mesh_color: map.mesh_color(),
                        carve_radius: map.carve_radius(),
                        carve_positions: map.carve_positions(),
                        overrides: map.overrides(),
                    },
                ));
            } else if let Some(measurement) = Self::object_as::<MeasurementObject>(obj.as_ref()) {
                snapshot
                    .measurements
                    .push((name, measurement.to_snapshot()));
            } else if let Some(label) = Self::object_as::<LabelObject>(obj.as_ref()) {
                snapshot.labels.push((name, label.to_snapshot()));
            }
        }
        snapshot.serialize(serializer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use patinae_algos::surface::Grid3D;
    use patinae_mol::{Atom, CoordSet, Element, ObjectMolecule};

    #[test]
    fn borrowed_registry_matches_owned_snapshot_for_every_object_kind() {
        let mut registry = ObjectRegistry::new();
        let mut molecule = ObjectMolecule::new("molecule");
        molecule.add_atom(Atom::new("CA", Element::Carbon));
        molecule.add_coord_set(CoordSet::from_coords(vec![1.0, 2.0, 3.0]));
        molecule.add_coord_set(CoordSet::from_coords(vec![4.0, 5.0, 6.0]));
        let mut object = MoleculeObject::from_raw(molecule);
        object.set_display_state(1);
        registry.add(object);
        let grid = Grid3D::from_dims([0.0; 3], [1.0; 3], [1; 3], vec![0.5; 8]);
        let mut map = MapObject::new("map", grid);
        map.set_level(2.25);
        map.set_carve_positions(vec![[1.0, 2.0, 3.0]]);
        map.set_carve_radius(3.0);
        map.set_display_mode(MapDisplayMode::Isosurface);
        registry.add(map);
        let mut measurement = MeasurementObject::new("distance", MeasurementKind::Distance);
        measurement
            .add_entry(MeasurementEntry::new(vec![
                MeasurementAnchor::new("molecule", AtomIndex(0)),
                MeasurementAnchor::new("molecule", AtomIndex(0)),
            ]))
            .unwrap();
        registry.add(measurement);
        registry.add(LabelObject::with_entities(
            "labels",
            vec![LabelEntity::new(
                AtomAnchor::new("molecule", AtomIndex(0)),
                "label",
            )],
        ));
        registry.add(GroupObject::new("group"));
        registry.add_to_group("group", "molecule");
        registry.add_to_group("group", "map");
        assert_eq!(
            serde_json::to_value(&registry).unwrap(),
            serde_json::to_value(registry.to_snapshot()).unwrap(),
        );
    }
}
