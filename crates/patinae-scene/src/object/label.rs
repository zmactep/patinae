//! Atom-anchored label collections.
//!
//! Label entities store semantic anchors and text. World positions and overlay
//! primitives are derived by the annotation resolver.

use patinae_color::{ColorIndex, NamedPalette};
use patinae_mol::AtomRemap;
use patinae_settings::Settings;
use serde::{Deserialize, Serialize};

use super::{
    annotation::has_mixed_annotation_colors, AnnotationPresentationError, AtomAnchor,
    LabelAlignment, LabelEntityPresentation, LabelObjectPresentation, LabelPresentation, Object,
    ObjectRegistry, ObjectState, ObjectType,
};

const DEFAULT_LABEL_SIZE: f32 = 14.0;
const DEFAULT_LABEL_COLOR: [f32; 3] = [
    0x22 as f32 / 255.0,
    0xD3 as f32 / 255.0,
    0xEE as f32 / 255.0,
];

/// Stores one atom-anchored text label.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LabelEntity {
    anchor: AtomAnchor,
    text: String,
    #[serde(default)]
    presentation: LabelEntityPresentation,
}

impl LabelEntity {
    /// Creates a label with no entity-level presentation overrides.
    pub fn new(anchor: AtomAnchor, text: impl Into<String>) -> Self {
        Self {
            anchor,
            text: text.into(),
            presentation: LabelEntityPresentation::default(),
        }
    }

    /// Creates a label with explicit sparse presentation overrides.
    pub fn with_presentation(
        anchor: AtomAnchor,
        text: impl Into<String>,
        presentation: LabelEntityPresentation,
    ) -> Self {
        Self {
            anchor,
            text: text.into(),
            presentation,
        }
    }

    /// Returns the dynamic atom anchor.
    pub const fn anchor(&self) -> &AtomAnchor {
        &self.anchor
    }

    /// Returns the stored label text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Replaces the stored label text.
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
    }

    /// Returns sparse entity presentation overrides.
    pub const fn presentation(&self) -> &LabelEntityPresentation {
        &self.presentation
    }

    /// Returns mutable sparse presentation overrides.
    pub fn presentation_mut(&mut self) -> &mut LabelEntityPresentation {
        &mut self.presentation
    }

    /// Resolves entity, object, and global presentation precedence.
    pub fn resolve_presentation(
        &self,
        object: &LabelObjectPresentation,
        global: LabelPresentation,
    ) -> LabelPresentation {
        self.presentation.resolve(object, global)
    }

    pub(crate) fn anchor_mut(&mut self) -> &mut AtomAnchor {
        &mut self.anchor
    }
}

/// Backward-compatible type name for one label entity.
#[deprecated(note = "use LabelEntity; world-position labels are no longer stored")]
pub type Label = LabelEntity;

/// Backward-compatible name for text alignment.
#[deprecated(note = "renamed to LabelAlignment")]
pub type LabelAnchor = LabelAlignment;

/// Tracks label-object changes for derived annotation caches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelRevisions {
    /// Changes when anchors or collection order changes.
    pub geometry: u64,
    /// Changes when colors or sizes change.
    pub material: u64,
    /// Changes when text or visibility changes.
    pub labels: u64,
}

/// Describes one atom anchor for external label inspection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelAnchorView {
    /// Name of the source molecule object.
    pub object_name: String,
    /// Current zero-based source atom index.
    pub atom_index: u32,
    /// Whether the source was permanently removed.
    pub orphaned: bool,
    /// Whether the anchor currently resolves to displayed coordinates.
    pub resolved: bool,
}

/// Describes one label entity for external inspection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LabelEntityView {
    /// Dynamic source-atom anchor.
    pub anchor: LabelAnchorView,
    /// Stored label text.
    pub text: String,
    /// Effective RGB color after override resolution.
    pub color: [f32; 3],
    /// Optional named-palette override index.
    pub color_override_index: Option<u32>,
    /// Effective font size in device-independent pixels.
    pub size: f32,
    /// Optional entity-level size override.
    pub size_override: Option<f32>,
    /// Effective entity visibility before owner and group visibility.
    pub visible: bool,
    /// Optional entity-level visibility override.
    pub visible_override: Option<bool>,
}

