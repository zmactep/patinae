//! Files for the native application.

use super::*;

impl App {
    pub(super) fn submit_repl_command(&mut self, cmd: String, app: &AppWindow) {
        self.execute_repl_input(cmd, Self::viewport_size(app));
    }

    pub(super) fn execute_repl_input(&mut self, cmd: String, viewport_size: (u32, u32)) {
        self.kernel.command_line.input = cmd;
        let cmd = self.kernel.command_line.take_command();
        if cmd.is_empty() {
            return;
        }
        self.kernel.command_line.add_to_history(cmd.clone());
        let rc: Option<&mut dyn CaptureRenderer> = self
            .rendering
            .renderer
            .as_mut()
            .map(|r| r as &mut dyn CaptureRenderer);
        let result = self.kernel.execute_command(&cmd, false, rc, viewport_size);
        if !self.plugin_runtime.plugin_startup.ready()
            && result.is_err_and(|error| error.is_unknown_command())
        {
            self.kernel.bus.print_info(
                "Plugins are still loading; unavailable commands are not retried automatically",
            );
        }
    }

    /// Route a native menu Open selection through the shared file router.
    pub(super) fn route_menu_open_file(&mut self, path: PathBuf) {
        self.route_native_file(path.as_path(), NativeFileSource::MenuOpen);
    }

    /// Route a native menu Run Script selection through the shared file router.
    pub(super) fn route_menu_run_script(&mut self, path: PathBuf) {
        self.route_native_file(path.as_path(), NativeFileSource::MenuRunScript);
    }

    pub(super) fn route_dropped_file(&mut self, path: &std::path::Path) {
        self.route_native_file(path, NativeFileSource::DragDrop);
    }

    pub(super) fn route_native_file(&mut self, path: &std::path::Path, source: NativeFileSource) {
        if (!self.plugin_runtime.plugin_startup.ready()
            || self.rendering.renderer_startup.is_some())
            && crate::native_file_actions::needs_plugin_startup(&self.kernel.executor, path)
        {
            self.file_actions
                .deferred_file_actions
                .push_back((path.to_path_buf(), source));
            self.kernel
                .bus
                .print_info("File queued until startup completes");
        } else {
            enqueue_file_action(&mut self.kernel, path, source);
        }
    }

    pub(super) fn open_recent_file(&mut self, app: &AppWindow, path: &str) {
        let path = PathBuf::from(path);
        match path.try_exists() {
            Ok(true) => {
                self.route_native_file(&path, NativeFileSource::RecentFile);
            }
            Ok(false) => {
                if let Err(err) = self.file_actions.recent_files.remove(&path, app) {
                    self.kernel.bus.print_warning(format!(
                        "Could not update recent files after missing file: {err}"
                    ));
                }
                self.kernel.bus.print_warning(format!(
                    "Recent file is no longer available: {}",
                    path.display()
                ));
            }
            Err(err) => {
                self.kernel.bus.print_warning(format!(
                    "Could not check recent file {}: {err}",
                    path.display()
                ));
            }
        }
        self.kernel.bus.request_redraw();
        app.window().request_redraw();
    }

    #[cfg(test)]
    /// Queues one native-only typed annotation request.
    pub(super) fn queue_annotation_request(&mut self, request: AnnotationRequest) {
        self.file_actions
            .pending_annotation_requests
            .push_back(request);
    }

    pub(super) fn drain_annotation_requests(&mut self) -> usize {
        let mut processed = 0;
        while let Some(request) = self.file_actions.pending_annotation_requests.pop_front() {
            let _ = self.kernel.execute_annotation_request(&request);
            processed += 1;
        }
        processed
    }

    pub(super) fn queue_save_file_request(&mut self, msg: &AppMessage) -> bool {
        let Some(request) = topics::subscribe::<SaveFileRequest>(msg, SAVE_FILE_REQUEST_TOPIC)
        else {
            return false;
        };

        self.file_actions
            .pending_save_file_requests
            .push_back(request);
        true
    }

    pub(super) fn dispatch_recent_file_message(
        &mut self,
        msg: &AppMessage,
        app: &AppWindow,
    ) -> bool {
        let AppMessage::RecordRecentFile { path, command } = msg else {
            return false;
        };

        if command != "load" {
            return true;
        }

        if let Err(err) = self
            .file_actions
            .recent_files
            .record(PathBuf::from(path), app)
        {
            self.kernel
                .bus
                .print_warning(format!("Could not save recent files: {err}"));
        } else {
            self.file_actions.pending_recent_thumbnail = Some(PathBuf::from(path));
        }
        true
    }

    pub(super) fn capture_pending_recent_thumbnail(&mut self, app: &AppWindow) {
        let Some(path) = self.file_actions.pending_recent_thumbnail.take() else {
            return;
        };
        if self.kernel.session.registry.is_empty() {
            return;
        }

        let paths = match self.file_actions.recent_files.thumbnail_write_paths(&path) {
            Ok(paths) => paths,
            Err(err) => {
                self.kernel
                    .bus
                    .print_warning(format!("Could not prepare recent thumbnail cache: {err}"));
                return;
            }
        };
        let Some(renderer) = self.rendering.renderer.as_mut() else {
            return;
        };

        let session = &mut self.kernel.session;
        let capture = renderer.capture_png(
            &paths.temp_path,
            RECENT_THUMBNAIL_WIDTH,
            RECENT_THUMBNAIL_HEIGHT,
            &mut session.camera,
            &mut session.registry,
            &session.settings,
            &session.named_palette,
            &session.palette,
            session.clear_color,
        );

        if let Err(err) = capture.and_then(|_| {
            paths
                .commit()
                .map_err(|err| patinae_scene::ViewerError::capture_error(err.to_string()))
        }) {
            paths.cleanup_temp();
            self.kernel
                .bus
                .print_warning(format!("Could not save recent thumbnail: {err}"));
            return;
        }

        self.file_actions.recent_files.refresh(app);
    }
}

pub(super) fn save_file_response_event(request: &SaveFileRequest, path: String) -> PanelEvent {
    PanelEvent {
        panel_id: request.panel_id.clone(),
        control_id: request.reply_control_id.clone(),
        kind: PanelEventKind::TextCommit,
        value: PanelValue::Text(path),
    }
}

/// Owns file actions state.
pub(super) struct FileActions {
    pub(super) recent_files: RecentFilesBridge,
    pub(super) pending_recent_thumbnail: Option<PathBuf>,
    pub(super) deferred_file_actions: VecDeque<(PathBuf, NativeFileSource)>,
    pub(super) pending_save_file_requests: VecDeque<SaveFileRequest>,
    pub(super) pending_annotation_requests: VecDeque<AnnotationRequest>,
    pub(super) save_file_dialog_scheduled: bool,
}
