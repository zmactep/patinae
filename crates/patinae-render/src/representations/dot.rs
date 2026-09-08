//! Dot representation. Migrated to `SceneStore` (group 2) + GPU compute
//! build. Per-atom CPU iteration is gone; the [`crate::compute::dot_build`]
//! kernel reads scene-wide atoms + coords and emits one compacted
//! [`DotAtomInstance`] per visible DOTS atom. The vertex shader expands each
//! atom into procedural dot-sample billboards from a shared direction LUT.
//!
//! Picking returns the owner atom's index — `RepKind::Dot`,
//! `atom_id = group_id` (object-local).

use bytemuck::{Pod, Zeroable};
use patinae_mol::DirtyFlags;
use patinae_settings::ResolvedSettings;

use crate::compute::dot_build::{indirect_seed, DotBuildParams, DotBuildPipeline};
use crate::memory::{buffer_usage, GpuMemoryUsage};
use crate::picking::RepKind;
use crate::pipelines::dot::{dot_direction_offset, DotDrawParams, DotParamsLayout};
use crate::render_input::{RenderObjectInput, SceneLod};
use crate::representation_budget::{RepMemoryEstimate, RepQualityLevel};
use crate::representations::cullable::CullableBuffers;
use crate::representations::{BuildCtx, CullPlan, CullPlanCtx, DrawPhase, Representation};

// Dot billboards are screen-space, but frustum culling operates in world space.
// Use a small conservative pad around the atom's vdW sphere for near-plane and
// pixel-radius expansion.
const DOT_CULL_WORLD_PAD: f32 = 0.5;

/// 32-byte atom instance entry. Layout matches `DotAtomInstance` in
/// `shaders/representations/dot.wgsl`; each instance is expanded procedurally
/// into `samples_per_atom * 6` vertices.
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct DotAtomInstance {
    pub center: [f32; 3],
    pub group_id: u32,
    pub vdw_radius: f32,
    pub _pad0: u32,
    pub _pad1: u32,
    pub _pad2: u32,
}

impl DotAtomInstance {
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
                    format: wgpu::VertexFormat::Uint32,
                    offset: 12,
                    shader_location: 1,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32,
                    offset: 16,
                    shader_location: 2,
                },
            ],
        }
    }
}

/// Default dots per atom. This middle-ground density stays readable without
/// overloading small scenes.
const DEFAULT_DOTS_PER_ATOM: u32 = 64;
const DOT_QUAD_VERTEX_COUNT: u32 = 6;
const MIN_DOT_BUFFER_BYTES: u64 = 256 * DotAtomInstance::SIZE;
const MAX_DOT_BUFFER_BYTES: u64 = 2_000_000_000;

fn requested_dots_per_atom_from_density(density: i32) -> u32 {
    let density = density.clamp(0, 4) as u32;
    16u32.saturating_mul(1u32 << density)
}

fn effective_dots_per_atom_for_lod(requested: u32, lod: SceneLod) -> u32 {
    match lod {
        SceneLod::Auto | SceneLod::High => requested,
        SceneLod::Medium => requested.min(32),
        SceneLod::Low => requested.min(16),
        SceneLod::Minimum => requested.min(8),
    }
}

fn dot_vertex_count(samples_per_atom: u32) -> u32 {
    samples_per_atom.saturating_mul(DOT_QUAD_VERTEX_COUNT)
}

fn instance_capacity_for(atom_count: u32) -> u64 {
    let raw = ((atom_count as u64).max(1) * DotAtomInstance::SIZE).min(MAX_DOT_BUFFER_BYTES);
    raw.checked_next_power_of_two()
        .unwrap_or(raw)
        .clamp(MIN_DOT_BUFFER_BYTES, MAX_DOT_BUFFER_BYTES)
}

fn can_reuse_dot_atom_instances(dirty: DirtyFlags) -> bool {
    !dirty.intersects(
        DirtyFlags::COORDS | DirtyFlags::REPS | DirtyFlags::VISIBILITY | DirtyFlags::TOPOLOGY,
    )
}

pub struct DotRep {
    gpu: CullableBuffers,
    build_params_buffer: wgpu::Buffer,
    draw_params_buffer: wgpu::Buffer,
    compute_bind_group: Option<wgpu::BindGroup>,
    draw_bind_group: Option<wgpu::BindGroup>,

    last_atom_count: Option<u32>,
    dots_per_atom: u32,
    effective_dots_per_atom: u32,
    needs_dispatch: bool,
}

