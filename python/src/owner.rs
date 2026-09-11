//! Session ownership and platform executors for standalone Python.

use patinae_cmd::tasks::{
    TaskConfig, TaskEffects, TaskId, TaskOutcome, TaskRunner, TaskSpec, TaskStartError, TaskTime,
};
use patinae_cmd::{
    AsyncCommandAcceptance, AsyncCommandRequest, CommandExecutor, CommandReply, FetchFormatCode,
    PluginTaskRequest,
};
use patinae_mol::ObjectMolecule;
use patinae_scene::{Session, SessionAdapter};
use pyo3::{prelude::*, types::PyDict};
use std::{
    collections::BTreeMap,
    hash::{BuildHasher, Hasher},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc, Arc, Weak,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use crate::backend::StandaloneBackend;
use patinae_cmd::loading::{infer_format, LoadOptions, LoadedData};

pub(crate) type OwnerCall = Box<dyn FnOnce(&mut SessionOwner) + Send>;
pub(crate) enum Message {
    Call(OwnerCall),
    Shutdown,
}

/// Transport only; task state belongs to the owner thread.
pub(crate) struct Client {
    pub tx: mpsc::Sender<Message>,
}
impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.tx.send(Message::Shutdown);
    }
}

struct PythonWork {
    id: TaskId,
    request: PluginTaskRequest,
    cancel: Arc<AtomicBool>,
}

pub(crate) struct SessionOwner {
    pub session: Session,
    pub executor: CommandExecutor,
    pub tasks: TaskRunner,
    pub needs_redraw: bool,
    pub revision: u64,
    client: Weak<Client>,
    runtime: tokio::runtime::Runtime,
    network: BTreeMap<TaskId, tokio::task::AbortHandle>,
    scripts: BTreeMap<TaskId, patinae_cmd::script::ScriptExecution>,
    script_lineage: Vec<String>,
    python_tx: mpsc::Sender<PythonWork>,
    python_cancel: BTreeMap<TaskId, Arc<AtomicBool>>,
}

