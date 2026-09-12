//! Minimal AI agent: model work stays in a worker, task ownership stays in Patinae.

mod config;
mod mcp;
mod screenshot;
mod worker;

use patinae_plugin::{
    patinae_plugin,
    prelude::*,
    tasks::{ChildFailurePolicy, TaskError, TaskEvent, TaskOutcome},
};
use std::{sync::mpsc, thread::JoinHandle};
use tokio::sync::oneshot;
use worker::{HostReply, HostRequest, Operation};

patinae_plugin! {
    name: "ai",
    description: "AI agent with live command discovery and cancellable host tasks",
    commands: [AiCommand],
    register: |reg| { reg.set_message_handler(AiHandler::default()); },
}

struct AiCommand;
impl Command for AiCommand {
    fn name(&self) -> &str {
        "ai"
    }
    fn argument_syntax(&self) -> ArgumentSyntax {
        ArgumentSyntax::Verbatim
    }
    fn runtime_requirements(&self) -> CommandRuntimeRequirements {
        // Admission only needs the prompt; scene names arrive through polling.
        CommandRuntimeRequirements::NONE
    }
    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let prompt = args.raw_args().unwrap_or_default().trim();
        if prompt.is_empty() {
            return Err(CmdError::invalid_argument(
                "prompt",
                "usage: ai <prompt>; see help ai",
            ));
        }
        if prompt.len() > worker::MAX_TEXT_BYTES {
            return Err(CmdError::invalid_argument(
                "prompt",
                "AI prompt is too large",
            ));
        }
        let kind = match prompt {
            "cancel" => "ai.cancel",
            "status" => "ai.status",
            _ => "ai",
        };
        let mut request = PluginTaskRequest::new(kind, serde_json::json!({"prompt": prompt}));
        request.scene_scoped = kind == "ai";
        request.child_failure_policy = ChildFailurePolicy::ParentDecides;
        // Explicit AI answers still use the bus; internal task diagnostics stay in logs.
        request.silent = true;
        ctx.request_task(AsyncCommandRequest::Plugin(request))
    }
    command_help! {
        CMD "ai"
        DESCRIPTION ["executes a natural-language request using a configured model.",
            "Configure <plugin_dir>/ai/config.toml. One request runs at a time.",
            "Use ai status to see the active worker and ai cancel to request cancellation.",
            "Commands can change the scene and write files. Cancellation does not undo changes."]
        REQUIRED [{ "prompt", "string", "natural-language request (the entire rest of the line)" }]
        OPTIONAL []
        EXAMPLES ["ai show the loaded structure as a cartoon", "ai color chain A blue and save a PNG"]
    }
}

