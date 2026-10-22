//! Frame for the native application.

use super::*;

impl App {
    pub(super) fn render_frame(&mut self, app: &AppWindow) {
        // Window chrome must be ready for the first frame, even while GPU
        // resources are still being prepared in the background.
        let theme_changed = crate::bridges::theme::sync_theme(
            &mut self.kernel,
            &app.global::<Theme>(),
            app,
            &mut self.prev_theme,
        );
        if theme_changed {
            self.kernel.session.registry.mark_all_dirty();
        }
        self.platform.sync(app);

        if self.rendering.renderer.is_none() {
            return;
        }

        // PATINAE_TIMING=1 → per-section timing dump on every frame.
        // Helps identify where the host loop spends time.
        let timing = std::env::var("PATINAE_TIMING").is_ok();
        let t_total = if timing {
            Some(std::time::Instant::now())
        } else {
            None
        };
        // End-to-end frame time = gap between consecutive `render_frame`
        // invocations. Captures Slint idle + present wait + GPU stall —
        // the wall-clock signal a user actually perceives. Fed
        // unconditionally into `perf_history` so the HUD has data the
        // moment `PATINAE_PERF=1` is toggled.
        let now = Instant::now();
        if let Some(prev) = self.diagnostics.last_frame_instant.get() {
            let gap_ms = now.duration_since(prev).as_secs_f32() * 1000.0;
            if gap_ms > 0.0 {
                self.diagnostics.perf_history.borrow_mut().push(gap_ms);
            }
            if timing && gap_ms > 1.0 {
                eprintln!("[patinae] inter-frame gap: {gap_ms:.2} ms");
            }
        }
        self.diagnostics.last_frame_instant.set(Some(now));
        let mut t_section = std::time::Instant::now();
        let mark = |label: &str, t: &mut std::time::Instant| {
            if !timing {
                return;
            }
            let now = std::time::Instant::now();
            let dt = now.duration_since(*t).as_secs_f32() * 1000.0;
            if dt > 0.05 {
                eprintln!("  [patinae] {label}: {dt:.2} ms");
            }
            *t = now;
        };

        let vp = app.global::<ViewportState>();
        let sf = app.window().scale_factor();
        let (vw, vh) = Self::viewport_size(app);
        let fetch_dialog_visible = app.global::<MenuState>().get_fetch_visible()
            || app.global::<crate::PluginListState>().get_visible();
        mark("setup", &mut t_section);

        // Resize + Input
        self.viewport.resize(vw, vh);
        self.kernel.resize(vw, vh);
        if self.take_live_texture_invalidation() {
            if let Some(renderer) = self.rendering.renderer.as_mut() {
                renderer.invalidate_live_textures();
            }
        }
        if !fetch_dialog_visible && !self.interaction.manipulation.is_reserved() {
            let hover_disabled = std::env::var("PATINAE_NO_HOVER").is_ok_and(|value| value == "1");
            if !hover_disabled {
                self.process_hover(vw, vh, sf, (app.get_scene_x(), app.get_scene_y()));
            }
        }
        mark("input+pick", &mut t_section);

        let animation_dt = self
            .last_animation_instant
            .get()
            .map(|prev| now.duration_since(prev).as_secs_f32().min(0.1))
            .unwrap_or(0.0);
        self.last_animation_instant.set(Some(now));
        self.kernel.update_animations(animation_dt);
        mark("animations", &mut t_section);

        self.sync_empty_mode(app);

        self.sync_plugins(app);
        mark("plugins.ui", &mut t_section);

        // Sync objects panel (sees dirty flags before render clears them)
        self.objects.sync(&mut self.kernel, app);
        mark("objects panel", &mut t_section);

        // Sync sequence panel
        self.sequence.sync(&self.kernel, app);
        mark("sequence panel", &mut t_section);

        // Sync movie panel
        let movie_object = self.objects.single_movie_keyframe_object();
        self.movie.sync(&self.kernel, app, movie_object);
        mark("movie panel", &mut t_section);

        // Sync REPL output model
        self.repl.sync(&self.kernel, app);
        mark("repl panel", &mut t_section);

        // Render (rebuilds GPU reps from dirty flags, then clears them)
        let renderer = self.rendering.renderer.as_mut().unwrap();
        let render_outcome = renderer.render(&mut self.kernel.session, vw, vh);
        mark("renderer.render", &mut t_section);
        if let Some(warning) = render_outcome.warning {
            self.kernel.bus.print_warning(warning);
        }
        for warning in renderer.take_memory_warnings() {
            self.kernel.bus.print_warning(warning);
        }
        let annotation_labels = if self.kernel.session.viewport_image.is_some() {
            renderer.clear_annotation_labels();
            Vec::new()
        } else {
            let viewport = (0.0, 0.0, vw as f32, vh as f32);
            renderer
                .annotation_labels(&self.kernel.session.camera, viewport, sf)
                .iter()
                .map(|label| viewport_label_from_projected(label, sf))
                .collect()
        };
        self.viewport.set_labels(annotation_labels);
        self.interaction.manipulation.set_targets(
            self.viewport
                .manipulation_targets(&self.kernel.session.registry),
        );
        let feedback = self.interaction.manipulation.feedback();
        vp.set_manipulation_visible(feedback.highlights_visible);
        if let Some(ViewportTargetId::Label { owner, entity }) = feedback.active_target {
            vp.set_manipulation_owner(owner.get().to_string().into());
            vp.set_manipulation_entity(entity as i32);
        } else {
            vp.set_manipulation_owner("".into());
            vp.set_manipulation_entity(-1);
        }
        vp.set_manipulation_cursor(match feedback.cursor {
            ViewportCursor::Grabbing => 2,
            ViewportCursor::Grab => 1,
            ViewportCursor::Default => 0,
        });

        if render_outcome.request_redraw {
            self.kernel.bus.request_redraw();
            app.window().request_redraw();
        }
        if render_outcome.present_frame {
            let image = viewport_image_to_slint(self.kernel.session.viewport_image.as_ref())
                .or(render_outcome.image);
            self.viewport.push_frame(image, &vp);
            mark("push_frame", &mut t_section);
        }

        // Drain GPU stats once per frame; both PATINAE_TIMING (log) and
        // PATINAE_PERF (HUD) consume the same snapshot.
        let gpu_stats = self
            .rendering
            .renderer
            .as_ref()
            .and_then(|r| r.take_frame_stats_for_log());

        if let Some(t0) = t_total {
            let total_ms = t0.elapsed().as_secs_f32() * 1000.0;
            eprintln!("[patinae] render_frame TOTAL: {total_ms:.2} ms");
            if let Some(stats) = gpu_stats.as_ref() {
                eprintln!(
                    "  [gpu] shadow={:.2}  opaque={:.2}  translucent={:.2}  composite={:.2}  \
	                     marking={:.2}  picking={:.2}  reproject={:.2}  silhouette={:.2}  \
	                     cull={:.2}  atlas_ao={:.2}  total={:.2} ms",
                    stats.gpu_shadow_ms,
                    stats.gpu_opaque_ms,
                    stats.gpu_translucent_ms,
                    stats.gpu_composite_ms,
                    stats.gpu_marking_ms,
                    stats.gpu_picking_ms,
                    stats.gpu_picking_reproject_ms,
                    stats.gpu_silhouette_ms,
                    stats.gpu_cull_ms,
                    stats.gpu_atlas_ao_ms,
                    stats.gpu_total_ms,
                );
            }
            if let Some(memory) = self
                .rendering
                .renderer
                .as_ref()
                .map(|r| r.memory_snapshot())
            {
                eprintln!("  {}", memory.timing_line());
            }
            if let Some(timings) = self
                .rendering
                .renderer
                .as_ref()
                .map(|r| r.last_sync_timings())
            {
                let scene = timings.scene_store_fragmentation;
                eprintln!(
                    "  [gpu-mem] scene_store live={} allocated={} orphaned={} capacity={} slack={}",
                    format_mib(scene.live_bytes),
                    format_mib(scene.allocated_bytes),
                    format_mib(scene.orphaned_bytes),
                    format_mib(scene.capacity_bytes),
                    format_mib(scene.capacity_slack_bytes),
                );
                if timings.scene_store_compaction.ran {
                    let compaction = timings.scene_store_compaction;
                    let largest = compaction
                        .largest_orphaned_buffer
                        .map(|kind| kind.label())
                        .unwrap_or("none");
                    eprintln!(
                        "  [gpu-mem] scene_store compacted reclaimed={} capacity_before={} \
                         capacity_after={} orphaned_before={} moved_objects={} largest_orphaned={}",
                        format_mib(compaction.reclaimed_bytes),
                        format_mib(compaction.capacity_before_bytes),
                        format_mib(compaction.capacity_after_bytes),
                        format_mib(compaction.orphaned_before_bytes),
                        compaction.moved_objects,
                        largest,
                    );
                }
            }
        }

        if self.diagnostics.perf_enabled {
            self.update_perf_hud(&vp, gpu_stats.as_ref());
        }

        self.capture_pending_recent_thumbnail(app);
    }
}

