//! Evaluation context for selections
//!
//! Provides the `EvalContext` which holds references to molecules,
//! named selections, and other state needed during evaluation.

use ahash::AHashMap;
use patinae_mol::ObjectMolecule;

use crate::result::SelectionResult;

/// Options controlling selection matching behavior.
///
/// Controls case-sensitivity for selection matching.
/// Construct from `GlobalSettings` at call sites that have access to patinae-settings.
#[derive(Debug, Clone, Copy)]
pub struct SelectionOptions {
    /// Ignore case for name/resn/elem/alt etc. (default: true)
    pub ignore_case: bool,
    /// Ignore case for chain/segi (default: false)
    pub ignore_case_chain: bool,
}

impl Default for SelectionOptions {
    fn default() -> Self {
        Self {
            ignore_case: true,
            ignore_case_chain: false,
        }
    }
}

/// Context for evaluating selection expressions
///
/// The context provides access to:
/// - Molecules to select from
/// - Named selections (stored results)
/// - Current state index for coordinate-based operations
/// - Optional spatial index for distance queries
pub struct EvalContext<'a> {
    /// Molecules in the context
    molecules: Vec<&'a ObjectMolecule>,

    /// Named selections that can be referenced
    named_selections: AHashMap<String, SelectionResult>,

    /// Semantic label text indexed by flattened atom index.
    atom_labels: AHashMap<usize, Vec<&'a str>>,

    /// Current state index (0-based, None = all states)
    state: Option<usize>,

    /// Total atom count across all molecules
    total_atoms: usize,

    /// Starting index for each molecule in the flattened atom array
    molecule_offsets: Vec<usize>,

    /// Selection matching options (case sensitivity, etc.)
    pub options: SelectionOptions,
}

impl<'a> EvalContext<'a> {
    /// Create a context with a single molecule
    pub fn single(mol: &'a ObjectMolecule) -> Self {
        let total_atoms = mol.atom_count();
        EvalContext {
            molecules: vec![mol],
            named_selections: AHashMap::new(),
            atom_labels: AHashMap::new(),
            state: None,
            total_atoms,
            molecule_offsets: vec![0],
            options: SelectionOptions::default(),
        }
    }

    /// Create a context with multiple molecules
    pub fn multi(molecules: Vec<&'a ObjectMolecule>) -> Self {
        let mut total_atoms = 0;
        let mut offsets = Vec::with_capacity(molecules.len());
        for mol in &molecules {
            offsets.push(total_atoms);
            total_atoms += mol.atom_count();
        }
        EvalContext {
            molecules,
            named_selections: AHashMap::new(),
            atom_labels: AHashMap::new(),
            state: None,
            total_atoms,
            molecule_offsets: offsets,
            options: SelectionOptions::default(),
        }
    }

    /// Create an empty context (no molecules)
    pub fn empty() -> Self {
        EvalContext {
            molecules: Vec::new(),
            named_selections: AHashMap::new(),
            atom_labels: AHashMap::new(),
            state: None,
            total_atoms: 0,
            molecule_offsets: Vec::new(),
            options: SelectionOptions::default(),
        }
    }

    /// Get the total number of atoms across all molecules
    #[inline]
    pub fn total_atoms(&self) -> usize {
        self.total_atoms
    }

    /// Get the number of molecules
    #[inline]
    pub fn molecule_count(&self) -> usize {
        self.molecules.len()
    }

    /// Get a molecule by index
    #[inline]
    pub fn molecule(&self, index: usize) -> Option<&ObjectMolecule> {
        self.molecules.get(index).copied()
    }

    /// Get the first molecule
    #[inline]
    pub fn first_molecule(&self) -> Option<&ObjectMolecule> {
        self.molecules.first().copied()
    }

    /// Iterate over all molecules with their global offset
    pub fn molecules_with_offsets(&self) -> impl Iterator<Item = (&ObjectMolecule, usize)> {
        self.molecules
            .iter()
            .zip(self.molecule_offsets.iter())
            .map(|(mol, &offset)| (*mol, offset))
    }

