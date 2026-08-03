//! Python Message Handler
//!
//! Implements `MessageHandler` with `needs_poll() = true` for:
//! 1. Draining results from the Python worker thread
//! 2. Setting notification overlay while Python is busy
//! 3. Updating shared molecule/name snapshots from `SharedContext`
//! 4. Installing `sys._patinae_backend` on first poll (via worker)
//! 5. Draining the command queue → `ctx.execute_command()`
//! 6. Syncing viewport image and atom streams between Python and the host

use std::collections::HashMap;
use std::sync::mpsc::Receiver;

use std::path::{Path, PathBuf};

use patinae_plugin::prelude::*;

use patinae_plugin::wire::{
    WireAtomPropertyChange, WireAtomPropertyValue, WireHostQuery, WireHostQueryValue,
    WireViewerAction,
};
use patinae_scene::ViewportImage;

use crate::atom_ops::{AtomChange, PropertyValue};
use crate::panel::ScriptPanelStateHandle;
use crate::shared::{
    HostBridgeHandle, HostBridgeRequest, HostBridgeRequestKind, HostBridgeValue, SharedStateHandle,
};
use crate::worker::{WorkItem, WorkOrigin, WorkResult, WorkResultPayload, WorkerHandle};

use pyo3::prelude::*;

const PYTHON_KEYBIND_TRIGGER_TOPIC: &str = "python.keybind.trigger";

/// Message handler for the Python plugin.
///
/// Polls each frame to drain worker results, synchronize state,
/// and process queued commands.
pub struct PythonHandler {
    worker: WorkerHandle,
    shared: SharedStateHandle,
    result_rx: Receiver<WorkResult>,
    panel_state: ScriptPanelStateHandle,
    backend_requested: bool,
    next_cmd_id: u64,
    next_query_id: u64,
    pending_viewport_image_query: Option<(u64, u64)>,
    pending_bridge_queries: HashMap<u64, PendingBridgeQuery>,
    last_busy: bool,
}

#[derive(Debug, Clone, Copy)]
enum PendingBridgeQuery {
    CountAtoms { bridge_id: u64 },
    LabelObject { bridge_id: u64 },
    OpenAtomStream { bridge_id: u64 },
    ReadAtomStream { bridge_id: u64 },
    CloseAtomStream { bridge_id: u64 },
}

impl PythonHandler {
    pub fn new(
        worker: WorkerHandle,
        shared: SharedStateHandle,
        result_rx: Receiver<WorkResult>,
        panel_state: ScriptPanelStateHandle,
    ) -> Self {
        Self {
            worker,
            shared,
            result_rx,
            panel_state,
            backend_requested: false,
            next_cmd_id: 1,
            next_query_id: 1,
            pending_viewport_image_query: None,
            pending_bridge_queries: HashMap::new(),
            last_busy: false,
        }
    }

    fn host_bridge(&self) -> HostBridgeHandle {
        self.shared.lock().unwrap().host_bridge.clone()
    }

    /// Drain results from the Python worker thread and route by origin.
    fn drain_python_results(&mut self, ctx: &mut PollContext<'_>) {
        while let Ok(result) = self.result_rx.try_recv() {
            let WorkResult { origin, payload } = result;
            match origin {
                WorkOrigin::Command | WorkOrigin::Script => match payload {
                    WorkResultPayload::Output(output) if !output.is_empty() => {
                        let trimmed = output.trim_end_matches('\n');
                        if !trimmed.is_empty() {
                            ctx.bus.print_info(trimmed);
                        }
                    }
                    WorkResultPayload::Finished(Err(err)) => {
                        ctx.bus.print_error(err);
                    }
                    _ => {}
                },
                WorkOrigin::Setup => match payload {
                    WorkResultPayload::Setup(Ok(msg)) => log::info!("Python plugin: {}", msg),
                    WorkResultPayload::Setup(Err(e)) => {
                        log::warn!("Python plugin: setup failed: {}", e);
                    }
                    WorkResultPayload::Finished(Err(e)) => {
                        log::warn!("Python plugin: setup failed: {}", e);
                    }
                    _ => {}
                },
                WorkOrigin::Panel => {
                    let changed = {
                        let mut panel = self.panel_state.lock().unwrap();
                        match payload {
                            WorkResultPayload::Output(output) if !output.is_empty() => {
                                panel.append_output(&output);
                                true
                            }
                            WorkResultPayload::Finished(Ok(())) => {
                                if let Some(duration) = panel.finish_run() {
                                    ctx.bus.print_info("Python script finished.");
                                    ctx.bus.print_timing(duration);
                                }
                                false
                            }
                            WorkResultPayload::Finished(Err(err)) => {
                                panel.append_error(&err);
                                if let Some(duration) = panel.finish_run() {
                                    ctx.bus.print_error("Python script failed.");
                                    ctx.bus.print_timing(duration);
                                }
                                true
                            }
                            _ => false,
                        }
                    };
                    if changed {
                        ctx.request_panel_update();
                    }
                }
            }
        }
    }