#[derive(Default)]
struct AiHandler {
    active: Option<Active>,
    next_id: u64,
    cancellations: Vec<(u64, TaskId)>,
}
struct Active {
    task: TaskId,
    worker: JoinHandle<TaskOutcome>,
    cancel: Option<oneshot::Sender<()>>,
    requests: mpsc::Receiver<HostRequest>,
    pending: Option<(u64, oneshot::Sender<Result<HostReply, TaskError>>)>,
}
impl Active {
    fn cancel(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.send(());
        }
        self.pending = None;
    }
}
impl Drop for AiHandler {
    fn drop(&mut self) {
        if let Some(mut active) = self.active.take() {
            active.cancel();
            // No plugin thread may outlive its dynamic library. Dropping the async
            // future interrupts network/host waits; only shutdown joins the thread.
            let _ = active.worker.join();
        }
    }
}
impl AiHandler {
    fn id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }
    fn event(&mut self, ctx: &mut PollContext<'_>, task: TaskId, event: TaskEvent) {
        ctx.report_task_event(self.id(), task, event);
    }
    fn reap(&mut self, ctx: &mut PollContext<'_>) {
        if !self
            .active
            .as_ref()
            .is_some_and(|active| active.worker.is_finished())
        {
            return;
        }
        let active = self.active.take().expect("finished worker exists");
        if let Some((id, _)) = active.pending {
            ctx.forget_task_wait(id);
        }
        let was_cancelled = active.cancel.is_none();
        let mut outcome = active
            .worker
            .join()
            .unwrap_or_else(|_| TaskOutcome::failure("worker_panic", "AI worker panicked"));
        if was_cancelled && matches!(outcome.status, TaskOutcomeStatus::Success { .. }) {
            outcome = TaskOutcome::cancelled("cancelled by host");
        }
        match &outcome.status {
            TaskOutcomeStatus::Success { data: Some(data) } => {
                if let Some(answer) = data.payload["answer"].as_str() {
                    ctx.bus.print_info(format!("AI: {answer}"));
                }
            }
            TaskOutcomeStatus::Failure { error } => ctx
                .bus
                .print_error(format!("AI [{}]: {}", error.code, error.message)),
            TaskOutcomeStatus::Cancelled { .. } => ctx.bus.print_info("AI: cancelled"),
            _ => {}
        }
        self.event(ctx, active.task, TaskEvent::Finished(outcome));
    }
}
impl MessageHandler for AiHandler {
    fn on_message(&mut self, _: &AppMessage, _: &mut MessageBus) {}
    fn needs_poll(&self) -> bool {
        true
    }
    fn poll(&mut self, ctx: &mut PollContext<'_>) {
        let controls = std::mem::take(&mut self.cancellations);
        for (id, task) in controls {
            let outcome = match ctx.task_result(id) {
                Some(TaskQueryResult::Cancel(Ok(reply))) => {
                    ctx.bus.print_info(format!("AI cancellation: {reply:?}"));
                    Some(TaskOutcome::success(
                        None,
                        patinae_plugin::tasks::TaskEffects::None,
                    ))
                }
                Some(TaskQueryResult::Cancel(Err(error))) => {
                    Some(TaskOutcome::failure("cancel_failed", error.to_string()))
                }
                Some(TaskQueryResult::Unavailable(error)) => {
                    Some(TaskOutcome::failure("host_unavailable", error))
                }
                _ => None,
            };
            if let Some(outcome) = outcome {
                self.event(ctx, task, TaskEvent::Finished(outcome));
            } else {
                self.cancellations.push((id, task));
            }
        }
        if let Some(active) = &mut self.active {
            if ctx.task_cancellations.contains(&active.task) {
                if let Some((id, _)) = &active.pending {
                    ctx.forget_task_wait(*id);
                }
                active.cancel();
            }
        }
        self.reap(ctx);
        for invocation in ctx.task_invocations {
            let task = invocation.task_id;
            if invocation.request.kind == "ai.cancel" {
                if let Some(active) = &self.active {
                    let target = active.task;
                    let id = self.id();
                    ctx.cancel_task(id, target);
                    self.cancellations.push((id, task));
                } else {
                    ctx.bus.print_info("AI: idle");
                    self.event(
                        ctx,
                        task,
                        TaskEvent::Finished(TaskOutcome::success(
                            None,
                            patinae_plugin::tasks::TaskEffects::None,
                        )),
                    );
                }
                continue;
            }
            if invocation.request.kind == "ai.status" {
                ctx.bus.print_info(self.active.as_ref().map_or_else(
                    || "AI: idle".into(),
                    |active| format!("AI worker active: {}", active.task),
                ));
                self.event(
                    ctx,
                    task,
                    TaskEvent::Finished(TaskOutcome::success(
                        None,
                        patinae_plugin::tasks::TaskEffects::None,
                    )),
                );
                continue;
            }
            if self.active.is_some() {
                self.event(
                    ctx,
                    task,
                    TaskEvent::Finished(TaskOutcome::failure(
                        "busy",
                        "AI request already running; use ai status or ai cancel",
                    )),
                );
                continue;
            }
            if ctx.task_cancellations.contains(&task) {
                self.event(
                    ctx,
                    task,
                    TaskEvent::Finished(TaskOutcome::cancelled("cancelled before start")),
                );
                continue;
            }
            let prompt = invocation.request.payload["prompt"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            let scene = serde_json::json!({
                "objects": ctx.poll_shared.object_names,
                "selections": ctx.poll_shared.selection_names,
                "recent_atoms": ctx.poll_shared.pick_paths,
            });
            let (sender, requests) = mpsc::sync_channel(1);
            let (cancel, cancelled) = oneshot::channel();
            let config_path = patinae_settings::paths::plugin_dir().join("ai/config.toml");
            match std::thread::Builder::new()
                .name("patinae-ai".into())
                .spawn(move || worker::run(config_path, prompt, scene, sender, cancelled))
            {
                Ok(worker) => {
                    self.active = Some(Active {
                        task,
                        worker,
                        cancel: Some(cancel),
                        requests,
                        pending: None,
                    });
                    self.event(ctx, task, TaskEvent::Started);
                }
                Err(_) => self.event(
                    ctx,
                    task,
                    TaskEvent::Finished(TaskOutcome::failure(
                        "worker_start",
                        "cannot start AI worker",
                    )),
                ),
            }
        }
        let Some(active) = &mut self.active else {
            return;
        };
        if active.cancel.is_none() {
            return;
        }
        if let Some((id, _)) = &active.pending {
            let response = match ctx.task_result(*id) {
                Some(TaskQueryResult::Acknowledged(reply)) => {
                    Some(reply.clone().map(|_| HostReply::Acknowledged))
                }
                Some(TaskQueryResult::Command(reply)) => {
                    Some(reply.clone().map(HostReply::Command))
                }
                Some(TaskQueryResult::Wait(reply)) => Some(
                    reply
                        .clone()
                        .map(|snapshot| HostReply::Task(Box::new(snapshot))),
                ),
                Some(TaskQueryResult::Unavailable(error)) => {
                    Some(Err(TaskError::new("host_unavailable", error)))
                }
                _ => None,
            };
            if let Some(response) = response {
                let (_, sender) = active.pending.take().expect("pending request exists");
                let _ = sender.send(response);
            }
        }
        if active.pending.is_none() {
            if let Ok(request) = active.requests.try_recv() {
                self.next_id += 1;
                let id = self.next_id;
                match request.operation {
                    Operation::Command(command) => {
                        ctx.execute_task_command(id, active.task, &command, true)
                    }
                    Operation::Wait(child) => ctx.wait_task(id, child, Some(active.task), None),
                    Operation::Log(message) => ctx.report_task_event(
                        id,
                        active.task,
                        TaskEvent::Progress(patinae_plugin::tasks::TaskProgress {
                            phase: "ai".into(),
                            message,
                            completed: None,
                            total: None,
                        }),
                    ),
                }
                active.pending = Some((id, request.reply));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use patinae_framework::kernel::AppKernel;
    use patinae_plugin::tasks::TaskState;
    use patinae_plugin_host::PluginHost;
    use serde_json::{json, Value};
    use std::{
        io::{Read, Write},
        net::TcpListener,
        path::{Path, PathBuf},
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        time::{Duration, Instant},
    };

    // Opt-in ABI smoke test with a local Responses server and in-memory scene pixels.
    #[test]
    #[ignore = "requires PATINAE_AI_TEST_LIBRARY pointing to a freshly built ai-plugin"]
    fn dynamic_plugin_round_trips_commands_images_and_cancellation() {
        let library = PathBuf::from(
            std::env::var_os("PATINAE_AI_TEST_LIBRARY").expect("set PATINAE_AI_TEST_LIBRARY"),
        );
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("ai")).unwrap();
        let _environment = TestEnvironment::new(root.path());
        let server = Server::new();
        let mcp_server = Server::mcp();
        std::fs::write(
            root.path().join("ai/config.toml"),
            format!(
                "endpoint = {:?}\nmodel = 'fixture'\napi_key = 'fixture-key'\nmax_steps = 10\ntimeout_seconds = 5\n[mcp_servers.fixture]\nurl = {:?}\nheaders = {{ Authorization = 'Bearer mcp-fixture-key' }}\n",
                server.endpoint, mcp_server.endpoint
            ),
        ).unwrap();
        let mut kernel = AppKernel::new();
        kernel.executor.registry_mut().register(PartialWrite);
        let mut host = PluginHost::new();
        host.load_library(&library, &mut kernel.executor).unwrap();

        let task = start(&mut kernel, "ai inspect; preserve # literally");
        let request = receive(&server, &mut host, &mut kernel);
        assert!(request.get("messages").is_none());
        assert_eq!(request["store"], false);
        let context: Value =
            serde_json::from_str(request["input"][0]["content"].as_str().unwrap()).unwrap();
        assert_eq!(context["request"], "inspect; preserve # literally");
        assert_eq!(request["tools"][0]["name"], "command");
        assert_eq!(request["tools"][1]["name"], "capture_scene");
        assert_eq!(request["tools"][2]["name"], "mcp_fixture_1");
        assert_eq!(request["tools"][2]["strict"], false);

        let calls = tool_calls(&["fixture_partial_write".into(), "bg_color blue".into()]);
        kernel.output.clear();
        server.reply(calls.clone());
        let request = receive(&server, &mut host, &mut kernel);
        let results = feedback(&request);
        assert_eq!(results[0]["error"]["code"], "command_failed");
        assert!(results[0]["receipt"]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| message["text"] == "output before failure"));
        assert_eq!(results[1]["error"]["code"], "skipped_after_error");
        assert!(kernel.output.buffer.is_empty());
        assert_eq!(kernel.session.clear_color, [1.0, 0.0, 0.0]);
        // Preserve opaque output items and correlate each result to its tool call.
        assert_eq!(
            &request["input"].as_array().unwrap()[1..4],
            calls["output"].as_array().unwrap()
        );
        assert_eq!(request["input"][4]["call_id"], "call_0");
        assert_eq!(request["input"][5]["call_id"], "call_1");

        server.reply(tool_calls(&["bg_color blue".into()]));
        let request = receive(&server, &mut host, &mut kernel);
        assert_eq!(feedback(&request).last().unwrap()["ok"], true);
        assert_eq!(kernel.session.clear_color, [0.0, 0.0, 1.0]);

        let pixels = vec![17, 34, 51, 255, 99, 88, 77, 255];
        kernel.session.viewport_image = Some(patinae_scene::ViewportImage {
            width: 2,
            height: 1,
            data: pixels.clone(),
        });
        server.reply(capture_call());
        let request = receive(&server, &mut host, &mut kernel);
        assert_eq!(feedback(&request).last().unwrap()["ok"], true);
        let image = scene_pixels(&request);
        assert_eq!(image.dimensions(), (2, 1));
        assert_eq!(image.as_raw(), &pixels);
        assert!(kernel.output.buffer.is_empty());

        server.reply(json!({"status": "completed", "output": [
            {"type": "function_call", "call_id": "mcp_failure", "name": "mcp_fixture_1", "arguments": "{\"fail\":true}"},
            {"type": "function_call", "call_id": "skipped", "name": "command", "arguments": "{\"command\":\"bg_color red\"}"}
        ]}));
        let request = receive(&server, &mut host, &mut kernel);
        let results = feedback(&request);
        assert_eq!(results[results.len() - 2]["ok"], false);
        assert_eq!(
            results.last().unwrap()["error"]["code"],
            "skipped_after_error"
        );
        assert_eq!(kernel.session.clear_color, [0.0, 0.0, 1.0]);
        server.reply(json!({"status": "completed", "output": [
            {"type": "function_call", "call_id": "mcp_success", "name": "mcp_fixture_1", "arguments": "{\"query\":\"structure\"}"}
        ]}));
        let request = receive(&server, &mut host, &mut kernel);
        let results = feedback(&request);
        let result = results.last().unwrap();
        assert_eq!(result["ok"], true);
        assert_eq!(result["result"]["structuredContent"]["query"], "structure");
        assert_eq!(result["result"]["content"][0]["text"], "MCP fixture result");

        // A malformed command after MCP output must be correctable without
        // executing valid commands before or after it in the same response.
        server.reply(json!({"status": "completed", "output": [
            {"type": "function_call", "call_id": "before_invalid", "name": "command", "arguments": "{\"command\":\"bg_color red\"}"},
            {"type": "function_call", "call_id": "invalid", "name": "command", "arguments": "{\"code\":\"print('fixture')\"}"},
            {"type": "function_call", "call_id": "after_invalid", "name": "command", "arguments": "{\"command\":\"bg_color green\"}"}
        ]}));
        let request = receive(&server, &mut host, &mut kernel);
        let results = feedback(&request);
        let results = &results[results.len() - 3..];
        assert_eq!(results[0]["error"]["code"], "skipped_invalid_arguments");
        assert_eq!(results[1]["error"]["code"], "invalid_tool_arguments");
        assert_eq!(results[2]["error"]["code"], "skipped_invalid_arguments");
        assert_eq!(kernel.session.clear_color, [0.0, 0.0, 1.0]);
        assert_eq!(
            request["input"].as_array().unwrap().last().unwrap()["call_id"],
            "after_invalid"
        );
        server.reply(tool_calls(&["bg_color green".into()]));
        let request = receive(&server, &mut host, &mut kernel);
        assert_eq!(feedback(&request).last().unwrap()["ok"], true);
        assert_eq!(kernel.session.clear_color, [0.0, 1.0, 0.0]);
        server.reply(answer("Done"));
        let outcome = finish(task, &mut host, &mut kernel);
        assert_eq!(outcome.state, TaskState::Succeeded);
        assert!(matches!(outcome.outcome.unwrap().status,
            TaskOutcomeStatus::Success { data: Some(data) } if data.payload["answer"] == "Done"));

        let pending = start(&mut kernel, "ai wait for model");
        receive(&server, &mut host, &mut kernel);
        let cancel = start(&mut kernel, "ai cancel");
        assert_eq!(
            finish(cancel, &mut host, &mut kernel).state,
            TaskState::Succeeded
        );
        assert_eq!(
            finish(pending, &mut host, &mut kernel).state,
            TaskState::Cancelled
        );
    }

    struct PartialWrite;
    impl Command for PartialWrite {
        fn name(&self) -> &str {
            "fixture_partial_write"
        }
        fn execute<'v, 'r>(
            &self,
            ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
            _: &ParsedCommand,
        ) -> CmdResult {
            ctx.viewer.set_clear_color([1.0, 0.0, 0.0]);
            ctx.print("output before failure");
            Err(CmdError::execution("failure after write"))
        }
    }
    fn start(kernel: &mut AppKernel, command: &str) -> TaskId {
        let receipt = kernel.execute_command_captured(command, true, None, (320, 180));
        assert!(receipt.result.is_ok(), "{:?}", receipt.result);
        assert_eq!(receipt.output.task_ids.len(), 1);
        receipt.output.task_ids[0]
    }
    fn pump(host: &mut PluginHost, kernel: &mut AppKernel) {
        kernel.process_async_tasks(None, (320, 180));
        host.prepare_task_dispatch(kernel);
        let session = &kernel.session;
        let shared = SharedContext {
            tasks: Some(&kernel.tasks),
            registry: &session.registry,
            camera: &session.camera,
            selections: &session.selections,
            recent_atoms: &session.recent_atoms,
            named_palette: &session.named_palette,
            movie: &session.movie,
            settings: &session.settings,
            clear_color: session.clear_color,
            gpu_device: None,
            gpu_queue: None,
            scene_generation: 0,
            viewport_image: session.viewport_image.as_ref(),
            command_names: &[],
            command_registry: kernel.executor.registry(),
            setting_names: &[],
            dynamic_settings: None,
        };
        host.poll_all(&shared, &mut kernel.bus);
        host.apply_task_controls(kernel, None, (320, 180));
        kernel.process_messages(None, (320, 180));
    }
    fn finish(task: TaskId, host: &mut PluginHost, kernel: &mut AppKernel) -> TaskSnapshot {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            pump(host, kernel);
            let snapshot = kernel.tasks.get(task).unwrap();
            if snapshot.state.is_terminal() {
                return snapshot;
            }
            assert!(Instant::now() < deadline, "task timed out: {snapshot:?}");
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn receive(server: &Server, host: &mut PluginHost, kernel: &mut AppKernel) -> Value {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            pump(host, kernel);
            if let Ok(request) = server.requests.try_recv() {
                return request;
            }
            assert!(
                Instant::now() < deadline,
                "model request did not arrive: {:?}",
                kernel.tasks.list(&Default::default()).unwrap()
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn answer(text: &str) -> Value {
        json!({"status": "completed", "output": [{
            "id": "msg_answer", "type": "message", "role": "assistant", "status": "completed",
            "content": [{"type": "output_text", "text": text, "annotations": []}]
        }]})
    }
    fn feedback(request: &Value) -> Vec<Value> {
        request["input"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["type"] == "function_call_output")
            .map(|item| serde_json::from_str(item["output"].as_str().unwrap()).unwrap())
            .collect()
    }

    fn tool_calls(commands: &[String]) -> Value {
        let mut output = vec![json!({
            "id": "rs_fixture", "type": "reasoning", "summary": [],
            "encrypted_content": "opaque-reasoning-fixture",
        })];
        output.extend(commands.iter().enumerate().map(|(i, command)| {
            json!({
                "id": format!("fc_{i}"), "call_id": format!("call_{i}"),
                "type": "function_call", "status": "completed", "name": "command",
                "arguments": json!({"command": command}).to_string(),
            })
        }));
        json!({"status": "completed", "output": output})
    }
    fn capture_call() -> Value {
        json!({"status": "completed", "output": [{"type": "function_call",
            "name": "capture_scene", "call_id": "capture_call", "id": "fc_capture",
            "status": "completed", "arguments": "{}"}]})
    }

    fn scene_pixels(request: &Value) -> image::RgbaImage {
        use base64::Engine as _;
        let content = request["input"].as_array().unwrap().last().unwrap()["content"]
            .as_array()
            .unwrap_or_else(|| {
                panic!("missing scene image; tool results: {:?}", feedback(request))
            });
        assert!(content[0]["text"]
            .as_str()
            .unwrap()
            .contains("capture_call"));
        let url = content[1]["image_url"].as_str().unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(url.strip_prefix("data:image/png;base64,").unwrap())
            .unwrap();
        image::load_from_memory(&bytes).unwrap().to_rgba8()
    }
    struct TestEnvironment {
        previous: Option<std::ffi::OsString>,
    }
    impl TestEnvironment {
        fn new(root: &Path) -> Self {
            let previous = std::env::var_os("PATINAE_PLUGIN_DIR");
            // This is the only environment-dependent test; other tests use explicit config inputs.
            std::env::set_var("PATINAE_PLUGIN_DIR", root);
            Self { previous }
        }
    }
    impl Drop for TestEnvironment {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => std::env::set_var("PATINAE_PLUGIN_DIR", value),
                None => std::env::remove_var("PATINAE_PLUGIN_DIR"),
            }
        }
    }
    struct Server {
        endpoint: String,
        requests: mpsc::Receiver<Value>,
        responses: mpsc::Sender<Value>,
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
    }
    impl Server {
        fn new() -> Self {
            Self::start(false)
        }
        fn mcp() -> Self {
            Self::start(true)
        }
        fn start(mcp: bool) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let path = if mcp {
                "/mcp?api_key=fixture-query&option=foo%2Fbar"
            } else {
                "/v1/responses"
            };
            let endpoint = format!("http://{}{path}", listener.local_addr().unwrap());
            listener.set_nonblocking(true).unwrap();
            let (request_tx, requests) = mpsc::channel();
            let (responses, response_rx) = mpsc::channel::<Value>();
            let stop = Arc::new(AtomicBool::new(false));
            let stopping = Arc::clone(&stop);
            let thread = std::thread::spawn(move || {
                while !stopping.load(Ordering::Relaxed) {
                    let (mut socket, _) = match listener.accept() {
                        Ok(socket) => socket,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(2));
                            continue;
                        }
                        Err(error) => panic!("accept: {error}"),
                    };
                    socket.set_nonblocking(false).unwrap();
                    socket
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    let mut bytes = Vec::new();
                    let mut byte = [0];
                    while !bytes.ends_with(b"\r\n\r\n") {
                        socket.read_exact(&mut byte).unwrap();
                        bytes.push(byte[0]);
                    }
                    let header = String::from_utf8(bytes).unwrap();
                    if mcp && !header.starts_with("POST ") {
                        let _ = write!(socket, "HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                        continue;
                    }
                    // Query parameters, including their escaping, survive config
                    // validation and every MCP initialization/discovery/tool POST.
                    assert!(header.starts_with(&format!("POST {path} HTTP/1.1\r\n")));
                    assert!(header.lines().any(|line| {
                        line.split_once(':').is_some_and(|(name, value)| {
                            name.eq_ignore_ascii_case("authorization")
                                && value.trim()
                                    == if mcp {
                                        "Bearer mcp-fixture-key"
                                    } else {
                                        "Bearer fixture-key"
                                    }
                        })
                    }));
                    let length: usize = header
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|value| value.trim().parse().unwrap())
                        })
                        .unwrap();
                    let mut body = vec![0; length];
                    socket.read_exact(&mut body).unwrap();
                    if mcp {
                        let request: Value = serde_json::from_slice(&body).unwrap();
                        let result = match request["method"].as_str().unwrap() {
                            "initialize" => {
                                json!({"protocolVersion": request["params"]["protocolVersion"],
                                "capabilities": {"tools": {}}, "serverInfo": {"name": "http-fixture", "version": "1"}})
                            }
                            "notifications/initialized" => {
                                let _ = write!(socket, "HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                                continue;
                            }
                            "tools/list" => {
                                json!({"tools": [{"name": "lookup", "description": "Look up fixture data",
                                "inputSchema": {"type": "object", "properties": {"query": {"type": "string"}, "fail": {"type": "boolean"}}}}]})
                            }
                            "tools/call" => {
                                assert_eq!(request["params"]["name"], "lookup");
                                json!({"content": [{"type": "text", "text": "MCP fixture result"}],
                                    "structuredContent": request["params"]["arguments"],
                                    "isError": request["params"]["arguments"]["fail"] == true})
                            }
                            method => panic!("unexpected MCP method: {method}"),
                        };
                        let response =
                            json!({"jsonrpc": "2.0", "id": request["id"], "result": result})
                                .to_string();
                        let _ = write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response);
                        continue;
                    }
                    if request_tx
                        .send(serde_json::from_slice(&body).unwrap())
                        .is_err()
                    {
                        break;
                    }
                    let response = loop {
                        if stopping.load(Ordering::Relaxed) {
                            return;
                        }
                        if let Ok(response) = response_rx.recv_timeout(Duration::from_millis(10)) {
                            break response.to_string();
                        }
                    };
                    let _ = write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response);
                }
            });
            Self {
                endpoint,
                requests,
                responses,
                stop,
                thread: Some(thread),
            }
        }
        fn reply(&self, reply: Value) {
            self.responses.send(reply).unwrap();
        }
    }
    impl Drop for Server {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(thread) = self.thread.take() {
                let result = thread.join();
                if !std::thread::panicking() {
                    result.unwrap();
                }
            }
        }
    }
}