impl DotRep {
    pub fn new(device: &wgpu::Device) -> Self {
        let build_params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("patinae.dot.build_params"),
            size: DotBuildParams::SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let draw_params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("patinae.dot.draw_params"),
            size: DotDrawParams::SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            gpu: CullableBuffers::new(device, "dot", DotAtomInstance::SIZE, true),
            build_params_buffer,
            draw_params_buffer,
            compute_bind_group: None,
            draw_bind_group: None,
            last_atom_count: None,
            dots_per_atom: DEFAULT_DOTS_PER_ATOM,
            effective_dots_per_atom: DEFAULT_DOTS_PER_ATOM,
            needs_dispatch: false,
        }
    }

    pub fn cull_bind_group(&self) -> Option<&wgpu::BindGroup> {
        self.gpu.cull_bind_group()
    }

    pub fn instance_capacity_count(&self) -> u32 {
        self.gpu.instance_capacity_count()
    }

    pub fn cull_upper_bound(&self) -> Option<u32> {
        self.last_atom_count
            .map(|n| n.min(self.gpu.instance_capacity_count()))
    }

    fn ensure_buffers(
        &mut self,
        device: &wgpu::Device,
        atom_count: u32,
        compute_pipeline: &DotBuildPipeline,
        dot_params_layout: &DotParamsLayout,
        cull_pipeline: Option<&crate::compute::cull::CullPipeline>,
    ) {
        let needed = instance_capacity_for(atom_count);
        let grew = self.gpu.ensure(device, needed, cull_pipeline);
        if grew || self.compute_bind_group.is_none() {
            self.compute_bind_group = Some(compute_pipeline.make_bind_group(
                device,
                &self.build_params_buffer,
                self.gpu.raw_instance_buffer().unwrap(),
                self.gpu.raw_count_buffer().unwrap(),
            ));
        }
        if self.draw_bind_group.is_none() {
            self.draw_bind_group =
                Some(dot_params_layout.make_bind_group(device, &self.draw_params_buffer));
        }
    }
}

impl Representation for DotRep {
    fn kind(&self) -> RepKind {
        RepKind::Dot
    }

    fn is_opaque(&self) -> bool {
        false
    }

    fn draw_phase(&self) -> DrawPhase {
        DrawPhase::FastOverlay
    }

    fn casts_shadow(&self) -> bool {
        false
    }

    fn build(
        &mut self,
        input: &RenderObjectInput,
        settings: &ResolvedSettings,
        dirty: DirtyFlags,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) {
        let atom_count = input.molecule.atom_count() as u32;
        self.last_atom_count = Some(atom_count);
        let s = input.object_settings.as_ref().unwrap_or(settings);
        let requested = requested_dots_per_atom_from_density(s.dot.density);
        if requested != self.dots_per_atom {
            self.dots_per_atom = requested;
        }
        let radius_px = (s.dot.width.max(0.5) * 0.5).max(0.25);
        let eff = effective_dots_per_atom_for_lod(self.dots_per_atom, input.lod);
        if eff != self.effective_dots_per_atom {
            self.effective_dots_per_atom = eff;
        }
        queue.write_buffer(
            &self.draw_params_buffer,
            0,
            bytemuck::bytes_of(&DotDrawParams {
                samples_per_atom: eff,
                dir_offset: dot_direction_offset(eff),
                radius_px,
                _pad0: 0,
            }),
        );
        queue.write_buffer(
            &self.build_params_buffer,
            0,
            bytemuck::bytes_of(&DotBuildParams {
                instance_capacity: self.gpu.instance_capacity_count().max(atom_count),
                _pad0: 0,
                _pad1: 0,
                _pad2: 0,
            }),
        );

        if can_reuse_dot_atom_instances(dirty) && self.gpu.has_raw_instances() {
            return;
        }
        self.needs_dispatch = true;
    }

