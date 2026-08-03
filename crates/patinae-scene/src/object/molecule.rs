//! Molecule object wrapper
//!
//! Wraps `ObjectMolecule` with render state and cached representations.

use lin_alg::f32::Vec3;
use patinae_mol::{Atom, AtomIndex, CoordSet, DirtyFlags, ObjectMolecule, RepMask};
use patinae_select::SelectionResult;
use patinae_settings::ObjectOverrides;

use super::{Object, ObjectState, ObjectType};

const MOLECULAR_REPS: RepMask = RepMask(RepMask::ALL.0 & !RepMask::LABELS.0);

/// A molecular object with render state.
///
/// Wraps `ObjectMolecule` and manages visual state (enabled, colour,
/// visible reps), per-object setting overrides, dirty flags. The actual
/// GPU representations live on `patinae_render::RenderState`; this
/// struct is pure host-side data.
pub struct MoleculeObject {
    /// The underlying molecular data
    molecule: ObjectMolecule,
    /// Visual state
    state: ObjectState,
    /// Current coordinate state index being displayed
    display_state: usize,
    /// Dirty flags indicating what needs rebuilding
    dirty: DirtyFlags,
    /// Per-object settings overrides
    overrides: Option<ObjectOverrides>,
    /// Surface quality (-4 to 4, default 0)
    surface_quality: i32,
}

impl MoleculeObject {
    /// Create a new molecule object
    pub fn new(mut molecule: ObjectMolecule) -> Self {
        let mut state = ObjectState::default();
        let mut object_reps = RepMask::NONE;
        for atom in molecule.atoms_mut() {
            let flags = atom.state.flags;
            atom.repr.visible_reps = if flags.is_solvent() {
                RepMask::DOTS
            } else if atom.element.is_metal() || flags.is_inorganic() {
                RepMask::SPHERES
            } else if flags.is_organic() {
                RepMask::STICKS
            } else {
                RepMask::CARTOON
            };
            object_reps = object_reps.union(atom.repr.visible_reps);
        }
        state.visible_reps = object_reps;
        state.draw_reps = object_reps;

        Self {
            molecule,
            state,
            display_state: 0,
            dirty: DirtyFlags::ALL,
            surface_quality: 0,
            overrides: None,
        }
    }

    /// Restore a molecule object from a snapshot without resetting per-atom representations.
    ///
    /// Unlike `new()`, this preserves all atom-level state (visible_reps, colors, etc.)
    /// exactly as stored in the molecule.
    pub fn from_raw(mut molecule: ObjectMolecule) -> Self {
        molecule.rebuild_atom_bonds();
        Self {
            molecule,
            state: ObjectState::default(),
            display_state: 0,
            dirty: DirtyFlags::ALL,
            surface_quality: 0,
            overrides: None,
        }
    }

    /// Create a named molecule object preserving per-atom representation state.
    ///
    /// Like `from_raw()`, this does NOT reset per-atom `visible_reps`. The object-level
    /// `visible_reps` is computed as the union of all per-atom reps so the renderer
    /// enables all needed representation pipelines.
    pub fn from_raw_with_name(mut molecule: ObjectMolecule, name: &str) -> Self {
        molecule.name = name.to_string();
        let mut obj_reps = RepMask::default();
        for atom in molecule.atoms() {
            obj_reps = obj_reps.union(atom.repr.visible_reps);
        }
        let state = ObjectState {
            visible_reps: obj_reps,
            draw_reps: obj_reps,
            ..Default::default()
        };
        Self {
            molecule,
            state,
            display_state: 0,
            dirty: DirtyFlags::ALL,
            surface_quality: 0,
            overrides: None,
        }
    }

    /// Create a molecule object with a specific name
    pub fn with_name(molecule: ObjectMolecule, name: &str) -> Self {
        let mut obj = Self::new(molecule);
        obj.molecule.name = name.to_string();
        obj
    }

    /// Get a reference to the underlying molecule data
    pub fn molecule(&self) -> &ObjectMolecule {
        &self.molecule
    }

    /// Get a mutable reference to the underlying molecule data
    ///
    /// Note: This will mark the object as dirty.
    pub fn molecule_mut(&mut self) -> &mut ObjectMolecule {
        self.dirty = DirtyFlags::ALL;
        &mut self.molecule
    }

    /// Get mutable molecule data while marking only the supplied dirty bits.
    pub fn molecule_mut_with_dirty(&mut self, dirty: DirtyFlags) -> &mut ObjectMolecule {
        self.dirty |= dirty;
        &mut self.molecule
    }

    /// Get the display state index
    pub fn display_state(&self) -> usize {
        self.display_state
    }

    /// Get the coordinate set for the state currently displayed by this object.
    pub fn display_coord_set(&self) -> Option<&CoordSet> {
        self.molecule.get_coord_set(self.display_state)
    }

