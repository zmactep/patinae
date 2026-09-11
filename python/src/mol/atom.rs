//! Python bindings for Atom

use patinae_mol::{Atom, Element, SecondaryStructure};
use pyo3::prelude::*;

use super::element::PyElement;

/// Python wrapper for atom data
///
/// This is a snapshot of atom properties, not a reference to the original.
/// Modifications to this object do not affect the parent molecule.
#[pyclass(name = "Atom")]
#[derive(Debug, Clone)]
pub struct PyAtom {
    // Identity
    pub(crate) name: String,
    pub(crate) element: Element,

    // Residue info
    pub(crate) resn: String,
    pub(crate) resv: i32,
    pub(crate) inscode: char,
    pub(crate) chain: String,
    pub(crate) segi: String,
    pub(crate) alt: char,

    // Physical properties
    pub(crate) b_factor: f32,
    pub(crate) occupancy: f32,
    pub(crate) vdw: f32,
    pub(crate) partial_charge: f32,
    pub(crate) formal_charge: i8,

    // Display
    pub(crate) color: i32,
    pub(crate) hetatm: bool,

    // Secondary structure
    pub(crate) ss_type: SecondaryStructure,

    // Coordinates (if available)
    pub(crate) coord: Option<(f32, f32, f32)>,

    // Index in parent molecule
    pub(crate) index: usize,
}

impl PyAtom {
    /// Create a PyAtom from a Rust Atom and optional coordinates
    pub fn from_atom(atom: &Atom, coord: Option<(f32, f32, f32)>, index: usize) -> Self {
        PyAtom {
            name: atom.name.to_string(),
            element: atom.element,
            resn: atom.residue.resn.clone(),
            resv: atom.residue.resv,
            inscode: atom.residue.inscode,
            chain: atom.residue.chain.clone(),
            segi: atom.residue.segi.clone(),
            alt: atom.alt,
            b_factor: atom.b_factor,
            occupancy: atom.occupancy,
            vdw: atom.vdw,
            partial_charge: atom.partial_charge,
            formal_charge: atom.formal_charge,
            color: atom.repr.colors.base,
            hetatm: atom.state.hetatm,
            ss_type: atom.ss_type,
            coord,
            index,
        }
    }
}

#[pymethods]
impl PyAtom {
    /// Atom name (e.g., "CA", "N", "O")
    #[getter]
    fn name(&self) -> &str {
        &self.name
    }

    /// Chemical element
    #[getter]
    fn element(&self) -> PyElement {
        PyElement {
            inner: self.element,
        }
    }

    /// Element symbol (e.g., "C", "N", "O")
    #[getter]
    fn elem(&self) -> &'static str {
        self.element.symbol()
    }

    /// Residue name (e.g., "ALA", "GLY")
    #[getter]
    fn resn(&self) -> &str {
        &self.resn
    }

    /// Residue sequence number
    #[getter]
    fn resv(&self) -> i32 {
        self.resv
    }

    /// Alias for resv (residue ID)
    #[getter]
    fn resi(&self) -> i32 {
        self.resv
    }

    /// Insertion code
    #[getter]
    fn inscode(&self) -> char {
        self.inscode
    }

    /// Chain identifier
    #[getter]
    fn chain(&self) -> &str {
        &self.chain
    }

    /// Segment identifier
    #[getter]
    fn segi(&self) -> &str {
        &self.segi
    }

    /// Alternate location indicator
    #[getter]
    fn alt(&self) -> char {
        self.alt
    }

    /// B-factor (temperature factor)
    #[getter]
    fn b(&self) -> f32 {
        self.b_factor
    }

    /// Occupancy (0.0 to 1.0)
    #[getter]
    fn q(&self) -> f32 {
        self.occupancy
    }

    /// Van der Waals radius in Angstroms
    #[getter]
    fn vdw(&self) -> f32 {
        if self.vdw > 0.0 {
            self.vdw
        } else {
            self.element.vdw_radius()
        }
    }

    /// Partial charge
    #[getter]
    fn partial_charge(&self) -> f32 {
        self.partial_charge
    }

    /// Formal charge
    #[getter]
    fn formal_charge(&self) -> i8 {
        self.formal_charge
    }

    /// Color index
    #[getter]
    fn color(&self) -> i32 {
        self.color
    }

    /// Whether this is a HETATM
    #[getter]
    fn hetatm(&self) -> bool {
        self.hetatm
    }

    /// Coordinate as (x, y, z) tuple, or None if not available
    #[getter]
    fn coord(&self) -> Option<(f32, f32, f32)> {
        self.coord
    }

    /// X coordinate
    #[getter]
    fn x(&self) -> Option<f32> {
        self.coord.map(|(x, _, _)| x)
    }

    /// Y coordinate
    #[getter]
    fn y(&self) -> Option<f32> {
        self.coord.map(|(_, y, _)| y)
    }

    /// Z coordinate
    #[getter]
    fn z(&self) -> Option<f32> {
        self.coord.map(|(_, _, z)| z)
    }

    /// Secondary structure type ("H" = helix, "S" = sheet, "L" = loop)
    #[getter]
    fn ss(&self) -> &'static str {
        match self.ss_type {
            SecondaryStructure::Helix
            | SecondaryStructure::Helix310
            | SecondaryStructure::HelixPi => "H",
            SecondaryStructure::Sheet => "S",
            _ => "L",
        }
    }

    /// Index of this atom in the parent molecule
    #[getter]
    fn index(&self) -> usize {
        self.index
    }

    /// Check if this is a hydrogen atom
    fn is_hydrogen(&self) -> bool {
        self.element.is_hydrogen()
    }

    /// Check if this is a heavy atom (not hydrogen)
    fn is_heavy(&self) -> bool {
        !self.element.is_hydrogen() && !self.element.is_unknown()
    }

    /// Check if this is a C-alpha atom
    fn is_ca(&self) -> bool {
        self.name == "CA" && self.element.is_carbon()
    }

    /// Check if this is a backbone atom (N, CA, C, O)
    fn is_backbone(&self) -> bool {
        matches!(self.name.as_str(), "N" | "CA" | "C" | "O")
    }

    fn __repr__(&self) -> String {
        format!(
            "Atom({}, {}, {}{}, chain='{}')",
            self.name,
            self.element.symbol(),
            self.resn,
            self.resv,
            self.chain
        )
    }

    fn __str__(&self) -> String {
        format!("{}/{}{}/{}", self.chain, self.resn, self.resv, self.name)
    }
}

/// Iterator over atoms in a molecule
#[pyclass]
pub struct PyAtomIter {
    atoms: Vec<PyAtom>,
    index: usize,
}

impl PyAtomIter {
    pub fn new(atoms: Vec<PyAtom>) -> Self {
        PyAtomIter { atoms, index: 0 }
    }
}

#[pymethods]
impl PyAtomIter {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(mut slf: PyRefMut<'_, Self>) -> Option<PyAtom> {
        if slf.index < slf.atoms.len() {
            let atom = slf.atoms[slf.index].clone();
            slf.index += 1;
            Some(atom)
        } else {
            None
        }
    }

    fn __len__(&self) -> usize {
        self.atoms.len()
    }
}
