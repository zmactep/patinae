//! Object management for scene objects
//!
//! This module provides:
//! - [`Object`] trait for all scene objects
//! - [`ObjectType`] enum for object type identification
//! - [`ObjectState`] for per-object visual state
//! - [`ObjectRegistry`] for managing named objects

mod annotation;
mod annotation_resolver;
mod group;
mod label;
mod map;
pub(crate) mod measurement;
mod molecule;

#[doc(inline)]
pub use annotation::{
    AnnotationColor, AnnotationPresentationError, AtomAnchor, LabelAlignment,
    LabelEntityPresentation, LabelObjectPresentation, LabelPresentation, LabelSize, LinePattern,
    MeasurementEntityPresentation, MeasurementObjectPresentation, MeasurementPresentation,
    ResolvedStrokePath, StrokePath, StrokeStyle,
};
use annotation_resolver::resolve_annotation_bounds_with_options;
#[doc(inline)]
pub use annotation_resolver::{
    resolve_annotation_bundles, resolve_measurement_entity_value, AnnotationColorSummary,
    AnnotationOwnerKind, AnnotationWarning, ResolvedAnnotationBundle, ResolvedLabelPrimitive,
};
pub use group::GroupObject;
#[doc(inline)]
#[expect(deprecated, reason = "public compatibility aliases remain re-exported")]
pub use label::{
    label_object_view, Label, LabelAnchor, LabelAnchorView, LabelEntity, LabelEntityView,
    LabelObject, LabelObjectSnapshot, LabelObjectView, LabelRevisions,
};
pub use map::{MapData, MapDisplayMode, MapObject};
#[doc(inline)]
pub use measurement::{
    MeasurementAnchor, MeasurementEntity, MeasurementEntry, MeasurementEntryError, MeasurementKind,
    MeasurementObject, MeasurementObjectSnapshot, MeasurementResolveOptions, MeasurementRevisions,
    MeasurementType,
};
pub use molecule::MoleculeObject;
pub use patinae_mol::DirtyFlags;

use std::any::Any;
use std::num::NonZeroU16;

use ahash::{AHashMap, AHashSet};
use lin_alg::f32::{Mat4, Vec3};
use patinae_color::ColorIndex;
use patinae_mol::{AtomIndex, RepMask};
use patinae_settings::ObjectOverrides;
use serde::{Deserialize, Serialize};

const NEXT_AFTER_MAX_RENDER_ID: u32 = RenderObjectId::MAX_PICKING_ID + 1;

/// Stable scene-side id used by renderer-facing bridges.
///
/// The id is constrained to the 12-bit picking payload used by
/// `patinae-render`. `0` is reserved by the renderer as the cleared-pixel
/// sentinel, so every valid `RenderObjectId` starts at `1`.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RenderObjectId(NonZeroU16);

impl RenderObjectId {
    /// First assignable render object id.
    pub const FIRST: Self = Self(NonZeroU16::MIN);
    /// Largest id that survives the renderer's 12-bit picking encoding.
    pub const MAX_PICKING_ID: u32 = 0x0FFF;

    /// Creates a render object id from a raw integer.
    pub fn new(raw: u32) -> Option<Self> {
        if raw == 0 || raw > Self::MAX_PICKING_ID {
            return None;
        }
        NonZeroU16::new(raw as u16).map(Self)
    }

    /// Returns the raw id value.
    pub const fn get(self) -> u32 {
        self.0.get() as u32
    }

    /// Returns the sparse lookup slot for this id.
    pub const fn slot_index(self) -> usize {
        (self.0.get() - 1) as usize
    }
}

// On native targets, Object requires Send + Sync for thread safety.
// On wasm32, wgpu's WebGPU backend types use RefCell (not Sync),
// and threading is not used, so we relax the bounds.
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSend: Send {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Send> MaybeSend for T {}

#[cfg(target_arch = "wasm32")]
pub trait MaybeSend {}
#[cfg(target_arch = "wasm32")]
impl<T> MaybeSend for T {}

#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSync: Sync {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Sync> MaybeSync for T {}

#[cfg(target_arch = "wasm32")]
pub trait MaybeSync {}
#[cfg(target_arch = "wasm32")]
impl<T> MaybeSync for T {}

use crate::error::{SceneError, SceneResult};

/// Object type enumeration
///
/// Identifies the semantic kind of a scene object.
///
/// This value is metadata for display and routing. It is not proof that a
/// trait object has a specific concrete Rust layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ObjectType {
    /// Molecular object (atoms, bonds, coordinates)
    Molecule,
    /// Electron density map
    Map,
    /// Triangle mesh
    Mesh,
    /// Molecular surface
    Surface,
    /// Compiled Graphics Object (custom primitives)
    Cgo,
    /// Group of objects
    Group,
    /// Measurement (distances, angles, etc.)
    Measurement,
    /// Label (text annotations)
    Label,
    /// Volume rendering
    Volume,
    /// Alignment object
    Alignment,
}

impl ObjectType {
    /// Whether objects of this type can be picked (clicked/selected).
    ///
    /// Maps and annotation primitives are visual and do not intercept picking.
    pub fn is_pickable(&self) -> bool {
        !matches!(
            self,
            ObjectType::Map | ObjectType::Measurement | ObjectType::Label
        )
    }
}

impl std::fmt::Display for ObjectType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ObjectType::Molecule => write!(f, "molecule"),
            ObjectType::Map => write!(f, "map"),
            ObjectType::Mesh => write!(f, "mesh"),
            ObjectType::Surface => write!(f, "surface"),
            ObjectType::Cgo => write!(f, "cgo"),
            ObjectType::Group => write!(f, "group"),
            ObjectType::Measurement => write!(f, "measurement"),
            ObjectType::Label => write!(f, "label"),
            ObjectType::Volume => write!(f, "volume"),
            ObjectType::Alignment => write!(f, "alignment"),
        }
    }
}

/// Per-object visual state
///
/// Contains the common visual properties shared by all object types.
#[derive(Debug, Clone, Serialize)]
pub struct ObjectState {
    /// Whether the object is enabled/visible
    pub enabled: bool,
    /// Default object color
    pub color: ColorIndex,
    /// Representations that exist on at least one atom.
    pub visible_reps: RepMask,
    /// Object-level draw mask for representation visibility.
    pub draw_reps: RepMask,
    /// 4x4 object transformation matrix.
    ///
    /// This matrix is applied to all coordinates when rendering.
    #[serde(with = "crate::serde_helpers::mat4_serde")]
    pub transform: Mat4,
    /// Draw-mask reps that may be safely restored without touching per-atom bits.
    pub draw_mask_restorable_reps: RepMask,
}

impl Default for ObjectState {
    fn default() -> Self {
        Self {
            enabled: true,
            color: ColorIndex::default(),
            visible_reps: RepMask::default(),
            draw_reps: RepMask::default(),
            transform: Mat4::new_identity(),
            draw_mask_restorable_reps: RepMask::NONE,
        }
    }
}

fn default_enabled() -> bool {
    true
}

fn default_transform() -> Mat4 {
    Mat4::new_identity()
}

impl<'de> Deserialize<'de> for ObjectState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct WireObjectState {
            #[serde(default = "default_enabled")]
            enabled: bool,
            #[serde(default)]
            color: ColorIndex,
            #[serde(default)]
            visible_reps: RepMask,
            #[serde(default)]
            draw_reps: Option<RepMask>,
            #[serde(
                default = "default_transform",
                with = "crate::serde_helpers::mat4_serde"
            )]
            transform: Mat4,
            #[serde(default)]
            draw_mask_restorable_reps: RepMask,
        }

        let wire = WireObjectState::deserialize(deserializer)?;
        Ok(Self {
            enabled: wire.enabled,
            color: wire.color,
            visible_reps: wire.visible_reps,
            draw_reps: wire.draw_reps.unwrap_or(wire.visible_reps),
            transform: wire.transform,
            draw_mask_restorable_reps: wire
                .draw_mask_restorable_reps
                .intersection(RepMask::DRAW_MASK_REPS),
        })
    }
}

impl ObjectState {
    /// Create a new object state with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a disabled object state
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Default::default()
        }
    }

    /// Set the transformation matrix
    pub fn set_transform(&mut self, transform: Mat4) {
        self.transform = transform;
    }

    /// Reset the transformation to identity
    pub fn reset_transform(&mut self) {
        self.transform = Mat4::new_identity();
    }

    /// Check if a representation is visible
    pub fn rep_visible(&self, rep: RepMask) -> bool {
        self.draw_reps.is_visible(rep)
    }

    /// Check whether a draw-mask-only restore is known to be valid.
    pub fn can_restore_draw_mask(&self, rep: RepMask) -> bool {
        rep.can_toggle_with_draw_mask() && rep.is_subset_of(self.draw_mask_restorable_reps)
    }

    /// Mark draw-mask reps as safely restorable.
    pub fn mark_draw_mask_restorable(&mut self, rep: RepMask) {
        self.draw_mask_restorable_reps
            .set_visible(rep.intersection(RepMask::DRAW_MASK_REPS));
    }

    /// Clear draw-mask restore permission for reps whose atom-level bits changed.
    pub fn clear_draw_mask_restorable(&mut self, rep: RepMask) {
        self.draw_mask_restorable_reps
            .set_hidden(rep.intersection(RepMask::DRAW_MASK_REPS));
    }

    /// Show a representation
    pub fn show_rep(&mut self, rep: RepMask) {
        self.visible_reps.set_visible(rep);
        self.draw_reps.set_visible(rep);
        self.clear_draw_mask_restorable(rep);
    }

    /// Hide a representation
    pub fn hide_rep(&mut self, rep: RepMask) {
        self.visible_reps.set_hidden(rep);
        self.draw_reps.set_hidden(rep);
        self.clear_draw_mask_restorable(rep);
    }

    /// Show an already-materialized representation.
    pub fn show_draw_rep(&mut self, rep: RepMask) {
        self.draw_reps.set_visible(rep);
    }

    /// Hide a representation without clearing per-atom rep bits.
    pub fn hide_draw_rep(&mut self, rep: RepMask) {
        self.draw_reps.set_hidden(rep);
        self.mark_draw_mask_restorable(rep);
    }
}

