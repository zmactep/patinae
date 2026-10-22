//! Frame for ViewportRenderer.

use super::*;

impl ViewportRenderer {
    pub(crate) fn render(
        &mut self,
        session: &mut Session,
        width: u32,
        height: u32,
    ) -> ViewportRenderOutcome {
        self.refresh_recovery_for_scene_change(session);
        if self.wgpu_errors.take_device_lost() {
            return self.block_after_device_loss();
        }
        if self.wgpu_errors.take_oom() {
            return self.recover_after_oom(session);
        }

        if let Some(warning) = self.apply_runtime_memory_policy(session) {
            return ViewportRenderOutcome::recovery(Some(warning), true);
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
        }
    }

    pub(super) fn render_frame(
        &mut self,
        session: &mut Session,
        width: u32,
        height: u32,
    ) -> wgpu::Texture {
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
        self.state.sync(&frame.render_input());
        drop(frame);
        // patinae-render reps track dirty internally; consume the host-side
        // flag set by commands so SceneModel::sync stops perpetually
        // rebuilding the objects panel (which would destroy panel
        // TouchAreas every frame and break hover/click detection).
        session.registry.clear_all_dirty_objects();

        // 5) Marker bits already flow through `render_scene` →
        // `RenderObjectInput.atom_markers` → `marker_lut`. The legacy
        // `apply_highlight` / `selection_mut` path is gone; the `max_atoms`
        // value is no longer consumed here.

        // 6) Silhouette pass on/off from settings.
        let common = &session.settings.shading.common;
        let silhouette_ink = resolve_literal_color(
            common.silhouette_color,
            &session.named_palette,
            [0.0, 0.0, 0.0, 1.0],
        );
        if common.silhouettes {
            self.state
                .set_silhouette(true, common.silhouette_width, silhouette_ink);
        } else {
            self.state.set_silhouette(false, 1.0, silhouette_ink);
        }

        let full = &session.settings.shading.full;
        self.state.set_shadows(
            session.settings.shading.mode == ShadingMode::Full,
            full.shadow_map_size.max(64) as u32,
            full.shadow_bias,
            full.shadow_intensity,
            full.shadow_pcf.max(1) as u32,
        );
        // 6b) Ambient-occlusion config. Skripkin uses a multi-directional
        // shadow atlas; Classic / Full use the explicit screen-space AO
        // settings.
        configure_ambient_occlusion(&mut self.state, &session.settings);
        // 6c) FXAA postprocess. Default-enabled; toggle via
        // `set fxaa_enabled, 0`.
        self.state.set_fxaa(effective_fxaa(&session.settings));

        // 7) Build (or reuse) the offscreen colour texture handed back to
        // Slint each frame and run the FrameGraph. We keep a ring of
        // textures and rotate. Slint may still be sampling
        // the previous frame's texture when we kick off this frame —
        // writing into a fresh one lets the GPU pipeline both passes
        // instead of serialising on the read fence.
        let color_texture_ring = self
            .memory_policy
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

        self.color_textures[idx].clone()
    }
}
pub(super) fn effective_fxaa(settings: &Settings) -> bool {
    settings.ui.antialias > 0 && settings.fxaa.enabled
}

pub(super) fn configure_ambient_occlusion(state: &mut RenderState, settings: &Settings) {
    let ssao = &settings.ssao;
    if settings.shading.mode == ShadingMode::Skripkin {
        let skripkin = &settings.shading.skripkin;
        state.set_ssao(false, ssao.radius, ssao.intensity, ssao.bias);
        state.set_skripkin_ao(
            true,
            skripkin.directions.max(1) as u32,
            skripkin.map_size.max(32) as u32,
            skripkin.bias,
            skripkin.intensity,
        );
    } else {
        state.set_skripkin_ao(false, 1, 32, 0.0, 0.0);
        state.set_ssao(ssao.enabled, ssao.radius, ssao.intensity, ssao.bias);
    }
}
