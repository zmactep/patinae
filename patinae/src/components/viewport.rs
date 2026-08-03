//! 3D viewport renderer for the Patinae (Slint) GUI.
//!
//! Pure GPU renderer — owns wgpu resources and shading pipelines.
//! Input handling and viewport state live in the application bridge layer.
//!
//! Drives `patinae_render::RenderState` for both live frames and headless
//! PNG capture (via `CaptureRenderer`).

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use patinae_color::{NamedPalette, ThemedPalette};
use patinae_render::picking::readback::PendingPick;
use patinae_render::{
    copy_rgba_buffer_to_viewport_texture, estimate_texture_2d_bytes, is_wgpu_oom,
    render_memory_policy_from_settings, select_render_memory_policy, DisplayedGeometry,
    GeometryExportOptions, GpuMemoryCategory, GpuMemorySnapshot, GpuMemoryUsage, GpuViewportImage,
    RenderArtifactSnapshot, RenderConfig, RenderInput, RenderMemoryPolicy, RenderMemoryProfile,
    RenderMemoryRecoveryAction, RenderMemoryRecoveryStage, RenderMemorySelectionInput, RenderState,
    RenderSyncTimings, SceneLod, TraceGeometryChunk, PERFORMANCE_MAX_BUFFER_SIZE,
    PERFORMANCE_MAX_STORAGE_BUFFER_BINDING_SIZE,
};
use patinae_scene::{Camera, CaptureRenderer, ObjectRegistry, PickHit, Session, ViewerError};
use patinae_settings::{ResolvedSettings, Settings, ShadingMode};

use patinae_scene::bridge::{
    frame_uniforms_from_camera, frame_uniforms_from_session, resolve_pick, resolve_setting_color,
    visit_render_scene, CachedRenderScene, ProjectedAnnotationLabel, ProjectedSceneLabels,
    ResolvedSceneColors, ResolvedSceneMarkers, ResolvedSceneStrokes,
};

const VIEWPORT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const LARGE_LOD_HOVER_PICK_INTERVAL: Duration = Duration::from_millis(33);

pub struct AdapterInfo {
    pub device_name: String,
    pub backend: String,
}

#[derive(Debug, Clone)]
struct WgpuErrorFlags {
    oom: Arc<AtomicBool>,
    device_lost: Arc<AtomicBool>,
}

