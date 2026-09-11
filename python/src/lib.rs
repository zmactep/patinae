//! Python bindings.
//!
//! Unified Python package for patinae. Works in two modes:
//! - **Standalone**: Direct Session + CommandExecutor (no IPC)
//! - **Embedded**: Plugin backend injected via `sys._patinae_backend`

use pyo3::prelude::*;
use pyo3::types::PyModule;

mod backend;
mod convert;
mod error;
pub mod iterate;
mod owner;

pub mod color;
pub mod io;
pub mod mol;
pub mod selecting;
pub mod settings;

pub use color::PyColor;
pub use error::{PatinaeError, SelectionError};
pub use mol::{PyAtom, PyBond, PyCoordSet, PyElement, PyObjectMolecule};
pub use selecting::PySelectionResult;

/// Python module initialization
#[pymodule]
fn _patinae(py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    // Register exception types
    m.add("PatinaeError", py.get_type::<error::PatinaeError>())?;
    m.add("SelectionError", py.get_type::<error::SelectionError>())?;

    // Register main types
    m.add_class::<backend::StandaloneBackend>()?;
    m.add_class::<PyObjectMolecule>()?;
    m.add_class::<mol::PyAtom>()?;
    m.add_class::<mol::PyBond>()?;
    m.add_class::<mol::PyElement>()?;
    m.add_class::<mol::PyCoordSet>()?;
    m.add_class::<color::PyColor>()?;
    m.add_class::<selecting::PySelectionResult>()?;

    // Register standalone backend factory
    m.add_function(wrap_pyfunction!(_create_standalone_backend, m)?)?;

    // Register submodules
    register_submodule(py, m, "mol", mol::register_module)?;
    register_submodule(py, m, "io", io::register_module)?;
    register_submodule(py, m, "selecting", selecting::register_module)?;
    register_submodule(py, m, "color", color::register_module)?;
    register_submodule(py, m, "settings", settings::register_module)?;

    // Add version info
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;

    Ok(())
}

/// Create a standalone backend (Session + CommandExecutor, no IPC).
#[pyfunction]
fn _create_standalone_backend() -> PyResult<backend::StandaloneBackend> {
    backend::StandaloneBackend::create()
}

/// Helper function to register a submodule
fn register_submodule<F>(
    py: Python<'_>,
    parent: &Bound<'_, PyModule>,
    name: &str,
    register_fn: F,
) -> PyResult<()>
where
    F: FnOnce(&Bound<'_, PyModule>) -> PyResult<()>,
{
    let submodule = PyModule::new(py, name)?;
    register_fn(&submodule)?;
    parent.add_submodule(&submodule)?;
    Ok(())
}
