//! Settings module for Python
//!
//! Provides access to runtime settings.

use patinae_settings::{GlobalSettings, SettingType, SettingValue};
use pyo3::prelude::*;

/// Get a setting value by name
pub fn get_setting_py<'py>(
    py: Python<'py>,
    settings: &GlobalSettings,
    name: &str,
) -> PyResult<Py<PyAny>> {
    let id = patinae_settings::get_setting_id(name).ok_or_else(|| {
        pyo3::exceptions::PyKeyError::new_err(format!("Unknown setting: {}", name))
    })?;

    let value = settings.get(id).ok_or_else(|| {
        pyo3::exceptions::PyKeyError::new_err(format!("Setting {} not found", name))
    })?;
    setting_value_to_py(py, &value)
}

/// Set a setting value by name
pub fn set_setting_py(
    settings: &mut GlobalSettings,
    name: &str,
    value: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let id = patinae_settings::get_setting_id(name).ok_or_else(|| {
        pyo3::exceptions::PyKeyError::new_err(format!("Unknown setting: {}", name))
    })?;

    let setting = patinae_settings::get_setting(id).ok_or_else(|| {
        pyo3::exceptions::PyKeyError::new_err(format!("Setting ID not found: {}", id))
    })?;

    let rust_value = py_to_setting_value(value, setting.setting_type)?;
    settings.set(id, rust_value).map_err(|e| {
        pyo3::exceptions::PyValueError::new_err(format!("Failed to set setting: {}", e))
    })?;
    Ok(())
}

/// Convert a Rust SettingValue to Python object
fn setting_value_to_py(py: Python<'_>, value: &SettingValue) -> PyResult<Py<PyAny>> {
    match value {
        SettingValue::Bool(b) => Ok((*b).into_pyobject(py)?.to_owned().into_any().unbind()),
        SettingValue::Int(i) => Ok((*i).into_pyobject(py)?.to_owned().into_any().unbind()),
        SettingValue::Float(f) => Ok((*f).into_pyobject(py)?.to_owned().into_any().unbind()),
        SettingValue::Float3(f) => Ok((f[0], f[1], f[2]).into_pyobject(py)?.into_any().unbind()),
        SettingValue::Color(c) => Ok((*c).into_pyobject(py)?.to_owned().into_any().unbind()),
        SettingValue::String(s) => Ok(s.clone().into_pyobject(py)?.into_any().unbind()),
    }
}

/// Convert a Python object to SettingValue
fn py_to_setting_value(
    value: &Bound<'_, PyAny>,
    expected_type: SettingType,
) -> PyResult<SettingValue> {
    match expected_type {
        SettingType::Bool => {
            let b: bool = value.extract()?;
            Ok(SettingValue::Bool(b))
        }
        SettingType::Int => {
            let i: i32 = value.extract()?;
            Ok(SettingValue::Int(i))
        }
        SettingType::Float => {
            let f: f32 = value.extract()?;
            Ok(SettingValue::Float(f))
        }
        SettingType::Float3 => {
            let (x, y, z): (f32, f32, f32) = value.extract()?;
            Ok(SettingValue::Float3([x, y, z]))
        }
        SettingType::Color => {
            let i: i32 = value.extract()?;
            Ok(SettingValue::Color(i))
        }
        SettingType::String => {
            let s: String = value.extract()?;
            Ok(SettingValue::String(s))
        }
        SettingType::Blank => Err(pyo3::exceptions::PyValueError::new_err(
            "Cannot set a blank setting",
        )),
    }
}

/// List all setting names
#[pyfunction]
pub fn list_settings() -> Vec<&'static str> {
    patinae_settings::setting_names()
}

/// Get setting names matching a pattern
#[pyfunction]
pub fn search_settings(pattern: &str) -> Vec<&'static str> {
    let pattern_lower = pattern.to_lowercase();
    patinae_settings::setting_names()
        .into_iter()
        .filter(|name| name.to_lowercase().contains(&pattern_lower))
        .collect()
}

/// Register the settings submodule
pub fn register_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(list_settings, m)?)?;
    m.add_function(wrap_pyfunction!(search_settings, m)?)?;
    m.add("SETTING_COUNT", patinae_settings::SETTING_COUNT)?;
    Ok(())
}