/// Trait for all scene objects.
///
/// This trait defines the common interface for all objects that can be
/// added to a scene, including molecules, maps, surfaces, etc. Implementers
/// must be `'static` so registries can safely downcast stored trait objects
/// without relying on user-provided [`ObjectType`] values.
pub trait Object: Any + MaybeSend + MaybeSync {
    /// Get the object name
    fn name(&self) -> &str;

    /// Get the object type
    fn object_type(&self) -> ObjectType;

    /// Get the visual state
    fn state(&self) -> &ObjectState;

    /// Get mutable access to the visual state
    fn state_mut(&mut self) -> &mut ObjectState;

    /// Get the bounding box (min, max) for the current coordinate state
    ///
    /// Returns `None` if the object has no geometry.
    fn extent(&self) -> Option<(Vec3, Vec3)>;

    /// Get the number of coordinate states
    ///
    /// For single-state objects, returns 1.
    fn n_states(&self) -> usize {
        1
    }

    /// Get the current state index
    fn current_state(&self) -> usize {
        0
    }

    /// Set the current state index
    fn set_current_state(&mut self, _state: usize) -> bool {
        false
    }

    /// Get object-level settings overrides (if any)
    fn overrides(&self) -> Option<&ObjectOverrides> {
        None
    }

    /// Get mutable object-level settings overrides
    fn overrides_mut(&mut self) -> Option<&mut ObjectOverrides> {
        None
    }

    /// Get or create mutable object-level settings overrides.
    ///
    /// Default implementation panics — override in types that support per-object settings.
    fn get_or_create_overrides(&mut self) -> &mut ObjectOverrides {
        unimplemented!(
            "{} does not support per-object settings overrides",
            self.name()
        )
    }

    /// Check if this object is enabled
    fn is_enabled(&self) -> bool {
        self.state().enabled
    }

    /// Enable the object
    fn enable(&mut self) {
        self.state_mut().enabled = true;
    }

    /// Disable the object
    fn disable(&mut self) {
        self.state_mut().enabled = false;
    }

    /// Get the center of the object
    fn center(&self) -> Option<Vec3> {
        let (min, max) = self.extent()?;
        Some(Vec3::new(
            (min.x + max.x) * 0.5,
            (min.y + max.y) * 0.5,
            (min.z + max.z) * 0.5,
        ))
    }

    /// Set the object name
    fn set_name(&mut self, name: String);
}

impl dyn Object + '_ {
    /// Returns this object as [`Any`] for safe runtime downcasting.
    pub fn as_any(&self) -> &dyn Any {
        self
    }

    /// Returns this object as mutable [`Any`] for safe runtime downcasting.
    pub fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Object registry for managing named objects
///
/// The registry stores all scene objects by name and maintains render order.
///
/// Note: The `objects` field contains trait objects (`Box<dyn Object>`) which
/// cannot be directly serialized. Use [`ObjectRegistrySnapshot`] for
/// serialization of the object data.
pub struct ObjectRegistry {
    /// Objects stored by name
    objects: AHashMap<String, Box<dyn Object>>,
    /// Ordered list of object names for rendering
    render_order: Vec<String>,
    /// Stable renderer id per object name.
    render_ids: AHashMap<String, RenderObjectId>,
    /// Counter for generating unique names
    next_id: u32,
    /// Next raw renderer id to assign.
    next_render_id: u16,
    /// Generation counter, incremented on structural changes (add/remove/rename)
    generation: u64,
}

/// Serializable snapshot of the object registry.
///
/// Captures persistent molecule, group, map, measurement, and label objects.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectRegistrySnapshot {
    /// Molecule objects (name -> data)
    pub molecules: Vec<(String, MoleculeObjectSnapshot)>,
    /// Group objects (name -> data)
    pub groups: Vec<(String, GroupObject)>,
    /// Map objects (name -> data)
    #[serde(default)]
    pub maps: Vec<(String, MapObjectSnapshot)>,
    /// Render order
    pub render_order: Vec<String>,
    /// Object states for all objects (name -> state)
    pub object_states: Vec<(String, ObjectState)>,
    /// Stable renderer ids for all objects (name -> id).
    #[serde(default)]
    pub render_ids: Vec<(String, u32)>,
    /// Next stable renderer id.
    #[serde(default = "default_next_render_id")]
    pub next_render_id: u32,
    /// Next id counter
    pub next_id: u32,
    /// Generation counter
    pub generation: u64,
    /// Measurement objects (name -> dynamic atom anchors).
    ///
    /// Kept as the final field so legacy positional MessagePack snapshots
    /// retain the field indices used by PRS v1/v2.
    #[serde(default)]
    pub measurements: Vec<(String, MeasurementObjectSnapshot)>,
    /// Label objects (name -> atom-anchored semantic labels).
    ///
    /// This remains the final field so old PRS v3 positional snapshots decode
    /// with an empty default.
    #[serde(default)]
    pub labels: Vec<(String, LabelObjectSnapshot)>,
}

fn default_next_render_id() -> u32 {
    RenderObjectId::FIRST.get()
}

/// Serializable snapshot of a MoleculeObject (without render caches).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoleculeObjectSnapshot {
    /// The underlying molecular data
    pub molecule: patinae_mol::ObjectMolecule,
    /// Visual state
    pub state: ObjectState,
    /// Current coordinate state index being displayed
    pub display_state: usize,
    /// Per-object settings overrides
    pub overrides: Option<ObjectOverrides>,
    /// Surface quality
    pub surface_quality: i32,
}

/// Serializable snapshot of a MapObject (without render caches).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapObjectSnapshot {
    /// Map data for each state
    pub states: Vec<MapData>,
    /// Visual state
    pub state: ObjectState,
    /// Current display state index
    pub current_state: usize,
    /// Isocontour level
    pub level: f32,
    /// Display mode
    #[serde(default)]
    pub display_mode: MapDisplayMode,
    /// Mesh color (RGBA)
    pub mesh_color: [f32; 4],
    /// Carve radius
    pub carve_radius: f32,
    /// Carve positions
    pub carve_positions: Option<Vec<[f32; 3]>>,
    /// Per-object settings overrides
    pub overrides: Option<ObjectOverrides>,
}

impl ObjectRegistry {
    /// Create a serializable snapshot of this registry.
    pub fn to_snapshot(&self) -> ObjectRegistrySnapshot {
        let mut molecules = Vec::new();
        let mut groups = Vec::new();
        let mut maps = Vec::new();
        let mut measurements = Vec::new();
        let mut labels = Vec::new();
        let mut object_states = Vec::new();

        for name in &self.render_order {
            if let Some(obj) = self.objects.get(name) {
                object_states.push((name.clone(), obj.state().clone()));
                if let Some(mol_obj) = Self::object_as::<MoleculeObject>(obj.as_ref()) {
                    molecules.push((
                        name.clone(),
                        MoleculeObjectSnapshot {
                            molecule: mol_obj.molecule().clone(),
                            state: mol_obj.state().clone(),
                            display_state: mol_obj.display_state(),
                            overrides: mol_obj.overrides().cloned(),
                            surface_quality: mol_obj.surface_quality(),
                        },
                    ));
                } else if let Some(group) = Self::object_as::<GroupObject>(obj.as_ref()) {
                    groups.push((name.clone(), group.clone()));
                } else if let Some(map_obj) = Self::object_as::<MapObject>(obj.as_ref()) {
                    maps.push((
                        name.clone(),
                        MapObjectSnapshot {
                            states: map_obj.states().to_vec(),
                            state: map_obj.state().clone(),
                            current_state: map_obj.current_state(),
                            level: map_obj.level(),
                            display_mode: map_obj.display_mode(),
                            mesh_color: map_obj.mesh_color(),
                            carve_radius: map_obj.carve_radius(),
                            carve_positions: map_obj.carve_positions().cloned(),
                            overrides: map_obj.overrides().cloned(),
                        },
                    ));
                } else if let Some(measurement) = Self::object_as::<MeasurementObject>(obj.as_ref())
                {
                    measurements.push((name.clone(), measurement.to_snapshot()));
                } else if let Some(label) = Self::object_as::<LabelObject>(obj.as_ref()) {
                    labels.push((name.clone(), label.to_snapshot()));
                }
            }
        }

        ObjectRegistrySnapshot {
            molecules,
            groups,
            maps,
            render_order: self.render_order.clone(),
            object_states,
            render_ids: self
                .render_order
                .iter()
                .filter_map(|name| self.render_ids.get(name).map(|id| (name.clone(), id.get())))
                .collect(),
            next_render_id: u32::from(self.next_render_id),
            next_id: self.next_id,
            generation: self.generation,
            measurements,
            labels,
        }
    }

