//! Dynamic distance, angle, and dihedral scene objects.

use std::error::Error;
use std::fmt;

use lin_alg::f32::Vec3;
use serde::{Deserialize, Deserializer, Serialize};

use super::{
    annotation::has_mixed_annotation_colors, AnnotationPresentationError, AtomAnchor,
    MeasurementEntityPresentation, MeasurementObjectPresentation, MeasurementPresentation, Object,
    ObjectState, ObjectType,
};

/// Identifies the homogeneous measurement kind of one object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MeasurementKind {
    /// Distance between two atoms.
    Distance,
    /// Angle formed by three atoms.
    Angle,
    /// Signed dihedral angle formed by four atoms.
    Dihedral,
}

impl MeasurementKind {
    /// Returns the required number of anchors per entry.
    pub const fn anchor_count(self) -> usize {
        match self {
            Self::Distance => 2,
            Self::Angle => 3,
            Self::Dihedral => 4,
        }
    }
}

/// Backward-compatible name for [`MeasurementKind`].
pub type MeasurementType = MeasurementKind;

/// Backward-compatible name for a shared atom anchor.
pub type MeasurementAnchor = AtomAnchor;

/// Stores one dynamic measurement and sparse presentation overrides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeasurementEntity {
    /// Ordered anchors defining the measurement.
    pub anchors: Vec<AtomAnchor>,
    #[serde(default)]
    presentation: MeasurementEntityPresentation,
}

impl MeasurementEntity {
    /// Creates an entity from ordered anchors.
    ///
    /// The owning [`MeasurementObject`] validates kind-specific cardinality
    /// before accepting the entity.
    pub fn new(anchors: Vec<AtomAnchor>) -> Self {
        Self {
            anchors,
            presentation: MeasurementEntityPresentation::default(),
        }
    }

    /// Creates an entity with sparse presentation overrides.
    pub fn with_presentation(
        anchors: Vec<AtomAnchor>,
        presentation: MeasurementEntityPresentation,
    ) -> Self {
        Self {
            anchors,
            presentation,
        }
    }

    /// Returns sparse entity presentation overrides.
    pub const fn presentation(&self) -> &MeasurementEntityPresentation {
        &self.presentation
    }

    /// Returns mutable sparse entity presentation overrides.
    pub fn presentation_mut(&mut self) -> &mut MeasurementEntityPresentation {
        &mut self.presentation
    }

    /// Resolves entity, object, and global presentation precedence.
    pub fn resolve_presentation(
        &self,
        object: &MeasurementObjectPresentation,
        global: MeasurementPresentation,
    ) -> MeasurementPresentation {
        self.presentation.resolve(object, global)
    }
}

/// Backward-compatible name for a measurement entity.
pub type MeasurementEntry = MeasurementEntity;

/// Reports a measurement entry with the wrong anchor count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeasurementEntryError {
    expected: usize,
    actual: usize,
}

impl MeasurementEntryError {
    /// Returns the required anchor count.
    pub const fn expected(self) -> usize {
        self.expected
    }

    /// Returns the supplied anchor count.
    pub const fn actual(self) -> usize {
        self.actual
    }
}

impl fmt::Display for MeasurementEntryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "measurement requires {} anchors, got {}",
            self.expected, self.actual
        )
    }
}

impl Error for MeasurementEntryError {}

/// Revisions used to synchronize measurement render resources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeasurementRevisions {
    /// Changes when anchors or source lifecycle state changes.
    pub geometry: u64,
    /// Changes when object-level measurement appearance changes.
    pub material: u64,
    /// Changes when measurement-label presentation changes.
    #[serde(default = "default_revision")]
    pub labels: u64,
}

const fn default_revision() -> u64 {
    1
}

impl Default for MeasurementRevisions {
    fn default() -> Self {
        Self {
            geometry: 1,
            material: 1,
            labels: 1,
        }
    }
}

