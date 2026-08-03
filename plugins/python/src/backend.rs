//! Plugin Backend for Embedded Mode
//!
//! Provides `PluginBackend` — a Python-visible object that the `patinae`
//! package discovers via `sys._patinae_backend` when running inside the GUI.
//!
//! Unlike `StandaloneBackend` (which owns a Session), this backend
//! communicates with the host through shared snapshots and a command queue.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use numpy::ndarray::Array3;
use numpy::{IntoPyArray, PyArray3, PyArrayMethods, PyUntypedArrayMethods};
use patinae_plugin::prelude::{
    parse_key_string, AtomColumn, AtomRow, AtomStreamMode, AtomStreamRequest, AtomStreamScope,
    AtomValue,
};
use patinae_plugin::wire::{WireAtomPropertyChange, WireAtomPropertyValue};
use patinae_scene::{LabelEntityView, LabelObjectView};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use crate::atom_ops::{build_globals, expression_to_cstring};
use crate::shared::{HostBridgeHandle, HostBridgeRequestKind, HostBridgeValue, SharedStateHandle};

const PYTHON_ATOM_STREAM_CHUNK_ROWS: usize = 4096;

/// Python-visible backend for embedded (plugin) mode.
///
/// Injected into `sys._patinae_backend` so that `patinae.__init__`
/// auto-detects embedded mode and wraps this in a `Cmd` object.
#[pyclass(name = "PluginBackend")]
pub struct PluginBackend {
    shared: SharedStateHandle,
    host_bridge: HostBridgeHandle,
    interrupt_requested: Arc<AtomicBool>,
}

impl PluginBackend {
    pub fn new(shared: SharedStateHandle) -> Self {
        let (interrupt_requested, host_bridge) = {
            let state = shared.lock().unwrap();
            (state.interrupt_requested.clone(), state.host_bridge.clone())
        };
        Self {
            shared,
            host_bridge,
            interrupt_requested,
        }
    }

    fn check_interrupt(&self) -> PyResult<()> {
        if self.interrupt_requested.load(Ordering::Acquire) {
            Err(pyo3::exceptions::PyRuntimeError::new_err(
                "Python script interrupted",
            ))
        } else {
            Ok(())
        }
    }

    fn host_request(&self, kind: HostBridgeRequestKind) -> PyResult<HostBridgeValue> {
        self.host_bridge
            .request(kind, &self.interrupt_requested)
            .map_err(pyo3::exceptions::PyRuntimeError::new_err)
    }

    fn open_atom_stream(&self, selection: &str, mode: AtomStreamMode) -> PyResult<u64> {
        let request = AtomStreamRequest {
            scope: AtomStreamScope::Selection(selection.to_string()),
            mode,
            columns: AtomColumn::pymol_locals(),
            chunk_size: PYTHON_ATOM_STREAM_CHUNK_ROWS,
        };
        match self.host_request(HostBridgeRequestKind::OpenAtomStream { request })? {
            HostBridgeValue::AtomStreamOpened { stream_id, .. } => Ok(stream_id),
            _ => Err(unexpected_host_result_error()),
        }
    }

    fn read_atom_stream(&self, stream_id: u64) -> PyResult<patinae_plugin::prelude::AtomChunk> {
        match self.host_request(HostBridgeRequestKind::ReadAtomStream {
            stream_id,
            max_rows: PYTHON_ATOM_STREAM_CHUNK_ROWS,
        })? {
            HostBridgeValue::AtomChunk(chunk) => Ok(chunk),
            _ => Err(unexpected_host_result_error()),
        }
    }

    fn close_atom_stream(&self, stream_id: u64) -> PyResult<()> {
        match self.host_request(HostBridgeRequestKind::CloseAtomStream { stream_id })? {
            HostBridgeValue::Unit => Ok(()),
            _ => Err(unexpected_host_result_error()),
        }
    }

