//! Stick (capsule) representation. Migrated to `SceneStore` (group 2) +
//! GPU compute build. Per-bond CPU iteration is gone; the
//! [`crate::compute::stick_build`] kernel reads scene-wide bonds + atoms +
//! coords and emits compacted [`StickInstance`]s, with
//! the visible-instance count written atomically into the indirect-draw
//! args buffer. Multi-bonds emit one offset capsule per slot.

use bytemuck::{Pod, Zeroable};
use patinae_mol::DirtyFlags;
use patinae_settings::ResolvedSettings;

use crate::compute::stick_build::{indirect_seed, StickBuildParams, StickBuildPipeline};
use crate::compute::stick_lod_count::StickLodCountPipeline;
use crate::memory::{buffer_usage, GpuMemoryUsage};
use crate::picking::RepKind;
use crate::pipelines::stick::{StickParams, StickParamsLayout};
use crate::render_input::{RenderObjectInput, SceneLod};
use crate::representation_budget::{RepBuildDecision, RepMemoryEstimate, RepQualityLevel};
use crate::representations::cullable::CullableBuffers;
use crate::representations::viewport_lod::ViewportLodReadback;
use crate::representations::{BuildCtx, CullPlan, CullPlanCtx, Representation, ViewportLodCtx};

/// 48-byte instance entry. Layout matches `StickInstance` in
/// `shaders/representations/stick.wgsl`.
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct StickInstance {
    pub p0_radius: [f32; 4],
    pub p1_pad: [f32; 4],
    pub groups: [u32; 2],
    pub _pad: [u32; 2],
}

impl StickInstance {
    pub const SIZE: u64 = std::mem::size_of::<Self>() as u64;

    pub fn vertex_layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: Self::SIZE,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 0,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 16,
                    shader_location: 1,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Uint32x2,
                    offset: 32,
                    shader_location: 2,
                },
            ],
        }
    }
}

/// Multi-bonds emit up to 3 instances per bond (`Triple`). Allocate this
/// upper bound × bond count, padded to power of two.
fn instance_capacity_for(bond_count: u32) -> u64 {
    instance_capacity_for_instances(bond_count.saturating_mul(3))
}

fn instance_capacity_for_instances(instance_count: u32) -> u64 {
    let needed = (instance_count as u64).max(1) * StickInstance::SIZE;
    needed.next_power_of_two().max(64 * StickInstance::SIZE)
}

/// Target visible source-bond count for 3J3Q-class automatic stick LOD.
const STICK_LOD_TARGET_BONDS: u32 = 350_000;
/// Hysteresis exit for viewport full-detail mode.
const STICK_LOD_VIEWPORT_EXIT_BONDS: u32 = STICK_LOD_TARGET_BONDS + STICK_LOD_TARGET_BONDS / 5;
const MAX_STICK_LOD_SAMPLE_SHIFT: u32 = 31;

fn stick_lod_sample_shift(bond_count: u32, lod: SceneLod) -> u32 {
    if lod != SceneLod::Minimum || bond_count <= STICK_LOD_TARGET_BONDS {
        return 0;
    }
    let mut shift = 0u32;
    let mut stride = 1u32;
    while bond_count.div_ceil(stride) > STICK_LOD_TARGET_BONDS && shift < MAX_STICK_LOD_SAMPLE_SHIFT
    {
        shift += 1;
        stride <<= 1;
    }
    shift
}

fn sampled_stick_bond_upper_bound(bond_count: u32, sample_shift: u32) -> u32 {
    if sample_shift == 0 {
        return bond_count;
    }
    bond_count.div_ceil(1u32 << sample_shift)
}

fn stick_lod_thinning_active(sample_shift: u32) -> bool {
    sample_shift > 0
}

fn stick_max_slots_per_bond(valence_enabled: bool) -> u32 {
    if valence_enabled {
        3
    } else {
        1
    }
}