    /// Get one atom coordinate from the state currently displayed by this object.
    pub fn display_coord(&self, atom: AtomIndex) -> Option<Vec3> {
        self.display_coord_set()?.get_atom_coord(atom)
    }

    /// Set the display state index
    pub fn set_display_state(&mut self, state: usize) -> bool {
        if state < self.molecule.state_count() {
            if self.display_state != state {
                self.display_state = state;
                self.dirty |= DirtyFlags::COORDS;
            }
            true
        } else {
            false
        }
    }

    /// Invalidate cached representations
    pub fn invalidate(&mut self, flags: DirtyFlags) {
        self.dirty |= flags;
    }

    /// Clear all dirty flags. Called by the host after it has consumed the
    /// flags (e.g. patinae-render finished its sync walk).
    pub fn clear_dirty(&mut self) {
        self.dirty = DirtyFlags::empty();
    }

    /// Invalidate all representations so they get rebuilt
    ///
    /// This is a convenience method for marking all representations as dirty,
    /// typically used when settings like transparency change.
    pub fn invalidate_representations(&mut self) {
        self.dirty |= DirtyFlags::REPS;
    }

    /// Check if the molecule needs rebuilding
    pub fn is_dirty(&self) -> bool {
        !self.dirty.is_empty()
    }

    /// Get the dirty flags
    pub fn dirty_flags(&self) -> DirtyFlags {
        self.dirty
    }

    /// Get the visible representations mask
    pub fn visible_reps(&self) -> RepMask {
        self.state.visible_reps
    }

    /// Get the object-level representation draw mask.
    pub fn draw_reps(&self) -> RepMask {
        self.state.draw_reps
    }

    /// Get representations currently effective for drawing.
    pub fn effective_reps(&self) -> RepMask {
        self.state.visible_reps.intersection(self.state.draw_reps)
    }

    /// Get representations currently effective for one atom.
    pub fn effective_atom_reps(&self, atom: &Atom) -> RepMask {
        atom.repr.visible_reps.intersection(self.effective_reps())
    }

    /// Get draw-mask reps that can be safely restored without atom rewrites.
    pub fn draw_mask_restorable_reps(&self) -> RepMask {
        self.state.draw_mask_restorable_reps
    }

    /// Check whether a draw-mask-only restore is known to be valid.
    pub fn can_restore_draw_mask(&self, rep: RepMask) -> bool {
        self.state.can_restore_draw_mask(rep)
    }

    /// Set the visible representations mask (e.g., copying from another object)
    pub fn set_visible_reps(&mut self, reps: RepMask) {
        let reps = reps.difference(RepMask::LABELS);
        self.state.visible_reps.set_hidden(MOLECULAR_REPS);
        self.state.visible_reps.set_visible(reps);
        self.state.draw_reps.set_hidden(MOLECULAR_REPS);
        self.state.draw_reps.set_visible(reps);
        self.state.draw_mask_restorable_reps = RepMask::NONE;
        self.dirty |= DirtyFlags::REPS;
    }

    /// Set the object-level representation draw mask.
    pub fn set_draw_reps(&mut self, reps: RepMask) {
        let reps = reps.difference(RepMask::LABELS);
        self.state.draw_reps.set_hidden(MOLECULAR_REPS);
        self.state
            .draw_reps
            .set_visible(reps.intersection(self.state.visible_reps));
        self.dirty |= DirtyFlags::DRAW_MASK;
    }

    /// Show a representation for selected atoms.
    pub fn show_rep_for_selection(&mut self, selected: &SelectionResult, rep: RepMask) {
        let rep = rep.difference(RepMask::LABELS);
        if rep.is_empty() {
            return;
        }
        let selected_all_atoms = self.selection_covers_all_atoms(selected);
        if rep.can_toggle_with_draw_mask()
            && selected_all_atoms
            && self.state.visible_reps.is_visible(rep)
            && !self.state.draw_reps.is_visible(rep)
            && self.state.can_restore_draw_mask(rep)
            && self.selected_atoms_all_have_rep(selected, rep)
        {
            self.state.show_draw_rep(rep);
            self.dirty |= DirtyFlags::DRAW_MASK;
            return;
        }

        let materialize_hidden_draw_mask = rep.can_toggle_with_draw_mask()
            && !selected_all_atoms
            && self.state.visible_reps.is_visible(rep)
            && !self.state.draw_reps.is_visible(rep)
            && self.state.can_restore_draw_mask(rep);

        if materialize_hidden_draw_mask {
            for atom in self.molecule.atoms_mut() {
                atom.repr.visible_reps.set_hidden(rep);
            }
        }
        for idx in selected.indices() {
            if let Some(atom) = self.molecule.get_atom_mut(AtomIndex(idx.0)) {
                atom.repr.visible_reps.set_visible(rep);
            }
        }

        self.state.visible_reps.set_visible(rep);
        self.state.draw_reps.set_visible(rep);
        self.state.clear_draw_mask_restorable(rep);
        self.dirty |= representation_membership_dirty(rep);
    }

