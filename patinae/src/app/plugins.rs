//! Plugins for the native application.

use super::*;

impl App {
    pub(super) fn poll_plugins(&mut self, viewport_size: (u32, u32)) {
        // Drain ready acknowledgements without waiting for another host tick.
        // No wait or speculative polling: stop as soon as transport is empty.
        for pass in 0..self.kernel.tasks.config().batch_size {
            self.poll_plugins_once(viewport_size, pass == 0);
            self.plugin_runtime
                .plugins
                .prepare_task_dispatch(&mut self.kernel);
            if !self.plugin_runtime.plugins.has_ready_task_delivery() {
                break;
            }
        }
    }

    pub(super) fn poll_plugins_once(&mut self, viewport_size: (u32, u32), first_pass: bool) {
        if self.plugin_runtime.plugins.plugin_count() == 0 {
            return;
        }

        self.plugin_runtime
            .plugins
            .prepare_task_dispatch(&mut self.kernel);
        let scene_generation = self.plugin_scene_generation();
        {
            let gpu_device = self
                .rendering
                .renderer
                .as_ref()
                .map(|r| r.gpu_device().as_ref());
            let gpu_queue = self
                .rendering
                .renderer
                .as_ref()
                .map(|r| r.gpu_queue().as_ref());
            let kernel = &mut self.kernel;
            let shared = SharedContext {
                registry: &kernel.session.registry,
                camera: &kernel.session.camera,
                selections: &kernel.session.selections,
                recent_atoms: &kernel.session.recent_atoms,
                named_palette: &kernel.session.named_palette,
                movie: &kernel.session.movie,
                settings: &kernel.session.settings,
                clear_color: kernel.session.clear_color,
                gpu_device,
                gpu_queue,
                scene_generation,
                viewport_image: kernel.session.viewport_image.as_ref(),
                command_names: &self.plugin_runtime.command_names_cache,
                command_registry: kernel.executor.registry(),
                setting_names: &self.plugin_runtime.setting_names_cache,
                dynamic_settings: Some(kernel.executor.dynamic_settings()),
                tasks: Some(&kernel.tasks),
            };
            if first_pass {
                self.plugin_runtime
                    .plugins
                    .poll_all(&shared, &mut kernel.bus);
            } else {
                self.plugin_runtime
                    .plugins
                    .poll_task_deliveries(&shared, &mut kernel.bus);
            }
        }

        let rc = self
            .rendering
            .renderer
            .as_mut()
            .map(|r| r as &mut dyn CaptureRenderer);
        self.plugin_runtime
            .plugins
            .apply_task_controls(&mut self.kernel, rc, viewport_size);

        let mut results = Vec::new();
        for request in self.plugin_runtime.plugins.take_pending_executions() {
            let rc: Option<&mut dyn CaptureRenderer> = self
                .rendering
                .renderer
                .as_mut()
                .map(|r| r as &mut dyn CaptureRenderer);
            let result = self.kernel.execute_command_captured(
                &request.command,
                request.silent,
                rc,
                viewport_size,
            );
            results.push(patinae_plugin_host::CommandResult::from_execution(
                request.id, result,
            ));
        }
        self.plugin_runtime.plugins.store_command_results(results);

        for mutation in self.plugin_runtime.plugins.take_pending_mutations() {
            let rc: Option<&mut dyn CaptureRenderer> = self
                .rendering
                .renderer
                .as_mut()
                .map(|r| r as &mut dyn CaptureRenderer);
            self.kernel.mutate_viewer(rc, viewport_size, mutation);
        }

        if self
            .plugin_runtime
            .plugins
            .apply_dynamic_command_changes(&mut self.kernel.executor)
        {
            self.refresh_command_names_cache();
            self.plugin_runtime.plugins.invalidate_panel_ui();
        }
        self.plugin_runtime.plugins.apply_hotkey_changes();
    }

    pub(super) fn dispatch_plugin_layout_message(
        &mut self,
        msg: &patinae_framework::message::AppMessage,
    ) -> bool {
        use patinae_framework::message::AppMessage;

        match msg {
            AppMessage::ShowPanel(id) if self.plugin_runtime.plugins.has_panel(id) => {
                self.plugin_runtime.plugins.show_panel(id);
                true
            }
            AppMessage::HidePanel(id) if self.plugin_runtime.plugins.has_panel(id) => {
                self.plugin_runtime.plugins.hide_panel(id);
                true
            }
            AppMessage::TogglePanel(id) if self.plugin_runtime.plugins.has_panel(id) => {
                self.plugin_runtime.plugins.toggle_panel(id);
                true
            }
            AppMessage::ActivateTab(id) if self.plugin_runtime.plugins.has_panel(id) => {
                self.plugin_runtime.plugins.activate_panel(id);
                true
            }
            _ => false,
        }
    }