/// Serializable measurement object data.
#[derive(Debug, Clone, Serialize)]
pub struct MeasurementObjectSnapshot {
    /// Homogeneous kind shared by every entry.
    kind: MeasurementKind,
    /// Dynamic atom references.
    entries: Vec<MeasurementEntity>,
    /// Common object state.
    state: ObjectState,
    /// Whether the object color explicitly overrides type-specific settings.
    #[serde(default)]
    color_explicit: bool,
    /// Render synchronization revisions.
    #[serde(default)]
    revisions: MeasurementRevisions,
    /// Sparse object presentation defaults.
    #[serde(default)]
    presentation: MeasurementObjectPresentation,
}

#[derive(Deserialize)]
struct MeasurementObjectSnapshotWire {
    kind: MeasurementKind,
    entries: Vec<MeasurementEntity>,
    state: ObjectState,
    #[serde(default)]
    color_explicit: bool,
    #[serde(default)]
    revisions: MeasurementRevisions,
    #[serde(default)]
    presentation: MeasurementObjectPresentation,
}

impl MeasurementObjectSnapshotWire {
    fn into_snapshot(self) -> Result<MeasurementObjectSnapshot, String> {
        validate_entries(self.kind, &self.entries).map_err(|error| error.to_string())?;
        let mut presentation = self.presentation;
        if self.color_explicit && presentation.color().is_none() {
            presentation
                .set_color(self.state.color)
                .map_err(|error| error.to_string())?;
        }
        let mut state = self.state;
        if let Some(color) = presentation.color() {
            state.color = color;
        }
        Ok(MeasurementObjectSnapshot {
            kind: self.kind,
            entries: self.entries,
            state,
            color_explicit: presentation.color().is_some(),
            revisions: self.revisions,
            presentation,
        })
    }
}

impl MeasurementObjectSnapshot {
    /// Creates a validated measurement snapshot.
    ///
    /// # Errors
    ///
    /// Returns [`MeasurementEntryError`] when any entity has the wrong anchor
    /// count for `kind`.
    pub fn new(
        kind: MeasurementKind,
        entries: Vec<MeasurementEntity>,
        mut state: ObjectState,
        presentation: MeasurementObjectPresentation,
        revisions: MeasurementRevisions,
    ) -> Result<Self, MeasurementEntryError> {
        validate_entries(kind, &entries)?;
        if let Some(color) = presentation.color() {
            state.color = color;
        }
        Ok(Self {
            kind,
            entries,
            state,
            color_explicit: presentation.color().is_some(),
            revisions,
            presentation,
        })
    }

    /// Returns the homogeneous measurement kind.
    pub const fn kind(&self) -> MeasurementKind {
        self.kind
    }

    /// Returns stored entities in serialized order.
    pub fn entries(&self) -> &[MeasurementEntity] {
        &self.entries
    }

    /// Returns common object state.
    pub const fn state(&self) -> &ObjectState {
        &self.state
    }

    /// Returns sparse object presentation defaults.
    pub const fn presentation(&self) -> &MeasurementObjectPresentation {
        &self.presentation
    }

    /// Returns render synchronization revisions.
    pub const fn revisions(&self) -> MeasurementRevisions {
        self.revisions
    }
}

impl<'de> Deserialize<'de> for MeasurementObjectSnapshot {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        MeasurementObjectSnapshotWire::deserialize(deserializer)?
            .into_snapshot()
            .map_err(serde::de::Error::custom)
    }
}

/// Configures derived measurement labels and line geometry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeasurementResolveOptions {
    /// Length of each dash in world units.
    pub dash_length: f32,
    /// Gap between dashes in world units.
    pub dash_gap: f32,
    /// Angle arc radius as a fraction of the shorter leg.
    pub angle_size: f32,
    /// Radial multiplier for angle label placement.
    pub angle_label_position: f32,
    /// Dihedral arc radius as a fraction of the shorter projected leg.
    pub dihedral_size: f32,
    /// Radial multiplier for dihedral label placement.
    pub dihedral_label_position: f32,
    /// Decimal places for distance labels.
    pub distance_digits: usize,
    /// Decimal places for angle labels.
    pub angle_digits: usize,
    /// Decimal places for dihedral labels.
    pub dihedral_digits: usize,
}

