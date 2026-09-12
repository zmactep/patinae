//! Application Kernel
//!
//! UI-agnostic domain orchestrator that owns the scene state, command system,
//! and message bus. Can be used headless (tests, CLI) or wrapped by any GUI
//! frontend (Slint, egui, web).

use lin_alg::f32::Vec3;
use patinae_cmd::{
    AnnotationOutcome, AnnotationRequest, AsyncCommandAcceptance, AsyncCommandRequest, CmdError,
    CommandAction, CommandExecution, CommandExecutor, CommandOutput, FetchRequest, ViewerLike,
};
use patinae_scene::{
    AnimationUpdate, CameraDelta, CaptureRenderer, Session, SessionAdapter, ViewportImage,
};

use crate::message::{AppMessage, MessageBus};
use crate::model::command_line::CommandLineModel;
use crate::model::output::OutputModel;
use crate::model::scene::SceneModel;
use crate::model::ViewportModel;
use crate::tasks::{native_task_runner, AsyncTask, NativeTaskExecutor, TaskRunner};

/// Host hook that maps command async requests to executable tasks.
pub type AsyncCommandHandler =
    Box<dyn FnMut(AsyncCommandRequest, u64) -> Option<Box<dyn AsyncTask>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandFailureOutput {
    Error,
    Warning,
}

/// UI-agnostic application core.
///
/// Owns the scene state, command system, and message bus.
/// Does **not** own GPU resources or UI state — those belong to the frontend.
pub struct AppKernel {
    pub session: Session,
    pub executor: CommandExecutor,
    pub bus: MessageBus,
    pub command_line: CommandLineModel,
    pub output: OutputModel,
    pub viewport: ViewportModel,
    pub scene: SceneModel,
    pub tasks: TaskRunner,
    task_executor: NativeTaskExecutor,
    async_command_handler: Option<AsyncCommandHandler>,
    task_invocations: Vec<patinae_cmd::TaskInvocation>,
    // Presentation labels only; lifecycle and elapsed time belong to TaskRunner.
    task_command_labels: std::collections::BTreeMap<patinae_cmd::tasks::TaskId, String>,
    command_parent: Option<patinae_cmd::tasks::TaskId>,
    pml_tasks: Vec<PmlExecution>,
    script_lineage: Vec<String>,
    needs_redraw: bool,
    command_generation: u64,
}

impl AppKernel {
    pub fn new() -> Self {
        Self {
            session: Session::new(),
            executor: CommandExecutor::new(),
            bus: MessageBus::new(),
            command_line: CommandLineModel::new(),
            output: OutputModel::new(),
            viewport: ViewportModel::new(),
            scene: SceneModel::new(),
            tasks: native_task_runner(),
            task_executor: NativeTaskExecutor::default(),
            async_command_handler: None,
            task_invocations: Vec::new(),
            task_command_labels: std::collections::BTreeMap::new(),
            command_parent: None,
            pml_tasks: Vec::new(),
            script_lineage: Vec::new(),
            needs_redraw: true,
            command_generation: 0,
        }
    }

    // =========================================================================
    // Command execution
    // =========================================================================

