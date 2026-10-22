//! Startup for the native application.

use super::*;

impl App {
    pub(super) fn run_startup_actions(&mut self, viewport_size: (u32, u32)) {
        if !self.plugin_runtime.plugin_startup.ready() || self.rendering.renderer_startup.is_some()
        {
            return;
        }
        while let Some(action) = self.startup_actions.pop_front() {
            match action {
                StartupAction::Warning(message) => {
                    self.kernel.bus.print_warning(message);
                }
                StartupAction::RunPatinaerc(path) => {
                    let path_text = path.to_string_lossy();
                    let command = format!("run {}", quote_command_arg(path_text.as_ref()));
                    let rc: Option<&mut dyn CaptureRenderer> = self
                        .rendering
                        .renderer
                        .as_mut()
                        .map(|r| r as &mut dyn CaptureRenderer);
                    let _ = self.kernel.execute_command_warning_on_error(
                        &command,
                        false,
                        rc,
                        viewport_size,
                    );
                }
                StartupAction::RouteArgvFile(path) => {
                    enqueue_file_action(&mut self.kernel, &path, NativeFileSource::StartupArgv);
                }
            }
        }
        while let Some((path, source)) = self.file_actions.deferred_file_actions.pop_front() {
            enqueue_file_action(&mut self.kernel, &path, source);
        }
    }
}

pub(super) fn startup_actions(
    patinaerc: PatinaercPath,
    argv_files: Vec<PathBuf>,
) -> VecDeque<StartupAction> {
    let mut actions = VecDeque::new();

    match patinaerc.path.try_exists() {
        Ok(true) | Err(_) => actions.push_back(StartupAction::RunPatinaerc(patinaerc.path)),
        Ok(false) if patinaerc.source == PatinaercSource::Env => {
            actions.push_back(StartupAction::Warning(format!(
                "PATINAERC startup rc script not found: {}",
                patinaerc.path.display()
            )));
        }
        Ok(false) => {}
    }

    for path in argv_files {
        actions.push_back(StartupAction::RouteArgvFile(path));
    }

    actions
}
