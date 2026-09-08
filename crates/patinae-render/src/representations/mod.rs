//! Representation trait + per-rep modules.
//!
//! A `Representation` owns its geometry (group 3, instance / vertex buffers +
//! per-rep params). Group 2 is the scene-wide [`crate::scene_store::SceneStore`] bound by
//! `RenderState` with the per-object dynamic offset.

pub mod cartoon;
pub(crate) mod catalog;
pub(crate) mod cullable;
pub mod dot;
pub mod ellipsoid;
pub mod line;
pub mod mesh;
pub mod sphere;
pub mod stick;
pub mod surface;
mod viewport_lod;

use crate::picking::RepKind;
use crate::render_input::RenderObjectInput;
use crate::render_state::state::GeometryRuntime;
use crate::representation_budget::{RepBuildDecision, RepQualityLevel};
use crate::GpuMemoryUsage;

use patinae_mol::DirtyFlags;
use patinae_settings::ResolvedSettings;

/// Resources every per-rep `record_compute_build` needs to record its GPU
/// build dispatch. Bundled to keep the trait signature short and to let
/// reps reach the shared `GeometryRuntime` (per-rep compute pipelines,
/// render-side params layouts, cull pipeline) uniformly.
///
/// `pub` because the trait `Representation` is also `pub`, but
/// instances cannot be constructed outside this crate
/// (`GeometryRuntime`'s fields are crate-private).
pub struct BuildCtx<'a> {
    pub(crate) encoder: &'a mut wgpu::CommandEncoder,
    pub(crate) scene_bg: &'a wgpu::BindGroup,
    pub(crate) object_coords_scene_bg: Option<&'a wgpu::BindGroup>,
    pub(crate) obj_dynamic_offset: u32,
    pub(crate) queue: &'a wgpu::Queue,
    pub(crate) device: &'a wgpu::Device,
    pub(crate) pipelines: &'a GeometryRuntime,
}

/// Per-frame state a cullable rep needs to upload its `CullParams` and
/// reset its indirect-draw args. Passed by `&` because `plan_cull`
/// uploads through `queue` only — no encoder state changes here.
pub struct CullPlanCtx<'a> {
    pub(crate) queue: &'a wgpu::Queue,
    pub(crate) view_proj: [[f32; 4]; 4],
    pub(crate) frustum_planes: [[f32; 4]; 6],
}

/// What a culled rep returns to the outer dispatch loop: the bind group
/// the cull kernel reads + the upper bound on raw instance count (which
/// drives workgroup count). The caller pairs this with the crate-local
/// representation catalog for the actual `dispatch_kind` call.
pub struct CullPlan<'a> {
    pub(crate) bind_group: &'a wgpu::BindGroup,
    pub(crate) upper: u32,
}

/// Context for camera-dependent LOD feedback passes.
pub struct ViewportLodCtx<'a> {
    pub(crate) encoder: &'a mut wgpu::CommandEncoder,
    pub(crate) scene_bg: &'a wgpu::BindGroup,
    pub(crate) obj_dynamic_offset: u32,
    pub(crate) pipelines: &'a GeometryRuntime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrawPhase {
    Opaque,
    FastOverlay,
    Wboit,
}

pub trait Representation: std::any::Any {
    // Identity and downcasting.
    fn kind(&self) -> RepKind;

    /// Erased downcast hooks.
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
    fn as_any(&self) -> &dyn std::any::Any;

    // Scene sync and per-representation ownership.
    /// CPU-side rebuild + per-rep buffer prep. Most reps record a compute
    /// dispatch in [`Self::record_compute_build`] and do little here.
    fn build(
        &mut self,
        input: &RenderObjectInput,
        settings: &ResolvedSettings,
        dirty: DirtyFlags,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    );

    /// Applies the current budget decision before building resources.
    fn apply_budget_decision(
        &mut self,
        _decision: RepBuildDecision,
        _quality: RepQualityLevel,
    ) -> bool {
        false
    }