    /// Submit the current command-line input for execution.
    ///
    /// Takes the text from [`CommandLineModel`], adds it to history,
    /// and executes it. This is the method frontends should call when
    /// the user presses Enter in the REPL.
    pub fn submit_command<'a>(
        &'a mut self,
        render_context: Option<&'a mut (dyn CaptureRenderer + 'a)>,
        viewport_size: (u32, u32),
    ) {
        let cmd = self.command_line.take_command();
        if cmd.is_empty() {
            return;
        }
        self.command_line.add_to_history(cmd.clone());
        let _ = self.execute_command(&cmd, false, render_context, viewport_size);
    }

    /// Execute a command string.
    ///
    /// The command and its output are recorded in the [`OutputModel`].
    /// `render_context` is supplied by the frontend (pass `None` for headless).
    pub fn execute_command<'a>(
        &'a mut self,
        cmd: &str,
        quiet: bool,
        render_context: Option<&'a mut (dyn CaptureRenderer + 'a)>,
        viewport_size: (u32, u32),
    ) -> Result<CommandOutput, CmdError> {
        self.execute_command_with_failure_output(
            cmd,
            quiet,
            render_context,
            viewport_size,
            CommandFailureOutput::Error,
            false,
        )
        .into_result()
    }

    /// Execute a command while capturing messages independently of UI silence.
    ///
    /// Actions are applied here exactly once. The returned output is a report,
    /// not a request to dispatch those actions again. Silent execution retains
    /// error display but suppresses informational output, warnings, and echo.
    ///
    /// # Errors
    /// Returns the original parsing or command execution error.
    pub fn execute_command_captured<'a>(
        &'a mut self,
        cmd: &str,
        silent: bool,
        render_context: Option<&'a mut (dyn CaptureRenderer + 'a)>,
        viewport_size: (u32, u32),
    ) -> CommandExecution {
        self.execute_command_with_failure_output(
            cmd,
            silent,
            render_context,
            viewport_size,
            CommandFailureOutput::Error,
            true,
        )
    }

    /// Execute a command string and report failures as warnings.
    pub fn execute_command_warning_on_error<'a>(
        &'a mut self,
        cmd: &str,
        quiet: bool,
        render_context: Option<&'a mut (dyn CaptureRenderer + 'a)>,
        viewport_size: (u32, u32),
    ) -> Result<CommandOutput, CmdError> {
        self.execute_command_with_failure_output(
            cmd,
            quiet,
            render_context,
            viewport_size,
            CommandFailureOutput::Warning,
            false,
        )
        .into_result()
    }

    fn execute_command_with_failure_output<'a>(
        &'a mut self,
        cmd: &str,
        quiet: bool,
        render_context: Option<&'a mut (dyn CaptureRenderer + 'a)>,
        viewport_size: (u32, u32),
        failure_output: CommandFailureOutput,
        capture_output: bool,
    ) -> CommandExecution {
        if !quiet {
            self.output.print_command(cmd);
        }
        self.command_generation = self.command_generation.wrapping_add(1);

        let scene_epoch = self.session.task_epoch();
        let mut adapter = SessionAdapter {
            session: &mut self.session,
            render_context,
            default_size: viewport_size,
            needs_redraw: &mut self.needs_redraw,
        };
        let parent_id = self.command_parent;
        let pml_tasks = &mut self.pml_tasks;
        let script_lineage = self.script_lineage.clone();
        let plugin_executors = self.executor.task_executors().clone();
        let task_invocations = &mut self.task_invocations;
        let executor = &mut self.executor;
        let tasks = &self.tasks;
        let task_executor = &self.task_executor;
        let async_command_handler = &mut self.async_command_handler;
        let mut async_sink = |request: AsyncCommandRequest| {
            if let AsyncCommandRequest::RunScript { path } = request {
                let path = patinae_cmd::commands::io::expand_path(&path);
                let canonical = path.canonicalize().unwrap_or(path.clone());
                let execution = match patinae_cmd::script::ScriptExecution::new(
                    path.to_string_lossy().into_owned(),
                    canonical.to_string_lossy().into_owned(),
                    &script_lineage,
                ) {
                    Ok(execution) => execution,
                    Err(error) => return AsyncCommandAcceptance::Rejected(error),
                };
                let mut spec = patinae_cmd::tasks::TaskSpec::new("script", PML_EXECUTOR);
                spec.parent_id = parent_id;
                spec.silent = quiet;
                spec.message = format!("Running {}", path.display());
                return match tasks.admit(spec) {
                    Ok(task_id) => {
                        pml_tasks.push(PmlExecution { task_id, execution });
                        AsyncCommandAcceptance::Accepted(task_id)
                    }
                    Err(error) => AsyncCommandAcceptance::Rejected(error),
                };
            }

            if let AsyncCommandRequest::Plugin(mut request) = request {
                request.silent |= quiet;
                if !plugin_executors.contains(&request.executor) {
                    return AsyncCommandAcceptance::Rejected(
                        patinae_cmd::tasks::TaskStartError::ExecutorUnavailable,
                    );
                }
                return match admit_plugin_task(
                    tasks,
                    task_invocations,
                    request,
                    parent_id,
                    scene_epoch,
                ) {
                    Ok(id) => AsyncCommandAcceptance::Accepted(id),
                    Err(error) => AsyncCommandAcceptance::Rejected(error),
                };
            }

            let Some(handler) = async_command_handler.as_mut() else {
                return AsyncCommandAcceptance::Unsupported;
            };
            let Some(task) = handler(request, scene_epoch) else {
                return AsyncCommandAcceptance::Unsupported;
            };
            match task_executor.spawn_with_silent(tasks, task, parent_id, quiet) {
                Ok(id) => AsyncCommandAcceptance::Accepted(id),
                Err(error) => AsyncCommandAcceptance::Rejected(error),
            }
        };
        let mut execution = executor.execute_captured(
            &mut adapter,
            cmd,
            quiet && !capture_output,
            Some(&mut async_sink),
        );

        let output = &mut execution.output;
        if !quiet {
            for id in &output.task_ids {
                self.task_command_labels.insert(*id, cmd.to_owned());
            }
        }
        if execution.result.is_ok() || capture_output {
            for msg in &output.messages {
                if quiet && capture_output {
                    continue;
                }
                self.output.add(msg.clone().into());
            }
        }
        match &execution.result {
            Ok(()) => {
                if !quiet && output.task_ids.is_empty() {
                    if let Some(d) = output.duration {
                        self.output.print_timing(format_duration(d));
                    }
                }
                for action in &output.actions {
                    match action {
                        CommandAction::ShowPanel(id) => {
                            self.bus.send(AppMessage::ShowPanel(id.clone()));
                        }
                        CommandAction::HidePanel(id) => {
                            self.bus.send(AppMessage::HidePanel(id.clone()));
                        }
                        CommandAction::ClearOutput => {
                            self.output.clear();
                        }
                        CommandAction::Quit => {
                            self.bus.send(AppMessage::Quit);
                        }
                        CommandAction::RecordRecentFile { path, command } => {
                            self.bus.record_recent_file(path.clone(), command.clone());
                        }
                    }
                }
            }
            Err(e) => {
                match failure_output {
                    _ if quiet && capture_output => log::error!("Command failed: {e}"),
                    CommandFailureOutput::Error => self.output.print_error(e.to_string()),
                    CommandFailureOutput::Warning => self.output.print_warning(e.to_string()),
                }
                output
                    .messages
                    .push(patinae_cmd::OutputMessage::error(e.to_string()));
            }
        }

        execution
    }

    /// Executes one typed native annotation request.
    ///
    /// Successful mutations are reported through the normal output model and
    /// request a redraw through the session adapter. Failures are reported as
    /// errors without setting the redraw flag.
    ///
    /// # Errors
    ///
    /// Returns validation or execution errors from the command layer.
    pub fn execute_annotation_request(
        &mut self,
        request: &AnnotationRequest,
    ) -> Result<AnnotationOutcome, CmdError> {
        self.command_generation = self.command_generation.wrapping_add(1);
        let result = self.mutate_viewer(None, (1, 1), |viewer| {
            patinae_cmd::execute_annotation_request(viewer, request)
        });
        match &result {
            Ok(outcome) => self.output.print_info(format!(
                " Annotation updated in \"{}\"",
                outcome.object_name()
            )),
            Err(error) => self.output.print_error(error.to_string()),
        }
        result
    }

    pub fn command_generation(&self) -> u64 {
        self.command_generation
    }

    /// Admit plugin work before delivering its invocation to the assigned executor.
    pub fn start_plugin_task(
        &mut self,
        request: patinae_cmd::PluginTaskRequest,
        parent_id: Option<patinae_cmd::tasks::TaskId>,
    ) -> Result<patinae_cmd::tasks::TaskId, patinae_cmd::tasks::TaskStartError> {
        if !self.executor.task_executor_available(&request.executor) {
            return Err(patinae_cmd::tasks::TaskStartError::ExecutorUnavailable);
        }
        admit_plugin_task(
            &self.tasks,
            &mut self.task_invocations,
            request,
            parent_id,
            self.session.task_epoch(),
        )
    }

    /// Drain admitted invocations; consumers must route by request.executor.
    pub fn take_task_invocations(&mut self) -> Vec<patinae_cmd::TaskInvocation> {
        std::mem::take(&mut self.task_invocations)
    }

    /// Execute a task's acknowledged command with parent propagation.
    ///
    /// Uses the same renderer and viewport defaults as ordinary host commands.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid ownership, cancellation, or stale scene state.
    pub fn execute_task_command<'a>(
        &mut self,
        task_id: patinae_cmd::tasks::TaskId,
        owner: &str,
        command: &str,
        silent: bool,
        mut render_context: Option<&'a mut (dyn CaptureRenderer + 'a)>,
        viewport_size: (u32, u32),
    ) -> Result<CommandExecution, patinae_cmd::tasks::TaskError> {
        self.tasks
            .can_apply_effect(task_id, owner, self.session.task_epoch())?;
        let before = self.session.mutation_revision();
        let previous = self.command_parent.replace(task_id);
        let silent = silent || self.tasks.is_silent(task_id).unwrap_or(true);
        let execution = self.execute_command_captured(
            command,
            silent,
            match &mut render_context {
                Some(renderer) => Some(&mut **renderer),
                None => None,
            },
            viewport_size,
        );
        self.command_parent = previous;
        // Commands may change view or scene before returning an error.
        self.tasks.record_effects(
            task_id,
            owner,
            patinae_cmd::tasks::TaskEffects::between(before, self.session.mutation_revision()),
        )?;
        Ok(execution)
    }

    /// Present task output according to its inherited silent policy, always retaining logs.
    pub fn present_task_output(
        &mut self,
        task_id: patinae_cmd::tasks::TaskId,
        output: &patinae_cmd::tasks::TaskDiagnostic,
    ) {
        match output.level.as_str() {
            "error" => log::error!("Task {task_id}: {}", output.message.trim_end()),
            "warning" => log::warn!("Task {task_id}: {}", output.message.trim_end()),
            _ => log::info!("Task {task_id}: {}", output.message.trim_end()),
        }
        if self.tasks.is_silent(task_id).unwrap_or(true) {
            return;
        }
        match output.level.as_str() {
            "error" => self.output.print_error(output.message.trim_end()),
            "warning" => self.output.print_warning(output.message.trim_end()),
            _ => self.output.print_info(output.message.trim_end()),
        }
    }

    /// Install or replace the command async handler.
    pub fn set_async_command_handler(&mut self, handler: Option<AsyncCommandHandler>) {
        self.async_command_handler = handler;
    }

    /// Apply a plugin-supplied viewer mutation through the same adapter used
    /// by commands.
    pub fn mutate_viewer<'a, T>(
        &'a mut self,
        render_context: Option<&'a mut (dyn CaptureRenderer + 'a)>,
        viewport_size: (u32, u32),
        f: impl FnOnce(&mut dyn ViewerLike) -> T,
    ) -> T {
        let mut adapter = SessionAdapter {
            session: &mut self.session,
            render_context,
            default_size: viewport_size,
            needs_redraw: &mut self.needs_redraw,
        };
        f(&mut adapter)
    }

    /// Print a fetch failure from an async task.
    pub fn print_fetch_error(&mut self, request: &FetchRequest, error: impl Into<String>) {
        self.output.print_error(format!(
            "Fetch failed for {}: {}",
            request.code,
            error.into()
        ));
    }

    /// Apply a bounded batch of results, publishing completion after their effects.
    pub fn process_async_tasks<'a>(
        &mut self,
        render_context: Option<&'a mut (dyn CaptureRenderer + 'a)>,
        viewport_size: (u32, u32),
    ) -> bool {
        use patinae_cmd::tasks::{TaskError, TaskOutcome, TaskOutcomeStatus};
        let mut processed = false;
        // Leave remaining ready results in the runner to preserve event-loop service.
        for _ in 0..self.tasks.config().batch_size {
            let Some(ready) = self.task_executor.poll(&self.tasks) else {
                break;
            };
            processed = true;
            if !self.tasks.begin_apply(ready.id, self.session.task_epoch()) {
                continue;
            }
            let outcome = match ready.result.preflight(self) {
                Err(error) => TaskOutcome::failure(error.code, error.message),
                Ok(()) => ready.result.apply(self, ready.id),
            };
            let mut outcome = outcome;
            if let Some(error) = ready.runtime_failure {
                outcome.status = TaskOutcomeStatus::Failure {
                    error: TaskError::new("worker_failed", error),
                };
            }
            self.tasks.finish(ready.id, outcome);
        }
        processed |= self.process_pml_tasks(render_context, viewport_size);
        for change in self.tasks.take_changes() {
            if change.state == patinae_cmd::tasks::TaskState::Cancelled
                && !self.task_command_labels.contains_key(&change.id)
                && !self.tasks.is_silent(change.id).unwrap_or(true)
            {
                self.output
                    .print_warning(format!("Cancelled: task {}", change.id));
                processed = true;
            }
            crate::topics::publish(&mut self.bus, "patinae.tasks.changed", &change);
        }
        // Re-read retained state rather than depending on delivery of change hints.
        self.task_command_labels.retain(|id, command| {
            let Ok(snapshot) = self.tasks.get(*id) else {
                self.output
                    .print_warning(format!("Task history expired: {command}"));
                processed = true;
                return false;
            };
            if !snapshot.state.is_terminal() {
                return true;
            }
            if snapshot.state == patinae_cmd::tasks::TaskState::Cancelled {
                self.output.print_warning(format!("Cancelled: {command}"));
            }
            if let Ok(elapsed) = self.tasks.elapsed_ms(*id) {
                self.output.print_timing(format!(
                    "{command}: {}",
                    format_duration(std::time::Duration::from_millis(elapsed))
                ));
            }
            processed = true;
            false
        });
        processed
    }

    fn process_pml_tasks<'a>(
        &mut self,
        mut render_context: Option<&'a mut (dyn CaptureRenderer + 'a)>,
        viewport_size: (u32, u32),
    ) -> bool {
        use patinae_cmd::script::{resolve_file_include, ScriptAction};
        use patinae_cmd::tasks::TaskOutcome;
        let mut processed = 0;
        for mut script in std::mem::take(&mut self.pml_tasks) {
            if processed >= self.tasks.config().batch_size {
                self.pml_tasks.push(script);
                continue;
            }
            let mut action = script
                .execution
                .advance(&self.tasks, script.task_id, PML_EXECUTOR);
            if action == ScriptAction::ReadSource {
                let source = std::fs::read_to_string(script.execution.path())
                    .map_err(|e| {
                        patinae_cmd::tasks::TaskError::new("script_read_failed", e.to_string())
                    })
                    .and_then(|source| {
                        script
                            .execution
                            .set_source(&source, self.executor.registry())
                    });
                if let Err(error) = source {
                    self.tasks.finish(
                        script.task_id,
                        TaskOutcome::failure(error.code, error.message),
                    );
                    processed += 1;
                    continue;
                }
                let _ = self.tasks.started(script.task_id, PML_EXECUTOR);
                action = script
                    .execution
                    .advance(&self.tasks, script.task_id, PML_EXECUTOR);
            }
            match action {
                ScriptAction::Wait => self.pml_tasks.push(script),
                ScriptAction::Execute { line, command } => {
                    processed += 1;
                    let result = script
                        .execution
                        .resolve_command(&command, self.executor.registry(), resolve_file_include)
                        .and_then(|command| {
                            self.script_lineage = script.execution.lineage().to_vec();
                            let result = self.execute_task_command(
                                script.task_id,
                                PML_EXECUTOR,
                                &command,
                                false,
                                match &mut render_context {
                                    Some(renderer) => Some(&mut **renderer),
                                    None => None,
                                },
                                viewport_size,
                            );
                            self.script_lineage.clear();
                            result
                        });
                    let result = match result {
                        Ok(execution) => {
                            for message in &execution.output.messages {
                                let _ =
                                    self.tasks
                                        .output(script.task_id, PML_EXECUTOR, message.into());
                            }
                            execution.result.map_err(|e| e.to_string())
                        }
                        Err(error) => Err(error.to_string()),
                    };
                    if script
                        .execution
                        .complete_step(&self.tasks, script.task_id, line, result)
                    {
                        self.pml_tasks.push(script);
                    }
                }
                ScriptAction::Finished => processed += 1,
                ScriptAction::ReadSource => unreachable!("source installed above"),
            }
        }
        processed > 0
    }

    /// Admit native work through this session's task lifecycle.
    pub fn spawn_task(
        &self,
        task: impl AsyncTask,
    ) -> Result<patinae_cmd::tasks::TaskId, patinae_cmd::tasks::TaskStartError> {
        self.task_executor.spawn(&self.tasks, Box::new(task), None)
    }

    // =========================================================================
    // Per-frame processing
    // =========================================================================

    /// Apply accumulated input deltas to the session camera.
    ///
    /// Returns `true` if the camera moved (i.e. a redraw is warranted).
    pub fn process_input(&mut self, viewport_height: f32) -> bool {
        let deltas = self.viewport.input.take_camera_deltas();
        if deltas.is_empty() {
            return false;
        }

        let screen_vertex_scale = self.session.camera.screen_vertex_scale(viewport_height);

        for delta in deltas {
            match delta {
                CameraDelta::Rotate { x, y } => {
                    self.session.camera.rotate_x(x);
                    self.session.camera.rotate_y(y);
                }
                CameraDelta::Translate(v) => {
                    let s = screen_vertex_scale * self.viewport.input.pan_sensitivity;
                    self.session
                        .camera
                        .translate(Vec3::new(v.x * s, v.y * s, 0.0));
                }
                CameraDelta::Zoom(z) => {
                    self.session.camera.zoom(z);
                }
                CameraDelta::Clip { front, back } => {
                    let view = self.session.camera.view_mut();
                    view.clip_front = (view.clip_front + front).max(0.01);
                    view.clip_back = (view.clip_back + back).max(view.clip_front + 0.01);
                }
                CameraDelta::SlabScale(raw_delta) => {
                    let mws = self.session.settings.ui.mouse_wheel_scale;
                    let scale = (1.0 + 0.04 * mws * raw_delta).clamp(0.5, 2.0);
                    let view = self.session.camera.view_mut();
                    let distance = view.position.z;
                    let half = ((view.clip_back - view.clip_front) * 0.5).max(0.1);
                    let new_half = (half * scale).max(2.0);
                    view.clip_front = (distance - new_half).max(0.01);
                    view.clip_back = (distance + new_half).max(view.clip_front + 0.1);
                }
            }
        }

        true
    }

    /// Advance session-owned movie, rock, and camera animations.
    pub fn update_animations(&mut self, dt: f32) -> AnimationUpdate {
        let update = self.session.update_animations(dt);
        if update.needs_redraw {
            self.needs_redraw = true;
        }
        update
    }

    /// Drain the message bus and dispatch domain messages.
    ///
    /// Messages handled internally (commands, logging, redraw requests) are
    /// consumed. Everything else is returned so the frontend can act on it
    /// (e.g. `Quit`, `TogglePanel`, `FocusPanel`).
    pub fn process_messages<'a>(
        &'a mut self,
        mut render_context: Option<&'a mut (dyn CaptureRenderer + 'a)>,
        viewport_size: (u32, u32),
    ) -> Vec<AppMessage> {
        let messages = self.bus.drain_outbox();
        let mut unhandled = Vec::new();

        for msg in messages {
            match msg {
                AppMessage::ExecuteCommand { command, silent } => {
                    // Inline reborrow — the temporary `&mut dyn CaptureRenderer`
                    // lives only for the duration of `execute_command`, so the
                    // outer parameter borrow stays valid for the next iteration.
                    let result = self.execute_command(
                        &command,
                        silent,
                        match &mut render_context {
                            Some(r) => Some(&mut **r),
                            None => None,
                        },
                        viewport_size,
                    );
                    if let Err(e) = result {
                        log::error!("Command failed: {}", e);
                    }
                }
                AppMessage::RequestRedraw => {
                    self.needs_redraw = true;
                }
                AppMessage::PrintOutput(message) => {
                    match message.kind {
                        patinae_cmd::MessageKind::Info => log::info!("{}", message.text),
                        patinae_cmd::MessageKind::Warning => log::warn!("{}", message.text),
                        patinae_cmd::MessageKind::Error => log::error!("{}", message.text),
                    }
                    self.output.add(message.into());
                }
                AppMessage::PrintInfo(s) => {
                    log::info!("{}", s);
                    self.output.print_info(&s);
                }
                AppMessage::PrintWarning(s) => {
                    log::warn!("{}", s);
                    self.output.print_warning(&s);
                }
                AppMessage::PrintError(s) => {
                    log::error!("{}", s);
                    self.output.print_error(&s);
                }
                AppMessage::PrintCommand(s) => {
                    self.output.print_command(&s);
                }
                AppMessage::PrintTiming(s) => {
                    self.output.print_timing(&s);
                }
                AppMessage::PrintClear => {
                    self.output.clear();
                }
                AppMessage::SetViewportImage {
                    data,
                    width,
                    height,
                } => {
                    let expected_len = viewport_image_len(width, height);
                    if width == 0 || height == 0 || expected_len != Some(data.len()) {
                        log::warn!(
                            "Ignoring invalid viewport image: {}x{} with {} bytes",
                            width,
                            height,
                            data.len()
                        );
                    } else {
                        self.session.viewport_image = Some(ViewportImage {
                            data,
                            width,
                            height,
                        });
                        self.needs_redraw = true;
                    }
                }
                AppMessage::ClearViewportImage => {
                    self.session.viewport_image = None;
                    self.needs_redraw = true;
                }
                other => unhandled.push(other),
            }
        }

        unhandled
    }

    // =========================================================================
    // State management
    // =========================================================================

    /// Update camera aspect ratio for the given viewport dimensions.
    pub fn resize(&mut self, width: u32, height: u32) {
        let w = width.max(1);
        let h = height.max(1);
        self.session.camera.set_aspect(w as f32 / h as f32);
    }

    /// Sync viewport background color from the theme.
    ///
    /// Only applies if the user hasn't explicitly set a background color
    /// via the `bg_color` command.
    pub fn sync_clear_color(&mut self, theme_bg: [f32; 3]) {
        if !self.session.clear_color_set {
            self.session.clear_color = theme_bg;
        }
    }

    pub fn needs_redraw(&self) -> bool {
        self.needs_redraw
    }

    pub fn clear_redraw_flag(&mut self) {
        self.needs_redraw = false;
    }
}

