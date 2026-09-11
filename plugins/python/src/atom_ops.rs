use std::ffi::CString;

use pyo3::prelude::*;
use pyo3::types::PyDict;

/// Build the globals dict for `iterate`/`alter` expression evaluation.
pub fn build_globals<'py>(
    py: Python<'py>,
    space: Option<&Bound<'py, PyDict>>,
) -> PyResult<Bound<'py, PyDict>> {
    let globals = py.import("__main__")?.dict();
    if let Some(s) = space {
        globals.update(s.as_mapping())?;
    }
    Ok(globals)
}

pub fn expression_to_cstring(expression: &str) -> PyResult<CString> {
    CString::new(expression)
        .map_err(|_| pyo3::exceptions::PyRuntimeError::new_err("Expression contains null byte"))
}