/// Describes one label object for external inspection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LabelObjectView {
    /// Registry-owned object name.
    pub name: String,
    /// Common object enabled state.
    pub enabled: bool,
    /// Effective object-default RGB color.
    pub color: [f32; 3],
    /// Optional object-level named-palette override index.
    pub color_override_index: Option<u32>,
    /// Effective object-default font size.
    pub size: f32,
    /// Optional object-level size override.
    pub size_override: Option<f32>,
    /// Effective object-default label visibility.
    pub visible: bool,
    /// Optional object-level visibility override.
    pub visible_override: Option<bool>,
    /// Effective kebab-case text alignment.
    pub alignment: String,
    /// Optional object-level kebab-case alignment override.
    pub alignment_override: Option<String>,
    /// Labels in deterministic insertion order.
    pub entities: Vec<LabelEntityView>,
    /// Number of entities whose anchors do not currently resolve.
    pub unresolved_count: usize,
    /// Revisions represented by this view.
    pub revisions: LabelRevisions,
}

/// Builds a portable, read-only view of one label object.
///
/// The returned values contain both stored overrides and effective
/// presentation, so external APIs do not need to reproduce precedence rules.
pub fn label_object_view(
    registry: &ObjectRegistry,
    settings: &Settings,
    named_palette: &NamedPalette,
    name: &str,
) -> Option<LabelObjectView> {
    let label = registry.get_label(name)?;
    let object_color_override = label.presentation().color();
    let fallback_color = label_default_color(named_palette);
    let object_color = resolve_label_color(object_color_override, named_palette, fallback_color);
    let global_size = valid_label_size(settings.measurement.label_size);
    let object_size = valid_label_size(label.presentation().size().unwrap_or(global_size));
    let object_visible = label.presentation().visible().unwrap_or(true);
    let object_alignment = label.presentation().alignment().unwrap_or_default();
    let mut unresolved_count = 0;

    let entities = label
        .entities()
        .iter()
        .map(|entity| {
            let anchor = entity.anchor();
            let resolved = !anchor.is_orphaned()
                && registry
                    .get_molecule(&anchor.object_name)
                    .and_then(|molecule| molecule.display_coord(anchor.atom_index))
                    .is_some();
            if !resolved {
                unresolved_count += 1;
            }

            let color_override = entity.presentation().color();
            let size_override = entity.presentation().size();
            let visible_override = entity.presentation().visible();
            LabelEntityView {
                anchor: LabelAnchorView {
                    object_name: anchor.object_name.clone(),
                    atom_index: anchor.atom_index.0,
                    orphaned: anchor.is_orphaned(),
                    resolved,
                },
                text: entity.text().to_string(),
                color: resolve_label_color(color_override, named_palette, object_color),
                color_override_index: named_color_index(color_override),
                size: valid_label_size(size_override.unwrap_or(object_size)),
                size_override,
                visible: visible_override.unwrap_or(object_visible),
                visible_override,
            }
        })
        .collect();

    Some(LabelObjectView {
        name: name.to_string(),
        enabled: label.is_enabled(),
        color: object_color,
        color_override_index: named_color_index(object_color_override),
        size: object_size,
        size_override: label.presentation().size(),
        visible: object_visible,
        visible_override: label.presentation().visible(),
        alignment: object_alignment.as_str().to_string(),
        alignment_override: label
            .presentation()
            .alignment()
            .map(|alignment| alignment.as_str().to_string()),
        entities,
        unresolved_count,
        revisions: label.revisions(),
    })
}

fn label_default_color(named_palette: &NamedPalette) -> [f32; 3] {
    named_palette
        .get_by_name("cyan")
        .map(|(_, color)| [color.r, color.g, color.b])
        .unwrap_or(DEFAULT_LABEL_COLOR)
}

