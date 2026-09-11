//! Standalone backend for the patinae Python package.
//!
//! Provides direct access to a Session + CommandExecutor without IPC.
//! Used when running Python scripts standalone (not embedded in the GUI).

use std::ffi::CString;

use pyo3::exceptions::{PyKeyError, PyRuntimeError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use patinae_mol::AtomIndex;
use patinae_scene::{label_object_view, LabelEntityView, LabelObjectView, ViewportImage};

use crate::iterate::{apply_locals_to_atom, build_globals, set_atom_locals};
use crate::mol::PyObjectMolecule;

type MoleculeSnapshots = Vec<(String, patinae_mol::ObjectMolecule, Vec<usize>)>;

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

use crate::owner::{Client, Message, SessionOwner};
use patinae_cmd::tasks::{TaskId, TaskListRequest};
use std::{
    sync::{mpsc, Arc},
    time::{Duration, Instant},
};

// The binding transports error fields; the Python API owns exception semantics.
fn task_error(code: &str, message: &str) -> PyErr {
    let error = pyo3::exceptions::PyRuntimeError::new_err(format!("{code}: {message}"));
    Python::attach(|py| {
        let value = error.value(py);
        value.setattr("code", code)?;
        value.setattr("message", message)
    })
    .err()
    .unwrap_or(error)
}

/// Python handle to a session serviced by its dedicated owner thread.
#[pyclass]
pub struct StandaloneBackend {
    client: Arc<Client>,
    parent: Option<TaskId>,
}

fn to_python<T: serde::Serialize>(py: Python<'_>, value: &T) -> PyResult<Py<PyAny>> {
    let json =
        serde_json::to_string(value).map_err(|error| PyRuntimeError::new_err(error.to_string()))?;
    Ok(py.import("json")?.call_method1("loads", (json,))?.unbind())
}

impl StandaloneBackend {
    pub fn create() -> PyResult<Self> {
        Ok(Self {
            client: crate::owner::start().map_err(PyRuntimeError::new_err)?,
            parent: None,
        })
    }

    pub(crate) fn for_task(client: Arc<Client>, id: TaskId) -> Self {
        Self {
            client,
            parent: Some(id),
        }
    }

    pub(crate) fn record_output(&self, py: Python<'_>, text: &str, error: bool) -> PyResult<()> {
        let Some(id) = self.parent else {
            return Ok(());
        };
        // The registry bounds retained diagnostics; also bound each transport
        // payload before handing it to the owner and acknowledge every write.
        let message: String = text.chars().take(4096).collect();
        self.call(py, false, move |owner| {
            owner
                .tasks
                .output(
                    id,
                    "python",
                    patinae_cmd::tasks::TaskDiagnostic {
                        level: if error { "error" } else { "info" }.into(),
                        message,
                    },
                )
                .map_err(|error| task_error(&error.code, &error.message))?;
            Ok(())
        })
    }

    fn call<T: Send + 'static>(
        &self,
        py: Python<'_>,
        mutation: bool,
        call: impl FnOnce(&mut SessionOwner) -> PyResult<T> + Send + 'static,
    ) -> PyResult<T> {
        let tx = self.client.tx.clone();
        let parent = self.parent;
        py.detach(move || {
            let (reply_tx, reply_rx) = mpsc::sync_channel(1);
            tx.send(Message::Call(Box::new(move |owner| {
                let result = if let Some(id) = parent.filter(|_| mutation) {
                    owner
                        .tasks
                        .can_apply_effect(id, "python", owner.session.task_epoch())
                        .map_err(|error| task_error(&error.code, &error.message))
                        .and_then(|()| call(owner))
                } else {
                    call(owner)
                };
                if mutation && result.is_ok() {
                    owner.revision = owner.revision.wrapping_add(1);
                    if let Some(id) = parent {
                        let _ = owner.tasks.record_effects(
                            id,
                            "python",
                            patinae_cmd::tasks::TaskEffects::Applied,
                        );
                    }
                }
                let _ = reply_tx.send(result);
            })))
            .map_err(|_| task_error("executor_lost", "session owner stopped"))?;
            reply_rx
                .recv()
                .map_err(|_| task_error("executor_lost", "session owner stopped"))?
        })
    }

    fn molecule_snapshot(
        &self,
        py: Python<'_>,
        selection: &str,
        mutable: bool,
    ) -> PyResult<(u64, MoleculeSnapshots)> {
        let selection = selection.to_string();
        self.call(py, false, move |owner| {
            let adapter = patinae_scene::SessionAdapter {
                session: &mut owner.session,
                render_context: None,
                default_size: (1024, 768),
                needs_redraw: &mut owner.needs_redraw,
            };
            let selections =
                patinae_cmd::commands::selecting::evaluate_selection(&adapter, &selection)
                    .map_err(|error| PyRuntimeError::new_err(error.to_string()))?;
            let mut molecules = Vec::new();
            for (name, mask) in selections {
                if let Some(object) = owner.session.registry.get_molecule(&name) {
                    let indices: Vec<_> = mask.raw_indices().collect();
                    if indices.is_empty() {
                        continue;
                    }
                    if mutable {
                        object.require_explicit().map_err(PyRuntimeError::new_err)?;
                    }
                    molecules.push((name.to_string(), object.molecule().clone(), indices));
                }
            }
            Ok((owner.revision, molecules))
        })
    }
}

