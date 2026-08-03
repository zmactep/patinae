//! Standalone backend for the patinae Python package.
//!
//! Provides direct access to a Session + CommandExecutor without IPC.
//! Used when running Python scripts standalone (not embedded in the GUI).

use std::ffi::CString;

use pyo3::exceptions::{PyKeyError, PyRuntimeError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use patinae_cmd::CommandExecutor;
use patinae_mol::AtomIndex;
use patinae_scene::{
    label_object_view, LabelEntityView, LabelObjectView, Session, SessionAdapter, ViewportImage,
};
use patinae_select::select;

use crate::iterate::{apply_locals_to_atom, build_globals, set_atom_locals};
use crate::mol::PyObjectMolecule;

fn label_entity_to_py<'py>(
    py: Python<'py>,
    entity: &LabelEntityView,
) -> PyResult<Bound<'py, PyDict>> {
    let anchor = PyDict::new(py);
    anchor.set_item("object_name", &entity.anchor.object_name)?;
    anchor.set_item("atom_index", entity.anchor.atom_index)?;
    anchor.set_item("orphaned", entity.anchor.orphaned)?;
    anchor.set_item("resolved", entity.anchor.resolved)?;

    let result = PyDict::new(py);
    result.set_item("anchor", anchor)?;
    result.set_item("text", &entity.text)?;
    result.set_item("color", entity.color)?;
    result.set_item("color_override_index", entity.color_override_index)?;
    result.set_item("size", entity.size)?;
    result.set_item("size_override", entity.size_override)?;
    result.set_item("visible", entity.visible)?;
    result.set_item("visible_override", entity.visible_override)?;
    Ok(result)
}

fn label_object_to_py(py: Python<'_>, label: &LabelObjectView) -> PyResult<Py<PyAny>> {
    let entities = PyList::empty(py);
    for entity in &label.entities {
        entities.append(label_entity_to_py(py, entity)?)?;
    }

    let revisions = PyDict::new(py);
    revisions.set_item("geometry", label.revisions.geometry)?;
    revisions.set_item("material", label.revisions.material)?;
    revisions.set_item("labels", label.revisions.labels)?;

    let result = PyDict::new(py);
    result.set_item("name", &label.name)?;
    result.set_item("enabled", label.enabled)?;
    result.set_item("color", label.color)?;
    result.set_item("color_override_index", label.color_override_index)?;
    result.set_item("size", label.size)?;
    result.set_item("size_override", label.size_override)?;
    result.set_item("visible", label.visible)?;
    result.set_item("visible_override", label.visible_override)?;
    result.set_item("alignment", &label.alignment)?;
    result.set_item("alignment_override", &label.alignment_override)?;
    result.set_item("entities", entities)?;
    result.set_item("unresolved_count", label.unresolved_count)?;
    result.set_item("revisions", revisions)?;
    Ok(result.into_any().unbind())
}

/// Standalone backend — owns a Session and CommandExecutor.
///
/// All commands execute directly against the local session.
/// No IPC, no GUI — pure headless operation.
#[pyclass]
pub struct StandaloneBackend {
    session: Session,
    executor: CommandExecutor,
    needs_redraw: bool,
}

impl StandaloneBackend {
    pub fn create() -> Self {
        Self {
            session: Session::new(),
            executor: CommandExecutor::new(),
            needs_redraw: false,
        }
    }
}

#[pymethods]
impl StandaloneBackend {
    #[new]
    fn new() -> Self {
        Self {
            session: Session::new(),
            executor: CommandExecutor::new(),
            needs_redraw: false,
        }
    }

    /// Execute a command string.
    #[pyo3(signature = (command, silent=false))]
    fn execute(&mut self, command: &str, silent: bool) -> PyResult<()> {
        let mut adapter = SessionAdapter {
            session: &mut self.session,
            render_context: None,
            default_size: (1024, 768),
            needs_redraw: &mut self.needs_redraw,
            async_fetch_fn: None,
        };
        self.executor
            .do_with_options(&mut adapter, command, silent)
            .map(|_| ())
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }

    /// Advance movie, rock, and camera animations by an explicit delta time.
    fn update_animations(&mut self, dt: f32) -> bool {
        let update = self.session.update_animations(dt);
        if update.needs_redraw {
            self.needs_redraw = true;
        }
        update.needs_redraw
    }

    /// Get the current movie state as a Python dict.
    fn get_movie_state(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let state = self.session.movie_state_snapshot();
        let dict = PyDict::new(py);
        dict.set_item("frame_count", state.frame_count)?;
        dict.set_item("current_frame", state.current_frame)?;
        dict.set_item("is_playing", state.is_playing)?;
        dict.set_item("rock_enabled", state.rock_enabled)?;
        Ok(dict.into_any().unbind())
    }

    /// Get a molecular object by name.
    fn get_model(&self, name: &str) -> PyResult<PyObjectMolecule> {
        let mol_obj = self
            .session
            .registry
            .get_molecule(name)
            .ok_or_else(|| PyKeyError::new_err(format!("Object '{}' not found", name)))?;
        Ok(mol_obj.molecule().clone().into())
    }

