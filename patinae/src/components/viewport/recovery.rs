//! Recovery for ViewportRenderer.

use super::*;

impl ViewportRenderer {
    pub(super) fn refresh_recovery_for_scene_change(&mut self, session: &mut Session) {
        let generation = session.registry.generation();
        if generation == self.last_registry_generation {
            return;
        }
        self.last_registry_generation = generation;
        let _ = self.wgpu_errors.take_oom();

        if self.recovery.is_normal() {
            return;
        }

        let base_profile = self.recovery.base_profile();
        let previous_profile = self.recovery.effective_profile();
        self.recovery.reset(base_profile);
        if previous_profile != base_profile {
            self.rebuild_for_profile(base_profile);
        } else {
            self.drop_transient_resources();
        }
        session.registry.mark_all_dirty();
        self.last_registry_generation = session.registry.generation();
        log::info!(
            "renderer memory recovery reset after scene generation changed; baseline profile={}",
            base_profile
        );
    }

    pub(super) fn render_frame_scoped(
        &mut self,
        session: &mut Session,
        width: u32,
        height: u32,
    ) -> Result<wgpu::Texture, ViewportRenderFailure> {
        let device = self.state.ctx.device.clone();
        let oom_scope = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let texture = self.render_frame(session, width, height);
        let scoped_error = pollster::block_on(oom_scope.pop());
        if let Some(error) = scoped_error {
            if is_wgpu_oom(&error) {
                log::warn!("Renderer frame captured WGPU OOM: {error}");
                return Err(ViewportRenderFailure::OutOfMemory);
            }
            log::error!("Renderer frame captured unexpected WGPU error: {error}");
        }
        if self.wgpu_errors.take_device_lost() {
            return Err(ViewportRenderFailure::DeviceLost);
        }
        if self.wgpu_errors.take_oom() {
            return Err(ViewportRenderFailure::OutOfMemory);
        }
        texture.map_err(|_| ViewportRenderFailure::ResourceLimit)
    }

    pub(super) fn recover_after_oom(&mut self, session: &mut Session) -> ViewportRenderOutcome {
        match self.recovery.advance_after_oom() {
            RenderMemoryRecoveryAction::RetryAfterDefrag { profile } => {
                self.prepare_oom_retry(session);
                log::warn!(
                    "renderer GPU OOM under profile {}; compacted SceneStore and retrying once",
                    profile
                );
                ViewportRenderOutcome::recovery(None, true)
            }
            RenderMemoryRecoveryAction::SwitchProfile { effective_profile } => {
                self.rebuild_for_profile(effective_profile);
                session.registry.mark_all_dirty();
                self.last_registry_generation = session.registry.generation();
                ViewportRenderOutcome::recovery(
                    Some(format!(
                        "Renderer memory profile switched to {} after confirmed GPU memory pressure.",
                        effective_profile
                    )),
                    true,
                )
            }
            RenderMemoryRecoveryAction::Blocked { last_profile } => {
                self.drop_transient_resources();
                ViewportRenderOutcome::recovery(
                    Some(format!(
                        "Renderer stopped retrying after GPU memory pressure in {} profile; reduce visible representations or load a smaller scene.",
                        last_profile
                    )),
                    false,
                )
            }
        }
    }

    pub(super) fn block_after_device_loss(&mut self) -> ViewportRenderOutcome {
        let base_profile = self.recovery.base_profile();
        let last_profile = self.recovery.effective_profile();
        self.recovery = RenderMemoryRecoveryStage::Blocked {
            base_profile,
            last_profile,
        };
        self.drop_transient_resources();
        ViewportRenderOutcome::recovery(
            Some("GPU device was lost; rendering is paused for this session state.".to_string()),
            false,
        )
    }

    // Renderer-owned invalidation must be acknowledged immediately: it is not
    // a user scene change and must not reset the next recovery step.
    pub(super) fn prepare_oom_retry(&mut self, session: &mut Session) {
        self.drop_transient_resources();
        let compaction = self.state.force_scene_store_compaction();
        self.state.drop_unused_optional_targets();
        session.registry.mark_all_dirty();
        self.last_registry_generation = session.registry.generation();
        if compaction.ran {
            log::info!(
                "renderer OOM recovery compacted SceneStore: capacity_before={:.2} MiB orphaned_before={:.2} MiB moved_objects={}",
                patinae_render::bytes_to_mib(compaction.capacity_before_bytes),
                patinae_render::bytes_to_mib(compaction.orphaned_before_bytes),
                compaction.moved_objects
            );
        }
    }