fn resolve_label_color(
    color: Option<ColorIndex>,
    named_palette: &NamedPalette,
    fallback: [f32; 3],
) -> [f32; 3] {
    let Some(ColorIndex::Named(index)) = color else {
        return fallback;
    };
    named_palette
        .get_by_index(index)
        .map(|color| [color.r, color.g, color.b])
        .unwrap_or(fallback)
}

const fn named_color_index(color: Option<ColorIndex>) -> Option<u32> {
    match color {
        Some(ColorIndex::Named(index)) => Some(index),
        _ => None,
    }
}

fn valid_label_size(size: f32) -> f32 {
    if size.is_finite() && size > 0.0 {
        size
    } else {
        DEFAULT_LABEL_SIZE
    }
}

impl Default for LabelRevisions {
    fn default() -> Self {
        Self {
            geometry: 1,
            material: 1,
            labels: 1,
        }
    }
}

/// Contains an ordered collection of atom-anchored labels.
#[derive(Debug, Serialize, Deserialize)]
pub struct LabelObject {
    name: String,
    state: ObjectState,
    #[serde(default)]
    entities: Vec<LabelEntity>,
    #[serde(default)]
    presentation: LabelObjectPresentation,
    #[serde(default)]
    revisions: LabelRevisions,
    #[serde(skip, default = "default_dirty_true")]
    dirty: bool,
}

/// Serializable label-object data without the registry-owned name.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabelObjectSnapshot {
    /// Common object state.
    state: ObjectState,
    /// Labels in deterministic insertion order.
    entities: Vec<LabelEntity>,
    /// Sparse object presentation defaults.
    #[serde(default)]
    presentation: LabelObjectPresentation,
    /// Revisions used to rebuild derived annotation data.
    #[serde(default)]
    revisions: LabelRevisions,
}

impl LabelObjectSnapshot {
    /// Creates a label-object snapshot from validated annotation values.
    pub fn new(
        entities: Vec<LabelEntity>,
        mut state: ObjectState,
        presentation: LabelObjectPresentation,
        revisions: LabelRevisions,
    ) -> Self {
        if let Some(color) = presentation.color() {
            state.color = color;
        }
        Self {
            state,
            entities,
            presentation,
            revisions,
        }
    }

    /// Returns stored entities in serialized order.
    pub fn entities(&self) -> &[LabelEntity] {
        &self.entities
    }

    /// Returns common object state.
    pub const fn state(&self) -> &ObjectState {
        &self.state
    }

    /// Returns sparse object presentation defaults.
    pub const fn presentation(&self) -> &LabelObjectPresentation {
        &self.presentation
    }

    /// Returns render synchronization revisions.
    pub const fn revisions(&self) -> LabelRevisions {
        self.revisions
    }
}

fn default_dirty_true() -> bool {
    true
}