impl Default for MeasurementResolveOptions {
    fn default() -> Self {
        Self {
            dash_length: 0.15,
            dash_gap: 0.45,
            angle_size: 0.6666,
            angle_label_position: 0.5,
            dihedral_size: 0.6666,
            dihedral_label_position: 1.2,
            distance_digits: 1,
            angle_digits: 1,
            dihedral_digits: 1,
        }
    }
}

impl MeasurementResolveOptions {
    /// Builds resolver options from typed measurement settings.
    pub fn from_settings(settings: &patinae_settings::groups::MeasurementSettings) -> Self {
        let common_digits = settings.label_digits.clamp(0, 9) as usize;
        let digits = |specific: i32| {
            if specific < 0 {
                common_digits
            } else {
                specific.clamp(0, 9) as usize
            }
        };
        Self {
            dash_length: settings.dash_length,
            dash_gap: settings.dash_gap,
            angle_size: settings.angle_size,
            angle_label_position: settings.angle_label_position,
            dihedral_size: settings.dihedral_size,
            dihedral_label_position: settings.dihedral_label_position,
            distance_digits: digits(settings.label_distance_digits),
            angle_digits: digits(settings.label_angle_digits),
            dihedral_digits: digits(settings.label_dihedral_digits),
        }
    }
}

/// A homogeneous collection of dynamic measurements.
#[derive(Debug, Serialize)]
pub struct MeasurementObject {
    name: String,
    kind: MeasurementKind,
    entries: Vec<MeasurementEntity>,
    state: ObjectState,
    #[serde(default)]
    color_explicit: bool,
    #[serde(default)]
    revisions: MeasurementRevisions,
    #[serde(default)]
    presentation: MeasurementObjectPresentation,
    #[serde(skip)]
    dirty: bool,
}

#[derive(Deserialize)]
struct MeasurementObjectWire {
    name: String,
    kind: MeasurementKind,
    entries: Vec<MeasurementEntity>,
    state: ObjectState,
    #[serde(default)]
    color_explicit: bool,
    #[serde(default)]
    revisions: MeasurementRevisions,
    #[serde(default)]
    presentation: MeasurementObjectPresentation,
}

impl<'de> Deserialize<'de> for MeasurementObject {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = MeasurementObjectWire::deserialize(deserializer)?;
        let snapshot = MeasurementObjectSnapshotWire {
            kind: wire.kind,
            entries: wire.entries,
            state: wire.state,
            color_explicit: wire.color_explicit,
            revisions: wire.revisions,
            presentation: wire.presentation,
        }
        .into_snapshot()
        .map_err(serde::de::Error::custom)?;
        Ok(Self::from_snapshot(wire.name, snapshot))
    }
}

fn validate_entries(
    kind: MeasurementKind,
    entries: &[MeasurementEntity],
) -> Result<(), MeasurementEntryError> {
    let expected = kind.anchor_count();
    for entry in entries {
        let actual = entry.anchors.len();
        if actual != expected {
            return Err(MeasurementEntryError { expected, actual });
        }
    }
    Ok(())
}

impl MeasurementObject {
    /// Creates an empty measurement object of one immutable kind.
    pub fn new(name: impl Into<String>, kind: MeasurementKind) -> Self {
        Self {
            name: name.into(),
            kind,
            entries: Vec::new(),
            state: ObjectState::default(),
            color_explicit: false,
            presentation: MeasurementObjectPresentation::default(),
            revisions: MeasurementRevisions::default(),
            dirty: true,
        }
    }

    /// Creates a measurement object from a validated ordered entity list.
    ///
    /// # Errors
    ///
    /// Returns [`MeasurementEntryError`] when any entity has the wrong anchor
    /// count for `kind`.
    pub fn with_entities(
        name: impl Into<String>,
        kind: MeasurementKind,
        entries: Vec<MeasurementEntity>,
    ) -> Result<Self, MeasurementEntryError> {
        validate_entries(kind, &entries)?;
        Ok(Self {
            name: name.into(),
            kind,
            entries,
            state: ObjectState::default(),
            color_explicit: false,
            presentation: MeasurementObjectPresentation::default(),
            revisions: MeasurementRevisions::default(),
            dirty: true,
        })
    }