pub(crate) fn start() -> Result<Arc<Client>, String> {
    let (tx, rx) = mpsc::channel();
    let client = Arc::new(Client { tx });
    let weak = Arc::downgrade(&client);
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("patinae-session".into())
        .spawn(move || {
            let mut owner = match SessionOwner::new(weak) {
                Ok(owner) => {
                    let _ = ready_tx.send(Ok(()));
                    owner
                }
                Err(error) => {
                    let _ = ready_tx.send(Err(error));
                    return;
                }
            };
            // The owner wakes independently of Python calls and GUI/render frames.
            const OWNER_TICK: Duration = Duration::from_millis(10);
            loop {
                match rx.recv_timeout(OWNER_TICK) {
                    Ok(Message::Call(call)) => call(&mut owner),
                    Ok(Message::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
                owner.pump();
            }
            for handle in owner.network.values() {
                handle.abort();
            }
            for cancel in owner.python_cancel.values() {
                cancel.store(true, Ordering::Release);
            }
        })
        .map_err(|error| error.to_string())?;
    ready_rx.recv().map_err(|error| error.to_string())??;
    Ok(client)
}

impl SessionOwner {
    fn new(client: Weak<Client>) -> Result<Self, String> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|error| error.to_string())?;
        let instance = (u128::from(
            std::collections::hash_map::RandomState::new()
                .build_hasher()
                .finish(),
        ) << 64)
            | u128::from(
                std::collections::hash_map::RandomState::new()
                    .build_hasher()
                    .finish(),
            );
        let started = Instant::now();
        let tasks = TaskRunner::new(
            instance,
            TaskConfig::default(),
            Box::new(move || TaskTime {
                unix_ms: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis()
                    .min(u128::from(u64::MAX)) as u64,
                monotonic_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            }),
        );
        let (python_tx, python_rx) = mpsc::channel();
        let python_client = client.clone();
        std::thread::Builder::new()
            .name("patinae-python".into())
            .spawn(move || python_loop(python_client, python_rx))
            .map_err(|error| error.to_string())?;
        let mut executor = CommandExecutor::new();
        executor.registry_mut().register(PythonCommand);
        executor.register_script_handler(
            "py",
            Arc::new(|path| {
                Ok(AsyncCommandRequest::Plugin(PluginTaskRequest::new(
                    "python",
                    serde_json::json!({"path":path}),
                )))
            }),
        );
        Ok(Self {
            session: Session::new(),
            executor,
            tasks,
            needs_redraw: false,
            revision: 0,
            client,
            runtime,
            network: BTreeMap::new(),
            scripts: BTreeMap::new(),
            script_lineage: Vec::new(),
            python_tx,
            python_cancel: BTreeMap::new(),
        })
    }

    pub fn execute(&mut self, command: &str, quiet: bool, parent: Option<TaskId>) -> CommandReply {
        let parent_owner = parent.and_then(|id| self.tasks.owner(id).ok());
        if let Some(id) = parent {
            if let Err(error) = self.tasks.can_apply_effect(
                id,
                parent_owner.as_deref().unwrap_or("python"),
                self.session.task_epoch(),
            ) {
                return CommandReply {
                    result: Err(error.to_string()),
                    messages: Vec::new(),
                    task_ids: Vec::new(),
                };
            }
        }
        let before = self.session.mutation_revision();
        let tasks = &self.tasks;
        let epoch = self.session.task_epoch();
        let mut accepted = Vec::new();
        let lineage = &self.script_lineage;
        let mut sink = |request: AsyncCommandRequest| {
            let kind = match &request {
                AsyncCommandRequest::Fetch(_) => "fetch",
                AsyncCommandRequest::LoadUrl { .. } => "load",
                AsyncCommandRequest::LoadFile { .. } => return AsyncCommandAcceptance::Unsupported,
                AsyncCommandRequest::RunScript { .. } => "script",
                AsyncCommandRequest::Plugin(request) => &request.kind,
            };
            if matches!(&request, AsyncCommandRequest::Plugin(request) if request.kind != "python")
            {
                return AsyncCommandAcceptance::Rejected(TaskStartError::ExecutorUnavailable);
            }
            let owner = if matches!(&request, AsyncCommandRequest::Plugin(_)) {
                "python"
            } else {
                "standalone"
            };
            let mut spec = TaskSpec::new(kind, owner);
            spec.parent_id = parent;
            // Scripts may deliberately replace the scene; their child loads
            // remain bound to the scene that existed when they were admitted.
            spec.scene_epoch = matches!(
                &request,
                AsyncCommandRequest::Fetch(_) | AsyncCommandRequest::LoadUrl { .. }
            )
            .then_some(epoch);
            spec.origin = "python.standalone".into();
            let script = if let AsyncCommandRequest::RunScript { path } = &request {
                let path = patinae_cmd::commands::io::expand_path(path);
                let identity = path
                    .canonicalize()
                    .unwrap_or(path.clone())
                    .to_string_lossy()
                    .into_owned();
                match patinae_cmd::script::ScriptExecution::new(
                    path.to_string_lossy().into_owned(),
                    identity,
                    lineage,
                ) {
                    Ok(script) => Some(script),
                    Err(error) => return AsyncCommandAcceptance::Rejected(error),
                }
            } else {
                None
            };
            match tasks.admit(spec) {
                Ok(id) => {
                    accepted.push((id, request, script));
                    AsyncCommandAcceptance::Accepted(id)
                }
                Err(error) => AsyncCommandAcceptance::Rejected(error),
            }
        };
        let reply: CommandReply = {
            let mut adapter = SessionAdapter {
                session: &mut self.session,
                render_context: None,
                default_size: (1024, 768),
                needs_redraw: &mut self.needs_redraw,
            };
            self.executor
                .execute_captured(&mut adapter, command, quiet, Some(&mut sink))
                .into()
        };
        self.revision = self.revision.wrapping_add(1);
        if let Some(id) = parent {
            for message in &reply.messages {
                let _ = self.tasks.output(
                    id,
                    parent_owner.as_deref().unwrap_or("python"),
                    message.into(),
                );
            }
            let _ = self.tasks.record_effects(
                id,
                parent_owner.as_deref().unwrap_or("python"),
                TaskEffects::between(before, self.session.mutation_revision()),
            );
        }
        for (id, request, script) in accepted {
            self.launch(id, request, script);
        }
        reply
    }

    fn launch(
        &mut self,
        id: TaskId,
        request: AsyncCommandRequest,
        script: Option<patinae_cmd::script::ScriptExecution>,
    ) {
        match request {
            AsyncCommandRequest::RunScript { .. } => {
                self.scripts
                    .insert(id, script.expect("script prepared before admission"));
            }
            AsyncCommandRequest::Plugin(request) => {
                let cancel = Arc::new(AtomicBool::new(false));
                self.python_cancel.insert(id, Arc::clone(&cancel));
                if self
                    .python_tx
                    .send(PythonWork {
                        id,
                        request,
                        cancel,
                    })
                    .is_err()
                {
                    self.tasks.fail_owner("python");
                }
            }
            request => {
                let _ = self.tasks.started(id, "standalone");
                let client = self.client.clone();
                let settings = LoadOptions::from(&self.session.settings);
                let worker = self
                    .runtime
                    .spawn(async move { download(request, settings).await });
                self.network.insert(id, worker.abort_handle());
                self.runtime.spawn(async move {
                    let result = worker.await;
                    if let Some(client) = client.upgrade() {
                        let _ = client.tx.send(Message::Call(Box::new(move |owner| {
                            if owner.network.remove(&id).is_none() {
                                return;
                            }
                            if !owner.tasks.begin_apply(id, owner.session.task_epoch()) {
                                return;
                            }
                            let outcome = match result {
                                Ok(Ok(result)) => result.apply(owner),
                                Ok(Err(error)) => TaskOutcome::failure("io", error),
                                Err(error) if error.is_cancelled() => {
                                    TaskOutcome::cancelled("cancelled")
                                }
                                Err(error) => {
                                    TaskOutcome::failure("executor_lost", error.to_string())
                                }
                            };
                            owner.tasks.finish(id, outcome);
                        })));
                    }
                });
            }
        }
    }

    fn pump(&mut self) {
        for (id, executor) in self.tasks.cancellation_targets() {
            if executor == "python" {
                if let Some(cancel) = self.python_cancel.get(&id) {
                    cancel.store(true, Ordering::Release);
                }
            }
            if let Some(handle) = self.network.get(&id) {
                handle.abort();
            }
        }
        let mut processed = 0;
        for (id, mut script) in std::mem::take(&mut self.scripts) {
            use patinae_cmd::script::{resolve_file_include, ScriptAction};
            if processed >= self.tasks.config().batch_size {
                self.scripts.insert(id, script);
                continue;
            }
            let mut action = script.advance(&self.tasks, id, "standalone");
            if action == ScriptAction::ReadSource {
                let source = std::fs::read_to_string(script.path())
                    .map_err(|e| {
                        patinae_cmd::tasks::TaskError::new("script_read_failed", e.to_string())
                    })
                    .and_then(|source| script.set_source(&source, self.executor.registry()));
                if let Err(error) = source {
                    self.tasks
                        .finish(id, TaskOutcome::failure(error.code, error.message));
                    processed += 1;
                    continue;
                }
                let _ = self.tasks.started(id, "standalone");
                action = script.advance(&self.tasks, id, "standalone");
            }
            match action {
                ScriptAction::Wait => {
                    self.scripts.insert(id, script);
                }
                ScriptAction::Execute { line, command } => {
                    processed += 1;
                    let result = script
                        .resolve_command(&command, self.executor.registry(), resolve_file_include)
                        .map_err(|e| e.to_string())
                        .and_then(|command| {
                            self.script_lineage = script.lineage().to_vec();
                            let reply = self.execute(&command, true, Some(id));
                            self.script_lineage.clear();
                            reply.result
                        });
                    if script.complete_step(&self.tasks, id, line, result) {
                        self.scripts.insert(id, script);
                    }
                }
                ScriptAction::Finished => processed += 1,
                ScriptAction::ReadSource => unreachable!("source installed above"),
            }
        }
        // Hints are intentionally discarded when no observer exists; snapshots persist.
        self.tasks.take_changes();
    }
}

