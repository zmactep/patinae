//! IPC Protocol definitions
//!
//! Defines the messages exchanged between the GUI and external clients (like patinae-python).
//! The protocol is bidirectional:
//! - Client → GUI: Send commands, register external commands, respond to callbacks
//! - GUI → Client: Send responses, request callback execution for external commands

use patinae_plugin::tasks::{
    TaskCancelReply, TaskId, TaskListPage, TaskListRequest, TaskLookupError, TaskSnapshot,
};
use serde::{Deserialize, Serialize};

/// Version required before commands or executor messages are accepted.
pub const IPC_PROTOCOL_VERSION: u32 = 2;

/// Message FROM client TO GUI
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum IpcRequest {
    /// Read status and retained outcome atomically without consuming it.
    GetTask { id: u64, task_id: TaskId },
    /// Enumerate a bounded page of task summaries.
    ListTasks {
        id: u64,
        #[serde(default)]
        request: TaskListRequest,
    },
    /// Request cooperative cancellation; acknowledgement is not completion.
    CancelTask { id: u64, task_id: TaskId },
    /// Discover the task protocol and retention limits.
    Capabilities { id: u64 },
    /// Execute a command string (parsed by CommandExecutor)
    Execute {
        /// Request ID for matching responses
        id: u64,
        /// Parent callback task for executor-originated commands.
        #[serde(default)]
        parent_id: Option<TaskId>,
        /// Command string to execute
        command: String,
        /// If true, suppress command echo and info/warning output
        #[serde(default)]
        silent: bool,
    },

    /// Register an external command (appears in autocomplete)
    /// When invoked from GUI command line, sends CallbackRequest to client
    RegisterCommand {
        /// Command name
        name: String,
        /// Optional description text
        description: Option<String>,
        /// Optional usage string
        usage: Option<String>,
        /// Optional arguments description
        arguments: Option<String>,
    },

    /// Unregister an external command
    UnregisterCommand {
        /// Command name to unregister
        name: String,
    },

    /// Response to a CallbackRequest from GUI
    /// Includes captured output from execution
    CallbackResponse {
        id: u64,
        task_id: TaskId,
        outcome: patinae_plugin::tasks::TaskOutcome,
    },

    /// Wait for completion while the host continues processing all clients.
    WaitTask {
        id: u64,
        task_id: TaskId,
        #[serde(default)]
        waiter: Option<TaskId>,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },

    /// Get current state (objects, settings, etc.)
    GetState {
        /// Request ID
        id: u64,
    },

    /// Get object names
    GetNames {
        /// Request ID
        id: u64,
    },

    /// Count atoms matching a selection
    CountAtoms {
        /// Request ID
        id: u64,
        /// Selection expression
        selection: String,
    },

    /// Client handshake: identifies the connecting client
    Hello {
        /// Client identifier string (e.g. "patinae-python", "script:analysis.py")
        client_id: String,
        /// Must match the current protocol; omitted versions are rejected.
        #[serde(default)]
        protocol_version: u32,
    },

    /// Close the GUI application
    Quit,

    /// Health check / keepalive
    Ping {
        /// Request ID
        id: u64,
    },

    /// Show the application window (make it visible)
    ShowWindow {
        /// Request ID
        id: u64,
    },

    /// Hide the application window (make it invisible)
    HideWindow {
        /// Request ID
        id: u64,
    },

    /// Get current view matrix (18 floats)
    GetView {
        /// Request ID
        id: u64,
    },
}

/// Message FROM GUI TO client
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum IpcResponse {
    /// Execution receipt; accepted tasks survive a subsequent command error.
    Execution {
        id: u64,
        #[serde(flatten)]
        reply: patinae_plugin::prelude::CommandReply,
    },
    TaskWait {
        id: u64,
        result: Result<TaskSnapshot, patinae_plugin::tasks::TaskError>,
    },
    TaskAcknowledged {
        id: u64,
        result: Result<bool, patinae_plugin::tasks::TaskError>,
    },

    /// Task status and terminal outcome, or a structured lookup error.
    Task {
        id: u64,
        result: Result<TaskSnapshot, TaskLookupError>,
    },
    /// Bounded summaries without terminal payloads.
    Tasks {
        id: u64,
        result: Result<TaskListPage, TaskLookupError>,
    },
    /// Cancellation acknowledgement from the host task runner.
    TaskCancellation {
        id: u64,
        result: Result<TaskCancelReply, TaskLookupError>,
    },
    /// Supported producers and upper bounds, not guaranteed retention durations.
    Capabilities {
        id: u64,
        capabilities: TaskCapabilities,
    },
    /// Command executed successfully
    Ok {
        /// Request ID this is responding to
        id: u64,
    },

    /// Command failed
    Error {
        /// Request ID this is responding to
        id: u64,
        /// Error message
        message: String,
    },

    /// Return value (for queries)
    Value {
        /// Request ID this is responding to
        id: u64,
        /// The value as JSON
        value: serde_json::Value,
    },

    /// GUI requests client to execute an external command or script
    /// Sent when user invokes a registered command from GUI command line
    CallbackRequest {
        task_id: TaskId,
        /// Request ID for matching responses
        id: u64,
        /// Command name
        name: String,
        /// Command arguments
        args: Vec<String>,
    },

    /// Cooperative cancellation for an external callback.
    CallbackCancel { task_id: TaskId },

    /// Pong response to Ping
    Pong {
        /// Request ID this is responding to
        id: u64,
    },

    /// GUI is closing
    Closing,
}