    /// Get the names of all loaded objects.
    fn get_names(&self) -> Vec<String> {
        self.session
            .registry
            .names()
            .map(|s| s.to_string())
            .collect()
    }

    /// Get a read-only label object snapshot.
    fn get_label(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        let label = label_object_view(
            &self.session.registry,
            &self.session.settings,
            &self.session.named_palette,
            name,
        )
        .ok_or_else(|| PyKeyError::new_err(format!("label object '{}' not found", name)))?;
        label_object_to_py(py, &label)
    }

    /// Count atoms matching a selection expression.
    #[pyo3(signature = (selection="all"))]
    fn count_atoms(&self, selection: &str) -> PyResult<usize> {
        let names: Vec<String> = self
            .session
            .registry
            .names()
            .map(|s| s.to_string())
            .collect();
        let mut total = 0;
        for name in &names {
            if let Some(mol_obj) = self.session.registry.get_molecule(name) {
                let mol = mol_obj.molecule();
                let result = select(mol, selection);
                match result {
                    Ok(mask) => total += mask.count(),
                    Err(e) => {
                        return Err(PyRuntimeError::new_err(format!("Selection error: {}", e)))
                    }
                }
            }
        }
        Ok(total)
    }

    /// Execute a Python expression for each atom matching a selection (read-only).
    ///
    /// The expression has access to atom properties as local variables:
    /// name, resn, resv, resi, chain, segi, alt, elem, b, q, vdw,
    /// partial_charge, formal_charge, ss, color, type, hetatm,
    /// index, ID, rank, model, x, y, z.
    #[pyo3(signature = (selection, expression, space=None))]
    fn iterate(
        &self,
        py: Python<'_>,
        selection: &str,
        expression: &str,
        space: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        let globals = build_globals(py, space)?;

        let code = CString::new(expression)
            .map_err(|_| PyRuntimeError::new_err("Expression contains null byte"))?;

        let names = self.get_names();
        for name in &names {
            if let Some(mol_obj) = self.session.registry.get_molecule(name) {
                let mol = mol_obj.molecule();
                let mask = select(mol, selection)
                    .map_err(|e| PyRuntimeError::new_err(format!("Selection error: {}", e)))?;

                let cs = mol.current_coord_set();
                for idx in mask.raw_indices() {
                    let atom = mol.get_atom(AtomIndex(idx as u32)).unwrap();
                    let coord = cs
                        .and_then(|c| c.get_atom_coord(AtomIndex(idx as u32)))
                        .map(|v| (v.x, v.y, v.z));
                    let locals = PyDict::new(py);
                    set_atom_locals(&locals, atom, coord, idx, name)?;
                    py.run(code.as_c_str(), Some(&globals), Some(&locals))?;
                }
            }
        }
        Ok(())
    }

    /// Execute a Python expression for each atom matching a selection,
    /// allowing modification of atom properties.
    ///
    /// Mutable properties: name, resn, resv, chain, segi, alt, elem,
    /// b, q, vdw, partial_charge, formal_charge, ss, color, type.
    /// Coordinates (x, y, z) are read-only (use alter_state for coords).
    #[pyo3(signature = (selection, expression, space=None))]
    fn alter(
        &mut self,
        py: Python<'_>,
        selection: &str,
        expression: &str,
        space: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        let globals = build_globals(py, space)?;

        let code = CString::new(expression)
            .map_err(|_| PyRuntimeError::new_err("Expression contains null byte"))?;

        let names = self.get_names();
        for name in &names {
            // First: select atoms (immutable borrow)
            let indices: Vec<usize> = {
                let Some(mol_obj) = self.session.registry.get_molecule(name) else {
                    continue;
                };
                let mol = mol_obj.molecule();
                let mask = select(mol, selection)
                    .map_err(|e| PyRuntimeError::new_err(format!("Selection error: {}", e)))?;
                mask.raw_indices().collect()
            };
            if indices.is_empty() {
                continue;
            }

            // Pre-read coordinates (immutable borrow)
            let coords: Vec<Option<(f32, f32, f32)>> = {
                let mol = self.session.registry.get_molecule(name).unwrap().molecule();
                let cs = mol.current_coord_set();
                indices
                    .iter()
                    .map(|&idx| {
                        cs.and_then(|c| c.get_atom_coord(AtomIndex(idx as u32)))
                            .map(|v| (v.x, v.y, v.z))
                    })
                    .collect()
            };

            // Iterate: read atom, run expression, apply changes
            let mol_obj = self.session.registry.get_molecule_mut(name).unwrap();
            for (i, &idx) in indices.iter().enumerate() {
                let locals = PyDict::new(py);
                {
                    let atom = mol_obj.molecule().get_atom(AtomIndex(idx as u32)).unwrap();
                    set_atom_locals(&locals, atom, coords[i], idx, name)?;
                }
                py.run(code.as_c_str(), Some(&globals), Some(&locals))?;
                {
                    let atom = mol_obj
                        .molecule_mut()
                        .get_atom_mut(AtomIndex(idx as u32))
                        .unwrap();
                    apply_locals_to_atom(&locals, atom)?;
                }
            }
        }
        self.needs_redraw = true;
        Ok(())
    }

