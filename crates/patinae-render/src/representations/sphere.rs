//! Sphere representation. Atom-impostor billboards, ray-sphere FS.
//!
//! Uses `SceneStore` (group 2) + GPU compute build. The CPU
//! no longer iterates atoms — the [`crate::compute::sphere_build`] kernel reads
//! the scene-wide atom soup and emits compacted [`SphereInstance`]s into
//! a per-rep instance buffer, with the visible-instance count written
//! atomically into the indirect-draw args buffer. 3J3Q-class scenes use
//! deterministic Auto-LOD sampling in the build shader while the full object
//! is in view. A tiny asynchronous GPU count pass measures the current
//! viewport atom count and temporarily disables sampling for zoomed-in
//! fragments, so local molecular detail remains complete. Selection and hover
//! stay LUT-only: markers never materialize atoms outside the active sphere
//! LOD sample.

use bytemuck::{Pod, Zeroable};
use patinae_mol::{DirtyFlags, RepMask};
use patinae_settings::ResolvedSettings;

use crate::compute::sphere_build::{indirect_seed, SphereBuildParams, SphereBuildPipeline};
use crate::compute::sphere_lod_count::SphereLodCountPipeline;
use crate::memory::{buffer_usage, GpuMemoryUsage};
use crate::picking::RepKind;
use crate::pipelines::sphere::{SphereParams, SphereParamsLayout};
use crate::render_input::{RenderObjectInput, SceneLod};
use crate::representation_budget::{RepBuildDecision, RepMemoryEstimate, RepQualityLevel};
use crate::representations::cullable::CullableBuffers;
use crate::representations::viewport_lod::ViewportLodReadback;
use crate::representations::{BuildCtx, CullPlan, CullPlanCtx, Representation, ViewportLodCtx};

/// 32-byte instance entry. Layout mirrors the compute kernel's
/// `SphereInstance` struct — WGSL rounds struct sizes to alignment, and
/// the embedded `vec3<f32>` forces 16 B alignment, so the storage layout
/// is 32 B. The vertex stage reads only the first 20 B (center, radius,
/// group_id); the trailing 12 B are pad.
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct SphereInstance {
    pub center: [f32; 3],
    pub radius: f32,
    pub group_id: u32,
    pub _pad0: u32,
    pub _pad1: u32,
    pub _pad2: u32,
}

impl SphereInstance {
    pub const SIZE: u64 = std::mem::size_of::<Self>() as u64;

    pub fn vertex_layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: Self::SIZE,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 0,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32,
                    offset: 12,
                    shader_location: 1,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Uint32,
                    offset: 16,
                    shader_location: 2,
                },
            ],
        }
    }
}

/// Per-instance upper bound assumed by the compute build kernel. Each
/// dispatch can emit at most `atom_count_for_object` instances; we
/// allocate exactly that, padded to power of two.
fn instance_capacity_for(atom_count: u32) -> u64 {
    let needed = (atom_count as u64).max(1) * SphereInstance::SIZE;
    needed.next_power_of_two().max(64 * SphereInstance::SIZE)
}

/// Target visible sphere count for 3J3Q-class automatic LOD.
///
/// This keeps the common `cartoon,sticks,spheres` stress preset interactive
/// without changing small-scene sphere fidelity. The value intentionally
/// targets a power-of-two stride near 8 for 3J3Q's 2.44M atoms.
const SPHERE_LOD_TARGET_INSTANCES: u32 = 350_000;
/// Hysteresis exit for viewport full-detail mode.
///
/// The renderer switches sampled 3J3Q-class spheres back to full detail when
/// the measured viewport count is at or below `SPHERE_LOD_TARGET_INSTANCES`.
/// It waits until the measured count is 20% above that target before
/// returning to sampling so small zoom/pan changes do not thrash GPU rebuilds.
const SPHERE_LOD_VIEWPORT_EXIT_INSTANCES: u32 =
    SPHERE_LOD_TARGET_INSTANCES + SPHERE_LOD_TARGET_INSTANCES / 5;
const MAX_SPHERE_LOD_SAMPLE_SHIFT: u32 = 31;

