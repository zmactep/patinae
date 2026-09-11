//! Python bindings for CoordSet

use patinae_mol::CoordSet;
use pyo3::prelude::*;

use crate::convert::coordset_to_py_list;

type BoundingBox = ((f32, f32, f32), (f32, f32, f32));

/// Python wrapper for coordinate set
#[pyclass(name = "CoordSet")]
#[derive(Debug, Clone)]
pub struct PyCoordSet {
    /// Coordinates as flat list of (x, y, z) tuples
    coords: Vec<(f32, f32, f32)>,
    /// Name of this coordinate set
    name: String,
    /// Index of this state
    state_index: usize,
}

impl PyCoordSet {
    /// Create a PyCoordSet from a Rust CoordSet
    pub fn from_coordset(cs: &CoordSet, state_index: usize) -> Self {
        PyCoordSet {
            coords: coordset_to_py_list(cs),
            name: cs.name.clone(),
            state_index,
        }
    }

    /// Create from raw coordinate list
    pub fn from_coords(coords: Vec<(f32, f32, f32)>, name: String, state_index: usize) -> Self {
        PyCoordSet {
            coords,
            name,
            state_index,
        }
    }
}

#[pymethods]
impl PyCoordSet {
    /// Create a new coordinate set from a list of (x, y, z) tuples
    #[new]
    #[pyo3(signature = (coords, name="".to_string(), state_index=0))]
    fn new(coords: Vec<(f32, f32, f32)>, name: String, state_index: usize) -> Self {
        PyCoordSet {
            coords,
            name,
            state_index,
        }
    }

    /// Number of atoms with coordinates
    #[getter]
    fn len(&self) -> usize {
        self.coords.len()
    }

    /// Check if empty
    fn is_empty(&self) -> bool {
        self.coords.is_empty()
    }

    /// Name of this coordinate set
    #[getter]
    fn name(&self) -> &str {
        &self.name
    }

    /// State index (0-based)
    #[getter]
    fn state_index(&self) -> usize {
        self.state_index
    }

    /// Get coordinate at index as (x, y, z) tuple
    fn get_coord(&self, index: usize) -> PyResult<(f32, f32, f32)> {
        self.coords.get(index).copied().ok_or_else(|| {
            pyo3::exceptions::PyIndexError::new_err(format!(
                "Coordinate index {} out of range (max: {})",
                index,
                self.coords.len()
            ))
        })
    }

    /// Get all coordinates as list of (x, y, z) tuples
    fn get_coords(&self) -> Vec<(f32, f32, f32)> {
        self.coords.clone()
    }

    /// Compute the geometric center
    fn center(&self) -> (f32, f32, f32) {
        if self.coords.is_empty() {
            return (0.0, 0.0, 0.0);
        }

        let mut sum = (0.0f32, 0.0f32, 0.0f32);
        for (x, y, z) in &self.coords {
            sum.0 += x;
            sum.1 += y;
            sum.2 += z;
        }
        let n = self.coords.len() as f32;
        (sum.0 / n, sum.1 / n, sum.2 / n)
    }

    /// Compute bounding box as ((min_x, min_y, min_z), (max_x, max_y, max_z))
    fn bounding_box(&self) -> Option<BoundingBox> {
        if self.coords.is_empty() {
            return None;
        }

        let mut min = (f32::MAX, f32::MAX, f32::MAX);
        let mut max = (f32::MIN, f32::MIN, f32::MIN);

        for (x, y, z) in &self.coords {
            min.0 = min.0.min(*x);
            min.1 = min.1.min(*y);
            min.2 = min.2.min(*z);
            max.0 = max.0.max(*x);
            max.1 = max.1.max(*y);
            max.2 = max.2.max(*z);
        }

        Some((min, max))
    }

    fn __len__(&self) -> usize {
        self.coords.len()
    }

    fn __getitem__(&self, index: usize) -> PyResult<(f32, f32, f32)> {
        self.get_coord(index)
    }

    fn __repr__(&self) -> String {
        format!(
            "CoordSet(n_atoms={}, state={}, name='{}')",
            self.coords.len(),
            self.state_index,
            self.name
        )
    }

    fn __iter__(slf: PyRef<'_, Self>) -> PyCoordIter {
        PyCoordIter {
            coords: slf.coords.clone(),
            index: 0,
        }
    }
}

/// Iterator over coordinates
#[pyclass]
pub struct PyCoordIter {
    coords: Vec<(f32, f32, f32)>,
    index: usize,
}

#[pymethods]
impl PyCoordIter {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(mut slf: PyRefMut<'_, Self>) -> Option<(f32, f32, f32)> {
        if slf.index < slf.coords.len() {
            let coord = slf.coords[slf.index];
            slf.index += 1;
            Some(coord)
        } else {
            None
        }
    }
}