    /// Hide a representation for selected atoms.
    pub fn hide_rep_for_selection(&mut self, selected: &SelectionResult, rep: RepMask) {
        let rep = rep.difference(RepMask::LABELS);
        if rep.is_empty() {
            return;
        }
        let selected_all_atoms = self.selection_covers_all_atoms(selected);
        if rep.can_toggle_with_draw_mask() && selected_all_atoms {
            let can_restore_draw_mask = self.selected_atoms_all_have_rep(selected, rep);
            self.state.draw_reps.set_hidden(rep);
            if can_restore_draw_mask {
                self.state.mark_draw_mask_restorable(rep);
            } else {
                self.state.clear_draw_mask_restorable(rep);
            }
            self.dirty |= DirtyFlags::DRAW_MASK;
            return;
        }

        for idx in selected.indices() {
            if let Some(atom) = self.molecule.get_atom_mut(AtomIndex(idx.0)) {
                atom.repr.visible_reps.set_hidden(rep);
            }
        }
        self.dirty |= representation_membership_dirty(rep);

        if selected_all_atoms {
            self.state.visible_reps.set_hidden(rep);
            self.state.draw_reps.set_hidden(rep);
            self.state.clear_draw_mask_restorable(rep);
            return;
        }

        let visible_reps = self.atom_rep_union();
        self.state.visible_reps.set_hidden(MOLECULAR_REPS);
        self.state.visible_reps.set_visible(visible_reps);
        self.state.draw_reps = self
            .state
            .draw_reps
            .intersection(visible_reps.union(RepMask::LABELS));
        self.state.clear_draw_mask_restorable(rep);
    }

    /// Show only one representation for selected atoms.
    pub fn show_as_rep_for_selection(&mut self, selected: &SelectionResult, rep: RepMask) {
        let rep = rep.difference(RepMask::LABELS);
        if rep.is_empty() {
            return;
        }
        let selected_all_atoms = self.selection_covers_all_atoms(selected);
        for idx in selected.indices() {
            if let Some(atom) = self.molecule.get_atom_mut(AtomIndex(idx.0)) {
                atom.repr.visible_reps.set_hidden(MOLECULAR_REPS);
                atom.repr.visible_reps.set_visible(rep);
            }
        }

        if selected_all_atoms {
            self.state.visible_reps.set_hidden(MOLECULAR_REPS);
            self.state.visible_reps.set_visible(rep);
            self.state.draw_reps.set_hidden(MOLECULAR_REPS);
            self.state.draw_reps.set_visible(rep);
        } else {
            self.state.visible_reps.set_visible(rep);
            self.state.draw_reps.set_visible(rep);
        }
        self.state.clear_draw_mask_restorable(RepMask::ALL);
        self.dirty |= DirtyFlags::REPS;
    }

    /// Show a representation
    ///
    /// This sets visibility both at the object level and for all atoms.
    pub fn show(&mut self, rep: RepMask) {
        let selected = SelectionResult::all(self.molecule.atom_count());
        self.show_rep_for_selection(&selected, rep);
    }

    /// Hide a representation
    ///
    /// This hides the representation both at the object level and for all atoms.
    pub fn hide(&mut self, rep: RepMask) {
        let selected = SelectionResult::all(self.molecule.atom_count());
        self.hide_rep_for_selection(&selected, rep);
    }

    /// Toggle a representation
    ///
    /// This toggles visibility both at the object level and for all atoms.
    pub fn toggle(&mut self, rep: RepMask) {
        if self.state.draw_reps.is_visible(rep) {
            self.hide(rep);
        } else {
            self.show(rep);
        }
    }

    /// Hide all representations
    pub fn hide_all(&mut self) {
        self.state.visible_reps = RepMask::NONE;
        self.state.draw_reps = RepMask::NONE;
        self.state.draw_mask_restorable_reps = RepMask::NONE;
        self.dirty |= DirtyFlags::REPS;
    }

    /// Set surface quality (-4 to 4, default 0)
    ///
    /// Lower values are faster but coarser, higher values are smoother but slower.
    /// - -4: Very coarse (fastest)
    /// - 0: Default quality
    /// - 4: Very fine (slowest)
    pub fn set_surface_quality(&mut self, quality: i32) {
        let quality = quality.clamp(-4, 4);
        if self.surface_quality != quality {
            self.surface_quality = quality;
            self.dirty.insert(DirtyFlags::REPS);
        }
    }