const PML_EXECUTOR: &str = "native:pml";
struct PmlExecution {
    task_id: patinae_cmd::tasks::TaskId,
    execution: patinae_cmd::script::ScriptExecution,
}

fn admit_plugin_task(
    tasks: &TaskRunner,
    invocations: &mut Vec<patinae_cmd::TaskInvocation>,
    request: patinae_cmd::PluginTaskRequest,
    parent_id: Option<patinae_cmd::tasks::TaskId>,
    scene_epoch: u64,
) -> Result<patinae_cmd::tasks::TaskId, patinae_cmd::tasks::TaskStartError> {
    let mut spec = patinae_cmd::tasks::TaskSpec::new(&request.kind, request.owner());
    spec.origin = request.executor.clone();
    spec.parent_id = parent_id;
    spec.scene_epoch = request.scene_scoped.then_some(scene_epoch);
    spec.cancellable = request.cancellable;
    spec.child_failure_policy = request.child_failure_policy;
    spec.silent = request.silent;
    spec.message = request.kind.clone();
    let task_id = tasks.admit(spec)?;
    invocations.push(patinae_cmd::TaskInvocation { task_id, request });
    Ok(task_id)
}

impl Default for AppKernel {
    fn default() -> Self {
        Self::new()
    }
}

fn format_duration(d: std::time::Duration) -> String {
    let nanos = d.as_nanos() as f64;
    if nanos < 100_000.0 {
        format!("{:.1} µs", nanos / 1000.0)
    } else if nanos < 1_000_000.0 {
        format!("{} µs", d.as_micros())
    } else if nanos < 1_000_000_000.0 {
        format!("{:.1} ms", nanos / 1_000_000.0)
    } else {
        format!("{:.1} s", d.as_secs_f64())
    }
}

