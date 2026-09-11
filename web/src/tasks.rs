//! Browser execution adapter for the shared task registry.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use patinae_cmd::tasks::{
    TaskConfig, TaskEffects, TaskError, TaskId, TaskListRequest, TaskOutcome, TaskRunner, TaskSpec,
    TaskTime,
};
use patinae_cmd::{AsyncCommandAcceptance, AsyncCommandRequest, CommandReply, FetchFormatCode};
use wasm_bindgen::{prelude::*, JsCast};
use wasm_bindgen_futures::{future_to_promise, spawn_local, JsFuture};

use patinae_cmd::loading::{infer_format, LoadOptions, LoadedData};

use super::{performance_now_ms, WebState, WebViewer};

const OWNER: &str = "browser";
/// Waiters yield to browser I/O without depending on animation frames.
const WAIT_INTERVAL_MS: i32 = 10;

type Files = HashMap<String, Vec<u8>>;
type SharedState = Rc<RefCell<WebState>>;

pub(super) fn browser_task_runner() -> TaskRunner {
    let instance = ((js_sys::Date::now() as u128) << 64)
        | ((js_sys::Math::random() * u64::MAX as f64) as u128);
    TaskRunner::new(
        instance,
        TaskConfig::default(),
        Box::new(|| TaskTime {
            unix_ms: js_sys::Date::now() as u64,
            monotonic_ms: performance_now_ms() as u64,
        }),
    )
}

fn js_error(code: &str, message: impl ToString) -> JsValue {
    serde_wasm_bindgen::to_value(&TaskError::new(code, message.to_string()))
        .unwrap_or_else(|_| JsValue::from_str(code))
}

fn json<T: serde::Serialize>(value: &T) -> Result<JsValue, JsValue> {
    value
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .map_err(|error| js_error("serialization_error", error))
}

fn parse_id(id: &str) -> Result<TaskId, JsValue> {
    id.parse().map_err(|error| js_error("invalid_id", error))
}

/// Publish hints after releasing the scene borrow; listeners may read task state.
fn publish_changes(state: &SharedState) {
    let (changes, listener) = {
        let host = state.borrow();
        (host.tasks.take_changes(), host.task_listener.clone())
    };
    if let Some(listener) = listener {
        if !changes.is_empty() {
            if let Ok(changes) = json(&changes) {
                let _ = listener.call1(&JsValue::UNDEFINED, &changes);
            }
        }
    }
}

#[wasm_bindgen]
impl WebViewer {
    /// Execute a command and return its admission or synchronous result.
    pub fn execute(&self, command: &str) -> Result<JsValue, JsValue> {
        json(&execute_command(
            &self.state,
            command,
            &Files::new(),
            None,
            &[],
        ))
    }

    /// Execute a command with addressed local file contents supplied by a client.
    ///
    /// `files` is an object mapping original paths to Uint8Array values.
    pub fn execute_with_files(&self, command: &str, files: JsValue) -> Result<JsValue, JsValue> {
        let files = decode_files(files)?;
        json(&execute_command(&self.state, command, &files, None, &[]))
    }

    /// Return local input paths using the shared command parser.
    pub fn command_files(
        &self,
        command: &str,
        script_path: Option<String>,
    ) -> Result<JsValue, JsValue> {
        let commands = patinae_cmd::script_steps(command, self.state.borrow().executor.registry())
            .map_err(|error| js_error("invalid_command", error))?;
        let mut paths = Vec::new();
        for (_, command) in commands {
            let command = resolve_include(
                &command,
                script_path.as_deref(),
                self.state.borrow().executor.registry(),
            )?;
            let command = self
                .state
                .borrow()
                .executor
                .registry()
                .parse_command(&command)
                .map_err(|error| js_error("invalid_command", error))?;
            let argument = match command.name.as_str() {
                "load" => command.str_arg(0, "filename"),
                "run" | "@" => command.str_arg(0, "filename"),
                _ => None,
            };
            if let Some(path) = argument {
                if !is_url(path) && !paths.iter().any(|value| value == path) {
                    paths.push(path.to_owned());
                }
            }
        }
        json(&paths)
    }

    /// Read the authoritative retained snapshot.
    pub fn get_task(&self, id: &str) -> Result<JsValue, JsValue> {
        let snapshot = self
            .state
            .borrow()
            .tasks
            .get(parse_id(id)?)
            .map_err(|error| js_error(&error.to_string(), error))?;
        json(&snapshot)
    }