impl LabelObject {
    /// Creates an empty label collection.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            state: ObjectState::default(),
            entities: Vec::new(),
            presentation: LabelObjectPresentation::default(),
            revisions: LabelRevisions::default(),
            dirty: true,
        }
    }

    /// Creates a label collection preserving the supplied entity order.
    pub fn with_entities(name: impl Into<String>, entities: Vec<LabelEntity>) -> Self {
        Self {
            name: name.into(),
            state: ObjectState::default(),
            entities,
            presentation: LabelObjectPresentation::default(),
            revisions: LabelRevisions::default(),
            dirty: true,
        }
    }

    /// Restores a label object from serialized registry data.
    pub fn from_snapshot(name: impl Into<String>, snapshot: LabelObjectSnapshot) -> Self {
        Self {
            name: name.into(),
            state: snapshot.state,
            entities: snapshot.entities,
            presentation: snapshot.presentation,
            revisions: snapshot.revisions,
            dirty: true,
        }
    }

    /// Creates serializable registry data for this object.
    pub fn to_snapshot(&self) -> LabelObjectSnapshot {
        LabelObjectSnapshot::new(
            self.entities.clone(),
            self.state.clone(),
            self.presentation,
            self.revisions,
        )
    }

    /// Creates a label collection preserving the supplied entity order.
    #[deprecated(note = "renamed to with_entities")]
    pub fn with_labels(name: impl Into<String>, labels: Vec<LabelEntity>) -> Self {
        Self::with_entities(name, labels)
    }

    /// Returns label entities in deterministic insertion order.
    pub fn entities(&self) -> &[LabelEntity] {
        &self.entities
    }

    /// Returns label entities in deterministic insertion order.
    #[deprecated(note = "renamed to entities")]
    pub fn labels(&self) -> &[LabelEntity] {
        self.entities()
    }

    /// Returns one mutable entity and invalidates all derived payloads.
    pub fn entity_mut(&mut self, index: usize) -> Option<&mut LabelEntity> {
        if index >= self.entities.len() {
            return None;
        }
        self.invalidate_all();
        self.entities.get_mut(index)
    }

    /// Appends one entity without deduplicating its atom anchor.
    pub fn add_entity(&mut self, entity: LabelEntity) {
        self.entities.push(entity);
        self.invalidate_all();
    }

    /// Appends entities in order and invalidates derived data once.
    pub fn extend_entities(&mut self, entities: impl IntoIterator<Item = LabelEntity>) {
        let old_len = self.entities.len();
        self.entities.extend(entities);
        if self.entities.len() != old_len {
            self.invalidate_all();
        }
    }

    /// Sets visibility overrides for selected entities.
    ///
    /// Returns the number of valid target indices. Repeated indices count as
    /// repeated targets but derived label data is invalidated at most once.
    pub fn set_entities_visible(
        &mut self,
        indices: impl IntoIterator<Item = usize>,
        visible: bool,
    ) -> usize {
        let mut affected = 0;
        let mut changed = false;
        for index in indices {
            let Some(entity) = self.entities.get_mut(index) else {
                continue;
            };
            affected += 1;
            if entity.presentation.visible() != Some(visible) {
                entity.presentation.set_visible(visible);
                changed = true;
            }
        }
        if changed {
            self.invalidate_labels();
        }
        affected
    }

    /// Appends one entity without deduplicating its atom anchor.
    #[deprecated(note = "renamed to add_entity")]
    pub fn add_label(&mut self, label: LabelEntity) {
        self.add_entity(label);
    }

    /// Removes one entity while preserving remaining order.
    pub fn remove_entity(&mut self, index: usize) -> Option<LabelEntity> {
        if index >= self.entities.len() {
            return None;
        }
        let entity = self.entities.remove(index);
        self.invalidate_all();
        Some(entity)
    }

    /// Removes one entity while preserving remaining order.
    #[deprecated(note = "renamed to remove_entity")]
    pub fn remove_label(&mut self, index: usize) -> Option<LabelEntity> {
        self.remove_entity(index)
    }

    /// Moves one entity and preserves the resulting order.
    pub fn move_entity(&mut self, from: usize, to: usize) -> bool {
        if from >= self.entities.len() || to >= self.entities.len() || from == to {
            return false;
        }
        let entity = self.entities.remove(from);
        self.entities.insert(to, entity);
        self.invalidate_all();
        true
    }

    /// Removes every entity without deleting the semantic object.
    pub fn clear(&mut self) {
        if self.entities.is_empty() {
            return;
        }
        self.entities.clear();
        self.invalidate_all();
    }

    /// Returns the number of stored label entities.
    pub fn len(&self) -> usize {
        self.entities.len()
    }

    /// Returns whether the collection has no entities.
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    /// Returns sparse object presentation defaults.
    pub const fn presentation(&self) -> &LabelObjectPresentation {
        &self.presentation
    }

    /// Returns mutable object presentation and invalidates derived payloads.
    pub fn presentation_mut(&mut self) -> &mut LabelObjectPresentation {
        self.invalidate_all();
        &mut self.presentation
    }

    /// Sets an object color and clears every entity color override.
    ///
    /// # Errors
    ///
    /// Returns an error when `color` is atom-dependent.
    pub fn try_set_color(&mut self, color: ColorIndex) -> Result<(), AnnotationPresentationError> {
        self.presentation.set_color(color)?;
        self.state.color = color;
        for entity in &mut self.entities {
            entity.presentation.clear_color();
        }
        self.invalidate_material_and_labels();
        Ok(())
    }

    /// Sets a validated named object color.
    ///
    /// # Panics
    ///
    /// Panics when `color` is atom-dependent. Use [`Self::try_set_color`] for
    /// fallible input handling.
    pub fn set_color(&mut self, color: ColorIndex) {
        self.try_set_color(color)
            .expect("annotation object colors must be named");
    }

    /// Returns whether stored entities resolve to more than one color.
    ///
    /// # Errors
    ///
    /// Returns an error when `inherited_color` is atom-dependent.
    pub fn has_mixed_colors(
        &self,
        inherited_color: ColorIndex,
    ) -> Result<bool, AnnotationPresentationError> {
        has_mixed_annotation_colors(
            inherited_color,
            self.presentation.color(),
            self.entities
                .iter()
                .map(|entity| entity.presentation.color()),
        )
    }

    /// Returns the shared alignment override.
    #[deprecated(note = "use presentation().alignment()")]
    pub fn anchor(&self) -> LabelAlignment {
        self.presentation.alignment().unwrap_or_default()
    }

    /// Sets the shared alignment override.
    #[deprecated(note = "use presentation_mut().set_alignment()")]
    pub fn set_anchor(&mut self, alignment: LabelAlignment) {
        self.presentation.set_alignment(alignment);
        self.invalidate_labels();
    }

    /// Returns revisions used by derived annotation caches.
    pub const fn revisions(&self) -> LabelRevisions {
        self.revisions
    }

    /// Returns whether render-facing label data changed.
    pub const fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Clears the render-facing dirty marker.
    pub fn clear_dirty(&mut self) {
        self.dirty = false;
    }

    /// Marks every derived label payload as stale.
    pub fn invalidate(&mut self) {
        self.invalidate_all();
    }

    pub(crate) fn orphan_anchors_to(&mut self, object_name: &str) -> bool {
        let mut changed = false;
        for entity in &mut self.entities {
            changed |= entity.anchor_mut().orphan_if_source(object_name);
        }
        if changed {
            self.invalidate_all();
        }
        changed
    }

    pub(crate) fn remap_anchors(&mut self, object_name: &str, remap: &AtomRemap) -> bool {
        let mut changed = false;
        for entity in &mut self.entities {
            changed |= entity.anchor_mut().remap_if_source(object_name, remap);
        }
        if changed {
            self.invalidate_all();
        }
        changed
    }

    pub(crate) fn rename_anchors_to(&mut self, old_name: &str, new_name: &str) -> bool {
        let mut changed = false;
        for entity in &mut self.entities {
            changed |= entity.anchor_mut().rename_source(old_name, new_name);
        }
        if changed {
            self.invalidate_all();
        }
        changed
    }

    fn invalidate_all(&mut self) {
        self.revisions.geometry = self.revisions.geometry.saturating_add(1);
        self.revisions.material = self.revisions.material.saturating_add(1);
        self.revisions.labels = self.revisions.labels.saturating_add(1);
        self.dirty = true;
    }

    fn invalidate_material_and_labels(&mut self) {
        self.revisions.material = self.revisions.material.saturating_add(1);
        self.revisions.labels = self.revisions.labels.saturating_add(1);
        self.dirty = true;
    }

    fn invalidate_labels(&mut self) {
        self.revisions.labels = self.revisions.labels.saturating_add(1);
        self.dirty = true;
    }
}