    /// Restores a measurement object from serialized data.
    pub fn from_snapshot(name: impl Into<String>, snapshot: MeasurementObjectSnapshot) -> Self {
        Self {
            name: name.into(),
            kind: snapshot.kind,
            entries: snapshot.entries,
            state: snapshot.state,
            color_explicit: snapshot.color_explicit,
            presentation: snapshot.presentation,
            revisions: snapshot.revisions,
            dirty: true,
        }
    }

    /// Creates a serializable snapshot.
    pub fn to_snapshot(&self) -> MeasurementObjectSnapshot {
        MeasurementObjectSnapshot {
            kind: self.kind,
            entries: self.entries.clone(),
            state: self.state.clone(),
            color_explicit: self.presentation.color().is_some(),
            revisions: self.revisions,
            presentation: self.presentation,
        }
    }

    /// Returns the immutable measurement kind.
    pub const fn kind(&self) -> MeasurementKind {
        self.kind
    }

    /// Returns stored dynamic entries.
    pub fn entries(&self) -> &[MeasurementEntity] {
        &self.entries
    }

    /// Adds one entry after validating its anchor count.
    ///
    /// # Errors
    ///
    /// Returns [`MeasurementEntryError`] when the entry cardinality differs
    /// from the object's measurement kind.
    pub fn add_entry(&mut self, entry: MeasurementEntity) -> Result<(), MeasurementEntryError> {
        self.add_entries(vec![entry])
    }

    /// Adds entries atomically after validating every anchor count.
    ///
    /// # Errors
    ///
    /// Returns [`MeasurementEntryError`] without mutation when any entry
    /// cardinality differs from the object's measurement kind.
    pub fn add_entries(
        &mut self,
        entries: Vec<MeasurementEntity>,
    ) -> Result<(), MeasurementEntryError> {
        validate_entries(self.kind, &entries)?;
        if entries.is_empty() {
            return Ok(());
        }
        self.entries.extend(entries);
        self.invalidate_all();
        Ok(())
    }

    /// Returns mutable presentation for one entity and invalidates payloads.
    pub fn entity_presentation_mut(
        &mut self,
        index: usize,
    ) -> Option<&mut MeasurementEntityPresentation> {
        if index >= self.entries.len() {
            return None;
        }
        self.invalidate_all();
        self.entries
            .get_mut(index)
            .map(MeasurementEntity::presentation_mut)
    }

    /// Sets label-visibility overrides for selected measurement entities.
    ///
    /// Returns the number of valid target indices and invalidates label data
    /// at most once when an override actually changes.
    pub fn set_entity_labels_visible(
        &mut self,
        indices: impl IntoIterator<Item = usize>,
        visible: bool,
    ) -> usize {
        let mut affected = 0;
        let mut changed = false;
        for index in indices {
            let Some(entity) = self.entries.get_mut(index) else {
                continue;
            };
            affected += 1;
            if entity.presentation.label_visible() != Some(visible) {
                entity.presentation.set_label_visible(visible);
                changed = true;
            }
        }
        if changed {
            self.revisions.labels = self.revisions.labels.saturating_add(1);
            self.dirty = true;
        }
        affected
    }

    /// Removes one entity while preserving remaining order.
    pub fn remove_entry(&mut self, index: usize) -> Option<MeasurementEntity> {
        if index >= self.entries.len() {
            return None;
        }
        let entry = self.entries.remove(index);
        self.invalidate_all();
        Some(entry)
    }

    /// Moves one entity and preserves the resulting order.
    pub fn move_entry(&mut self, from: usize, to: usize) -> bool {
        if from >= self.entries.len() || to >= self.entries.len() || from == to {
            return false;
        }
        let entry = self.entries.remove(from);
        self.entries.insert(to, entry);
        self.invalidate_all();
        true
    }

