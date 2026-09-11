use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{atomic::AtomicBool, atomic::Ordering, Arc, Condvar, Mutex};
use std::time::Duration;

use patinae_mol::ObjectMolecule;
use patinae_plugin::prelude::{AtomChunk, AtomStreamRequest};
use patinae_plugin::wire::WireAtomPropertyChange;
use patinae_scene::LabelObjectView;
use pyo3::prelude::*;

use patinae_plugin::tasks::{TaskError, TaskId, TaskListRequest};

/// Host bridge wait slice while Python code checks for Stop requests.
const HOST_BRIDGE_WAIT_SLICE: Duration = Duration::from_millis(20);

/// Keybinding state for the Python plugin.
pub struct KeybindState {
    /// Python callbacks registered via `set_key()`, keyed by unique ID.
    pub callbacks: HashMap<u64, Py<PyAny>>,
    /// Pending registration requests: `(callback_id, key_string)`.
    pub requests: Vec<(u64, String)>,
    /// Pending unregistration requests (key strings).
    pub unreg_requests: Vec<String>,
    /// Callback IDs triggered by hotkey closures, drained each poll.
    pub triggers: Vec<u64>,
    /// Maps normalized key string to callback ID for rebind/unbind lookup.
    pub key_to_id: HashMap<String, u64>,
    /// Next available callback ID.
    pub next_id: u64,
}

impl KeybindState {
    pub fn new() -> Self {
        Self {
            callbacks: HashMap::new(),
            requests: Vec::new(),
            unreg_requests: Vec::new(),
            triggers: Vec::new(),
            key_to_id: HashMap::new(),
            next_id: 1,
        }
    }
}

/// Lightweight movie state snapshot copied from the host poll context.
#[derive(Debug, Clone, Default)]
pub struct MovieSnapshot {
    pub frame_count: usize,
    pub current_frame: usize,
    pub is_playing: bool,
    pub rock_enabled: bool,
}

/// Shared state between the host (poll) and the Python backend.
pub struct SharedState {
    /// Object names currently loaded in the viewer.
    pub names: Vec<String>,
    /// Snapshot molecules (cloned from the viewer's registry).
    pub molecules: Vec<(String, ObjectMolecule)>,
    /// Object-registry generation represented by `molecules`.
    pub molecule_generation: Option<u64>,
    /// Viewport image snapshot for reading (RGBA data, width, height).
    pub viewport_image: Option<(Vec<u8>, u32, u32)>,
    /// Lightweight identity of the viewport image represented by `viewport_image`.
    pub viewport_image_signature: Option<u64>,
    /// Movie state snapshot from the latest host poll.
    pub movie_state: MovieSnapshot,
    /// Keybinding state for `set_key()` / `unset_key()`.
    pub keybinds: KeybindState,
    /// Set by Stop to request cooperative cancellation of running Python code.
    pub interrupt_requested: Arc<AtomicBool>,
    /// Identity of the body currently running on the sequential Python worker.
    pub current_task: Option<TaskId>,
    /// Blocking bridge used by Python APIs that need host data.
    pub host_bridge: HostBridgeHandle,
}

impl SharedState {
    pub fn new(interrupt_requested: Arc<AtomicBool>) -> Self {
        Self {
            names: Vec::new(),
            molecules: Vec::new(),
            molecule_generation: None,
            viewport_image: None,
            viewport_image_signature: None,
            movie_state: MovieSnapshot::default(),
            keybinds: KeybindState::new(),
            interrupt_requested,
            current_task: None,
            host_bridge: HostBridgeHandle::new(),
        }
    }
}

/// Thread-safe handle to shared state.
pub type SharedStateHandle = Arc<Mutex<SharedState>>;

/// Blocking bridge between the Python worker and host poll loop.
#[derive(Clone)]
pub struct HostBridgeHandle {
    inner: Arc<HostBridgeInner>,
}

struct HostBridgeInner {
    state: Mutex<HostBridgeState>,
    ready: Condvar,
}

#[derive(Default)]
struct HostBridgeState {
    next_id: u64,
    requests: VecDeque<HostBridgeRequest>,
    results: HashMap<u64, HostBridgeResult>,
    pending: HashSet<u64>,
    closed: bool,
}

/// Request from Python worker to host poll.
#[derive(Clone)]
pub struct HostBridgeRequest {
    pub id: u64,
    pub task_id: Option<TaskId>,
    pub kind: HostBridgeRequestKind,
}

/// Host operation requested by the Python worker.
#[derive(Clone)]
pub enum HostBridgeRequestKind {
    Execute {
        command: String,
        quiet: bool,
    },
    GetTask {
        task_id: TaskId,
    },
    ListTasks {
        request: TaskListRequest,
    },
    CancelTask {
        task_id: TaskId,
    },
    WaitTask {
        task_id: TaskId,
        timeout_ms: Option<u64>,
    },
    SetViewportImage {
        image: Option<patinae_scene::ViewportImage>,
    },
    CountAtoms {
        selection: String,
    },
    LabelObject {
        name: String,
    },
    OpenAtomStream {
        request: AtomStreamRequest,
    },
    ReadAtomStream {
        stream_id: u64,
        max_rows: usize,
    },
    CloseAtomStream {
        stream_id: u64,
    },
    ApplyAtomPropertyChanges {
        changes: Vec<WireAtomPropertyChange>,
    },
}

/// Host operation result delivered to the Python worker.
pub enum HostBridgeValue {
    Json(serde_json::Value),
    CountAtoms(usize),
    LabelObject(Option<LabelObjectView>),
    AtomStreamOpened { stream_id: u64, total_count: usize },
    AtomChunk(AtomChunk),
    Unit,
}

