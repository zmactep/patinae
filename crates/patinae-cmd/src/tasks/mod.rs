//! Portable task lifecycle and values shared by hosts, executors, and clients.
//!
//! Each session owns one runner. Executors report events through the host and
//! never mutate its records. Runtime scheduling, executable results, and clocks
//! belong to platform adapters; this module performs no I/O or blocking waits.

use std::{fmt, str::FromStr};

mod policy;
mod runner;
pub use runner::TaskRunner;

use serde::{Deserialize, Serialize};

/// Maximum JSON-encoded snapshot size, keeping query replies bounded.
pub const MAX_TASK_SNAPSHOT_BYTES: usize = 64 * 1024;
/// Maximum ready results applied by one host pump, preserving UI opportunities.
pub const ASYNC_TASK_BATCH_SIZE: usize = 16;

/// Opaque identity scoped to one application instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TaskId {
    instance: u128,
    sequence: u64,
}

impl TaskId {
    /// Construct an identity issued by a task runner.
    pub const fn new(instance: u128, sequence: u64) -> Self {
        Self { instance, sequence }
    }
    /// Application instance that issued this identity.
    pub const fn instance(self) -> u128 {
        self.instance
    }
    /// Monotonic issuance number within the application instance.
    pub const fn sequence(self) -> u64 {
        self.sequence
    }
}
impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:032x}-{:016x}", self.instance, self.sequence)
    }
}
impl From<TaskId> for String {
    fn from(id: TaskId) -> Self {
        id.to_string()
    }
}
impl TryFrom<String> for TaskId {
    type Error = TaskLookupError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}
impl FromStr for TaskId {
    type Err = TaskLookupError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (instance, sequence) = value.split_once('-').ok_or(TaskLookupError::InvalidId)?;
        if instance.len() != 32 || sequence.len() != 16 {
            return Err(TaskLookupError::InvalidId);
        }
        Ok(Self::new(
            u128::from_str_radix(instance, 16).map_err(|_| TaskLookupError::InvalidId)?,
            u64::from_str_radix(sequence, 16).map_err(|_| TaskLookupError::InvalidId)?,
        ))
    }
}

/// Execution state, including the main-thread commit phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Queued,
    Running,
    Applying,
    Succeeded,
    Failed,
    Cancelled,
}
impl TaskState {
    /// Whether the task has an immutable retained outcome.
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

/// Effects observed when the task ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskEffects {
    None,
    Applied,
    Partial,
    Unknown,
}

impl TaskEffects {
    /// Classify host writes without inspecting or copying scene contents.
    ///
    /// Unrestricted mutable access cannot prove a write and yields `Unknown`.
    /// Failure and cancellation convert observed writes to `Partial` at finish.
    pub fn between(
        before: patinae_scene::MutationStamp,
        after: patinae_scene::MutationStamp,
    ) -> Self {
        if after.has_untracked_since(before) {
            Self::Unknown
        } else if after.has_applied_since(before) {
            Self::Applied
        } else {
            Self::None
        }
    }
}

/// Small versioned task-specific result payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskData {
    pub kind: String,
    pub schema_version: u32,
    pub payload: serde_json::Value,
}

/// Machine-readable failure and bounded human-readable detail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskError {
    pub code: String,
    pub message: String,
}
impl TaskError {
    /// Construct a task failure.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}
impl fmt::Display for TaskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for TaskError {}

/// Diagnostic accompanying a terminal outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskDiagnostic {
    pub level: String,
    pub message: String,
}

impl From<&crate::OutputMessage> for TaskDiagnostic {
    fn from(message: &crate::OutputMessage) -> Self {
        Self {
            level: match message.kind {
                crate::MessageKind::Info => "info",
                crate::MessageKind::Warning => "warning",
                crate::MessageKind::Error => "error",
            }
            .into(),
            message: message.text.clone(),
        }
    }
}

/// Exactly one terminal disposition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum TaskOutcomeStatus {
    Success { data: Option<TaskData> },
    Failure { error: TaskError },
    Cancelled { reason: String },
}

/// Retained terminal outcome; executable task results remain in the framework.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskOutcome {
    pub status: TaskOutcomeStatus,
    pub effects: TaskEffects,
    pub diagnostics: Vec<TaskDiagnostic>,
    pub diagnostics_truncated: bool,
}
impl TaskOutcome {
    /// Construct a successful outcome after mandatory effects were applied.
    pub fn success(data: Option<TaskData>, effects: TaskEffects) -> Self {
        Self {
            status: TaskOutcomeStatus::Success { data },
            effects,
            diagnostics: Vec::new(),
            diagnostics_truncated: false,
        }
    }
    /// Construct a failure with no applied effects.
    pub fn failure(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status: TaskOutcomeStatus::Failure {
                error: TaskError::new(code, message),
            },
            effects: TaskEffects::None,
            diagnostics: Vec::new(),
            diagnostics_truncated: false,
        }
    }
    /// Construct an acknowledged cancellation.
    pub fn cancelled(reason: impl Into<String>) -> Self {
        Self {
            status: TaskOutcomeStatus::Cancelled {
                reason: reason.into(),
            },
            effects: TaskEffects::None,
            diagnostics: Vec::new(),
            diagnostics_truncated: false,
        }
    }
    /// Terminal state corresponding to this outcome.
    pub const fn state(&self) -> TaskState {
        match self.status {
            TaskOutcomeStatus::Success { .. } => TaskState::Succeeded,
            TaskOutcomeStatus::Failure { .. } => TaskState::Failed,
            TaskOutcomeStatus::Cancelled { .. } => TaskState::Cancelled,
        }
    }
}