    /// Returns the number of stored entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns whether no entries are stored.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns render synchronization revisions.
    pub const fn revisions(&self) -> MeasurementRevisions {
        self.revisions
    }

    /// Returns sparse object presentation defaults.
    pub const fn presentation(&self) -> &MeasurementObjectPresentation {
        &self.presentation
    }

    /// Returns mutable object presentation and invalidates derived payloads.
    pub fn presentation_mut(&mut self) -> &mut MeasurementObjectPresentation {
        self.invalidate_all();
        &mut self.presentation
    }

    /// Returns whether object color overrides type-specific color settings.
    pub const fn has_explicit_color(&self) -> bool {
        self.color_explicit || self.presentation.color().is_some()
    }

    /// Returns whether stored entities resolve to more than one color.
    ///
    /// # Errors
    ///
    /// Returns an error when `inherited_color` is atom-dependent.
    pub fn has_mixed_colors(
        &self,
        inherited_color: patinae_color::ColorIndex,
    ) -> Result<bool, AnnotationPresentationError> {
        has_mixed_annotation_colors(
            inherited_color,
            self.presentation.color(),
            self.entries.iter().map(|entry| entry.presentation.color()),
        )
    }

    /// Sets an object color and clears every entity color override.
    ///
    /// # Errors
    ///
    /// Returns an error when `color` is atom-dependent.
    pub fn try_set_color(
        &mut self,
        color: patinae_color::ColorIndex,
    ) -> Result<(), AnnotationPresentationError> {
        self.presentation.set_color(color)?;
        for entry in &mut self.entries {
            entry.presentation.clear_color();
        }
        self.state.color = color;
        self.color_explicit = true;
        self.invalidate_material();
        self.revisions.labels = self.revisions.labels.saturating_add(1);
        Ok(())
    }

    /// Sets a validated named object color for every entity and label.
    ///
    /// # Panics
    ///
    /// Panics when `color` is atom-dependent. Use [`Self::try_set_color`] for
    /// fallible input handling.
    pub fn set_color(&mut self, color: patinae_color::ColorIndex) {
        self.try_set_color(color)
            .expect("annotation object colors must be named");
    }

    /// Marks material data as changed.
    pub fn invalidate_material(&mut self) {
        self.revisions.material = self.revisions.material.saturating_add(1);
        self.dirty = true;
    }

    fn invalidate_all(&mut self) {
        self.revisions.geometry = self.revisions.geometry.saturating_add(1);
        self.revisions.material = self.revisions.material.saturating_add(1);
        self.revisions.labels = self.revisions.labels.saturating_add(1);
        self.dirty = true;
    }

    /// Returns whether render-facing data changed.
    pub const fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Clears the render-facing dirty marker.
    pub fn clear_dirty(&mut self) {
        self.dirty = false;
    }

    pub(crate) fn orphan_anchors_to(&mut self, object_name: &str) -> bool {
        let mut changed = false;
        for anchor in self.entries.iter_mut().flat_map(|entry| &mut entry.anchors) {
            changed |= anchor.orphan_if_source(object_name);
        }
        if changed {
            self.revisions.geometry = self.revisions.geometry.saturating_add(1);
            self.dirty = true;
        }
        changed
    }

    pub(crate) fn remap_anchors(
        &mut self,
        object_name: &str,
        remap: &patinae_mol::AtomRemap,
    ) -> bool {
        let mut changed = false;
        for anchor in self.entries.iter_mut().flat_map(|entry| &mut entry.anchors) {
            changed |= anchor.remap_if_source(object_name, remap);
        }
        if changed {
            self.revisions.geometry = self.revisions.geometry.saturating_add(1);
            self.dirty = true;
        }
        changed
    }

    pub(crate) fn rename_anchors_to(&mut self, old_name: &str, new_name: &str) -> bool {
        let mut changed = false;
        for anchor in self.entries.iter_mut().flat_map(|entry| &mut entry.anchors) {
            changed |= anchor.rename_source(old_name, new_name);
        }
        if changed {
            self.revisions.geometry = self.revisions.geometry.saturating_add(1);
            self.dirty = true;
        }
        changed
    }
}