impl Default for WgpuErrorFlags {
    fn default() -> Self {
        Self {
            oom: Arc::new(AtomicBool::new(false)),
            device_lost: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl WgpuErrorFlags {
    fn mark_oom(&self) {
        self.oom.store(true, Ordering::Relaxed);
    }

    fn mark_device_lost(&self) {
        self.device_lost.store(true, Ordering::Relaxed);
    }

    fn take_oom(&self) -> bool {
        self.oom.swap(false, Ordering::Relaxed)
    }

    fn take_device_lost(&self) -> bool {
        self.device_lost.swap(false, Ordering::Relaxed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewportRenderFailure {
    OutOfMemory,
    DeviceLost,
}

pub(crate) struct ViewportRenderOutcome {
    pub(crate) image: Option<slint::Image>,
    pub(crate) warning: Option<String>,
    pub(crate) request_redraw: bool,
    pub(crate) present_frame: bool,
}

impl ViewportRenderOutcome {
    fn frame(image: Option<slint::Image>) -> Self {
        Self {
            image,
            warning: None,
            request_redraw: false,
            present_frame: true,
        }
    }

    fn recovery(warning: Option<String>, request_redraw: bool) -> Self {
        Self {
            image: None,
            warning,
            request_redraw,
            present_frame: false,
        }
    }
}

/// GPU viewport that renders a [`Session`] to an offscreen wgpu texture.
pub struct ViewportRenderer {
    /// Live-render path — patinae-render.
    state: RenderState,
    /// Current renderer configuration, including temporary recovery profile.
    config: RenderConfig,
    /// Startup policy used when runtime settings select `auto`.
    auto_memory_policy: RenderMemoryPolicy,
    /// OOM recovery state relative to the sequence baseline profile.
    recovery: RenderMemoryRecoveryStage,
    /// Error signals reported outside explicit error scopes.
    wgpu_errors: WgpuErrorFlags,
    /// Last viewport size — used to detect resize before each frame.
    last_viewport_size: Option<(u32, u32)>,
    /// Persistent host-side render cache shared with the web viewer.
    render_scene: CachedRenderScene,
    /// Camera-dependent overlay cache, independent of semantic annotation resolution.
    projected_labels: ProjectedSceneLabels,
    /// Last structural object-registry generation observed by recovery logic.
    last_registry_generation: u64,
    /// In-flight async hover pick. At most one outstanding handle.
    pending_hover: Option<PendingPick>,
    /// Last hover readback submission, used to pace 3J3Q-class sphere scenes.
    last_hover_submit: Option<Instant>,
    /// Ring of offscreen color textures handed to Slint. Rotating across
    /// 3 textures lets us write the next frame's image while Slint is
    /// still reading the previous frame — otherwise the GPU serialises
    /// our write on Slint's read fence, capping fps independent of CPU.
    color_textures: Vec<wgpu::Texture>,
    color_textures_next: usize,
    /// Transient renderer-owned ray output texture for viewport `ray`.
    viewport_gpu_image: Option<GpuViewportImage>,
    /// Construction-time memory policy for native viewport handoff resources.
    memory_policy: RenderMemoryPolicy,
    warned_unsupported_memory_policy: bool,
}

impl ViewportRenderer {
    /// Projects all annotation labels resolved for the latest rendered frame.
    pub(crate) fn annotation_labels(
        &mut self,
        camera: &Camera,
        viewport: (f32, f32, f32, f32),
    ) -> &[ProjectedAnnotationLabel] {
        self.projected_labels
            .rebuild(camera, viewport, self.render_scene.annotation_bundles());
        self.projected_labels.labels()
    }

    /// Clears the host overlay while a captured viewport image replaces live rendering.
    pub(crate) fn clear_annotation_labels(&mut self) {
        self.projected_labels.clear();
    }

    fn new_with_config_and_errors(
        device: wgpu::Device,
        queue: wgpu::Queue,
        config: RenderConfig,
        wgpu_errors: WgpuErrorFlags,
    ) -> Self {
        // wgpu::Device / wgpu::Queue clone bumps an internal Arc refcount.
        let device_arc = Arc::new(device);
        let queue_arc = Arc::new(queue);
        let memory_policy = config.memory;
        let recovery = RenderMemoryRecoveryStage::normal(memory_policy.profile);
        let state =
            RenderState::with_config(device_arc, queue_arc, VIEWPORT_FORMAT, (1, 1), config);
        Self {
            state,
            config,
            auto_memory_policy: memory_policy,
            recovery,
            wgpu_errors,
            last_viewport_size: None,
            render_scene: CachedRenderScene::default(),
            projected_labels: ProjectedSceneLabels::default(),
            last_registry_generation: 0,
            pending_hover: None,
            last_hover_submit: None,
            color_textures: Vec::new(),
            color_textures_next: 0,
            viewport_gpu_image: None,
            memory_policy,
            warned_unsupported_memory_policy: false,
        }
    }

    /// Create from a Slint graphics API handle. Returns the renderer and
    /// optional adapter info (for HUD display).
    pub fn setup(api: &slint::GraphicsAPI) -> Option<(Self, Option<AdapterInfo>)> {
        let slint::GraphicsAPI::WGPU29 {
            instance,
            device,
            queue,
            ..
        } = api
        else {
            return None;
        };

        let adapter_info =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                ..Default::default()
            }))
            .ok()
            .map(|adapter| adapter.get_info());
        let info = adapter_info.as_ref().map(|info| AdapterInfo {
            device_name: info.name.clone(),
            backend: format!("{:?}", info.backend),
        });
        let memory_policy = render_memory_policy_for_device(
            adapter_info.as_ref(),
            &device.limits(),
            "PATINAE_RENDER_MEMORY_PROFILE",
            false,
        );
        log::info!(
            "render memory profile: profile={} budget={:?} handoff_ring={}",
            memory_policy.profile,
            memory_policy.budget_bytes,
            memory_policy.frame_targets.viewport_handoff_ring
        );
        log_wgpu_device_diagnostics(device);
        let wgpu_errors = install_wgpu_error_handlers(device);

        let renderer = Self::new_with_config_and_errors(
            device.clone(),
            queue.clone(),
            RenderConfig {
                memory: memory_policy,
                ..Default::default()
            },
            wgpu_errors,
        );
        Some((renderer, info))
    }

    /// Render the scene and return the result as a Slint image.
    /// Drain whatever FrameStats are ready right now (gated by the
    /// `patinae-render/stats` feature, which patinae enables by default).
    pub fn take_frame_stats_for_log(&self) -> Option<patinae_render::FrameStats> {
        self.state.take_frame_stats()
    }

    /// Return the renderer memory profile selected at construction.
    pub fn memory_profile(&self) -> RenderMemoryProfile {
        self.memory_policy.profile
    }

    /// Return a renderer memory snapshot including desktop handoff textures.
    pub fn memory_snapshot(&self) -> GpuMemorySnapshot {
        let mut snapshot = self.state.memory_snapshot();
        if let Some((width, height)) = self.last_viewport_size {
            let bytes = estimate_texture_2d_bytes(width, height, VIEWPORT_FORMAT);
            let count = self.color_textures.len() as u64;
            snapshot.add_usage(
                GpuMemoryCategory::ViewportHandoff,
                GpuMemoryUsage::new(
                    bytes.saturating_mul(count),
                    bytes.saturating_mul(count),
                    bytes.saturating_mul(count),
                    count,
                ),
            );
        }
        if let Some(image) = self.viewport_gpu_image.as_ref() {
            snapshot.add_allocation(
                GpuMemoryCategory::PluginOrArtifact,
                estimate_texture_2d_bytes(image.width, image.height, VIEWPORT_FORMAT),
            );
        }
        snapshot
    }

    /// Return timings from the most recent renderer sync.
    pub fn last_sync_timings(&self) -> RenderSyncTimings {
        self.state.last_sync_timings()
    }

    pub fn take_memory_warnings(&mut self) -> Vec<String> {
        self.state.take_memory_warnings()
    }

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

    fn refresh_recovery_for_scene_change(&mut self, session: &mut Session) {
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
        log::info!(
            "renderer memory recovery reset after scene generation changed; baseline profile={}",
            base_profile
        );
    }

    fn render_frame_scoped(
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
        Ok(texture)
    }

    fn recover_after_oom(&mut self, session: &mut Session) -> ViewportRenderOutcome {
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

    fn block_after_device_loss(&mut self) -> ViewportRenderOutcome {
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

    fn prepare_oom_retry(&mut self, session: &mut Session) {
        self.drop_transient_resources();
        let compaction = self.state.force_scene_store_compaction();
        self.state.drop_unused_optional_targets();
        session.registry.mark_all_dirty();
        if compaction.ran {
            log::info!(
                "renderer OOM recovery compacted SceneStore: capacity_before={:.2} MiB orphaned_before={:.2} MiB moved_objects={}",
                patinae_render::bytes_to_mib(compaction.capacity_before_bytes),
                patinae_render::bytes_to_mib(compaction.orphaned_before_bytes),
                compaction.moved_objects
            );
        }
    }

    fn apply_runtime_memory_policy(&mut self, session: &mut Session) -> Option<String> {
        let requested =
            render_memory_policy_from_settings(&session.settings, self.auto_memory_policy);
        if requested == self.memory_policy {
            return None;
        }
        if !memory_policy_supported_by_device(requested, &self.state.ctx.device.limits()) {
            if !self.warned_unsupported_memory_policy {
                self.warned_unsupported_memory_policy = true;
                return Some(format!(
                    "Renderer memory profile {} requires GPU limits unavailable on this device; keeping {}.",
                    requested.profile, self.memory_policy.profile
                ));
            }
            return None;
        }

        self.config.memory = requested;
        self.memory_policy = requested;
        self.warned_unsupported_memory_policy = false;
        self.recovery.reset(requested.profile);
        self.rebuild_for_policy(requested);
        session.registry.mark_all_dirty();
        Some(format!(
            "Renderer memory profile switched to {} by user setting.",
            requested.profile
        ))
    }

    fn rebuild_for_profile(&mut self, profile: RenderMemoryProfile) {
        self.rebuild_for_policy(RenderMemoryPolicy::from_profile(profile));
    }

    fn rebuild_for_policy(&mut self, policy: RenderMemoryPolicy) {
        let device = self.state.ctx.device.clone();
        let queue = self.state.ctx.queue.clone();
        self.config.memory = policy;
        self.memory_policy = self.config.memory;
        let viewport = self.last_viewport_size.unwrap_or((1, 1));
        self.state =
            RenderState::with_config(device, queue, VIEWPORT_FORMAT, viewport, self.config);
        self.drop_transient_resources();
    }

    fn drop_transient_resources(&mut self) {
        self.pending_hover = None;
        self.last_hover_submit = None;
        self.viewport_gpu_image = None;
        self.color_textures.clear();
        self.color_textures_next = 0;
    }

    pub fn clear_viewport_gpu_image(&mut self) -> bool {
        self.viewport_gpu_image.take().is_some()
    }

    pub(crate) fn invalidate_live_textures(&mut self) {
        self.last_viewport_size = None;
        self.color_textures.clear();
        self.color_textures_next = 0;
    }

    fn render_frame(&mut self, session: &mut Session, width: u32, height: u32) -> wgpu::Texture {
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
        let silhouette_ink = resolve_setting_color(
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

    /// Async hover pick: skip while a previous hover readback is in flight.
    /// - `Some(Some(hit))` — fresh hit landed this frame.
    /// - `Some(None)` — fresh miss landed this frame.
    /// - `None` — no fresh result (one is in flight or being submitted).
    pub fn poll_hover_pick(
        &mut self,
        session: &Session,
        x: u32,
        y: u32,
    ) -> Option<Option<PickHit>> {
        let (width, height) = self.last_viewport_size?;
        if x >= width || y >= height {
            return None;
        }
        self.refresh_pick_frame_uniforms(session, (width, height));

        if let Some(pending) = self.pending_hover.as_ref() {
            // Wait for the GPU to finish; never submit a second pick.
            let result = self.state.try_collect_pick(pending)?;
            self.pending_hover = None;
            return Some(
                result.and_then(|hit| resolve_pick(hit, self.render_scene.object_names(), session)),
            );
        }

        // No pick in flight — submit a fresh one. `submit_pick` returns
        // `None` if the renderer was built with `PickingMode::Disabled`;
        // in that case there's nothing to wait on.
        if self.large_lod_hover_pick_waiting() {
            return None;
        }
        self.pending_hover = self.state.submit_pick(x, y);
        if self.pending_hover.is_some() {
            self.last_hover_submit = Some(Instant::now());
        }
        None
    }

    fn large_lod_hover_pick_waiting(&self) -> bool {
        (self.state.sphere_lod_diagnostics().active || self.state.stick_lod_diagnostics().active)
            && self
                .last_hover_submit
                .is_some_and(|last| last.elapsed() < LARGE_LOD_HOVER_PICK_INTERVAL)
    }

    /// Synchronous click pick. Uses a separate renderer readback from hover,
    /// so it never waits for a pending hover query before resolving a click.
    pub fn pick_at_click(&mut self, session: &Session, x: u32, y: u32) -> Option<PickHit> {
        let (width, height) = self.last_viewport_size?;
        if x >= width || y >= height {
            return None;
        }
        self.refresh_pick_frame_uniforms(session, (width, height));

        let hit = self.state.pick(x, y)?;
        resolve_pick(hit, self.render_scene.object_names(), session)
    }

    fn refresh_pick_frame_uniforms(&mut self, session: &Session, viewport: (u32, u32)) {
        let frame = frame_uniforms_from_session(session, viewport, session.clear_color);
        self.state.uniforms = frame;
        self.state.ctx.upload_frame(&self.state.uniforms);
    }
}

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
        let colors = ResolvedSceneColors::build(registry, settings, named, themed);
        // Capture path doesn't have access to a `Session`; render with no
        // markers (selection / hover overlays are interactive UI affordances
        // and don't belong in offscreen captures).
        let markers = ResolvedSceneMarkers::default();
        let mut object_inputs = Vec::new();
        let mut map_inputs = Vec::new();
        visit_render_scene(
            registry,
            settings,
            &colors,
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
        let colors = ResolvedSceneColors::build(registry, settings, named, themed);
        let markers = ResolvedSceneMarkers::default();
        let mut object_inputs = Vec::new();
        let mut map_inputs = Vec::new();
        visit_render_scene(
            registry,
            settings,
            &colors,
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
        let colors = ResolvedSceneColors::build(registry, settings, named, themed);
        let markers = ResolvedSceneMarkers::default();
        let mut object_inputs = Vec::new();
        let mut map_inputs = Vec::new();
        visit_render_scene(
            registry,
            settings,
            &colors,
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
        let colors = ResolvedSceneColors::build(registry, settings, named, themed);
        let markers = ResolvedSceneMarkers::default();
        let mut object_inputs = Vec::new();
        let mut map_inputs = Vec::new();
        visit_render_scene(
            registry,
            settings,
            &colors,
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

        let snapshot = self.state.render_artifact_snapshot(&input);
        visitor(snapshot).map_err(ViewerError::capture_error)?;
        registry.clear_all_dirty_objects();

        self.last_viewport_size = None;
        Ok(())
    }
}

fn log_wgpu_device_diagnostics(device: &wgpu::Device) {
    let limits = device.limits();
    let features = device.features();
    log::info!(
        "wgpu device limits: max_storage_buffer_binding_size={} max_buffer_size={} \
         max_compute_workgroups_per_dimension={} max_texture_dimension_2d={}",
        limits.max_storage_buffer_binding_size,
        limits.max_buffer_size,
        limits.max_compute_workgroups_per_dimension,
        limits.max_texture_dimension_2d,
    );
    log::info!(
        "wgpu device features: buffer_binding_array={} storage_resource_binding_array={} timestamp_query={}",
        features.contains(wgpu::Features::BUFFER_BINDING_ARRAY),
        features.contains(wgpu::Features::STORAGE_RESOURCE_BINDING_ARRAY),
        features.contains(wgpu::Features::TIMESTAMP_QUERY),
    );
}

fn install_wgpu_error_handlers(device: &wgpu::Device) -> WgpuErrorFlags {
    let flags = WgpuErrorFlags::default();
    let uncaptured = flags.clone();
    device.on_uncaptured_error(Arc::new(move |error| {
        if is_wgpu_oom(&error) {
            uncaptured.mark_oom();
            log::warn!("Uncaptured WGPU OOM: {error}");
        } else {
            log::error!("Uncaptured WGPU error: {error}");
        }
    }));
    let lost = flags.clone();
    device.set_device_lost_callback(move |reason, message| {
        lost.mark_device_lost();
        log::error!("WGPU device lost: {reason:?}: {message}");
    });
    flags
}

pub(crate) fn render_memory_policy_for_device(
    adapter_info: Option<&wgpu::AdapterInfo>,
    limits: &wgpu::Limits,
    env_var: &str,
    is_web: bool,
) -> RenderMemoryPolicy {
    let override_profile = std::env::var(env_var).ok().and_then(|value| match value
        .parse::<RenderMemoryProfile>(
    ) {
        Ok(profile) => Some(profile),
        Err(err) => {
            log::warn!("ignoring invalid {env_var}={value:?}: {err}");
            None
        }
    });
    let input = adapter_info.map_or_else(
        || RenderMemorySelectionInput {
            adapter_type: patinae_render::RenderAdapterType::Other,
            backend: if is_web {
                patinae_render::RenderBackend::BrowserWebGpu
            } else {
                patinae_render::RenderBackend::Other
            },
            max_buffer_size: limits.max_buffer_size,
            max_storage_buffer_binding_size: limits.max_storage_buffer_binding_size,
            max_texture_dimension_2d: limits.max_texture_dimension_2d,
            is_web,
            observed_downgrade: false,
        },
        |info| RenderMemorySelectionInput::from_wgpu(info, limits, is_web),
    );
    select_render_memory_policy(input, override_profile)
}

fn memory_policy_supported_by_device(policy: RenderMemoryPolicy, limits: &wgpu::Limits) -> bool {
    match policy.profile {
        RenderMemoryProfile::Performance => {
            limits.max_buffer_size >= PERFORMANCE_MAX_BUFFER_SIZE
                && limits.max_storage_buffer_binding_size
                    >= PERFORMANCE_MAX_STORAGE_BUFFER_BINDING_SIZE
        }
        RenderMemoryProfile::Balanced
        | RenderMemoryProfile::Lite
        | RenderMemoryProfile::Manual { .. } => true,
    }
}

fn effective_fxaa(settings: &Settings) -> bool {
    settings.ui.antialias > 0 && settings.fxaa.enabled
}

fn configure_ambient_occlusion(state: &mut RenderState, settings: &Settings) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_outcome_requests_redraw_without_presenting_frame() {
        let outcome = ViewportRenderOutcome::recovery(Some("retry".to_string()), true);

        assert_eq!(outcome.warning.as_deref(), Some("retry"));
        assert!(outcome.request_redraw);
        assert!(!outcome.present_frame);
        assert!(outcome.image.is_none());
    }
}