    /// Get the current viewport image as a numpy array (H, W, 4) uint8.
    ///
    /// Returns `None` if no viewport image is set.
    #[cfg(feature = "numpy")]
    fn get_viewport_image<'py>(&self, py: Python<'py>) -> PyResult<Option<Py<PyAny>>> {
        use numpy::ndarray::Array3;
        use numpy::IntoPyArray;

        match &self.session.viewport_image {
            Some(img) => {
                let h = img.height as usize;
                let w = img.width as usize;
                let arr = Array3::from_shape_vec((h, w, 4), img.data.clone()).map_err(|e| {
                    PyRuntimeError::new_err(format!("Failed to create array: {}", e))
                })?;
                Ok(Some(arr.into_pyarray(py).into_any().unbind()))
            }
            None => Ok(None),
        }
    }

    /// Set the viewport image from a numpy array (H, W, 4) uint8.
    #[cfg(feature = "numpy")]
    fn set_viewport_image(&mut self, array: &Bound<'_, PyAny>) -> PyResult<()> {
        use numpy::{PyArray3, PyArrayMethods, PyUntypedArrayMethods};

        let arr = array.cast::<PyArray3<u8>>().map_err(|_| {
            pyo3::exceptions::PyTypeError::new_err(
                "Expected numpy array with shape (H, W, 4) and dtype uint8",
            )
        })?;
        let shape = arr.shape();
        if shape.len() != 3 || shape[2] != 4 {
            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Expected shape (H, W, 4), got ({}, {}, {})",
                shape[0], shape[1], shape[2]
            )));
        }
        let height = shape[0] as u32;
        let width = shape[1] as u32;
        let data = arr.to_vec()?;
        self.session.viewport_image = Some(ViewportImage {
            data,
            width,
            height,
        });
        Ok(())
    }

    /// Clear the viewport image.
    fn clear_viewport_image(&mut self) {
        self.session.viewport_image = None;
    }

    /// No-op in standalone mode (no GUI event loop).
    fn set_key(&self, _key: &str, _callback: Py<PyAny>) -> PyResult<()> {
        Ok(())
    }

    /// No-op in standalone mode (no GUI event loop).
    fn unset_key(&self, _key: &str) -> PyResult<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use patinae_scene::{AtomAnchor, LabelEntity, LabelObject};

    use super::*;

    #[test]
    fn standalone_movie_state_and_explicit_animation_tick_are_deterministic() {
        let mut backend = StandaloneBackend::create();
        backend.session.settings.movie.movie_fps = 10.0;

        backend.execute("mset 1 x3", true).unwrap();
        backend.execute("mplay", true).unwrap();

        assert!(backend.update_animations(0.11));
        assert_eq!(backend.session.movie.current_frame(), 1);

        Python::attach(|py| {
            let state = backend.get_movie_state(py).unwrap();
            let dict = state.bind(py).cast::<PyDict>().unwrap();
            assert_eq!(
                dict.get_item("frame_count")
                    .unwrap()
                    .unwrap()
                    .extract::<usize>()
                    .unwrap(),
                3
            );
            assert_eq!(
                dict.get_item("current_frame")
                    .unwrap()
                    .unwrap()
                    .extract::<usize>()
                    .unwrap(),
                1
            );
            assert!(dict
                .get_item("is_playing")
                .unwrap()
                .unwrap()
                .extract::<bool>()
                .unwrap());
        });
    }

    #[test]
    fn standalone_label_snapshot_preserves_entities_and_resolution_state() {
        let mut backend = StandaloneBackend::create();
        backend.session.registry.add(LabelObject::with_entities(
            "notes",
            vec![LabelEntity::new(
                AtomAnchor::new("missing", AtomIndex(3)),
                "orphan candidate",
            )],
        ));

        Python::attach(|py| {
            let snapshot = backend.get_label(py, "notes").unwrap();
            let dict = snapshot.bind(py).cast::<PyDict>().unwrap();
            assert_eq!(
                dict.get_item("name")
                    .unwrap()
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                "notes"
            );
            assert_eq!(
                dict.get_item("unresolved_count")
                    .unwrap()
                    .unwrap()
                    .extract::<usize>()
                    .unwrap(),
                1
            );
            let entities_value = dict.get_item("entities").unwrap().unwrap();
            let entities = entities_value.cast::<PyList>().unwrap();
            let entity_value = entities.get_item(0).unwrap();
            let entity = entity_value.cast::<PyDict>().unwrap();
            assert_eq!(
                entity
                    .get_item("text")
                    .unwrap()
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                "orphan candidate"
            );
        });
    }
}