impl Object for MeasurementObject {
    fn name(&self) -> &str {
        &self.name
    }

    fn object_type(&self) -> ObjectType {
        ObjectType::Measurement
    }

    fn state(&self) -> &ObjectState {
        &self.state
    }

    fn state_mut(&mut self) -> &mut ObjectState {
        &mut self.state
    }

    fn extent(&self) -> Option<(Vec3, Vec3)> {
        None
    }

    fn set_name(&mut self, name: String) {
        self.name = name;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use patinae_mol::{Atom, AtomIndex, CoordSet, Element, ObjectMolecule};

    use crate::{MoleculeObject, ObjectRegistry};

    fn molecule(name: &str, points: &[Vec3]) -> MoleculeObject {
        let mut molecule = ObjectMolecule::new(name);
        for _ in points {
            molecule.add_atom(Atom::new("C", Element::Carbon));
        }
        molecule.add_coord_set(CoordSet::from_vec3(points));
        MoleculeObject::with_name(molecule, name)
    }

    fn anchor(registry: &ObjectRegistry, object_name: &str, index: u32) -> MeasurementAnchor {
        assert!(registry.get_molecule(object_name).is_some());
        MeasurementAnchor::new(object_name, AtomIndex(index))
    }

    #[test]
    fn object_rejects_wrong_entry_cardinality() {
        let mut object = MeasurementObject::new("d", MeasurementKind::Distance);
        let mut registry = ObjectRegistry::new();
        registry.add(molecule(
            "m",
            &[Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)],
        ));
        let error = object
            .add_entries(vec![
                MeasurementEntry::new(vec![anchor(&registry, "m", 0), anchor(&registry, "m", 1)]),
                MeasurementEntry::new(vec![anchor(&registry, "m", 0)]),
            ])
            .unwrap_err();
        assert_eq!(error.expected(), 2);
        assert_eq!(error.actual(), 1);
        assert!(object.is_empty());
    }

    #[test]
    fn batch_label_visibility_only_invalidates_labels_once() {
        let mut registry = ObjectRegistry::new();
        registry.add(molecule(
            "m",
            &[
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(2.0, 0.0, 0.0),
            ],
        ));
        let mut object = MeasurementObject::with_entities(
            "distances",
            MeasurementKind::Distance,
            vec![
                MeasurementEntity::new(vec![anchor(&registry, "m", 0), anchor(&registry, "m", 1)]),
                MeasurementEntity::new(vec![anchor(&registry, "m", 1), anchor(&registry, "m", 2)]),
            ],
        )
        .unwrap();
        let before = object.revisions();

        assert_eq!(object.set_entity_labels_visible([0, 1], false), 2);
        let changed = object.revisions();
        assert_eq!(changed.geometry, before.geometry);
        assert_eq!(changed.material, before.material);
        assert_eq!(changed.labels, before.labels + 1);

        assert_eq!(object.set_entity_labels_visible([0, 1], false), 2);
        assert_eq!(object.revisions(), changed);
    }

    #[test]
    fn source_rename_updates_anchors_and_extent() {
        let mut registry = ObjectRegistry::new();
        registry.add(molecule(
            "source",
            &[Vec3::new(0.0, 0.0, 0.0), Vec3::new(4.0, 0.0, 0.0)],
        ));
        let anchors = vec![
            anchor(&registry, "source", 0),
            anchor(&registry, "source", 1),
        ];
        let mut measurement = MeasurementObject::new("distance", MeasurementKind::Distance);
        measurement
            .add_entry(MeasurementEntry::new(anchors))
            .unwrap();
        registry.add(measurement);

        registry.rename("source", "renamed").unwrap();

        let measurement = registry.get_measurement("distance").unwrap();
        assert!(measurement.entries()[0]
            .anchors
            .iter()
            .all(|anchor| anchor.object_name == "renamed"));
        let (min, max) = registry.object_extent("distance").unwrap();
        assert_eq!(min, Vec3::new(0.0, 0.0, 0.0));
        assert_eq!(max, Vec3::new(4.0, 0.0, 0.0));
    }