    fn apply_atom_changes(&self, changes: Vec<WireAtomPropertyChange>) -> PyResult<()> {
        if changes.is_empty() {
            return Ok(());
        }
        match self.host_request(HostBridgeRequestKind::ApplyAtomPropertyChanges { changes })? {
            HostBridgeValue::Unit => Ok(()),
            _ => Err(unexpected_host_result_error()),
        }
    }

    fn run_iterate_stream(
        &self,
        py: Python<'_>,
        stream_id: u64,
        code: &std::ffi::CStr,
        globals: &Bound<'_, PyDict>,
    ) -> PyResult<()> {
        loop {
            self.check_interrupt()?;
            let chunk = self.read_atom_stream(stream_id)?;
            for row in &chunk.rows {
                self.check_interrupt()?;
                let locals = PyDict::new(py);
                set_row_locals(&locals, row)?;
                py.run(code, Some(globals), Some(&locals))?;
            }
            if chunk.done {
                return Ok(());
            }
        }
    }

    fn run_alter_stream(
        &self,
        py: Python<'_>,
        stream_id: u64,
        code: &std::ffi::CStr,
        globals: &Bound<'_, PyDict>,
    ) -> PyResult<()> {
        loop {
            self.check_interrupt()?;
            let chunk = self.read_atom_stream(stream_id)?;
            let mut changes = Vec::new();
            for row in &chunk.rows {
                self.check_interrupt()?;
                let locals = PyDict::new(py);
                set_row_locals(&locals, row)?;
                py.run(code, Some(globals), Some(&locals))?;
                let diff = diff_row_locals(&locals, row)?;
                if !diff.is_empty() {
                    changes.push(WireAtomPropertyChange {
                        object: row.key.object.clone(),
                        atom_index: row.key.atom_index,
                        changes: diff,
                    });
                }
            }
            self.apply_atom_changes(changes)?;
            if chunk.done {
                return Ok(());
            }
        }
    }
}

fn molecule_snapshot_unavailable_error() -> PyErr {
    pyo3::exceptions::PyRuntimeError::new_err(
        "Python molecule snapshot is unavailable in lightweight plugin mode; molecule-heavy APIs require a full scene snapshot and may be unavailable when the serialized scene exceeds the 64 MiB plugin ABI limit",
    )
}

fn unexpected_host_result_error() -> PyErr {
    pyo3::exceptions::PyRuntimeError::new_err("host returned an unexpected Python bridge result")
}

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

fn set_row_locals(locals: &Bound<'_, PyDict>, row: &AtomRow) -> PyResult<()> {
    for (column, value) in &row.values {
        if !matches!(value, AtomValue::None) {
            set_row_local(locals, *column, value)?;
        }
    }
    Ok(())
}

fn set_row_local(
    locals: &Bound<'_, PyDict>,
    column: AtomColumn,
    value: &AtomValue,
) -> PyResult<()> {
    match value {
        AtomValue::None => {}
        AtomValue::Str(value) => locals.set_item(column.local_name(), value)?,
        AtomValue::F32(value) => locals.set_item(column.local_name(), *value)?,
        AtomValue::I32(value) => locals.set_item(column.local_name(), *value)?,
        AtomValue::I8(value) => locals.set_item(column.local_name(), *value)?,
        AtomValue::Bool(value) => locals.set_item(column.local_name(), *value)?,
    }
    Ok(())
}

fn diff_row_locals(
    locals: &Bound<'_, PyDict>,
    row: &AtomRow,
) -> PyResult<Vec<(String, WireAtomPropertyValue)>> {
    let mut changes = Vec::new();
    for (column, original) in &row.values {
        let Some(key) = mutable_wire_key(*column) else {
            continue;
        };
        let Some(value) = locals.get_item(column.local_name())? else {
            continue;
        };
        if let Some(change) = changed_wire_value(&value, original)? {
            changes.push((key.to_string(), change));
        }
    }
    Ok(changes)
}

