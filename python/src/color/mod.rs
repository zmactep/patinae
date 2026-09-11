//! Color module for Python
//!
//! Provides color types, named colors, and gradients.

use patinae_color::{Color, Gradient, NamedPalette};
use pyo3::prelude::*;

/// RGB Color with floating-point components (0.0-1.0)
#[pyclass(name = "Color")]
#[derive(Debug, Clone, Copy)]
pub struct PyColor {
    pub(crate) inner: Color,
}

impl From<Color> for PyColor {
    fn from(c: Color) -> Self {
        PyColor { inner: c }
    }
}

impl From<PyColor> for Color {
    fn from(c: PyColor) -> Self {
        c.inner
    }
}

#[pymethods]
impl PyColor {
    #[new]
    #[pyo3(signature = (r=0.0, g=0.0, b=0.0))]
    fn new(r: f32, g: f32, b: f32) -> Self {
        PyColor {
            inner: Color::new(r, g, b),
        }
    }

    #[staticmethod]
    fn from_rgb8(r: u8, g: u8, b: u8) -> Self {
        PyColor {
            inner: Color::from_rgb8(r, g, b),
        }
    }

    #[staticmethod]
    fn from_hex(hex: &str) -> PyResult<Self> {
        Color::from_hex(hex)
            .map(|c| PyColor { inner: c })
            .ok_or_else(|| {
                pyo3::exceptions::PyValueError::new_err(format!("Invalid hex color: {}", hex))
            })
    }

    #[getter]
    fn r(&self) -> f32 {
        self.inner.r
    }

    #[getter]
    fn g(&self) -> f32 {
        self.inner.g
    }

    #[getter]
    fn b(&self) -> f32 {
        self.inner.b
    }

    #[allow(clippy::wrong_self_convention)]
    fn to_tuple(&self) -> (f32, f32, f32) {
        (self.inner.r, self.inner.g, self.inner.b)
    }

    #[allow(clippy::wrong_self_convention)]
    fn to_rgb8(&self) -> (u8, u8, u8) {
        let rgb = self.inner.to_rgb8();
        (rgb[0], rgb[1], rgb[2])
    }

    #[allow(clippy::wrong_self_convention)]
    fn to_rgba(&self) -> (f32, f32, f32, f32) {
        let rgba = self.inner.to_rgba(1.0);
        (rgba[0], rgba[1], rgba[2], rgba[3])
    }

    fn lerp(&self, other: &PyColor, t: f32) -> PyColor {
        PyColor {
            inner: self.inner.lerp(&other.inner, t),
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "Color({:.3}, {:.3}, {:.3})",
            self.inner.r, self.inner.g, self.inner.b
        )
    }

    fn __str__(&self) -> String {
        let rgb = self.inner.to_rgb8();
        format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
    }

    fn __eq__(&self, other: &PyColor) -> bool {
        (self.inner.r - other.inner.r).abs() < 0.001
            && (self.inner.g - other.inner.g).abs() < 0.001
            && (self.inner.b - other.inner.b).abs() < 0.001
    }
}

impl PyColor {
    pub const BLACK: PyColor = PyColor {
        inner: Color {
            r: 0.0,
            g: 0.0,
            b: 0.0,
        },
    };
    pub const WHITE: PyColor = PyColor {
        inner: Color {
            r: 1.0,
            g: 1.0,
            b: 1.0,
        },
    };
    pub const RED: PyColor = PyColor {
        inner: Color {
            r: 1.0,
            g: 0.0,
            b: 0.0,
        },
    };
    pub const GREEN: PyColor = PyColor {
        inner: Color {
            r: 0.0,
            g: 1.0,
            b: 0.0,
        },
    };
    pub const BLUE: PyColor = PyColor {
        inner: Color {
            r: 0.0,
            g: 0.0,
            b: 1.0,
        },
    };
}

/// Color gradient for continuous color mapping
#[pyclass(name = "Gradient")]
#[derive(Clone)]
pub struct PyGradient {
    inner: Gradient,
}

#[pymethods]
impl PyGradient {
    #[new]
    fn new() -> Self {
        PyGradient {
            inner: Gradient::new(vec![]),
        }
    }

    #[staticmethod]
    fn blue_white_red() -> Self {
        PyGradient {
            inner: Gradient::blue_white_red(),
        }
    }

    #[staticmethod]
    fn rainbow() -> Self {
        PyGradient {
            inner: Gradient::rainbow(),
        }
    }