fn sphere_lod_sample_shift(atom_count: u32, lod: SceneLod) -> u32 {
    if lod != SceneLod::Minimum || atom_count <= SPHERE_LOD_TARGET_INSTANCES {
        return 0;
    }
    let mut shift = 0u32;
    let mut stride = 1u32;
    while atom_count.div_ceil(stride) > SPHERE_LOD_TARGET_INSTANCES
        && shift < MAX_SPHERE_LOD_SAMPLE_SHIFT
    {
        shift += 1;
        stride <<= 1;
    }
    shift
}

fn sampled_sphere_upper_bound(atom_count: u32, sample_shift: u32) -> u32 {
    if sample_shift == 0 {
        return atom_count;
    }
    atom_count.div_ceil(1u32 << sample_shift)
}

fn sphere_lod_thinning_active(sample_shift: u32) -> bool {
    sample_shift > 0
}

fn sphere_cull_upper_bound(atom_count: u32, sample_shift: u32) -> u32 {
    if sphere_lod_thinning_active(sample_shift) {
        sampled_sphere_upper_bound(atom_count, sample_shift)
    } else {
        atom_count
    }
}

fn sphere_lod_active_sample_shift(base_sample_shift: u32, viewport_full_detail: bool) -> u32 {
    if viewport_full_detail {
        0
    } else {
        base_sample_shift
    }
}

fn sphere_lod_viewport_full_detail(visible_count: u32, currently_full_detail: bool) -> bool {
    if visible_count == 0 {
        return false;
    }
    if currently_full_detail {
        visible_count <= SPHERE_LOD_VIEWPORT_EXIT_INSTANCES
    } else {
        visible_count <= SPHERE_LOD_TARGET_INSTANCES
    }
}