    /// Drain portable host query results.
    fn drain_host_query_results(&mut self, ctx: &PollContext<'_>) {
        if let Some((pending_id, requested_signature)) = self.pending_viewport_image_query {
            if let Some(result) = ctx
                .host_query_results
                .iter()
                .find(|result| result.id == pending_id)
            {
                self.pending_viewport_image_query = None;
                let mut state = self.shared.lock().unwrap();
                match &result.result {
                    Ok(WireHostQueryValue::ViewportImage(image)) => {
                        state.viewport_image = image
                            .as_ref()
                            .map(|image| (image.data.clone(), image.width, image.height));
                        state.viewport_image_signature = Some(requested_signature);
                    }
                    Err(error) => {
                        log::warn!("Python plugin: viewport image query failed: {}", error);
                    }
                    _ => {}
                }
            }
        }

        let bridge = self.host_bridge();
        for result in ctx.host_query_results {
            let Some(pending) = self.pending_bridge_queries.remove(&result.id) else {
                continue;
            };
            let (bridge_id, value) = match (pending, &result.result) {
                (
                    PendingBridgeQuery::CountAtoms { bridge_id },
                    Ok(WireHostQueryValue::CountAtoms(count)),
                ) => (bridge_id, Ok(HostBridgeValue::CountAtoms(*count))),
                (
                    PendingBridgeQuery::LabelObject { bridge_id },
                    Ok(WireHostQueryValue::LabelObject(label)),
                ) => (bridge_id, Ok(HostBridgeValue::LabelObject(label.clone()))),
                (
                    PendingBridgeQuery::OpenAtomStream { bridge_id },
                    Ok(WireHostQueryValue::AtomStreamOpened(opened)),
                ) => (
                    bridge_id,
                    Ok(HostBridgeValue::AtomStreamOpened {
                        stream_id: opened.stream_id,
                        total_count: opened.total_count,
                    }),
                ),
                (
                    PendingBridgeQuery::ReadAtomStream { bridge_id },
                    Ok(WireHostQueryValue::AtomStreamChunk(chunk)),
                ) => (bridge_id, Ok(HostBridgeValue::AtomChunk(chunk.clone()))),
                (
                    PendingBridgeQuery::CloseAtomStream { bridge_id },
                    Ok(WireHostQueryValue::AtomStreamClosed),
                ) => (bridge_id, Ok(HostBridgeValue::Unit)),
                (PendingBridgeQuery::CountAtoms { bridge_id }, Err(error))
                | (PendingBridgeQuery::LabelObject { bridge_id }, Err(error))
                | (PendingBridgeQuery::OpenAtomStream { bridge_id }, Err(error))
                | (PendingBridgeQuery::ReadAtomStream { bridge_id }, Err(error))
                | (PendingBridgeQuery::CloseAtomStream { bridge_id }, Err(error)) => {
                    (bridge_id, Err(error.clone()))
                }
                (PendingBridgeQuery::CountAtoms { bridge_id }, _)
                | (PendingBridgeQuery::LabelObject { bridge_id }, _)
                | (PendingBridgeQuery::OpenAtomStream { bridge_id }, _)
                | (PendingBridgeQuery::ReadAtomStream { bridge_id }, _)
                | (PendingBridgeQuery::CloseAtomStream { bridge_id }, _) => (
                    bridge_id,
                    Err("host returned an unexpected Python bridge result".to_string()),
                ),
            };
            bridge.complete(bridge_id, value);
        }
    }

    fn drain_host_bridge_requests(&mut self, ctx: &mut PollContext<'_>) {
        let bridge = self.host_bridge();
        for request in bridge.take_requests() {
            self.submit_host_bridge_request(ctx, &bridge, request);
        }
    }

