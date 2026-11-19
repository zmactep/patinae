//! Capture and geometry export using the live renderer resources.

use super::*;

impl CaptureRenderer for ViewportRenderer {
    fn gpu_device(&self) -> &Arc<wgpu::Device> {
        &self.state.ctx.device
    }

    fn gpu_queue(&self) -> &Arc<wgpu::Queue> {
        &self.state.ctx.queue
    }

    fn capture_png(
        &mut self,
        path: &Path,
        width: u32,
        height: u32,
        camera: &mut Camera,
        registry: &mut ObjectRegistry,
        settings: &Settings,
        named: &NamedPalette,
        themed: &ThemedPalette,
        clear_color: [f32; 3],
    ) -> Result<(), ViewerError> {
        self.refresh_recovery_for_scene_change(registry);
        if let Some(message) = self.apply_runtime_memory_policy(settings, registry) {
            log::info!("{message}");
        }
        if self.recovery.is_blocked() {
            return Err(ViewerError::capture_error(
                "Rendering is blocked after GPU memory exhaustion",
            ));
        }
        // Build per-atom colours + render objects via the same bridges the
        // live frame uses, so the captured PNG matches the on-screen scene
        // (modulo resolution).
        // Capture path doesn't have access to a `Session`; render with no
        // markers (selection / hover overlays are interactive UI affordances
        // and don't belong in offscreen captures).
        let markers = ResolvedSceneMarkers::default();
        let mut object_inputs = Vec::new();
        let mut map_inputs = Vec::new();
        visit_render_scene_deferred(
            registry,
            settings,
            named,
            themed,
            &markers,
            &mut |_name, obj| object_inputs.push(obj),
            &mut |_name, map| map_inputs.push(map),
        );
        let mut resolved_strokes = ResolvedSceneStrokes::default();
        resolved_strokes.rebuild(registry, settings, named);
        let stroke_inputs = resolved_strokes.render_inputs();

        let frame = frame_uniforms_from_camera(camera, settings, (width, height), clear_color);

        let resolved = ResolvedSettings::resolve(settings, None);
        let lod = object_inputs
            .first()
            .map(|o| o.lod)
            .unwrap_or(SceneLod::Auto);
        let input = RenderInput {
            objects: &object_inputs,
            maps: &map_inputs,
            strokes: &stroke_inputs,
            settings: &resolved,
            lod,
        };

        configure_frame_effects(&mut self.state, settings, named);
        self.state
            .set_clear_color(clear_color, settings.ui.opaque_background);

        patinae_render::capture::capture_png(&mut self.state, path, width, height, &frame, &input)
            .map_err(|e| ViewerError::capture_error(e.to_string()))?;
        drop(object_inputs);
        registry.clear_all_dirty_objects();

        Ok(())
    }

    fn set_viewport_gpu_image_from_buffer(
        &mut self,
        buffer: &wgpu::Buffer,
        buffer_size: u64,
        width: u32,
        height: u32,
    ) -> Result<(), ViewerError> {
        let image = copy_rgba_buffer_to_viewport_texture(
            &self.state.ctx.device,
            &self.state.ctx.queue,
            buffer,
            buffer_size,
            width,
            height,
        )
        .map_err(|error| ViewerError::capture_error(error.to_string()))?;
        self.viewport_gpu_image = Some(image);
        Ok(())
    }

    fn save_viewport_gpu_image(&mut self, path: &Path) -> Result<Option<(u32, u32)>, ViewerError> {
        let Some(image) = self.viewport_gpu_image.as_ref() else {
            return Ok(None);
        };
        let rgba = image
            .read_rgba(&self.state.ctx.device, &self.state.ctx.queue)
            .map_err(|error| ViewerError::capture_error(error.to_string()))?;
        let buffer = image::RgbaImage::from_raw(image.width, image.height, rgba)
            .ok_or_else(|| ViewerError::capture_error("RgbaImage::from_raw size mismatch"))?;
        buffer
            .save_with_format(path, image::ImageFormat::Png)
            .map_err(|error| ViewerError::capture_error(format!("PNG encode failed: {error}")))?;
        Ok(Some((image.width, image.height)))
    }

    fn clear_viewport_gpu_image(&mut self) {
        self.viewport_gpu_image = None;
    }