    #[test]
    fn removed_source_does_not_rebind_to_same_name() {
        let mut registry = ObjectRegistry::new();
        registry.add(molecule(
            "source",
            &[Vec3::new(0.0, 0.0, 0.0), Vec3::new(4.0, 0.0, 0.0)],
        ));
        let anchors = vec![
            anchor(&registry, "source", 0),
            anchor(&registry, "source", 1),
        ];
        let mut measurement = MeasurementObject::new("distance", MeasurementKind::Distance);
        measurement
            .add_entry(MeasurementEntry::new(anchors))
            .unwrap();
        registry.add(measurement);

        registry.remove("source");
        registry.add(molecule(
            "source",
            &[Vec3::new(0.0, 0.0, 0.0), Vec3::new(8.0, 0.0, 0.0)],
        ));

        let measurement = registry.get_measurement("distance").unwrap();
        assert!(measurement.entries()[0]
            .anchors
            .iter()
            .all(MeasurementAnchor::is_orphaned));
        assert!(registry.object_extent("distance").is_none());
    }

    #[test]
    fn removed_atom_leaves_entry_unresolved() {
        let mut registry = ObjectRegistry::new();
        registry.add(molecule(
            "source",
            &[Vec3::new(0.0, 0.0, 0.0), Vec3::new(4.0, 0.0, 0.0)],
        ));
        let anchors = vec![
            anchor(&registry, "source", 0),
            anchor(&registry, "source", 1),
        ];
        let mut measurement = MeasurementObject::new("distance", MeasurementKind::Distance);
        measurement
            .add_entry(MeasurementEntry::new(anchors))
            .unwrap();
        registry.add(measurement);

        registry
            .remove_molecule_atoms("source", &[AtomIndex(1)])
            .unwrap();

        assert!(registry.object_extent("distance").is_none());
    }

    #[test]
    fn all_measurement_kinds_roundtrip_with_valid_cardinality() {
        let mut registry = ObjectRegistry::new();
        registry.add(molecule(
            "source",
            &[
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(1.0, 1.0, 0.0),
                Vec3::new(1.0, 1.0, 1.0),
            ],
        ));

        for kind in [
            MeasurementKind::Distance,
            MeasurementKind::Angle,
            MeasurementKind::Dihedral,
        ] {
            let anchors = (0..kind.anchor_count() as u32)
                .map(|index| anchor(&registry, "source", index))
                .collect();
            let object = MeasurementObject::with_entities(
                format!("{kind:?}"),
                kind,
                vec![MeasurementEntity::new(anchors)],
            )
            .expect("valid cardinality");

            let json = serde_json::to_string(&object).expect("serialize measurement");
            let restored: MeasurementObject =
                serde_json::from_str(&json).expect("restore measurement");

            assert_eq!(restored.kind(), kind);
            assert_eq!(restored.entries()[0].anchors.len(), kind.anchor_count());
        }
    }

    #[test]
    fn deserialization_rejects_wrong_measurement_cardinality() {
        let mut registry = ObjectRegistry::new();
        registry.add(molecule("source", &[Vec3::new(0.0, 0.0, 0.0)]));
        let invalid = MeasurementObject {
            name: "distance".into(),
            kind: MeasurementKind::Distance,
            entries: vec![MeasurementEntity::new(vec![anchor(&registry, "source", 0)])],
            state: ObjectState::default(),
            color_explicit: false,
            presentation: MeasurementObjectPresentation::default(),
            revisions: MeasurementRevisions::default(),
            dirty: true,
        };
        let json = serde_json::to_string(&invalid).expect("serialize invalid fixture");

        let error = serde_json::from_str::<MeasurementObject>(&json)
            .expect_err("invalid cardinality must fail");

        assert!(error
            .to_string()
            .contains("measurement requires 2 anchors, got 1"));
    }