fn mutable_wire_key(column: AtomColumn) -> Option<&'static str> {
    match column {
        AtomColumn::Name => Some("name"),
        AtomColumn::Resn => Some("resn"),
        AtomColumn::Resv => Some("resv"),
        AtomColumn::Resi => Some("resv"),
        AtomColumn::Chain => Some("chain"),
        AtomColumn::Segi => Some("segi"),
        AtomColumn::Alt => Some("alt"),
        AtomColumn::Elem => Some("elem"),
        AtomColumn::B => Some("b"),
        AtomColumn::Q => Some("q"),
        AtomColumn::Vdw => Some("vdw"),
        AtomColumn::PartialCharge => Some("partial_charge"),
        AtomColumn::FormalCharge => Some("formal_charge"),
        AtomColumn::Ss => Some("ss"),
        AtomColumn::Color => Some("color"),
        AtomColumn::Type => Some("type"),
        AtomColumn::Hetatm
        | AtomColumn::Index
        | AtomColumn::Id
        | AtomColumn::Rank
        | AtomColumn::Model
        | AtomColumn::X
        | AtomColumn::Y
        | AtomColumn::Z => None,
    }
}

fn changed_wire_value(
    value: &Bound<'_, PyAny>,
    original: &AtomValue,
) -> PyResult<Option<WireAtomPropertyValue>> {
    match original {
        AtomValue::None => Ok(None),
        AtomValue::Str(original) => {
            let current: String = value.extract()?;
            Ok((current != *original).then_some(WireAtomPropertyValue::Str(current)))
        }
        AtomValue::F32(original) => {
            let current: f32 = value.extract()?;
            Ok((current != *original).then_some(WireAtomPropertyValue::F32(current)))
        }
        AtomValue::I32(original) => {
            let current: i32 = value.extract()?;
            Ok((current != *original).then_some(WireAtomPropertyValue::I32(current)))
        }
        AtomValue::I8(original) => {
            let current: i8 = value.extract()?;
            Ok((current != *original).then_some(WireAtomPropertyValue::I8(current)))
        }
        AtomValue::Bool(original) => {
            let current: bool = value.extract()?;
            Ok((current != *original).then_some(WireAtomPropertyValue::Bool(current)))
        }
    }
}

#[pymethods]
impl PluginBackend {
    /// Queue a command for execution by the host.
    ///
    /// The command is not executed immediately — it is drained during
    /// the next `poll()` cycle by the message handler.
    #[pyo3(signature = (command, silent=false))]
    fn execute(&self, command: &str, silent: bool) -> PyResult<()> {
        let mut state = self.shared.lock().unwrap();
        state.cmd_queue.push((command.to_string(), silent));
        Ok(())
    }

    /// Return whether the host has requested cooperative cancellation.
    fn is_interrupt_requested(&self) -> bool {
        self.interrupt_requested.load(Ordering::Acquire)
    }

    /// Get the latest host movie state snapshot as a Python dict.
    fn get_movie_state(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let state = self.shared.lock().unwrap();
        let movie = state.movie_state.clone();
        drop(state);

        let dict = PyDict::new(py);
        dict.set_item("frame_count", movie.frame_count)?;
        dict.set_item("current_frame", movie.current_frame)?;
        dict.set_item("is_playing", movie.is_playing)?;
        dict.set_item("rock_enabled", movie.rock_enabled)?;
        Ok(dict.into_any().unbind())
    }

    /// Embedded mode is ticked by the host render loop; explicit ticks are a no-op.
    fn update_animations(&self, _dt: f32) -> bool {
        false
    }

    /// Get a molecular object by name from the snapshot.
    fn get_model(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        let state = self.shared.lock().unwrap();
        if state.molecules.is_empty() && state.names.iter().any(|n| n == name) {
            return Err(molecule_snapshot_unavailable_error());
        }
        for (n, mol) in &state.molecules {
            if n == name {
                // Return a basic dict with molecular data.
                // The patinae package can wrap it if needed.
                let dict = pyo3::types::PyDict::new(py);
                dict.set_item("name", &mol.name)?;
                dict.set_item("atom_count", mol.atom_count())?;
                dict.set_item("bond_count", mol.bond_count())?;
                dict.set_item("state_count", mol.state_count())?;
                return Ok(dict.into_any().unbind());
            }
        }
        Err(pyo3::exceptions::PyKeyError::new_err(format!(
            "object '{}' not found",
            name
        )))
    }

