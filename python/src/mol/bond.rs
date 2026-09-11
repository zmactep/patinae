//! Python bindings for Bond

use patinae_mol::{Bond, BondOrder, BondStereo};
use pyo3::prelude::*;

/// Python wrapper for bond data
#[pyclass(name = "Bond")]
#[derive(Debug, Clone)]
pub struct PyBond {
    /// Index of first atom
    pub(crate) atom1: u32,
    /// Index of second atom
    pub(crate) atom2: u32,
    /// Bond order
    pub(crate) order: BondOrder,
    /// Stereochemistry
    pub(crate) stereo: BondStereo,
    /// Index in parent molecule
    pub(crate) index: usize,
}

impl PyBond {
    /// Create a PyBond from a Rust Bond
    pub fn from_bond(bond: &Bond, index: usize) -> Self {
        PyBond {
            atom1: bond.atom1.0,
            atom2: bond.atom2.0,
            order: bond.order,
            stereo: bond.stereo,
            index,
        }
    }
}

#[pymethods]
impl PyBond {
    /// Index of first atom
    #[getter]
    fn atom1(&self) -> u32 {
        self.atom1
    }

    /// Index of second atom
    #[getter]
    fn atom2(&self) -> u32 {
        self.atom2
    }

    /// Bond order as integer (1=single, 2=double, 3=triple, 4=aromatic)
    #[getter]
    fn order(&self) -> i8 {
        self.order.as_raw()
    }

    /// Bond order as float (aromatic = 1.5)
    #[getter]
    fn order_float(&self) -> f32 {
        self.order.as_float()
    }

    /// Bond order as string ("single", "double", "triple", "aromatic", "unknown")
    #[getter]
    fn order_str(&self) -> &'static str {
        match self.order {
            BondOrder::Unknown => "unknown",
            BondOrder::Single => "single",
            BondOrder::Double => "double",
            BondOrder::Triple => "triple",
            BondOrder::Aromatic => "aromatic",
        }
    }

    /// Check if this is a single bond
    fn is_single(&self) -> bool {
        self.order == BondOrder::Single
    }

    /// Check if this is a double bond
    fn is_double(&self) -> bool {
        self.order == BondOrder::Double
    }

    /// Check if this is a triple bond
    fn is_triple(&self) -> bool {
        self.order == BondOrder::Triple
    }

    /// Check if this is an aromatic bond
    fn is_aromatic(&self) -> bool {
        self.order == BondOrder::Aromatic
    }

    /// Check if this is a multiple bond (double, triple, or aromatic)
    fn is_multiple(&self) -> bool {
        self.order.is_multiple()
    }

    /// Stereochemistry as string ("none", "up", "down", "either", "E", "Z")
    #[getter]
    fn stereo(&self) -> &'static str {
        match self.stereo {
            BondStereo::None => "none",
            BondStereo::Up => "up",
            BondStereo::Down => "down",
            BondStereo::Either => "either",
            BondStereo::E => "E",
            BondStereo::Z => "Z",
        }
    }

    /// Index of this bond in the parent molecule
    #[getter]
    fn index(&self) -> usize {
        self.index
    }

    /// Check if this bond involves the given atom index
    fn involves(&self, atom_idx: u32) -> bool {
        self.atom1 == atom_idx || self.atom2 == atom_idx
    }

    /// Get the other atom in this bond (given one atom index)
    fn other(&self, atom_idx: u32) -> Option<u32> {
        if self.atom1 == atom_idx {
            Some(self.atom2)
        } else if self.atom2 == atom_idx {
            Some(self.atom1)
        } else {
            None
        }
    }

    fn __repr__(&self) -> String {
        let order_sym = match self.order {
            BondOrder::Unknown => "?",
            BondOrder::Single => "-",
            BondOrder::Double => "=",
            BondOrder::Triple => "#",
            BondOrder::Aromatic => ":",
        };
        format!("Bond({}{}{}", self.atom1, order_sym, self.atom2)
    }

    fn __str__(&self) -> String {
        format!("{}-{}", self.atom1, self.atom2)
    }
}

/// Iterator over bonds in a molecule
#[pyclass]
pub struct PyBondIter {
    bonds: Vec<PyBond>,
    index: usize,
}

impl PyBondIter {
    pub fn new(bonds: Vec<PyBond>) -> Self {
        PyBondIter { bonds, index: 0 }
    }
}

#[pymethods]
impl PyBondIter {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(mut slf: PyRefMut<'_, Self>) -> Option<PyBond> {
        if slf.index < slf.bonds.len() {
            let bond = slf.bonds[slf.index].clone();
            slf.index += 1;
            Some(bond)
        } else {
            None
        }
    }

    fn __len__(&self) -> usize {
        self.bonds.len()
    }
}
