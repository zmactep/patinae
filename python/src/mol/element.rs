//! Python bindings for Element

use patinae_mol::Element;
use pyo3::prelude::*;

/// Python wrapper for chemical elements
#[pyclass(name = "Element")]
#[derive(Debug, Clone, Copy)]
pub struct PyElement {
    pub(crate) inner: Element,
}

impl From<Element> for PyElement {
    fn from(e: Element) -> Self {
        PyElement { inner: e }
    }
}

impl From<PyElement> for Element {
    fn from(e: PyElement) -> Self {
        e.inner
    }
}

#[pymethods]
impl PyElement {
    /// Create an element from atomic number
    #[new]
    #[pyo3(signature = (atomic_number=0))]
    fn new(atomic_number: u8) -> Self {
        PyElement {
            inner: Element::from_atomic_number(atomic_number).unwrap_or(Element::Unknown),
        }
    }

    /// Create an element from symbol (e.g., "C", "Fe", "Ca")
    #[staticmethod]
    fn from_symbol(symbol: &str) -> Option<Self> {
        Element::from_symbol(symbol).map(|e| PyElement { inner: e })
    }

    /// Get the atomic number
    #[getter]
    fn atomic_number(&self) -> u8 {
        self.inner.atomic_number()
    }

    /// Get the element symbol (e.g., "C", "N", "O")
    #[getter]
    fn symbol(&self) -> &'static str {
        self.inner.symbol()
    }

    /// Get the element name (e.g., "Carbon", "Nitrogen")
    #[getter]
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    /// Get the van der Waals radius in Angstroms
    #[getter]
    fn vdw_radius(&self) -> f32 {
        self.inner.vdw_radius()
    }

    /// Get the atomic mass in g/mol
    #[getter]
    fn mass(&self) -> f32 {
        self.inner.mass()
    }

    /// Check if this is hydrogen
    fn is_hydrogen(&self) -> bool {
        self.inner.is_hydrogen()
    }

    /// Check if this is carbon
    fn is_carbon(&self) -> bool {
        self.inner.is_carbon()
    }

    /// Check if this is an organic element (H, C, N, O, P, S)
    fn is_organic(&self) -> bool {
        self.inner.is_organic()
    }

    /// Check if this is a halogen (F, Cl, Br, I, At)
    fn is_halogen(&self) -> bool {
        self.inner.is_halogen()
    }

    /// Check if this is a metal
    fn is_metal(&self) -> bool {
        self.inner.is_metal()
    }

    /// Check if this is a noble gas
    fn is_noble_gas(&self) -> bool {
        self.inner.is_noble_gas()
    }

    /// Check if this is unknown/unspecified
    fn is_unknown(&self) -> bool {
        self.inner.is_unknown()
    }

    fn __repr__(&self) -> String {
        format!(
            "Element({}, '{}')",
            self.inner.atomic_number(),
            self.inner.symbol()
        )
    }

    fn __str__(&self) -> &'static str {
        self.inner.symbol()
    }

    fn __eq__(&self, other: &PyElement) -> bool {
        self.inner == other.inner
    }

    fn __hash__(&self) -> u64 {
        self.inner.atomic_number() as u64
    }
}

// Common element constants
impl PyElement {
    /// Hydrogen element
    pub const H: PyElement = PyElement {
        inner: Element::Hydrogen,
    };
    /// Carbon element
    pub const C: PyElement = PyElement {
        inner: Element::Carbon,
    };
    /// Nitrogen element
    pub const N: PyElement = PyElement {
        inner: Element::Nitrogen,
    };
    /// Oxygen element
    pub const O: PyElement = PyElement {
        inner: Element::Oxygen,
    };
    /// Sulfur element
    pub const S: PyElement = PyElement {
        inner: Element::Sulfur,
    };
    /// Phosphorus element
    pub const P: PyElement = PyElement {
        inner: Element::Phosphorus,
    };
}
