//! Line representation. Migrated to `SceneStore` (group 2) + GPU compute
//! build. Per-bond CPU iteration is gone; the [`crate::compute::line_build`]
//! kernel reads scene-wide bonds + atoms + coords and emits compacted
//! [`LineInstance`]s with the visible-instance count written atomically
//! into the indirect-draw args buffer. Multi-bonds emit one offset line
//! instance per valence slot, using the same SceneStore-baked
//! `BondGpu::valence_perp_pad` direction as sticks.
//!
//! Lines have no per-representation transparency, so the render pipeline has
//! no group 3.

use bytemuck::{Pod, Zeroable};
use patinae_mol::DirtyFlags;
use patinae_settings::ResolvedSettings;

use crate::compute::line_build::{indirect_seed, LineBuildParams, LineBuildPipeline};
use crate::memory::{buffer_usage, GpuMemoryUsage};
use crate::picking::RepKind;
use crate::representation_budget::{RepMemoryEstimate, RepQualityLevel};
use crate::representations::cullable::CullableBuffers;
use crate::representations::{BuildCtx, CullPlan, CullPlanCtx};
// Constant width used to bound visible lines under camera cull. Lines have
// no per-instance radius; this matches `LINE_KIND_RADIUS` from the
// pre-refactor `culling_flow.rs` so cull behaviour is preserved.
const LINE_KIND_RADIUS: f32 = 0.1;
use crate::render_input::RenderObjectInput;
use crate::representations::Representation;

/// 48-byte instance entry. Layout matches `LineInstance` in
/// `shaders/representations/line.wgsl`.
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct LineInstance {
    pub p0_pad: [f32; 4],
    pub p1_pad: [f32; 4],
    pub groups: [u32; 2],
    pub _pad: [u32; 2],
}

impl LineInstance {
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
    let needed = (bond_count as u64).max(1) * 3 * LineInstance::SIZE;
    needed.next_power_of_two().max(64 * LineInstance::SIZE)
}

pub struct LineRep {
    gpu: CullableBuffers,
    build_params_buffer: wgpu::Buffer,
    compute_bind_group: Option<wgpu::BindGroup>,

    last_bond_count: Option<u32>,
    needs_dispatch: bool,
}

impl LineRep {
    pub fn new(device: &wgpu::Device) -> Self {
        let build_params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("patinae.line.build_params"),
            size: LineBuildParams::SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            gpu: CullableBuffers::new(device, "line", LineInstance::SIZE, true),
            build_params_buffer,
            compute_bind_group: None,
            last_bond_count: None,
            needs_dispatch: false,
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

    pub fn cull_upper_bound(&self) -> Option<u32> {
        self.last_bond_count.map(|n| n.saturating_mul(3))
    }

    fn ensure_buffers(
        &mut self,
        device: &wgpu::Device,
        bond_count: u32,
        compute_pipeline: &LineBuildPipeline,
        cull_pipeline: Option<&crate::compute::cull::CullPipeline>,
    ) {
        let needed = instance_capacity_for(bond_count);
        let grew = self.gpu.ensure(device, needed, cull_pipeline);
        if grew || self.compute_bind_group.is_none() {
            self.compute_bind_group = Some(compute_pipeline.make_bind_group(
                device,
                &self.build_params_buffer,
                self.gpu.raw_instance_buffer().unwrap(),
                self.gpu.raw_count_buffer().unwrap(),
            ));
        }
    }
}

impl Representation for LineRep {
    fn kind(&self) -> RepKind {
        RepKind::Line
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
        let valence_scale_factor = s.stick.radius * 1.5 * s.stick.valence_scale;

        queue.write_buffer(
            &self.build_params_buffer,
            0,
            bytemuck::bytes_of(&LineBuildParams {
                valence_scale: valence_scale_factor,
                valence_enabled: if s.stick.valence { 1 } else { 0 },
                _pad1: 0,
                _pad2: 0,
            }),
        );

        let bond_count = input.molecule.bonds().count() as u32;
        self.last_bond_count = Some(bond_count);

        if dirty.is_lut_only() && self.gpu.has_raw_instances() {
            return;
        }
        self.needs_dispatch = true;
    }

    fn record_translucent<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        let (Some(instance_buf), Some(indirect_buf)) = (
            self.gpu.compacted_instance_buffer(),
            self.gpu.indirect_buffer(),
        ) else {
            return;
        };
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
        let seed = indirect_seed(4);
        self.gpu.prepare_raw_shadow_indirect(encoder, queue, &seed);
    }

    fn record_shadow_depth<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        let (Some(instance_buf), Some(indirect_buf)) = (
            self.gpu.raw_instance_buffer(),
            self.gpu.shadow_indirect_buffer(),
        ) else {
            return;
        };
        pass.set_vertex_buffer(0, instance_buf.slice(..));
        pass.draw_indirect(indirect_buf, 0);
    }

    fn record_compute_build(&mut self, ctx: &mut BuildCtx<'_>) -> bool {
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
        let compute_pipeline = &ctx.pipelines.line_compute;
        let cull_pipeline = &ctx.pipelines.cull_pipeline;
        self.ensure_buffers(
            ctx.device,
            bond_count,
            compute_pipeline,
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
        let seed = indirect_seed(4);
        self.gpu.plan_cull(ctx, upper, LINE_KIND_RADIUS, &seed)
    }

    fn memory_usage(&self) -> GpuMemoryUsage {
        let mut usage = self.gpu.memory_usage();
        usage.add(buffer_usage(&self.build_params_buffer));
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
    let bond_count = input.molecule.bonds().count() as u32;
    vec![line_estimate(bond_count, RepQualityLevel::Full)]
}

fn line_estimate(bond_count: u32, quality: RepQualityLevel) -> RepMemoryEstimate {
    let instance_capacity_bytes = instance_capacity_for(bond_count);
    let capacity_bytes = CullableBuffers::estimated_memory(instance_capacity_bytes, true)
        .saturating_add(LineBuildParams::SIZE);
    RepMemoryEstimate {
        required_bytes: instance_capacity_bytes,
        scratch_bytes: 0,
        capacity_bytes,
        quality,
        can_chunk: false,
        can_skip: true,
    }
}