#[pymethods]
impl StandaloneBackend {
    #[new]
    fn new() -> PyResult<Self> {
        Self::create()
    }

    /// Execute a command and retain accepted task identities on failure.
    #[pyo3(signature = (command, quiet=false))]
    fn execute(&self, py: Python<'_>, command: &str, quiet: bool) -> PyResult<Py<PyAny>> {
        let command = command.to_string();
        let parent = self.parent;
        let receipt = self.call(py, false, move |owner| {
            Ok(owner.execute(&command, quiet, parent))
        })?;
        to_python(py, &receipt)
    }

    fn get_task(&self, py: Python<'_>, id: &str) -> PyResult<Py<PyAny>> {
        let id: TaskId = id
            .parse()
            .map_err(|error: patinae_cmd::tasks::TaskLookupError| {
                task_error(&error.to_string(), &error.to_string())
            })?;
        let snapshot = self.call(py, false, move |owner| {
            owner
                .tasks
                .get(id)
                .map_err(|error| task_error(&error.to_string(), &error.to_string()))
        })?;
        to_python(py, &snapshot)
    }

    #[pyo3(signature = (request=None))]
    fn list_tasks(
        &self,
        py: Python<'_>,
        request: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        let request: TaskListRequest = if let Some(request) = request {
            let encoded: String = py
                .import("json")?
                .call_method1("dumps", (request,))?
                .extract()?;
            serde_json::from_str(&encoded)
                .map_err(|error| pyo3::exceptions::PyValueError::new_err(error.to_string()))?
        } else {
            TaskListRequest::default()
        };
        let page = self.call(py, false, move |owner| {
            owner
                .tasks
                .list(&request)
                .map_err(|error| task_error(&error.to_string(), &error.to_string()))
        })?;
        to_python(py, &page)
    }

    fn cancel_task(&self, py: Python<'_>, id: &str) -> PyResult<Py<PyAny>> {
        let id: TaskId = id
            .parse()
            .map_err(|error: patinae_cmd::tasks::TaskLookupError| {
                task_error(&error.to_string(), &error.to_string())
            })?;
        let reply = self.call(py, false, move |owner| {
            owner
                .tasks
                .cancel(id)
                .map_err(|error| task_error(&error.to_string(), &error.to_string()))
        })?;
        to_python(py, &reply)
    }

