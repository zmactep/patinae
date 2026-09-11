//! Python Worker Thread
//!
//! Runs the Python interpreter in a dedicated background thread so that
//! `eval()` / `exec_file()` never block the main (UI) thread.
//!
//! All callers submit [`WorkItem`]s via the [`WorkerHandle`] and results
//! are returned through an `mpsc` channel, drained in `PythonHandler::poll()`.

use patinae_plugin::tasks::TaskId;
use std::collections::HashMap;
use std::ffi::CString;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;

use pyo3::prelude::*;

use crate::backend::PluginBackend;
use crate::engine::PythonEngine;
use crate::shared::SharedStateHandle;

// =============================================================================
// StreamingWriter — real-time stdout/stderr forwarding
// =============================================================================

/// A Python file-like object that sends each `write()` call through an mpsc
/// channel, enabling real-time output streaming from long-running scripts.
#[pyclass]
struct StreamingWriter {
    tx: SyncSender<WorkResult>,
    origin: WorkOrigin,
    task_id: TaskId,
    chunk_bytes: usize,
    cancellation: Arc<AtomicBool>,
    /// Line buffer — accumulates text until a newline is seen, then sends
    /// complete lines. This avoids splitting `print("x")` into separate
    /// `"x"` and `"\n"` entries (which would render as extra blank lines).
    buffer: String,
}

#[pymethods]
impl StreamingWriter {
    fn write(&mut self, py: Python<'_>, text: &str) -> PyResult<usize> {
        let mut remaining = text;
        while !remaining.is_empty() {
            check_interrupt_requested(&self.cancellation)?;
            let capacity = self.chunk_bytes.saturating_sub(self.buffer.len());
            let mut end = capacity.min(remaining.len());
            while !remaining.is_char_boundary(end) {
                end -= 1;
            }
            if end == 0 {
                self.flush(py)?;
                continue;
            }
            self.buffer.push_str(&remaining[..end]);
            remaining = &remaining[end..];
            while let Some(pos) = self.buffer.find('\n') {
                let line = self.buffer.drain(..=pos).collect();
                self.send(py, line)?;
            }
            if self.buffer.len() == self.chunk_bytes {
                self.flush(py)?;
            }
        }
        Ok(text.chars().count())
    }

    fn flush(&mut self, py: Python<'_>) -> PyResult<()> {
        if !self.buffer.is_empty() {
            let text = std::mem::take(&mut self.buffer);
            self.send(py, text)?;
        }
        Ok(())
    }
}

impl StreamingWriter {
    fn send(&self, py: Python<'_>, text: String) -> PyResult<()> {
        let event = WorkResult {
            task_id: Some(self.task_id),
            origin: self.origin,
            payload: WorkResultPayload::Output(text),
        };
        // The host may need the GIL while polling callbacks. Waiting for bounded
        // transport capacity while holding it would deadlock both threads.
        py.detach(|| self.tx.send(event)).map_err(|_| {
            pyo3::exceptions::PyRuntimeError::new_err("Python output transport disconnected")
        })
    }
}

#[pyclass]
struct CancellationToken {
    interrupt_requested: Arc<AtomicBool>,
}

#[pymethods]
impl CancellationToken {
    fn check(&self) -> PyResult<()> {
        check_interrupt_requested(&self.interrupt_requested)
    }
}

fn check_interrupt_requested(interrupt_requested: &AtomicBool) -> PyResult<()> {
    if interrupt_requested.load(Ordering::Acquire) {
        Err(pyo3::exceptions::PyRuntimeError::new_err(
            "Python script interrupted",
        ))
    } else {
        Ok(())
    }
}

// =============================================================================
// Types
// =============================================================================

/// Identifies the origin of a work request (for routing results).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkOrigin {
    /// From the `python` / `/` command line.
    Command,
    /// From `run script.py`.
    Script,
    /// From the scripting panel.
    Panel,
    /// Backend installation (one-time setup).
    Setup,
}

/// A unit of work to send to the Python worker thread.
pub enum WorkItem {
    /// Evaluate a code string.
    Eval { code: String, origin: WorkOrigin },
    /// Execute a file by path.
    ExecFile { path: String, origin: WorkOrigin },
    /// Install the PluginBackend into `sys._patinae_backend`.
    InstallBackend { shared: SharedStateHandle },
    /// Invoke a Python callable bound to a key (no arguments).
    InvokeKeybindCallback {
        callback: Py<PyAny>,
        origin: WorkOrigin,
    },
}