    fn record_translucent<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        let (Some(instance_buf), Some(indirect_buf), Some(draw_bg)) = (
            self.gpu.compacted_instance_buffer(),
            self.gpu.indirect_buffer(),
            self.draw_bind_group.as_ref(),
        ) else {
            return;
        };
        pass.set_bind_group(3, draw_bg, &[]);
        pass.set_vertex_buffer(0, instance_buf.slice(..));
        pass.draw_indirect(indirect_buf, 0);
    }

    fn record_picking<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        let (Some(instance_buf), Some(indirect_buf), Some(draw_bg)) = (
            self.gpu.compacted_instance_buffer(),
            self.gpu.indirect_buffer(),
            self.draw_bind_group.as_ref(),
        ) else {
            return;
        };
        pass.set_bind_group(3, draw_bg, &[]);
        pass.set_vertex_buffer(0, instance_buf.slice(..));
        pass.draw_indirect(indirect_buf, 0);
    }

    fn record_raw_picking<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        let (Some(instance_buf), Some(indirect_buf), Some(draw_bg)) = (
            self.gpu.raw_instance_buffer(),
            self.gpu.shadow_indirect_buffer(),
            self.draw_bind_group.as_ref(),
        ) else {
            return;
        };
        pass.set_bind_group(3, draw_bg, &[]);
        pass.set_vertex_buffer(0, instance_buf.slice(..));
        pass.draw_indirect(indirect_buf, 0);
    }

    fn record_shadow_depth<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        let (Some(instance_buf), Some(indirect_buf), Some(draw_bg)) = (
            self.gpu.raw_instance_buffer(),
            self.gpu.shadow_indirect_buffer(),
            self.draw_bind_group.as_ref(),
        ) else {
            return;
        };
        pass.set_bind_group(3, draw_bg, &[]);
        pass.set_vertex_buffer(0, instance_buf.slice(..));
        pass.draw_indirect(indirect_buf, 0);
    }

    fn prepare_shadow_depth(&self, encoder: &mut wgpu::CommandEncoder, queue: &wgpu::Queue) {
        let seed = indirect_seed(dot_vertex_count(self.effective_dots_per_atom));
        self.gpu.prepare_raw_shadow_indirect(encoder, queue, &seed);
    }

    fn record_compute_build(&mut self, ctx: &mut BuildCtx<'_>) -> bool {
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
        let compute_pipeline = &ctx.pipelines.dot_compute;
        let dot_params_layout = &ctx.pipelines.dot_params_layout;
        let cull_pipeline = &ctx.pipelines.cull_pipeline;
        self.ensure_buffers(
            ctx.device,
            atom_count,
            compute_pipeline,
            dot_params_layout,
            Some(cull_pipeline),
        );
        ctx.queue.write_buffer(
            &self.build_params_buffer,
            0,
            bytemuck::bytes_of(&DotBuildParams {
                instance_capacity: self.gpu.instance_capacity_count(),
                _pad0: 0,
                _pad1: 0,
                _pad2: 0,
            }),
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
            atom_count,
        );
        self.needs_dispatch = false;
        true
    }

    fn plan_cull(&mut self, ctx: &CullPlanCtx<'_>) -> Option<CullPlan<'_>> {
        let upper = self.cull_upper_bound()?;
        let seed = indirect_seed(dot_vertex_count(self.effective_dots_per_atom));
        self.gpu.plan_cull(ctx, upper, DOT_CULL_WORLD_PAD, &seed)
    }

    fn memory_usage(&self) -> GpuMemoryUsage {
        let mut usage = self.gpu.memory_usage();
        usage.add(buffer_usage(&self.build_params_buffer));
        usage.add(buffer_usage(&self.draw_params_buffer));
        usage
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

pub(crate) fn budget_estimates(input: &RenderObjectInput<'_>) -> Vec<RepMemoryEstimate> {
    let atom_count = input.molecule.atom_count() as u32;
    vec![dot_estimate(atom_count, RepQualityLevel::Full)]
}

fn dot_estimate(atom_count: u32, quality: RepQualityLevel) -> RepMemoryEstimate {
    let instance_capacity_bytes = instance_capacity_for(atom_count);
    let capacity_bytes = CullableBuffers::estimated_memory(instance_capacity_bytes, false)
        .saturating_add(DotBuildParams::SIZE)
        .saturating_add(DotDrawParams::SIZE);
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
    fn dot_density_maps_to_generated_sample_count() {
        assert_eq!(requested_dots_per_atom_from_density(0), 16);
        assert_eq!(requested_dots_per_atom_from_density(1), 32);
        assert_eq!(
            requested_dots_per_atom_from_density(2),
            DEFAULT_DOTS_PER_ATOM
        );
        assert_eq!(requested_dots_per_atom_from_density(4), 256);
        assert_eq!(requested_dots_per_atom_from_density(99), 256);
    }

    #[test]
    fn dot_lod_caps_effective_sample_count() {
        assert_eq!(effective_dots_per_atom_for_lod(256, SceneLod::Auto), 256);
        assert_eq!(effective_dots_per_atom_for_lod(256, SceneLod::High), 256);
        assert_eq!(effective_dots_per_atom_for_lod(256, SceneLod::Medium), 32);
        assert_eq!(effective_dots_per_atom_for_lod(256, SceneLod::Low), 16);
        assert_eq!(effective_dots_per_atom_for_lod(256, SceneLod::Minimum), 8);
        assert_eq!(effective_dots_per_atom_for_lod(16, SceneLod::Minimum), 8);
    }

    #[test]
    fn dot_atom_instance_is_one_entry_per_atom() {
        assert_eq!(DotAtomInstance::SIZE, 32);
        assert_eq!(DotAtomInstance::vertex_layout().array_stride, 32);
        assert_eq!(instance_capacity_for(100_000), 4_194_304);
    }

    #[test]
    fn dot_indirect_seed_uses_effective_samples_per_atom() {
        assert_eq!(dot_vertex_count(8), 48);
        assert_eq!(dot_vertex_count(32), 192);
        assert_eq!(indirect_seed(dot_vertex_count(32)), [192, 0, 0, 0]);
    }
}