/// Executable data awaiting owner-thread application; it carries no lifecycle state.
enum Downloaded {
    Molecule(patinae_cmd::FetchRequest, Box<ObjectMolecule>),
    File {
        bytes: Vec<u8>,
        name: String,
        format: String,
        settings: LoadOptions,
    },
}

impl Downloaded {
    fn apply(self, owner: &mut SessionOwner) -> TaskOutcome {
        let mut adapter = SessionAdapter {
            session: &mut owner.session,
            render_context: None,
            default_size: (1024, 768),
            needs_redraw: &mut owner.needs_redraw,
        };
        let outcome = match self {
            Self::Molecule(request, molecule) => LoadedData::fetched(&request).apply_molecule(
                &mut adapter,
                *molecule,
                LoadOptions::from(&request),
                owner.tasks.config(),
            ),
            Self::File {
                bytes,
                name,
                format,
                settings,
            } => LoadedData::file(&name, &format).apply_bytes(
                &mut adapter,
                &bytes,
                settings,
                owner.tasks.config(),
            ),
        };
        if outcome.effects != TaskEffects::None {
            owner.revision = owner.revision.wrapping_add(1);
        }
        outcome
    }
}

async fn download(
    request: AsyncCommandRequest,
    settings: LoadOptions,
) -> Result<Downloaded, String> {
    match request {
        AsyncCommandRequest::Fetch(request) => {
            let format = match request.format {
                FetchFormatCode::Pdb => patinae_io::FetchFormat::Pdb,
                FetchFormatCode::Cif => patinae_io::FetchFormat::Cif,
                FetchFormatCode::Bcif => patinae_io::FetchFormat::Bcif,
            };
            let molecule = tokio::time::timeout(
                Duration::from_secs(10),
                patinae_io::fetch_async_with_bond_tolerance(
                    &request.code,
                    format,
                    request.bond_tolerance,
                ),
            )
            .await
            .map_err(|_| "network timeout".to_string())?
            .map_err(|error| error.to_string())?;
            Ok(Downloaded::Molecule(request, Box::new(molecule)))
        }
        AsyncCommandRequest::LoadUrl { url, name, format } => {
            let format = format.unwrap_or_else(|| infer_format(&url));
            let bytes = patinae_io::fetch::fetch_url_bytes(&url, Duration::from_secs(10))
                .await
                .map_err(|error| error.to_string())?;
            Ok(Downloaded::File {
                bytes,
                name,
                format,
                settings,
            })
        }
        _ => Err("unsupported network task".into()),
    }
}