    #[test]
    fn reordered_measurements_roundtrip_without_entity_keys() {
        let mut registry = ObjectRegistry::new();
        registry.add(molecule(
            "source",
            &[
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(2.0, 0.0, 0.0),
                Vec3::new(3.0, 0.0, 0.0),
            ],
        ));
        let mut object = MeasurementObject::with_entities(
            "distances",
            MeasurementKind::Distance,
            vec![
                MeasurementEntity::new(vec![
                    anchor(&registry, "source", 0),
                    anchor(&registry, "source", 1),
                ]),
                MeasurementEntity::new(vec![
                    anchor(&registry, "source", 1),
                    anchor(&registry, "source", 2),
                ]),
                MeasurementEntity::new(vec![
                    anchor(&registry, "source", 2),
                    anchor(&registry, "source", 3),
                ]),
            ],
        )
        .expect("distance cardinality");
        object.remove_entry(1).expect("middle entity");
        assert!(object.move_entry(1, 0));

        let json = serde_json::to_string(&object).expect("serialize reordered object");
        let restored: MeasurementObject =
            serde_json::from_str(&json).expect("restore reordered object");

        assert_eq!(restored.entries().len(), 2);
        assert_eq!(
            restored.entries()[0].anchors[0].atom_index,
            anchor(&registry, "source", 2).atom_index
        );
        assert_eq!(
            restored.entries()[1].anchors[0].atom_index,
            anchor(&registry, "source", 0).atom_index
        );
    }

    #[test]
    fn object_color_clears_overrides_and_mixed_state() {
        let mut registry = ObjectRegistry::new();
        registry.add(molecule(
            "source",
            &[
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(2.0, 0.0, 0.0),
            ],
        ));
        let mut override_presentation = MeasurementEntityPresentation::default();
        override_presentation
            .set_color(patinae_color::ColorIndex::Named(2))
            .expect("entity color");
        let mut object = MeasurementObject::with_entities(
            "distances",
            MeasurementKind::Distance,
            vec![
                MeasurementEntity::new(vec![
                    anchor(&registry, "source", 0),
                    anchor(&registry, "source", 1),
                ]),
                MeasurementEntity::with_presentation(
                    vec![
                        anchor(&registry, "source", 1),
                        anchor(&registry, "source", 2),
                    ],
                    override_presentation,
                ),
            ],
        )
        .expect("distance cardinality");
        assert!(object
            .has_mixed_colors(patinae_color::ColorIndex::Named(1))
            .expect("named inherited color"));

        object.set_color(patinae_color::ColorIndex::Named(7));

        assert!(!object
            .has_mixed_colors(patinae_color::ColorIndex::Named(1))
            .expect("named inherited color"));
        assert!(object
            .entries()
            .iter()
            .all(|entry| entry.presentation().color().is_none()));
    }

    #[test]
    fn registry_snapshot_preserves_measurement_lifecycle_data() {
        let mut registry = ObjectRegistry::new();
        registry.add(molecule(
            "source",
            &[Vec3::new(0.0, 0.0, 0.0), Vec3::new(4.0, 0.0, 0.0)],
        ));
        let anchors = vec![
            anchor(&registry, "source", 0),
            anchor(&registry, "source", 1),
        ];
        let mut measurement = MeasurementObject::new("distance", MeasurementKind::Distance);
        measurement
            .add_entry(MeasurementEntry::new(anchors))
            .unwrap();
        measurement.set_color(patinae_color::ColorIndex::Named(6));
        registry.add(measurement);
        assert!(registry.add_to_group("measurements", "distance"));
        let render_id = registry.render_id("distance");

        let restored = ObjectRegistry::from_snapshot(registry.to_snapshot());
        let measurement = restored.get_measurement("distance").unwrap();

        assert_eq!(measurement.kind(), MeasurementKind::Distance);
        assert_eq!(measurement.len(), 1);
        assert_eq!(
            measurement.state().color,
            patinae_color::ColorIndex::Named(6)
        );
        assert!(measurement.has_explicit_color());
        assert_eq!(restored.render_id("distance"), render_id);
        assert_eq!(restored.parent_group("distance"), Some("measurements"));
        assert!(restored.object_extent("distance").is_some());
    }
}