    /// List tasks from this viewer, using the common filters and cursor.
    pub fn list_tasks(&self, filters: JsValue) -> Result<JsValue, JsValue> {
        let request: TaskListRequest = if filters.is_null() || filters.is_undefined() {
            TaskListRequest::default()
        } else {
            serde_wasm_bindgen::from_value(filters)
                .map_err(|error| js_error("invalid_filters", error))?
        };
        let page = self
            .state
            .borrow()
            .tasks
            .list(&request)
            .map_err(|error| js_error(&error.to_string(), error))?;
        json(&page)
    }

    /// Request cancellation and abort the corresponding browser requests.
    pub fn cancel_task(&self, id: &str) -> Result<JsValue, JsValue> {
        let reply = {
            let host = self.state.borrow();
            let reply = host
                .tasks
                .cancel(parse_id(id)?)
                .map_err(|error| js_error(&error.to_string(), error))?;
            for (id, owner) in host.tasks.cancellation_targets() {
                if owner == OWNER {
                    if let Some(abort) = host.aborts.get(&id) {
                        abort.abort();
                    }
                }
            }
            reply
        };
        publish_changes(&self.state);
        json(&reply)
    }

    /// Wait for a terminal snapshot; timeout never cancels the task.
    pub fn wait_task(&self, id: &str, timeout_ms: Option<f64>) -> js_sys::Promise {
        let id = parse_id(id);
        let state = Rc::downgrade(&self.state);
        future_to_promise(async move {
            if timeout_ms.is_some_and(|timeout| !timeout.is_finite() || timeout < 0.0) {
                return Err(js_error(
                    "invalid_timeout",
                    "timeout must be finite and nonnegative",
                ));
            }
            let snapshot = wait_for_task(state, id?, timeout_ms).await?;
            json(&snapshot)
        })
    }

    /// Subscribe to task changes as hints to reread the registry.
    pub fn set_task_listener(&self, listener: Option<js_sys::Function>) {
        self.state.borrow_mut().task_listener = listener;
    }

    /// Apply bytes synchronously and return the common command receipt.
    pub fn load_data(&self, data: &[u8], name: &str, format: &str) -> Result<JsValue, JsValue> {
        let result = self
            .state
            .borrow_mut()
            .load_data(data, name, format)
            .map_err(|error| describe_js_error(&error));
        json(&CommandReply {
            result,
            messages: Vec::new(),
            task_ids: Vec::new(),
        })
    }
}

fn decode_files(value: JsValue) -> Result<Files, JsValue> {
    if value.is_null() || value.is_undefined() {
        return Ok(Files::new());
    }
    if !value.is_object() {
        return Err(js_error(
            "invalid_files",
            "files must be a path-to-bytes object",
        ));
    }
    let object = js_sys::Object::from(value);
    let mut files = Files::new();
    for key in js_sys::Object::keys(&object) {
        let path = key
            .as_string()
            .ok_or_else(|| js_error("invalid_files", "file path must be text"))?;
        let bytes = js_sys::Reflect::get(&object, &key)?;
        if !bytes.is_instance_of::<js_sys::Uint8Array>() {
            return Err(js_error(
                "invalid_files",
                "file contents must be Uint8Array",
            ));
        }
        files.insert(path, js_sys::Uint8Array::new(&bytes).to_vec());
    }
    Ok(files)
}

struct Work {
    id: TaskId,
    request: AsyncCommandRequest,
    abort: web_sys::AbortController,
    files: Files,
    script: Option<patinae_cmd::script::ScriptExecution>,
}

