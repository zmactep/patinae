//! IPC Message Handler
//!
//! Implements the plugin `MessageHandler` trait, bridging the IPC server
//! with the host application through the `PollContext` polling API.

use patinae_plugin::prelude::*;
use patinae_plugin::registrar::TaskQueryResult;
use patinae_plugin::wire::{WireHostQuery, WireHostQueryValue};
use std::collections::HashMap;

use crate::protocol::{IpcRequest, IpcResponse};
use crate::server::IpcServer;

/// IPC message handler — bridges the IPC server with the plugin system.
///
/// Uses `PollContext` for deferred command execution, dynamic command
/// registration, and reading application state.
pub struct IpcMessageHandler {
    server: IpcServer,
    pending: HashMap<u64, PendingReply>,
    next_request: u64,
    callbacks: HashMap<u64, (patinae_plugin::tasks::TaskId, u64)>,
    callback_generation: Option<u64>,
    registered_commands: Vec<String>,
    negotiated_generation: Option<u64>,
}

// Bound per-frame work and replies retained for a slow host/client.
const REQUESTS_PER_TICK: usize = 64;
const MAX_PENDING_REPLIES: usize = 1024;

#[derive(Clone, Copy)]
enum ReplyKind {
    Execute,
    CountAtoms,
    Capabilities,
    Task,
}

#[derive(Clone, Copy)]
struct PendingReply {
    generation: u64,
    client_id: u64,
    kind: ReplyKind,
}

impl IpcMessageHandler {
    pub fn new(server: IpcServer) -> Self {
        Self {
            server,
            pending: HashMap::new(),
            next_request: 1,
            callbacks: HashMap::new(),
            callback_generation: None,
            registered_commands: Vec::new(),
            negotiated_generation: None,
        }
    }

    fn owns_callback(&self, id: patinae_plugin::tasks::TaskId) -> bool {
        self.callbacks.values().any(|(task, generation)| {
            *task == id && Some(*generation) == self.server.connection_generation()
        })
    }

    fn route_request(&mut self, id: u64, kind: ReplyKind) -> Result<u64, String> {
        let generation = self
            .server
            .connection_generation()
            .ok_or("IPC client disconnected")?;
        if self
            .pending
            .values()
            .any(|pending| pending.generation == generation && pending.client_id == id)
        {
            return Err("request ID is already pending".into());
        }
        if self.pending.len() >= MAX_PENDING_REPLIES {
            return Err("Too many pending IPC requests".into());
        }
        let token = self.next_request;
        self.next_request = token
            .checked_add(1)
            .ok_or("IPC request identity exhausted")?;
        self.pending.insert(
            token,
            PendingReply {
                generation,
                client_id: id,
                kind,
            },
        );
        Ok(token)
    }

    fn take_reply(&mut self, token: u64) -> Option<PendingReply> {
        self.pending
            .remove(&token)
            .filter(|reply| Some(reply.generation) == self.server.connection_generation())
    }

    fn check_protocol(&mut self, request: &IpcRequest) -> Result<(), String> {
        match request {
            IpcRequest::Ping { .. } | IpcRequest::Capabilities { .. } => Ok(()),
            IpcRequest::Hello {
                protocol_version, ..
            } => {
                if *protocol_version != crate::protocol::IPC_PROTOCOL_VERSION {
                    self.negotiated_generation = None;
                    return Err(format!(
                        "unsupported_protocol: expected IPC version {}",
                        crate::protocol::IPC_PROTOCOL_VERSION
                    ));
                }
                self.negotiated_generation = self.server.connection_generation();
                Ok(())
            }
            _ if self.negotiated_generation.is_some()
                && self.negotiated_generation == self.server.connection_generation() =>
            {
                Ok(())
            }
            _ => Err(
                "protocol_required: send Hello with the current protocol_version before using IPC"
                    .into(),
            ),
        }
    }

