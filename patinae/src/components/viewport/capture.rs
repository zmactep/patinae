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

        // Apply the silhouette setting before render.
        let common = &settings.shading.common;
        let silhouette_ink =
            resolve_setting_color(common.silhouette_color, named, [0.0, 0.0, 0.0, 1.0]);
        if common.silhouettes {
            self.state
                .set_silhouette(true, common.silhouette_width, silhouette_ink);
        } else {
            self.state.set_silhouette(false, 1.0, silhouette_ink);
        }
        let full = &settings.shading.full;
        self.state.set_shadows(
            settings.shading.mode == ShadingMode::Full,
            full.shadow_map_size.max(64) as u32,
            full.shadow_bias,
            full.shadow_intensity,
            full.shadow_pcf.max(1) as u32,
        );
        configure_ambient_occlusion(&mut self.state, settings);
        self.state
            .set_clear_color(clear_color, settings.ui.opaque_background);
        self.state.set_fxaa(effective_fxaa(settings));

        patinae_render::capture::capture_png(&mut self.state, path, width, height, &frame, &input)
            .map_err(|e| ViewerError::capture_error(e.to_string()))?;
        drop(object_inputs);
        registry.clear_all_dirty_objects();

        // Restore live viewport size on the next frame — `render_frame`
        // detects the dimension mismatch and resizes back.
        self.last_viewport_size = None;
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
        let snapshot = self.state.render_artifact_snapshot(&input);
        visitor(snapshot).map_err(ViewerError::capture_error)?;
        drop(object_inputs);
        registry.clear_all_dirty_objects();

        self.last_viewport_size = None;
        Ok(())
    }
}