    fn submit_host_bridge_request(
        &mut self,
        ctx: &mut PollContext<'_>,
        bridge: &HostBridgeHandle,
        request: HostBridgeRequest,
    ) {
        match request.kind {
            HostBridgeRequestKind::CountAtoms { selection } => {
                let wire_id = self.next_query_id;
                self.next_query_id += 1;
                self.pending_bridge_queries.insert(
                    wire_id,
                    PendingBridgeQuery::CountAtoms {
                        bridge_id: request.id,
                    },
                );
                ctx.query_host(WireHostQuery::CountAtoms {
                    id: wire_id,
                    selection,
                });
            }
            HostBridgeRequestKind::LabelObject { name } => {
                let wire_id = self.next_query_id;
                self.next_query_id += 1;
                self.pending_bridge_queries.insert(
                    wire_id,
                    PendingBridgeQuery::LabelObject {
                        bridge_id: request.id,
                    },
                );
                ctx.query_host(WireHostQuery::LabelObject { id: wire_id, name });
            }
            HostBridgeRequestKind::OpenAtomStream {
                request: stream_request,
            } => {
                let wire_id = self.next_query_id;
                self.next_query_id += 1;
                self.pending_bridge_queries.insert(
                    wire_id,
                    PendingBridgeQuery::OpenAtomStream {
                        bridge_id: request.id,
                    },
                );
                ctx.query_host(WireHostQuery::OpenAtomStream {
                    id: wire_id,
                    request: stream_request,
                });
            }
            HostBridgeRequestKind::ReadAtomStream {
                stream_id,
                max_rows,
            } => {
                let wire_id = self.next_query_id;
                self.next_query_id += 1;
                self.pending_bridge_queries.insert(
                    wire_id,
                    PendingBridgeQuery::ReadAtomStream {
                        bridge_id: request.id,
                    },
                );
                ctx.query_host(WireHostQuery::ReadAtomStream {
                    id: wire_id,
                    stream_id,
                    max_rows,
                });
            }
            HostBridgeRequestKind::CloseAtomStream { stream_id } => {
                let wire_id = self.next_query_id;
                self.next_query_id += 1;
                self.pending_bridge_queries.insert(
                    wire_id,
                    PendingBridgeQuery::CloseAtomStream {
                        bridge_id: request.id,
                    },
                );
                ctx.query_host(WireHostQuery::CloseAtomStream {
                    id: wire_id,
                    stream_id,
                });
            }
            HostBridgeRequestKind::ApplyAtomPropertyChanges { changes } => {
                ctx.queue_viewer_action(WireViewerAction::ApplyAtomPropertyChanges(changes));
                bridge.complete(request.id, Ok(HostBridgeValue::Unit));
            }
        }
    }

    /// Sync cheap host state from portable poll state.
    fn update_snapshots(&mut self, ctx: &mut PollContext<'_>) {
        let mut state = self.shared.lock().unwrap();

        // Update names
        state.names = ctx.poll_shared.object_names.clone();

        // Request viewport image bytes only when the lightweight identity changes.
        let image_signature = ctx
            .poll_shared
            .viewport_image
            .map(|summary| summary.signature);
        if state.viewport_image_signature != image_signature {
            if image_signature.is_none() {
                state.viewport_image = None;
                state.viewport_image_signature = None;
                self.pending_viewport_image_query = None;
            } else if self.pending_viewport_image_query.is_none() {
                let id = self.next_query_id;
                self.next_query_id += 1;
                self.pending_viewport_image_query = Some((id, image_signature.unwrap_or_default()));
                ctx.query_host(WireHostQuery::ViewportImage { id });
            }
        }

        // Update movie state snapshot
        state.movie_state.frame_count = ctx.poll_shared.movie.frame_count;
        state.movie_state.current_frame = ctx.poll_shared.movie.current_frame;
        state.movie_state.is_playing = ctx.poll_shared.movie.is_playing;
        state.movie_state.rock_enabled = ctx.poll_shared.movie.rock_enabled;
    }

    /// Drain the command queue and schedule execution.
    fn drain_commands(&mut self, ctx: &mut PollContext<'_>) {
        let commands: Vec<(String, bool)> = {
            let mut state = self.shared.lock().unwrap();
            std::mem::take(&mut state.cmd_queue)
        };

        for (command, silent) in commands {
            let id = self.next_cmd_id;
            self.next_cmd_id += 1;
            ctx.execute_command(id, &command, silent);
        }
    }