fn execute_command(
    state: &SharedState,
    command: &str,
    files: &Files,
    parent: Option<TaskId>,
    lineage: &[String],
) -> CommandReply {
    let mut work = Vec::new();
    let reply = {
        let mut host = state.borrow_mut();
        let WebState {
            session,
            executor,
            needs_redraw,
            tasks,
            width,
            height,
            pending_output,
            ..
        } = &mut *host;
        let epoch = session.task_epoch();
        let mut sink = |request: AsyncCommandRequest| {
            let kind = match &request {
                AsyncCommandRequest::Fetch(_) => "fetch",
                AsyncCommandRequest::LoadUrl { .. } => "load",
                AsyncCommandRequest::LoadFile { path, .. } if files.contains_key(path) => "load",
                AsyncCommandRequest::RunScript { path }
                    if is_url(path) || files.contains_key(path) =>
                {
                    "script"
                }
                _ => return AsyncCommandAcceptance::Unsupported,
            };
            let script = if let AsyncCommandRequest::RunScript { path } = &request {
                match patinae_cmd::script::ScriptExecution::new(path.clone(), path.clone(), lineage)
                {
                    Ok(script) => Some(script),
                    Err(error) => return AsyncCommandAcceptance::Rejected(error),
                }
            } else {
                None
            };
            let Ok(abort) = web_sys::AbortController::new() else {
                return AsyncCommandAcceptance::Rejected(
                    patinae_cmd::tasks::TaskStartError::ExecutorUnavailable,
                );
            };
            let mut spec = TaskSpec::new(kind, OWNER);
            spec.parent_id = parent;
            spec.scene_epoch = (kind != "script").then_some(epoch);
            spec.origin = "command".into();
            match tasks.admit(spec) {
                Ok(id) => {
                    work.push(Work {
                        id,
                        request,
                        abort,
                        files: files.clone(),
                        script,
                    });
                    AsyncCommandAcceptance::Accepted(id)
                }
                Err(error) => AsyncCommandAcceptance::Rejected(error),
            }
        };
        let mut changed = false;
        let before = session.mutation_revision();
        let mut adapter = patinae_scene::SessionAdapter {
            session,
            render_context: None,
            default_size: (*width, *height),
            needs_redraw: &mut changed,
        };
        let execution = executor.execute_captured(&mut adapter, command, false, Some(&mut sink));
        *needs_redraw |= changed;
        if let Some(parent) = parent {
            let effects = TaskEffects::between(before, session.mutation_revision());
            let _ = tasks.record_effects(parent, OWNER, effects);
            for message in &execution.output.messages {
                let _ = tasks.output(parent, OWNER, message.into());
            }
        }
        for action in &execution.output.actions {
            if matches!(action, patinae_cmd::CommandAction::ClearOutput) {
                pending_output.push(super::OutputMsg {
                    level: "clear",
                    text: String::new(),
                });
            }
        }
        CommandReply::from(execution)
    };
    for work in work {
        state
            .borrow_mut()
            .aborts
            .insert(work.id, work.abort.clone());
        let weak = Rc::downgrade(state);
        spawn_local(async move {
            execute_work(weak, work).await;
        });
    }
    publish_changes(state);
    reply
}

async fn execute_work(state: Weak<RefCell<WebState>>, mut work: Work) {
    let Some(host) = state.upgrade() else {
        return;
    };
    let _ = host.borrow().tasks.started(work.id, OWNER);
    publish_changes(&host);
    drop(host);
    let result = match &work.request {
        AsyncCommandRequest::Fetch(request) => {
            let (format, url) = match request.format {
                FetchFormatCode::Pdb => (
                    "pdb",
                    format!("https://files.rcsb.org/download/{}.pdb.gz", request.code),
                ),
                FetchFormatCode::Cif => (
                    "cif",
                    format!("https://files.rcsb.org/download/{}.cif.gz", request.code),
                ),
                FetchFormatCode::Bcif => (
                    "bcif",
                    format!(
                        "https://models.rcsb.org/{}.bcif.gz",
                        request.code.to_lowercase()
                    ),
                ),
            };
            fetch_bytes(&url, &work.abort)
                .await
                .map(|bytes| (bytes, request.name.clone(), format.to_owned()))
        }
        AsyncCommandRequest::LoadUrl { url, name, format } => {
            fetch_bytes(url, &work.abort).await.map(|bytes| {
                (
                    bytes,
                    name.clone(),
                    format.clone().unwrap_or_else(|| infer_format(url)),
                )
            })
        }
        AsyncCommandRequest::LoadFile { path, name, format } => work
            .files
            .get(path)
            .cloned()
            .ok_or_else(|| js_error("missing_file", path))
            .map(|bytes| {
                (
                    bytes,
                    name.clone(),
                    format.clone().unwrap_or_else(|| infer_format(path)),
                )
            }),
        AsyncCommandRequest::RunScript { path } => {
            let script = work
                .script
                .take()
                .expect("script prepared before admission");
            run_script(&state, &work, path, script).await;
            return;
        }
        AsyncCommandRequest::Plugin(_) => Err(js_error(
            "executor_unavailable",
            "plugins are not available in this viewer",
        )),
    };
    let Some(host) = state.upgrade() else {
        return;
    };
    {
        let mut host = host.borrow_mut();
        host.aborts.remove(&work.id);
        let epoch = host.session.task_epoch();
        if host.tasks.begin_apply(work.id, epoch) {
            let outcome = match result {
                Ok((bytes, name, format)) => {
                    let descriptor = match &work.request {
                        AsyncCommandRequest::Fetch(request) => LoadedData::fetched(request),
                        _ => LoadedData::file(&name, &format),
                    };
                    let options = match &work.request {
                        AsyncCommandRequest::Fetch(request) => LoadOptions::from(request),
                        _ => LoadOptions::from(&host.session.settings),
                    };
                    let config = host.tasks.config().clone();
                    let WebState {
                        session,
                        needs_redraw,
                        width,
                        height,
                        ..
                    } = &mut *host;
                    let mut adapter = patinae_scene::SessionAdapter {
                        session,
                        needs_redraw,
                        render_context: None,
                        default_size: (*width, *height),
                    };
                    let outcome = descriptor.apply_bytes(&mut adapter, &bytes, options, &config);
                    host.pending_warnings
                        .extend(outcome.diagnostics.iter().map(|d| d.message.clone()));
                    outcome
                }

                Err(error) => TaskOutcome::failure("download_failed", describe_js_error(&error)),
            };
            host.tasks.finish(work.id, outcome);
        }
    }
    publish_changes(&host);
}

