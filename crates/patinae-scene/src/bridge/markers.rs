//! Per-atom marker pre-resolution. Owns the buffers so `RenderInput`'s
//! borrowed slices stay valid during `RenderState::sync`.
//!
//! Markers carry UI state (selected / hover / recent atom)
//! per atom. Bit layout lives in `patinae_render::scene_store::marker`;
//! this bridge sets bits 0 (selected), 1 (hover), and 2 (recent atom). Indexed by
//! object-local atom id; the renderer offsets to global on upload.
//!
//! Replaces the old `apply_highlight()` flow which wrote to a flat,
//! object-aliased `SelectionState` bitset (multi-object scenes had atom
//! id N in object A and atom id N in object B sharing the same bit). The
//! renderer's scene-wide `marker_lut` is indexed by **global** atom id,
//! so per-object markers compose without aliasing.

use std::collections::{hash_map::DefaultHasher, HashMap, HashSet};
use std::hash::{Hash, Hasher};

use patinae_mol::DirtyFlags;
pub use patinae_render::scene_store::marker::MARKER_RECENT;
use patinae_render::MarkerUpdate;
use patinae_select::SelectionOptions;

use crate::object::{Object, ObjectRegistry};
use crate::selection::SelectionManager;
use crate::session::HoverTarget;

/// Marker bit 0 — atom is in some visible host selection.
pub const MARKER_SELECTED: u32 = 1 << 0;
/// Marker bit 1 — atom is the current hover target.
pub const MARKER_HOVER: u32 = 1 << 1;
const MARKER_SCREEN_OVERLAY: u32 = MARKER_SELECTED | MARKER_HOVER;

/// Pre-packed per-atom marker bits keyed by object name. Same caching
/// shape as [`super::ResolvedSceneColors`] — owned `Vec`s persist between
/// frames; only objects with a marker-state change get repacked.
#[derive(Default)]
pub struct ResolvedSceneMarkers {
    by_object: HashMap<String, Vec<u32>>,
    hashes: HashMap<String, u64>,
    dirty_objects: HashSet<String>,
    dirty_flags_by_object: HashMap<String, DirtyFlags>,
    updates_by_object: HashMap<String, Vec<MarkerUpdate>>,
    screen_overlay_marker_counts: HashMap<String, usize>,
    last_selection_atoms: Vec<(String, u32)>,
    last_recent_atoms: Vec<(String, u32)>,
    last_hover_atoms: Vec<(String, u32)>,
    last_registry_generation: Option<u64>,
    last_selection_generation: Option<u64>,
    last_recent_observation: Option<(u64, u64)>,
    last_hover_hash: u64,
}

impl ResolvedSceneMarkers {
    pub fn build(
        selections: &mut SelectionManager,
        registry: &ObjectRegistry,
        hover: Option<&HoverTarget>,
    ) -> Self {
        let mut s = Self::default();
        s.rebuild(selections, registry, hover);
        s
    }

    /// In-place rebuild. Drops entries for objects that are gone / disabled
    /// / coordless, then walks visible selections + hover and ORs their
    /// bits into per-object slices.
    ///
    /// Takes split borrows of `Session` (`selections: &mut`, `registry: &`,
    /// `hover: Option<&...>`) so the host can call us while still holding
    /// other immutable borrows on `Session`.
    pub fn rebuild(
        &mut self,
        selections: &mut SelectionManager,
        registry: &ObjectRegistry,
        hover: Option<&HoverTarget>,
    ) {
        self.rebuild_with_recent(selections, registry, &[], (0, 0), hover);
    }