fn stick_cull_upper_bound(bond_count: u32, sample_shift: u32, max_slots_per_bond: u32) -> u32 {
    let source_bonds = if stick_lod_thinning_active(sample_shift) {
        sampled_stick_bond_upper_bound(bond_count, sample_shift)
    } else {
        bond_count
    };
    source_bonds.saturating_mul(max_slots_per_bond)
}

fn stick_lod_active_sample_shift(base_sample_shift: u32, viewport_full_detail: bool) -> u32 {
    if viewport_full_detail {
        0
    } else {
        base_sample_shift
    }
}

fn stick_lod_viewport_full_detail(visible_count: u32, currently_full_detail: bool) -> bool {
    if visible_count == 0 {
        return false;
    }
    if currently_full_detail {
        visible_count <= STICK_LOD_VIEWPORT_EXIT_BONDS
    } else {
        visible_count <= STICK_LOD_TARGET_BONDS
    }
}

fn stick_viewport_radius_pad(radius: f32, valence_scale: f32, valence_enabled: bool) -> f32 {
    if valence_enabled {
        radius + valence_scale.abs()
    } else {
        radius
    }
}

fn can_reuse_stick_instances(dirty: DirtyFlags) -> bool {
    dirty.is_lut_only()
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StickLodDiagnostics {
    pub active: bool,
    pub sample_shift: u32,
    pub sample_stride: u32,
    pub base_sample_shift: u32,
    pub source_bond_count: u64,
    pub sampled_bond_upper_bound: u64,
    pub cull_upper_bound: u64,
    pub viewport_visible_count: u64,
    pub viewport_full_detail: bool,
}

impl StickLodDiagnostics {
    pub(crate) fn include(&mut self, other: StickLodDiagnostics) {
        self.active |= other.active;
        self.sample_shift = self.sample_shift.max(other.sample_shift);
        self.sample_stride = 1u32 << self.sample_shift;
        self.base_sample_shift = self.base_sample_shift.max(other.base_sample_shift);
        self.source_bond_count = self
            .source_bond_count
            .saturating_add(other.source_bond_count);
        self.sampled_bond_upper_bound = self
            .sampled_bond_upper_bound
            .saturating_add(other.sampled_bond_upper_bound);
        self.cull_upper_bound = self.cull_upper_bound.saturating_add(other.cull_upper_bound);
        self.viewport_visible_count = self
            .viewport_visible_count
            .saturating_add(other.viewport_visible_count);
        self.viewport_full_detail |= other.viewport_full_detail;
    }
}

pub struct StickRep {
    gpu: CullableBuffers,
    build_params_buffer: wgpu::Buffer,
    render_params_buffer: wgpu::Buffer,
    render_params_bind_group: Option<wgpu::BindGroup>,
    compute_bind_group: Option<wgpu::BindGroup>,
    viewport_count_buffer: wgpu::Buffer,
    viewport_count_bind_group: Option<wgpu::BindGroup>,

    last_bond_count: Option<u32>,
    base_sample_shift: u32,
    sample_shift: u32,
    sampled_bond_upper_bound: u32,
    cull_upper_bound: u32,
    budget_sample_shift: Option<u32>,
    budget_capacity_count: Option<u32>,
    max_slots_per_bond: u32,
    viewport_radius_pad: f32,
    viewport_visible_count: u32,
    viewport_full_detail: bool,
    viewport_lod_readback: ViewportLodReadback,
    needs_dispatch: bool,
    is_opaque_cache: bool,
}

impl StickRep {
    pub fn new(device: &wgpu::Device) -> Self {
        let build_params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("patinae.stick.build_params"),
            size: StickBuildParams::SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let render_params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("patinae.stick.render_params"),
            size: StickParams::SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let viewport_count_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("patinae.stick.viewport_count"),
            size: 4,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        Self {
            gpu: CullableBuffers::new(device, "stick", StickInstance::SIZE, true),
            build_params_buffer,
            render_params_buffer,
            render_params_bind_group: None,
            compute_bind_group: None,
            viewport_count_buffer,
            viewport_count_bind_group: None,
            last_bond_count: None,
            base_sample_shift: 0,
            sample_shift: 0,
            sampled_bond_upper_bound: 0,
            cull_upper_bound: 0,
            budget_sample_shift: None,
            budget_capacity_count: None,
            max_slots_per_bond: 3,
            viewport_radius_pad: 0.0,
            viewport_visible_count: 0,
            viewport_full_detail: false,
            viewport_lod_readback: ViewportLodReadback::new(
                device,
                "patinae.stick.viewport_lod_count",
            ),
            needs_dispatch: false,
            is_opaque_cache: true,
        }
    }

    pub fn cull_bind_group(&self) -> Option<&wgpu::BindGroup> {
        self.gpu.cull_bind_group()
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

    /// Upper bound on raw instances written by the build kernel. Used by
    /// `dispatch_cull` to size the cull dispatch.
    pub fn cull_upper_bound(&self) -> Option<u32> {
        Some(self.cull_upper_bound)
    }

    pub fn lod_diagnostics(&self) -> StickLodDiagnostics {
        let source_bond_count = self.last_bond_count.unwrap_or(0) as u64;
        let sample_stride = 1u32 << self.sample_shift;
        StickLodDiagnostics {
            active: self.sample_shift > 0,
            sample_shift: self.sample_shift,
            sample_stride,
            base_sample_shift: self.base_sample_shift,
            source_bond_count,
            sampled_bond_upper_bound: self.sampled_bond_upper_bound as u64,
            cull_upper_bound: self.cull_upper_bound as u64,
            viewport_visible_count: self.viewport_visible_count as u64,
            viewport_full_detail: self.viewport_full_detail,
        }
    }

    fn refresh_lod_bounds(&mut self, bond_count: u32) {
        self.sampled_bond_upper_bound =
            sampled_stick_bond_upper_bound(bond_count, self.sample_shift);
        self.cull_upper_bound =
            stick_cull_upper_bound(bond_count, self.sample_shift, self.max_slots_per_bond);
    }

    fn upload_build_params(&self, queue: &wgpu::Queue, radius: f32, valence_scale: f32) {
        queue.write_buffer(
            &self.build_params_buffer,
            0,
            bytemuck::bytes_of(&StickBuildParams {
                radius,
                valence_scale,
                valence_enabled: if self.max_slots_per_bond > 1 { 1 } else { 0 },
                sample_shift: self.sample_shift,
            }),
        );
    }

    fn apply_viewport_count(&mut self, visible_count: u32, queue: &wgpu::Queue) -> bool {
        if self.base_sample_shift == 0 {
            self.viewport_visible_count = 0;
            self.viewport_full_detail = false;
            return false;
        }
        let Some(bond_count) = self.last_bond_count else {
            return false;
        };
        let visible_count = visible_count.min(bond_count);
        self.viewport_visible_count = visible_count;
        let full_detail = stick_lod_viewport_full_detail(visible_count, self.viewport_full_detail);
        if full_detail == self.viewport_full_detail {
            return false;
        }
        self.viewport_full_detail = full_detail;
        let next_shift =
            stick_lod_active_sample_shift(self.base_sample_shift, self.viewport_full_detail)
                .max(self.budget_sample_shift.unwrap_or(0));
        if next_shift == self.sample_shift {
            return false;
        }
        self.sample_shift = next_shift;
        self.refresh_lod_bounds(bond_count);
        if self.budget_sample_shift.is_some() {
            self.budget_capacity_count = Some(self.cull_upper_bound);
        }
        queue.write_buffer(
            &self.build_params_buffer,
            12,
            bytemuck::bytes_of(&self.sample_shift),
        );
        self.needs_dispatch = true;
        true
    }

    pub fn ensure_render_bind_group(&mut self, device: &wgpu::Device, layout: &StickParamsLayout) {
        if self.render_params_bind_group.is_some() {
            return;
        }
        self.render_params_bind_group =
            Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("patinae.stick.render_bg"),
                layout: &layout.bind_group_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.render_params_buffer.as_entire_binding(),
                }],
            }));
    }

    fn ensure_buffers(
        &mut self,
        device: &wgpu::Device,
        bond_count: u32,
        compute_pipeline: &StickBuildPipeline,
        count_pipeline: &StickLodCountPipeline,
        cull_pipeline: Option<&crate::compute::cull::CullPipeline>,
    ) {
        let needed = self
            .budget_capacity_count
            .map(instance_capacity_for_instances)
            .unwrap_or_else(|| instance_capacity_for(bond_count));
        let grew = self.gpu.ensure(device, needed, cull_pipeline);
        if grew || self.compute_bind_group.is_none() {
            self.compute_bind_group = Some(compute_pipeline.make_bind_group(
                device,
                &self.build_params_buffer,
                self.gpu.raw_instance_buffer().unwrap(),
                self.gpu.raw_count_buffer().unwrap(),
            ));
        }
        if self.viewport_count_bind_group.is_none() {
            self.viewport_count_bind_group = Some(count_pipeline.make_bind_group(
                device,
                self.gpu.cull_params_buffer(),
                &self.viewport_count_buffer,
            ));
        }
    }
}