/// Result sent back from the worker thread.
pub struct WorkResult {
    pub task_id: Option<TaskId>,
    pub origin: WorkOrigin,
    pub payload: WorkResultPayload,
}

/// Typed worker event payload.
pub enum WorkResultPayload {
    Started,
    /// A stdout/stderr chunk emitted by running Python code.
    Output(String),
    /// A work item completed, with an optional error.
    Finished(Result<(), String>),
    /// The body stopped after observing its own task's cancellation signal.
    Cancelled,
    /// The executor failed before it could acknowledge its external effects.
    ExecutorLost(String),
    /// One-time backend setup result.
    Setup(Result<String, String>),
}

// =============================================================================
// WorkerHandle
// =============================================================================

/// Transport handle; cancellation tokens belong to individual accepted tasks.
#[derive(Clone)]
pub struct WorkerHandle {
    tx: Sender<WorkerRequest>,
    tokens: Arc<Mutex<HashMap<TaskId, Arc<AtomicBool>>>>,
    config: Arc<Mutex<patinae_plugin::tasks::TaskConfig>>,
}

struct WorkerRequest {
    task_id: Option<TaskId>,
    item: WorkItem,
    cancellation: Arc<AtomicBool>,
}

impl WorkerHandle {
    pub fn config(&self) -> patinae_plugin::tasks::TaskConfig {
        self.config.lock().unwrap().clone()
    }

    pub fn configure(&self, config: patinae_plugin::tasks::TaskConfig) {
        *self.config.lock().unwrap() = config;
    }
    pub fn submit(&self, task_id: TaskId, item: WorkItem) -> Result<(), String> {
        let cancellation = Arc::new(AtomicBool::new(false));
        {
            let mut tokens = self.tokens.lock().unwrap();
            if tokens.contains_key(&task_id) {
                return Err("Python task was already submitted".into());
            }
            tokens.insert(task_id, cancellation.clone());
        }
        if self
            .tx
            .send(WorkerRequest {
                task_id: Some(task_id),
                item,
                cancellation,
            })
            .is_err()
        {
            self.tokens.lock().unwrap().remove(&task_id);
            return Err("Python executor disconnected".into());
        }
        Ok(())
    }

    pub fn install(&self, shared: SharedStateHandle) -> Result<(), String> {
        self.tx
            .send(WorkerRequest {
                task_id: None,
                item: WorkItem::InstallBackend { shared },
                cancellation: Arc::new(AtomicBool::new(false)),
            })
            .map_err(|_| "Python executor disconnected".into())
    }

    pub fn cancel(&self, task_id: TaskId) {
        if let Some(token) = self.tokens.lock().unwrap().get(&task_id) {
            token.store(true, Ordering::Release);
        }
    }

    pub fn cancel_all(&self) {
        for token in self.tokens.lock().unwrap().values() {
            token.store(true, Ordering::Release);
        }
    }

    /// Drains transport handles after the worker result channel disconnects.
    pub fn take_disconnected_tasks(&self) -> Vec<TaskId> {
        self.tokens
            .lock()
            .unwrap()
            .drain()
            .map(|(id, _)| id)
            .collect()
    }
}

pub fn spawn_worker() -> (WorkerHandle, Receiver<WorkResult>) {
    let (work_tx, work_rx) = mpsc::channel::<WorkerRequest>();
    let config = Arc::new(Mutex::new(patinae_plugin::tasks::TaskConfig::default()));
    let (result_tx, result_rx) =
        mpsc::sync_channel::<WorkResult>(config.lock().unwrap().batch_size);
    let tokens = Arc::new(Mutex::new(HashMap::new()));
    let worker_tokens = tokens.clone();
    let worker_config = config.clone();
    thread::Builder::new()
        .name("python-worker".into())
        .spawn(move || worker_loop(work_rx, result_tx, worker_tokens, worker_config))
        .expect("Failed to spawn Python worker thread");
    (
        WorkerHandle {
            tx: work_tx,
            tokens,
            config,
        },
        result_rx,
    )
}