    /// Rebuilds marker state including resolved recent atom targets.
    pub fn rebuild_with_recent(
        &mut self,
        selections: &mut SelectionManager,
        registry: &ObjectRegistry,
        recent_atoms: &[(String, patinae_mol::AtomIndex)],
        recent_observation: (u64, u64),
        hover: Option<&HoverTarget>,
    ) {
        self.dirty_objects.clear();
        self.dirty_flags_by_object.clear();
        self.updates_by_object.clear();
        let registry_generation = registry.generation();
        let selection_generation = selections.generation();
        let hover_hash = hash_hover(hover);
        let registry_unchanged = self.last_registry_generation == Some(registry_generation);
        let selection_unchanged = self.last_selection_generation == Some(selection_generation);
        let recent_unchanged = self.last_recent_observation == Some(recent_observation);
        let cache_matches_registry = self.cache_matches_registry(registry);
        if registry_unchanged && selection_unchanged && cache_matches_registry {
            if !recent_unchanged {
                self.rebuild_recent_only(recent_atoms, recent_observation);
            }
            if self.last_hover_hash == hover_hash {
                return;
            }
            self.rebuild_hover_only(hover, hover_hash);
            self.invalidate_dirty_hashes();
            return;
        }

        if registry_unchanged && recent_unchanged && cache_matches_registry {
            self.rebuild_selection_only(
                selections,
                registry,
                hover,
                selection_generation,
                hover_hash,
            );
            self.last_registry_generation = Some(registry_generation);
            return;
        }

        if !self.needs_rebuild(
            registry,
            registry_generation,
            selection_generation,
            recent_observation,
            hover_hash,
        ) {
            return;
        }

        self.by_object.retain(|name, _| {
            registry
                .get_molecule(name)
                .map(|mol_obj| mol_obj.state().enabled && mol_obj.display_coord_set().is_some())
                .unwrap_or(false)
        });
        let by_object = &self.by_object;
        self.hashes.retain(|name, _| by_object.contains_key(name));
        self.screen_overlay_marker_counts
            .retain(|name, _| by_object.contains_key(name));
        self.dirty_flags_by_object
            .retain(|name, _| by_object.contains_key(name));

        // Allocate / clear per-object marker buffers up front. Objects with
        // no selection or hover hits keep an all-zero slice.
        for name in registry.names() {
            let Some(mol_obj) = registry.get_molecule(name) else {
                continue;
            };
            if !mol_obj.state().enabled {
                continue;
            }
            if mol_obj.display_coord_set().is_none() {
                continue;
            }
            let n_atoms = mol_obj.molecule().atom_count();
            let entry = self.by_object.entry(name.to_string()).or_default();
            entry.clear();
            entry.resize(n_atoms, 0);
        }

        // Selection bits — `evaluate_visible` returns one (object_name,
        // SelectionResult) pair per visible host selection.
        self.last_selection_atoms.clear();
        let visible = selections.evaluate_visible(registry, SelectionOptions::default());
        for (obj_name, sel_result) in &visible {
            let Some(buf) = self.by_object.get_mut(obj_name) else {
                continue;
            };
            for atom in sel_result.indices() {
                let id = atom.as_u32() as usize;
                if id < buf.len() && (buf[id] & MARKER_SELECTED) == 0 {
                    buf[id] |= MARKER_SELECTED;
                    self.last_selection_atoms
                        .push((obj_name.clone(), atom.as_u32()));
                }
            }
        }

        self.last_recent_atoms.clear();
        for (object_name, atom_index) in recent_atoms {
            let Some(buf) = self.by_object.get_mut(object_name) else {
                continue;
            };
            let id = atom_index.as_usize();
            if id < buf.len() {
                buf[id] |= MARKER_RECENT;
                self.last_recent_atoms
                    .push((object_name.clone(), atom_index.as_u32()));
            }
        }

        // Hover — at most one atom per scene, scoped to a specific object.
        self.last_hover_atoms.clear();
        if let Some(hover) = hover {
            if let Some(buf) = self.by_object.get_mut(&hover.object) {
                for atom in hover.selection.indices() {
                    let id = atom.as_u32() as usize;
                    if id < buf.len() {
                        buf[id] |= MARKER_HOVER;
                        self.last_hover_atoms
                            .push((hover.object.clone(), atom.as_u32()));
                    }
                }
            }
        }

        for (name, buf) in &self.by_object {
            let hash = marker_hash(buf);
            if self.hashes.get(name).copied() != Some(hash) {
                self.dirty_objects.insert(name.clone());
                self.dirty_flags_by_object
                    .insert(name.clone(), DirtyFlags::SELECTION | DirtyFlags::HOVER);
            }
            self.hashes.insert(name.clone(), hash);
            self.screen_overlay_marker_counts.insert(
                name.clone(),
                buf.iter()
                    .filter(|&&bits| bits & MARKER_SCREEN_OVERLAY != 0)
                    .count(),
            );
        }
        self.last_registry_generation = Some(registry_generation);
        self.last_selection_generation = Some(selection_generation);
        self.last_recent_observation = Some(recent_observation);
        self.last_hover_hash = hover_hash;
    }