    /// Convert a global atom index to (molecule_index, local_atom_index)
    pub fn global_to_local(&self, global_idx: usize) -> Option<(usize, usize)> {
        for (mol_idx, &offset) in self.molecule_offsets.iter().enumerate() {
            let _mol = self.molecules.get(mol_idx)?;
            let next_offset = if mol_idx + 1 < self.molecule_offsets.len() {
                self.molecule_offsets[mol_idx + 1]
            } else {
                self.total_atoms
            };

            if global_idx >= offset && global_idx < next_offset {
                return Some((mol_idx, global_idx - offset));
            }
        }
        None
    }

    /// Convert (molecule_index, local_atom_index) to global atom index
    pub fn local_to_global(&self, mol_idx: usize, local_idx: usize) -> Option<usize> {
        self.molecule_offsets
            .get(mol_idx)
            .map(|&offset| offset + local_idx)
    }

    /// Get the current state index
    #[inline]
    pub fn state(&self) -> Option<usize> {
        self.state
    }

    /// Set the current state index
    pub fn set_state(&mut self, state: Option<usize>) {
        self.state = state;
    }

    /// Create a context with a specific state
    pub fn with_state(mut self, state: usize) -> Self {
        self.state = Some(state);
        self
    }

    /// Add a named selection
    pub fn add_selection(&mut self, name: impl Into<String>, selection: SelectionResult) {
        self.named_selections.insert(name.into(), selection);
    }

    /// Get a named selection
    pub fn get_selection(&self, name: &str) -> Option<&SelectionResult> {
        self.named_selections.get(name)
    }

    /// Check if a named selection exists
    pub fn has_selection(&self, name: &str) -> bool {
        self.named_selections.contains_key(name)
    }

    /// Remove a named selection
    pub fn remove_selection(&mut self, name: &str) -> Option<SelectionResult> {
        self.named_selections.remove(name)
    }

    /// Clear all named selections
    pub fn clear_selections(&mut self) {
        self.named_selections.clear();
    }

    /// Get an iterator over named selections
    pub fn selections(&self) -> impl Iterator<Item = (&String, &SelectionResult)> {
        self.named_selections.iter()
    }

    /// Adds semantic label text for one flattened atom index.
    pub fn add_atom_label(&mut self, global_index: usize, text: &'a str) {
        if global_index < self.total_atoms {
            self.atom_labels.entry(global_index).or_default().push(text);
        }
    }

    /// Iterates semantic label text attached to one flattened atom index.
    pub fn atom_labels(&self, global_index: usize) -> impl Iterator<Item = &str> {
        self.atom_labels
            .get(&global_index)
            .into_iter()
            .flatten()
            .copied()
    }

    /// Get the effective state for a molecule (resolves None to current state)
    pub fn effective_state(&self, mol: &ObjectMolecule) -> usize {
        match self.state {
            Some(s) => s.min(mol.state_count().saturating_sub(1)),
            None => mol.current_state,
        }
    }

    /// Find a molecule by name
    pub fn find_molecule(&self, name: &str) -> Option<(usize, &ObjectMolecule)> {
        self.molecules
            .iter()
            .enumerate()
            .find(|(_, mol)| mol.name == name)
            .map(|(idx, mol)| (idx, *mol))
    }

    /// Get all molecule names
    pub fn molecule_names(&self) -> Vec<&str> {
        self.molecules.iter().map(|mol| mol.name.as_str()).collect()
    }
}

impl Default for EvalContext<'_> {
    fn default() -> Self {
        Self::empty()
    }
}

/// Builder for creating evaluation contexts
#[allow(dead_code)]
pub struct EvalContextBuilder<'a> {
    molecules: Vec<&'a ObjectMolecule>,
    named_selections: AHashMap<String, SelectionResult>,
    state: Option<usize>,
}

#[allow(dead_code)]
impl<'a> EvalContextBuilder<'a> {
    /// Create a new builder
    pub fn new() -> Self {
        EvalContextBuilder {
            molecules: Vec::new(),
            named_selections: AHashMap::new(),
            state: None,
        }
    }

    /// Add a molecule
    pub fn molecule(mut self, mol: &'a ObjectMolecule) -> Self {
        self.molecules.push(mol);
        self
    }

    /// Add multiple molecules
    pub fn molecules(mut self, mols: impl IntoIterator<Item = &'a ObjectMolecule>) -> Self {
        self.molecules.extend(mols);
        self
    }