    /// Wait without holding the GIL or blocking the session owner.
    #[pyo3(signature = (id, timeout=None))]
    fn wait_task(&self, py: Python<'_>, id: &str, timeout: Option<f64>) -> PyResult<Py<PyAny>> {
        let id: TaskId = id
            .parse()
            .map_err(|error: patinae_cmd::tasks::TaskLookupError| {
                task_error(&error.to_string(), &error.to_string())
            })?;
        let timeout = timeout
            .map(|seconds| {
                Duration::try_from_secs_f64(seconds).map_err(|_| {
                    pyo3::exceptions::PyValueError::new_err(
                        "timeout must be finite and nonnegative",
                    )
                })
            })
            .transpose()?;
        let parent = self.parent;
        let started = Instant::now();
        loop {
            let snapshot = self.call(py, false, move |owner| {
                if parent.is_some_and(|parent| {
                    owner
                        .tasks
                        .get(parent)
                        .is_ok_and(|snapshot| snapshot.cancel_requested)
                }) {
                    return Err(task_error("cancelled", "task cancelled"));
                }
                owner
                    .tasks
                    .validate_wait(parent, id, parent.map(|_| "python"))
                    .map_err(|error| task_error(&error.code, &error.message))?;
                owner
                    .tasks
                    .get(id)
                    .map_err(|error| task_error(&error.to_string(), &error.to_string()))
            })?;
            if snapshot.state.is_terminal() {
                return to_python(py, &snapshot);
            }
            if timeout.is_some_and(|timeout| started.elapsed() >= timeout) {
                return Err(task_error("timeout", &format!("waiting for task {id}")));
            }
            py.detach(|| std::thread::sleep(Duration::from_millis(10)));
            py.check_signals()?;
        }
    }

    fn is_interrupt_requested(&self, py: Python<'_>) -> PyResult<bool> {
        let parent = self.parent;
        self.call(py, false, move |owner| {
            Ok(parent.is_some_and(|id| {
                owner
                    .tasks
                    .get(id)
                    .is_ok_and(|snapshot| snapshot.cancel_requested)
            }))
        })
    }

    fn update_animations(&self, py: Python<'_>, dt: f32) -> PyResult<bool> {
        self.call(py, true, move |owner| {
            let update = owner.session.update_animations(dt);
            owner.needs_redraw |= update.needs_redraw;
            Ok(update.needs_redraw)
        })
    }

    fn get_movie_state(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let state = self.call(py, false, |owner| Ok(owner.session.movie_state_snapshot()))?;
        let dict = PyDict::new(py);
        dict.set_item("frame_count", state.frame_count)?;
        dict.set_item("current_frame", state.current_frame)?;
        dict.set_item("is_playing", state.is_playing)?;
        dict.set_item("rock_enabled", state.rock_enabled)?;
        Ok(dict.into_any().unbind())
    }

    fn get_model(&self, py: Python<'_>, name: &str) -> PyResult<PyObjectMolecule> {
        let name = name.to_string();
        self.call(py, false, move |owner| {
            owner
                .session
                .registry
                .get_molecule(&name)
                .map(|object| object.molecule().clone())
                .ok_or_else(|| PyKeyError::new_err(format!("Object '{name}' not found")))
        })
        .map(Into::into)
    }

    fn get_names(&self, py: Python<'_>) -> PyResult<Vec<String>> {
        self.call(py, false, |owner| {
            Ok(owner.session.registry.names().map(str::to_string).collect())
        })
    }

    fn get_label(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        let name = name.to_string();
        let label = self.call(py, false, move |owner| {
            label_object_view(
                &owner.session.registry,
                &owner.session.settings,
                &owner.session.named_palette,
                &name,
            )
            .ok_or_else(|| PyKeyError::new_err(format!("label object '{name}' not found")))
        })?;
        label_object_to_py(py, &label)
    }

    #[pyo3(signature = (selection="all"))]
    fn count_atoms(&self, py: Python<'_>, selection: &str) -> PyResult<usize> {
        Ok(self
            .molecule_snapshot(py, selection, false)?
            .1
            .iter()
            .map(|(_, _, indices)| indices.len())
            .sum())
    }

    /// Evaluate expressions on owned snapshots, allowing nested backend calls.
    #[pyo3(signature = (selection, expression, space=None))]
    fn iterate(
        &self,
        py: Python<'_>,
        selection: &str,
        expression: &str,
        space: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        let (_, molecules) = self.molecule_snapshot(py, selection, false)?;
        let globals = build_globals(py, space)?;
        let code = CString::new(expression)
            .map_err(|_| PyRuntimeError::new_err("Expression contains null byte"))?;
        for (name, molecule, indices) in molecules {
            let coords = molecule.current_coord_set();
            for index in indices {
                let atom_index = AtomIndex(index as u32);
                let atom = molecule
                    .get_atom(atom_index)
                    .ok_or_else(|| PyRuntimeError::new_err("invalid atom snapshot"))?;
                let coord = coords
                    .and_then(|coords| coords.get_atom_coord(atom_index))
                    .map(|coord| (coord.x, coord.y, coord.z));
                let locals = PyDict::new(py);
                set_atom_locals(&locals, atom, coord, index, &name)?;
                py.run(code.as_c_str(), Some(&globals), Some(&locals))?;
            }
        }
        Ok(())
    }