    pub fn get(&self, name: &str) -> Option<&[u32]> {
        self.by_object.get(name).map(|v| v.as_slice())
    }

    pub fn updates(&self, name: &str) -> Option<&[MarkerUpdate]> {
        self.updates_by_object.get(name).map(|v| v.as_slice())
    }

    /// Returns whether an object needs the selection and hover screen overlay.
    pub fn has_markers(&self, name: &str) -> bool {
        self.screen_overlay_marker_counts
            .get(name)
            .copied()
            .unwrap_or(0)
            > 0
    }

    pub fn dirty_flags(&self, name: &str) -> DirtyFlags {
        self.dirty_flags_by_object
            .get(name)
            .copied()
            .unwrap_or_else(DirtyFlags::empty)
    }

    pub fn is_dirty(&self, name: &str) -> bool {
        self.dirty_objects.contains(name)
    }

    fn rebuild_hover_only(&mut self, hover: Option<&HoverTarget>, hover_hash: u64) {
        let previous = std::mem::take(&mut self.last_hover_atoms);
        for (object, atom_index) in previous {
            if let Some(bits) = self.marker_bits(&object, atom_index) {
                self.set_marker_bits(&object, atom_index, bits & !MARKER_HOVER);
            }
        }

        if let Some(hover) = hover {
            let object = hover.object.as_str();
            for atom in hover.selection.indices() {
                let atom_index = atom.as_u32();
                if let Some(bits) = self.marker_bits(object, atom_index) {
                    self.set_marker_bits(object, atom_index, bits | MARKER_HOVER);
                    self.last_hover_atoms
                        .push((hover.object.clone(), atom_index));
                }
            }
        }

        self.last_hover_hash = hover_hash;
    }

    fn rebuild_recent_only(
        &mut self,
        recent_atoms: &[(String, patinae_mol::AtomIndex)],
        recent_observation: (u64, u64),
    ) {
        let previous = std::mem::take(&mut self.last_recent_atoms);
        let next = recent_atoms
            .iter()
            .filter_map(|(object, atom_index)| {
                let atom_index = atom_index.as_u32();
                self.marker_bits(object, atom_index)
                    .is_some()
                    .then(|| (object.clone(), atom_index))
            })
            .collect::<Vec<_>>();
        {
            let previous_set = previous
                .iter()
                .map(|(object, atom_index)| (object.as_str(), *atom_index))
                .collect::<HashSet<_>>();
            let next_set = next
                .iter()
                .map(|(object, atom_index)| (object.as_str(), *atom_index))
                .collect::<HashSet<_>>();

            for (object, atom_index) in &previous {
                if !next_set.contains(&(object.as_str(), *atom_index)) {
                    if let Some(bits) = self.marker_bits(object, *atom_index) {
                        self.set_marker_bits(object, *atom_index, bits & !MARKER_RECENT);
                    }
                }
            }

            for (object, atom_index) in &next {
                if !previous_set.contains(&(object.as_str(), *atom_index)) {
                    if let Some(bits) = self.marker_bits(object, *atom_index) {
                        self.set_marker_bits(object, *atom_index, bits | MARKER_RECENT);
                    }
                }
            }
        }

        self.last_recent_atoms = next;
        self.invalidate_dirty_hashes();
        self.last_recent_observation = Some(recent_observation);
    }