impl Representation for StickRep {
    fn kind(&self) -> RepKind {
        RepKind::Stick
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
        let radius = s.stick.radius;
        let valence_enabled = s.stick.valence;
        // Per-bond perpendicular offset for the split-cylinder valence join.
        let valence_scale_factor = radius * 1.5 * s.stick.valence_scale;
        let stick_transparency = s.stick.transparency.clamp(0.0, 1.0);
        let alpha_mul = 1.0 - stick_transparency;
        let bond_count = input.molecule.bonds().count() as u32;
        let bond_count_changed = self.last_bond_count != Some(bond_count);
        let max_slots_per_bond = stick_max_slots_per_bond(valence_enabled);
        let base_sample_shift = stick_lod_sample_shift(bond_count, input.lod);
        if bond_count_changed || base_sample_shift == 0 {
            self.viewport_visible_count = 0;
            self.viewport_full_detail = false;
        }
        let sample_shift =
            stick_lod_active_sample_shift(base_sample_shift, self.viewport_full_detail)
                .max(self.budget_sample_shift.unwrap_or(0));
        let old_base_sample_shift = self.base_sample_shift;
        let old_sample_shift = self.sample_shift;
        let old_sampled_bond_upper_bound = self.sampled_bond_upper_bound;
        let old_cull_upper_bound = self.cull_upper_bound;
        let old_max_slots_per_bond = self.max_slots_per_bond;

        self.base_sample_shift = base_sample_shift;
        self.sample_shift = sample_shift;
        self.last_bond_count = Some(bond_count);
        self.max_slots_per_bond = max_slots_per_bond;
        self.viewport_radius_pad =
            stick_viewport_radius_pad(radius, valence_scale_factor, valence_enabled);
        self.refresh_lod_bounds(bond_count);
        if self.budget_sample_shift.is_some() {
            self.budget_capacity_count = Some(self.cull_upper_bound);
        }
        self.upload_build_params(queue, radius, valence_scale_factor);
        queue.write_buffer(
            &self.render_params_buffer,
            0,
            bytemuck::bytes_of(&StickParams {
                alpha_mul,
                _pad0: 0.0,
                _pad1: 0.0,
                _pad2: 0.0,
            }),
        );

        let mut opaque = alpha_mul >= 0.999;
        if opaque {
            for atom in input.molecule.atoms() {
                if let Some(t) = atom.repr.stick_transparency {
                    if (1.0 - t.clamp(0.0, 1.0)) < 0.999 {
                        opaque = false;
                        break;
                    }
                }
            }
        }
        self.is_opaque_cache = opaque;

        let lod_changed = old_base_sample_shift != self.base_sample_shift
            || old_sample_shift != self.sample_shift
            || old_sampled_bond_upper_bound != self.sampled_bond_upper_bound
            || old_cull_upper_bound != self.cull_upper_bound
            || old_max_slots_per_bond != self.max_slots_per_bond;

        if can_reuse_stick_instances(dirty) && !lod_changed && self.gpu.has_raw_instances() {
            return;
        }
        self.needs_dispatch = true;
    }