impl Object for LabelObject {
    fn name(&self) -> &str {
        &self.name
    }

    fn object_type(&self) -> ObjectType {
        ObjectType::Label
    }

    fn state(&self) -> &ObjectState {
        &self.state
    }

    fn state_mut(&mut self) -> &mut ObjectState {
        &mut self.state
    }

    fn extent(&self) -> Option<(lin_alg::f32::Vec3, lin_alg::f32::Vec3)> {
        None
    }

    fn set_name(&mut self, name: String) {
        self.name = name;
    }
}

#[cfg(test)]
mod tests {
    use patinae_color::NamedPalette;
    use patinae_mol::AtomIndex;
    use patinae_settings::Settings;

    use super::*;

    fn anchor(index: u32) -> AtomAnchor {
        AtomAnchor::new("protein", AtomIndex(index - 1))
    }

    #[test]
    fn collection_preserves_duplicate_anchors_and_insertion_order() {
        let shared = anchor(1);
        let object = LabelObject::with_entities(
            "labels",
            vec![
                LabelEntity::new(shared.clone(), "first"),
                LabelEntity::new(anchor(2), "middle"),
                LabelEntity::new(shared, "last"),
            ],
        );

        let texts = object
            .entities()
            .iter()
            .map(LabelEntity::text)
            .collect::<Vec<_>>();
        assert_eq!(texts, ["first", "middle", "last"]);
        assert_eq!(object.entities()[0].anchor(), object.entities()[2].anchor());
    }

