use std::collections::{HashMap, VecDeque};
use std::sync::{atomic::AtomicBool, atomic::Ordering, Arc, Condvar, Mutex};
use std::time::Duration;

use patinae_mol::ObjectMolecule;
use patinae_plugin::prelude::{AtomChunk, AtomStreamRequest};
use patinae_plugin::wire::WireAtomPropertyChange;
use patinae_scene::LabelObjectView;
use pyo3::prelude::*;

use crate::atom_ops::AlterBuffer;

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
    /// Queued commands to execute on the host side.
    pub cmd_queue: Vec<(String, bool)>,
    /// Viewport image snapshot for reading (RGBA data, width, height).
    pub viewport_image: Option<(Vec<u8>, u32, u32)>,
    /// Lightweight identity of the viewport image represented by `viewport_image`.
    pub viewport_image_signature: Option<u64>,
    /// Movie state snapshot from the latest host poll.
    pub movie_state: MovieSnapshot,
    /// Pending viewport image to set: `Some(Some(...))` = set, `Some(None)` = clear.
    pub set_image_queue: Option<Option<(Vec<u8>, u32, u32)>>,
    /// Shared buffer for atom mutations from `alter()` — drained by the handler's mutation queue.
    pub alter_buffer: AlterBuffer,
    /// Keybinding state for `set_key()` / `unset_key()`.
    pub keybinds: KeybindState,
    /// Set by Stop to request cooperative cancellation of running Python code.
    pub interrupt_requested: Arc<AtomicBool>,
    /// Blocking bridge used by Python APIs that need host data.
    pub host_bridge: HostBridgeHandle,
}

impl SharedState {
    pub fn new(alter_buffer: AlterBuffer, interrupt_requested: Arc<AtomicBool>) -> Self {
        Self {
            names: Vec::new(),
            molecules: Vec::new(),
            molecule_generation: None,
            cmd_queue: Vec::new(),
            viewport_image: None,
            viewport_image_signature: None,
            movie_state: MovieSnapshot::default(),
            set_image_queue: None,
            alter_buffer,
            keybinds: KeybindState::new(),
            interrupt_requested,
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
}

/// Request from Python worker to host poll.
#[derive(Debug, Clone)]
pub struct HostBridgeRequest {
    pub id: u64,
    pub kind: HostBridgeRequestKind,
}

/// Host operation requested by the Python worker.
#[derive(Debug, Clone)]
pub enum HostBridgeRequestKind {
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
    CountAtoms(usize),
    LabelObject(Option<LabelObjectView>),
    AtomStreamOpened { stream_id: u64, total_count: usize },
    AtomChunk(AtomChunk),
    Unit,
}

/// Result stored for one bridge request.
pub type HostBridgeResult = Result<HostBridgeValue, String>;

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
        interrupt_requested: &AtomicBool,
    ) -> HostBridgeResult {
        let id = {
            let mut state = self.inner.state.lock().unwrap();
            state.next_id = state.next_id.wrapping_add(1).max(1);
            let id = state.next_id;
            state.requests.push_back(HostBridgeRequest { id, kind });
            id
        };
        self.inner.ready.notify_all();

        let mut state = self.inner.state.lock().unwrap();
        loop {
            if let Some(result) = state.results.remove(&id) {
                return result;
            }
            if interrupt_requested.load(Ordering::Acquire) {
                state.results.remove(&id);
                return Err("Python script interrupted".to_string());
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
        state.results.insert(id, result);
        self.inner.ready.notify_all();
    }
}

impl Default for HostBridgeHandle {
    fn default() -> Self {
        Self::new()
    }
}