    // Visibility contracts.
    /// Whether this rep's contents are fully opaque this frame. Reps with
    /// any per-atom or per-rep transparency override and return `false`
    /// while transparent fragments are visible.
    fn is_opaque(&self) -> bool {
        true
    }

    fn draw_phase(&self) -> DrawPhase {
        if self.is_opaque() {
            DrawPhase::Opaque
        } else {
            DrawPhase::Wboit
        }
    }

    /// Whether this representation contributes depth to shadow-map and
    /// atlas-AO passes. The default follows the opaque fast-path, while
    /// representations with special screen-depth behavior can opt in or out
    /// independently.
    fn casts_shadow(&self) -> bool {
        self.is_opaque()
    }

    // GPU build/compute contract.
    /// Record a compute dispatch that (re)builds this rep's instance /
    /// vertex buffer on the GPU. Default is a no-op returning `false`,
    /// signalling the caller that no compute work is pending. Reps that
    /// dispatched return `true` so `RenderState` knows the encoder needs
    /// a submission. `ctx.pipelines` is the shared `GeometryRuntime`
    /// from which the rep picks the compute pipelines it owns.
    fn record_compute_build(&mut self, _ctx: &mut BuildCtx<'_>) -> bool {
        false
    }

    /// Camera-cull contract. Cullable reps upload `CullParams`, reset
    /// their indirect-draw args, and return the bind group + upper bound
    /// the cull kernel will dispatch over. Non-cullable reps return
    /// `None` (the default) and the outer loop skips them.
    fn plan_cull(&mut self, _ctx: &CullPlanCtx<'_>) -> Option<CullPlan<'_>> {
        None
    }

    /// Apply any asynchronous viewport-LOD feedback before this frame's
    /// compute/cull work. Returns `true` when the representation changed its
    /// geometry and cached id passes must be invalidated.
    fn poll_viewport_lod(&mut self, _queue: &wgpu::Queue) -> bool {
        false
    }

    /// Record tiny GPU readbacks used by viewport-sensitive LOD decisions.
    /// Defaults to no-op for representations that do not adapt to the camera.
    fn record_viewport_lod_readback(&mut self, _ctx: &mut ViewportLodCtx<'_>) {}

    /// Estimated persistent GPU memory owned by this representation.
    fn memory_usage(&self) -> GpuMemoryUsage {
        GpuMemoryUsage::default()
    }

    // Draw contracts. `record_shadow_depth` deliberately uses raw,
    // un-compacted geometry for camera-independent lighting/occlusion passes.
    fn record_translucent<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>);
    fn record_picking<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>);
    /// Picking with the complete shared source, independent of camera culling.
    fn record_raw_picking<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        self.record_picking(pass);
    }
    fn record_depth_prepass<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        self.record_translucent(pass);
    }
    /// Prepare any per-representation draw state needed by the shadow pass.
    /// Camera-culled reps use this to derive an uncullled indirect draw from
    /// their raw instance count before the render pass begins.
    fn prepare_shadow_depth(&self, _encoder: &mut wgpu::CommandEncoder, _queue: &wgpu::Queue) {}
    /// Depth-only shadow draw. Defaults to the normal depth pre-pass path;
    /// camera-culled reps override this to draw raw, uncullled geometry.
    fn record_shadow_depth<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        self.record_depth_prepass(pass);
    }
}

pub(crate) fn prepare_raw_shadow_indirect(
    encoder: &mut wgpu::CommandEncoder,
    queue: &wgpu::Queue,
    raw_count: Option<&wgpu::Buffer>,
    indirect: Option<&wgpu::Buffer>,
    seed: &[u32; 4],
) {
    let (Some(raw_count), Some(indirect)) = (raw_count, indirect) else {
        return;
    };
    queue.write_buffer(indirect, 0, bytemuck::cast_slice(seed));
    encoder.copy_buffer_to_buffer(raw_count, 0, indirect, 4, 4);
}