fn worker_loop(
    work_rx: Receiver<WorkerRequest>,
    result_tx: SyncSender<WorkResult>,
    tokens: Arc<Mutex<HashMap<TaskId, Arc<AtomicBool>>>>,
    config: Arc<Mutex<patinae_plugin::tasks::TaskConfig>>,
) {
    let mut engine = PythonEngine::new();
    let mut shared_state: Option<SharedStateHandle> = None;
    for request in work_rx {
        let WorkerRequest {
            task_id,
            item,
            cancellation,
        } = request;
        if let WorkItem::InstallBackend { shared } = item {
            let result = install_backend(&mut engine, &shared)
                .map_err(|error| bound_error(error, &config.lock().unwrap()));
            shared_state = Some(shared);
            let _ = result_tx.send(WorkResult {
                task_id: None,
                origin: WorkOrigin::Setup,
                payload: WorkResultPayload::Setup(result),
            });
            continue;
        }
        let Some(task_id) = task_id else { continue };
        if let Some(shared) = &shared_state {
            let mut state = shared.lock().unwrap();
            state.current_task = Some(task_id);
            state.interrupt_requested = cancellation.clone();
        }
        let origin = match &item {
            WorkItem::Eval { origin, .. }
            | WorkItem::ExecFile { origin, .. }
            | WorkItem::InvokeKeybindCallback { origin, .. } => *origin,
            _ => WorkOrigin::Setup,
        };
        let _ = result_tx.send(WorkResult {
            task_id: Some(task_id),
            origin,
            payload: WorkResultPayload::Started,
        });
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            check_interrupt_requested(&cancellation).map_err(|error| error.to_string())?;
            let chunk_bytes = {
                let config = config.lock().unwrap();
                // At most one snapshot's worth of output crosses the wire in
                // a poll batch. Four bytes always fit one UTF-8 scalar value.
                (config.max_snapshot_bytes / config.batch_size.max(1)).max(4)
            };
            match item {
                WorkItem::Eval { code, .. } => execute_streaming(
                    &mut engine,
                    PythonBody::Code(&code),
                    origin,
                    task_id,
                    &result_tx,
                    &cancellation,
                    chunk_bytes,
                ),
                WorkItem::ExecFile { path, .. } => {
                    let code = read_script_file(&ProcessScriptReader, &path)?;
                    execute_streaming(
                        &mut engine,
                        PythonBody::Code(&code),
                        origin,
                        task_id,
                        &result_tx,
                        &cancellation,
                        chunk_bytes,
                    )
                }
                WorkItem::InvokeKeybindCallback { callback, .. } => execute_streaming(
                    &mut engine,
                    PythonBody::Callback(callback),
                    origin,
                    task_id,
                    &result_tx,
                    &cancellation,
                    chunk_bytes,
                ),
                _ => Ok(()),
            }
        }));
        let executor_lost = result.is_err();
        let payload = match result {
            Ok(_) if cancellation.load(Ordering::Acquire) => WorkResultPayload::Cancelled,
            Ok(result) => WorkResultPayload::Finished(
                result.map_err(|error| bound_error(error, &config.lock().unwrap())),
            ),
            Err(_) => WorkResultPayload::ExecutorLost("Python executor panicked".into()),
        };
        if let Some(shared) = &shared_state {
            shared.lock().unwrap().current_task = None;
        }
        tokens.lock().unwrap().remove(&task_id);
        let _ = result_tx.send(WorkResult {
            task_id: Some(task_id),
            origin,
            payload,
        });
        if executor_lost {
            // Remaining handles are drained by the host when this channel
            // closes. Do not continue using a potentially inconsistent engine.
            break;
        }
    }
}

fn bound_error(mut error: String, config: &patinae_plugin::tasks::TaskConfig) -> String {
    // Error text can contain arbitrarily large user values. Keep a useful
    // prefix before queueing it, so completion itself cannot overflow the wire.
    let limit = (config.max_snapshot_bytes / config.batch_size.max(1)).max(4);
    if error.len() > limit {
        let mut end = limit;
        while !error.is_char_boundary(end) {
            end -= 1;
        }
        error.truncate(end);
        error.push_str(" [truncated]");
    }
    error
}

trait ScriptReader {
    fn read_to_string(&self, path: &str) -> io::Result<String>;
}

struct ProcessScriptReader;

impl ScriptReader for ProcessScriptReader {
    fn read_to_string(&self, path: &str) -> io::Result<String> {
        std::fs::read_to_string(path)
    }
}

fn read_script_file(reader: &impl ScriptReader, path: &str) -> Result<String, String> {
    reader
        .read_to_string(path)
        .map_err(|e| format!("failed to read {}: {}", path, e))
}

