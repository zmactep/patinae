//! Frame for ViewportRenderer.

use super::*;

impl ViewportRenderer {
    pub(crate) fn render(
        &mut self,
        session: &mut Session,
        width: u32,
        height: u32,
    ) -> ViewportRenderOutcome {
        self.refresh_recovery_for_scene_change(&mut session.registry);
        if self.wgpu_errors.take_device_lost() {
            return self.block_after_device_loss();
        }
        if self.wgpu_errors.take_oom() {
            return self.recover_after_oom(session);
        }

        if let Some(warning) =
            self.apply_runtime_memory_policy(&session.settings, &mut session.registry)
        {
            return ViewportRenderOutcome::recovery(Some(warning), true);
        }
        if self.recovery.is_blocked() {
            return ViewportRenderOutcome::recovery(None, false);
        }

        if let Some(image) = self.viewport_gpu_image.as_ref() {
            return match slint::Image::try_from(image.texture.clone()) {
                Ok(image) => ViewportRenderOutcome::frame(Some(image)),
                Err(e) => {
                    log::error!("Failed to import GPU viewport image: {:?}", e);
                    ViewportRenderOutcome::frame(None)
                }
            };
        }

        match self.render_frame_scoped(session, width, height) {
            Ok(texture) => match slint::Image::try_from(texture) {
                Ok(image) => {
                    self.recovery.record_success();
                    ViewportRenderOutcome::frame(Some(image))
                }
                Err(e) => {
                    log::error!("Failed to import texture: {:?}", e);
                    ViewportRenderOutcome::frame(None)
                }
            },
            Err(ViewportRenderFailure::OutOfMemory) => self.recover_after_oom(session),
            Err(ViewportRenderFailure::DeviceLost) => self.block_after_device_loss(),
            // The renderer queues a deduplicated warning. Unlike OOM, a hard
            // resource limit cannot be fixed by lowering the quality profile.
            Err(ViewportRenderFailure::ResourceLimit) => {
                ViewportRenderOutcome::recovery(None, false)
            }
        }
    }

    pub(super) fn render_frame(
        &mut self,
        session: &mut Session,
        width: u32,
        height: u32,
    ) -> Result<wgpu::Texture, patinae_render::BufferSizeError> {
        // Resize state's internal targets if the viewport changed.
        let resized = self.last_viewport_size != Some((width, height));
        if resized {
            self.state.resize((width, height));
            self.last_viewport_size = Some((width, height));
            // Drop the cached ring so it's re-created at the new size.
            self.color_textures.clear();
            self.color_textures_next = 0;
        }

        // 1) Frame uniforms.
        let frame = frame_uniforms_from_session(session, (width, height), session.clear_color);
        self.state.uniforms = frame;
        self.state.ctx.upload_frame(&self.state.uniforms);
        // Live viewports should show the RGB background; PNG capture keeps
        // using `opaque_background` to decide exported alpha.
        self.state.set_clear_color(session.clear_color, true);
        self.state
            .set_marking_width(session.settings.ui.selection_width);

        // 2) Build cached host-side render input, then sync the renderer.
        // The cache owns expensive per-atom color/marker buffers and the
        // picking name lookup. The frame input borrows from `session` only
        // until `RenderState::sync` returns.
        let frame = self.render_scene.prepare(session);
        self.state.sync(&frame.render_input())?;
        drop(frame);
        // patinae-render reps track dirty internally; consume the host-side
        // flag set by commands so SceneModel::sync stops perpetually
        // rebuilding the objects panel (which would destroy panel
        // TouchAreas every frame and break hover/click detection).
        session.registry.clear_all_dirty_objects();

        configure_frame_effects(&mut self.state, &session.settings, &session.named_palette);

        // Build (or reuse) the offscreen colour texture handed back to
        // Slint each frame and run the FrameGraph. We keep a ring of
        // textures and rotate. Slint may still be sampling
        // the previous frame's texture when we kick off this frame —
        // writing into a fresh one lets the GPU pipeline both passes
        // instead of serialising on the read fence.
        let color_texture_ring = self
            .config
            .memory
            .frame_targets
            .viewport_handoff_ring
            .max(1);
        if self.color_textures.is_empty() {
            self.color_textures = (0..color_texture_ring)
                .map(|i| {
                    self.state
                        .ctx
                        .device
                        .create_texture(&wgpu::TextureDescriptor {
                            label: Some(match i {
                                0 => "Patinae Viewport 0",
                                1 => "Patinae Viewport 1",
                                2 => "Patinae Viewport 2",
                                _ => "Patinae Viewport Extra",
                            }),
                            size: wgpu::Extent3d {
                                width,
                                height,
                                depth_or_array_layers: 1,
                            },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: VIEWPORT_FORMAT,
                            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                                | wgpu::TextureUsages::TEXTURE_BINDING,
                            view_formats: &[],
                        })
                })
                .collect();
            self.color_textures_next = 0;
        }
        let idx = self.color_textures_next;
        self.color_textures_next = (idx + 1) % self.color_textures.len();
        let color_texture = &self.color_textures[idx];
        let color_view = color_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder =
            self.state
                .ctx
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Patinae Render Encoder"),
                });
        self.state.render(&color_view, &mut encoder);

        // PATINAE_GPU_SYNC=1: force device.poll(Wait) after submit and
        // measure real GPU wall-clock per frame. This remains a useful
        // end-to-end check even when timestamp queries are available for
        // finer per-pass diagnostics.
        let gpu_sync = std::env::var("PATINAE_GPU_SYNC")
            .ok()
            .map(|v| v == "1")
            .unwrap_or(false);
        let t_submit = std::time::Instant::now();
        self.state
            .ctx
            .queue
            .submit(std::iter::once(encoder.finish()));
        if gpu_sync {
            self.state
                .ctx
                .device
                .poll(wgpu::PollType::Wait {
                    submission_index: None,
                    timeout: None,
                })
                .ok();
            let dt_ms = t_submit.elapsed().as_secs_f64() * 1000.0;
            eprintln!("[patinae] gpu submit+wait: {:.2} ms", dt_ms);
        }

        Ok(self.color_textures[idx].clone())
    }
}