    /// Handle a single IPC request, returning an optional immediate response.
    ///
    /// Some requests (Execute, RegisterCommand, UnregisterCommand) are deferred
    /// through `PollContext` and have no immediate response. Others (Ping,
    /// GetNames, etc.) can be answered synchronously.
    fn handle_request(
        &mut self,
        request: &IpcRequest,
        ctx: &mut PollContext<'_>,
    ) -> Option<IpcResponse> {
        if let Err(message) = self.check_protocol(request) {
            let id = serde_json::to_value(request)
                .ok()
                .and_then(|value| value.get("id").and_then(serde_json::Value::as_u64))
                .unwrap_or(0);
            return Some(IpcResponse::Error { id, message });
        }
        match request {
            IpcRequest::Capabilities { id } => {
                let token = match self.route_request(*id, ReplyKind::Capabilities) {
                    Ok(token) => token,
                    Err(message) => return Some(IpcResponse::Error { id: *id, message }),
                };
                ctx.query_host(WireHostQuery::TaskConfig { id: token });
                None
            }
            IpcRequest::GetTask { id, task_id } => {
                let token = match self.route_request(*id, ReplyKind::Task) {
                    Ok(token) => token,
                    Err(message) => return Some(IpcResponse::Error { id: *id, message }),
                };
                ctx.get_task(token, *task_id);
                None
            }
            IpcRequest::ListTasks { id, request } => {
                let token = match self.route_request(*id, ReplyKind::Task) {
                    Ok(token) => token,
                    Err(message) => return Some(IpcResponse::Error { id: *id, message }),
                };
                ctx.list_tasks(token, request.clone());
                None
            }
            IpcRequest::CancelTask { id, task_id } => {
                let token = match self.route_request(*id, ReplyKind::Task) {
                    Ok(token) => token,
                    Err(message) => return Some(IpcResponse::Error { id: *id, message }),
                };
                ctx.cancel_task(token, *task_id);
                None
            }
            IpcRequest::Execute {
                parent_id,
                id,
                command,
                silent,
            } => {
                log::debug!("IPC Execute: {}", command);
                let token = match self.route_request(*id, ReplyKind::Execute) {
                    Ok(token) => token,
                    Err(message) => return Some(IpcResponse::Error { id: *id, message }),
                };
                if let Some(parent) = parent_id {
                    if !self.owns_callback(*parent) {
                        self.pending.remove(&token);
                        return Some(IpcResponse::Error {
                            id: *id,
                            message: "callback belongs to another connection".into(),
                        });
                    }
                    if let Some(reply) = self.pending.get_mut(&token) {
                        reply.kind = ReplyKind::Task;
                    }
                    ctx.execute_task_command(token, *parent, command, *silent);
                } else {
                    ctx.execute_command(token, command, *silent);
                }

                // Response sent later when result arrives
                None
            }

            IpcRequest::RegisterCommand {
                name,
                description,
                usage,
                arguments,
            } => {
                log::info!("IPC RegisterCommand: {}", name);
                self.registered_commands.push(name.clone());
                ctx.register_owned_dynamic_command(
                    name.clone(),
                    description.clone().unwrap_or_default(),
                    usage.clone().unwrap_or_default(),
                    arguments.clone().unwrap_or_default(),
                    self.server.connection_generation()?,
                );
                None
            }

            IpcRequest::UnregisterCommand { name } => {
                log::info!("IPC UnregisterCommand: {}", name);
                ctx.unregister_dynamic_command(name);
                None
            }

            IpcRequest::CallbackResponse {
                id,
                task_id,
                outcome,
            } => {
                let valid = self.callbacks.get(id).is_some_and(|(task, generation)| {
                    task == task_id && Some(*generation) == self.server.connection_generation()
                });
                if !valid {
                    return Some(IpcResponse::Error {
                        id: *id,
                        message: "unknown callback or wrong connection".into(),
                    });
                }
                let token = match self.route_request(*id, ReplyKind::Task) {
                    Ok(token) => token,
                    Err(message) => return Some(IpcResponse::Error { id: *id, message }),
                };
                self.callbacks.remove(id);
                ctx.report_task_event(
                    token,
                    *task_id,
                    patinae_plugin::tasks::TaskEvent::Finished(outcome.clone()),
                );
                None
            }
            IpcRequest::WaitTask {
                id,
                task_id,
                waiter,
                timeout_ms,
            } => {
                if waiter.is_some_and(|id| !self.owns_callback(id)) {
                    return Some(IpcResponse::Error {
                        id: *id,
                        message: "waiter belongs to another connection".into(),
                    });
                }
                let token = match self.route_request(*id, ReplyKind::Task) {
                    Ok(token) => token,
                    Err(message) => return Some(IpcResponse::Error { id: *id, message }),
                };
                ctx.wait_task(token, *task_id, *waiter, *timeout_ms);
                None
            }

            IpcRequest::GetState { id } => {
                // TODO: Implement state serialization
                Some(IpcResponse::Value {
                    id: *id,
                    value: serde_json::json!({}),
                })
            }

            IpcRequest::GetNames { id } => Some(IpcResponse::Value {
                id: *id,
                value: serde_json::json!(&ctx.poll_shared.object_names),
            }),

            IpcRequest::CountAtoms { id, selection } => {
                let token = match self.route_request(*id, ReplyKind::CountAtoms) {
                    Ok(token) => token,
                    Err(message) => return Some(IpcResponse::Error { id: *id, message }),
                };
                ctx.query_host(WireHostQuery::CountAtoms {
                    id: token,
                    selection: selection.clone(),
                });
                None
            }

            IpcRequest::Hello { client_id, .. } => {
                log::info!("IPC client identified as: {}", client_id);
                self.server.set_client_id(client_id.clone());
                Some(IpcResponse::Ok { id: 0 })
            }

            IpcRequest::Quit => {
                log::info!("IPC Quit received");
                ctx.bus.send(AppMessage::Quit);
                Some(IpcResponse::Closing)
            }

            IpcRequest::Ping { id } => Some(IpcResponse::Pong { id: *id }),

            IpcRequest::ShowWindow { id } => {
                log::info!("IPC ShowWindow received");
                ctx.bus.send(AppMessage::ShowWindow);
                Some(IpcResponse::Ok { id: *id })
            }

            IpcRequest::HideWindow { id } => {
                log::info!("IPC HideWindow received");
                ctx.bus.send(AppMessage::HideWindow);
                Some(IpcResponse::Ok { id: *id })
            }

            IpcRequest::GetView { id } => {
                let view = ctx.poll_shared.camera.current_view();
                let r = &view.rotation;

                // Build the 18-value array:
                // [0-8]: 3x3 rotation matrix (row-major)
                // [9-11]: Camera position
                // [12-14]: Origin
                // [15]: Front clip, [16]: Back clip, [17]: FOV
                let values: Vec<f64> = vec![
                    r.data[0] as f64,
                    r.data[1] as f64,
                    r.data[2] as f64,
                    r.data[4] as f64,
                    r.data[5] as f64,
                    r.data[6] as f64,
                    r.data[8] as f64,
                    r.data[9] as f64,
                    r.data[10] as f64,
                    view.position.x as f64,
                    view.position.y as f64,
                    view.position.z as f64,
                    view.origin.x as f64,
                    view.origin.y as f64,
                    view.origin.z as f64,
                    view.clip_front as f64,
                    view.clip_back as f64,
                    view.fov as f64,
                ];

                Some(IpcResponse::Value {
                    id: *id,
                    value: serde_json::json!(values),
                })
            }
        }
    }
}