fn install_interrupt_trace(
    py: Python<'_>,
    interrupt_requested: &Arc<AtomicBool>,
) -> Result<(), String> {
    let token = Py::new(
        py,
        CancellationToken {
            interrupt_requested: interrupt_requested.clone(),
        },
    )
    .map_err(|e| e.to_string())?;

    // Each worker/task owns the trace closure's globals. Installing another
    // task's trace must never replace this task's cancellation token.
    let globals = pyo3::types::PyDict::new(py);
    globals
        .set_item("__patinae_interrupt_token", token.bind(py))
        .map_err(|e| e.to_string())?;

    let trace_code = CString::new(
        r#"def __patinae_interrupt_trace(frame, event, arg):
    __patinae_interrupt_token.check()
    return __patinae_interrupt_trace
"#,
    )
    .map_err(|e| e.to_string())?;
    py.run(trace_code.as_c_str(), Some(&globals), None)
        .map_err(|e| e.to_string())?;

    let trace_fn = globals
        .get_item("__patinae_interrupt_trace")
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "failed to install Python interrupt trace".to_string())?;
    py.import("sys")
        .map_err(|e| e.to_string())?
        .call_method1("settrace", (trace_fn,))
        .map_err(|e| e.to_string())?;

    Ok(())
}

fn clear_interrupt_trace(py: Python<'_>) {
    if let Ok(sys) = py.import("sys") {
        let _ = sys.call_method1("settrace", (py.None(),));
    }
}

/// Run Python code with real-time output streaming via [`StreamingWriter`].
///
/// Output is sent through `result_tx` as it is produced. If the code raises
/// an exception, the error is sent as a final `Err` result.
enum PythonBody<'a> {
    Code(&'a str),
    Callback(Py<PyAny>),
}

fn execute_streaming(
    engine: &mut PythonEngine,
    body: PythonBody<'_>,
    origin: WorkOrigin,
    task_id: TaskId,
    result_tx: &SyncSender<WorkResult>,
    interrupt_requested: &Arc<AtomicBool>,
    chunk_bytes: usize,
) -> Result<(), String> {
    engine.ensure_init()?;
    Python::attach(|py| -> Result<(), String> {
        let stdout_w = Py::new(
            py,
            StreamingWriter {
                tx: result_tx.clone(),
                origin,
                task_id,
                chunk_bytes,
                cancellation: interrupt_requested.clone(),
                buffer: String::new(),
            },
        )
        .map_err(|e| e.to_string())?;

        let stderr_w = Py::new(
            py,
            StreamingWriter {
                tx: result_tx.clone(),
                origin,
                task_id,
                chunk_bytes,
                cancellation: interrupt_requested.clone(),
                buffer: String::new(),
            },
        )
        .map_err(|e| e.to_string())?;

        install_interrupt_trace(py, interrupt_requested)?;
        let result = match body {
            PythonBody::Code(code) => {
                engine.eval_with_writers(code, stdout_w.bind(py), stderr_w.bind(py))
            }
            PythonBody::Callback(callback) => {
                let sys = py.import("sys").map_err(|error| error.to_string())?;
                let old_stdout = sys.getattr("stdout").map_err(|error| error.to_string())?;
                let old_stderr = sys.getattr("stderr").map_err(|error| error.to_string())?;
                sys.setattr("stdout", stdout_w.bind(py))
                    .map_err(|error| error.to_string())?;
                sys.setattr("stderr", stderr_w.bind(py))
                    .map_err(|error| error.to_string())?;
                let result = callback
                    .call0(py)
                    .map(|_| ())
                    .map_err(|error| error.to_string());
                let _ = sys.setattr("stdout", old_stdout);
                let _ = sys.setattr("stderr", old_stderr);
                result
            }
        };

        // Flush any remaining buffered text (e.g. output without trailing newline)
        let _ = stdout_w.borrow_mut(py).flush(py);
        let _ = stderr_w.borrow_mut(py).flush(py);
        clear_interrupt_trace(py);

        result
    })
}