    /// Add a named selection
    pub fn selection(mut self, name: impl Into<String>, selection: SelectionResult) -> Self {
        self.named_selections.insert(name.into(), selection);
        self
    }

    /// Set the state
    pub fn state(mut self, state: usize) -> Self {
        self.state = Some(state);
        self
    }

    /// Build the context
    pub fn build(self) -> EvalContext<'a> {
        let mut ctx = EvalContext::multi(self.molecules);
        ctx.named_selections = self.named_selections;
        ctx.state = self.state;
        ctx
    }
}

impl<'a> Default for EvalContextBuilder<'a> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_molecule(name: &str, atom_count: usize) -> ObjectMolecule {
        use patinae_mol::{Atom, Element};
        let mut mol = ObjectMolecule::new(name);
        for i in 0..atom_count {
            mol.add_atom(Atom::new(format!("A{}", i), Element::Carbon));
        }
        mol
    }

    #[test]
    fn test_single_molecule_context() {
        let mol = create_test_molecule("test", 10);
        let ctx = EvalContext::single(&mol);

        assert_eq!(ctx.total_atoms(), 10);
        assert_eq!(ctx.molecule_count(), 1);
        assert!(ctx.first_molecule().is_some());
    }

    #[test]
    fn test_multi_molecule_context() {
        let mol1 = create_test_molecule("mol1", 5);
        let mol2 = create_test_molecule("mol2", 10);
        let ctx = EvalContext::multi(vec![&mol1, &mol2]);

        assert_eq!(ctx.total_atoms(), 15);
        assert_eq!(ctx.molecule_count(), 2);
    }

    #[test]
    fn test_global_to_local() {
        let mol1 = create_test_molecule("mol1", 5);
        let mol2 = create_test_molecule("mol2", 10);
        let ctx = EvalContext::multi(vec![&mol1, &mol2]);

        assert_eq!(ctx.global_to_local(0), Some((0, 0)));
        assert_eq!(ctx.global_to_local(4), Some((0, 4)));
        assert_eq!(ctx.global_to_local(5), Some((1, 0)));
        assert_eq!(ctx.global_to_local(14), Some((1, 9)));
        assert_eq!(ctx.global_to_local(15), None);
    }

    #[test]
    fn test_local_to_global() {
        let mol1 = create_test_molecule("mol1", 5);
        let mol2 = create_test_molecule("mol2", 10);
        let ctx = EvalContext::multi(vec![&mol1, &mol2]);

        assert_eq!(ctx.local_to_global(0, 0), Some(0));
        assert_eq!(ctx.local_to_global(0, 4), Some(4));
        assert_eq!(ctx.local_to_global(1, 0), Some(5));
        assert_eq!(ctx.local_to_global(1, 9), Some(14));
    }

    #[test]
    fn test_named_selections() {
        let mol = create_test_molecule("test", 10);
        let mut ctx = EvalContext::single(&mol);

        let mut sel = SelectionResult::new(10);
        sel.set(patinae_mol::AtomIndex(5));
        ctx.add_selection("sele1", sel);

        assert!(ctx.has_selection("sele1"));
        assert!(!ctx.has_selection("sele2"));

        let retrieved = ctx.get_selection("sele1").unwrap();
        assert!(retrieved.contains(patinae_mol::AtomIndex(5)));
    }

    #[test]
    fn test_find_molecule() {
        let mol1 = create_test_molecule("protein", 100);
        let mol2 = create_test_molecule("ligand", 20);
        let ctx = EvalContext::multi(vec![&mol1, &mol2]);

        let (idx, mol) = ctx.find_molecule("ligand").unwrap();
        assert_eq!(idx, 1);
        assert_eq!(mol.name, "ligand");

        assert!(ctx.find_molecule("water").is_none());
    }

    #[test]
    fn test_builder() {
        let mol1 = create_test_molecule("mol1", 5);
        let mol2 = create_test_molecule("mol2", 10);

        let ctx = EvalContextBuilder::new()
            .molecule(&mol1)
            .molecule(&mol2)
            .state(0)
            .build();

        assert_eq!(ctx.total_atoms(), 15);
        assert_eq!(ctx.state(), Some(0));
    }
}