    /// Get current surface quality
    pub fn surface_quality(&self) -> i32 {
        self.surface_quality
    }

    /// Get per-object overrides (create if needed)
    pub fn get_or_create_overrides(&mut self) -> &mut ObjectOverrides {
        if self.overrides.is_none() {
            self.overrides = Some(ObjectOverrides::default());
        }
        self.overrides.as_mut().unwrap()
    }

    /// Check if cartoon representation is visible
    pub fn is_cartoon_visible(&self) -> bool {
        self.state.draw_reps.is_visible(RepMask::CARTOON)
    }

    /// Check if surface representation is visible
    pub fn is_surface_visible(&self) -> bool {
        self.state.draw_reps.is_visible(RepMask::SURFACE)
    }

    /// Check if mesh representation is visible
    pub fn is_mesh_visible(&self) -> bool {
        self.state.draw_reps.is_visible(RepMask::MESH)
    }

    fn selection_covers_all_atoms(&self, selected: &SelectionResult) -> bool {
        selected.count() == self.molecule.atom_count()
    }

    fn selected_atoms_all_have_rep(&self, selected: &SelectionResult, rep: RepMask) -> bool {
        selected.indices().all(|idx| {
            self.molecule
                .get_atom(AtomIndex(idx.0))
                .is_some_and(|atom| atom.repr.visible_reps.is_visible(rep))
        })
    }

    fn atom_rep_union(&self) -> RepMask {
        let mut union = RepMask::NONE;
        for atom in self.molecule.atoms() {
            union = union.union(atom.repr.visible_reps);
        }
        union.difference(RepMask::LABELS)
    }
}

fn representation_membership_dirty(rep: RepMask) -> DirtyFlags {
    if matches!(
        rep,
        RepMask::SURFACE | RepMask::MESH | RepMask::CARTOON | RepMask::RIBBON
    ) {
        DirtyFlags::VISIBILITY
    } else {
        DirtyFlags::REPS
    }
}

impl Object for MoleculeObject {
    fn name(&self) -> &str {
        &self.molecule.name
    }

    fn object_type(&self) -> ObjectType {
        ObjectType::Molecule
    }

    fn state(&self) -> &ObjectState {
        &self.state
    }

    fn state_mut(&mut self) -> &mut ObjectState {
        &mut self.state
    }

    fn extent(&self) -> Option<(Vec3, Vec3)> {
        self.molecule.bounding_box(self.display_state)
    }

    fn n_states(&self) -> usize {
        self.molecule.state_count().max(1)
    }

    fn current_state(&self) -> usize {
        self.display_state
    }

    fn set_current_state(&mut self, state: usize) -> bool {
        self.set_display_state(state)
    }

    fn overrides(&self) -> Option<&ObjectOverrides> {
        self.overrides.as_ref()
    }

    fn overrides_mut(&mut self) -> Option<&mut ObjectOverrides> {
        self.overrides.as_mut()
    }

    fn get_or_create_overrides(&mut self) -> &mut ObjectOverrides {
        self.get_or_create_overrides()
    }

    fn set_name(&mut self, name: String) {
        self.molecule.name = name;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use patinae_mol::{Atom, AtomResidue, BondOrder, Element};
    use std::sync::Arc;

    fn create_test_molecule() -> ObjectMolecule {
        let mut mol = ObjectMolecule::new("test");
        let c1 = mol.add_atom(Atom::new("C1", Element::Carbon));
        let c2 = mol.add_atom(Atom::new("C2", Element::Carbon));
        mol.add_bond(c1, c2, BondOrder::Single).unwrap();

        let cs =
            patinae_mol::CoordSet::from_vec3(&[Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.5, 0.0, 0.0)]);
        mol.add_coord_set(cs);

        mol
    }

    fn all_atoms(obj: &MoleculeObject) -> SelectionResult {
        SelectionResult::all(obj.molecule().atom_count())
    }

    fn one_atom(obj: &MoleculeObject, idx: u32) -> SelectionResult {
        SelectionResult::from_indices(obj.molecule().atom_count(), [AtomIndex(idx)].into_iter())
    }

    fn prepare_partial_cartoon_then_full_hide(obj: &mut MoleculeObject) {
        let all_atoms = all_atoms(obj);
        obj.hide_rep_for_selection(&all_atoms, RepMask::CARTOON);
        let first_atom = one_atom(obj, 0);
        obj.show_rep_for_selection(&first_atom, RepMask::CARTOON);
        obj.hide_rep_for_selection(&all_atoms, RepMask::CARTOON);
        obj.clear_dirty();
    }