    fn export_displayed_geometry(
        &mut self,
        _camera: &mut Camera,
        registry: &mut ObjectRegistry,
        settings: &Settings,
        named: &NamedPalette,
        themed: &ThemedPalette,
        _clear_color: [f32; 3],
        options: &GeometryExportOptions,
    ) -> Result<DisplayedGeometry, ViewerError> {
        let markers = ResolvedSceneMarkers::default();
        let mut object_inputs = Vec::new();
        let mut map_inputs = Vec::new();
        visit_render_scene_deferred(
            registry,
            settings,
            named,
            themed,
            &markers,
            &mut |_name, obj| object_inputs.push(obj),
            &mut |_name, map| map_inputs.push(map),
        );

        let resolved = ResolvedSettings::resolve(settings, None);
        let lod = object_inputs
            .first()
            .map(|o| o.lod)
            .unwrap_or(SceneLod::Auto);
        let input = RenderInput {
            objects: &object_inputs,
            maps: &map_inputs,
            strokes: &[],
            settings: &resolved,
            lod,
        };

        let geometry = self
            .state
            .export_displayed_geometry(&input, options)
            .map_err(|e| ViewerError::capture_error(e.to_string()))?;
        drop(object_inputs);
        registry.clear_all_dirty_objects();

        // Restore live viewport size on the next frame, matching capture.
        self.last_viewport_size = None;
        Ok(geometry)
    }

    fn for_each_trace_geometry_chunk(
        &mut self,
        _camera: &mut Camera,
        registry: &mut ObjectRegistry,
        settings: &Settings,
        named: &NamedPalette,
        themed: &ThemedPalette,
        _clear_color: [f32; 3],
        options: &GeometryExportOptions,
        visitor: &mut dyn FnMut(TraceGeometryChunk) -> Result<(), String>,
    ) -> Result<(), ViewerError> {
        let markers = ResolvedSceneMarkers::default();
        let mut object_inputs = Vec::new();
        let mut map_inputs = Vec::new();
        visit_render_scene_deferred(
            registry,
            settings,
            named,
            themed,
            &markers,
            &mut |_name, obj| object_inputs.push(obj),
            &mut |_name, map| map_inputs.push(map),
        );

        let resolved = ResolvedSettings::resolve(settings, None);
        let lod = object_inputs
            .first()
            .map(|o| o.lod)
            .unwrap_or(SceneLod::Auto);
        let input = RenderInput {
            objects: &object_inputs,
            maps: &map_inputs,
            strokes: &[],
            settings: &resolved,
            lod,
        };

        self.state
            .for_each_trace_geometry_chunk(&input, options, visitor)
            .map_err(|e| ViewerError::capture_error(e.to_string()))?;
        drop(object_inputs);
        registry.clear_all_dirty_objects();

        self.last_viewport_size = None;
        Ok(())
    }

    fn visit_render_artifacts(
        &mut self,
        _camera: &mut Camera,
        registry: &mut ObjectRegistry,
        settings: &Settings,
        named: &NamedPalette,
        themed: &ThemedPalette,
        _clear_color: [f32; 3],
        visitor: &mut dyn FnMut(RenderArtifactSnapshot<'_>) -> Result<(), String>,
    ) -> Result<(), ViewerError> {
        let markers = ResolvedSceneMarkers::default();
        let mut object_inputs = Vec::new();
        let mut map_inputs = Vec::new();
        visit_render_scene_deferred(
            registry,
            settings,
            named,
            themed,
            &markers,
            &mut |_name, obj| object_inputs.push(obj),
            &mut |_name, map| map_inputs.push(map),
        );

        let resolved = ResolvedSettings::resolve(settings, None);
        let lod = object_inputs
            .first()
            .map(|o| o.lod)
            .unwrap_or(SceneLod::Auto);
        let input = RenderInput {
            objects: &object_inputs,
            maps: &map_inputs,
            strokes: &[],
            settings: &resolved,
            lod,
        };

        if object_inputs
            .iter()
            .any(|object| object.instances.is_some())
        {
            return Err(ViewerError::capture_error(
                "Ray rendering requires explicit objects; run materialize first".to_string(),
            ));
        }
        let snapshot = self
            .state
            .render_artifact_snapshot(&input)
            .map_err(|error| ViewerError::capture_error(error.to_string()))?;
        visitor(snapshot).map_err(ViewerError::capture_error)?;
        drop(object_inputs);
        registry.clear_all_dirty_objects();

        self.last_viewport_size = None;
        Ok(())
    }
}