/// Process stream proxy that routes only the calling task's context.
#[pyclass]
struct TaskOutput {
    fallback: Py<PyAny>,
    task_cmd: Py<PyAny>,
    error: bool,
    users: AtomicUsize,
}

#[pymethods]
impl TaskOutput {
    fn write(&self, py: Python<'_>, text: &str) -> PyResult<usize> {
        self.fallback.call_method1(py, "write", (text,))?;
        let cmd = self.task_cmd.call_method0(py, "get")?;
        if !cmd.is_none(py) {
            if let Ok(backend) = cmd.getattr(py, "_backend") {
                if let Ok(backend) = backend.extract::<PyRef<'_, StandaloneBackend>>(py) {
                    backend.record_output(py, text, self.error)?;
                }
            }
        }
        Ok(text.chars().count())
    }

    fn flush(&self, py: Python<'_>) -> PyResult<()> {
        self.fallback.call_method0(py, "flush")?;
        Ok(())
    }

    fn __getattr__(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        self.fallback.getattr(py, name)
    }
}

struct OutputCapture {
    stdout: Py<TaskOutput>,
    stderr: Py<TaskOutput>,
}

impl OutputCapture {
    fn install(py: Python<'_>) -> PyResult<Self> {
        let stdout = install_stream(py, "stdout", false)?;
        match install_stream(py, "stderr", true) {
            Ok(stderr) => Ok(Self { stdout, stderr }),
            Err(error) => {
                let _ = release_stream(py, "stdout", &stdout);
                Err(error)
            }
        }
    }
}