    fn rebuild_selection_only(
        &mut self,
        selections: &mut SelectionManager,
        registry: &ObjectRegistry,
        hover: Option<&HoverTarget>,
        selection_generation: u64,
        hover_hash: u64,
    ) {
        let previous = std::mem::take(&mut self.last_selection_atoms);
        for (object, atom_index) in previous {
            if let Some(bits) = self.marker_bits(&object, atom_index) {
                self.set_marker_bits(&object, atom_index, bits & !MARKER_SELECTED);
            }
        }

        let visible = selections.evaluate_visible(registry, SelectionOptions::default());
        for (obj_name, sel_result) in &visible {
            for atom in sel_result.indices() {
                let atom_index = atom.as_u32();
                if let Some(bits) = self.marker_bits(obj_name, atom_index) {
                    if bits & MARKER_SELECTED == 0 {
                        self.set_marker_bits(obj_name, atom_index, bits | MARKER_SELECTED);
                        self.last_selection_atoms
                            .push((obj_name.clone(), atom_index));
                    }
                }
            }
        }

        if self.last_hover_hash != hover_hash {
            self.rebuild_hover_only(hover, hover_hash);
        } else {
            self.last_hover_hash = hover_hash;
        }
        self.invalidate_dirty_hashes();
        self.last_selection_generation = Some(selection_generation);
    }

    fn marker_bits(&self, object: &str, atom_index: u32) -> Option<u32> {
        self.by_object
            .get(object)
            .and_then(|buf| buf.get(atom_index as usize).copied())
    }

    fn set_marker_bits(&mut self, object: &str, atom_index: u32, bits: u32) {
        let Some(buf) = self.by_object.get_mut(object) else {
            return;
        };
        let Some(slot) = buf.get_mut(atom_index as usize) else {
            return;
        };
        let old = *slot;
        if old == bits {
            return;
        }
        *slot = bits;
        let count = self
            .screen_overlay_marker_counts
            .entry(object.to_string())
            .or_default();
        match (
            old & MARKER_SCREEN_OVERLAY == 0,
            bits & MARKER_SCREEN_OVERLAY == 0,
        ) {
            (true, false) => *count = count.saturating_add(1),
            (false, true) => *count = count.saturating_sub(1),
            _ => {}
        }
        self.dirty_objects.insert(object.to_string());
        let changed = old ^ bits;
        let mut dirty = DirtyFlags::empty();
        if changed & MARKER_SELECTED != 0 {
            dirty |= DirtyFlags::SELECTION;
        }
        if changed & MARKER_HOVER != 0 {
            dirty |= DirtyFlags::HOVER;
        }
        if changed & MARKER_RECENT != 0 {
            dirty |= DirtyFlags::SELECTION;
        }
        self.dirty_flags_by_object
            .entry(object.to_string())
            .and_modify(|flags| *flags |= dirty)
            .or_insert(dirty);
        self.updates_by_object
            .entry(object.to_string())
            .or_default()
            .push(MarkerUpdate { atom_index, bits });
    }

    fn invalidate_dirty_hashes(&mut self) {
        for object in &self.dirty_objects {
            self.hashes.remove(object);
        }
    }

    fn needs_rebuild(
        &self,
        registry: &ObjectRegistry,
        registry_generation: u64,
        selection_generation: u64,
        recent_observation: (u64, u64),
        hover_hash: u64,
    ) -> bool {
        if self.last_registry_generation != Some(registry_generation)
            || self.last_selection_generation != Some(selection_generation)
            || self.last_recent_observation != Some(recent_observation)
            || self.last_hover_hash != hover_hash
        {
            return true;
        }

        !self.cache_matches_registry(registry)
    }

    fn cache_matches_registry(&self, registry: &ObjectRegistry) -> bool {
        let mut alive_count = 0usize;
        for name in registry.names() {
            let Some(mol_obj) = registry.get_molecule(name) else {
                continue;
            };
            if !mol_obj.state().enabled || mol_obj.display_coord_set().is_none() {
                continue;
            }
            alive_count += 1;
            let Some(buf) = self.by_object.get(name) else {
                return false;
            };
            if buf.len() != mol_obj.molecule().atom_count() {
                return false;
            }
        }

        self.by_object.len() == alive_count
    }
}

fn marker_hash(markers: &[u32]) -> u64 {
    let mut h = DefaultHasher::new();
    markers.hash(&mut h);
    h.finish()
}

fn hash_hover(hover: Option<&HoverTarget>) -> u64 {
    let mut h = DefaultHasher::new();
    if let Some(hover) = hover {
        hover.object.hash(&mut h);
        for atom in hover.selection.indices() {
            atom.as_u32().hash(&mut h);
        }
    }
    h.finish()
}
