//! Python Message Handler
//!
//! Implements `MessageHandler` with `needs_poll() = true` for:
//! 1. Draining results from the Python worker thread
//! 2. Reporting task output and completion to the host registry
//! 3. Updating shared molecule/name snapshots from `SharedContext`
//! 4. Installing `sys._patinae_backend` on first poll (via worker)
//! 5. Acknowledging task commands and scene mutations through the host bridge
//! 6. Syncing viewport image and atom streams between Python and the host

use patinae_plugin::tasks::TaskError;
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, TryRecvError};

use std::path::{Path, PathBuf};

use patinae_plugin::prelude::*;

use patinae_plugin::tasks::{TaskDiagnostic, TaskEffects, TaskEvent, TaskOutcome};
use patinae_plugin::wire::{WireHostQuery, WireHostQueryValue, WireViewerAction};

use crate::panel::{PanelTaskRequest, ScriptPanelStateHandle};
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
    next_query_id: u64,
    pending_viewport_image_query: Option<(u64, u64)>,
    pending_bridge_queries: HashMap<u64, PendingBridgeQuery>,
    pending_panel_start: Option<u64>,
    pending_panel_snapshot: Option<u64>,
    pending_starts: HashMap<u64, String>,
    pending_config: Option<u64>,
}

#[derive(Debug, Clone, Copy)]
enum PendingBridgeQuery {
    Json { bridge_id: u64 },
    Unit { bridge_id: u64 },
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
            next_query_id: 1,
            pending_viewport_image_query: None,
            pending_bridge_queries: HashMap::new(),
            pending_panel_start: None,
            pending_panel_snapshot: None,
            pending_starts: HashMap::new(),
            pending_config: None,
        }
    }

    fn host_bridge(&self) -> HostBridgeHandle {
        self.shared.lock().unwrap().host_bridge.clone()
    }

    fn query_id(&mut self) -> u64 {
        let id = self.next_query_id;
        self.next_query_id += 1;
        id
    }

    fn report_executor_loss(&mut self, ctx: &mut PollContext<'_>, error: &str) {
        self.host_bridge().close();
        for task_id in self.worker.take_disconnected_tasks() {
            let mut outcome = TaskOutcome::failure("executor_lost", error);
            outcome.effects = TaskEffects::Unknown;
            ctx.report_task_event(self.query_id(), task_id, TaskEvent::Finished(outcome));
        }
    }

    fn drain_python_results(&mut self, ctx: &mut PollContext<'_>) {
        for _ in 0..self.worker.config().batch_size {
            let result = match self.result_rx.try_recv() {
                Ok(result) => result,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.report_executor_loss(ctx, "Python executor disconnected");
                    break;
                }
            };
            let WorkResult {
                task_id,
                origin,
                payload,
            } = result;
            let Some(task_id) = task_id else {
                if let WorkResultPayload::Setup(Err(error)) = payload {
                    ctx.bus.print_error(error);
                }
                continue;
            };
            let event = match payload {
                WorkResultPayload::Started => TaskEvent::Started,
                WorkResultPayload::Output(output) => {
                    if origin == WorkOrigin::Panel
                        && self.panel_state.lock().unwrap().selected == Some(task_id)
                    {
                        self.panel_state.lock().unwrap().append_output(&output);
                        ctx.request_panel_update();
                    }
                    TaskEvent::Output(TaskDiagnostic {
                        level: "info".into(),
                        message: output,
                    })
                }
                WorkResultPayload::Finished(result) => {
                    let outcome = match result {
                        Ok(()) => TaskOutcome::success(None, TaskEffects::None),
                        Err(error) => {
                            if origin == WorkOrigin::Panel
                                && self.panel_state.lock().unwrap().selected == Some(task_id)
                            {
                                self.panel_state.lock().unwrap().append_error(&error);
                                ctx.request_panel_update();
                            }
                            TaskOutcome::failure("python_error", error)
                        }
                    };
                    TaskEvent::Finished(outcome)
                }
                WorkResultPayload::ExecutorLost(error) => {
                    // Python may retain an output writer after a panic, keeping
                    // the result channel alive. Fail queued work immediately.
                    self.report_executor_loss(ctx, &error);
                    let mut outcome = TaskOutcome::failure("executor_lost", error);
                    outcome.effects = TaskEffects::Unknown;
                    TaskEvent::Finished(outcome)
                }
                WorkResultPayload::Cancelled => {
                    TaskEvent::Finished(TaskOutcome::cancelled("Python script interrupted"))
                }
                WorkResultPayload::Setup(_) => continue,
            };
            ctx.report_task_event(self.query_id(), task_id, event);
        }
    }

    fn accept_invocations(&mut self, ctx: &mut PollContext<'_>) {
        for invocation in ctx.task_invocations {
            let payload = &invocation.request.payload;
            let origin = match payload["origin"].as_str() {
                Some("panel") => WorkOrigin::Panel,
                Some("script") => WorkOrigin::Script,
                _ => WorkOrigin::Command,
            };
            let item = if let Some(path) = payload["path"].as_str() {
                Some(WorkItem::ExecFile {
                    path: path.into(),
                    origin,
                })
            } else if let Some(code) = payload["code"].as_str() {
                Some(WorkItem::Eval {
                    code: code.into(),
                    origin,
                })
            } else if let Some(id) = payload["callback"].as_u64() {
                Python::attach(|py| {
                    self.shared
                        .lock()
                        .unwrap()
                        .keybinds
                        .callbacks
                        .get(&id)
                        .map(|cb| WorkItem::InvokeKeybindCallback {
                            callback: cb.clone_ref(py),
                            origin,
                        })
                })
            } else {
                None
            };
            let result = item
                .ok_or_else(|| "Invalid Python task payload or missing callback".to_string())
                .and_then(|item| self.worker.submit(invocation.task_id, item));
            if let Err(error) = result {
                ctx.report_task_event(
                    self.query_id(),
                    invocation.task_id,
                    TaskEvent::Finished(TaskOutcome::failure("executor_lost", error)),
                );
            }
        }
        for task_id in &ctx.task_cancellations {
            self.worker.cancel(*task_id);
        }
    }

    fn update_panel_tasks(&mut self, ctx: &mut PollContext<'_>) {
        if let Some(id) = self.pending_panel_start {
            if let Some(result) = ctx.task_result(id) {
                self.pending_panel_start = None;
                match result {
                    TaskQueryResult::Started(Ok(task_id)) => {
                        let mut panel = self.panel_state.lock().unwrap();
                        panel.selected = Some(*task_id);
                        panel.snapshot = None;
                    }
                    TaskQueryResult::Started(Err(error)) => self
                        .panel_state
                        .lock()
                        .unwrap()
                        .append_error(&error.to_string()),
                    _ => self
                        .panel_state
                        .lock()
                        .unwrap()
                        .append_error("Task admission failed"),
                }
                ctx.request_panel_update();
            }
        }
        if let Some(id) = self.pending_panel_snapshot {
            if let Some(result) = ctx.task_result(id) {
                self.pending_panel_snapshot = None;
                if let TaskQueryResult::Snapshot(Ok(snapshot)) = result {
                    let mut panel = self.panel_state.lock().unwrap();
                    if panel.snapshot.as_ref() != Some(snapshot) {
                        panel.snapshot = Some(snapshot.clone());
                        ctx.request_panel_update();
                    }
                }
            }
        }
        let requests = std::mem::take(&mut self.panel_state.lock().unwrap().requests);
        for request in requests {
            match request {
                PanelTaskRequest::Run(code) => {
                    let id = self.query_id();
                    let AsyncCommandRequest::Plugin(request) =
                        crate::commands::python_request(code, "panel")
                    else {
                        unreachable!()
                    };
                    ctx.start_task(id, request, None);
                    self.pending_panel_start = Some(id);
                }
                PanelTaskRequest::Cancel(task_id) => ctx.cancel_task(self.query_id(), task_id),
            }
        }
        let selected = self.panel_state.lock().unwrap().selected;
        if self.pending_panel_snapshot.is_none() {
            if let Some(task_id) = selected {
                let id = self.query_id();
                ctx.get_task(id, task_id);
                self.pending_panel_snapshot = Some(id);
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
            if let PendingBridgeQuery::Json { bridge_id } | PendingBridgeQuery::Unit { bridge_id } =
                pending
            {
                let value = match ctx.task_result(result.id) {
                    Some(TaskQueryResult::Command(value)) => value
                        .as_ref()
                        .map(|v| serde_json::to_value(v).unwrap())
                        .map_err(Clone::clone),
                    Some(TaskQueryResult::Snapshot(value)) => value
                        .as_ref()
                        .map(|v| serde_json::to_value(v).unwrap())
                        .map_err(|e| TaskError::new(e.to_string(), e.to_string())),
                    Some(TaskQueryResult::List(value)) => value
                        .as_ref()
                        .map(|v| serde_json::to_value(v).unwrap())
                        .map_err(|e| TaskError::new(e.to_string(), e.to_string())),
                    Some(TaskQueryResult::Cancel(value)) => value
                        .as_ref()
                        .map(|v| serde_json::to_value(v).unwrap())
                        .map_err(|e| TaskError::new(e.to_string(), e.to_string())),
                    Some(TaskQueryResult::Wait(value)) => value
                        .as_ref()
                        .map(|v| serde_json::to_value(v).unwrap())
                        .map_err(Clone::clone),
                    Some(TaskQueryResult::Acknowledged(value)) => value
                        .as_ref()
                        .map(|_| serde_json::Value::Null)
                        .map_err(Clone::clone),
                    Some(TaskQueryResult::Unavailable(error)) => {
                        Err(TaskError::new("host_error", error.to_string()))
                    }
                    _ => Err(TaskError::new(
                        "invalid_response",
                        "Unexpected task bridge response",
                    )),
                };
                bridge.complete(
                    bridge_id,
                    value.map(|value| {
                        if matches!(pending, PendingBridgeQuery::Unit { .. }) {
                            HostBridgeValue::Unit
                        } else {
                            HostBridgeValue::Json(value)
                        }
                    }),
                );
                continue;
            }
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
                    (bridge_id, Err(TaskError::new("host_error", error.clone())))
                }
                (PendingBridgeQuery::CountAtoms { bridge_id }, _)
                | (PendingBridgeQuery::LabelObject { bridge_id }, _)
                | (PendingBridgeQuery::OpenAtomStream { bridge_id }, _)
                | (PendingBridgeQuery::ReadAtomStream { bridge_id }, _)
                | (PendingBridgeQuery::CloseAtomStream { bridge_id }, _) => (
                    bridge_id,
                    Err(TaskError::new(
                        "invalid_response",
                        "host returned an unexpected Python bridge result",
                    )),
                ),
                (PendingBridgeQuery::Json { .. } | PendingBridgeQuery::Unit { .. }, _) => {
                    unreachable!()
                }
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
        let wire_id = self.query_id();
        let parent = request.task_id;
        match request.kind {
            HostBridgeRequestKind::Execute { command, quiet } => {
                let Some(task_id) = parent else {
                    bridge.complete(
                        request.id,
                        Err(TaskError::new(
                            "invalid_parent",
                            "Python command has no task identity",
                        )),
                    );
                    return;
                };
                self.pending_bridge_queries.insert(
                    wire_id,
                    PendingBridgeQuery::Json {
                        bridge_id: request.id,
                    },
                );
                ctx.execute_task_command(wire_id, task_id, &command, quiet);
            }
            HostBridgeRequestKind::GetTask { task_id } => {
                self.pending_bridge_queries.insert(
                    wire_id,
                    PendingBridgeQuery::Json {
                        bridge_id: request.id,
                    },
                );
                ctx.get_task(wire_id, task_id);
            }
            HostBridgeRequestKind::ListTasks { request: filters } => {
                self.pending_bridge_queries.insert(
                    wire_id,
                    PendingBridgeQuery::Json {
                        bridge_id: request.id,
                    },
                );
                ctx.list_tasks(wire_id, filters);
            }
            HostBridgeRequestKind::CancelTask { task_id } => {
                self.pending_bridge_queries.insert(
                    wire_id,
                    PendingBridgeQuery::Json {
                        bridge_id: request.id,
                    },
                );
                ctx.cancel_task(wire_id, task_id);
            }
            HostBridgeRequestKind::WaitTask {
                task_id,
                timeout_ms,
            } => {
                self.pending_bridge_queries.insert(
                    wire_id,
                    PendingBridgeQuery::Json {
                        bridge_id: request.id,
                    },
                );
                ctx.wait_task(wire_id, task_id, parent, timeout_ms);
            }
            HostBridgeRequestKind::SetViewportImage { image } => {
                let Some(task_id) = parent else {
                    bridge.complete(
                        request.id,
                        Err(TaskError::new(
                            "invalid_parent",
                            "Python mutation has no task identity",
                        )),
                    );
                    return;
                };
                self.pending_bridge_queries.insert(
                    wire_id,
                    PendingBridgeQuery::Unit {
                        bridge_id: request.id,
                    },
                );
                let action = image
                    .map(WireViewerAction::SetViewportImage)
                    .unwrap_or(WireViewerAction::ClearViewportImage);
                ctx.apply_task_action(wire_id, task_id, action);
            }
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
                let Some(task_id) = parent else {
                    bridge.complete(
                        request.id,
                        Err(TaskError::new(
                            "invalid_parent",
                            "Python mutation has no task identity",
                        )),
                    );
                    return;
                };
                self.pending_bridge_queries.insert(
                    wire_id,
                    PendingBridgeQuery::Unit {
                        bridge_id: request.id,
                    },
                );
                ctx.apply_task_action(
                    wire_id,
                    task_id,
                    WireViewerAction::ApplyAtomPropertyChanges(changes),
                );
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

    fn drain_keybind_triggers(&mut self, ctx: &mut PollContext<'_>) {
        let triggers = std::mem::take(&mut self.shared.lock().unwrap().keybinds.triggers);
        for callback in triggers {
            let request_id = self.query_id();
            self.pending_starts
                .insert(request_id, "Python key callback".into());
            ctx.start_task(
                request_id,
                crate::commands::python_task(serde_json::json!({"callback": callback})),
                None,
            );
        }
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
}

impl Drop for PythonHandler {
    fn drop(&mut self) {
        // A worker blocked on a host acknowledgement must wake before the host
        // drops its transport. Cancellation remains specific to its task token.
        self.host_bridge().close();
        self.worker.cancel_all();
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
        if let Some(id) = self.pending_config {
            if let Some(reply) = ctx.host_query_results.iter().find(|reply| reply.id == id) {
                self.pending_config = None;
                match &reply.result {
                    Ok(WireHostQueryValue::TaskConfig(config)) => {
                        self.worker.configure(config.clone())
                    }
                    Err(error) => ctx
                        .bus
                        .print_error(format!("Python task configuration: {error}")),
                    _ => ctx
                        .bus
                        .print_error("Python host returned an invalid task configuration"),
                }
            }
        }
        if !self.backend_requested {
            let id = self.query_id();
            ctx.query_host(WireHostQuery::TaskConfig { id });
            self.pending_config = Some(id);
            if let Err(error) = self.worker.install(self.shared.clone()) {
                ctx.bus.print_error(error);
            }
            for path in collect_startup_scripts(ctx.plugin_dirs) {
                let request_id = self.query_id();
                self.pending_starts.insert(
                    request_id,
                    format!("Python startup script {}", path.display()),
                );
                ctx.start_task(
                    request_id,
                    crate::commands::python_task(
                        serde_json::json!({"path": path.to_string_lossy(), "origin": "script"}),
                    ),
                    None,
                );
            }
            self.backend_requested = true;
        }
        self.pending_starts.retain(|id, origin| {
            let Some(result) = ctx.task_result(*id) else {
                return true;
            };
            let error = match result {
                TaskQueryResult::Started(Ok(_)) => None,
                TaskQueryResult::Started(Err(error)) => Some(format!("{origin}: {error}")),
                TaskQueryResult::Unavailable(error) => Some(format!("{origin}: {error}")),
                _ => Some(format!("{origin}: unexpected task admission response")),
            };
            if let Some(error) = error {
                ctx.bus.print_error(error);
            }
            false
        });
        self.accept_invocations(ctx);
        self.update_snapshots(ctx);
        // Refresh cheap reads before an acknowledgement wakes Python. This
        // makes get_names()/get_movie_state() observe the applied command.
        self.drain_host_query_results(ctx);
        self.drain_host_bridge_requests(ctx);
        self.drain_python_results(ctx);
        self.update_panel_tasks(ctx);
        self.drain_keybind_triggers(ctx);
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
    use crate::runtime::tests::{poll_input, AbiHarness};
    use crate::shared::SharedState;
    use patinae_plugin::tasks::{TaskError, TaskId, TaskOutcomeStatus};
    use patinae_plugin::wire::WireHostQueryResult;
    use std::collections::BTreeMap;
    use std::sync::{atomic::AtomicBool, Arc, Mutex};
    use std::time::{Duration, Instant};

    #[test]
    fn abi_python_cancellation_acknowledges_running_and_queued_tasks() {
        use patinae_plugin::prelude::TaskInvocation;
        use patinae_plugin::tasks::{TaskConfig, TaskRunner, TaskSpec, TaskState, TaskTime};

        let registry = TaskRunner::new(17, TaskConfig::default(), Box::new(TaskTime::default));
        let active = registry.admit(TaskSpec::new("python", "python")).unwrap();
        let queued = registry.admit(TaskSpec::new("python", "python")).unwrap();
        let shared = Arc::new(Mutex::new(SharedState::new(Arc::new(AtomicBool::new(
            false,
        )))));
        let (worker, results) = crate::worker::spawn_worker();
        let mut handler =
            PythonHandler::new(worker, shared, results, crate::panel::shared_panel_state());
        handler.backend_requested = true;
        let mut harness = AbiHarness::with_handler(handler);
        let mut input = poll_input();
        input.task_invocations = vec![
            TaskInvocation {
                task_id: active,
                request: crate::commands::python_task(serde_json::json!({
                    "code": "print('running task')\nwhile True:\n    pass", "origin": "script",
                })),
            },
            TaskInvocation {
                task_id: queued,
                request: crate::commands::python_task(serde_json::json!({
                    "code": "raise AssertionError('cancelled queued task ran')", "origin": "script",
                })),
            },
        ];
        registry.cancel(queued).unwrap();
        input.task_cancellations = vec![queued];
        let apply_events = |output: patinae_plugin::wire::WirePollOutput| {
            for query in output.host_queries {
                if let WireHostQuery::TaskEvent { task_id, event, .. } = query {
                    match event {
                        TaskEvent::Started => {
                            registry.started(task_id, "python").unwrap();
                        }
                        TaskEvent::Progress(progress) => {
                            registry.progress(task_id, "python", progress).unwrap();
                        }
                        TaskEvent::Output(output) => {
                            registry.output(task_id, "python", output).unwrap();
                        }
                        TaskEvent::Finished(outcome) => {
                            registry.finish_owned(task_id, "python", outcome).unwrap();
                        }
                    }
                }
            }
        };
        apply_events(harness.poll(&input));
        let deadline = Instant::now() + Duration::from_secs(5);
        while registry.get(active).unwrap().diagnostics.is_empty() {
            assert!(
                Instant::now() < deadline,
                "running Python task did not start"
            );
            apply_events(harness.poll(&poll_input()));
            std::thread::yield_now();
        }
        assert_eq!(registry.get(active).unwrap().state, TaskState::Running);
        assert!(!registry.get(active).unwrap().cancel_requested);
        assert!(!registry.get(queued).unwrap().state.is_terminal());
        registry
            .record_effects(active, "python", TaskEffects::Applied)
            .unwrap();
        registry.cancel(active).unwrap();
        let mut cancel = poll_input();
        cancel.task_cancellations = vec![active];
        apply_events(harness.poll(&cancel));
        while !registry.get(queued).unwrap().state.is_terminal() {
            assert!(
                Instant::now() < deadline,
                "Python cancellation was not acknowledged"
            );
            apply_events(harness.poll(&poll_input()));
            std::thread::yield_now();
        }
        assert_eq!(registry.get(active).unwrap().state, TaskState::Cancelled);
        assert_eq!(registry.get(active).unwrap().effects, TaskEffects::Partial);
        assert_eq!(registry.get(queued).unwrap().state, TaskState::Cancelled);
        assert_eq!(registry.get(queued).unwrap().effects, TaskEffects::None);
    }

    #[test]
    fn abi_bridge_preserves_task_identity_and_returns_apply_failure_before_completion() {
        let task_id = TaskId::new(9, 1);
        let shared = Arc::new(Mutex::new(SharedState::new(Arc::new(AtomicBool::new(
            false,
        )))));
        let bridge = shared.lock().unwrap().host_bridge.clone();
        let (worker, _worker_results) = crate::worker::spawn_worker();
        let (results, receiver) = std::sync::mpsc::channel();
        let mut handler =
            PythonHandler::new(worker, shared, receiver, crate::panel::shared_panel_state());
        // This test exercises the real SDK poll vtable while controlling worker
        // transport events. Interpreter execution is covered by worker tests.
        handler.backend_requested = true;
        let mut harness = AbiHarness::with_handler(handler);
        std::thread::scope(|scope| {
            let wait = scope.spawn(|| {
                bridge.request(
                    HostBridgeRequestKind::ApplyAtomPropertyChanges { changes: vec![] },
                    Some(task_id),
                    &AtomicBool::new(false),
                )
            });
            let deadline = Instant::now() + Duration::from_secs(5);
            let request_id = loop {
                let output = harness.poll(&poll_input());
                if let Some(id) = output.host_queries.iter().find_map(|query| match query {
                    WireHostQuery::TaskAction {
                        id, task_id: owner, ..
                    } => {
                        assert_eq!(*owner, task_id);
                        Some(*id)
                    }
                    _ => None,
                }) {
                    break id;
                }
                assert!(Instant::now() < deadline, "mutation was not submitted");
                std::thread::yield_now();
            };
            assert!(
                !wait.is_finished(),
                "mutation must wait for application acknowledgement"
            );
            let mut input = poll_input();
            input.host_query_results.push(WireHostQueryResult {
                id: request_id,
                result: Ok(WireHostQueryValue::TaskAcknowledged(Err(TaskError {
                    code: "apply_failed".into(),
                    message: "object was removed".into(),
                }))),
            });
            harness.poll(&input);
            let error = match wait.join().unwrap() {
                Err(error) => error,
                Ok(_) => panic!("failed scene application must reach Python"),
            };
            assert!(error.message.contains("object was removed"));
            results
                .send(WorkResult {
                    task_id: Some(task_id),
                    origin: WorkOrigin::Script,
                    payload: WorkResultPayload::Finished(Err(error.to_string())),
                })
                .unwrap();
            let output = harness.poll(&poll_input());
            assert!(output.host_queries.iter().any(|query| matches!(query,
                WireHostQuery::TaskEvent { task_id: owner, event: TaskEvent::Finished(outcome), .. }
                    if *owner == task_id && matches!(&outcome.status, TaskOutcomeStatus::Failure { error } if error.message.contains("object was removed"))
            )));
            assert!(output.command_exec.is_empty());
            assert!(
                output.viewer_actions.is_empty(),
                "task mutations must use acknowledged task actions"
            );
        });
    }

    #[test]
    fn abi_executor_disconnect_closes_bridge() {
        let shared = Arc::new(Mutex::new(SharedState::new(Arc::new(AtomicBool::new(
            false,
        )))));
        let (worker, _worker_results) = crate::worker::spawn_worker();
        // A dropped worker event channel is an executor failure, independent of
        // whether the observer continues polling this plugin.
        let (sender, receiver) = std::sync::mpsc::channel();
        drop(sender);
        let bridge = shared.lock().unwrap().host_bridge.clone();
        let mut handler =
            PythonHandler::new(worker, shared, receiver, crate::panel::shared_panel_state());
        handler.backend_requested = true;
        let mut harness = AbiHarness::with_handler(handler);
        harness.poll(&poll_input());
        let result = bridge.request(
            HostBridgeRequestKind::CountAtoms {
                selection: "all".into(),
            },
            None,
            &AtomicBool::new(false),
        );
        assert!(matches!(result, Err(error) if error.message.contains("disconnected")));
    }

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