fn can_reuse_sphere_instances(dirty: DirtyFlags) -> bool {
    dirty.is_lut_only()
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SphereLodDiagnostics {
    pub active: bool,
    pub sample_shift: u32,
    pub sample_stride: u32,
    pub base_sample_shift: u32,
    pub source_atom_count: u64,
    pub instance_upper_bound: u64,
    pub cull_upper_bound: u64,
    pub viewport_visible_count: u64,
    pub viewport_full_detail: bool,
}

impl SphereLodDiagnostics {
    pub(crate) fn include(&mut self, other: SphereLodDiagnostics) {
        self.active |= other.active;
        self.sample_shift = self.sample_shift.max(other.sample_shift);
        self.sample_stride = 1u32 << self.sample_shift;
        self.base_sample_shift = self.base_sample_shift.max(other.base_sample_shift);
        self.source_atom_count = self
            .source_atom_count
            .saturating_add(other.source_atom_count);
        self.instance_upper_bound = self
            .instance_upper_bound
            .saturating_add(other.instance_upper_bound);
        self.cull_upper_bound = self.cull_upper_bound.saturating_add(other.cull_upper_bound);
        self.viewport_visible_count = self
            .viewport_visible_count
            .saturating_add(other.viewport_visible_count);
        self.viewport_full_detail |= other.viewport_full_detail;
    }
}

pub struct SphereRep {
    gpu: CullableBuffers,
    /// Per-rep build params (sphere_scale). Group 1 binding 0 of the
    /// build pipeline.
    build_params_buffer: wgpu::Buffer,
    /// Per-rep render params (alpha_mul). Group 3 binding 0 of the
    /// render pipeline.
    render_params_buffer: wgpu::Buffer,
    render_params_bind_group: Option<wgpu::BindGroup>,
    /// Bind group for the build kernel (params + raw_inst + raw_count).
    compute_bind_group: Option<wgpu::BindGroup>,
    /// GPU-written count of real sphere atoms visible in the current viewport.
    viewport_count_buffer: wgpu::Buffer,
    viewport_count_bind_group: Option<wgpu::BindGroup>,

    /// Atom count of the object this rep was last built against. Drives
    /// dispatch size and compute-buffer capacity. `None` until the first
    /// build.
    last_atom_count: Option<u32>,
    sphere_scale: f32,
    base_sample_shift: u32,
    sample_shift: u32,
    sampled_upper_bound: u32,
    cull_upper_bound: u32,
    budget_sample_shift: Option<u32>,
    viewport_visible_count: u32,
    viewport_full_detail: bool,
    viewport_lod_readback: ViewportLodReadback,
    /// Whether the next render frame should run the compute build.
    /// `false` ⇒ instance buffer is up to date.
    needs_dispatch: bool,
    /// Cached opacity flag for `is_opaque`. Pulled from
    /// `settings.sphere.transparency` (and per-atom overrides resolved on
    /// the GPU side; CPU side conservatively flips to translucent if the
    /// global multiplier or any per-atom override is sub-unit).
    is_opaque_cache: bool,
}

impl SphereRep {
    pub fn new(device: &wgpu::Device) -> Self {
        let build_params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("patinae.sphere.build_params"),
            size: SphereBuildParams::SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let render_params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("patinae.sphere.render_params"),
            size: SphereParams::SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let viewport_count_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("patinae.sphere.viewport_count"),
            size: 4,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        Self {
            gpu: CullableBuffers::new(device, "sphere", SphereInstance::SIZE, true),
            build_params_buffer,
            render_params_buffer,
            render_params_bind_group: None,
            compute_bind_group: None,
            viewport_count_buffer,
            viewport_count_bind_group: None,
            last_atom_count: None,
            sphere_scale: 1.0,
            base_sample_shift: 0,
            sample_shift: 0,
            sampled_upper_bound: 0,
            cull_upper_bound: 0,
            budget_sample_shift: None,
            viewport_visible_count: 0,
            viewport_full_detail: false,
            viewport_lod_readback: ViewportLodReadback::new(
                device,
                "patinae.sphere.viewport_lod_count",
            ),
            needs_dispatch: false,
            is_opaque_cache: true,
        }
    }

    pub fn raw_count_buffer(&self) -> Option<&wgpu::Buffer> {
        self.gpu.raw_count_buffer()
    }

    pub fn cull_bind_group(&self) -> Option<&wgpu::BindGroup> {
        self.gpu.cull_bind_group()
    }

    pub fn cull_params_buffer(&self) -> &wgpu::Buffer {
        self.gpu.cull_params_buffer()
    }

    pub fn instance_capacity_count(&self) -> u32 {
        self.gpu.instance_capacity_count()
    }

    pub(crate) fn export_instances(
        &self,
    ) -> Option<(
        &wgpu::Buffer,
        Option<&wgpu::Buffer>,
        Option<&wgpu::Buffer>,
        u32,
    )> {
        let buffer = self.gpu.compacted_instance_buffer()?;
        Some((
            buffer,
            self.gpu.raw_count_buffer(),
            self.gpu.indirect_buffer(),
            self.gpu.instance_capacity_count(),
        ))
    }

    pub fn last_atom_count(&self) -> Option<u32> {
        self.last_atom_count
    }

    pub fn lod_diagnostics(&self) -> SphereLodDiagnostics {
        let source_atom_count = self.last_atom_count.unwrap_or(0) as u64;
        let sample_stride = 1u32 << self.sample_shift;
        SphereLodDiagnostics {
            active: self.sample_shift > 0,
            sample_shift: self.sample_shift,
            sample_stride,
            base_sample_shift: self.base_sample_shift,
            source_atom_count,
            instance_upper_bound: self.sampled_upper_bound as u64,
            cull_upper_bound: self.cull_upper_bound as u64,
            viewport_visible_count: self.viewport_visible_count as u64,
            viewport_full_detail: self.viewport_full_detail,
        }
    }

    fn refresh_lod_bounds(&mut self, atom_count: u32) {
        self.sampled_upper_bound = sampled_sphere_upper_bound(atom_count, self.sample_shift);
        self.cull_upper_bound = sphere_cull_upper_bound(atom_count, self.sample_shift);
    }

    fn upload_build_params(&self, queue: &wgpu::Queue) {
        queue.write_buffer(
            &self.build_params_buffer,
            0,
            bytemuck::bytes_of(&SphereBuildParams {
                sphere_scale: self.sphere_scale,
                sample_shift: self.sample_shift,
                _pad0: 0,
                _pad1: 0,
            }),
        );
    }

    fn apply_viewport_count(&mut self, visible_count: u32, queue: &wgpu::Queue) -> bool {
        if self.base_sample_shift == 0 {
            self.viewport_visible_count = 0;
            self.viewport_full_detail = false;
            return false;
        }
        let Some(atom_count) = self.last_atom_count else {
            return false;
        };
        let visible_count = visible_count.min(atom_count);
        self.viewport_visible_count = visible_count;
        let full_detail = sphere_lod_viewport_full_detail(visible_count, self.viewport_full_detail);
        if full_detail == self.viewport_full_detail {
            return false;
        }
        self.viewport_full_detail = full_detail;
        let next_shift =
            sphere_lod_active_sample_shift(self.base_sample_shift, self.viewport_full_detail)
                .max(self.budget_sample_shift.unwrap_or(0));
        if next_shift == self.sample_shift {
            return false;
        }
        self.sample_shift = next_shift;
        self.refresh_lod_bounds(atom_count);
        self.upload_build_params(queue);
        self.needs_dispatch = true;
        true
    }

    /// Build / configure the per-rep render-side bind group (group 3).
    /// Idempotent — recreated only when the underlying params buffer
    /// reallocates (which it never does post-construction).
    pub fn ensure_render_bind_group(&mut self, device: &wgpu::Device, layout: &SphereParamsLayout) {
        if self.render_params_bind_group.is_some() {
            return;
        }
        self.render_params_bind_group =
            Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("patinae.sphere.render_bg"),
                layout: &layout.bind_group_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.render_params_buffer.as_entire_binding(),
                }],
            }));
    }

    /// Allocate (or grow) the raw/compacted/raw_count/indirect buffers
    /// and rebuild the build + cull bind groups if any storage buffer
    /// changed.
    fn ensure_buffers(
        &mut self,
        device: &wgpu::Device,
        compute_pipeline: &SphereBuildPipeline,
        count_pipeline: &SphereLodCountPipeline,
        cull_pipeline: Option<&crate::compute::cull::CullPipeline>,
    ) -> bool {
        let needed = instance_capacity_for(self.sampled_upper_bound);
        let instance_grew = self.gpu.ensure(device, needed, cull_pipeline);
        if instance_grew || self.compute_bind_group.is_none() {
            let bg = compute_pipeline.make_bind_group(
                device,
                &self.build_params_buffer,
                self.gpu.raw_instance_buffer().unwrap(),
                self.gpu.raw_count_buffer().unwrap(),
            );
            self.compute_bind_group = Some(bg);
        }
        if self.viewport_count_bind_group.is_none() {
            self.viewport_count_bind_group = Some(count_pipeline.make_bind_group(
                device,
                self.gpu.cull_params_buffer(),
                &self.viewport_count_buffer,
            ));
        }
        instance_grew
    }
}