async fn fetch_bytes(url: &str, abort: &web_sys::AbortController) -> Result<Vec<u8>, JsValue> {
    let options = web_sys::RequestInit::new();
    options.set_signal(Some(&abort.signal()));
    let window = web_sys::window()
        .ok_or_else(|| js_error("executor_unavailable", "browser window unavailable"))?;
    let response = JsFuture::from(window.fetch_with_str_and_init(url, &options)).await?;
    let response: web_sys::Response = response.dyn_into()?;
    if !response.ok() {
        return Err(js_error(
            "http_error",
            format!("HTTP {} {}", response.status(), response.status_text()),
        ));
    }
    let buffer = JsFuture::from(response.array_buffer()?).await?;
    Ok(js_sys::Uint8Array::new(&buffer).to_vec())
}

#[cfg(test)]
fn script_commands(source: &str) -> Result<Vec<String>, String> {
    patinae_cmd::script_steps(source, &patinae_cmd::CommandRegistry::new())
        .map(|steps| steps.into_iter().map(|(_, command)| command).collect())
        .map_err(|error| error.to_string())
}

fn resolve_include(
    command: &str,
    script_path: Option<&str>,
    registry: &patinae_cmd::CommandRegistry,
) -> Result<String, JsValue> {
    let Some(base) = script_path else {
        return Ok(command.into());
    };
    let script = patinae_cmd::script::ScriptExecution::new(base.into(), base.into(), &[])
        .map_err(|e| js_error("invalid_script", e))?;
    script
        .resolve_command(command, registry, resolve_script_path)
        .map_err(|e| js_error(&e.code, e.message))
}

fn resolve_script_path(base: &str, include: &str) -> Result<String, TaskError> {
    if is_url(base) {
        web_sys::Url::new_with_base(include, base)
            .map(|url| url.href())
            .map_err(|e| TaskError::new("invalid_script", describe_js_error(&e)))
    } else {
        patinae_cmd::script::resolve_file_include(base, include)
    }
}

fn is_url(path: &str) -> bool {
    path.starts_with("http://") || path.starts_with("https://")
}

fn describe_js_error(error: &JsValue) -> String {
    error
        .as_string()
        .or_else(|| {
            js_sys::Reflect::get(error, &JsValue::from_str("message"))
                .ok()
                .and_then(|value| value.as_string())
        })
        .unwrap_or_else(|| format!("{error:?}"))
}

async fn browser_tick() -> Result<(), JsValue> {
    let promise = js_sys::Promise::new(&mut |resolve, reject| {
        let result = web_sys::window()
            .ok_or_else(|| js_error("executor_unavailable", "browser window unavailable"))
            .and_then(|window| {
                window
                    .set_timeout_with_callback_and_timeout_and_arguments_0(
                        &resolve,
                        WAIT_INTERVAL_MS,
                    )
                    .map(|_| ())
            });
        if let Err(error) = result {
            let _ = reject.call1(&JsValue::UNDEFINED, &error);
        }
    });
    JsFuture::from(promise).await.map(|_| ())
}