    fn collect_surface_visibility_mask(
        mol: &ObjectMolecule,
        coord_set: &patinae_mol::CoordSet,
    ) -> Vec<u32> {
        let mut mask = vec![0; coord_set.len().div_ceil(32)];
        for (atom_idx, _) in coord_set.iter_with_atoms() {
            let i = atom_idx.as_usize();
            if mol
                .get_atom(atom_idx)
                .is_some_and(|atom| atom.repr.visible_reps.is_visible(RepMask::SURFACE))
            {
                mask[i / 32] |= 1 << (i % 32);
            }
        }
        mask
    }

    #[test]
    fn test_molecule_object_creation() {
        let mol = create_test_molecule();
        let obj = MoleculeObject::new(mol);

        assert_eq!(obj.name(), "test");
        assert_eq!(obj.object_type(), ObjectType::Molecule);
        assert!(obj.state().enabled);
    }

    #[test]
    fn default_object_rep_mask_matches_atom_rep_union() {
        let mol = create_test_molecule();
        let obj = MoleculeObject::new(mol);

        assert_eq!(obj.visible_reps(), RepMask::CARTOON);
        assert_eq!(obj.draw_reps(), RepMask::CARTOON);
        assert!(obj
            .molecule()
            .atoms()
            .all(|atom| atom.repr.visible_reps == RepMask::CARTOON));
    }

    #[test]
    fn effective_atom_reps_apply_object_draw_mask() {
        let mol = create_test_molecule();
        let mut obj = MoleculeObject::new(mol);
        let first_atom = obj.molecule().get_atom(AtomIndex(0)).unwrap();

        assert_eq!(obj.effective_atom_reps(first_atom), RepMask::CARTOON);

        obj.set_draw_reps(RepMask::NONE);
        let first_atom = obj.molecule().get_atom(AtomIndex(0)).unwrap();

        assert_eq!(obj.effective_reps(), RepMask::NONE);
        assert_eq!(obj.effective_atom_reps(first_atom), RepMask::NONE);
    }

    #[test]
    fn test_dirty_flags() {
        let mol = create_test_molecule();
        let mut obj = MoleculeObject::new(mol);

        assert!(obj.is_dirty());

        // Accessing molecule_mut should set dirty
        let _ = obj.molecule_mut();
        assert!(obj.dirty_flags().contains(DirtyFlags::ALL));
    }

    #[test]
    fn test_show_hide() {
        let mol = create_test_molecule();
        let mut obj = MoleculeObject::new(mol);

        obj.hide(RepMask::LINES);
        assert!(!obj.state().visible_reps.is_visible(RepMask::LINES));

        obj.show(RepMask::SPHERES);
        assert!(obj.state().visible_reps.is_visible(RepMask::SPHERES));
    }

    #[test]
    fn whole_object_cartoon_hide_uses_draw_mask_without_mutating_atoms() {
        let mol = create_test_molecule();
        let mut obj = MoleculeObject::new(mol);
        obj.clear_dirty();

        obj.hide(RepMask::CARTOON);

        assert!(obj.visible_reps().is_visible(RepMask::CARTOON));
        assert!(!obj.draw_reps().is_visible(RepMask::CARTOON));
        assert!(obj.draw_mask_restorable_reps().is_visible(RepMask::CARTOON));
        assert!(obj
            .molecule()
            .atoms()
            .all(|atom| atom.repr.visible_reps.is_visible(RepMask::CARTOON)));
        assert_eq!(obj.dirty_flags(), DirtyFlags::DRAW_MASK);

        obj.clear_dirty();
        obj.show(RepMask::CARTOON);

        assert!(obj.visible_reps().is_visible(RepMask::CARTOON));
        assert!(obj.draw_reps().is_visible(RepMask::CARTOON));
        assert!(obj.draw_mask_restorable_reps().is_visible(RepMask::CARTOON));
        assert_eq!(obj.dirty_flags(), DirtyFlags::DRAW_MASK);
    }

    #[test]
    fn first_whole_object_cartoon_show_materializes_atom_bits_once() {
        let mol = create_test_molecule();
        let mut obj = MoleculeObject::new(mol);
        obj.state_mut().visible_reps.set_hidden(RepMask::CARTOON);
        obj.state_mut().draw_reps.set_hidden(RepMask::CARTOON);
        for atom in obj.molecule_mut_with_dirty(DirtyFlags::empty()).atoms_mut() {
            atom.repr.visible_reps.set_hidden(RepMask::CARTOON);
        }
        obj.clear_dirty();

        obj.show(RepMask::CARTOON);

        assert!(obj.visible_reps().is_visible(RepMask::CARTOON));
        assert!(obj.draw_reps().is_visible(RepMask::CARTOON));
        assert!(obj
            .molecule()
            .atoms()
            .all(|atom| atom.repr.visible_reps.is_visible(RepMask::CARTOON)));
        assert_eq!(obj.dirty_flags(), DirtyFlags::VISIBILITY);
    }