    pub(super) fn apply_runtime_memory_policy(&mut self, session: &mut Session) -> Option<String> {
        let requested =
            render_memory_policy_from_settings(&session.settings, self.auto_memory_policy);
        if !self.recovery.request_profile(requested.profile) {
            return None;
        }
        self.rebuild_for_policy(requested);
        session.registry.mark_all_dirty();
        self.last_registry_generation = session.registry.generation();
        let mut message = format!(
            "Renderer memory profile switched to {} by user setting.",
            requested.profile
        );
        if let Some(notice) = crate::allocator_startup::restart_notice(requested.profile) {
            message.push(' ');
            message.push_str(&notice);
        }
        Some(message)
    }

    pub(super) fn rebuild_for_profile(&mut self, profile: RenderMemoryProfile) {
        self.rebuild_for_policy(RenderMemoryPolicy::from_profile(profile));
    }

    pub(super) fn rebuild_for_policy(&mut self, policy: RenderMemoryPolicy) {
        let device = self.state.ctx.device.clone();
        let queue = self.state.ctx.queue.clone();
        self.config.memory = policy;
        let viewport = self.last_viewport_size.unwrap_or((1, 1));
        self.state =
            RenderState::with_config(device, queue, VIEWPORT_FORMAT, viewport, self.config);
        self.drop_transient_resources();
    }

    pub(super) fn drop_transient_resources(&mut self) {
        self.pending_hover = None;
        self.last_hover_submit = None;
        self.viewport_gpu_image = None;
        self.color_textures.clear();
        self.color_textures_next = 0;
    }

    pub(crate) fn clear_viewport_gpu_image(&mut self) -> bool {
        self.viewport_gpu_image.take().is_some()
    }

    pub(crate) fn invalidate_live_textures(&mut self) {
        self.last_viewport_size = None;
        self.color_textures.clear();
        self.color_textures_next = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a native GPU to exercise viewport rebuilds"]
    fn gpu_oom_recovery_survives_renderer_owned_scene_invalidation() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: patinae_render::required_device_limits(&adapter.limits()).unwrap(),
            ..Default::default()
        }))
        .unwrap();
        let config = RenderConfig {
            memory: RenderMemoryPolicy::performance(),
            ..Default::default()
        };
        let state = RenderState::with_config(
            Arc::new(device),
            Arc::new(queue),
            VIEWPORT_FORMAT,
            (1, 1),
            config,
        );
        let mut viewport =
            ViewportRenderer::from_prepared(state, config, WgpuErrorFlags::default());
        let mut session = Session::new();
        viewport.refresh_recovery_for_scene_change(&mut session);

        for (expected, redraw) in [
            (RenderMemoryProfile::Performance, true),
            (RenderMemoryProfile::Balanced, true),
            (RenderMemoryProfile::Lite, true),
            (RenderMemoryProfile::Lite, false),
        ] {
            let outcome = viewport.recover_after_oom(&mut session);
            assert_eq!(outcome.request_redraw, redraw);
            let stage = viewport.recovery;
            assert_eq!(stage.effective_profile(), expected);
            viewport.refresh_recovery_for_scene_change(&mut session);
            assert!(viewport.apply_runtime_memory_policy(&mut session).is_none());
            assert_eq!(viewport.recovery, stage);
        }
        assert!(viewport.recovery.is_blocked());
        let outcome = viewport.render(&mut session, 1, 1);
        assert!(!outcome.request_redraw);
        assert!(!outcome.present_frame);

        session.registry.invalidate();
        viewport.refresh_recovery_for_scene_change(&mut session);
        assert!(viewport.recovery.is_normal());
        assert_eq!(
            viewport.config.memory.profile,
            RenderMemoryProfile::Performance
        );
    }
}