fn viewport_image_len(width: u32, height: u32) -> Option<usize> {
    (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lin_alg::f32::Vec3;
    use patinae_cmd::{AnnotationRequest, MeasurementRequest, MeasurementTarget};
    use patinae_mol::ObjectMolecule;
    use patinae_mol::{Atom, CoordSet, Element};
    use patinae_scene::{MeasurementKind, MoleculeObject};

    fn kernel_with_measurement_atoms() -> AppKernel {
        let mut kernel = AppKernel::new();
        let mut molecule = ObjectMolecule::new("source");
        molecule.add_atom(Atom::new("A", Element::Carbon));
        molecule.add_atom(Atom::new("B", Element::Carbon));
        molecule.add_coord_set(CoordSet::from_vec3(&[
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
        ]));
        kernel
            .session
            .registry
            .add(MoleculeObject::with_name(molecule, "source"));
        kernel
    }

    fn script_fixture(text: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "patinae-pml-{}-{}.pml",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn background_output_waits_for_completion_and_reports_cancellation_once() {
        use crate::model::output::OutputKind;
        use patinae_cmd::tasks::{TaskConfig, TaskEffects, TaskOutcome, TaskTime};
        use std::sync::{
            atomic::{AtomicU64, Ordering},
            Arc,
        };

        struct BackgroundCommand;
        impl patinae_cmd::Command for BackgroundCommand {
            fn name(&self) -> &str {
                "background"
            }
            fn execute<'v, 'r>(
                &self,
                ctx: &mut patinae_cmd::CommandContext<'v, 'r, dyn ViewerLike + 'v>,
                _args: &patinae_cmd::ParsedCommand,
            ) -> patinae_cmd::CmdResult {
                let mut request =
                    patinae_cmd::PluginTaskRequest::new("fixture", serde_json::Value::Null);
                request.executor = "fixture".into();
                ctx.request_task(AsyncCommandRequest::Plugin(request))?;
                Err(CmdError::execution("failure after admission"))
            }
        }

        for (outcome, cancelled) in [
            (TaskOutcome::success(None, TaskEffects::Applied), false),
            (TaskOutcome::failure("fixture", "failed"), false),
            (TaskOutcome::cancelled("stopped"), true),
        ] {
            let clock = Arc::new(AtomicU64::new(100));
            let source = clock.clone();
            let mut kernel = AppKernel::new();
            kernel.tasks = TaskRunner::new(
                11,
                TaskConfig::default(),
                Box::new(move || {
                    let ms = source.load(Ordering::Relaxed);
                    TaskTime {
                        unix_ms: ms,
                        monotonic_ms: ms,
                    }
                }),
            );
            kernel.executor.register_task_executor("fixture");
            kernel.executor.registry_mut().register(BackgroundCommand);
            // A partial command failure must retain presentation of accepted work.
            let receipt = kernel.execute_command_captured("background", false, None, (1, 1));
            assert!(receipt.result.is_err());
            let id = receipt.output.task_ids[0];
            kernel.process_async_tasks(None, (1, 1));
            assert!(!kernel
                .output
                .buffer
                .iter()
                .any(|m| m.kind == OutputKind::Timing));
            if cancelled {
                kernel.tasks.cancel(id).unwrap();
                kernel.process_async_tasks(None, (1, 1));
                assert!(!kernel
                    .output
                    .buffer
                    .iter()
                    .any(|m| m.text.starts_with("Cancelled:")));
            }
            clock.store(2100, Ordering::Relaxed);
            kernel.tasks.finish_owned(id, "fixture", outcome).unwrap();
            // Display still works when a notification hint was consumed elsewhere.
            kernel.tasks.take_changes();
            clock.store(9100, Ordering::Relaxed);
            kernel.process_async_tasks(None, (1, 1));
            let timings: Vec<_> = kernel
                .output
                .buffer
                .iter()
                .filter(|m| m.kind == OutputKind::Timing)
                .collect();
            assert_eq!(timings.len(), 1);
            assert_eq!(timings[0].text, "background: 2.0 s");
            assert_eq!(
                kernel
                    .output
                    .buffer
                    .iter()
                    .filter(|m| m.text == "Cancelled: background")
                    .count(),
                usize::from(cancelled)
            );
            let count = kernel.output.buffer.len();
            kernel
                .tasks
                .finish(id, TaskOutcome::success(None, TaskEffects::None));
            kernel.process_async_tasks(None, (1, 1));
            assert_eq!(kernel.output.buffer.len(), count);
            assert!(kernel.task_command_labels.is_empty());
        }
    }

    #[cfg(unix)]
    #[test]
    fn pml_symlink_preserves_include_base_and_detects_canonical_cycles() {
        let seed = script_fixture("");
        let dir = seed.with_extension("dir");
        std::fs::create_dir_all(dir.join("source")).unwrap();
        std::fs::create_dir_all(dir.join("entry")).unwrap();
        let original = dir.join("source/main.pml");
        let entry = dir.join("entry/main.pml");
        std::fs::write(&original, "@child.pml\ngroup after_include\n").unwrap();
        std::fs::write(dir.join("entry/child.pml"), "group correct_base\n").unwrap();
        std::fs::write(dir.join("source/child.pml"), "group wrong_base\n").unwrap();
        std::os::unix::fs::symlink(&original, &entry).unwrap();
        let mut kernel = AppKernel::new();
        let receipt = kernel.execute_command_captured(
            &format!("run {}", serde_json::to_string(&entry).unwrap()),
            true,
            None,
            (1, 1),
        );
        assert!(receipt.result.is_ok());
        let id = receipt.output.task_ids[0];
        for _ in 0..30 {
            kernel.process_async_tasks(None, (1, 1));
        }
        assert_eq!(
            kernel.tasks.get(id).unwrap().state,
            patinae_cmd::tasks::TaskState::Succeeded
        );
        assert!(kernel.session.registry.contains("correct_base"));
        assert!(kernel.session.registry.contains("after_include"));
        assert!(!kernel.session.registry.contains("wrong_base"));
        std::fs::write(&original, "@../source/main.pml\ngroup unreachable\n").unwrap();
        let receipt = kernel.execute_command_captured(
            &format!("run {}", serde_json::to_string(&entry).unwrap()),
            true,
            None,
            (1, 1),
        );
        let id = receipt.output.task_ids[0];
        for _ in 0..30 {
            kernel.process_async_tasks(None, (1, 1));
        }
        assert_eq!(
            kernel.tasks.get(id).unwrap().state,
            patinae_cmd::tasks::TaskState::Failed
        );
        assert!(!kernel.session.registry.contains("unreachable"));
        std::fs::remove_dir_all(dir).unwrap();
        std::fs::remove_file(seed).unwrap();
    }

    #[test]
    fn pml_timing_is_emitted_after_scene_changes_and_quiet_suppresses_it() {
        use crate::model::output::OutputKind;
        for quiet in [false, true] {
            let mut kernel = AppKernel::new();
            let path = script_fixture("group loaded\n");
            let receipt = kernel.execute_command_captured(
                &format!("run \"{}\"", path.display()),
                quiet,
                None,
                (1, 1),
            );
            let id = receipt.output.task_ids[0];
            assert!(!kernel
                .output
                .buffer
                .iter()
                .any(|m| m.kind == OutputKind::Timing));
            while !kernel.tasks.get(id).unwrap().state.is_terminal() {
                kernel.process_async_tasks(None, (1, 1));
            }
            assert!(kernel.session.registry.contains("loaded"));
            let timings = kernel
                .output
                .buffer
                .iter()
                .filter(|m| m.kind == OutputKind::Timing && m.text.starts_with("run "))
                .count();
            assert_eq!(timings, usize::from(!quiet));
            std::fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn pml_waits_for_child_and_survives_child_history_eviction() {
        use patinae_cmd::tasks::{TaskConfig, TaskEffects, TaskOutcome, TaskSpec, TaskTime};
        let mut kernel = AppKernel::new();
        kernel.tasks = TaskRunner::new(
            11,
            TaskConfig {
                max_terminal: 1,
                ..Default::default()
            },
            Box::new(TaskTime::default),
        );
        kernel.executor.register_task_executor("fixture");
        kernel
            .executor
            .registry_mut()
            .register(patinae_cmd::DynamicCommand::new(
                "background".into(),
                "fixture".into(),
                String::new(),
                String::new(),
                "fixture".into(),
                None,
            ));
        let path = script_fixture("background; group after\n");
        let receipt = kernel.execute_command_captured(
            &format!("run \"{}\"", path.display()),
            true,
            None,
            (1, 1),
        );
        assert!(receipt.result.is_ok());
        let parent = receipt.output.task_ids[0];
        assert!(!kernel.session.registry.contains("after"));
        kernel.process_async_tasks(None, (1, 1));
        let invocation = kernel.take_task_invocations().remove(0);
        assert_eq!(
            kernel.tasks.get(invocation.task_id).unwrap().parent_id,
            Some(parent)
        );
        kernel.process_async_tasks(None, (1, 1));
        assert!(!kernel.session.registry.contains("after"));
        kernel
            .tasks
            .finish_owned(
                invocation.task_id,
                "fixture",
                TaskOutcome::success(None, TaskEffects::None),
            )
            .unwrap();
        let other = kernel
            .tasks
            .admit(TaskSpec::new("other", "fixture"))
            .unwrap();
        kernel
            .tasks
            .finish(other, TaskOutcome::success(None, TaskEffects::None));
        assert!(kernel.tasks.get(invocation.task_id).is_err());
        kernel.process_async_tasks(None, (1, 1));
        assert!(kernel.session.registry.contains("after"));
        kernel.process_async_tasks(None, (1, 1));
        assert_eq!(
            kernel.tasks.get(parent).unwrap().state,
            patinae_cmd::tasks::TaskState::Succeeded
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn pml_cancel_before_start_does_not_apply_commands() {
        let mut kernel = AppKernel::new();
        let path = script_fixture("group should_not_exist\n");
        let execution = kernel.execute_command_captured(
            &format!("run \"{}\"", path.display()),
            true,
            None,
            (1, 1),
        );
        let id = execution.output.task_ids[0];
        kernel.tasks.cancel(id).unwrap();
        kernel.process_async_tasks(None, (1, 1));
        assert_eq!(
            kernel.tasks.get(id).unwrap().state,
            patinae_cmd::tasks::TaskState::Cancelled
        );
        assert!(!kernel.session.registry.contains("should_not_exist"));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn silent_commands_and_script_children_keep_output_only_in_receipts() {
        let mut kernel = AppKernel::new();
        for command in ["help", "nonexistent_command"] {
            let result = kernel.execute_command_captured(command, true, None, (1, 1));
            assert!(!result.output.messages.is_empty());
            assert!(kernel.output.buffer.is_empty());
        }
        let path = script_fixture("help\nnonexistent_command\n");
        let result = kernel.execute_command_captured(
            &format!("run \"{}\"", path.display()),
            true,
            None,
            (1, 1),
        );
        let task = result.output.task_ids[0];
        for _ in 0..8 {
            kernel.process_async_tasks(None, (1, 1));
        }
        let result = kernel.tasks.get(task).unwrap();
        assert_eq!(result.state, patinae_cmd::tasks::TaskState::Failed);
        assert!(!result.diagnostics.is_empty());
        assert!(
            kernel.output.buffer.is_empty(),
            "{:?}",
            kernel.output.buffer
        );
        kernel.execute_command_captured("nonexistent_command", false, None, (1, 1));
        assert!(!kernel.output.buffer.is_empty());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn task_commands_distinguish_reads_and_untracked_writes() {
        use patinae_cmd::tasks::{TaskEffects, TaskSpec};
        let mut kernel = AppKernel::new();
        let id = kernel
            .tasks
            .admit(TaskSpec::new("script", "fixture"))
            .unwrap();
        let execution = kernel
            .execute_task_command(id, "fixture", "help", true, None, (1, 1))
            .unwrap();
        assert!(execution.result.is_ok());
        assert_eq!(kernel.tasks.get(id).unwrap().effects, TaskEffects::None);
        let execution = kernel
            .execute_task_command(id, "fixture", "group inserted", true, None, (1, 1))
            .unwrap();
        assert!(execution.result.is_ok());
        assert_eq!(kernel.tasks.get(id).unwrap().effects, TaskEffects::Unknown);
        assert!(kernel
            .execute_task_command(id, "foreign", "group forbidden", true, None, (1, 1))
            .is_err());
        assert!(!kernel.session.registry.contains("forbidden"));
    }

    #[test]
    fn task_color_effects_track_actual_writes_and_terminal_disposition() {
        use patinae_cmd::tasks::{TaskEffects, TaskOutcome, TaskSpec};
        let mut kernel = kernel_with_measurement_atoms();
        for (command, expected) in [
            ("color red", TaskEffects::Applied),
            ("color red", TaskEffects::None),
            ("color blue, none", TaskEffects::None),
            ("color blue, name A", TaskEffects::Applied),
            ("color blue, name A", TaskEffects::None),
            ("bg_color red", TaskEffects::Applied),
            ("bg_color red", TaskEffects::None),
            ("bg_color", TaskEffects::Applied),
            ("bg_color", TaskEffects::None),
            (
                "set_color experiment, [0.1, 0.2, 0.3]",
                TaskEffects::Applied,
            ),
            ("set_color experiment, [0.1, 0.2, 0.3]", TaskEffects::None),
        ] {
            let id = kernel
                .tasks
                .admit(TaskSpec::new("script", "fixture"))
                .unwrap();
            let execution = kernel
                .execute_task_command(id, "fixture", command, true, None, (1, 1))
                .unwrap();
            assert!(
                execution.result.is_ok(),
                "{command}: {:?}",
                execution.result
            );
            assert_eq!(kernel.tasks.get(id).unwrap().effects, expected, "{command}");
            kernel
                .tasks
                .finish_owned(id, "fixture", TaskOutcome::success(None, TaskEffects::None))
                .unwrap();
            assert_eq!(kernel.tasks.get(id).unwrap().effects, expected, "{command}");
        }
        // Palette insertion precedes selection validation and remains observable
        // even if the command subsequently fails.
        let id = kernel
            .tasks
            .admit(TaskSpec::new("script", "fixture"))
            .unwrap();
        let execution = kernel
            .execute_task_command(id, "fixture", "color 0x123456, (", true, None, (1, 1))
            .unwrap();
        assert!(execution.result.is_err());
        assert_eq!(kernel.tasks.get(id).unwrap().effects, TaskEffects::Applied);
        kernel
            .tasks
            .finish_owned(id, "fixture", TaskOutcome::failure("test", "failed"))
            .unwrap();
        assert_eq!(kernel.tasks.get(id).unwrap().effects, TaskEffects::Partial);

        let id = kernel
            .tasks
            .admit(TaskSpec::new("script", "fixture"))
            .unwrap();
        kernel
            .execute_task_command(id, "fixture", "color green", true, None, (1, 1))
            .unwrap();
        kernel.tasks.cancel(id).unwrap();
        kernel
            .tasks
            .finish_owned(id, "fixture", TaskOutcome::success(None, TaskEffects::None))
            .unwrap();
        assert_eq!(kernel.tasks.get(id).unwrap().effects, TaskEffects::Partial);
        assert!(kernel
            .execute_task_command(id, "fixture", "color red", true, None, (1, 1))
            .is_err());
    }

    #[test]
    fn raw_task_access_is_unknown_and_does_not_taint_later_commands() {
        use patinae_cmd::tasks::{TaskEffects, TaskOutcome, TaskSpec};
        struct Probe;
        impl patinae_cmd::Command for Probe {
            fn name(&self) -> &str {
                "probe_access"
            }
            fn execute<'v, 'r>(
                &self,
                ctx: &mut patinae_cmd::CommandContext<'v, 'r, dyn ViewerLike + 'v>,
                args: &patinae_cmd::ParsedCommand,
            ) -> patinae_cmd::CmdResult {
                match args.str_arg_or(0, "mode", "read") {
                    "read" => {
                        let _ = ctx.viewer.camera();
                    }
                    "redraw" => ctx.viewer.request_redraw(),
                    "borrow" => {
                        let _ = ctx.viewer.camera_mut();
                    }
                    "write" => ctx.viewer.camera_mut().view_mut().origin.x += 1.0,
                    _ => unreachable!(),
                }
                Ok(())
            }
        }
        let mut kernel = AppKernel::new();
        kernel.executor.registry_mut().register(Probe);
        for (mode, expected) in [
            ("borrow", TaskEffects::Unknown),
            ("write", TaskEffects::Unknown),
            ("read", TaskEffects::None),
            ("redraw", TaskEffects::None),
        ] {
            let id = kernel
                .tasks
                .admit(TaskSpec::new("script", "fixture"))
                .unwrap();
            kernel
                .execute_task_command(
                    id,
                    "fixture",
                    &format!("probe_access {mode}"),
                    true,
                    None,
                    (1, 1),
                )
                .unwrap();
            assert_eq!(kernel.tasks.get(id).unwrap().effects, expected);
            kernel
                .tasks
                .finish_owned(id, "fixture", TaskOutcome::failure("test", "failed"))
                .unwrap();
            assert_eq!(kernel.tasks.get(id).unwrap().effects, expected);
        }
        for commands in [
            ["probe_access borrow", "bg_color blue"],
            ["bg_color red", "probe_access borrow"],
        ] {
            let id = kernel
                .tasks
                .admit(TaskSpec::new("script", "fixture"))
                .unwrap();
            for command in commands {
                let execution = kernel
                    .execute_task_command(id, "fixture", command, true, None, (1, 1))
                    .unwrap();
                assert!(execution.result.is_ok());
            }
            assert_eq!(kernel.tasks.get(id).unwrap().effects, TaskEffects::Unknown);
        }
    }

    #[test]
    fn task_effects_include_writes_before_failure_and_session_replacement() {
        use patinae_cmd::tasks::{TaskEffects, TaskOutcome, TaskSpec};
        struct PartialWrite;
        impl patinae_cmd::Command for PartialWrite {
            fn name(&self) -> &str {
                "partial_write"
            }
            fn execute<'v, 'r>(
                &self,
                ctx: &mut patinae_cmd::CommandContext<'v, 'r, dyn ViewerLike + 'v>,
                _args: &patinae_cmd::ParsedCommand,
            ) -> patinae_cmd::CmdResult {
                ctx.viewer.set_clear_color([1.0, 0.0, 0.0]);
                Err(CmdError::execution("failure after write"))
            }
        }
        let mut kernel = AppKernel::new();
        kernel.executor.registry_mut().register(PartialWrite);
        for command in ["partial_write", "reinitialize"] {
            let id = kernel
                .tasks
                .admit(TaskSpec::new("script", "fixture"))
                .unwrap();
            let execution = kernel
                .execute_task_command(id, "fixture", command, true, None, (1, 1))
                .unwrap();
            assert_eq!(execution.result.is_err(), command == "partial_write");
            assert_eq!(kernel.tasks.get(id).unwrap().effects, TaskEffects::Applied);
            kernel
                .tasks
                .finish_owned(id, "fixture", TaskOutcome::failure("test", "failed"))
                .unwrap();
            assert_eq!(kernel.tasks.get(id).unwrap().effects, TaskEffects::Partial);
        }
        for command in ["", "help", "color not_a_color", "view missing_view, recall"] {
            let id = kernel
                .tasks
                .admit(TaskSpec::new("script", "fixture"))
                .unwrap();
            kernel
                .execute_task_command(id, "fixture", command, true, None, (1, 1))
                .unwrap();
            assert_eq!(
                kernel.tasks.get(id).unwrap().effects,
                TaskEffects::None,
                "{command}"
            );
        }
    }

    #[test]
    fn typed_annotation_success_reports_output_and_requests_redraw() {
        let mut kernel = kernel_with_measurement_atoms();
        kernel.clear_redraw_flag();
        let request = AnnotationRequest::Measurement(MeasurementRequest::new(
            ["name A", "name B"],
            MeasurementTarget::New,
        ));

        let outcome = kernel.execute_annotation_request(&request).unwrap();

        assert_eq!(outcome.object_name(), "distance01");
        assert_eq!(
            kernel
                .session
                .registry
                .get_measurement("distance01")
                .unwrap()
                .kind(),
            MeasurementKind::Distance
        );
        assert!(kernel.needs_redraw());
        assert!(kernel
            .output
            .buffer
            .back()
            .unwrap()
            .text
            .contains("distance01"));
    }

    #[test]
    fn typed_annotation_failure_reports_error_without_redraw() {
        use crate::model::output::OutputKind;

        let mut kernel = kernel_with_measurement_atoms();
        kernel.clear_redraw_flag();
        let request = AnnotationRequest::Measurement(MeasurementRequest::new(
            ["all", "name B"],
            MeasurementTarget::New,
        ));

        assert!(kernel.execute_annotation_request(&request).is_err());

        assert!(!kernel.needs_redraw());
        assert_eq!(kernel.output.buffer.back().unwrap().kind, OutputKind::Error);
        assert!(kernel
            .session
            .registry
            .get_measurement("distance01")
            .is_none());
    }

    #[test]
    fn kernel_creates_with_defaults() {
        let kernel = AppKernel::new();
        assert!(kernel.needs_redraw());
    }

    #[test]
    fn execute_command_headless() {
        let mut kernel = AppKernel::new();
        // Unknown command should error gracefully
        let result = kernel.execute_command("nonexistent_cmd", true, None, (800, 600));
        assert!(result.is_err());
    }

    #[test]
    fn execute_command_error_appears_in_output() {
        use crate::model::output::OutputKind;

        let mut kernel = AppKernel::new();
        let initial_len = kernel.output.buffer.len();
        let _ = kernel.execute_command("nonexistent_cmd", false, None, (800, 600));
        // Should have at least the command echo + the error
        assert!(kernel.output.buffer.len() >= initial_len + 2);
        assert_eq!(
            kernel
                .output
                .buffer
                .back()
                .expect("failure output should be present")
                .kind,
            OutputKind::Error
        );
    }

    #[test]
    fn command_failure_can_be_reported_as_warning() {
        use crate::model::output::OutputKind;

        let mut kernel = AppKernel::new();

        let result =
            kernel.execute_command_warning_on_error("nonexistent_cmd", false, None, (800, 600));

        assert!(result.is_err());
        assert_eq!(
            kernel
                .output
                .buffer
                .back()
                .expect("failure output should be present")
                .kind,
            OutputKind::Warning
        );
    }

    #[test]
    fn execute_command_quiet_skips_echo() {
        use crate::model::output::OutputKind;
        let mut kernel = AppKernel::new();
        let _ = kernel.execute_command("nonexistent_cmd", true, None, (800, 600));
        // No Command-kind entry (quiet suppresses the echo)
        let has_echo = kernel
            .output
            .buffer
            .iter()
            .any(|m| m.kind == OutputKind::Command);
        assert!(!has_echo);
    }

    #[test]
    fn execute_command_quiet_skips_timing() {
        use crate::model::output::OutputKind;
        let mut kernel = AppKernel::new();
        let _ = kernel.execute_command("set sphere_scale, 0.5", true, None, (800, 600));

        let has_timing = kernel
            .output
            .buffer
            .iter()
            .any(|m| m.kind == OutputKind::Timing);
        assert!(!has_timing);
    }

    #[test]
    fn execute_clear_command_clears_output_log() {
        let mut kernel = AppKernel::new();
        kernel.output.print_info("old line");

        kernel
            .execute_command("clear", false, None, (800, 600))
            .unwrap();

        assert!(kernel.output.buffer.is_empty());
    }

    #[test]
    fn reinitialize_objects_clears_named_selections() {
        let mut kernel = AppKernel::new();
        kernel.session.selections.define("sele", "all");

        kernel
            .execute_command("reinitialize objects", true, None, (800, 600))
            .unwrap();

        assert!(kernel.session.registry.is_empty());
        assert!(kernel.session.selections.is_empty());
    }

    #[test]
    fn print_messages_routed_to_output() {
        use crate::model::output::OutputKind;
        let mut kernel = AppKernel::new();
        kernel.bus.print_info("hello info");
        kernel.bus.print_warning("hello warn");
        kernel.bus.print_error("hello err");
        kernel.bus.print_timing("1.0 ms");
        let initial_len = kernel.output.buffer.len();

        kernel.process_messages(None, (800, 600));

        let new_msgs: Vec<_> = kernel.output.buffer.iter().skip(initial_len).collect();
        assert_eq!(new_msgs.len(), 4);
        assert_eq!(new_msgs[0].kind, OutputKind::Info);
        assert_eq!(new_msgs[0].text, "hello info");
        assert_eq!(new_msgs[1].kind, OutputKind::Warning);
        assert_eq!(new_msgs[1].text, "hello warn");
        assert_eq!(new_msgs[2].kind, OutputKind::Error);
        assert_eq!(new_msgs[2].text, "hello err");
        assert_eq!(new_msgs[3].kind, OutputKind::Timing);
        assert_eq!(new_msgs[3].text, "1.0 ms");
    }

    #[test]
    fn print_clear_clears_output() {
        let mut kernel = AppKernel::new();
        kernel.output.print_info("old line");
        kernel.bus.print_clear();

        let unhandled = kernel.process_messages(None, (800, 600));

        assert!(unhandled.is_empty());
        assert!(kernel.output.buffer.is_empty());
    }

    #[test]
    fn bus_preserves_markdown_format_and_severity() {
        let mut kernel = AppKernel::new();
        kernel.bus.print_markdown("# Heading");
        kernel.bus.print_info("**literal**");
        kernel.bus.print_message(
            patinae_cmd::OutputMessage::warning("**warning**")
                .with_format(patinae_cmd::OutputFormat::Markdown),
        );
        assert!(kernel.process_messages(None, (800, 600)).is_empty());
        let messages: Vec<_> = kernel.output.buffer.iter().collect();
        assert_eq!(messages[0].format, patinae_cmd::OutputFormat::Markdown);
        assert_eq!(messages[1].format, patinae_cmd::OutputFormat::Text);
        assert_eq!(messages[2].kind, crate::model::output::OutputKind::Warning);
        assert_eq!(messages[2].format, patinae_cmd::OutputFormat::Markdown);
    }

    #[test]
    fn process_messages_dispatches_commands() {
        let mut kernel = AppKernel::new();
        kernel.bus.execute_command("set sphere_scale, 0.5");
        kernel.bus.request_redraw();

        let unhandled = kernel.process_messages(None, (800, 600));
        assert!(kernel.needs_redraw());
        assert!(unhandled.is_empty());
    }

    #[test]
    fn process_messages_applies_viewport_image() {
        let mut kernel = AppKernel::new();
        kernel.clear_redraw_flag();

        kernel.bus.set_viewport_image(vec![255; 8], 2, 1);
        let unhandled = kernel.process_messages(None, (800, 600));

        assert!(unhandled.is_empty());
        assert!(kernel.needs_redraw());
        let image = kernel.session.viewport_image.as_ref().unwrap();
        assert_eq!(image.width, 2);
        assert_eq!(image.height, 1);
        assert_eq!(image.data, vec![255; 8]);
    }

    #[test]
    fn process_messages_ignores_invalid_viewport_image() {
        let mut kernel = AppKernel::new();

        kernel.bus.set_viewport_image(vec![255; 7], 2, 1);
        let unhandled = kernel.process_messages(None, (800, 600));

        assert!(unhandled.is_empty());
        assert!(kernel.session.viewport_image.is_none());

        kernel
            .bus
            .set_viewport_image(Vec::new(), u32::MAX, u32::MAX);
        let unhandled = kernel.process_messages(None, (800, 600));

        assert!(unhandled.is_empty());
        assert!(kernel.session.viewport_image.is_none());
    }

    #[test]
    fn process_messages_clears_viewport_image() {
        let mut kernel = AppKernel::new();
        kernel.session.viewport_image = Some(ViewportImage {
            data: vec![255; 4],
            width: 1,
            height: 1,
        });
        kernel.clear_redraw_flag();

        kernel.bus.clear_viewport_image();
        let unhandled = kernel.process_messages(None, (800, 600));

        assert!(unhandled.is_empty());
        assert!(kernel.needs_redraw());
        assert!(kernel.session.viewport_image.is_none());
    }

    #[test]
    fn update_animations_marks_redraw_when_movie_advances() {
        let mut kernel = AppKernel::new();
        kernel.session.settings.movie.movie_fps = 10.0;
        kernel.session.movie.set_frame_count(2);
        kernel.session.movie.play();
        kernel.clear_redraw_flag();

        let update = kernel.update_animations(0.11);

        assert!(update.movie_frame_changed);
        assert!(kernel.needs_redraw());
    }

    #[test]
    fn process_messages_returns_unhandled() {
        let mut kernel = AppKernel::new();
        kernel.bus.send(AppMessage::Quit);
        kernel.bus.send(AppMessage::TogglePanel("objects".into()));

        let unhandled = kernel.process_messages(None, (800, 600));
        assert_eq!(unhandled.len(), 2);
    }

    #[test]
    fn quit_commands_emit_unhandled_quit_message() {
        for command in ["quit", "exit"] {
            let mut kernel = AppKernel::new();

            kernel
                .execute_command(command, true, None, (800, 600))
                .unwrap();
            let unhandled = kernel.process_messages(None, (800, 600));

            assert!(
                matches!(unhandled.as_slice(), [AppMessage::Quit]),
                "{command} should emit exactly one AppMessage::Quit"
            );
        }
    }

    #[test]
    fn successful_load_emits_recent_file_action() {
        let mut kernel = AppKernel::new();
        let fixture = tempfile::Builder::new()
            .suffix(".pdb")
            .tempfile()
            .expect("temporary PDB fixture should be created");
        std::fs::write(
            fixture.path(),
            b"ATOM      1  CA  ALA A   1       0.000   0.000   0.000  1.00 20.00           C  \nEND\n",
        )
        .expect("temporary PDB fixture should be written");
        let path = std::fs::canonicalize(fixture.path())
            .expect("load fixture should have an absolute path");
        let command = format!("load {}", path.display());

        kernel
            .execute_command(&command, true, None, (800, 600))
            .expect("load should succeed");

        let unhandled = kernel.process_messages(None, (800, 600));
        assert!(matches!(
            unhandled.as_slice(),
            [AppMessage::RecordRecentFile { path: recorded, command }]
                if recorded == path.to_string_lossy().as_ref() && command == "load"
        ));
    }

    #[test]
    fn failed_load_does_not_emit_recent_file_action() {
        let mut kernel = AppKernel::new();

        let result = kernel.execute_command(
            "load /tmp/patinae-missing-recent-file-fixture.pdb",
            true,
            None,
            (800, 600),
        );

        assert!(result.is_err());
        let unhandled = kernel.process_messages(None, (800, 600));
        assert!(!unhandled
            .iter()
            .any(|msg| matches!(msg, AppMessage::RecordRecentFile { .. })));
    }

    #[test]
    fn load_traj_hint_does_not_emit_recent_file_action() {
        let mut kernel = AppKernel::new();

        let result = kernel.execute_command(
            "load /tmp/patinae-missing-recent-file-fixture.xtc",
            true,
            None,
            (800, 600),
        );

        assert!(result.is_err());
        let unhandled = kernel.process_messages(None, (800, 600));
        assert!(!unhandled
            .iter()
            .any(|msg| matches!(msg, AppMessage::RecordRecentFile { .. })));
    }

    #[test]
    fn sync_clear_color_respects_explicit_set() {
        let mut kernel = AppKernel::new();
        kernel.session.clear_color_set = true;
        kernel.session.clear_color = [1.0, 0.0, 0.0];
        kernel.sync_clear_color([0.0, 0.0, 0.0]);
        assert_eq!(kernel.session.clear_color, [1.0, 0.0, 0.0]);
    }

    #[test]
    fn sync_clear_color_applies_when_not_set() {
        let mut kernel = AppKernel::new();
        kernel.sync_clear_color([0.1, 0.2, 0.3]);
        assert_eq!(kernel.session.clear_color, [0.1, 0.2, 0.3]);
    }

    #[test]
    fn submit_command_takes_input_and_adds_history() {
        let mut kernel = AppKernel::new();
        kernel.command_line.input = "set sphere_scale, 0.5".into();

        kernel.submit_command(None, (800, 600));

        assert!(kernel.command_line.input.is_empty());
        assert_eq!(kernel.command_line.history, vec!["set sphere_scale, 0.5"]);
    }

    #[test]
    fn submit_command_empty_is_noop() {
        let mut kernel = AppKernel::new();
        let output_len = kernel.output.buffer.len();

        kernel.submit_command(None, (800, 600));

        assert!(kernel.command_line.history.is_empty());
        assert_eq!(kernel.output.buffer.len(), output_len);
    }

    #[test]
    fn submit_command_echoes_and_records_output() {
        use crate::model::output::OutputKind;
        let mut kernel = AppKernel::new();
        kernel.command_line.input = "bogus_cmd".into();
        let initial_len = kernel.output.buffer.len();

        kernel.submit_command(None, (800, 600));

        let new_msgs: Vec<_> = kernel.output.buffer.iter().skip(initial_len).collect();
        assert!(new_msgs.len() >= 2); // echo + error
        assert_eq!(new_msgs[0].kind, OutputKind::Command);
        assert!(new_msgs[0].text.contains("bogus_cmd"));
    }

    #[test]
    fn resize_updates_aspect() {
        let mut kernel = AppKernel::new();
        kernel.resize(1920, 1080);
        let aspect = 1920.0_f32 / 1080.0;
        assert!((kernel.session.camera.aspect() - aspect).abs() < 0.001);
    }
}