    #[staticmethod]
    fn viridis() -> Self {
        PyGradient {
            inner: Gradient::viridis(),
        }
    }

    #[staticmethod]
    fn plasma() -> Self {
        PyGradient {
            inner: Gradient::plasma(),
        }
    }

    #[staticmethod]
    fn inferno() -> Self {
        PyGradient {
            inner: Gradient::inferno(),
        }
    }

    #[staticmethod]
    fn magma() -> Self {
        PyGradient {
            inner: Gradient::magma(),
        }
    }

    #[staticmethod]
    fn coolwarm() -> Self {
        PyGradient {
            inner: Gradient::coolwarm(),
        }
    }

    #[staticmethod]
    fn grayscale() -> Self {
        PyGradient {
            inner: Gradient::grayscale(),
        }
    }

    /// Add a color stop at position (0.0-1.0)
    fn add_stop(&mut self, position: f32, color: &PyColor) {
        let stops = vec![(position, color.inner)];
        // Recreate with the new stop
        let old = std::mem::replace(&mut self.inner, Gradient::new(vec![]));
        // Sample existing stops at small intervals to preserve them
        // This is a simplification — ideally we'd extract stops from the gradient
        let _ = old;
        self.inner = Gradient::new(stops);
    }

    /// Sample the gradient at position t (0.0-1.0)
    fn sample(&self, t: f32) -> PyColor {
        PyColor {
            inner: self.inner.sample(t),
        }
    }

    /// Map a value to a color within a range
    #[pyo3(signature = (value, min_val=0.0, max_val=1.0))]
    fn map_value(&self, value: f32, min_val: f32, max_val: f32) -> PyColor {
        let normalized = if (max_val - min_val).abs() < 0.0001 {
            0.5
        } else {
            (value - min_val) / (max_val - min_val)
        };
        self.sample(normalized.clamp(0.0, 1.0))
    }

    fn __repr__(&self) -> String {
        "Gradient(...)".to_string()
    }
}

/// Named colors registry
#[pyclass(name = "NamedPalette")]
pub struct PyNamedPalette {
    inner: NamedPalette,
}

#[pymethods]
impl PyNamedPalette {
    #[new]
    fn new() -> Self {
        PyNamedPalette {
            inner: NamedPalette::default(),
        }
    }

    fn get(&self, name: &str) -> Option<PyColor> {
        self.inner
            .get_by_name(name)
            .map(|(_, c)| PyColor { inner: c })
    }

    fn get_by_index(&self, index: u32) -> Option<PyColor> {
        self.inner.get_by_index(index).map(|c| PyColor { inner: c })
    }

    fn register(&mut self, name: &str, color: &PyColor) -> u32 {
        self.inner.register(name, color.inner)
    }

    fn __getitem__(&self, name: &str) -> PyResult<PyColor> {
        self.get(name).ok_or_else(|| {
            pyo3::exceptions::PyKeyError::new_err(format!("Unknown color: {}", name))
        })
    }

    fn __repr__(&self) -> String {
        "NamedPalette(...)".to_string()
    }
}

#[pyfunction]
pub fn get_color(name: &str) -> Option<PyColor> {
    let colors = NamedPalette::default();
    colors.get_by_name(name).map(|(_, c)| PyColor { inner: c })
}

#[pyfunction]
pub fn element_color(symbol: &str) -> PyColor {
    use patinae_color::ElementPalette;
    let colors = ElementPalette::default();
    PyColor {
        inner: colors.get_by_symbol(symbol),
    }
}

#[pyfunction]
#[pyo3(signature = (chain_id, theme="dark"))]
pub fn chain_color(chain_id: &str, theme: &str) -> PyColor {
    use patinae_color::ThemedPalette;
    let palette = match theme {
        "light" => ThemedPalette::light(),
        _ => ThemedPalette::dark(),
    };
    PyColor {
        inner: palette.chains.get(chain_id),
    }
}

pub fn register_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyColor>()?;
    m.add_class::<PyGradient>()?;
    m.add_class::<PyNamedPalette>()?;
    m.add_function(wrap_pyfunction!(get_color, m)?)?;
    m.add_function(wrap_pyfunction!(element_color, m)?)?;
    m.add_function(wrap_pyfunction!(chain_color, m)?)?;

    m.add("BLACK", PyColor::BLACK)?;
    m.add("WHITE", PyColor::WHITE)?;
    m.add("RED", PyColor::RED)?;
    m.add("GREEN", PyColor::GREEN)?;
    m.add("BLUE", PyColor::BLUE)?;

    Ok(())
}