    fn is_opaque(&self) -> bool {
        self.is_opaque_cache
    }

    fn poll_viewport_lod(&mut self, queue: &wgpu::Queue) -> bool {
        self.viewport_lod_readback.kick_map_if_needed();
        let Some(culled_source_bonds) = self.viewport_lod_readback.collect() else {
            return false;
        };
        self.apply_viewport_count(culled_source_bonds, queue)
    }

    fn record_viewport_lod_readback(&mut self, ctx: &mut ViewportLodCtx<'_>) {
        if self.base_sample_shift == 0 {
            return;
        }
        let Some(bond_count) = self.last_bond_count else {
            return;
        };
        if bond_count == 0 {
            return;
        }
        let Some(count_bg) = self.viewport_count_bind_group.as_ref() else {
            return;
        };
        ctx.encoder
            .clear_buffer(&self.viewport_count_buffer, 0, None);
        ctx.pipelines.stick_lod_count.dispatch(
            ctx.encoder,
            ctx.scene_bg,
            ctx.obj_dynamic_offset,
            count_bg,
            bond_count,
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

    fn record_raw_picking<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        let (Some(instance_buf), Some(indirect_buf)) = (
            self.gpu.raw_instance_buffer(),
            self.gpu.shadow_indirect_buffer(),
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
        self.ensure_render_bind_group(ctx.device, &ctx.pipelines.stick_params_layout);
        if !self.needs_dispatch {
            return false;
        }
        let bond_count = match self.last_bond_count {
            Some(n) if n > 0 => n,
            _ => {
                self.needs_dispatch = false;
                return false;
            }
        };
        let compute_pipeline = &ctx.pipelines.stick_compute;
        let count_pipeline = &ctx.pipelines.stick_lod_count;
        let cull_pipeline = &ctx.pipelines.cull_pipeline;
        self.ensure_buffers(
            ctx.device,
            bond_count,
            compute_pipeline,
            count_pipeline,
            Some(cull_pipeline),
        );
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
            bond_count,
        );
        self.needs_dispatch = false;
        true
    }

    fn plan_cull(&mut self, ctx: &CullPlanCtx<'_>) -> Option<CullPlan<'_>> {
        let upper = self.cull_upper_bound()?;
        let seed = indirect_seed(6);
        self.gpu
            .plan_cull(ctx, upper, self.viewport_radius_pad, &seed)
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
        let next_budget_sample_shift = match (decision, quality) {
            (RepBuildDecision::Downgrade, RepQualityLevel::Sampled { sample_shift }) => {
                Some(sample_shift)
            }
            _ => None,
        };
        let changed = self.budget_sample_shift != next_budget_sample_shift;
        self.budget_sample_shift = next_budget_sample_shift;
        if changed {
            self.budget_capacity_count = None;
        }
        changed
    }
}

pub(crate) fn budget_estimates(
    input: &RenderObjectInput<'_>,
    settings: &ResolvedSettings,
) -> Vec<RepMemoryEstimate> {
    let s = input.object_settings.as_ref().unwrap_or(settings);
    let bond_count = input.molecule.bonds().count() as u32;
    let mut estimates = Vec::with_capacity(2);
    estimates.push(stick_estimate(
        bond_count.saturating_mul(3),
        RepQualityLevel::Full,
    ));

    let budget_shift = stick_lod_sample_shift(bond_count, SceneLod::Minimum);
    if budget_shift > 0 {
        let sampled_bonds = sampled_stick_bond_upper_bound(bond_count, budget_shift);
        let capacity_count =
            sampled_bonds.saturating_mul(stick_max_slots_per_bond(s.stick.valence));
        estimates.push(stick_estimate(
            capacity_count,
            RepQualityLevel::Sampled {
                sample_shift: budget_shift,
            },
        ));
    }

    estimates
}

fn stick_estimate(instance_count: u32, quality: RepQualityLevel) -> RepMemoryEstimate {
    let instance_capacity_bytes = instance_capacity_for_instances(instance_count);
    let capacity_bytes = CullableBuffers::estimated_memory(instance_capacity_bytes, true)
        .saturating_add(StickBuildParams::SIZE)
        .saturating_add(StickParams::SIZE)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stick_lod_sample_shift_thresholds() {
        let huge = 2_440_800;
        assert_eq!(stick_lod_sample_shift(huge, SceneLod::Auto), 0);
        assert_eq!(stick_lod_sample_shift(huge, SceneLod::High), 0);
        assert_eq!(stick_lod_sample_shift(huge, SceneLod::Medium), 0);
        assert_eq!(stick_lod_sample_shift(huge, SceneLod::Low), 0);
        assert_eq!(
            stick_lod_sample_shift(STICK_LOD_TARGET_BONDS, SceneLod::Minimum),
            0
        );
        assert_eq!(
            stick_lod_sample_shift(STICK_LOD_TARGET_BONDS + 1, SceneLod::Minimum),
            1
        );
        assert_eq!(stick_lod_sample_shift(huge, SceneLod::Minimum), 3);
        assert_eq!(sampled_stick_bond_upper_bound(huge, 3), 305_100);
    }

    #[test]
    fn stick_lod_cull_bound_respects_valence_slots() {
        let huge = 2_440_800;
        assert_eq!(stick_cull_upper_bound(huge, 3, 1), 305_100);
        assert_eq!(stick_cull_upper_bound(huge, 3, 3), 915_300);
        assert_eq!(stick_cull_upper_bound(504, 0, 1), 504);
        assert_eq!(stick_cull_upper_bound(504, 0, 3), 1_512);
    }

    #[test]
    fn stick_lod_viewport_full_detail_uses_hysteresis() {
        assert!(!stick_lod_viewport_full_detail(0, false));
        assert!(!stick_lod_viewport_full_detail(0, true));
        assert!(stick_lod_viewport_full_detail(
            STICK_LOD_TARGET_BONDS,
            false
        ));
        assert!(!stick_lod_viewport_full_detail(
            STICK_LOD_TARGET_BONDS + 1,
            false
        ));
        assert!(stick_lod_viewport_full_detail(
            STICK_LOD_VIEWPORT_EXIT_BONDS,
            true
        ));
        assert!(!stick_lod_viewport_full_detail(
            STICK_LOD_VIEWPORT_EXIT_BONDS + 1,
            true
        ));
        assert_eq!(stick_lod_active_sample_shift(3, false), 3);
        assert_eq!(stick_lod_active_sample_shift(3, true), 0);
    }

    #[test]
    fn marker_dirty_keeps_stick_instances_lut_only() {
        assert!(can_reuse_stick_instances(DirtyFlags::HOVER));
        assert!(can_reuse_stick_instances(DirtyFlags::SELECTION));
        assert!(can_reuse_stick_instances(
            DirtyFlags::SELECTION | DirtyFlags::HOVER
        ));
        assert!(can_reuse_stick_instances(DirtyFlags::COLOR));
        assert!(!can_reuse_stick_instances(DirtyFlags::COORDS));
    }
}
