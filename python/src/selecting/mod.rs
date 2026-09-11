//! Selection module for Python
//!
//! Provides selection language parsing and evaluation.

use patinae_mol::AtomIndex;
use patinae_select::SelectionResult;
use pyo3::prelude::*;

use crate::error::ResultExt;
use crate::mol::PyObjectMolecule;

/// Result of a selection operation
///
/// Contains the set of selected atom indices and provides methods
/// for manipulating and querying the selection.
#[pyclass(name = "SelectionResult")]
#[derive(Clone)]
pub struct PySelectionResult {
    inner: SelectionResult,
    atom_count: usize,
}

impl PySelectionResult {
    /// Create from a Rust SelectionResult
    pub fn from_result(result: SelectionResult, atom_count: usize) -> Self {
        PySelectionResult {
            inner: result,
            atom_count,
        }
    }

    /// Get the inner SelectionResult
    pub fn inner(&self) -> &SelectionResult {
        &self.inner
    }
}

#[pymethods]
impl PySelectionResult {
    /// Create a new empty selection
    #[new]
    #[pyo3(signature = (atom_count=0))]
    fn new(atom_count: usize) -> Self {
        PySelectionResult {
            inner: SelectionResult::new(atom_count),
            atom_count,
        }
    }

    /// Create a selection containing all atoms
    #[staticmethod]
    fn all(atom_count: usize) -> Self {
        PySelectionResult {
            inner: SelectionResult::all(atom_count),
            atom_count,
        }
    }

    /// Create an empty selection
    #[staticmethod]
    fn none(atom_count: usize) -> Self {
        PySelectionResult {
            inner: SelectionResult::none(atom_count),
            atom_count,
        }
    }

    /// Create a selection from a list of atom indices
    #[staticmethod]
    fn from_indices(indices: Vec<u32>, atom_count: usize) -> Self {
        let atom_indices: Vec<AtomIndex> = indices.into_iter().map(AtomIndex).collect();
        PySelectionResult {
            inner: SelectionResult::from_indices(atom_count, atom_indices.into_iter()),
            atom_count,
        }
    }

    /// Number of selected atoms
    #[getter]
    fn count(&self) -> usize {
        self.inner.count()
    }

    /// Check if the selection is empty
    fn is_empty(&self) -> bool {
        self.inner.count() == 0
    }

    /// Check if an atom is selected
    fn contains(&self, index: u32) -> bool {
        self.inner.contains(AtomIndex(index))
    }

    /// Get list of selected atom indices
    fn indices(&self) -> Vec<u32> {
        self.inner.indices().map(|idx| idx.0).collect()
    }

    /// Union with another selection (OR)
    fn union(&self, other: &PySelectionResult) -> PySelectionResult {
        PySelectionResult {
            inner: self.inner.union(&other.inner),
            atom_count: self.atom_count.max(other.atom_count),
        }
    }

    /// Intersection with another selection (AND)
    fn intersection(&self, other: &PySelectionResult) -> PySelectionResult {
        PySelectionResult {
            inner: self.inner.intersection(&other.inner),
            atom_count: self.atom_count.max(other.atom_count),
        }
    }

    /// Difference from another selection (self - other)
    fn difference(&self, other: &PySelectionResult) -> PySelectionResult {
        PySelectionResult {
            inner: self.inner.difference(&other.inner),
            atom_count: self.atom_count,
        }
    }

    /// Complement (invert) the selection
    fn complement(&self) -> PySelectionResult {
        PySelectionResult {
            inner: self.inner.complement(),
            atom_count: self.atom_count,
        }
    }

    /// Logical OR operator (|)
    fn __or__(&self, other: &PySelectionResult) -> PySelectionResult {
        self.union(other)
    }

    /// Logical AND operator (&)
    fn __and__(&self, other: &PySelectionResult) -> PySelectionResult {
        self.intersection(other)
    }

    /// Subtraction operator (-)
    fn __sub__(&self, other: &PySelectionResult) -> PySelectionResult {
        self.difference(other)
    }

    /// Invert operator (~)
    fn __invert__(&self) -> PySelectionResult {
        self.complement()
    }

    fn __len__(&self) -> usize {
        self.inner.count()
    }

    fn __bool__(&self) -> bool {
        self.inner.count() > 0
    }

    fn __repr__(&self) -> String {
        format!(
            "SelectionResult({} of {} atoms)",
            self.inner.count(),
            self.atom_count
        )
    }

    fn __iter__(slf: PyRef<'_, Self>) -> PySelectionIter {
        PySelectionIter {
            indices: slf.indices(),
            index: 0,
        }
    }
}

/// Iterator over selected atom indices
#[pyclass]
pub struct PySelectionIter {
    indices: Vec<u32>,
    index: usize,
}

#[pymethods]
impl PySelectionIter {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(mut slf: PyRefMut<'_, Self>) -> Option<u32> {
        if slf.index < slf.indices.len() {
            let idx = slf.indices[slf.index];
            slf.index += 1;
            Some(idx)
        } else {
            None
        }
    }
}

/// Parse a selection expression into an AST
///
/// Args:
///     selection: Selection expression string (e.g., "name CA and chain A")
///
/// Returns:
///     String representation of the parsed AST (for debugging)
#[pyfunction]
pub fn parse(selection: &str) -> PyResult<String> {
    let expr = patinae_select::parse(selection)
        .map_err(|e| crate::error::SelectionError::new_err(format!("Parse error: {}", e)))?;
    Ok(format!("{:?}", expr))
}

/// Select atoms from a molecule using a selection expression
///
/// Args:
///     mol: The molecule to select from
///     selection: Selection expression (e.g., "name CA", "chain A", "resn ALA")
///
/// Returns:
///     SelectionResult containing the selected atoms
#[pyfunction]
pub fn select(mol: &PyObjectMolecule, selection: &str) -> PyResult<PySelectionResult> {
    let result = patinae_select::select(mol.inner(), selection).map_py_err()?;
    Ok(PySelectionResult::from_result(
        result,
        mol.inner().atom_count(),
    ))
}

/// Select atoms and return their indices
///
/// Args:
///     mol: The molecule to select from
///     selection: Selection expression
///
/// Returns:
///     List of atom indices
#[pyfunction]
pub fn select_atoms(mol: &PyObjectMolecule, selection: &str) -> PyResult<Vec<u32>> {
    let indices = patinae_select::select_atoms(mol.inner(), selection).map_py_err()?;
    Ok(indices.into_iter().map(|idx| idx.0).collect())
}

/// Count atoms matching a selection
///
/// Args:
///     mol: The molecule to count atoms in
///     selection: Selection expression
///
/// Returns:
///     Number of matching atoms
#[pyfunction]
pub fn count_atoms(mol: &PyObjectMolecule, selection: &str) -> PyResult<usize> {
    let result = patinae_select::select(mol.inner(), selection).map_py_err()?;
    Ok(result.count())
}

/// Register the selecting submodule
pub fn register_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PySelectionResult>()?;
    m.add_function(wrap_pyfunction!(parse, m)?)?;
    m.add_function(wrap_pyfunction!(select, m)?)?;
    m.add_function(wrap_pyfunction!(select_atoms, m)?)?;
    m.add_function(wrap_pyfunction!(count_atoms, m)?)?;
    Ok(())
}