/// Version and limits of the native task protocol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskCapabilities {
    pub task_protocol_version: u32,
    pub tracked_kinds: Vec<String>,
    pub max_active_tasks: usize,
    pub max_terminal_records: usize,
    pub terminal_ttl_seconds: u64,
    pub max_snapshot_bytes: usize,
    pub max_list_items: usize,
    pub max_list_bytes: usize,
}

impl TaskCapabilities {
    pub fn from_config(config: &patinae_plugin::tasks::TaskConfig) -> Self {
        Self {
            task_protocol_version: IPC_PROTOCOL_VERSION,
            tracked_kinds: vec![
                "fetch".into(),
                "pdb_metadata".into(),
                "python".into(),
                "dynamic_command".into(),
                "script".into(),
                "load".into(),
            ],
            max_active_tasks: config.max_active,
            max_terminal_records: config.max_terminal,
            terminal_ttl_seconds: config.terminal_ttl_ms / 1000,
            max_snapshot_bytes: config.max_snapshot_bytes,
            max_list_items: config.max_page_items,
            max_list_bytes: config.max_snapshot_bytes,
        }
    }
}

impl Default for TaskCapabilities {
    fn default() -> Self {
        Self::from_config(&patinae_plugin::tasks::TaskConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_requests_use_opaque_identity_and_optional_list_options() {
        let task_id = TaskId::new(42, 1);
        let encoded =
            serde_json::json!({"type": "GetTask", "id": 7, "task_id": task_id.to_string()});
        assert!(
            matches!(serde_json::from_value::<IpcRequest>(encoded).unwrap(), IpcRequest::GetTask { id: 7, task_id: decoded } if decoded == task_id)
        );
        assert!(
            matches!(serde_json::from_str::<IpcRequest>(r#"{"type":"ListTasks","id":8}"#).unwrap(), IpcRequest::ListTasks { id: 8, request } if request == TaskListRequest::default())
        );
    }

    #[test]
    fn lookup_and_cancel_replies_remain_machine_readable() {
        let expired = IpcResponse::Task {
            id: 1,
            result: Err(TaskLookupError::Expired),
        };
        assert_eq!(
            serde_json::to_value(expired).unwrap()["result"]["Err"],
            "expired"
        );
        let cancelled = IpcResponse::TaskCancellation {
            id: 2,
            result: Ok(TaskCancelReply::Requested),
        };
        assert_eq!(
            serde_json::to_value(cancelled).unwrap()["result"]["Ok"],
            "requested"
        );
    }

    #[test]
    fn capabilities_distinguish_tracked_producers_and_retention_bounds() {
        let capabilities = TaskCapabilities::default();
        assert_eq!(capabilities.task_protocol_version, 2);
        assert!(capabilities
            .tracked_kinds
            .iter()
            .any(|kind| kind == "fetch"));

        assert_eq!(capabilities.max_snapshot_bytes, 64 * 1024);
        assert_eq!(capabilities.terminal_ttl_seconds, 1800);
    }

    #[test]
    fn test_request_serialization() {
        let req = IpcRequest::Execute {
            parent_id: None,
            id: 1,
            command: "load protein.pdb".to_string(),
            silent: false,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("Execute"));
        assert!(json.contains("load protein.pdb"));

        let parsed: IpcRequest = serde_json::from_str(&json).unwrap();
        if let IpcRequest::Execute { id, command, .. } = parsed {
            assert_eq!(id, 1);
            assert_eq!(command, "load protein.pdb");
        } else {
            panic!("Wrong variant");
        }
    }

    #[test]
    fn test_response_serialization() {
        let resp = IpcResponse::CallbackRequest {
            task_id: TaskId::new(1, 1),
            id: 42,
            name: "highlight".to_string(),
            args: vec!["chain".to_string(), "A".to_string()],
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("CallbackRequest"));

        let parsed: IpcResponse = serde_json::from_str(&json).unwrap();
        if let IpcResponse::CallbackRequest { id, name, args, .. } = parsed {
            assert_eq!(id, 42);
            assert_eq!(name, "highlight");
            assert_eq!(args, vec!["chain", "A"]);
        } else {
            panic!("Wrong variant");
        }
    }
}