    /// Commit altered atom snapshots only while the source revision remains current.
    #[pyo3(signature = (selection, expression, space=None))]
    fn alter(
        &self,
        py: Python<'_>,
        selection: &str,
        expression: &str,
        space: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        let (revision, mut molecules) = self.molecule_snapshot(py, selection, true)?;
        let globals = build_globals(py, space)?;
        let code = CString::new(expression)
            .map_err(|_| PyRuntimeError::new_err("Expression contains null byte"))?;
        for (name, molecule, indices) in &mut molecules {
            for &index in indices.iter() {
                let atom_index = AtomIndex(index as u32);
                let coord = molecule
                    .current_coord_set()
                    .and_then(|coords| coords.get_atom_coord(atom_index))
                    .map(|coord| (coord.x, coord.y, coord.z));
                let locals = PyDict::new(py);
                let atom = molecule
                    .get_atom(atom_index)
                    .ok_or_else(|| PyRuntimeError::new_err("invalid atom snapshot"))?;
                set_atom_locals(&locals, atom, coord, index, name)?;
                py.run(code.as_c_str(), Some(&globals), Some(&locals))?;
                apply_locals_to_atom(
                    &locals,
                    molecule
                        .get_atom_mut(atom_index)
                        .ok_or_else(|| PyRuntimeError::new_err("invalid atom snapshot"))?,
                )?;
            }
        }
        self.call(py, true, move |owner| {
            if owner.revision != revision {
                return Err(PyRuntimeError::new_err(
                    "stale_context: scene changed during alter",
                ));
            }
            for (name, molecule, indices) in molecules {
                let object = owner
                    .session
                    .registry
                    .get_molecule_mut(&name)
                    .ok_or_else(|| PyRuntimeError::new_err("stale_context: object removed"))?;
                for index in indices {
                    let atom_index = AtomIndex(index as u32);
                    *object
                        .molecule_mut()
                        .get_atom_mut(atom_index)
                        .ok_or_else(|| PyRuntimeError::new_err("stale_context: atom removed"))? =
                        molecule
                            .get_atom(atom_index)
                            .ok_or_else(|| PyRuntimeError::new_err("invalid atom snapshot"))?
                            .clone();
                }
            }
            owner.needs_redraw = true;
            Ok(())
        })
    }

    #[cfg(feature = "numpy")]
    fn get_viewport_image(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        use numpy::{ndarray::Array3, IntoPyArray};
        let image = self.call(py, false, |owner| Ok(owner.session.viewport_image.clone()))?;
        image
            .map(|image| {
                let array = Array3::from_shape_vec(
                    (image.height as usize, image.width as usize, 4),
                    image.data,
                )
                .map_err(|error| PyRuntimeError::new_err(error.to_string()))?;
                Ok(array.into_pyarray(py).into_any().unbind())
            })
            .transpose()
    }