impl Representation for SphereRep {
    fn kind(&self) -> RepKind {
        RepKind::Sphere
    }

    fn build(
        &mut self,
        input: &RenderObjectInput,
        settings: &ResolvedSettings,
        dirty: DirtyFlags,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) {
        let s = input.object_settings.as_ref().unwrap_or(settings);
        self.sphere_scale = s.sphere.scale;
        let sphere_transparency = s.sphere.transparency.clamp(0.0, 1.0);
        let alpha_mul = 1.0 - sphere_transparency;
        let atom_count = input.molecule.atom_count() as u32;
        let atom_count_changed = self.last_atom_count != Some(atom_count);
        let base_sample_shift = sphere_lod_sample_shift(atom_count, input.lod);
        if atom_count_changed || base_sample_shift == 0 {
            self.viewport_visible_count = 0;
            self.viewport_full_detail = false;
        }
        let sample_shift =
            sphere_lod_active_sample_shift(base_sample_shift, self.viewport_full_detail)
                .max(self.budget_sample_shift.unwrap_or(0));
        let old_sample_shift = self.sample_shift;
        let old_sampled_upper_bound = self.sampled_upper_bound;
        let old_cull_upper_bound = self.cull_upper_bound;
        let old_base_sample_shift = self.base_sample_shift;

        // Upload per-rep params on every build. Tiny writes (16 B + 16 B).
        self.base_sample_shift = base_sample_shift;
        self.sample_shift = sample_shift;
        self.last_atom_count = Some(atom_count);
        self.refresh_lod_bounds(atom_count);
        self.upload_build_params(queue);
        queue.write_buffer(
            &self.render_params_buffer,
            0,
            bytemuck::bytes_of(&SphereParams {
                alpha_mul,
                _pad0: 0.0,
                _pad1: 0.0,
                _pad2: 0.0,
            }),
        );

        let marker_only = dirty
            .difference(DirtyFlags::SELECTION | DirtyFlags::HOVER)
            .is_empty();
        if !marker_only || !self.gpu.has_raw_instances() {
            self.is_opaque_cache = visible_sphere_atoms_are_opaque(input, alpha_mul);
        }

        let lod_changed = old_base_sample_shift != self.base_sample_shift
            || old_sample_shift != self.sample_shift
            || old_sampled_upper_bound != self.sampled_upper_bound
            || old_cull_upper_bound != self.cull_upper_bound;

        // LUT-only fast-path: SceneStore handles colour/visibility writes
        // for us; the compute-built instance buffer doesn't need a
        // re-dispatch when only LUT bits flipped. Selection and hover must
        // remain overlays over the currently materialized sphere geometry.
        if can_reuse_sphere_instances(dirty) && !lod_changed && self.gpu.has_raw_instances() {
            return;
        }

        // Geometry-affecting dirty wave (or first build) — schedule a
        // compute dispatch on the next `record_compute_build` call.
        self.needs_dispatch = true;
    }