    /// Drain the alter buffer and queue a viewer mutation to apply atom changes.
    fn drain_alter_queue(&mut self, ctx: &mut PollContext<'_>) {
        let batches = {
            let state = self.shared.lock().unwrap();
            let mut buf = state.alter_buffer.lock().unwrap();
            if buf.is_empty() {
                return;
            }
            std::mem::take(&mut *buf)
        };
        let actions: Vec<WireAtomPropertyChange> = batches
            .into_iter()
            .flatten()
            .map(atom_change_to_wire)
            .collect();
        if !actions.is_empty() {
            if let Ok(mut state) = self.shared.lock() {
                state.molecule_generation = None;
            }
            ctx.queue_viewer_action(WireViewerAction::ApplyAtomPropertyChanges(actions));
        }
    }

    /// Drain keybind trigger IDs and submit callbacks to the worker thread.
    fn drain_keybind_triggers(&mut self) {
        let triggers: Vec<u64> = {
            let mut state = self.shared.lock().unwrap();
            std::mem::take(&mut state.keybinds.triggers)
        };

        if triggers.is_empty() {
            return;
        }

        // Py<PyAny>::clone() requires the GIL
        Python::attach(|py| {
            for id in triggers {
                let callback: Option<Py<PyAny>> = {
                    let state = self.shared.lock().unwrap();
                    state.keybinds.callbacks.get(&id).map(|cb| cb.clone_ref(py))
                };
                if let Some(cb) = callback {
                    self.worker.submit(WorkItem::InvokeKeybindCallback {
                        callback: cb,
                        origin: WorkOrigin::Command,
                    });
                }
            }
        });
    }

    /// Drain pending keybind registration/unregistration requests.
    fn drain_keybind_requests(&self, ctx: &mut PollContext<'_>) {
        let (requests, unreg_requests) = {
            let mut state = self.shared.lock().unwrap();
            let r = std::mem::take(&mut state.keybinds.requests);
            let u = std::mem::take(&mut state.keybinds.unreg_requests);
            (r, u)
        };

        // Process unregistrations
        for key_str in unreg_requests {
            ctx.unregister_hotkey(key_str);
        }

        // Process registrations with a portable custom action. The host owns
        // the hotkey action and routes the trigger back through the message bus.
        for (id, key_str) in requests {
            ctx.register_hotkey(
                key_str,
                PluginKeyAction::Custom {
                    topic: PYTHON_KEYBIND_TRIGGER_TOPIC.to_string(),
                    payload: id.to_le_bytes().to_vec(),
                },
            );
        }
    }

    /// Drain queued viewport image set/clear and send via message bus.
    fn drain_image_queue(&self, ctx: &mut PollContext<'_>) {
        let pending = {
            let mut state = self.shared.lock().unwrap();
            state.set_image_queue.take()
        };

        if let Some(action) = pending {
            match action {
                Some((data, width, height)) => {
                    ctx.set_viewport_image(ViewportImage {
                        data,
                        width,
                        height,
                    });
                }
                None => {
                    ctx.clear_viewport_image();
                }
            }
        }
    }
}

fn atom_change_to_wire(change: AtomChange) -> WireAtomPropertyChange {
    WireAtomPropertyChange {
        object: change.obj,
        atom_index: change.idx,
        changes: change
            .changes
            .into_iter()
            .map(|(key, value)| (key, property_value_to_wire(value)))
            .collect(),
    }
}

fn property_value_to_wire(value: PropertyValue) -> WireAtomPropertyValue {
    match value {
        PropertyValue::Str(value) => WireAtomPropertyValue::Str(value),
        PropertyValue::F32(value) => WireAtomPropertyValue::F32(value),
        PropertyValue::I32(value) => WireAtomPropertyValue::I32(value),
        PropertyValue::I8(value) => WireAtomPropertyValue::I8(value),
        PropertyValue::Bool(value) => WireAtomPropertyValue::Bool(value),
    }
}

impl MessageHandler for PythonHandler {
    fn on_message(&mut self, msg: &AppMessage, _bus: &mut MessageBus) {
        let AppMessage::Custom { topic, payload } = msg else {
            return;
        };
        if topic != PYTHON_KEYBIND_TRIGGER_TOPIC || payload.len() != 8 {
            return;
        }
        let mut id = [0_u8; 8];
        id.copy_from_slice(payload);
        if let Ok(mut state) = self.shared.lock() {
            state.keybinds.triggers.push(u64::from_le_bytes(id));
        }
    }

    fn needs_poll(&self) -> bool {
        true
    }