    /// Restore from a serializable snapshot.
    pub fn from_snapshot(snapshot: ObjectRegistrySnapshot) -> Self {
        let mut registry = Self::new();
        registry.next_id = snapshot.next_id;
        registry.generation = snapshot.generation;

        // Add molecules (use from_raw to preserve per-atom visible_reps)
        for (name, snap) in snapshot.molecules {
            let mut mol_obj = MoleculeObject::from_raw(snap.molecule);
            mol_obj.molecule_mut().name = name.clone();
            *mol_obj.state_mut() = snap.state;
            if snap.display_state > 0 {
                mol_obj.set_display_state(snap.display_state);
            }
            if let Some(overrides) = snap.overrides {
                *mol_obj.get_or_create_overrides() = overrides;
            }
            mol_obj.set_surface_quality(snap.surface_quality);
            registry.objects.insert(name, Box::new(mol_obj));
        }

        // Add groups
        for (name, group) in snapshot.groups {
            registry.objects.insert(name, Box::new(group));
        }

        // Add maps
        for (name, snap) in snapshot.maps {
            let mut map_obj = MapObject::empty(&name);
            for map_data in snap.states {
                map_obj.add_state(map_data);
            }
            *map_obj.state_mut() = snap.state;
            if snap.current_state > 0 {
                map_obj.set_current_state(snap.current_state);
            }
            map_obj.set_level(snap.level);
            map_obj.set_display_mode(snap.display_mode);
            map_obj.set_mesh_color(snap.mesh_color);
            map_obj.set_carve_radius(snap.carve_radius);
            if let Some(positions) = snap.carve_positions {
                map_obj.set_carve_positions(positions);
            }
            if let Some(overrides) = snap.overrides {
                map_obj.set_overrides(overrides);
            }
            registry.objects.insert(name, Box::new(map_obj));
        }

        // Add measurements.
        for (name, snapshot) in snapshot.measurements {
            registry.objects.insert(
                name.clone(),
                Box::new(MeasurementObject::from_snapshot(name, snapshot)),
            );
        }

        // Add semantic label collections.
        for (name, snapshot) in snapshot.labels {
            registry.objects.insert(
                name.clone(),
                Box::new(LabelObject::from_snapshot(name, snapshot)),
            );
        }

        // Restore render order (only names that exist)
        registry.render_order = snapshot
            .render_order
            .into_iter()
            .filter(|n| registry.objects.contains_key(n))
            .collect();
        registry.restore_render_ids(snapshot.render_ids, snapshot.next_render_id);
        registry.migrate_atom_local_labels();

        registry
    }
}

impl Default for ObjectRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ObjectRegistry {
    fn object_as<T: Object>(obj: &dyn Object) -> Option<&T> {
        obj.as_any().downcast_ref::<T>()
    }

    fn object_as_mut<T: Object>(obj: &mut dyn Object) -> Option<&mut T> {
        obj.as_any_mut().downcast_mut::<T>()
    }

    fn get_typed<T: Object>(&self, name: &str) -> Option<&T> {
        let obj = self.objects.get(name)?;
        Self::object_as(obj.as_ref())
    }

    fn get_typed_mut<T: Object>(&mut self, name: &str) -> Option<&mut T> {
        let obj = self.objects.get_mut(name)?;
        Self::object_as_mut(obj.as_mut())
    }

    fn normalize_next_render_id(raw: u32) -> u16 {
        raw.clamp(RenderObjectId::FIRST.get(), NEXT_AFTER_MAX_RENDER_ID) as u16
    }

    fn allocate_render_id(&mut self) -> Option<RenderObjectId> {
        let mut raw = u32::from(self.next_render_id);
        if raw > RenderObjectId::MAX_PICKING_ID {
            raw = RenderObjectId::FIRST.get();
        }
        for _ in 0..RenderObjectId::MAX_PICKING_ID {
            let id = RenderObjectId::new(raw)?;
            let next = if raw == RenderObjectId::MAX_PICKING_ID {
                RenderObjectId::FIRST.get()
            } else {
                raw + 1
            };
            self.next_render_id = Self::normalize_next_render_id(next);
            if !self.render_ids.values().any(|existing| *existing == id) {
                return Some(id);
            }
            raw = next;
        }
        None
    }

    fn assign_render_id_if_missing(&mut self, name: &str) {
        if self.render_ids.contains_key(name) {
            return;
        }
        if let Some(id) = self.allocate_render_id() {
            self.render_ids.insert(name.to_string(), id);
        }
    }

    fn insert_named_boxed(&mut self, name: String, obj: Box<dyn Object>) {
        if self.objects.contains_key(&name) {
            self.orphan_annotation_anchors(&name);
        }
        self.render_order.retain(|n| n != &name);
        self.render_order.push(name.clone());
        self.objects.insert(name.clone(), obj);
        self.assign_render_id_if_missing(&name);
        self.generation += 1;
    }

