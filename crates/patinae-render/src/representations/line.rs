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
//!
//! One-pixel lines use native segments. Wider bonds use one cap/body strip so
//! the `line_width` setting works on WebGPU, which only rasterises
//! `LineList` primitives at a fixed 1 px width. Hit testing always uses strips. The build kernel stamps the
//! resolved width into `LineInstance::p0_width.w`.

use bytemuck::{Pod, Zeroable};
use patinae_mol::DirtyFlags;
use patinae_settings::ResolvedSettings;

use crate::compute::line_build::{indirect_seed, LineBuildParams, LineBuildPipeline};
use crate::memory::{buffer_usage, GpuMemoryUsage};
use crate::memory::{RepMemoryEstimate, RepQualityLevel};
use crate::picking::RepKind;
use crate::render_input::RenderObjectInput;
use crate::representations::cullable::CullableBuffers;
use crate::representations::Representation;
use crate::representations::{BuildCtx, CullPlan, CullPlanCtx};

// Lines have no per-instance world radius: their footprint is `line_width`
// screen pixels. `plan_cull` expands the side planes once per object to
// cover the visual and picking footprints, leaving per-bond culling unchanged.
// Shader constants are emitted by shader_source from these host-owned values.
pub(crate) const LINE_PICK_MIN_HALF_WIDTH_PX: f32 = 2.0;
pub(crate) const LINE_VISUAL_MIN_HALF_WIDTH_PX: f32 = 0.5;
// A disc centered on a texel corner must reach its nearest sample (sqrt(0.5)).
pub(crate) const LINE_PICK_MIN_TEXEL_RADIUS: f32 = 0.75;
const LINE_CULL_MARGIN_PX: f32 = 1.0;

fn line_cull_ndc_pad(line_width_px: f32, viewport_px: [f32; 2]) -> f32 {
    // Bound every supported picking target, including integer truncation of
    // small/odd dimensions. Culling is shared by visual and hit-test passes.
    let minimum_scale = crate::memory::MIN_PICKING_SCALE;
    let max_pixel_size = viewport_px.into_iter().fold(1.0_f32, |size, axis| {
        size.max(axis.max(1.0) / (axis * minimum_scale).floor().max(1.0))
    });
    let half_width_px = (0.5 * line_width_px)
        .max(LINE_PICK_MIN_HALF_WIDTH_PX)
        .max(LINE_PICK_MIN_TEXEL_RADIUS * max_pixel_size)
        + LINE_CULL_MARGIN_PX;
    2.0 * half_width_px / viewport_px[0].min(viewport_px[1]).max(1.0)
}

// Expanding clip-space x/y bounds is depth-correct in either projection.
// Keep z/w unchanged so near/far clipping retains the ordinary frustum.
fn line_cull_frustum(mut view_proj: [[f32; 4]; 4], ndc_pad: f32) -> [[f32; 4]; 6] {
    for column in &mut view_proj {
        column[0] /= 1.0 + ndc_pad;
        column[1] /= 1.0 + ndc_pad;
    }
    crate::compute::cull::frustum_planes_from_view_proj(&view_proj)
}

/// Two native segments preserve the separate endpoint atom owners.
pub(crate) const LINE_NATIVE_VERTICES_PER_INSTANCE: u32 = 4;

/// Four pairs span the two caps and the body, without a connector.
pub(crate) const LINE_VERTICES_PER_INSTANCE: u32 = 8;
pub(crate) const LINE_TOPOLOGY: wgpu::PrimitiveTopology = wgpu::PrimitiveTopology::TriangleStrip;

