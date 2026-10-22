//! 3D viewport renderer for the Patinae (Slint) GUI.
//!
//! Pure GPU renderer — owns wgpu resources and shading pipelines.
//! Input handling and viewport state live in the application bridge layer.
//!
//! Drives `patinae_render::RenderState` for both live frames and headless
//! PNG capture (via `CaptureRenderer`).

mod capture;
mod frame;
use frame::*;
mod picking;
mod preparation;
mod recovery;
pub(crate) use preparation::render_memory_policy_for_device;
use preparation::*;

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
    visit_render_scene_deferred, CachedRenderScene, ProjectedAnnotationLabel, ProjectedSceneLabels,
    ResolvedSceneMarkers, ResolvedSceneStrokes,
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

/// GPU-only preparation result; live scene caches are created on attachment.
pub(crate) struct PreparedViewport {
    state: patinae_render::PreparedRenderState,
    config: RenderConfig,
    wgpu_errors: WgpuErrorFlags,
    info: Option<AdapterInfo>,
}

impl PreparedViewport {
    pub(crate) fn finish(self) -> (ViewportRenderer, Option<AdapterInfo>) {
        (
            ViewportRenderer::from_prepared(self.state.finish(), self.config, self.wgpu_errors),
            self.info,
        )
    }
}

impl ViewportRenderer {
    /// Projects all annotation labels resolved for the latest rendered frame.
    pub(crate) fn annotation_labels(
        &mut self,
        camera: &Camera,
        viewport: (f32, f32, f32, f32),
        scale: f32,
    ) -> &[ProjectedAnnotationLabel] {
        self.projected_labels.rebuild_with_scale(
            camera,
            viewport,
            self.render_scene.annotation_bundles(),
            scale,
        );
        self.projected_labels.labels()
    }

    /// Clears the host overlay while a captured viewport image replaces live rendering.
    pub(crate) fn clear_annotation_labels(&mut self) {
        self.projected_labels.clear();
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
}

#[cfg(test)]
mod tests;