    #[cfg(feature = "numpy")]
    fn set_viewport_image(&self, py: Python<'_>, array: &Bound<'_, PyAny>) -> PyResult<()> {
        use numpy::{PyArray3, PyArrayMethods, PyUntypedArrayMethods};
        let array = array.cast::<PyArray3<u8>>().map_err(|_| {
            pyo3::exceptions::PyTypeError::new_err(
                "Expected numpy array with shape (H, W, 4) and dtype uint8",
            )
        })?;
        let shape = array.shape();
        if shape[2] != 4 {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "Expected shape (H, W, 4)",
            ));
        }
        let image = ViewportImage {
            width: u32::try_from(shape[1])
                .map_err(|_| pyo3::exceptions::PyValueError::new_err("image width is too large"))?,
            height: u32::try_from(shape[0]).map_err(|_| {
                pyo3::exceptions::PyValueError::new_err("image height is too large")
            })?,
            data: array.to_vec()?,
        };
        self.call(py, true, move |owner| {
            owner.session.viewport_image = Some(image);
            owner.needs_redraw = true;
            Ok(())
        })
    }

    fn clear_viewport_image(&self, py: Python<'_>) -> PyResult<()> {
        self.call(py, true, |owner| {
            owner.session.viewport_image = None;
            owner.needs_redraw = true;
            Ok(())
        })
    }

    /// Standalone sessions have no keyboard event source.
    fn set_key(&self, _key: &str, _callback: Py<PyAny>) -> PyResult<()> {
        Ok(())
    }
    /// Standalone sessions have no keyboard event source.
    fn unset_key(&self, _key: &str) -> PyResult<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn binding_transports_unknown_error_codes_without_interpreting_them() {
        Python::attach(|py| {
            let error = task_error("future_executor_error", "host detail");
            assert_eq!(
                error
                    .value(py)
                    .getattr("code")
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                "future_executor_error"
            );
            assert_eq!(
                error
                    .value(py)
                    .getattr("message")
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                "host detail"
            );
        });
    }

    use super::*;
    use patinae_scene::{AtomAnchor, LabelEntity, LabelObject};

    #[test]
    fn standalone_movie_state_and_explicit_animation_tick_are_deterministic() {
        Python::attach(|py| {
            let backend = StandaloneBackend::create().unwrap();
            backend
                .call(py, true, |owner| {
                    owner.session.settings.movie.movie_fps = 10.0;
                    Ok(())
                })
                .unwrap();
            backend.execute(py, "mset 1 x3", true).unwrap();
            backend.execute(py, "mplay", true).unwrap();
            assert!(backend.update_animations(py, 0.11).unwrap());
            let state = backend.get_movie_state(py).unwrap();
            let state = state.bind(py).cast::<PyDict>().unwrap();
            assert_eq!(
                state
                    .get_item("frame_count")
                    .unwrap()
                    .unwrap()
                    .extract::<usize>()
                    .unwrap(),
                3
            );
            assert_eq!(
                state
                    .get_item("current_frame")
                    .unwrap()
                    .unwrap()
                    .extract::<usize>()
                    .unwrap(),
                1
            );
            assert!(state
                .get_item("is_playing")
                .unwrap()
                .unwrap()
                .extract::<bool>()
                .unwrap());
        });
    }

    #[test]
    fn standalone_label_snapshot_preserves_entities_and_resolution_state() {
        Python::attach(|py| {
            let backend = StandaloneBackend::create().unwrap();
            backend
                .call(py, true, |owner| {
                    owner.session.registry.add(LabelObject::with_entities(
                        "notes",
                        vec![LabelEntity::new(
                            AtomAnchor::new("missing", AtomIndex(3)),
                            "orphan candidate",
                        )],
                    ));
                    Ok(())
                })
                .unwrap();
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
        });
    }

    #[test]
    fn owner_applies_script_without_caller_polling_and_keeps_result() {
        Python::attach(|py| {
            let backend = StandaloneBackend::create().unwrap();
            let path =
                std::env::temp_dir().join(format!("patinae-standalone-{}.pml", std::process::id()));
            std::fs::write(&path, "mset 1 x4\nmplay\n").unwrap();
            let receipt = backend
                .execute(py, &format!("run {}", path.display()), true)
                .unwrap();
            let ids: Vec<String> = receipt
                .bind(py)
                .get_item("task_ids")
                .unwrap()
                .extract()
                .unwrap();
            assert_eq!(ids.len(), 1);
            // A detached idle caller makes no request while the owner advances.
            py.detach(|| std::thread::sleep(Duration::from_millis(100)));
            let snapshot = backend.get_task(py, &ids[0]).unwrap();
            assert_eq!(
                snapshot
                    .bind(py)
                    .get_item("state")
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                "succeeded"
            );
            let repeated = backend.wait_task(py, &ids[0], Some(0.0)).unwrap();
            assert_eq!(
                repeated
                    .bind(py)
                    .get_item("state")
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                "succeeded"
            );
            std::fs::remove_file(path).unwrap();
        });
    }

    #[test]
    fn waiting_timeout_does_not_cancel_and_self_wait_is_rejected() {
        Python::attach(|py| {
            let backend = StandaloneBackend::create().unwrap();
            let id = backend
                .call(py, false, |owner| {
                    Ok(owner
                        .tasks
                        .admit(patinae_cmd::tasks::TaskSpec::new("test", "python"))
                        .unwrap())
                })
                .unwrap();
            let task_backend = StandaloneBackend::for_task(Arc::clone(&backend.client), id);
            let error = task_backend
                .wait_task(py, &id.to_string(), Some(0.0))
                .unwrap_err();
            assert!(error.to_string().contains("would_deadlock"));
            let error = backend
                .wait_task(py, &id.to_string(), Some(0.0))
                .unwrap_err();
            assert_eq!(
                error
                    .value(py)
                    .getattr("code")
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                "timeout"
            );
            let snapshot = backend.get_task(py, &id.to_string()).unwrap();
            assert!(!snapshot
                .bind(py)
                .get_item("cancel_requested")
                .unwrap()
                .extract::<bool>()
                .unwrap());
            backend
                .call(py, false, move |owner| {
                    owner.tasks.finish(
                        id,
                        patinae_cmd::tasks::TaskOutcome::success(
                            None,
                            patinae_cmd::tasks::TaskEffects::None,
                        ),
                    );
                    Ok(())
                })
                .unwrap();
            let snapshot = backend.wait_task(py, &id.to_string(), Some(0.0)).unwrap();
            assert_eq!(
                snapshot
                    .bind(py)
                    .get_item("state")
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                "succeeded"
            );
        });
    }

    #[test]
    fn url_results_apply_without_manual_polling_and_cancel_acknowledges_stop() {
        use std::io::{Read, Write};
        fn start_server(
            delay: Duration,
        ) -> (String, mpsc::Receiver<()>, std::thread::JoinHandle<()>) {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/fixture.xyz", listener.local_addr().unwrap());
            let (accepted_tx, accepted_rx) = mpsc::sync_channel(1);
            let worker = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0u8; 4096];
                let _ = stream.read(&mut request);
                let _ = accepted_tx.send(());
                std::thread::sleep(delay);
                let body = "1\nfixture\nC 0 0 0\n";
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
            });
            (url, accepted_rx, worker)
        }
        Python::attach(|py| {
            let backend = StandaloneBackend::create().unwrap();
            let (url, _accepted, server) = start_server(Duration::from_millis(10));
            let reply = backend
                .execute(py, &format!("load {url}, loaded"), true)
                .unwrap();
            let ids: Vec<String> = reply
                .bind(py)
                .get_item("task_ids")
                .unwrap()
                .extract()
                .unwrap();
            assert_eq!(ids.len(), 1);
            py.detach(|| {
                server.join().unwrap();
                std::thread::sleep(Duration::from_millis(100));
            });
            assert_eq!(backend.count_atoms(py, "all").unwrap(), 1);
            backend.execute(py, "create copied, loaded", true).unwrap();
            assert_eq!(backend.count_atoms(py, "all").unwrap(), 2);
            assert_eq!(backend.count_atoms(py, "loaded").unwrap(), 1);
            assert_eq!(backend.count_atoms(py, "copied").unwrap(), 1);
            assert_eq!(
                backend
                    .get_task(py, &ids[0])
                    .unwrap()
                    .bind(py)
                    .get_item("state")
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                "succeeded"
            );

            let (url, accepted, server) = start_server(Duration::from_millis(250));
            let reply = backend
                .execute(py, &format!("load {url}, cancelled"), true)
                .unwrap();
            let ids: Vec<String> = reply
                .bind(py)
                .get_item("task_ids")
                .unwrap()
                .extract()
                .unwrap();
            py.detach(move || accepted.recv_timeout(Duration::from_secs(2)).unwrap());
            backend.cancel_task(py, &ids[0]).unwrap();
            let snapshot = backend.wait_task(py, &ids[0], Some(2.0)).unwrap();
            assert_eq!(
                snapshot
                    .bind(py)
                    .get_item("state")
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                "cancelled"
            );
            assert!(!backend
                .get_names(py)
                .unwrap()
                .contains(&"cancelled".to_string()));
            py.detach(|| server.join().unwrap());
        });
    }
}
