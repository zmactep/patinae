//! Python bindings for ObjectMolecule

use patinae_mol::{AtomIndex, ObjectMolecule};
use pyo3::prelude::*;

use super::atom::{PyAtom, PyAtomIter};
use super::bond::{PyBond, PyBondIter};
use super::coordset::PyCoordSet;
use crate::convert::vec3_to_tuple;

type BoundingBox = ((f32, f32, f32), (f32, f32, f32));

/// Python wrapper for ObjectMolecule
///
/// Represents a molecular object containing atoms, bonds, and coordinates.
#[pyclass(name = "ObjectMolecule")]
pub struct PyObjectMolecule {
    pub(crate) inner: ObjectMolecule,
}

impl From<ObjectMolecule> for PyObjectMolecule {
    fn from(mol: ObjectMolecule) -> Self {
        PyObjectMolecule { inner: mol }
    }
}

impl PyObjectMolecule {
    /// Get a reference to the inner ObjectMolecule
    pub fn inner(&self) -> &ObjectMolecule {
        &self.inner
    }

    /// Get a mutable reference to the inner ObjectMolecule
    pub fn inner_mut(&mut self) -> &mut ObjectMolecule {
        &mut self.inner
    }

    /// Take ownership of the inner ObjectMolecule
    pub fn into_inner(self) -> ObjectMolecule {
        self.inner
    }
}

#[pymethods]
impl PyObjectMolecule {
    /// Create a new empty molecule with the given name
    #[new]
    #[pyo3(signature = (name="molecule"))]
    fn new(name: &str) -> Self {
        PyObjectMolecule {
            inner: ObjectMolecule::new(name),
        }
    }

    /// Object name
    #[getter]
    fn name(&self) -> &str {
        &self.inner.name
    }

    /// Set object name
    #[setter]
    fn set_name(&mut self, name: String) {
        self.inner.name = name;
    }

    /// Title/description
    #[getter]
    fn title(&self) -> &str {
        &self.inner.title
    }

    /// Set title
    #[setter]
    fn set_title(&mut self, title: String) {
        self.inner.title = title;
    }

    /// Number of atoms
    #[getter]
    fn atom_count(&self) -> usize {
        self.inner.atom_count()
    }

    /// Alias for `atom_count`.
    fn n_atoms(&self) -> usize {
        self.atom_count()
    }

    /// Number of bonds
    #[getter]
    fn bond_count(&self) -> usize {
        self.inner.bond_count()
    }

    /// Alias for `bond_count`.
    fn n_bonds(&self) -> usize {
        self.bond_count()
    }

    /// Number of states/frames
    #[getter]
    fn state_count(&self) -> usize {
        self.inner.state_count()
    }

    /// Alias for `state_count`.
    fn n_states(&self) -> usize {
        self.state_count()
    }

    /// Check if molecule has coordinates
    fn has_coords(&self) -> bool {
        self.inner.has_coords()
    }

    /// Current state index (0-based)
    #[getter]
    fn current_state(&self) -> usize {
        self.inner.current_state
    }

    /// Set current state
    #[setter]
    fn set_current_state(&mut self, state: usize) {
        self.inner.current_state = state;
    }

    /// Whether this is a discrete object
    #[getter]
    fn discrete(&self) -> bool {
        self.inner.discrete
    }

    /// Get an atom by index
    ///
    /// Returns atom data with coordinates from the current state.
    #[pyo3(signature = (index, state=None))]
    fn get_atom(&self, index: usize, state: Option<usize>) -> PyResult<PyAtom> {
        let atom = self
            .inner
            .get_atom(AtomIndex(index as u32))
            .ok_or_else(|| {
                pyo3::exceptions::PyIndexError::new_err(format!(
                    "Atom index {} out of range (max: {})",
                    index,
                    self.inner.atom_count()
                ))
            })?;

        // Get coordinates for the atom
        let state_idx = state.unwrap_or(self.inner.current_state);
        let coord = if let Some(cs) = self.inner.get_coord_set(state_idx) {
            cs.get_atom_coord(AtomIndex(index as u32))
                .map(|v| (v.x, v.y, v.z))
        } else {
            None
        };

        Ok(PyAtom::from_atom(atom, coord, index))
    }

    /// Iterate over all atoms
    #[pyo3(signature = (state=None))]
    fn atoms(&self, state: Option<usize>) -> PyAtomIter {
        let state_idx = state.unwrap_or(self.inner.current_state);
        let cs = self.inner.get_coord_set(state_idx);

        let atoms: Vec<PyAtom> = (0..self.inner.atom_count())
            .map(|i| {
                let atom = self.inner.get_atom(AtomIndex(i as u32)).unwrap();
                let coord = cs
                    .and_then(|c| c.get_atom_coord(AtomIndex(i as u32)))
                    .map(|v| (v.x, v.y, v.z));
                PyAtom::from_atom(atom, coord, i)
            })
            .collect();

        PyAtomIter::new(atoms)
    }