    #[test]
    fn partial_cartoon_hide_keeps_draw_mask_active() {
        let mol = create_test_molecule();
        let mut obj = MoleculeObject::new(mol);
        let selected = one_atom(&obj, 0);
        obj.clear_dirty();

        obj.hide_rep_for_selection(&selected, RepMask::CARTOON);

        assert_eq!(obj.dirty_flags(), DirtyFlags::VISIBILITY);
        assert!(!obj.draw_mask_restorable_reps().is_visible(RepMask::CARTOON));
        assert!(!obj
            .molecule()
            .get_atom(AtomIndex(0))
            .unwrap()
            .repr
            .visible_reps
            .is_visible(RepMask::CARTOON));
        assert!(obj
            .molecule()
            .get_atom(AtomIndex(1))
            .unwrap()
            .repr
            .visible_reps
            .is_visible(RepMask::CARTOON));
        assert!(obj.draw_reps().is_visible(RepMask::CARTOON));
    }

    #[test]
    fn partial_cartoon_show_after_draw_mask_hide_keeps_unselected_atoms_hidden() {
        let mol = create_test_molecule();
        let mut obj = MoleculeObject::new(mol);
        let all_atoms = all_atoms(&obj);
        obj.hide_rep_for_selection(&all_atoms, RepMask::CARTOON);
        obj.clear_dirty();
        let first_atom = one_atom(&obj, 0);

        obj.show_rep_for_selection(&first_atom, RepMask::CARTOON);

        assert_eq!(obj.dirty_flags(), DirtyFlags::VISIBILITY);
        assert!(obj.draw_reps().is_visible(RepMask::CARTOON));
        assert!(!obj.draw_mask_restorable_reps().is_visible(RepMask::CARTOON));
        assert!(obj
            .molecule()
            .get_atom(AtomIndex(0))
            .unwrap()
            .repr
            .visible_reps
            .is_visible(RepMask::CARTOON));
        assert!(!obj
            .molecule()
            .get_atom(AtomIndex(1))
            .unwrap()
            .repr
            .visible_reps
            .is_visible(RepMask::CARTOON));
    }

    #[test]
    fn partial_cartoon_or_ribbon_show_uses_visibility_dirty() {
        for rep in [RepMask::CARTOON, RepMask::RIBBON] {
            let mol = create_test_molecule();
            let mut obj = MoleculeObject::new(mol);
            {
                let mol = obj.molecule_mut();
                for atom in mol.atoms_mut() {
                    atom.repr.visible_reps.set_hidden(rep);
                }
                mol.get_atom_mut(AtomIndex(0))
                    .unwrap()
                    .repr
                    .visible_reps
                    .set_visible(rep);
            }
            {
                let state = obj.state_mut();
                state.visible_reps.set_visible(rep);
                state.draw_reps.set_visible(rep);
                state.clear_draw_mask_restorable(rep);
            }
            obj.clear_dirty();
            let second_atom = one_atom(&obj, 1);

            obj.show_rep_for_selection(&second_atom, rep);

            assert_eq!(obj.dirty_flags(), DirtyFlags::VISIBILITY, "{rep:?}");
            assert!(obj
                .molecule()
                .get_atom(AtomIndex(1))
                .unwrap()
                .repr
                .visible_reps
                .is_visible(rep));
        }
    }

    #[test]
    fn whole_object_cartoon_show_restores_atom_bits_when_draw_is_already_active() {
        let mol = create_test_molecule();
        let mut obj = MoleculeObject::new(mol);
        let first_atom = one_atom(&obj, 0);
        obj.hide_rep_for_selection(&first_atom, RepMask::CARTOON);
        obj.clear_dirty();
        let all_atoms = all_atoms(&obj);

        obj.show_rep_for_selection(&all_atoms, RepMask::CARTOON);

        assert_eq!(obj.dirty_flags(), DirtyFlags::VISIBILITY);
        assert!(obj
            .molecule()
            .atoms()
            .all(|atom| atom.repr.visible_reps.is_visible(RepMask::CARTOON)));
    }

    #[test]
    fn whole_object_cartoon_show_materializes_when_restore_flag_is_invalid() {
        let mol = create_test_molecule();
        let mut obj = MoleculeObject::new(mol);
        {
            let state = obj.state_mut();
            state.visible_reps.set_visible(RepMask::CARTOON);
            state.draw_reps.set_hidden(RepMask::CARTOON);
            state.mark_draw_mask_restorable(RepMask::CARTOON);
        }
        obj.molecule_mut()
            .get_atom_mut(AtomIndex(1))
            .unwrap()
            .repr
            .visible_reps
            .set_hidden(RepMask::CARTOON);
        obj.clear_dirty();
        let all_atoms = all_atoms(&obj);

        obj.show_rep_for_selection(&all_atoms, RepMask::CARTOON);

        assert_eq!(obj.dirty_flags(), DirtyFlags::VISIBILITY);
        assert!(obj.draw_reps().is_visible(RepMask::CARTOON));
        assert!(!obj.draw_mask_restorable_reps().is_visible(RepMask::CARTOON));
        assert!(obj
            .molecule()
            .atoms()
            .all(|atom| atom.repr.visible_reps.is_visible(RepMask::CARTOON)));
    }