    #[test]
    fn batch_visibility_invalidates_labels_once_and_skips_noop_updates() {
        let mut object = LabelObject::with_entities(
            "labels",
            vec![
                LabelEntity::new(anchor(1), "first"),
                LabelEntity::new(anchor(2), "second"),
            ],
        );
        let before = object.revisions();

        assert_eq!(object.set_entities_visible([0, 1], false), 2);
        let changed = object.revisions();
        assert_eq!(changed.geometry, before.geometry);
        assert_eq!(changed.material, before.material);
        assert_eq!(changed.labels, before.labels + 1);

        assert_eq!(object.set_entities_visible([0, 1], false), 2);
        assert_eq!(object.revisions(), changed);
    }

    #[test]
    fn portable_view_exposes_effective_and_override_values() {
        let palette = NamedPalette::default();
        let red = palette.get_by_name("red").unwrap().0;
        let mut presentation = LabelEntityPresentation::default();
        presentation.set_color(ColorIndex::Named(red)).unwrap();
        presentation.set_size(20.0).unwrap();
        presentation.set_visible(false);
        let entity = LabelEntity::with_presentation(
            AtomAnchor::new("missing", AtomIndex(4)),
            "ALA5",
            presentation,
        );
        let mut object = LabelObject::with_entities("labels", vec![entity]);
        object.presentation_mut().set_size(18.0).unwrap();
        object
            .presentation_mut()
            .set_alignment(LabelAlignment::Center);
        let mut registry = ObjectRegistry::new();
        registry.add(object);

        let view = label_object_view(&registry, &Settings::default(), &palette, "labels").unwrap();

        assert_eq!(view.name, "labels");
        assert_eq!(view.size, 18.0);
        assert_eq!(view.alignment, "center");
        assert_eq!(view.unresolved_count, 1);
        assert_eq!(view.entities[0].text, "ALA5");
        assert_eq!(view.entities[0].color_override_index, Some(red));
        assert_eq!(view.entities[0].size, 20.0);
        assert!(!view.entities[0].visible);
        assert!(!view.entities[0].anchor.resolved);
    }

    #[test]
    fn bulk_append_invalidates_collection_once() {
        let mut object = LabelObject::new("labels");
        let before = object.revisions();

        object.extend_entities([
            LabelEntity::new(anchor(1), "first"),
            LabelEntity::new(anchor(2), "second"),
        ]);

        assert_eq!(object.len(), 2);
        assert_eq!(object.revisions().geometry, before.geometry + 1);
        assert_eq!(object.revisions().material, before.material + 1);
        assert_eq!(object.revisions().labels, before.labels + 1);
    }

    #[test]
    fn remove_and_reorder_do_not_manufacture_entity_identity() {
        let mut object = LabelObject::with_entities(
            "labels",
            vec![
                LabelEntity::new(anchor(1), "one"),
                LabelEntity::new(anchor(2), "two"),
                LabelEntity::new(anchor(3), "three"),
            ],
        );

        assert_eq!(
            object.remove_entity(1).map(|entity| entity.text),
            Some("two".into())
        );
        assert!(object.move_entity(1, 0));

        let texts = object
            .entities()
            .iter()
            .map(LabelEntity::text)
            .collect::<Vec<_>>();
        assert_eq!(texts, ["three", "one"]);
    }