/// Result stored for one bridge request.
pub type HostBridgeResult = Result<HostBridgeValue, TaskError>;

impl HostBridgeHandle {
    /// Creates an empty host bridge.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(HostBridgeInner {
                state: Mutex::new(HostBridgeState::default()),
                ready: Condvar::new(),
            }),
        }
    }

    /// Sends a request and blocks until the host completes it.
    ///
    /// # Errors
    /// Returns host errors or interruption errors.
    pub fn request(
        &self,
        kind: HostBridgeRequestKind,
        task_id: Option<TaskId>,
        interrupt_requested: &AtomicBool,
    ) -> HostBridgeResult {
        let id = {
            let mut state = self.inner.state.lock().unwrap();
            if state.closed {
                return Err(TaskError::new(
                    "executor_lost",
                    "Python host bridge disconnected",
                ));
            }
            if interrupt_requested.load(Ordering::Acquire) {
                return Err(TaskError::new("cancelled", "Python script interrupted"));
            }
            state.next_id = state.next_id.wrapping_add(1).max(1);
            let id = state.next_id;
            state.pending.insert(id);
            state
                .requests
                .push_back(HostBridgeRequest { id, task_id, kind });
            id
        };
        self.inner.ready.notify_all();

        let mut state = self.inner.state.lock().unwrap();
        loop {
            if let Some(result) = state.results.remove(&id) {
                state.pending.remove(&id);
                return result;
            }
            if state.closed || interrupt_requested.load(Ordering::Acquire) {
                state.pending.remove(&id);
                state.requests.retain(|request| request.id != id);
                return Err(if state.closed {
                    TaskError::new("executor_lost", "Python host bridge disconnected")
                } else {
                    TaskError::new("cancelled", "Python script interrupted")
                });
            }
            let (next_state, _) = self
                .inner
                .ready
                .wait_timeout(state, HOST_BRIDGE_WAIT_SLICE)
                .unwrap();
            state = next_state;
        }
    }

    /// Takes pending requests for host processing.
    pub fn take_requests(&self) -> Vec<HostBridgeRequest> {
        let mut state = self.inner.state.lock().unwrap();
        state.requests.drain(..).collect()
    }

    /// Completes a pending request.
    pub fn complete(&self, id: u64, result: HostBridgeResult) {
        let mut state = self.inner.state.lock().unwrap();
        if state.pending.contains(&id) {
            // Delivery is idempotent; a late reply must not overwrite the first.
            state.results.entry(id).or_insert(result);
        }
        self.inner.ready.notify_all();
    }

    /// Disconnects the transport and wakes every outstanding request.
    pub fn close(&self) {
        let mut state = self.inner.state.lock().unwrap();
        state.closed = true;
        state.requests.clear();
        self.inner.ready.notify_all();
    }
}

impl Default for HostBridgeHandle {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Instant;

    fn await_request(bridge: &HostBridgeHandle) -> HostBridgeRequest {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(request) = bridge.take_requests().into_iter().next() {
                return request;
            }
            assert!(
                Instant::now() < deadline,
                "bridge request was not delivered"
            );
            thread::yield_now();
        }
    }

    #[test]
    fn cancelled_bridge_wait_drops_late_replies() {
        let bridge = HostBridgeHandle::new();
        let cancelled = AtomicBool::new(false);
        thread::scope(|scope| {
            let waiter = scope.spawn(|| {
                bridge.request(
                    HostBridgeRequestKind::CountAtoms {
                        selection: "all".into(),
                    },
                    Some(TaskId::new(1, 1)),
                    &cancelled,
                )
            });
            let request = await_request(&bridge);
            cancelled.store(true, Ordering::Release);
            assert!(waiter.join().unwrap().is_err());
            bridge.complete(request.id, Ok(HostBridgeValue::CountAtoms(1)));
            let state = bridge.inner.state.lock().unwrap();
            assert!(state.pending.is_empty());
            assert!(state.results.is_empty());
        });
    }

    #[test]
    fn closing_bridge_wakes_pending_requests_and_rejects_new_work() {
        let bridge = HostBridgeHandle::new();
        let cancelled = AtomicBool::new(false);
        thread::scope(|scope| {
            let waiter = scope.spawn(|| {
                bridge.request(
                    HostBridgeRequestKind::CountAtoms {
                        selection: "all".into(),
                    },
                    None,
                    &cancelled,
                )
            });
            await_request(&bridge);
            bridge.close();
            assert!(
                matches!(waiter.join().unwrap(), Err(error) if error.message.contains("disconnected"))
            );
        });
        assert!(bridge
            .request(
                HostBridgeRequestKind::CountAtoms {
                    selection: "all".into(),
                },
                None,
                &cancelled
            )
            .is_err());
        assert!(bridge.take_requests().is_empty());
    }

    #[test]
    fn bridge_preserves_first_reply_and_task_identity() {
        let bridge = HostBridgeHandle::new();
        let task_id = TaskId::new(2, 3);
        thread::scope(|scope| {
            let waiter = scope.spawn(|| {
                bridge.request(
                    HostBridgeRequestKind::Execute {
                        command: "color red".into(),
                        quiet: false,
                    },
                    Some(task_id),
                    &AtomicBool::new(false),
                )
            });
            let request = await_request(&bridge);
            assert_eq!(request.task_id, Some(task_id));
            bridge.complete(
                request.id,
                Err(TaskError::new("apply_failed", "apply failed")),
            );
            bridge.complete(request.id, Ok(HostBridgeValue::Unit));
            assert!(matches!(waiter.join().unwrap(), Err(error) if error.code == "apply_failed"));
        });
        assert!(bridge.inner.state.lock().unwrap().results.is_empty());
    }
}