impl Drop for OutputCapture {
    fn drop(&mut self) {
        Python::attach(|py| {
            let _ = release_stream(py, "stdout", &self.stdout);
            let _ = release_stream(py, "stderr", &self.stderr);
        });
    }
}

fn install_stream(py: Python<'_>, name: &str, error: bool) -> PyResult<Py<TaskOutput>> {
    let sys = py.import("sys")?;
    let current = sys.getattr(name)?;
    if let Ok(output) = current.cast::<TaskOutput>() {
        output.borrow().users.fetch_add(1, Ordering::Relaxed);
        return Ok(output.clone().unbind());
    }
    let task_cmd = py.import("patinae")?.getattr("_task_cmd")?.unbind();
    let output = Py::new(
        py,
        TaskOutput {
            fallback: current.unbind(),
            task_cmd,
            error,
            users: AtomicUsize::new(1),
        },
    )?;
    sys.setattr(name, &output)?;
    Ok(output)
}

fn release_stream(py: Python<'_>, name: &str, output: &Py<TaskOutput>) -> PyResult<()> {
    let (last, fallback) = {
        let output = output.borrow(py);
        (
            output.users.fetch_sub(1, Ordering::Relaxed) == 1,
            output.fallback.clone_ref(py),
        )
    };
    let sys = py.import("sys")?;
    if last && sys.getattr(name)?.is(output.bind(py)) {
        sys.setattr(name, fallback)?;
    }
    Ok(())
}

struct PythonCommand;
impl patinae_cmd::Command for PythonCommand {
    fn argument_syntax(&self) -> patinae_cmd::ArgumentSyntax {
        patinae_cmd::ArgumentSyntax::Verbatim
    }

    fn name(&self) -> &str {
        "python"
    }
    fn aliases(&self) -> &[&str] {
        &["/"]
    }
    fn execute<'v, 'r>(
        &self,
        context: &mut patinae_cmd::CommandContext<'v, 'r, dyn patinae_cmd::ViewerLike + 'v>,
        args: &patinae_cmd::ParsedCommand,
    ) -> patinae_cmd::CmdResult {
        context.request_task(AsyncCommandRequest::Plugin(PluginTaskRequest::new(
            "python",
            serde_json::json!({"code":args.raw_args().unwrap_or("")}),
        )))
    }
}

#[pyclass]
struct CancellationTrace {
    cancel: Arc<AtomicBool>,
}
#[pymethods]
impl CancellationTrace {
    fn __call__(
        slf: PyRef<'_, Self>,
        _frame: &Bound<'_, PyAny>,
        _event: &Bound<'_, PyAny>,
        _arg: &Bound<'_, PyAny>,
    ) -> PyResult<Py<CancellationTrace>> {
        if slf.cancel.load(Ordering::Acquire) {
            return Err(pyo3::exceptions::PyInterruptedError::new_err(
                "task cancelled",
            ));
        }
        Ok(slf.into())
    }
}