    /// Get list of loaded object names.
    fn get_names(&self) -> Vec<String> {
        let state = self.shared.lock().unwrap();
        state.names.clone()
    }

    /// Get a read-only label object snapshot.
    fn get_label(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        match self.host_request(HostBridgeRequestKind::LabelObject {
            name: name.to_string(),
        })? {
            HostBridgeValue::LabelObject(Some(label)) => label_object_to_py(py, &label),
            HostBridgeValue::LabelObject(None) => Err(pyo3::exceptions::PyKeyError::new_err(
                format!("label object '{}' not found", name),
            )),
            _ => Err(unexpected_host_result_error()),
        }
    }

    /// Count atoms matching a selection across all molecules.
    fn count_atoms(&self, selection: &str) -> PyResult<usize> {
        match self.host_request(HostBridgeRequestKind::CountAtoms {
            selection: selection.to_string(),
        })? {
            HostBridgeValue::CountAtoms(count) => Ok(count),
            _ => Err(unexpected_host_result_error()),
        }
    }

    /// Execute a Python expression for each atom matching a selection (read-only).
    ///
    /// Operates on host-owned atom stream chunks.
    #[pyo3(signature = (selection, expression, space=None))]
    fn iterate(
        &self,
        py: Python<'_>,
        selection: &str,
        expression: &str,
        space: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        let stream_id = self.open_atom_stream(selection, AtomStreamMode::Read)?;
        let globals = build_globals(py, space)?;
        let code = expression_to_cstring(expression)?;
        let result = self.run_iterate_stream(py, stream_id, code.as_c_str(), &globals);
        if result.is_err() {
            let _ = self.close_atom_stream(stream_id);
        }
        result
    }

    /// Execute a Python expression for each atom matching a selection,
    /// allowing modification of atom properties.
    ///
    /// Changes are diffed per streamed chunk and sent back to the host.
    #[pyo3(signature = (selection, expression, space=None))]
    fn alter(
        &self,
        py: Python<'_>,
        selection: &str,
        expression: &str,
        space: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        let stream_id = self.open_atom_stream(selection, AtomStreamMode::Alter)?;
        let globals = build_globals(py, space)?;
        let code = expression_to_cstring(expression)?;
        let result = self.run_alter_stream(py, stream_id, code.as_c_str(), &globals);
        if result.is_err() {
            let _ = self.close_atom_stream(stream_id);
        }
        result
    }