/// Progress observed by the host; counters are absent when work is unmeasured.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskProgress {
    pub phase: String,
    pub message: String,
    pub completed: Option<u64>,
    pub total: Option<u64>,
}

/// Atomic task status and outcome returned by `get`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskSnapshot {
    pub id: TaskId,
    pub parent_id: Option<TaskId>,
    pub kind: String,
    pub origin: String,
    pub state: TaskState,
    pub revision: u64,
    pub progress: Option<TaskProgress>,
    pub cancel_requested: bool,
    pub cancellable: bool,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub effects: TaskEffects,
    pub diagnostics: Vec<TaskDiagnostic>,
    pub diagnostics_truncated: bool,
    pub outcome: Option<TaskOutcome>,
}

/// Lightweight list entry; result payloads require an explicit `get`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskSummary {
    pub id: TaskId,
    pub parent_id: Option<TaskId>,
    pub kind: String,
    pub origin: String,
    pub state: TaskState,
    pub revision: u64,
    pub cancel_requested: bool,
    pub cancellable: bool,
    pub progress: Option<TaskProgress>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub effects: TaskEffects,
}
impl From<&TaskSnapshot> for TaskSummary {
    fn from(task: &TaskSnapshot) -> Self {
        Self {
            id: task.id,
            parent_id: task.parent_id,
            kind: task.kind.clone(),
            origin: task.origin.clone(),
            state: task.state,
            revision: task.revision,
            cancel_requested: task.cancel_requested,
            cancellable: task.cancellable,
            progress: task.progress.clone(),
            created_at_ms: task.created_at_ms,
            updated_at_ms: task.updated_at_ms,
            effects: task.effects,
        }
    }
}

/// Bounded enumeration filters and an exclusive identity cursor.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskListRequest {
    pub after: Option<TaskId>,
    pub limit: Option<usize>,
    pub active_only: bool,
    pub origin: Option<String>,
}
/// One list page ordered by issuance identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskListPage {
    pub tasks: Vec<TaskSummary>,
    pub next_cursor: Option<TaskId>,
}

/// Lookup failure without retaining an unbounded tombstone registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskLookupError {
    InvalidId,
    WrongInstance,
    Expired,
    NotFound,
}
impl fmt::Display for TaskLookupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidId => "invalid_id",
            Self::WrongInstance => "wrong_instance",
            Self::Expired => "expired",
            Self::NotFound => "not_found",
        })
    }
}
impl std::error::Error for TaskLookupError {}

/// Admission failure; no task identity has been issued.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStartError {
    Busy,
    IdExhausted,
    InvalidParent,
    ExecutorUnavailable,
}
impl fmt::Display for TaskStartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Busy => "too many active tasks",
            Self::IdExhausted => "task identity space exhausted",
            Self::ExecutorUnavailable => "task executor unavailable",
            Self::InvalidParent => "parent task is unavailable or no longer accepts children",
        })
    }
}
impl std::error::Error for TaskStartError {}

/// Cancellation acknowledgement; `requested` does not mean the worker has stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskCancelReply {
    Requested,
    AlreadyRequested,
    Unsupported,
    TooLate,
}

/// Invalidation hint broadcast after the corresponding record was updated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskChanged {
    pub id: TaskId,
    pub revision: u64,
    pub state: TaskState,
}

/// Executor messages; the receiving host supplies and validates the owner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskEvent {
    Started,
    Progress(TaskProgress),
    Output(TaskDiagnostic),
    Finished(TaskOutcome),
}

/// Shared admission, retention, response, and host-pump limits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskConfig {
    pub max_active: usize,
    pub max_terminal: usize,
    pub terminal_ttl_ms: u64,
    pub max_snapshot_bytes: usize,
    pub max_page_items: usize,
    pub batch_size: usize,
}
impl Default for TaskConfig {
    fn default() -> Self {
        Self {
            max_active: 128,
            max_terminal: 512,
            terminal_ttl_ms: 30 * 60 * 1000,
            max_snapshot_bytes: MAX_TASK_SNAPSHOT_BYTES,
            max_page_items: 64,
            batch_size: ASYNC_TASK_BATCH_SIZE,
        }
    }
}

/// Wall time for display and monotonic time for retention.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TaskTime {
    pub unix_ms: u64,
    pub monotonic_ms: u64,
}

/// Platform-provided time source, independent of an execution runtime.
pub trait TaskClock {
    /// Read both clocks without blocking.
    fn now(&self) -> TaskTime;
}
impl<F: Fn() -> TaskTime> TaskClock for F {
    fn now(&self) -> TaskTime {
        self()
    }
}

/// Host-authorized description accepted before an executor starts work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskSpec {
    pub kind: String,
    pub origin: String,
    pub owner: String,
    pub parent_id: Option<TaskId>,
    pub scene_epoch: Option<u64>,
    pub cancellable: bool,
    pub message: String,
}
impl TaskSpec {
    /// Describe cancellable work owned by a host-selected executor.
    pub fn new(kind: impl Into<String>, owner: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            origin: "host".into(),
            owner: owner.into(),
            parent_id: None,
            scene_epoch: None,
            cancellable: true,
            message: String::new(),
        }
    }
}