/// 48-byte instance entry shared by the line compute and render shaders.
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct LineInstance {
    /// First endpoint in xyz; line width in physical pixels in w.
    pub p0_width: [f32; 4],
    /// Second endpoint in xyz; unused alignment padding in w.
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
    /// Resolved `line_width` (px) from the last build, for cull padding.
    line_width_px: f32,
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
            line_width_px: 0.0,
            needs_dispatch: false,
        }
    }

    pub(crate) fn uses_native_lines(&self) -> bool {
        self.line_width_px == 1.0
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
                line_width: s.line.width,
                _pad: 0,
            }),
        );

        let bond_count = input.molecule.bonds().count() as u32;
        self.last_bond_count = Some(bond_count);
        self.line_width_px = s.line.width;

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
        self.record_translucent(pass);
    }

    fn record_raw_picking<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        self.record_shadow_depth(pass);
    }

    fn prepare_shadow_depth(&self, encoder: &mut wgpu::CommandEncoder, queue: &wgpu::Queue) {
        let seed = indirect_seed(if self.uses_native_lines() {
            LINE_NATIVE_VERTICES_PER_INSTANCE
        } else {
            LINE_VERTICES_PER_INSTANCE
        });
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
        let seed = indirect_seed(if self.uses_native_lines() {
            LINE_NATIVE_VERTICES_PER_INSTANCE
        } else {
            LINE_VERTICES_PER_INSTANCE
        });
        let ndc_pad = line_cull_ndc_pad(self.line_width_px, ctx.viewport_px);
        let padded = CullPlanCtx {
            frustum_planes: line_cull_frustum(ctx.view_proj, ndc_pad),
            ..*ctx
        };
        self.gpu.plan_cull(&padded, upper, 0.0, &seed)
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

#[cfg(test)]
mod tests {
    use super::{line_cull_frustum, line_cull_ndc_pad};

    #[test]
    fn padded_frustum_covers_pixel_footprint_at_every_depth() {
        for perspective in [false, true] {
            let vp = [
                [2.0, 0.0, 0.0, 0.0],
                [0.0, 3.0, 0.0, 0.0],
                [
                    0.0,
                    0.0,
                    if perspective { -1.01 } else { -0.01 },
                    if perspective { -1.0 } else { 0.0 },
                ],
                [
                    0.0,
                    0.0,
                    if perspective { -1.01 } else { -0.01 },
                    if perspective { 0.0 } else { 1.0 },
                ],
            ];
            let planes = line_cull_frustum(vp, 0.125);
            let original = crate::compute::cull::frustum_planes_from_view_proj(&vp);
            assert_eq!(planes[4..], original[4..]);
            for distance in [2.0, 30.0, 90.0] {
                let w = if perspective { distance } else { 1.0 };
                for axis in 0..2 {
                    for sign in [-1.0, 1.0] {
                        for (extent, visible) in [(1.124, true), (1.126, false)] {
                            let mut p = [0.0, 0.0, -distance];
                            p[axis] = sign * extent * w / vp[axis][axis];
                            let inside = planes.iter().all(|plane| {
                                plane[0] * p[0] + plane[1] * p[1] + plane[2] * p[2] + plane[3]
                                    >= 0.0
                            });
                            assert_eq!(inside, visible, "perspective={perspective}, distance={distance}, axis={axis}, sign={sign}, extent={extent}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn cull_pad_covers_line_half_width_in_ndc() {
        // 20 px wide → 10 px half-width + 1 px margin, on a 400 px short axis.
        let pad = line_cull_ndc_pad(20.0, [800.0, 400.0]);
        assert!((pad - 2.0 * 11.0 / 400.0).abs() < 1e-6);
    }

    #[test]
    fn cull_pad_never_drops_below_picking_footprint() {
        let pad = line_cull_ndc_pad(1.0, [100.0, 100.0]);
        assert!(pad >= 2.0 * (0.5 * 100.0 / 12.0 + 1.0) / 100.0);
        assert!(line_cull_ndc_pad(1.0, [0.0, 0.0]).is_finite());
    }

    #[test]
    fn cull_pad_covers_rounded_picking_targets() {
        for viewport in [[15.0, 511.0], [257.0, 255.0], [1.0, 1.0]] {
            for scale in [0.125, 0.25, 0.5, 1.0] {
                let pad = line_cull_ndc_pad(1.0, viewport);
                for axis in viewport {
                    let target = (axis * scale).floor().max(1.0);
                    assert!(pad >= (axis / target + 2.0) / viewport[0].min(viewport[1]));
                }
            }
        }
    }
}