    /// Get the current viewport image as a numpy array (H, W, 4) uint8.
    ///
    /// Returns `None` if no viewport image is set.
    /// The image is a snapshot from the last poll cycle.
    fn get_viewport_image<'py>(&self, py: Python<'py>) -> PyResult<Option<Py<PyAny>>> {
        let state = self.shared.lock().unwrap();
        match &state.viewport_image {
            Some((data, width, height)) => {
                let h = *height as usize;
                let w = *width as usize;
                let arr = Array3::from_shape_vec((h, w, 4), data.clone()).map_err(|e| {
                    pyo3::exceptions::PyRuntimeError::new_err(format!(
                        "Failed to create array: {}",
                        e
                    ))
                })?;
                Ok(Some(arr.into_pyarray(py).into_any().unbind()))
            }
            None => Ok(None),
        }
    }

    /// Set the viewport image from a numpy array (H, W, 4) uint8.
    ///
    /// The image is queued and applied on the next poll cycle.
    fn set_viewport_image(&self, array: &Bound<'_, PyAny>) -> PyResult<()> {
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
        let mut state = self.shared.lock().unwrap();
        state.set_image_queue = Some(Some((data, width, height)));
        Ok(())
    }

    /// Clear the viewport image overlay.
    ///
    /// The clear is queued and applied on the next poll cycle.
    fn clear_viewport_image(&self) -> PyResult<()> {
        let mut state = self.shared.lock().unwrap();
        state.set_image_queue = Some(None);
        Ok(())
    }

    /// Bind a key combination to a Python callable.
    ///
    /// The key string uses the format `[modifier+]*key`, e.g.
    /// `"F1"`, `"ctrl+s"`, `"ctrl+shift+r"`.
    /// The callback is invoked with no arguments when the key is pressed.
    /// Rebinding the same key replaces the previous callback.
    fn set_key(&self, key: &str, callback: Py<PyAny>) -> PyResult<()> {
        // Validate key string early so the user gets immediate feedback
        let normalized = key.trim().to_lowercase();
        parse_key_string(&normalized).map_err(|e| {
            pyo3::exceptions::PyValueError::new_err(format!("Invalid key string: {}", e))
        })?;

        let mut state = self.shared.lock().unwrap();
        let kb = &mut state.keybinds;

        // If this key was already bound, remove the old callback
        if let Some(old_id) = kb.key_to_id.remove(&normalized) {
            kb.callbacks.remove(&old_id);
        }

        let id = kb.next_id;
        kb.next_id += 1;

        kb.callbacks.insert(id, callback);
        kb.key_to_id.insert(normalized.clone(), id);
        kb.requests.push((id, normalized.clone()));

        Ok(())
    }

    /// Unbind a key combination.
    ///
    /// Silently succeeds if the key was not bound.
    fn unset_key(&self, key: &str) -> PyResult<()> {
        let normalized = key.trim().to_lowercase();

        let mut state = self.shared.lock().unwrap();
        let kb = &mut state.keybinds;

        if let Some(old_id) = kb.key_to_id.remove(&normalized) {
            kb.callbacks.remove(&old_id);
        }

        kb.unreg_requests.push(normalized);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use patinae_plugin::prelude::AtomRowKey;

    fn test_row() -> AtomRow {
        AtomRow {
            key: AtomRowKey {
                object: "obj".to_string(),
                atom_index: 0,
            },
            values: vec![
                (AtomColumn::Name, AtomValue::Str("CA".to_string())),
                (AtomColumn::B, AtomValue::F32(1.0)),
                (AtomColumn::Resv, AtomValue::I32(1)),
                (AtomColumn::Resi, AtomValue::I32(1)),
                (AtomColumn::Index, AtomValue::I32(1)),
            ],
        }
    }

    #[test]
    fn row_local_diff_captures_mutable_changes() {
        Python::attach(|py| {
            let locals = PyDict::new(py);
            let row = test_row();
            set_row_locals(&locals, &row).unwrap();
            locals.set_item("name", "CB").unwrap();
            locals.set_item("b", 2.5_f32).unwrap();
            locals.set_item("resi", 7_i32).unwrap();
            locals.set_item("index", 99_i32).unwrap();

            let changes = diff_row_locals(&locals, &row).unwrap();

            assert_eq!(changes.len(), 3);
            assert!(matches!(
                find_change(&changes, "name"),
                WireAtomPropertyValue::Str(value) if value == "CB"
            ));
            assert!(matches!(
                find_change(&changes, "b"),
                WireAtomPropertyValue::F32(value) if (*value - 2.5).abs() < f32::EPSILON
            ));
            assert!(matches!(
                find_change(&changes, "resv"),
                WireAtomPropertyValue::I32(7)
            ));
        });
    }

    fn find_change<'a>(
        changes: &'a [(String, WireAtomPropertyValue)],
        key: &str,
    ) -> &'a WireAtomPropertyValue {
        changes
            .iter()
            .find_map(|(candidate, value)| (candidate == key).then_some(value))
            .unwrap_or_else(|| panic!("missing change {key}"))
    }
}