pub(super) fn viewport_label_from_projected(
    label: &ProjectedAnnotationLabel,
    scale_factor: f32,
) -> ViewportLabel {
    let scale_factor = scale_factor.max(f32::EPSILON);
    let (anchor_x, anchor_y) = label.alignment.anchor_factors();
    ViewportLabel {
        x: label.x / scale_factor,
        y: label.y / scale_factor,
        text: label.text.clone().into(),
        color: slint::Color::from_argb_f32(
            label.color[3],
            label.color[0],
            label.color[1],
            label.color[2],
        ),
        // Label sizes are device-independent UI pixels. Slint applies the
        // window scale factor when it rasterizes the glyphs.
        size: label.size,
        anchor_x,
        anchor_y,
        alignment: label.alignment.as_str().into(),
        owner_id: label.owner_id.get().to_string().into(),
        insertion_ordinal: i32::try_from(label.insertion_ordinal).unwrap_or(i32::MAX),
        display_order: label.display_order.to_string().into(),
    }
}

pub(super) fn viewport_image_signature(image: Option<&ViewportImage>) -> u64 {
    let Some(image) = image else {
        return 0;
    };
    let mut sig = image.width as u64;
    sig = mix_generation(sig, image.height as u64);
    sig = mix_generation(sig, image.data.len() as u64);
    sig = mix_generation(sig, image.data.as_ptr() as usize as u64);
    if let Some(first) = image.data.first() {
        sig = mix_generation(sig, *first as u64);
    }
    if let Some(last) = image.data.last() {
        sig = mix_generation(sig, *last as u64);
    }
    sig
}

pub(super) fn viewport_image_to_slint(image: Option<&ViewportImage>) -> Option<Image> {
    let image = image?;
    let expected_len = viewport_image_len(image.width, image.height)?;
    if image.width == 0 || image.height == 0 || image.data.len() != expected_len {
        return None;
    }

    let buffer =
        SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&image.data, image.width, image.height);
    Some(Image::from_rgba8(buffer))
}

pub(super) fn viewport_image_len(width: u32, height: u32) -> Option<usize> {
    (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)
}