    fn restore_render_ids(&mut self, saved_ids: Vec<(String, u32)>, saved_next: u32) {
        self.render_ids.clear();
        let mut used_ids = AHashSet::new();
        let mut max_seen = 0;
        for (name, raw_id) in saved_ids {
            let Some(id) = RenderObjectId::new(raw_id) else {
                continue;
            };
            if !self.objects.contains_key(&name) || self.render_ids.contains_key(&name) {
                continue;
            }
            if !used_ids.insert(id) {
                continue;
            }
            max_seen = max_seen.max(id.get());
            self.render_ids.insert(name, id);
        }

        self.next_render_id =
            Self::normalize_next_render_id(saved_next.max(max_seen.saturating_add(1)));

        let names: Vec<String> = self.render_order.clone();
        for name in names {
            self.assign_render_id_if_missing(&name);
        }
    }

    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            objects: AHashMap::new(),
            render_order: Vec::new(),
            render_ids: AHashMap::new(),
            next_id: 1,
            next_render_id: Self::normalize_next_render_id(RenderObjectId::FIRST.get()),
            generation: 0,
        }
    }

    /// Get the number of objects
    pub fn len(&self) -> usize {
        self.objects.len()
    }

    /// Check if the registry is empty
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    /// Get the generation counter (incremented on structural changes)
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Get the stable renderer id for an object.
    pub fn render_id(&self, name: &str) -> Option<RenderObjectId> {
        self.render_ids.get(name).copied()
    }

    /// Force observers that track `generation()` to resync.
    pub fn invalidate(&mut self) {
        self.generation += 1;
    }

    /// Returns true if any molecule object has pending dirty flags
    /// (e.g. COLOR, COORDS, REPS set by commands but not yet consumed by render).
    pub fn has_any_dirty_molecule(&self) -> bool {
        self.names()
            .any(|name| self.get_molecule(name).is_some_and(|m| m.is_dirty()))
    }

    /// Returns true if any map object has pending render-facing changes.
    pub fn has_any_dirty_map(&self) -> bool {
        self.names()
            .any(|name| self.get_map(name).is_some_and(MapObject::is_dirty))
    }

    /// Returns whether any measurement has pending render-facing changes.
    pub fn has_any_dirty_measurement(&self) -> bool {
        self.names().any(|name| {
            self.get_measurement(name)
                .is_some_and(MeasurementObject::is_dirty)
        })
    }

    /// Returns whether any label object has pending render-facing changes.
    pub fn has_any_dirty_label(&self) -> bool {
        self.names()
            .any(|name| self.get_label(name).is_some_and(LabelObject::is_dirty))
    }

    /// Mark all molecule objects as fully dirty so representations rebuild.
    pub fn mark_all_dirty(&mut self) {
        self.generation += 1;
        for object in self.objects.values_mut() {
            match object.object_type() {
                ObjectType::Molecule => {
                    if let Some(molecule) = Self::object_as_mut::<MoleculeObject>(object.as_mut()) {
                        molecule.invalidate(DirtyFlags::ALL);
                    }
                }
                ObjectType::Map => {
                    if let Some(map) = Self::object_as_mut::<MapObject>(object.as_mut()) {
                        map.invalidate();
                    }
                }
                ObjectType::Measurement => {
                    if let Some(measurement) =
                        Self::object_as_mut::<MeasurementObject>(object.as_mut())
                    {
                        measurement.invalidate_material();
                    }
                }
                ObjectType::Label => {
                    if let Some(label) = Self::object_as_mut::<LabelObject>(object.as_mut()) {
                        label.invalidate();
                    }
                }
                _ => {}
            }
        }
    }

    /// Clear pending dirty flags on every molecule object.
    /// patinae-render tracks dirty per-rep internally; this clears the
    /// host-side flag set by commands so the scene model doesn't perpetually
    /// rebuild (which would destroy panel TouchAreas every frame).
    pub fn clear_all_dirty_molecules(&mut self) {
        for object in self.objects.values_mut() {
            if let Some(mol) = Self::object_as_mut::<MoleculeObject>(object.as_mut()) {
                mol.clear_dirty();
            }
        }
    }

    /// Clear pending dirty state on every map object.
    pub fn clear_all_dirty_maps(&mut self) {
        for object in self.objects.values_mut() {
            if let Some(map) = Self::object_as_mut::<MapObject>(object.as_mut()) {
                map.clear_dirty();
            }
        }
    }

    /// Clears pending dirty state on every measurement object.
    pub fn clear_all_dirty_measurements(&mut self) {
        for object in self.objects.values_mut() {
            if let Some(measurement) = Self::object_as_mut::<MeasurementObject>(object.as_mut()) {
                measurement.clear_dirty();
            }
        }
    }

    /// Clears pending dirty state on every label object.
    pub fn clear_all_dirty_labels(&mut self) {
        for object in self.objects.values_mut() {
            if let Some(label) = Self::object_as_mut::<LabelObject>(object.as_mut()) {
                label.clear_dirty();
            }
        }
    }

    /// Clears render-facing dirty state for every supported scene object.
    pub fn clear_all_dirty_objects(&mut self) {
        for object in self.objects.values_mut() {
            match object.object_type() {
                ObjectType::Molecule => {
                    if let Some(molecule) = Self::object_as_mut::<MoleculeObject>(object.as_mut()) {
                        molecule.clear_dirty();
                    }
                }
                ObjectType::Map => {
                    if let Some(map) = Self::object_as_mut::<MapObject>(object.as_mut()) {
                        map.clear_dirty();
                    }
                }
                ObjectType::Measurement => {
                    if let Some(measurement) =
                        Self::object_as_mut::<MeasurementObject>(object.as_mut())
                    {
                        measurement.clear_dirty();
                    }
                }
                ObjectType::Label => {
                    if let Some(label) = Self::object_as_mut::<LabelObject>(object.as_mut()) {
                        label.clear_dirty();
                    }
                }
                _ => {}
            }
        }
    }

    /// Add an object to the registry
    ///
    /// Returns the name used for the object. If an object with the same name
    /// already exists, it will be replaced.
    /// Insert a pre-boxed object by name.
    pub fn insert_boxed(&mut self, name: &str, obj: Box<dyn Object>) {
        self.insert_named_boxed(name.to_string(), obj);
    }

    pub fn add<O: Object + 'static>(&mut self, obj: O) -> &str {
        let name = obj.name().to_string();
        self.insert_named_boxed(name, Box::new(obj));
        self.render_order.last().map(|s| s.as_str()).unwrap()
    }

    /// Add an object with an auto-generated name
    pub fn add_with_name<O: Object + 'static>(&mut self, mut obj: O, base_name: &str) -> String {
        let name = self.generate_unique_name(base_name);
        obj.set_name(name.clone());
        self.add(obj);
        name
    }

    /// Generate a unique name based on a base name
    fn generate_unique_name(&mut self, base: &str) -> String {
        if !self.objects.contains_key(base) {
            return base.to_string();
        }

        loop {
            let name = format!("{}_{}", base, self.next_id);
            self.next_id += 1;
            if !self.objects.contains_key(&name) {
                return name;
            }
        }
    }

    /// Remove an object by name
    ///
    /// Also removes the object from any group that contains it.
    pub fn remove(&mut self, name: &str) -> Option<Box<dyn Object>> {
        if self.objects.contains_key(name) {
            self.orphan_annotation_anchors(name);
        }
        self.remove_from_groups(name);
        self.render_order.retain(|n| n != name);
        let removed = self.objects.remove(name);
        if removed.is_some() {
            self.render_ids.remove(name);
            self.generation += 1;
        }
        removed
    }

    /// Get an object by name
    pub fn get(&self, name: &str) -> Option<&dyn Object> {
        self.objects.get(name).map(|b| b.as_ref())
    }

    /// Get mutable access to an object by name
    pub fn get_mut(&mut self, name: &str) -> Option<&mut dyn Object> {
        match self.objects.get_mut(name) {
            Some(boxed) => Some(boxed.as_mut()),
            None => None,
        }
    }

    /// Get a molecule object by name
    pub fn get_molecule(&self, name: &str) -> Option<&MoleculeObject> {
        self.get_typed(name)
    }

    /// Get mutable access to a molecule object by name
    pub fn get_molecule_mut(&mut self, name: &str) -> Option<&mut MoleculeObject> {
        self.get_typed_mut(name)
    }

    /// Get a group object by name
    pub fn get_group(&self, name: &str) -> Option<&GroupObject> {
        self.get_typed(name)
    }

    /// Get mutable access to a group object by name
    pub fn get_group_mut(&mut self, name: &str) -> Option<&mut GroupObject> {
        self.get_typed_mut(name)
    }

    /// Get a measurement object by name
    pub fn get_measurement(&self, name: &str) -> Option<&MeasurementObject> {
        self.get_typed(name)
    }

    /// Get mutable access to a measurement object by name
    pub fn get_measurement_mut(&mut self, name: &str) -> Option<&mut MeasurementObject> {
        self.get_typed_mut(name)
    }

    /// Gets a label object by name.
    pub fn get_label(&self, name: &str) -> Option<&LabelObject> {
        self.get_typed(name)
    }

    /// Gets mutable access to a label object by name.
    pub fn get_label_mut(&mut self, name: &str) -> Option<&mut LabelObject> {
        self.get_typed_mut(name)
    }

    /// Get a map object by name
    pub fn get_map(&self, name: &str) -> Option<&MapObject> {
        self.get_typed(name)
    }

    /// Get mutable access to a map object by name
    pub fn get_map_mut(&mut self, name: &str) -> Option<&mut MapObject> {
        self.get_typed_mut(name)
    }

    /// Get the combined bounding box of all children in a group
    ///
    /// Recursively computes the extent including nested groups.
    pub fn group_extent(&self, group_name: &str) -> Option<(Vec3, Vec3)> {
        self.group_extent_with_options(group_name, MeasurementResolveOptions::default())
    }

    /// Computes group bounds with explicit measurement resolver options.
    pub fn group_extent_with_options(
        &self,
        group_name: &str,
        options: MeasurementResolveOptions,
    ) -> Option<(Vec3, Vec3)> {
        let group = self.get_group(group_name)?;
        if !group.is_enabled() {
            return None;
        }
        let children = group.children().to_vec();

        let mut min = Vec3::new(f32::MAX, f32::MAX, f32::MAX);
        let mut max = Vec3::new(f32::MIN, f32::MIN, f32::MIN);
        let mut has_extent = false;

        for child_name in &children {
            // Check if child is a group (recursive)
            if self.get_group(child_name).is_some() {
                if let Some((child_min, child_max)) =
                    self.group_extent_with_options(child_name, options)
                {
                    min.x = min.x.min(child_min.x);
                    min.y = min.y.min(child_min.y);
                    min.z = min.z.min(child_min.z);
                    max.x = max.x.max(child_max.x);
                    max.y = max.y.max(child_max.y);
                    max.z = max.z.max(child_max.z);
                    has_extent = true;
                }
            } else if let Some((child_min, child_max)) =
                self.object_extent_with_options(child_name, options)
            {
                min.x = min.x.min(child_min.x);
                min.y = min.y.min(child_min.y);
                min.z = min.z.min(child_min.z);
                max.x = max.x.max(child_max.x);
                max.y = max.y.max(child_max.y);
                max.z = max.z.max(child_max.z);
                has_extent = true;
            }
        }

        if has_extent {
            Some((min, max))
        } else {
            None
        }
    }

    /// Computes one object's current dynamic extent.
    pub fn object_extent(&self, name: &str) -> Option<(Vec3, Vec3)> {
        self.object_extent_with_options(name, MeasurementResolveOptions::default())
    }

    /// Computes one object's extent with explicit measurement resolver options.
    pub fn object_extent_with_options(
        &self,
        name: &str,
        options: MeasurementResolveOptions,
    ) -> Option<(Vec3, Vec3)> {
        if self.get_measurement(name).is_some() || self.get_label(name).is_some() {
            return resolve_annotation_bounds_with_options(self, name, options);
        }
        self.get(name).and_then(Object::extent)
    }

    fn orphan_annotation_anchors(&mut self, source_name: &str) {
        for object in self.objects.values_mut() {
            if let Some(measurement) = Self::object_as_mut::<MeasurementObject>(object.as_mut()) {
                measurement.orphan_anchors_to(source_name);
            } else if let Some(label) = Self::object_as_mut::<LabelObject>(object.as_mut()) {
                label.orphan_anchors_to(source_name);
            }
        }
    }

    /// Updates annotation anchors after one topology edit.
    pub fn remap_atom_anchors(&mut self, source_name: &str, remap: &patinae_mol::AtomRemap) {
        if remap.is_identity() {
            return;
        }
        let mut changed = false;
        for object in self.objects.values_mut() {
            if let Some(measurement) = Self::object_as_mut::<MeasurementObject>(object.as_mut()) {
                changed |= measurement.remap_anchors(source_name, remap);
            } else if let Some(label) = Self::object_as_mut::<LabelObject>(object.as_mut()) {
                changed |= label.remap_anchors(source_name, remap);
            }
        }
        if changed {
            self.generation = self.generation.saturating_add(1);
        }
    }

    /// Removes molecule atoms while orphaning dependent annotation anchors.
    ///
    /// Removing every atom deletes the source object through the normal
    /// registry lifecycle. Partial removal preserves the molecule object and
    /// invalidates all of its derived representations.
    ///
    /// # Errors
    ///
    /// Returns an error when `source_name` is not a molecule object.
    pub fn remove_molecule_atoms(
        &mut self,
        source_name: &str,
        indices: &[AtomIndex],
    ) -> SceneResult<usize> {
        let molecule = self
            .get_molecule(source_name)
            .ok_or_else(|| SceneError::object_not_found(source_name))?;
        let atom_count = molecule.molecule().atom_count();
        let unique_indices = indices
            .iter()
            .map(|index| index.as_usize())
            .collect::<AHashSet<_>>();
        let removes_every_atom = atom_count != 0
            && unique_indices.len() == atom_count
            && unique_indices.iter().all(|&index| index < atom_count);
        if removes_every_atom {
            self.remove(source_name);
            return Ok(indices.len());
        }

        let remap = {
            let molecule = self
                .get_molecule_mut(source_name)
                .ok_or_else(|| SceneError::object_not_found(source_name))?;
            let remap = molecule.molecule_mut().remove_atoms(indices);
            molecule.invalidate(DirtyFlags::ALL);
            remap
        };
        let generation_before_remap = self.generation;
        let topology_changed = !remap.is_identity();
        self.remap_atom_anchors(source_name, &remap);
        if topology_changed && self.generation == generation_before_remap {
            self.generation = self.generation.saturating_add(1);
        }
        Ok(indices.len())
    }

    fn migrate_atom_local_labels(&mut self) {
        let source_names = self.render_order.clone();
        let mut migrated = Vec::new();
        for source_name in source_names {
            let Some(molecule) = self.get_molecule(&source_name) else {
                continue;
            };
            let object_labels_visible = molecule.draw_reps().is_visible(RepMask::LABELS);
            let entities = molecule
                .molecule()
                .atoms_indexed()
                .filter_map(|(index, atom)| {
                    if atom.repr.label.is_empty() {
                        return None;
                    }
                    let mut presentation = LabelEntityPresentation::default();
                    presentation.set_visible(
                        object_labels_visible && atom.repr.visible_reps.is_visible(RepMask::LABELS),
                    );
                    Some(LabelEntity::with_presentation(
                        AtomAnchor::new(source_name.clone(), index),
                        atom.repr.label.clone(),
                        presentation,
                    ))
                })
                .collect::<Vec<_>>();
            if !entities.is_empty() {
                migrated.push((source_name, entities));
            }
        }

        let mut next_label_index = 1;
        for (source_name, entities) in migrated {
            let name = self.first_free_label_name_from(&mut next_label_index);
            self.add(LabelObject::with_entities(name, entities));
            if let Some(molecule) = self.get_molecule_mut(&source_name) {
                for atom in molecule.molecule_mut().atoms_mut() {
                    atom.repr.label.clear();
                    atom.repr.visible_reps.set_hidden(RepMask::LABELS);
                }
                molecule
                    .state_mut()
                    .visible_reps
                    .set_hidden(RepMask::LABELS);
                molecule.state_mut().draw_reps.set_hidden(RepMask::LABELS);
            }
        }
    }

    /// Returns the first free `labelNN` name in the shared object namespace.
    ///
    /// # Panics
    ///
    /// Panics if every suffix representable by `u32` is already occupied.
    pub fn first_free_label_name(&self) -> String {
        let mut index = 1;
        self.first_free_label_name_from(&mut index)
    }

    /// Returns the first free kind-specific measurement name.
    ///
    /// The generated `distanceNN`, `angleNN`, or `dihedralNN` name is checked
    /// against the registry's shared object namespace.
    ///
    /// # Panics
    ///
    /// Panics if every suffix representable by `u32` is already occupied.
    pub fn first_free_measurement_name(&self, kind: MeasurementKind) -> String {
        let prefix = match kind {
            MeasurementKind::Distance => "distance",
            MeasurementKind::Angle => "angle",
            MeasurementKind::Dihedral => "dihedral",
        };
        let mut index = 1_u32;
        loop {
            let name = format!("{prefix}{index:02}");
            if !self.objects.contains_key(&name) {
                return name;
            }
            index = index
                .checked_add(1)
                .expect("measurement object name space exhausted");
        }
    }

    fn first_free_label_name_from(&self, index: &mut u32) -> String {
        loop {
            let name = format!("label{index:02}");
            *index = index
                .checked_add(1)
                .expect("label object name space exhausted");
            if !self.objects.contains_key(&name) {
                return name;
            }
        }
    }

    /// Enable or disable a group and all its children recursively.
    ///
    /// # Errors
    ///
    /// Returns an error if `group_name` does not name a group object.
    pub fn set_group_enabled(&mut self, group_name: &str, enabled: bool) -> SceneResult<()> {
        // First, collect the children names
        let children = {
            let group = self
                .get_group(group_name)
                .ok_or_else(|| SceneError::object_not_found(group_name))?;
            group.children().to_vec()
        };

        // Enable/disable the group itself
        if let Some(obj) = self.objects.get_mut(group_name) {
            obj.state_mut().enabled = enabled;
        }

        // Recursively enable/disable children
        for child_name in children {
            // Check if child is a group
            if self.get_group(&child_name).is_some() {
                // Recursive call for nested groups
                let _ = self.set_group_enabled(&child_name, enabled);
            } else if let Some(obj) = self.objects.get_mut(&child_name) {
                obj.state_mut().enabled = enabled;
            }
        }

        self.generation += 1;
        Ok(())
    }

    /// Find the parent group of an object (if any)
    ///
    /// An object can be in at most one group. Returns the group name.
    pub fn parent_group(&self, name: &str) -> Option<&str> {
        for obj in self.objects.values() {
            if let Some(group) = Self::object_as::<GroupObject>(obj.as_ref()) {
                if group.contains_child(name) {
                    return Some(group.name());
                }
            }
        }
        None
    }

    /// Remove an object from any group that contains it
    pub fn remove_from_groups(&mut self, name: &str) {
        for obj in self.objects.values_mut() {
            if let Some(group) = Self::object_as_mut::<GroupObject>(obj.as_mut()) {
                group.remove_child(name);
            }
        }
    }

    /// Add an object to a group, creating the group if it doesn't exist
    ///
    /// The child is removed from any existing group first (an object can only
    /// be in one group). Returns false if the child doesn't exist or if
    /// trying to add an object to itself.
    pub fn add_to_group(&mut self, group_name: &str, child_name: &str) -> bool {
        if !self.objects.contains_key(child_name) {
            return false;
        }
        if group_name == child_name {
            return false;
        }

        // Remove from any existing group
        self.remove_from_groups(child_name);

        // Create group if needed
        if !self.objects.contains_key(group_name) {
            self.add(GroupObject::new(group_name));
        }

        if let Some(group) = self.get_group_mut(group_name) {
            group.add_child(child_name.to_string());
        } else {
            return false;
        }

        self.reorder_children_after_group(group_name);
        self.generation += 1;
        true
    }

    /// Reorder render_order so group children appear immediately after their group
    fn reorder_children_after_group(&mut self, group_name: &str) {
        let children: Vec<String> = match self.get_group(group_name) {
            Some(g) => g.children().to_vec(),
            None => return,
        };

        // Remove children from render_order
        self.render_order.retain(|n| !children.contains(n));

        // Find group position and insert children right after
        if let Some(pos) = self.render_order.iter().position(|n| n == group_name) {
            for (i, child) in children.into_iter().enumerate() {
                self.render_order.insert(pos + 1 + i, child);
            }
        }
    }

    /// Get names of top-level objects (not children of any group)
    ///
    /// Returns names in render order, excluding objects that are children
    /// of a group. Used by the GUI to start tree iteration.
    pub fn top_level_names(&self) -> Vec<&str> {
        // Collect all children across all groups
        let mut children_set = std::collections::HashSet::new();
        for obj in self.objects.values() {
            if let Some(group) = Self::object_as::<GroupObject>(obj.as_ref()) {
                for child in group.children() {
                    children_set.insert(child.as_str());
                }
            }
        }
        self.render_order
            .iter()
            .map(|s| s.as_str())
            .filter(|name| !children_set.contains(name))
            .collect()
    }

    /// Check if an object exists
    pub fn contains(&self, name: &str) -> bool {
        self.objects.contains_key(name)
    }

    /// Get all object names
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.render_order.iter().map(|s| s.as_str())
    }

    /// Get all objects in render order
    pub fn iter(&self) -> impl Iterator<Item = &dyn Object> {
        self.render_order
            .iter()
            .filter_map(|name| self.objects.get(name).map(|b| b.as_ref()))
    }

    /// Get all objects mutably as boxed references
    ///
    /// Note: Returns iterator over Box references, not trait object references,
    /// to avoid lifetime issues with trait objects.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Box<dyn Object>> + '_ {
        self.objects.values_mut()
    }

    /// Enable an object.
    ///
    /// # Errors
    ///
    /// Returns an error if `name` does not exist in the registry.
    pub fn enable(&mut self, name: &str, enabled: bool) -> SceneResult<()> {
        let obj = self
            .objects
            .get_mut(name)
            .ok_or_else(|| SceneError::object_not_found(name))?;
        obj.state_mut().enabled = enabled;
        self.generation += 1;
        Ok(())
    }

    /// Sets enabled state for every semantic label object.
    ///
    /// Returns the number of label objects and advances registry generation at
    /// most once when at least one state changes.
    pub fn set_label_objects_enabled(&mut self, enabled: bool) -> usize {
        let mut affected = 0;
        let mut changed = false;
        for object in self.objects.values_mut() {
            let Some(label) = Self::object_as_mut::<LabelObject>(object.as_mut()) else {
                continue;
            };
            affected += 1;
            if label.is_enabled() != enabled {
                label.state_mut().enabled = enabled;
                changed = true;
            }
        }
        if changed {
            self.generation = self.generation.saturating_add(1);
        }
        affected
    }

    /// Disable all objects
    pub fn disable_all(&mut self) {
        for obj in self.objects.values_mut() {
            obj.state_mut().enabled = false;
        }
        self.generation += 1;
    }

    /// Enable all objects
    pub fn enable_all(&mut self) {
        for obj in self.objects.values_mut() {
            obj.state_mut().enabled = true;
        }
        self.generation += 1;
    }

    /// Get enabled objects in render order
    pub fn enabled_objects(&self) -> impl Iterator<Item = &dyn Object> {
        self.render_order.iter().filter_map(|name| {
            self.objects.get(name).and_then(|obj| {
                if obj.state().enabled {
                    Some(obj.as_ref())
                } else {
                    None
                }
            })
        })
    }

    /// Get object names matching a pattern.
    ///
    /// Supports `*` (any characters) and `?` (single character) wildcards
    /// via `patinae_select::Pattern::Wildcard`. Pattern `*` or `all` matches
    /// everything. Names without wildcard characters are matched exactly.
    pub fn matching<'a>(&'a self, pattern: &'a str) -> Vec<&'a str> {
        if pattern == "*" || pattern == "all" {
            return self.render_order.iter().map(|s| s.as_str()).collect();
        }

        if pattern.contains('*') || pattern.contains('?') {
            let pat = patinae_select::Pattern::Wildcard(pattern.to_string());
            return self
                .render_order
                .iter()
                .filter(|name| pat.matches(name, false))
                .map(|s| s.as_str())
                .collect();
        }

        // Exact match
        if self.objects.contains_key(pattern) {
            // Return a reference to our stored name, not the input
            self.render_order
                .iter()
                .find(|n| n.as_str() == pattern)
                .map(|s| vec![s.as_str()])
                .unwrap_or_default()
        } else {
            vec![]
        }
    }

    /// Set the render order
    pub fn set_render_order(&mut self, names: &[&str]) {
        // Keep only names that exist and preserve any not in the list at the end
        let mut new_order = Vec::new();
        let mut used = std::collections::HashSet::new();

        for name in names {
            if self.objects.contains_key(*name) && !used.contains(*name) {
                new_order.push((*name).to_string());
                used.insert(*name);
            }
        }

        // Add any remaining objects at the end
        for name in &self.render_order {
            if !used.contains(name.as_str()) {
                new_order.push(name.clone());
            }
        }

        self.render_order = new_order;
    }

    /// Move an object to the front of the render order
    pub fn bring_to_front(&mut self, name: &str) {
        if let Some(pos) = self.render_order.iter().position(|n| n == name) {
            let name = self.render_order.remove(pos);
            self.render_order.push(name);
        }
    }

    /// Move an object to the back of the render order
    pub fn send_to_back(&mut self, name: &str) {
        if let Some(pos) = self.render_order.iter().position(|n| n == name) {
            let name = self.render_order.remove(pos);
            self.render_order.insert(0, name);
        }
    }

    /// Compute the bounding box of all enabled objects
    pub fn extent(&self) -> Option<(Vec3, Vec3)> {
        self.extent_with_options(MeasurementResolveOptions::default())
    }

    /// Computes enabled-object bounds with explicit measurement options.
    pub fn extent_with_options(&self, options: MeasurementResolveOptions) -> Option<(Vec3, Vec3)> {
        let mut min = Vec3::new(f32::MAX, f32::MAX, f32::MAX);
        let mut max = Vec3::new(f32::MIN, f32::MIN, f32::MIN);
        let mut has_extent = false;

        for name in &self.render_order {
            let Some(object) = self.get(name) else {
                continue;
            };
            if object.is_enabled() {
                if let Some((obj_min, obj_max)) = self.object_extent_with_options(name, options) {
                    min.x = min.x.min(obj_min.x);
                    min.y = min.y.min(obj_min.y);
                    min.z = min.z.min(obj_min.z);
                    max.x = max.x.max(obj_max.x);
                    max.y = max.y.max(obj_max.y);
                    max.z = max.z.max(obj_max.z);
                    has_extent = true;
                }
            }
        }

        if has_extent {
            Some((min, max))
        } else {
            None
        }
    }

    /// Get the center of all enabled objects
    pub fn center(&self) -> Option<Vec3> {
        let (min, max) = self.extent()?;
        Some(Vec3::new(
            (min.x + max.x) * 0.5,
            (min.y + max.y) * 0.5,
            (min.z + max.z) * 0.5,
        ))
    }

    /// Rename an object.
    ///
    /// # Errors
    ///
    /// Returns an error if `old_name` does not exist or `new_name` already exists.
    pub fn rename(&mut self, old_name: &str, new_name: &str) -> SceneResult<()> {
        if !self.objects.contains_key(old_name) {
            return Err(SceneError::object_not_found(old_name));
        }
        if self.objects.contains_key(new_name) {
            return Err(SceneError::object_exists(new_name));
        }

        if let Some(mut obj) = self.objects.remove(old_name) {
            // Update the internal name field
            obj.set_name(new_name.to_string());
            self.objects.insert(new_name.to_string(), obj);
            if let Some(id) = self.render_ids.remove(old_name) {
                self.render_ids.insert(new_name.to_string(), id);
            }

            // Update render order
            for name in &mut self.render_order {
                if name == old_name {
                    *name = new_name.to_string();
                }
            }
        }

        // Update group children references
        let old = old_name.to_string();
        let new = new_name.to_string();
        for obj in self.objects.values_mut() {
            if let Some(group) = Self::object_as_mut::<GroupObject>(obj.as_mut()) {
                for child in group.children_mut() {
                    if *child == old {
                        *child = new.clone();
                    }
                }
            } else if let Some(measurement) = Self::object_as_mut::<MeasurementObject>(obj.as_mut())
            {
                measurement.rename_anchors_to(old_name, new_name);
            } else if let Some(label) = Self::object_as_mut::<LabelObject>(obj.as_mut()) {
                label.rename_anchors_to(old_name, new_name);
            }
        }

        self.generation += 1;
        Ok(())
    }

    /// Clear all objects
    pub fn clear(&mut self) {
        if !self.objects.is_empty() {
            self.generation += 1;
        }
        self.objects.clear();
        self.render_order.clear();
        self.render_ids.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn molecule_with_atoms(name: &str, count: usize) -> MoleculeObject {
        let mut molecule = patinae_mol::ObjectMolecule::new(name);
        for _ in 0..count {
            molecule.add_atom(patinae_mol::Atom::new("CA", patinae_mol::Element::Carbon));
        }
        MoleculeObject::from_raw(molecule)
    }

    // Mock object for testing
    struct MockObject {
        name: String,
        state: ObjectState,
        object_type: ObjectType,
    }

    impl MockObject {
        fn new(name: &str) -> Self {
            Self::new_with_type(name, ObjectType::Molecule)
        }

        fn new_with_type(name: &str, object_type: ObjectType) -> Self {
            Self {
                name: name.to_string(),
                state: ObjectState::default(),
                object_type,
            }
        }
    }

    impl Object for MockObject {
        fn name(&self) -> &str {
            &self.name
        }

        fn object_type(&self) -> ObjectType {
            self.object_type
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

    #[test]
    fn test_registry_add_remove() {
        let mut registry = ObjectRegistry::new();

        registry.add(MockObject::new("obj1"));
        registry.add(MockObject::new("obj2"));

        assert_eq!(registry.len(), 2);
        assert!(registry.contains("obj1"));
        assert!(registry.contains("obj2"));

        registry.remove("obj1");
        assert_eq!(registry.len(), 1);
        assert!(!registry.contains("obj1"));
    }

    #[test]
    fn type_specific_getters_ignore_false_molecule_metadata() {
        let mut registry = ObjectRegistry::new();
        registry.add(MockObject::new("fake_molecule"));

        assert_eq!(
            registry.get("fake_molecule").unwrap().object_type(),
            ObjectType::Molecule
        );
        assert!(registry.get_molecule("fake_molecule").is_none());
        assert!(registry.get_molecule_mut("fake_molecule").is_none());

        let snapshot = registry.to_snapshot();
        assert!(snapshot.molecules.is_empty());
        assert_eq!(snapshot.object_states.len(), 1);
    }

    #[test]
    fn group_traversal_ignores_false_group_metadata() {
        let mut registry = ObjectRegistry::new();
        registry.add(MockObject::new("child"));
        registry.add(MockObject::new_with_type("fake_group", ObjectType::Group));
        registry.add(GroupObject::new("real_group"));
        registry.add_to_group("real_group", "child");

        assert_eq!(registry.parent_group("child"), Some("real_group"));
        assert!(registry.top_level_names().contains(&"fake_group"));

        registry.rename("child", "renamed_child").unwrap();
        assert!(registry
            .get_group("real_group")
            .unwrap()
            .contains_child("renamed_child"));

        registry.remove_from_groups("renamed_child");
        assert!(!registry
            .get_group("real_group")
            .unwrap()
            .contains_child("renamed_child"));

        let snapshot = registry.to_snapshot();
        assert_eq!(snapshot.groups.len(), 1);
        assert_eq!(snapshot.groups[0].0, "real_group");
    }

    #[test]
    fn test_registry_render_order() {
        let mut registry = ObjectRegistry::new();

        registry.add(MockObject::new("a"));
        registry.add(MockObject::new("b"));
        registry.add(MockObject::new("c"));

        let names: Vec<_> = registry.names().collect();
        assert_eq!(names, vec!["a", "b", "c"]);

        registry.bring_to_front("a");
        let names: Vec<_> = registry.names().collect();
        assert_eq!(names, vec!["b", "c", "a"]);

        registry.send_to_back("a");
        let names: Vec<_> = registry.names().collect();
        assert_eq!(names, vec!["a", "b", "c"]);
    }

    #[test]
    fn render_object_id_rejects_values_outside_picking_range() {
        assert!(RenderObjectId::new(0).is_none());
        assert_eq!(RenderObjectId::FIRST.get(), 1);
        assert_eq!(RenderObjectId::FIRST.slot_index(), 0);
        assert_eq!(
            RenderObjectId::new(RenderObjectId::MAX_PICKING_ID)
                .unwrap()
                .get(),
            RenderObjectId::MAX_PICKING_ID
        );
        assert!(RenderObjectId::new(RenderObjectId::MAX_PICKING_ID + 1).is_none());
    }

    #[test]
    fn annotation_object_types_do_not_enter_picking() {
        assert!(!ObjectType::Measurement.is_pickable());
        assert!(!ObjectType::Label.is_pickable());
        assert!(ObjectType::Molecule.is_pickable());
    }

    #[test]
    fn render_ids_survive_reorder_and_rename_without_reuse() {
        let mut registry = ObjectRegistry::new();

        registry.add(MockObject::new("a"));
        registry.add(MockObject::new("b"));
        registry.add(MockObject::new("c"));

        let a_id = registry.render_id("a").unwrap();
        let b_id = registry.render_id("b").unwrap();
        let c_id = registry.render_id("c").unwrap();

        registry.enable("b", false).unwrap();
        registry.enable("b", true).unwrap();
        assert_eq!(registry.render_id("b"), Some(b_id));

        registry.bring_to_front("a");
        registry.send_to_back("a");
        registry.set_render_order(&["c", "a", "b"]);
        assert_eq!(registry.render_id("a"), Some(a_id));
        assert_eq!(registry.render_id("b"), Some(b_id));
        assert_eq!(registry.render_id("c"), Some(c_id));

        registry.rename("c", "renamed_c").unwrap();
        assert_eq!(registry.render_id("c"), None);
        assert_eq!(registry.render_id("renamed_c"), Some(c_id));

        registry.remove("b");
        registry.add(MockObject::new("d"));
        assert_ne!(registry.render_id("d"), Some(b_id));
    }

    #[test]
    fn label_objects_round_trip_with_order_group_state_and_render_id() {
        let mut registry = ObjectRegistry::new();
        registry.add(molecule_with_atoms("source", 2));
        let mut labels = LabelObject::with_entities(
            "labels",
            vec![
                LabelEntity::new(
                    AtomAnchor::new("source", patinae_mol::AtomIndex(0)),
                    "first",
                ),
                LabelEntity::new(
                    AtomAnchor::new("source", patinae_mol::AtomIndex(1)),
                    "second",
                ),
                LabelEntity::new(
                    AtomAnchor::new("source", patinae_mol::AtomIndex(0)),
                    "duplicate",
                ),
            ],
        );
        labels.state_mut().enabled = false;
        labels.set_color(ColorIndex::Named(7));
        registry.add(labels);
        registry.add_to_group("annotations", "labels");
        let render_id = registry.render_id("labels");
        let revisions = registry.get_label("labels").unwrap().revisions();

        let restored = ObjectRegistry::from_snapshot(registry.to_snapshot());
        let labels = restored.get_label("labels").unwrap();

        assert_eq!(
            labels
                .entities()
                .iter()
                .map(LabelEntity::text)
                .collect::<Vec<_>>(),
            ["first", "second", "duplicate"]
        );
        assert!(!labels.state().enabled);
        assert_eq!(labels.presentation().color(), Some(ColorIndex::Named(7)));
        assert_eq!(labels.revisions(), revisions);
        assert_eq!(restored.render_id("labels"), render_id);
        assert_eq!(restored.parent_group("labels"), Some("annotations"));
    }

    #[test]
    fn bulk_label_object_visibility_coalesces_generation_and_noops() {
        let mut registry = ObjectRegistry::new();
        registry.add(LabelObject::new("first"));
        registry.add(LabelObject::new("second"));
        registry.add(GroupObject::new("other"));
        let before = registry.generation();

        assert_eq!(registry.set_label_objects_enabled(false), 2);
        assert_eq!(registry.generation(), before + 1);
        assert!(!registry.get_label("first").unwrap().is_enabled());
        assert!(!registry.get_label("second").unwrap().is_enabled());

        let unchanged = registry.generation();
        assert_eq!(registry.set_label_objects_enabled(false), 2);
        assert_eq!(registry.generation(), unchanged);
    }

    #[test]
    fn source_lifecycle_updates_both_annotation_types_without_rebinding() {
        let mut registry = ObjectRegistry::new();
        registry.add(molecule_with_atoms("source", 2));
        let indices = [0, 1].map(patinae_mol::AtomIndex);
        let mut measurement = MeasurementObject::new("distance", MeasurementKind::Distance);
        measurement
            .add_entry(MeasurementEntity::new(vec![
                AtomAnchor::new("source", indices[0]),
                AtomAnchor::new("source", indices[1]),
            ]))
            .unwrap();
        registry.add(measurement);
        registry.add(LabelObject::with_entities(
            "labels",
            vec![LabelEntity::new(
                AtomAnchor::new("source", indices[0]),
                "CA",
            )],
        ));
        let label_id = registry.render_id("labels");

        registry.rename("labels", "renamed_labels").unwrap();
        assert_eq!(registry.render_id("renamed_labels"), label_id);
        registry.rename("source", "renamed_source").unwrap();
        assert_eq!(
            registry.get_measurement("distance").unwrap().entries()[0].anchors[0].object_name,
            "renamed_source"
        );
        assert_eq!(
            registry.get_label("renamed_labels").unwrap().entities()[0]
                .anchor()
                .object_name,
            "renamed_source"
        );

        registry.remove("renamed_source");
        registry.add(molecule_with_atoms("renamed_source", 2));

        assert!(
            registry.get_measurement("distance").unwrap().entries()[0].anchors[0].is_orphaned()
        );
        assert!(registry.get_label("renamed_labels").unwrap().entities()[0]
            .anchor()
            .is_orphaned());
    }

    #[test]
    fn removed_atoms_orphan_matching_anchors_in_one_revision_step() {
        let mut registry = ObjectRegistry::new();
        registry.add(molecule_with_atoms("source", 3));
        let indices = [0, 1, 2].map(patinae_mol::AtomIndex);
        let mut measurement = MeasurementObject::new("distance", MeasurementKind::Distance);
        measurement
            .add_entry(MeasurementEntity::new(vec![
                AtomAnchor::new("source", indices[0]),
                AtomAnchor::new("source", indices[1]),
            ]))
            .unwrap();
        registry.add(measurement);
        registry.add(LabelObject::with_entities(
            "labels",
            vec![
                LabelEntity::new(AtomAnchor::new("source", indices[0]), "removed"),
                LabelEntity::new(AtomAnchor::new("source", indices[1]), "also removed"),
                LabelEntity::new(AtomAnchor::new("source", indices[2]), "retained"),
            ],
        ));
        let generation_before = registry.generation();
        let measurement_revision_before = registry
            .get_measurement("distance")
            .unwrap()
            .revisions()
            .geometry;
        let label_revision_before = registry.get_label("labels").unwrap().revisions().geometry;

        registry
            .remove_molecule_atoms(
                "source",
                &[patinae_mol::AtomIndex(0), patinae_mol::AtomIndex(1)],
            )
            .unwrap();

        let measurement = registry.get_measurement("distance").unwrap();
        assert!(measurement.entries()[0].anchors[0].is_orphaned());
        assert!(measurement.entries()[0].anchors[1].is_orphaned());
        assert_eq!(
            measurement.revisions().geometry,
            measurement_revision_before + 1
        );
        let labels = registry.get_label("labels").unwrap();
        assert!(labels.entities()[0].anchor().is_orphaned());
        assert!(labels.entities()[1].anchor().is_orphaned());
        assert!(!labels.entities()[2].anchor().is_orphaned());
        assert_eq!(
            labels.entities()[2].anchor().atom_index,
            patinae_mol::AtomIndex(0)
        );
        assert_eq!(labels.revisions().geometry, label_revision_before + 1);
        assert_eq!(registry.generation(), generation_before + 1);

        let generation_after = registry.generation();
        let measurement_revisions_after = registry.get_measurement("distance").unwrap().revisions();
        let label_revisions_after = registry.get_label("labels").unwrap().revisions();
        let unrelated_remap = {
            let mut molecule = patinae_mol::ObjectMolecule::new("temporary");
            molecule.add_atom(patinae_mol::Atom::new("A", patinae_mol::Element::Carbon));
            molecule.add_atom(patinae_mol::Atom::new("B", patinae_mol::Element::Carbon));
            molecule.remove_atoms(&[patinae_mol::AtomIndex(0)])
        };
        registry.remap_atom_anchors("unrelated", &unrelated_remap);

        assert_eq!(registry.generation(), generation_after);
        assert_eq!(
            registry.get_measurement("distance").unwrap().revisions(),
            measurement_revisions_after
        );
        assert_eq!(
            registry.get_label("labels").unwrap().revisions(),
            label_revisions_after
        );
    }

    #[test]
    fn registry_atom_removal_orphans_dependencies_and_deletes_empty_sources() {
        let mut registry = ObjectRegistry::new();
        registry.add(molecule_with_atoms("source", 2));
        registry.add(LabelObject::with_entities(
            "labels",
            vec![LabelEntity::new(
                AtomAnchor::new("source", patinae_mol::AtomIndex(0)),
                "first",
            )],
        ));

        registry
            .remove_molecule_atoms("source", &[patinae_mol::AtomIndex(0)])
            .unwrap();
        assert_eq!(
            registry
                .get_molecule("source")
                .unwrap()
                .molecule()
                .atom_count(),
            1
        );
        assert!(registry.get_label("labels").unwrap().entities()[0]
            .anchor()
            .is_orphaned());

        registry
            .remove_molecule_atoms("source", &[patinae_mol::AtomIndex(0)])
            .unwrap();
        assert!(registry.get_molecule("source").is_none());
    }

    #[test]
    fn render_ids_wrap_and_reuse_only_retired_ids() {
        let mut registry = ObjectRegistry::new();
        registry.add(MockObject::new("retired"));
        let retired_id = registry.render_id("retired").unwrap();
        registry.remove("retired");
        registry.next_render_id =
            ObjectRegistry::normalize_next_render_id(NEXT_AFTER_MAX_RENDER_ID);

        registry.add(MockObject::new("replacement"));

        assert_eq!(registry.render_id("replacement"), Some(retired_id));
    }

    #[test]
    fn render_ids_restore_old_snapshots_without_saved_ids() {
        let mut registry = ObjectRegistry::new();
        registry.add(GroupObject::new("a"));
        registry.add(GroupObject::new("b"));
        registry.add(GroupObject::new("c"));

        let mut snapshot = registry.to_snapshot();
        snapshot.render_ids.clear();
        snapshot.next_render_id = default_next_render_id();

        let restored = ObjectRegistry::from_snapshot(snapshot);

        assert_eq!(restored.render_id("a").map(RenderObjectId::get), Some(1));
        assert_eq!(restored.render_id("b").map(RenderObjectId::get), Some(2));
        assert_eq!(restored.render_id("c").map(RenderObjectId::get), Some(3));
    }

    #[test]
    fn render_ids_restore_rejects_duplicate_zero_and_out_of_range_ids() {
        let mut registry = ObjectRegistry::new();
        registry.add(GroupObject::new("a"));
        registry.add(GroupObject::new("b"));
        registry.add(GroupObject::new("c"));
        registry.add(GroupObject::new("d"));

        let mut snapshot = registry.to_snapshot();
        snapshot.render_ids = vec![
            ("a".to_string(), 1),
            ("b".to_string(), 1),
            ("c".to_string(), 0),
            ("d".to_string(), RenderObjectId::MAX_PICKING_ID + 1),
        ];
        snapshot.next_render_id = default_next_render_id();

        let restored = ObjectRegistry::from_snapshot(snapshot);
        let ids: Vec<_> = ["a", "b", "c", "d"]
            .into_iter()
            .map(|name| restored.render_id(name).unwrap().get())
            .collect();
        let unique: AHashSet<_> = ids.iter().copied().collect();

        assert_eq!(restored.render_id("a").map(RenderObjectId::get), Some(1));
        assert_eq!(unique.len(), ids.len());
        assert!(ids
            .iter()
            .all(|id| (1..=RenderObjectId::MAX_PICKING_ID).contains(id)));
    }

    #[test]
    fn test_registry_matching() {
        let mut registry = ObjectRegistry::new();

        registry.add(MockObject::new("protein"));
        registry.add(MockObject::new("protein_chain_A"));
        registry.add(MockObject::new("ligand"));

        let matches = registry.matching("protein*");
        assert_eq!(matches.len(), 2);

        let matches = registry.matching("*chain_A");
        assert_eq!(matches.len(), 1);

        let matches = registry.matching("*");
        assert_eq!(matches.len(), 3);
    }

    #[test]
    fn test_enable_disable() {
        let mut registry = ObjectRegistry::new();
        registry.add(MockObject::new("obj1"));
        registry.add(MockObject::new("obj2"));

        registry.disable_all();
        assert!(registry.enabled_objects().next().is_none());

        registry.enable("obj1", true).unwrap();
        assert_eq!(registry.enabled_objects().count(), 1);
    }

    #[test]
    fn test_add_to_group() {
        let mut registry = ObjectRegistry::new();
        registry.add(MockObject::new("obj1"));
        registry.add(MockObject::new("obj2"));
        registry.add(MockObject::new("obj3"));

        // Create group by adding to it
        assert!(registry.add_to_group("grp", "obj1"));
        assert!(registry.add_to_group("grp", "obj2"));

        // Group should exist and contain children
        let group = registry.get_group("grp").unwrap();
        assert!(group.contains_child("obj1"));
        assert!(group.contains_child("obj2"));
        assert!(!group.contains_child("obj3"));

        // Children should follow group in render order
        let names: Vec<_> = registry.names().collect();
        let grp_pos = names.iter().position(|n| *n == "grp").unwrap();
        let obj1_pos = names.iter().position(|n| *n == "obj1").unwrap();
        let obj2_pos = names.iter().position(|n| *n == "obj2").unwrap();
        assert!(obj1_pos == grp_pos + 1);
        assert!(obj2_pos == grp_pos + 2);
    }

    #[test]
    fn test_add_to_group_prevents_self() {
        let mut registry = ObjectRegistry::new();
        registry.add(GroupObject::new("grp"));
        assert!(!registry.add_to_group("grp", "grp"));
    }

    #[test]
    fn test_add_to_group_moves_between_groups() {
        let mut registry = ObjectRegistry::new();
        registry.add(MockObject::new("obj1"));
        registry.add(GroupObject::new("grp1"));
        registry.add(GroupObject::new("grp2"));

        registry.add_to_group("grp1", "obj1");
        assert!(registry.get_group("grp1").unwrap().contains_child("obj1"));

        // Move to different group
        registry.add_to_group("grp2", "obj1");
        assert!(!registry.get_group("grp1").unwrap().contains_child("obj1"));
        assert!(registry.get_group("grp2").unwrap().contains_child("obj1"));
    }

    #[test]
    fn test_parent_group() {
        let mut registry = ObjectRegistry::new();
        registry.add(MockObject::new("obj1"));
        registry.add(MockObject::new("obj2"));

        assert!(registry.parent_group("obj1").is_none());

        registry.add_to_group("grp", "obj1");
        assert_eq!(registry.parent_group("obj1"), Some("grp"));
        assert!(registry.parent_group("obj2").is_none());
    }

    #[test]
    fn test_remove_cleans_up_groups() {
        let mut registry = ObjectRegistry::new();
        registry.add(MockObject::new("obj1"));
        registry.add_to_group("grp", "obj1");

        assert!(registry.get_group("grp").unwrap().contains_child("obj1"));

        registry.remove("obj1");
        assert!(!registry.get_group("grp").unwrap().contains_child("obj1"));
    }

    #[test]
    fn test_top_level_names() {
        let mut registry = ObjectRegistry::new();
        registry.add(MockObject::new("obj1"));
        registry.add(MockObject::new("obj2"));
        registry.add(MockObject::new("obj3"));

        // All are top-level initially
        assert_eq!(registry.top_level_names().len(), 3);

        // Add obj1 and obj2 to a group
        registry.add_to_group("grp", "obj1");
        registry.add_to_group("grp", "obj2");

        let top = registry.top_level_names();
        assert!(top.contains(&"grp"));
        assert!(top.contains(&"obj3"));
        assert!(!top.contains(&"obj1"));
        assert!(!top.contains(&"obj2"));
        assert_eq!(top.len(), 2);
    }

    #[test]
    fn first_free_measurement_names_are_kind_specific_and_share_namespace() {
        let mut registry = ObjectRegistry::new();
        registry.add(MockObject::new("distance01"));
        registry.add(MockObject::new("angle01"));
        registry.add(MockObject::new("dihedral01"));
        registry.add(MockObject::new("distance03"));

        assert_eq!(
            registry.first_free_measurement_name(MeasurementKind::Distance),
            "distance02"
        );
        assert_eq!(
            registry.first_free_measurement_name(MeasurementKind::Angle),
            "angle02"
        );
        assert_eq!(
            registry.first_free_measurement_name(MeasurementKind::Dihedral),
            "dihedral02"
        );
    }
}