impl MessageHandler for IpcMessageHandler {
    fn on_message(&mut self, _msg: &AppMessage, _bus: &mut MessageBus) {
        // IPC handler doesn't react to broadcast messages
    }

    fn needs_poll(&self) -> bool {
        true
    }

    fn poll(&mut self, ctx: &mut PollContext<'_>) {
        self.server.begin_tick();
        let generation = self.server.connection_generation();
        if self.callback_generation != generation {
            for (&token, reply) in &self.pending {
                if Some(reply.generation) != generation {
                    ctx.forget_task_wait(token);
                }
            }
            if let Some(previous) = self.callback_generation {
                ctx.fail_task_owner(u64::MAX, previous);
                for name in self.registered_commands.drain(..) {
                    ctx.unregister_dynamic_command(&name);
                }
            }
            self.callbacks.clear();
            self.callback_generation = generation;
        }

        self.pending
            .retain(|_, reply| Some(reply.generation) == generation);

        // Task controls and queries have distinct host queues but one client view.
        let task_replies: Vec<_> = self
            .pending
            .iter()
            .filter_map(|(token, route)| {
                if !matches!(route.kind, ReplyKind::Task) {
                    return None;
                }
                let id = route.client_id;
                let response = match ctx.task_result(*token)? {
                    TaskQueryResult::Wait(result) => IpcResponse::TaskWait {
                        id,
                        result: result.clone(),
                    },
                    TaskQueryResult::Acknowledged(result) => IpcResponse::TaskAcknowledged {
                        id,
                        result: result.clone(),
                    },
                    TaskQueryResult::Command(result) => match result {
                        Ok(reply) => IpcResponse::Execution {
                            id,
                            reply: reply.clone(),
                        },
                        Err(error) => IpcResponse::Execution {
                            id,
                            reply: patinae_plugin::prelude::CommandReply {
                                result: Err(error.to_string()),
                                messages: Vec::new(),
                                task_ids: Vec::new(),
                            },
                        },
                    },
                    TaskQueryResult::Started(_) => return None,
                    TaskQueryResult::Snapshot(result) => IpcResponse::Task {
                        id,
                        result: result.clone(),
                    },
                    TaskQueryResult::List(result) => IpcResponse::Tasks {
                        id,
                        result: result.clone(),
                    },
                    TaskQueryResult::Cancel(result) => IpcResponse::TaskCancellation {
                        id,
                        result: *result,
                    },
                    TaskQueryResult::Unavailable(message) => IpcResponse::Error {
                        id,
                        message: message.into(),
                    },
                };
                Some((*token, response))
            })
            .collect();
        for (token, response) in task_replies {
            if self.take_reply(token).is_some() {
                if let Err(error) = self.server.send(response) {
                    log::debug!("IPC task reply failed: {error}");
                }
            }
        }

        for result in ctx.host_query_results {
            let Some(route) = self.take_reply(result.id) else {
                continue;
            };
            let response = match &result.result {
                Ok(WireHostQueryValue::TaskConfig(config)) => IpcResponse::Capabilities {
                    id: route.client_id,
                    capabilities: crate::protocol::TaskCapabilities::from_config(config),
                },
                Ok(WireHostQueryValue::CountAtoms(count)) => IpcResponse::Value {
                    id: route.client_id,
                    value: serde_json::json!(count),
                },
                Err(message) => IpcResponse::Error {
                    id: route.client_id,
                    message: message.clone(),
                },
                _ => continue,
            };
            if let Err(e) = self.server.send(response) {
                log::error!("Failed to send query result: {}", e);
            }
        }

        // Deliver results from previously queued command executions
        for result in ctx.command_results {
            let Some(route) = self.take_reply(result.id) else {
                continue;
            };
            let response = command_response(route.client_id, result);
            if let Err(e) = self.server.send(response) {
                log::error!("Failed to send execution result: {}", e);
            }
        }

        for invocation in ctx.task_invocations {
            let task_id = invocation.task_id;
            if invocation.request.owner_tag != generation || generation.is_none() {
                ctx.report_task_event(
                    u64::MAX,
                    task_id,
                    patinae_plugin::tasks::TaskEvent::Finished(
                        patinae_plugin::tasks::TaskOutcome::failure(
                            "executor_lost",
                            "callback connection no longer exists",
                        ),
                    ),
                );
                continue;
            }
            let Some(name) = invocation
                .request
                .payload
                .get("name")
                .and_then(serde_json::Value::as_str)
            else {
                continue;
            };
            let args = invocation
                .request
                .payload
                .get("args")
                .and_then(serde_json::Value::as_array)
                .map(|args| {
                    args.iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            let id = self.server.next_callback_id();
            self.callbacks
                .insert(id, (task_id, generation.expect("checked connection")));
            ctx.report_task_event(u64::MAX, task_id, patinae_plugin::tasks::TaskEvent::Started);
            let response = IpcResponse::CallbackRequest {
                id,
                task_id,
                name: name.into(),
                args,
            };
            if let Err(error) = self.server.send(response) {
                log::debug!("IPC callback send failed: {error}");
            }
        }
        for task_id in &ctx.task_cancellations {
            if self.owns_callback(*task_id) {
                let _ = self
                    .server
                    .send(IpcResponse::CallbackCancel { task_id: *task_id });
            }
        }

        // Process incoming IPC requests
        for _ in 0..REQUESTS_PER_TICK {
            let Some(request) = self.server.poll() else {
                break;
            };
            if let Some(response) = self.handle_request(&request, ctx) {
                if let Err(e) = self.server.send(response) {
                    log::error!("Failed to send IPC response: {}", e);
                }
            }
        }
        self.server.set_pending_replies(self.pending.len());
        self.server.end_tick();
    }
}

fn command_response(id: u64, result: &CommandResult) -> IpcResponse {
    IpcResponse::Execution {
        id,
        reply: result.reply.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use patinae_plugin::tasks::TaskId;

    #[cfg(unix)]
    #[test]
    fn protocol_handshake_precedes_execution_and_is_connection_scoped() {
        use std::os::unix::net::UnixStream;
        let path = crate::tests::socket_path("version");
        let mut handler = IpcMessageHandler::new(IpcServer::bind(&path).unwrap());
        let client = UnixStream::connect(&path).unwrap();
        handler.server.begin_tick();
        let execute: IpcRequest = serde_json::from_value(serde_json::json!({
            "type": "Execute", "id": 1, "command": "delete all"
        }))
        .unwrap();
        assert!(handler.check_protocol(&execute).is_err());
        assert!(handler.check_protocol(&IpcRequest::Ping { id: 2 }).is_ok());
        let legacy: IpcRequest = serde_json::from_value(serde_json::json!({
            "type": "Hello", "client_id": "legacy"
        }))
        .unwrap();
        assert!(handler
            .check_protocol(&legacy)
            .unwrap_err()
            .contains("unsupported_protocol"));
        assert!(handler.check_protocol(&execute).is_err());
        handler
            .check_protocol(&IpcRequest::Hello {
                client_id: "current".into(),
                protocol_version: crate::protocol::IPC_PROTOCOL_VERSION,
            })
            .unwrap();
        assert!(handler.check_protocol(&execute).is_ok());
        drop(client);
        handler.server.begin_tick();
        handler.server.send(IpcResponse::Pong { id: 2 }).unwrap();
        handler.server.end_tick();
        let _replacement = UnixStream::connect(&path).unwrap();
        handler.server.begin_tick();
        assert!(handler.check_protocol(&execute).is_err());
    }

    #[test]
    fn tracked_partial_error_preserves_accepted_tasks_and_silent_messages() {
        let task_id = TaskId::new(1, 7);
        let result = CommandResult {
            id: 99,
            reply: patinae_plugin::prelude::CommandReply {
                result: Err("later command failed".into()),
                messages: vec![patinae_plugin::prelude::OutputMessage {
                    kind: MessageKind::Warning,
                    format: Default::default(),
                    text: "accepted before error".into(),
                }],
                task_ids: vec![task_id],
            },
        };
        let response = command_response(1, &result);
        let json = serde_json::to_value(response).unwrap();
        assert_eq!(json["id"], 1);
        assert_eq!(json["result"]["Err"], "later command failed");
        assert_eq!(json["task_ids"][0], task_id.to_string());
        assert_eq!(json["messages"][0]["text"], "accepted before error");
    }

    #[test]
    fn synchronous_success_uses_the_same_receipt() {
        let result = CommandResult {
            id: 3,
            reply: patinae_plugin::prelude::CommandReply {
                result: Ok(()),
                messages: Vec::new(),
                task_ids: Vec::new(),
            },
        };
        assert_eq!(
            serde_json::to_value(command_response(1, &result)).unwrap(),
            serde_json::json!({"type": "Execution", "id": 1, "result": {"Ok": null}, "messages": [], "task_ids": []})
        );
    }

    #[cfg(unix)]
    #[test]
    fn reconnect_drops_old_command_and_query_routes_with_reused_client_id() {
        use std::os::unix::net::UnixStream;
        let path = crate::tests::socket_path("routes");
        let mut handler = IpcMessageHandler::new(IpcServer::bind(&path).unwrap());
        let first = UnixStream::connect(&path).unwrap();
        handler.server.begin_tick();
        let old_command = handler.route_request(1, ReplyKind::Execute).unwrap();
        assert!(
            handler.route_request(1, ReplyKind::Task).is_err(),
            "overlapping client IDs are ambiguous"
        );
        let old_query = handler.route_request(2, ReplyKind::Task).unwrap();
        let old_count = handler.route_request(3, ReplyKind::CountAtoms).unwrap();
        handler.server.set_pending_replies(handler.pending.len());
        drop(first);
        handler.server.begin_tick();
        // A failed reply write distinguishes a full disconnect from a half-close.
        handler.server.send(IpcResponse::Pong { id: 1 }).unwrap();
        handler.server.end_tick();
        assert!(handler.server.connection_generation().is_none());
        let _second = UnixStream::connect(&path).unwrap();
        handler.server.begin_tick();
        let current = handler.route_request(1, ReplyKind::Task).unwrap();
        assert!(handler.take_reply(old_command).is_none());
        assert!(handler.take_reply(old_query).is_none());
        assert!(handler.take_reply(old_count).is_none());
        assert_eq!(handler.take_reply(current).unwrap().client_id, 1);
    }
}