    #[test]
    fn full_object_hide_after_partial_cartoon_membership_cannot_restore_draw_mask() {
        let mol = create_test_molecule();
        let mut obj = MoleculeObject::new(mol);

        prepare_partial_cartoon_then_full_hide(&mut obj);

        assert_eq!(obj.dirty_flags(), DirtyFlags::empty());
        assert!(!obj.draw_reps().is_visible(RepMask::CARTOON));
        assert!(!obj.draw_mask_restorable_reps().is_visible(RepMask::CARTOON));
        assert!(obj
            .molecule()
            .get_atom(AtomIndex(0))
            .unwrap()
            .repr
            .visible_reps
            .is_visible(RepMask::CARTOON));
        assert!(!obj
            .molecule()
            .get_atom(AtomIndex(1))
            .unwrap()
            .repr
            .visible_reps
            .is_visible(RepMask::CARTOON));
    }

    #[test]
    fn full_object_cartoon_show_after_partial_hide_enables_all_atoms() {
        let mol = create_test_molecule();
        let mut obj = MoleculeObject::new(mol);
        prepare_partial_cartoon_then_full_hide(&mut obj);
        let all_atoms = all_atoms(&obj);

        obj.show_rep_for_selection(&all_atoms, RepMask::CARTOON);

        assert_eq!(obj.dirty_flags(), DirtyFlags::VISIBILITY);
        assert!(obj.draw_reps().is_visible(RepMask::CARTOON));
        assert!(!obj.draw_mask_restorable_reps().is_visible(RepMask::CARTOON));
        assert!(obj
            .molecule()
            .atoms()
            .all(|atom| atom.repr.visible_reps.is_visible(RepMask::CARTOON)));
    }

    #[test]
    fn partial_cartoon_mutation_clears_draw_mask_restore_flag() {
        let mol = create_test_molecule();
        let mut obj = MoleculeObject::new(mol);
        let all_atoms = all_atoms(&obj);
        obj.hide_rep_for_selection(&all_atoms, RepMask::CARTOON);
        assert!(obj.draw_mask_restorable_reps().is_visible(RepMask::CARTOON));
        obj.clear_dirty();
        let first_atom = one_atom(&obj, 0);

        obj.hide_rep_for_selection(&first_atom, RepMask::CARTOON);

        assert_eq!(obj.dirty_flags(), DirtyFlags::VISIBILITY);
        assert!(!obj.draw_mask_restorable_reps().is_visible(RepMask::CARTOON));
    }

    #[test]
    fn show_as_rep_for_selection_keeps_only_requested_rep_on_selected_atoms() {
        let mol = create_test_molecule();
        let mut obj = MoleculeObject::new(mol);
        let selected = one_atom(&obj, 0);
        obj.clear_dirty();

        obj.show_as_rep_for_selection(&selected, RepMask::STICKS);

        assert_eq!(obj.dirty_flags(), DirtyFlags::REPS);
        assert_eq!(
            obj.molecule()
                .get_atom(AtomIndex(0))
                .unwrap()
                .repr
                .visible_reps,
            RepMask::STICKS
        );
        assert!(obj
            .molecule()
            .get_atom(AtomIndex(1))
            .unwrap()
            .repr
            .visible_reps
            .is_visible(RepMask::CARTOON));
        assert!(obj.visible_reps().is_visible(RepMask::STICKS));
        assert!(obj.draw_reps().is_visible(RepMask::STICKS));
        assert_eq!(obj.draw_mask_restorable_reps(), RepMask::NONE);
    }

    #[test]
    fn molecular_rep_mutations_never_change_legacy_label_bits() {
        let mol = create_test_molecule();
        let mut obj = MoleculeObject::new(mol);
        for atom in obj.molecule_mut().atoms_mut() {
            atom.repr.visible_reps.set_visible(RepMask::LABELS);
        }
        obj.state.visible_reps.set_visible(RepMask::LABELS);
        obj.state.draw_reps.set_visible(RepMask::LABELS);
        let selected = all_atoms(&obj);

        obj.hide_rep_for_selection(&selected, RepMask::ALL);
        obj.show_rep_for_selection(&selected, RepMask::ALL);
        obj.show_as_rep_for_selection(&selected, RepMask::STICKS);

        assert!(obj.state.visible_reps.is_visible(RepMask::LABELS));
        assert!(obj.state.draw_reps.is_visible(RepMask::LABELS));
        assert!(obj
            .molecule()
            .atoms()
            .all(|atom| atom.repr.visible_reps.is_visible(RepMask::LABELS)));

        obj.clear_dirty();
        obj.hide_rep_for_selection(&selected, RepMask::LABELS);
        obj.show_rep_for_selection(&selected, RepMask::LABELS);
        assert_eq!(obj.dirty_flags(), DirtyFlags::empty());
    }