    fn is_opaque(&self) -> bool {
        self.is_opaque_cache
    }

    fn poll_viewport_lod(&mut self, queue: &wgpu::Queue) -> bool {
        self.viewport_lod_readback.kick_map_if_needed();
        let Some(culled_instance_count) = self.viewport_lod_readback.collect() else {
            return false;
        };
        self.apply_viewport_count(culled_instance_count, queue)
    }

    fn record_viewport_lod_readback(&mut self, ctx: &mut ViewportLodCtx<'_>) {
        if self.base_sample_shift == 0 {
            return;
        }
        let Some(atom_count) = self.last_atom_count else {
            return;
        };
        if atom_count == 0 {
            return;
        }
        let Some(count_bg) = self.viewport_count_bind_group.as_ref() else {
            return;
        };
        ctx.encoder
            .clear_buffer(&self.viewport_count_buffer, 0, None);
        ctx.pipelines.sphere_lod_count.dispatch(
            ctx.encoder,
            ctx.scene_bg,
            ctx.obj_dynamic_offset,
            count_bg,
            atom_count,
        );
        self.viewport_lod_readback
            .record_count_copy(ctx.encoder, &self.viewport_count_buffer);
    }

    fn memory_usage(&self) -> GpuMemoryUsage {
        let mut usage = self.gpu.memory_usage();
        usage.add(buffer_usage(&self.build_params_buffer));
        usage.add(buffer_usage(&self.render_params_buffer));
        usage.add(buffer_usage(&self.viewport_count_buffer));
        usage.add(self.viewport_lod_readback.memory_usage());
        usage
    }