async fn wait_for_task(
    state: Weak<RefCell<WebState>>,
    id: TaskId,
    timeout: Option<f64>,
) -> Result<patinae_cmd::tasks::TaskSnapshot, JsValue> {
    let started = performance_now_ms();
    loop {
        let snapshot = {
            let state = state
                .upgrade()
                .ok_or_else(|| js_error("viewer_closed", "viewer was disposed"))?;
            let host = state.borrow();
            host.tasks
                .get(id)
                .map_err(|error| js_error(&error.to_string(), error))?
        };
        if snapshot.state.is_terminal() {
            return Ok(snapshot);
        }
        if timeout.is_some_and(|timeout| performance_now_ms() - started >= timeout) {
            return Err(js_error(
                "timeout",
                format!("waiting for task {id} timed out"),
            ));
        }
        browser_tick().await?;
    }
}

async fn run_script(
    state: &Weak<RefCell<WebState>>,
    work: &Work,
    path: &str,
    mut script: patinae_cmd::script::ScriptExecution,
) {
    use patinae_cmd::script::ScriptAction;
    loop {
        let Some(host) = state.upgrade() else {
            return;
        };
        let action = script.advance(&host.borrow().tasks, work.id, OWNER);
        match action {
            ScriptAction::Finished => break,
            ScriptAction::ReadSource => {
                drop(host);
                let source = if let Some(bytes) = work.files.get(path) {
                    Ok(bytes.clone())
                } else {
                    fetch_bytes(path, &work.abort)
                        .await
                        .map_err(|e| TaskError::new("script_read_failed", describe_js_error(&e)))
                }
                .and_then(|bytes| {
                    String::from_utf8(bytes)
                        .map_err(|e| TaskError::new("invalid_script", e.to_string()))
                })
                .and_then(|source| {
                    let host = state
                        .upgrade()
                        .ok_or_else(|| TaskError::new("executor_lost", "host closed"))?;
                    let host = host.borrow();
                    script.set_source(&source, host.executor.registry())
                });
                if let Err(error) = source {
                    if let Some(host) = state.upgrade() {
                        let outcome = if work.abort.signal().aborted() {
                            TaskOutcome::cancelled("requested")
                        } else {
                            TaskOutcome::failure(error.code, error.message)
                        };
                        host.borrow().tasks.finish(work.id, outcome);
                    }
                    break;
                }
                continue;
            }
            ScriptAction::Execute { line, command } => {
                let result = script
                    .resolve_command(
                        &command,
                        host.borrow().executor.registry(),
                        resolve_script_path,
                    )
                    .and_then(|command| {
                        {
                            let host = host.borrow();
                            host.tasks.can_apply_effect(
                                work.id,
                                OWNER,
                                host.session.task_epoch(),
                            )?;
                        }
                        Ok(execute_command(
                            &host,
                            &command,
                            &work.files,
                            Some(work.id),
                            script.lineage(),
                        ))
                    })
                    .map_err(|e| e.to_string())
                    .and_then(|reply| reply.result);
                if !script.complete_step(&host.borrow().tasks, work.id, line, result) {
                    break;
                }
            }
            ScriptAction::Wait => {}
        }
        publish_changes(&host);
        drop(host);
        if let Err(error) = browser_tick().await {
            if let Some(host) = state.upgrade() {
                host.borrow().tasks.finish(
                    work.id,
                    TaskOutcome::failure("executor_lost", describe_js_error(&error)),
                );
            }
            break;
        }
    }
    if let Some(host) = state.upgrade() {
        host.borrow_mut().aborts.remove(&work.id);
        publish_changes(&host);
    }
}

impl Drop for WebViewer {
    fn drop(&mut self) {
        let host = self.state.borrow();
        for abort in host.aborts.values() {
            abort.abort();
        }
        host.tasks.fail_owner(OWNER);
    }
}

#[cfg(test)]
mod tests {
    use super::{infer_format, script_commands};

    #[test]
    fn script_steps_preserve_quoted_separators_and_continuations() {
        let source = concat!(
            r#"load "a;b.pdb"; color red, \"#,
            "\n",
            " all\n# comment\nzoom"
        );
        let steps = script_commands(source).unwrap();
        assert_eq!(steps.len(), 3);
        assert_eq!(
            patinae_cmd::parse_command(&steps[0]).unwrap().get_str(0),
            Some("a;b.pdb")
        );
        assert_eq!(patinae_cmd::parse_command(&steps[1]).unwrap().name, "color");
        assert_eq!(steps[2], "zoom");
    }

    #[test]
    fn format_detection_ignores_url_queries_and_compression() {
        assert_eq!(
            infer_format("https://example.org/structure.cif.gz?token=x#fragment"),
            "cif"
        );
        assert_eq!(infer_format("structure.PDB"), "pdb");
    }
}