    #[test]
    fn test_metal_inside_organic_ligand_defaults_to_solid_sphere() {
        let mut mol = ObjectMolecule::new("heme");
        let residue = Arc::new(AtomResidue::from_parts("A", "HEM", 142, ' ', ""));

        let mut carbon = Atom::new("C1", Element::Carbon);
        carbon.residue = residue.clone();
        carbon.state.hetatm = true;
        mol.add_atom(carbon);

        let mut iron = Atom::new("FE", Element::Iron);
        iron.residue = residue;
        iron.state.hetatm = true;
        mol.add_atom(iron);

        mol.add_coord_set(patinae_mol::CoordSet::from_vec3(&[
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.5, 0.0, 0.0),
        ]));
        mol.classify_atoms();

        let obj = MoleculeObject::new(mol);
        let atoms = obj.molecule().atoms_slice();

        assert!(atoms[0].repr.visible_reps.is_visible(RepMask::STICKS));
        assert!(atoms[1].repr.visible_reps.is_visible(RepMask::SPHERES));
        assert_eq!(obj.visible_reps(), RepMask::STICKS.union(RepMask::SPHERES));
        assert_eq!(obj.draw_reps(), obj.visible_reps());
        assert_eq!(atoms[1].repr.sphere_transparency, None);
    }

    #[test]
    fn test_collect_surface_visibility_mask_packs_bits_lsb_first() {
        let mut mol = ObjectMolecule::new("vis");
        for i in 0..40 {
            mol.add_atom(Atom::new(format!("C{}", i), Element::Carbon));
        }
        let coords: Vec<Vec3> = (0..40).map(|i| Vec3::new(i as f32, 0.0, 0.0)).collect();
        mol.add_coord_set(patinae_mol::CoordSet::from_vec3(&coords));

        // Show surface only for atoms 0, 5, 32, 39.
        for &i in &[0u32, 5, 32, 39] {
            mol.get_atom_mut(patinae_mol::AtomIndex(i))
                .unwrap()
                .repr
                .visible_reps
                .set_visible(RepMask::SURFACE);
        }

        let coord_set = mol.get_coord_set(0).unwrap();
        let mask = collect_surface_visibility_mask(&mol, coord_set);
        // 40 atoms → ceil(40/32) = 2 words.
        assert_eq!(mask.len(), 2);
        assert_eq!(mask[0], (1 << 0) | (1 << 5), "word 0: atoms 0 and 5");
        assert_eq!(mask[1], (1 << 0) | (1 << 7), "word 1: atoms 32 and 39");
    }

    #[test]
    fn test_dirty_flags_include_transparency_and_visibility() {
        // Sanity check: ALL covers the new bits so existing call sites that
        // mark ALL keep working.
        assert!(DirtyFlags::ALL.contains(DirtyFlags::TRANSPARENCY));
        assert!(DirtyFlags::ALL.contains(DirtyFlags::VISIBILITY));
        assert!(DirtyFlags::ALL.contains(DirtyFlags::DRAW_MASK));
        assert!(DirtyFlags::TRANSPARENCY.bits() != DirtyFlags::COLOR.bits());
        assert!(DirtyFlags::VISIBILITY.bits() != DirtyFlags::REPS.bits());
        assert!(DirtyFlags::DRAW_MASK.bits() != DirtyFlags::VISIBILITY.bits());
    }

    #[test]
    fn test_display_state() {
        let mut mol = ObjectMolecule::new("test");
        mol.add_atom(Atom::new("C", Element::Carbon));

        // Add multiple coordinate sets
        mol.add_coord_set(patinae_mol::CoordSet::from_vec3(&[Vec3::new(
            0.0, 0.0, 0.0,
        )]));
        mol.add_coord_set(patinae_mol::CoordSet::from_vec3(&[Vec3::new(
            1.0, 1.0, 1.0,
        )]));

        let mut obj = MoleculeObject::new(mol);

        assert_eq!(obj.n_states(), 2);
        assert_eq!(obj.display_state(), 0);

        assert!(obj.set_display_state(1));
        assert_eq!(obj.display_state(), 1);

        // Invalid state
        assert!(!obj.set_display_state(5));
    }
}
