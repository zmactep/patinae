//! Application Kernel
//!
//! UI-agnostic domain orchestrator that owns the scene state, command system,
//! and message bus. Can be used headless (tests, CLI) or wrapped by any GUI
//! frontend (Slint, egui, web).

use lin_alg::f32::Vec3;
use patinae_cmd::{
    AnnotationOutcome, AnnotationRequest, AsyncCommandRequest, CmdError, CommandAction,
    CommandExecutor, CommandOutput, FetchRequest, MessageKind, ViewerLike,
};
use patinae_mol::ObjectMolecule;
use patinae_scene::{
    AnimationUpdate, CameraDelta, CaptureRenderer, Session, SessionAdapter, ViewportImage,
};

use crate::message::{AppMessage, MessageBus};
use crate::model::command_line::CommandLineModel;
use crate::model::output::OutputModel;
use crate::model::scene::SceneModel;
use crate::model::ViewportModel;
use crate::tasks::{AsyncTask, TaskRunner};

/// Host hook that maps command async requests to executable tasks.
pub type AsyncCommandHandler = Box<dyn FnMut(AsyncCommandRequest) -> Option<Box<dyn AsyncTask>>>;

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
    async_command_handler: Option<AsyncCommandHandler>,
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
            tasks: TaskRunner::new(),
            async_command_handler: None,
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
        )
    }

    fn execute_command_with_failure_output<'a>(
        &'a mut self,
        cmd: &str,
        quiet: bool,
        render_context: Option<&'a mut (dyn CaptureRenderer + 'a)>,
        viewport_size: (u32, u32),
        failure_output: CommandFailureOutput,
    ) -> Result<CommandOutput, CmdError> {
        if !quiet {
            self.output.print_command(cmd);
        }
        self.command_generation = self.command_generation.wrapping_add(1);

        let mut adapter = SessionAdapter {
            session: &mut self.session,
            render_context,
            default_size: viewport_size,
            needs_redraw: &mut self.needs_redraw,
            async_fetch_fn: None,
        };
        let executor = &mut self.executor;
        let tasks = &self.tasks;
        let async_command_handler = &mut self.async_command_handler;
        let mut async_sink = |request: AsyncCommandRequest| {
            let Some(handler) = async_command_handler.as_mut() else {
                return false;
            };
            let Some(task) = handler(request) else {
                return false;
            };
            tasks.spawn_boxed(task);
            true
        };
        let result = executor.do_with_async_sink(&mut adapter, cmd, quiet, Some(&mut async_sink));

        match &result {
            Ok(output) => {
                for msg in &output.messages {
                    match msg.kind {
                        MessageKind::Info => self.output.print_info(&msg.text),
                        MessageKind::Warning => self.output.print_warning(&msg.text),
                        MessageKind::Error => self.output.print_error(&msg.text),
                    }
                }
                if !quiet {
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
            Err(e) => match failure_output {
                CommandFailureOutput::Error => self.output.print_error(e.to_string()),
                CommandFailureOutput::Warning => self.output.print_warning(e.to_string()),
            },
        }

        result
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
            async_fetch_fn: None,
        };
        f(&mut adapter)
    }

    /// Apply a fetched molecule using the same viewer behavior as the sync command.
    pub fn apply_fetched_molecule(&mut self, request: &FetchRequest, mol: ObjectMolecule) {
        let mut adapter = SessionAdapter {
            session: &mut self.session,
            render_context: None,
            default_size: (1, 1),
            needs_redraw: &mut self.needs_redraw,
            async_fetch_fn: None,
        };
        patinae_cmd::commands::io::finalize_fetched_molecule(
            &mut adapter,
            &request.name,
            mol,
            request.auto_dss,
            request.dss_algorithm,
        );
        self.output
            .print_info(format!(" Fetched {} as \"{}\"", request.code, request.name));
    }

    /// Print a fetch failure from an async task.
    pub fn print_fetch_error(&mut self, request: &FetchRequest, error: impl Into<String>) {
        self.output.print_error(format!(
            "Fetch failed for {}: {}",
            request.code,
            error.into()
        ));
    }

    /// Poll and apply all completed async task results.
    pub fn process_async_tasks(&mut self) -> bool {
        let mut results = Vec::new();
        while let Some(result) = self.tasks.poll() {
            results.push(result);
        }

        if results.is_empty() {
            return false;
        }

        for result in results {
            result.apply(self);
        }
        self.needs_redraw = true;
        true
    }

    /// User-visible messages for currently running async tasks.
    pub fn task_notification_messages(&self) -> Vec<String> {
        self.tasks.pending_messages()
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