    fn poll(&mut self, ctx: &mut PollContext<'_>) {
        // Request backend installation on first poll (async via worker).
        // Then queue any Python startup scripts found in plugin directories.
        // The worker processes items sequentially, so the backend is installed
        // before any scripts execute.
        if !self.backend_requested {
            self.worker.submit(WorkItem::InstallBackend {
                shared: self.shared.clone(),
            });
            for path in collect_startup_scripts(ctx.plugin_dirs) {
                log::info!("Queuing Python startup script: {}", path.display());
                self.worker.submit(WorkItem::ExecFile {
                    path: path.to_string_lossy().to_string(),
                    origin: WorkOrigin::Script,
                });
            }
            self.backend_requested = true;
        }

        // Drain results from the Python worker thread
        self.drain_python_results(ctx);

        // Drain host query results from previous poll cycles
        self.drain_host_query_results(ctx);

        // Show notification overlay while Python is busy
        let busy = self.worker.is_busy();
        if busy {
            ctx.set_notification("Running Python...");
        }
        if busy != self.last_busy {
            self.last_busy = busy;
            ctx.request_panel_update();
        }

        // Sync state snapshots
        self.update_snapshots(ctx);

        // Drain blocking host bridge requests from Python APIs
        self.drain_host_bridge_requests(ctx);

        // Drain queued commands from Python
        self.drain_commands(ctx);

        // Drain atom mutations from alter()
        self.drain_alter_queue(ctx);

        // Drain queued viewport image changes
        self.drain_image_queue(ctx);

        // Process keybind triggers queued by callbacks.
        self.drain_keybind_triggers();

        // Process pending keybind registration/unregistration requests
        self.drain_keybind_requests(ctx);
    }
}

/// Collect `.py` files from `python/` subdirectories of the given plugin directories.
///
/// Returns paths sorted alphabetically within each directory so execution order
/// is deterministic. Directories are processed in the order they were loaded
/// (bundled first, then user).
fn collect_startup_scripts(plugin_dirs: &[PathBuf]) -> Vec<PathBuf> {
    collect_startup_scripts_with_fs(plugin_dirs, &ProcessStartupScriptFs)
}

trait StartupScriptFs {
    fn read_dir_paths(&self, dir: &Path) -> Option<Vec<PathBuf>>;
}

struct ProcessStartupScriptFs;

impl StartupScriptFs for ProcessStartupScriptFs {
    fn read_dir_paths(&self, dir: &Path) -> Option<Vec<PathBuf>> {
        let entries = std::fs::read_dir(dir).ok()?;
        Some(entries.flatten().map(|entry| entry.path()).collect())
    }
}

fn collect_startup_scripts_with_fs(
    plugin_dirs: &[PathBuf],
    fs: &impl StartupScriptFs,
) -> Vec<PathBuf> {
    let mut scripts = Vec::new();
    for dir in plugin_dirs {
        let python_dir = dir.join("python");
        let Some(entries) = fs.read_dir_paths(&python_dir) else {
            continue;
        };
        let mut dir_scripts: Vec<PathBuf> = entries
            .into_iter()
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("py"))
            .collect();
        dir_scripts.sort();
        scripts.extend(dir_scripts);
    }
    scripts
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct FakeStartupScriptFs {
        dirs: BTreeMap<PathBuf, Vec<PathBuf>>,
    }

    impl StartupScriptFs for FakeStartupScriptFs {
        fn read_dir_paths(&self, dir: &Path) -> Option<Vec<PathBuf>> {
            self.dirs.get(dir).cloned()
        }
    }

    #[test]
    fn startup_scripts_are_sorted_within_each_plugin_dir() {
        let plugin_a = PathBuf::from("/plugins/a");
        let plugin_b = PathBuf::from("/plugins/b");
        let mut fs = FakeStartupScriptFs::default();
        fs.dirs.insert(
            plugin_a.join("python"),
            vec![
                plugin_a.join("python/z.py"),
                plugin_a.join("python/readme.txt"),
                plugin_a.join("python/a.py"),
            ],
        );
        fs.dirs.insert(
            plugin_b.join("python"),
            vec![plugin_b.join("python/b.py"), plugin_b.join("python/a.py")],
        );

        let scripts = collect_startup_scripts_with_fs(&[plugin_a.clone(), plugin_b.clone()], &fs);

        assert_eq!(
            scripts,
            vec![
                plugin_a.join("python/a.py"),
                plugin_a.join("python/z.py"),
                plugin_b.join("python/a.py"),
                plugin_b.join("python/b.py"),
            ]
        );
    }

    #[test]
    fn missing_startup_script_dirs_are_ignored() {
        let scripts = collect_startup_scripts_with_fs(
            &[PathBuf::from("/plugins/missing")],
            &FakeStartupScriptFs::default(),
        );

        assert!(scripts.is_empty());
    }
}