    pub(super) fn sync_plugin_list(&mut self, app: &AppWindow) {
        if !app.global::<crate::PluginListState>().get_visible() {
            return;
        }
        self.plugin_runtime.plugin_list.sync(
            app,
            &self.plugin_runtime.plugins,
            self.plugin_runtime.plugin_startup.status(),
        );
    }

    pub(super) fn sync_plugins(&mut self, app: &AppWindow) {
        if self.plugin_runtime.plugins.plugin_count() == 0 {
            if self.plugin_runtime.last_plugin_sync_key != Some(0) {
                self.plugin_runtime.plugin_bridge.sync(Vec::new(), app);
                self.plugin_runtime.last_plugin_sync_key = Some(0);
            }
            return;
        }

        let sync_key = self.plugin_sync_key();
        if self.plugin_runtime.last_plugin_sync_key == Some(sync_key) {
            return;
        }

        let gpu_device = self
            .rendering
            .renderer
            .as_ref()
            .map(|r| r.gpu_device().as_ref());
        let gpu_queue = self
            .rendering
            .renderer
            .as_ref()
            .map(|r| r.gpu_queue().as_ref());
        let scene_generation = self.plugin_scene_generation();
        let shared = SharedContext {
            tasks: Some(&self.kernel.tasks),
            registry: &self.kernel.session.registry,
            camera: &self.kernel.session.camera,
            selections: &self.kernel.session.selections,
            recent_atoms: &self.kernel.session.recent_atoms,
            named_palette: &self.kernel.session.named_palette,
            movie: &self.kernel.session.movie,
            settings: &self.kernel.session.settings,
            clear_color: self.kernel.session.clear_color,
            gpu_device,
            gpu_queue,
            scene_generation,
            viewport_image: self.kernel.session.viewport_image.as_ref(),
            command_names: &self.plugin_runtime.command_names_cache,
            command_registry: self.kernel.executor.registry(),
            setting_names: &self.plugin_runtime.setting_names_cache,
            dynamic_settings: Some(self.kernel.executor.dynamic_settings()),
        };
        let frames = self.plugin_runtime.plugins.panel_frames(&shared, sync_key);
        self.plugin_runtime.plugin_bridge.sync(frames, app);
        self.plugin_runtime.last_plugin_sync_key = Some(sync_key);
    }

    pub(super) fn refresh_command_names_cache(&mut self) {
        self.plugin_runtime.command_names_cache = self
            .kernel
            .executor
            .registry()
            .names()
            .map(|s| s.to_string())
            .collect();
    }

    pub(super) fn plugin_sync_key(&self) -> u64 {
        let mut key = self.plugin_runtime.plugins.panel_ui_generation();
        key = mix_generation(key, self.plugin_scene_generation());
        key = mix_generation(key, self.kernel.command_generation());
        key = mix_generation(
            key,
            viewport_image_signature(self.kernel.session.viewport_image.as_ref()),
        );
        key
    }

    pub(super) fn plugin_scene_generation(&self) -> u64 {
        self.kernel
            .session
            .registry
            .generation()
            .wrapping_add(self.kernel.session.selections.generation().rotate_left(1))
            .wrapping_add(self.kernel.session.movie.generation().rotate_left(2))
            .wrapping_add(self.kernel.session.recent_atoms.generation().rotate_left(3))
    }
}

pub(super) fn mix_generation(acc: u64, value: u64) -> u64 {
    acc.rotate_left(13)
        .wrapping_mul(0x9E37_79B1_85EB_CA87)
        .wrapping_add(value)
}

/// Owns plugin runtime state.
pub(super) struct Plugins {
    pub(super) plugins: PluginHost,
    pub(super) plugin_bridge: PluginBridge,
    pub(super) plugin_startup: PluginStartup,
    pub(super) plugin_list: crate::bridges::plugin_list::PluginList,
    pub(super) command_names_cache: Vec<String>,
    pub(super) setting_names_cache: Vec<&'static str>,
    pub(super) last_plugin_sync_key: Option<u64>,
}