/// Install the PluginBackend into `sys._patinae_backend` and auto-import `cmd`.
fn install_backend(
    engine: &mut PythonEngine,
    shared: &SharedStateHandle,
) -> Result<String, String> {
    engine.ensure_init()?;

    let py_backend = Python::attach(|py| -> Result<Py<PyAny>, String> {
        let backend = PluginBackend::new(shared.clone());
        let py_obj = Py::new(py, backend).map_err(|e| e.to_string())?;
        Ok(py_obj.into_any())
    })?;

    engine.set_backend(py_backend)?;

    // Auto-import cmd and stored into __main__ so it's available in the REPL
    match engine.eval("from patinae import cmd; from patinae import stored") {
        Ok(_) => Ok("backend installed, cmd auto-imported".to_string()),
        Err(e) => {
            log::warn!(
                "Python plugin: backend installed but auto-import cmd failed: {}",
                e
            );
            Ok("backend installed (cmd auto-import failed)".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[derive(Clone, Copy)]
    enum FakeRead {
        Ok(&'static str),
        Err(io::ErrorKind),
    }

    struct FakeScriptReader {
        result: FakeRead,
    }

    impl ScriptReader for FakeScriptReader {
        fn read_to_string(&self, _path: &str) -> io::Result<String> {
            match self.result {
                FakeRead::Ok(code) => Ok(code.to_string()),
                FakeRead::Err(kind) => Err(io::Error::new(kind, "fake read failure")),
            }
        }
    }

    #[test]
    fn script_reader_returns_code_without_process_fs() {
        let reader = FakeScriptReader {
            result: FakeRead::Ok("print('ok')"),
        };

        let code = read_script_file(&reader, "/fake/script.py").expect("script should read");

        assert_eq!(code, "print('ok')");
    }

    #[test]
    fn script_reader_formats_read_failures() {
        let reader = FakeScriptReader {
            result: FakeRead::Err(io::ErrorKind::NotFound),
        };

        let err = read_script_file(&reader, "/fake/missing.py").expect_err("read should fail");

        assert!(
            err.contains("failed to read /fake/missing.py"),
            "unexpected error: {err}"
        );
        assert!(err.contains("fake read failure"), "unexpected error: {err}");
    }

    #[test]
    fn request_interrupt_stops_running_python_code() {
        let (worker, rx) = spawn_worker();
        let task_id = TaskId::new(1, 1);
        worker.submit(task_id, WorkItem::Eval {
            code: "def cancellable_function():\n    print('started')\n    while True:\n        pass\ncancellable_function()\n".to_string(),
            origin: WorkOrigin::Panel,
        }).unwrap();

        let startup_deadline = Instant::now() + Duration::from_secs(5);
        let mut saw_started = false;
        while !saw_started {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(result) => match result.payload {
                    WorkResultPayload::Output(output) => {
                        saw_started = output.contains("started");
                    }
                    WorkResultPayload::Finished(result) => {
                        panic!("Python worker finished before interrupt: {result:?}");
                    }
                    WorkResultPayload::Setup(_) | WorkResultPayload::Started => {}
                    WorkResultPayload::ExecutorLost(error) => {
                        panic!("Python executor failed: {error}")
                    }
                    WorkResultPayload::Cancelled => panic!("task cancelled without a request"),
                },
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    assert!(
                        Instant::now() < startup_deadline,
                        "Python worker did not start executing before timeout"
                    );
                }
                Err(e) => panic!("Python worker result channel closed: {e}"),
            }
        }

        // A cancellation addressed to queued work must not interrupt this body.
        let queued_id = TaskId::new(1, 2);
        worker
            .submit(
                queued_id,
                WorkItem::Eval {
                    code: "raise AssertionError('cancelled queued body ran')".into(),
                    origin: WorkOrigin::Command,
                },
            )
            .unwrap();
        worker.cancel(queued_id);
        assert!(!worker.tokens.lock().unwrap()[&task_id].load(Ordering::Acquire));
        assert!(worker.tokens.lock().unwrap()[&queued_id].load(Ordering::Acquire));
        worker.cancel(task_id);

        let finished_deadline = Instant::now() + Duration::from_secs(5);
        let mut cancellation_acknowledged = false;
        while Instant::now() < finished_deadline {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(result) => {
                    if let WorkResultPayload::Cancelled = result.payload {
                        cancellation_acknowledged = true;
                        break;
                    }
                    if let WorkResultPayload::Finished(result) = result.payload {
                        panic!("cancelled task sent an ordinary completion: {result:?}");
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(e) => panic!("Python worker result channel closed: {e}"),
            }
        }

        assert!(
            cancellation_acknowledged,
            "Python code did not acknowledge cancellation"
        );
        loop {
            let result = rx
                .recv_timeout(Duration::from_secs(5))
                .expect("queued cancellation completion");
            if result.task_id == Some(queued_id) {
                if let WorkResultPayload::Cancelled = result.payload {
                    break;
                }
                if let WorkResultPayload::Finished(result) = result.payload {
                    panic!("queued cancellation sent an ordinary completion: {result:?}");
                }
            }
        }
        assert!(worker.tokens.lock().unwrap().is_empty());

        let callback_id = TaskId::new(1, 3);
        let callback = Python::attach(|py| {
            py.eval(c"lambda: print('callback output')", None, None)
                .unwrap()
                .unbind()
        });
        worker
            .submit(
                callback_id,
                WorkItem::InvokeKeybindCallback {
                    callback,
                    origin: WorkOrigin::Command,
                },
            )
            .unwrap();
        let mut callback_output = String::new();
        loop {
            let result = rx
                .recv_timeout(Duration::from_secs(5))
                .expect("callback completion");
            assert_eq!(result.task_id, Some(callback_id));
            match result.payload {
                WorkResultPayload::Output(output) => callback_output.push_str(&output),
                WorkResultPayload::Finished(result) => {
                    result.unwrap();
                    break;
                }
                WorkResultPayload::ExecutorLost(error) => {
                    panic!("callback executor failed: {error}")
                }
                _ => {}
            }
        }
        assert_eq!(callback_output.trim(), "callback output");

        let cancelled_callback_id = TaskId::new(1, 4);
        let callback = Python::attach(|py| {
            py.eval(
                c"lambda: exec(\"print('callback started')\\nwhile True:\\n    pass\")",
                None,
                None,
            )
            .unwrap()
            .unbind()
        });
        worker
            .submit(
                cancelled_callback_id,
                WorkItem::InvokeKeybindCallback {
                    callback,
                    origin: WorkOrigin::Command,
                },
            )
            .unwrap();
        loop {
            let result = rx
                .recv_timeout(Duration::from_secs(5))
                .expect("callback cancellation");
            assert_eq!(result.task_id, Some(cancelled_callback_id));
            match result.payload {
                WorkResultPayload::Output(_) => worker.cancel(cancelled_callback_id),
                WorkResultPayload::Cancelled => break,
                WorkResultPayload::Finished(result) => {
                    panic!("callback cancellation sent ordinary completion: {result:?}")
                }
                WorkResultPayload::ExecutorLost(error) => {
                    panic!("callback executor failed: {error}")
                }
                _ => {}
            }
        }

        let config = worker.config();
        let chunk_bytes = config.max_snapshot_bytes / config.batch_size;
        let output_id = TaskId::new(1, 5);
        // A single write exceeds both the snapshot budget and bounded channel
        // capacity, including multi-byte characters at chunk boundaries.
        let repetitions = config.max_snapshot_bytes;
        worker
            .submit(
                output_id,
                WorkItem::Eval {
                    code: format!("print('Ж🙂' * {repetitions})"),
                    origin: WorkOrigin::Command,
                },
            )
            .unwrap();
        let mut output = String::new();
        loop {
            let result = rx
                .recv_timeout(Duration::from_secs(5))
                .expect("large output completion");
            assert_eq!(result.task_id, Some(output_id));
            match result.payload {
                WorkResultPayload::Output(chunk) => {
                    assert!(chunk.len() <= chunk_bytes);
                    output.push_str(&chunk);
                    // Host callbacks can still acquire the GIL while the
                    // worker waits for space in the bounded output channel.
                    Python::attach(|py| {
                        py.eval(c"1 + 1", None, None).unwrap();
                    });
                }
                WorkResultPayload::Finished(result) => {
                    result.unwrap();
                    break;
                }
                WorkResultPayload::ExecutorLost(error) => panic!("output executor failed: {error}"),
                _ => {}
            }
        }
        assert_eq!(output, format!("{}\n", "Ж🙂".repeat(repetitions)));
    }

    #[test]
    fn duplicate_delivery_does_not_replace_cancellation_token() {
        let (tx, rx) = mpsc::channel();
        let worker = WorkerHandle {
            tx,
            tokens: Arc::new(Mutex::new(HashMap::new())),
            config: Arc::new(Mutex::new(patinae_plugin::tasks::TaskConfig::default())),
        };
        let task_id = TaskId::new(3, 1);
        let item = || WorkItem::Eval {
            code: "pass".into(),
            origin: WorkOrigin::Command,
        };
        worker.submit(task_id, item()).unwrap();
        worker.cancel(task_id);
        assert!(worker.submit(task_id, item()).is_err());
        assert!(rx.recv().unwrap().cancellation.load(Ordering::Acquire));
        assert!(rx.try_recv().is_err());
    }
}