    /// Get a bond by index
    fn get_bond(&self, index: usize) -> PyResult<PyBond> {
        use patinae_mol::BondIndex;
        let bond = self
            .inner
            .get_bond(BondIndex(index as u32))
            .ok_or_else(|| {
                pyo3::exceptions::PyIndexError::new_err(format!(
                    "Bond index {} out of range (max: {})",
                    index,
                    self.inner.bond_count()
                ))
            })?;

        Ok(PyBond::from_bond(bond, index))
    }

    /// Iterate over all bonds
    fn bonds(&self) -> PyBondIter {
        let bonds: Vec<PyBond> = self
            .inner
            .bonds()
            .enumerate()
            .map(|(i, bond)| PyBond::from_bond(bond, i))
            .collect();

        PyBondIter::new(bonds)
    }

    /// Get bonds for a specific atom
    fn get_bonds_for_atom(&self, atom_index: usize) -> Vec<PyBond> {
        self.inner
            .atom_bond_indices(AtomIndex(atom_index as u32))
            .iter()
            .filter_map(|&bond_idx| {
                self.inner
                    .get_bond(bond_idx)
                    .map(|b| PyBond::from_bond(b, bond_idx.0 as usize))
            })
            .collect()
    }

    /// Get coordinate set for a state
    #[pyo3(signature = (state=None))]
    fn get_coord_set(&self, state: Option<usize>) -> Option<PyCoordSet> {
        let state_idx = state.unwrap_or(self.inner.current_state);
        self.inner
            .get_coord_set(state_idx)
            .map(|cs| PyCoordSet::from_coordset(cs, state_idx))
    }

    /// Get all coordinates as a list of (x, y, z) tuples
    #[pyo3(signature = (state=None))]
    fn get_coords(&self, state: Option<usize>) -> Option<Vec<(f32, f32, f32)>> {
        let state_idx = state.unwrap_or(self.inner.current_state);
        self.inner
            .get_coord_set(state_idx)
            .map(|cs| cs.iter().map(|v| (v.x, v.y, v.z)).collect())
    }

    /// Get coordinate for a specific atom
    #[pyo3(signature = (atom_index, state=None))]
    fn get_coord(&self, atom_index: usize, state: Option<usize>) -> Option<(f32, f32, f32)> {
        let state_idx = state.unwrap_or(self.inner.current_state);
        self.inner
            .get_coord(AtomIndex(atom_index as u32), state_idx)
            .map(|v| (v.x, v.y, v.z))
    }

    /// Compute the geometric center
    #[pyo3(signature = (state=None))]
    fn center(&self, state: Option<usize>) -> Option<(f32, f32, f32)> {
        let state_idx = state.unwrap_or(self.inner.current_state);
        self.inner.center(state_idx).map(vec3_to_tuple)
    }

    /// Compute the bounding box
    #[pyo3(signature = (state=None))]
    fn bounding_box(&self, state: Option<usize>) -> Option<BoundingBox> {
        let state_idx = state.unwrap_or(self.inner.current_state);
        self.inner
            .bounding_box(state_idx)
            .map(|(min, max)| (vec3_to_tuple(min), vec3_to_tuple(max)))
    }

    /// Count atoms matching a simple name pattern
    fn count_atoms_by_name(&self, name: &str) -> usize {
        self.inner.atoms().filter(|a| &*a.name == name).count()
    }

    /// Count atoms matching a residue name
    fn count_atoms_by_resn(&self, resn: &str) -> usize {
        self.inner
            .atoms()
            .filter(|a| a.residue.resn == resn)
            .count()
    }

    /// Count atoms matching a chain
    fn count_atoms_by_chain(&self, chain: &str) -> usize {
        self.inner
            .atoms()
            .filter(|a| a.residue.chain == chain)
            .count()
    }

    /// Get unique chain identifiers
    fn get_chains(&self) -> Vec<String> {
        let mut chains: Vec<String> = self
            .inner
            .atoms()
            .map(|a| a.residue.chain.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        chains.sort();
        chains
    }

    /// Get unique residue names
    fn get_residue_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .inner
            .atoms()
            .map(|a| a.residue.resn.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        names.sort();
        names
    }

    fn __len__(&self) -> usize {
        self.inner.atom_count()
    }

    fn __repr__(&self) -> String {
        format!(
            "ObjectMolecule('{}', atoms={}, bonds={}, states={})",
            self.inner.name,
            self.inner.atom_count(),
            self.inner.bond_count(),
            self.inner.state_count()
        )
    }

    fn __str__(&self) -> String {
        self.inner.name.clone()
    }
}