    fn record_translucent<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        let (Some(instance_buf), Some(indirect_buf), Some(bg)) = (
            self.gpu.compacted_instance_buffer(),
            self.gpu.indirect_buffer(),
            self.render_params_bind_group.as_ref(),
        ) else {
            return;
        };
        pass.set_bind_group(3, bg, &[]);
        pass.set_vertex_buffer(0, instance_buf.slice(..));
        pass.draw_indirect(indirect_buf, 0);
    }

    fn record_picking<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        let (Some(instance_buf), Some(indirect_buf)) = (
            self.gpu.compacted_instance_buffer(),
            self.gpu.indirect_buffer(),
        ) else {
            return;
        };
        pass.set_vertex_buffer(0, instance_buf.slice(..));
        pass.draw_indirect(indirect_buf, 0);
    }

    fn prepare_shadow_depth(&self, encoder: &mut wgpu::CommandEncoder, queue: &wgpu::Queue) {
        let seed = indirect_seed(6);
        self.gpu.prepare_raw_shadow_indirect(encoder, queue, &seed);
    }

    fn record_shadow_depth<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        let (Some(instance_buf), Some(indirect_buf), Some(bg)) = (
            self.gpu.raw_instance_buffer(),
            self.gpu.shadow_indirect_buffer(),
            self.render_params_bind_group.as_ref(),
        ) else {
            return;
        };
        pass.set_bind_group(3, bg, &[]);
        pass.set_vertex_buffer(0, instance_buf.slice(..));
        pass.draw_indirect(indirect_buf, 0);
    }

    fn record_compute_build(&mut self, ctx: &mut BuildCtx<'_>) -> bool {
        self.ensure_render_bind_group(ctx.device, &ctx.pipelines.sphere_params_layout);
        if !self.needs_dispatch {
            return false;
        }
        let atom_count = match self.last_atom_count {
            Some(n) if n > 0 => n,
            _ => {
                self.needs_dispatch = false;
                return false;
            }
        };
        let compute_pipeline = &ctx.pipelines.sphere_compute;
        let count_pipeline = &ctx.pipelines.sphere_lod_count;
        let cull_pipeline = &ctx.pipelines.cull_pipeline;
        self.ensure_buffers(
            ctx.device,
            compute_pipeline,
            count_pipeline,
            Some(cull_pipeline),
        );
        // Reset the raw atomic counter; build will atomicAdd into it.
        self.gpu.reset_raw_count(ctx.queue);
        let bg = match self.compute_bind_group.as_ref() {
            Some(b) => b,
            None => {
                self.needs_dispatch = false;
                return false;
            }
        };
        compute_pipeline.dispatch(
            ctx.encoder,
            ctx.scene_bg,
            ctx.obj_dynamic_offset,
            bg,
            atom_count,
        );
        self.needs_dispatch = false;
        true
    }

    fn plan_cull(&mut self, ctx: &CullPlanCtx<'_>) -> Option<CullPlan<'_>> {
        let upper = self.cull_upper_bound;
        let seed = indirect_seed(6);
        self.gpu.plan_cull(ctx, upper, self.sphere_scale, &seed)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn apply_budget_decision(
        &mut self,
        decision: RepBuildDecision,
        quality: RepQualityLevel,
    ) -> bool {
        let budget_sample_shift = match (decision, quality) {
            (RepBuildDecision::Downgrade, RepQualityLevel::Sampled { sample_shift }) => {
                Some(sample_shift)
            }
            _ => None,
        };
        let changed = self.budget_sample_shift != budget_sample_shift;
        self.budget_sample_shift = budget_sample_shift;
        changed
    }
}

pub(crate) fn budget_estimates(input: &RenderObjectInput<'_>) -> Vec<RepMemoryEstimate> {
    let atom_count = input.molecule.atom_count() as u32;
    let mut estimates = Vec::with_capacity(2);
    estimates.push(sphere_estimate(atom_count, RepQualityLevel::Full));

    let budget_shift = sphere_lod_sample_shift(atom_count, SceneLod::Minimum);
    if budget_shift > 0 {
        estimates.push(sphere_estimate(
            sampled_sphere_upper_bound(atom_count, budget_shift),
            RepQualityLevel::Sampled {
                sample_shift: budget_shift,
            },
        ));
    }

    estimates
}

fn sphere_estimate(instance_count: u32, quality: RepQualityLevel) -> RepMemoryEstimate {
    let instance_capacity_bytes = instance_capacity_for(instance_count);
    let capacity_bytes = CullableBuffers::estimated_memory(instance_capacity_bytes, true)
        .saturating_add(SphereBuildParams::SIZE)
        .saturating_add(SphereParams::SIZE)
        .saturating_add(4);
    RepMemoryEstimate {
        required_bytes: instance_capacity_bytes,
        scratch_bytes: 0,
        capacity_bytes,
        quality,
        can_chunk: false,
        can_skip: true,
    }
}

fn visible_sphere_atoms_are_opaque(input: &RenderObjectInput<'_>, alpha_mul: f32) -> bool {
    if alpha_mul < 0.999 {
        return false;
    }
    input.molecule.atoms().all(|atom| {
        if !atom.repr.visible_reps.is_visible(RepMask::SPHERES) {
            return true;
        }
        atom.repr
            .sphere_transparency
            .map(|t| (1.0 - t.clamp(0.0, 1.0)) >= 0.999)
            .unwrap_or(true)
    })
}

#[cfg(test)]
mod tests {
    use lin_alg::f32::Vec3;
    use patinae_mol::{AtomBuilder, DirtyFlags, MoleculeBuilder, RepMask};

    use super::*;
    use crate::{ObjectId, SceneLod};

    fn input_for<'a>(
        molecule: &'a patinae_mol::ObjectMolecule,
        coord_set: &'a patinae_mol::CoordSet,
    ) -> RenderObjectInput<'a> {
        RenderObjectInput {
            object_id: ObjectId(1),
            molecule,
            coord_set,
            transform: crate::render_input::IDENTITY_TRANSFORM,
            visible_reps: RepMask::SPHERES,
            draw_reps: RepMask::SPHERES,
            object_settings: None,
            atom_colors: &[],
            atom_rep_colors: &[],
            atom_markers: &[],
            marker_updates: &[],
            has_markers: false,
            lod: SceneLod::Auto,
            dirty: DirtyFlags::ALL,
        }
    }

    #[test]
    fn sphere_lod_sample_shift_thresholds() {
        let huge = 2_440_800;
        assert_eq!(sphere_lod_sample_shift(huge, SceneLod::Auto), 0);
        assert_eq!(sphere_lod_sample_shift(huge, SceneLod::High), 0);
        assert_eq!(sphere_lod_sample_shift(huge, SceneLod::Medium), 0);
        assert_eq!(sphere_lod_sample_shift(huge, SceneLod::Low), 0);
        assert_eq!(
            sphere_lod_sample_shift(SPHERE_LOD_TARGET_INSTANCES, SceneLod::Minimum),
            0
        );
        assert_eq!(
            sphere_lod_sample_shift(SPHERE_LOD_TARGET_INSTANCES + 1, SceneLod::Minimum),
            1
        );
        assert_eq!(sphere_lod_sample_shift(huge, SceneLod::Minimum), 3);
        assert_eq!(sampled_sphere_upper_bound(huge, 3), 305_100);
    }

    #[test]
    fn sphere_lod_cull_bound_matches_sampled_instances() {
        let huge = 2_440_800;
        assert_eq!(sphere_cull_upper_bound(huge, 3), 305_100);
        assert_eq!(sphere_cull_upper_bound(504, 0), 504);
    }

    #[test]
    fn sphere_lod_viewport_full_detail_uses_hysteresis() {
        assert!(!sphere_lod_viewport_full_detail(0, false));
        assert!(!sphere_lod_viewport_full_detail(0, true));
        assert!(sphere_lod_viewport_full_detail(
            SPHERE_LOD_TARGET_INSTANCES,
            false
        ));
        assert!(!sphere_lod_viewport_full_detail(
            SPHERE_LOD_TARGET_INSTANCES + 1,
            false
        ));
        assert!(sphere_lod_viewport_full_detail(
            SPHERE_LOD_VIEWPORT_EXIT_INSTANCES,
            true
        ));
        assert!(!sphere_lod_viewport_full_detail(
            SPHERE_LOD_VIEWPORT_EXIT_INSTANCES + 1,
            true
        ));
        assert_eq!(sphere_lod_active_sample_shift(3, false), 3);
        assert_eq!(sphere_lod_active_sample_shift(3, true), 0);
    }

    #[test]
    fn marker_dirty_keeps_sphere_instances_lut_only() {
        assert!(can_reuse_sphere_instances(DirtyFlags::HOVER));
        assert!(can_reuse_sphere_instances(DirtyFlags::SELECTION));
        assert!(can_reuse_sphere_instances(
            DirtyFlags::SELECTION | DirtyFlags::HOVER
        ));
        assert!(can_reuse_sphere_instances(DirtyFlags::COLOR));
        assert!(!can_reuse_sphere_instances(DirtyFlags::COORDS));
    }

    #[test]
    fn hidden_sphere_transparency_does_not_force_visible_spheres_to_wboit() {
        let mut ion = AtomBuilder::new().name("FE").element_symbol("FE").build();
        ion.repr.visible_reps = RepMask::SPHERES;

        let mut hidden = AtomBuilder::new().name("C").element_symbol("C").build();
        hidden.repr.visible_reps = RepMask::STICKS;
        hidden.repr.sphere_transparency = Some(0.75);

        let mol = MoleculeBuilder::new("ions")
            .add_atom(ion, Vec3::new(0.0, 0.0, 0.0))
            .add_atom(hidden, Vec3::new(4.0, 0.0, 0.0))
            .build();
        let coord = mol.current_coord_set().expect("coord set");
        let input = input_for(&mol, coord);

        assert!(visible_sphere_atoms_are_opaque(&input, 1.0));
    }

    #[test]
    fn visible_sphere_transparency_forces_wboit() {
        let mut ion = AtomBuilder::new().name("FE").element_symbol("FE").build();
        ion.repr.visible_reps = RepMask::SPHERES;
        ion.repr.sphere_transparency = Some(0.25);

        let mol = MoleculeBuilder::new("ions")
            .add_atom(ion, Vec3::new(0.0, 0.0, 0.0))
            .build();
        let coord = mol.current_coord_set().expect("coord set");
        let input = input_for(&mol, coord);

        assert!(!visible_sphere_atoms_are_opaque(&input, 1.0));
    }
}
