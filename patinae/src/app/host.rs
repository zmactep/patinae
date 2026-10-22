//! Host for the native application.

use super::*;

impl App {
    /// Services commands and tasks independently of rendering or window visibility.
    pub(super) fn pump_host(&mut self, app: &AppWindow) -> Option<bool> {
        self.poll_renderer_startup(app);
        let viewport_size = Self::viewport_size(app);
        let startup_changed = match self
            .plugin_runtime
            .plugin_startup
            .tick(&mut self.plugin_runtime.plugins, &mut self.kernel.executor)
        {
            Ok(changed) => changed,
            Err(error) => {
                log::warn!("{error}");
                self.kernel.bus.print_warning(error);
                false
            }
        };
        let list_changed = self
            .plugin_runtime
            .plugin_list
            .tick(&mut self.plugin_runtime.plugins, &mut self.kernel);
        if startup_changed || list_changed {
            self.refresh_command_names_cache();
            self.repl.update_completions(
                &self.kernel,
                app,
                app.global::<crate::ReplState>().get_input_text().as_str(),
            );
        }
        self.sync_plugin_list(app);
        self.run_startup_actions(viewport_size);
        self.poll_plugins(viewport_size);
        self.drain_annotation_requests();
        let window_visibility = self.process_commands(app);
        let rc = self
            .rendering
            .renderer
            .as_mut()
            .map(|r| r as &mut dyn CaptureRenderer);
        self.kernel.process_async_tasks(rc, viewport_size);
        crate::native_menu::sync_fetch_preview(&self.kernel, app);
        self.sync_notifications(app);
        self.sync_empty_mode(app);
        self.sync_plugins(app);
        self.repl.sync(&self.kernel, app);
        if self.kernel.needs_redraw() {
            app.window().request_redraw();
            self.kernel.clear_redraw_flag();
        }
        window_visibility
    }

    pub(super) fn process_commands(&mut self, app: &AppWindow) -> Option<bool> {
        let viewport_size = Self::viewport_size(app);
        let mut window_visibility = None;
        let rc = self
            .rendering
            .renderer
            .as_mut()
            .map(|r| r as &mut dyn CaptureRenderer);
        let unhandled = self.kernel.process_messages(rc, viewport_size);
        if unhandled.iter().any(|m| matches!(m, AppMessage::Quit)) {
            self.events.send(events::GuiEvent::Close);
        }
        let mut layout_messages = Vec::new();
        for msg in &unhandled {
            let window_action = match msg {
                AppMessage::ShowWindow => Some(true),
                AppMessage::HideWindow => Some(false),
                _ => None,
            };
            if window_action.is_some() {
                window_visibility = window_action;
                continue;
            }
            if self.dispatch_recent_file_message(msg, app) || self.queue_save_file_request(msg) {
                continue;
            }
            self.plugin_runtime
                .plugins
                .broadcast(msg, &mut self.kernel.bus);
            if !self.dispatch_plugin_layout_message(msg) {
                layout_messages.push(msg.clone());
            }
        }
        if !layout_messages.is_empty() {
            crate::bridges::layout::dispatch_messages(&layout_messages, app);
        }
        window_visibility
    }

    pub(super) fn sync_empty_mode(&mut self, app: &AppWindow) {
        let empty = self.kernel.session.registry.is_empty();
        let layout = app.global::<LayoutState>();
        layout.set_empty_mode(empty);

        if should_apply_standard_panel_preset(self.last_empty_mode, empty) {
            apply_standard_panel_preset(&layout);
        }

        self.last_empty_mode = empty;
    }
}

pub(super) fn should_apply_standard_panel_preset(was_empty: bool, is_empty: bool) -> bool {
    was_empty && !is_empty
}

pub(super) fn apply_standard_panel_preset(layout: &LayoutState) {
    layout.set_objects_visible(true);
    layout.set_selections_visible(true);
    layout.set_recent_atoms_visible(true);
    layout.set_repl_visible(true);
    layout.set_sequence_visible(false);
    layout.set_movie_visible(false);
    layout.set_bottom_active_tab(0);
}

#[cfg(test)]
pub(super) fn dispatch_lifecycle_messages(
    messages: &[AppMessage],
    mut quit_event_loop: impl FnMut() -> Result<(), slint::EventLoopError>,
) -> bool {
    if !messages.iter().any(|msg| matches!(msg, AppMessage::Quit)) {
        return false;
    }

    request_event_loop_quit(&mut quit_event_loop);

    true
}

pub(super) fn request_event_loop_quit(
    mut quit_event_loop: impl FnMut() -> Result<(), slint::EventLoopError>,
) {
    if let Err(err) = quit_event_loop() {
        log::warn!("Failed to quit Slint event loop: {err}");
    }
}