    #[test]
    fn object_color_clears_entity_color_overrides() {
        let mut presentation = LabelEntityPresentation::default();
        presentation
            .set_color(ColorIndex::Named(2))
            .expect("entity color");
        let mut object = LabelObject::with_entities(
            "labels",
            vec![LabelEntity::with_presentation(
                anchor(1),
                "CA",
                presentation,
            )],
        );

        object.set_color(ColorIndex::Named(7));

        assert_eq!(object.presentation().color(), Some(ColorIndex::Named(7)));
        assert_eq!(object.entities()[0].presentation().color(), None);
    }

    #[test]
    fn semantic_labels_have_no_stored_world_extent() {
        let object = LabelObject::with_entities("labels", vec![LabelEntity::new(anchor(1), "CA")]);

        assert!(object.extent().is_none());
    }

    #[test]
    fn snapshot_constructor_exposes_read_only_state_and_normalizes_color() {
        let mut presentation = LabelObjectPresentation::default();
        presentation
            .set_color(ColorIndex::Named(7))
            .expect("object color");
        let revisions = LabelRevisions {
            geometry: 2,
            material: 3,
            labels: 4,
        };
        let snapshot = LabelObjectSnapshot::new(
            vec![LabelEntity::new(anchor(1), "CA")],
            ObjectState::default(),
            presentation,
            revisions,
        );

        assert_eq!(snapshot.entities()[0].text(), "CA");
        assert_eq!(snapshot.state().color, ColorIndex::Named(7));
        assert_eq!(snapshot.presentation().color(), Some(ColorIndex::Named(7)));
        assert_eq!(snapshot.revisions(), revisions);
    }

    #[test]
    fn roundtrip_preserves_duplicates_presentation_and_current_order() {
        let shared = anchor(1);
        let mut first_presentation = LabelEntityPresentation::default();
        first_presentation
            .set_color(ColorIndex::Named(2))
            .expect("first color");
        first_presentation.set_size(11.0).expect("first size");
        let mut last_presentation = LabelEntityPresentation::default();
        last_presentation
            .set_color(ColorIndex::Named(3))
            .expect("last color");
        last_presentation.set_size(18.0).expect("last size");
        let mut object = LabelObject::with_entities(
            "labels",
            vec![
                LabelEntity::with_presentation(shared.clone(), "first", first_presentation),
                LabelEntity::new(anchor(2), "removed"),
                LabelEntity::with_presentation(shared, "last", last_presentation),
            ],
        );
        object.remove_entity(1).expect("middle entity");
        assert!(object.move_entity(1, 0));

        let json = serde_json::to_string(&object).expect("serialize label object");
        let restored: LabelObject = serde_json::from_str(&json).expect("restore label object");

        assert_eq!(restored.entities().len(), 2);
        assert_eq!(restored.entities()[0].text(), "last");
        assert_eq!(restored.entities()[1].text(), "first");
        assert_eq!(
            restored.entities()[0].anchor(),
            restored.entities()[1].anchor()
        );
        assert_eq!(restored.entities()[0].presentation().size(), Some(18.0));
        assert_eq!(
            restored.entities()[1].presentation().color(),
            Some(ColorIndex::Named(2))
        );
    }

    #[test]
    fn mixed_color_state_is_discoverable() {
        let mut override_color = LabelEntityPresentation::default();
        override_color
            .set_color(ColorIndex::Named(2))
            .expect("entity color");
        let object = LabelObject::with_entities(
            "labels",
            vec![
                LabelEntity::new(anchor(1), "inherited"),
                LabelEntity::with_presentation(anchor(2), "override", override_color),
            ],
        );

        assert!(object
            .has_mixed_colors(ColorIndex::Named(1))
            .expect("named inherited color"));
    }
}