fn python_loop(client: Weak<Client>, rx: mpsc::Receiver<PythonWork>) {
    while let Ok(work) = rx.recv() {
        let Some(client) = client.upgrade() else {
            break;
        };
        let id = work.id;
        let _ = client.tx.send(Message::Call(Box::new(move |owner| {
            let _ = owner.tasks.started(id, "python");
        })));
        let result = if work.cancel.load(Ordering::Acquire) {
            Err("cancelled".into())
        } else {
            Python::attach(|py| -> PyResult<()> {
                let backend = Py::new(py, StandaloneBackend::for_task(Arc::clone(&client), id))?;
                let cmd = py
                    .import("patinae._cmd")?
                    .getattr("Cmd")?
                    .call1((backend,))?;
                let locals = PyDict::new(py);
                locals.set_item("cmd", &cmd)?;
                locals.set_item("__name__", "__main__")?;
                let code = if let Some(path) = work
                    .request
                    .payload
                    .get("path")
                    .and_then(serde_json::Value::as_str)
                {
                    locals.set_item("__file__", path)?;
                    std::fs::read_to_string(path)
                        .map_err(|error| pyo3::exceptions::PyOSError::new_err(error.to_string()))?
                } else {
                    work.request
                        .payload
                        .get("code")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("")
                        .to_string()
                };
                let trace = Py::new(
                    py,
                    CancellationTrace {
                        cancel: Arc::clone(&work.cancel),
                    },
                )?;
                let sys = py.import("sys")?;
                let task_cmd = py.import("patinae")?.getattr("_task_cmd")?;
                let _capture = OutputCapture::install(py)?;
                let previous_trace = sys.call_method0("gettrace")?;
                let token = task_cmd.call_method1("set", (&cmd,))?;
                let result = (|| {
                    sys.call_method1("settrace", (trace,))?;
                    let code = std::ffi::CString::new(code).map_err(|error| {
                        pyo3::exceptions::PyValueError::new_err(error.to_string())
                    })?;
                    py.run(code.as_c_str(), Some(&locals), Some(&locals))
                })();
                let trace_restored = sys.call_method1("settrace", (previous_trace,));
                let context_restored = task_cmd.call_method1("reset", (token,));
                result?;
                trace_restored?;
                context_restored?;
                Ok(())
            })
            .map_err(|error| error.to_string())
        };
        let cancelled = work.cancel.load(Ordering::Acquire);
        let _ = client.tx.send(Message::Call(Box::new(move |owner| {
            owner.python_cancel.remove(&id);
            let outcome = if cancelled {
                TaskOutcome::cancelled("cancelled")
            } else {
                match result {
                    Ok(()) => TaskOutcome::success(None, TaskEffects::None),
                    Err(error) => TaskOutcome::failure("python_error", error),
                }
            };
            let _ = owner.tasks.finish_owned(id, "python", outcome);
        })));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standalone_scripts_resolve_quoted_includes_and_reject_cycles() {
        let directory = std::env::temp_dir().join(format!(
            "patinae-script-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let main = directory.join("main.pml");
        std::fs::write(&main, "@ \"child file.pml\"\ngroup after").unwrap();
        std::fs::write(directory.join("child file.pml"), "group child").unwrap();
        let mut owner = SessionOwner::new(Weak::new()).unwrap();
        let command = format!(
            "run {}",
            serde_json::to_string(&main.to_string_lossy()).unwrap()
        );
        let id = owner.execute(&command, true, None).task_ids[0];
        for _ in 0..16 {
            owner.pump();
        }
        assert_eq!(
            owner.tasks.get(id).unwrap().state,
            patinae_cmd::tasks::TaskState::Succeeded
        );
        assert!(owner.session.registry.get("child").is_some());
        assert!(owner.session.registry.get("after").is_some());
        std::fs::write(&main, "@ main.pml\ngroup unreachable").unwrap();
        let id = owner.execute(&command, true, None).task_ids[0];
        for _ in 0..16 {
            owner.pump();
        }
        assert_eq!(
            owner.tasks.get(id).unwrap().state,
            patinae_cmd::tasks::TaskState::Failed
        );
        assert!(owner.session.registry.get("unreachable").is_none());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn owner_thread_records_exact_and_unknown_command_effects() {
        let client = start().unwrap();
        let (tx, rx) = mpsc::sync_channel(1);
        client
            .tx
            .send(Message::Call(Box::new(move |owner| {
                let mut molecule = ObjectMolecule::new("sample");
                molecule.add_atom(patinae_mol::Atom::new("C", patinae_mol::Element::Carbon));
                owner
                    .session
                    .registry
                    .add(patinae_scene::MoleculeObject::new(molecule));
                let mut results = Vec::new();
                for command in [
                    "color red",
                    "color red",
                    "color red, none",
                    "bg_color blue",
                    "bg_color blue",
                    "group inserted",
                    "help",
                ] {
                    let id = owner
                        .tasks
                        .admit(TaskSpec::new("script", "python"))
                        .unwrap();
                    let reply = owner.execute(command, true, Some(id));
                    results.push((reply.result, owner.tasks.get(id).unwrap().effects));
                    owner
                        .tasks
                        .finish(id, TaskOutcome::success(None, TaskEffects::None));
                }
                tx.send(results).unwrap();
            })))
            .unwrap();
        let results = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(
            results.iter().all(|(result, _)| result.is_ok()),
            "{results:?}"
        );
        assert_eq!(
            results
                .into_iter()
                .map(|(_, effects)| effects)
                .collect::<Vec<_>>(),
            [
                TaskEffects::Applied,
                TaskEffects::None,
                TaskEffects::None,
                TaskEffects::Applied,
                TaskEffects::None,
                TaskEffects::Unknown,
                TaskEffects::None,
            ]
        );
    }

    #[test]
    fn stream_output_is_bound_to_calling_task_context() {
        Python::attach(|py| {
            let client = start().unwrap();
            let (reply_tx, reply_rx) = mpsc::sync_channel(1);
            client
                .tx
                .send(Message::Call(Box::new(move |owner| {
                    reply_tx
                        .send(
                            owner
                                .tasks
                                .admit(TaskSpec::new("python", "python"))
                                .unwrap(),
                        )
                        .unwrap();
                })))
                .unwrap();
            let id = py.detach(move || reply_rx.recv().unwrap());
            let backend =
                Py::new(py, StandaloneBackend::for_task(Arc::clone(&client), id)).unwrap();
            let cmd = py
                .import("types")
                .unwrap()
                .getattr("SimpleNamespace")
                .unwrap()
                .call0()
                .unwrap();
            cmd.setattr("_backend", &backend).unwrap();
            let context = py
                .import("contextvars")
                .unwrap()
                .getattr("ContextVar")
                .unwrap()
                .call1(("output_test",))
                .unwrap();
            context.call_method1("set", (py.None(),)).unwrap();
            let fallback = py
                .import("io")
                .unwrap()
                .getattr("StringIO")
                .unwrap()
                .call0()
                .unwrap();
            let stream = TaskOutput {
                fallback: fallback.clone().unbind(),
                task_cmd: context.clone().unbind(),
                error: false,
                users: AtomicUsize::new(1),
            };
            stream.write(py, "outside task").unwrap();
            let token = context.call_method1("set", (&cmd,)).unwrap();
            stream.write(py, "task output").unwrap();
            context.call_method1("reset", (token,)).unwrap();
            assert_eq!(
                fallback
                    .call_method0("getvalue")
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                "outside tasktask output"
            );
            let snapshot = backend
                .call_method1(py, "get_task", (id.to_string(),))
                .unwrap();
            let diagnostics = snapshot.bind(py).get_item("diagnostics").unwrap();
            assert_eq!(diagnostics.len().unwrap(), 1);
            assert_eq!(
                diagnostics
                    .get_item(0)
                    .unwrap()
                    .get_item("message")
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                "task output"
            );
            client
                .tx
                .send(Message::Call(Box::new(move |owner| {
                    owner
                        .tasks
                        .finish(id, TaskOutcome::success(None, TaskEffects::None));
                })))
                .unwrap();
        });
    }
}
