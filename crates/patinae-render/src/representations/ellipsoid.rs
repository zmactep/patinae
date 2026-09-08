//! Ellipsoid representation — anisotropic-displacement-parameter (ADP)
//! impostor.
//!
//! Uses `SceneStore` (group 2) + GPU compute build using the
//! sparse **build-input pattern**: CPU walks ELLIPSOIDS-visible atoms on
//! non-LUT dirty, runs Jacobi eigendecomp (or isotropic fallback for
//! atoms without `anisou`), packs the resulting axes into a sparse list
//! of [`EllipsoidBuildEntry`], uploads it. The compute kernel reads the
//! list + `SceneStore.coords` and emits compacted [`EllipsoidInstance`]s.
//!
//! Why not GPU-direct like sphere/stick: anisou is sparse on most
//! structures (only crystallographic ADP-refined PDBs carry it), and the
//! Jacobi rotation is rare (TOPOLOGY-frequency, not COORDS). Storing
//! anisou scene-wide in `AtomGpu` would waste +24 B × N atoms for the
//! 99% of structures without ADPs. Keeping the sparse list host-built avoids
//! spending scene-wide memory on data that most structures do not carry.

use bytemuck::{Pod, Zeroable};
use patinae_mol::{DirtyFlags, RepMask};
use patinae_settings::ResolvedSettings;

use crate::compute::ellipsoid_build::{
    indirect_seed, EllipsoidBuildEntry, EllipsoidBuildParams, EllipsoidBuildPipeline,
};
use crate::memory::{buffer_usage, GpuMemoryUsage};
use crate::picking::RepKind;
use crate::pipelines::ellipsoid::{EllipsoidParams, EllipsoidParamsLayout};
use crate::render_input::RenderObjectInput;
use crate::representation_budget::{RepMemoryEstimate, RepQualityLevel};
use crate::representations::cullable::CullableBuffers;
use crate::representations::{BuildCtx, CullPlan, CullPlanCtx, Representation};

/// 64-byte instance entry. Layout matches `EllipsoidInstance` in
/// `shaders/representations/ellipsoid.wgsl`.
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct EllipsoidInstance {
    pub center: [f32; 3],
    pub max_extent: f32,
    pub axis0: [f32; 3],
    pub group_id: u32,
    pub axis1: [f32; 3],
    pub _pad0: u32,
    pub axis2: [f32; 3],
    pub _pad1: u32,
}

const _: () = assert!(std::mem::size_of::<EllipsoidInstance>() == 64);

impl EllipsoidInstance {
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
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 16,
                    shader_location: 2,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Uint32,
                    offset: 28,
                    shader_location: 3,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 32,
                    shader_location: 4,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Uint32,
                    offset: 44,
                    shader_location: 5,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 48,
                    shader_location: 6,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Uint32,
                    offset: 60,
                    shader_location: 7,
                },
            ],
        }
    }
}

/// χ²₃ inverse CDF lookup for common probabilities. Linear interpolation
/// between rows is good enough for visualization (the eyes can't tell a 47%
/// ellipsoid from a 50% one).
fn probability_factor(p: f32) -> f32 {
    const TABLE: &[(f32, f32)] = &[
        (0.10, 1.0046),
        (0.25, 1.4232),
        (0.50, 1.8724),
        (0.68, 2.1556),
        (0.75, 2.2589),
        (0.90, 2.5003),
        (0.95, 2.7955),
        (0.99, 3.3682),
    ];
    let p = p.clamp(0.10, 0.99);
    for w in TABLE.windows(2) {
        let (p0, f0) = w[0];
        let (p1, f1) = w[1];
        if p >= p0 && p <= p1 {
            let t = (p - p0) / (p1 - p0);
            return f0 + t * (f1 - f0);
        }
    }
    1.8724
}

/// Solve the 3x3 symmetric eigenproblem via Jacobi rotations. `u` is given
/// in PDB ANISOU order: `[U11, U22, U33, U12, U13, U23]`. Returns
/// `(eigenvalues, eigenvectors)` where each eigenvector is a column of the
/// returned 3×3 array.
fn eigen_symmetric_3x3(u: [f32; 6]) -> ([f32; 3], [[f32; 3]; 3]) {
    let mut a = [[u[0], u[3], u[4]], [u[3], u[1], u[5]], [u[4], u[5], u[2]]];
    let mut q = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

    for _ in 0..30 {
        let (p, qx, max_abs) = {
            let off01 = a[0][1].abs();
            let off02 = a[0][2].abs();
            let off12 = a[1][2].abs();
            if off01 >= off02 && off01 >= off12 {
                (0usize, 1usize, off01)
            } else if off02 >= off12 {
                (0usize, 2usize, off02)
            } else {
                (1usize, 2usize, off12)
            }
        };
        if max_abs < 1e-10 {
            break;
        }

        let app = a[p][p];
        let aqq = a[qx][qx];
        let apq = a[p][qx];
        let theta = (aqq - app) / (2.0 * apq);
        let t = if theta >= 0.0 {
            1.0 / (theta + (1.0 + theta * theta).sqrt())
        } else {
            1.0 / (theta - (1.0 + theta * theta).sqrt())
        };
        let c = 1.0 / (1.0 + t * t).sqrt();
        let s = t * c;

        let new_app = c * c * app - 2.0 * s * c * apq + s * s * aqq;
        let new_aqq = s * s * app + 2.0 * s * c * apq + c * c * aqq;
        a[p][p] = new_app;
        a[qx][qx] = new_aqq;
        a[p][qx] = 0.0;
        a[qx][p] = 0.0;
        // Rotate the off-diagonal column entries. Reads come from row `i`,
        // writes update both row `i` and the mirrored row/column entries
        // in `a[p]` / `a[qx]`. `i != p && i != qx` keeps the row/column
        // accesses disjoint so the symmetric mirror is well-defined.
        for (i, row) in a.iter_mut().enumerate() {
            if i != p && i != qx {
                let aip = row[p];
                let aiq = row[qx];
                row[p] = c * aip - s * aiq;
                row[qx] = s * aip + c * aiq;
            }
        }
        for i in [0usize, 1, 2] {
            if i != p && i != qx {
                a[p][i] = a[i][p];
                a[qx][i] = a[i][qx];
            }
        }
        for row in &mut q {
            let qip = row[p];
            let qiq = row[qx];
            row[p] = c * qip - s * qiq;
            row[qx] = s * qip + c * qiq;
        }
    }

    let evals = [a[0][0], a[1][1], a[2][2]];
    (evals, q)
}

/// B-factor → isotropic U_iso = B / (8π²); world-axis-aligned diagonal
/// U-tensor.
fn isotropic_axes(b_factor: f32, prob_factor: f32, scale: f32) -> ([f32; 3], [f32; 3], [f32; 3]) {
    const EIGHT_PI2: f32 = 78.95684;
    let u_iso = (b_factor / EIGHT_PI2).max(0.0);
    let r = u_iso.sqrt() * prob_factor * scale;
    ([r, 0.0, 0.0], [0.0, r, 0.0], [0.0, 0.0, r])
}

fn build_input_capacity_for(entry_count: u32) -> u64 {
    let needed = (entry_count as u64).max(1) * EllipsoidBuildEntry::SIZE;
    needed
        .next_power_of_two()
        .max(64 * EllipsoidBuildEntry::SIZE)
}

fn instance_capacity_for(entry_count: u32) -> u64 {
    let needed = (entry_count as u64).max(1) * EllipsoidInstance::SIZE;
    needed.next_power_of_two().max(64 * EllipsoidInstance::SIZE)
}

pub struct EllipsoidRep {
    /// CPU staging — re-allocated on TOPOLOGY/REPS dirty, uploaded into
    /// `build_input_buffer`.
    cpu_build_input: Vec<EllipsoidBuildEntry>,
    build_input_buffer: Option<wgpu::Buffer>,
    build_input_capacity: u64,

    gpu: CullableBuffers,

    build_params_buffer: wgpu::Buffer,
    render_params_buffer: wgpu::Buffer,
    render_params_bind_group: Option<wgpu::BindGroup>,
    compute_bind_group: Option<wgpu::BindGroup>,

    last_entry_count: u32,
    needs_dispatch: bool,
    is_opaque_cache: bool,
}

impl EllipsoidRep {
    pub fn new(device: &wgpu::Device) -> Self {
        let build_params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("patinae.ellipsoid.build_params"),
            size: EllipsoidBuildParams::SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let render_params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("patinae.ellipsoid.render_params"),
            size: EllipsoidParams::SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            cpu_build_input: Vec::new(),
            build_input_buffer: None,
            build_input_capacity: 0,
            gpu: CullableBuffers::new(device, "ellipsoid", EllipsoidInstance::SIZE, true),
            build_params_buffer,
            render_params_buffer,
            render_params_bind_group: None,
            compute_bind_group: None,
            last_entry_count: 0,
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

    pub fn cull_upper_bound(&self) -> Option<u32> {
        if self.last_entry_count == 0 {
            None
        } else {
            Some(self.last_entry_count)
        }
    }

    pub fn ensure_render_bind_group(
        &mut self,
        device: &wgpu::Device,
        layout: &EllipsoidParamsLayout,
    ) {
        if self.render_params_bind_group.is_some() {
            return;
        }
        self.render_params_bind_group =
            Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("patinae.ellipsoid.render_bg"),
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
        entry_count: u32,
        compute_pipeline: &EllipsoidBuildPipeline,
        cull_pipeline: Option<&crate::compute::cull::CullPipeline>,
    ) {
        let build_needed = build_input_capacity_for(entry_count);
        let mut grew = false;
        if self.build_input_capacity < build_needed || self.build_input_buffer.is_none() {
            self.build_input_buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("patinae.ellipsoid.build_input"),
                size: build_needed,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
            self.build_input_capacity = build_needed;
            grew = true;
        }
        let inst_needed = instance_capacity_for(entry_count);
        grew |= self.gpu.ensure(device, inst_needed, cull_pipeline);
        if grew || self.compute_bind_group.is_none() {
            self.compute_bind_group = Some(compute_pipeline.make_bind_group(
                device,
                &self.build_params_buffer,
                self.build_input_buffer.as_ref().unwrap(),
                self.gpu.raw_instance_buffer().unwrap(),
                self.gpu.raw_count_buffer().unwrap(),
            ));
        }
    }
}

impl Representation for EllipsoidRep {
    fn kind(&self) -> RepKind {
        RepKind::Ellipsoid
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
        let scale = s.ellipsoid.scale;
        let transparency = s.ellipsoid.transparency.clamp(0.0, 1.0);
        let prob = probability_factor(s.ellipsoid.probability);
        let alpha_mul = 1.0 - transparency;

        queue.write_buffer(
            &self.render_params_buffer,
            0,
            bytemuck::bytes_of(&EllipsoidParams {
                alpha_mul,
                _pad0: 0.0,
                _pad1: 0.0,
                _pad2: 0.0,
            }),
        );

        // Opacity heuristic — same approach as sphere/stick. CPU pre-scan
        // for any sub-unit per-atom override; conservatively flips to
        // WBOIT.
        let mut opaque = alpha_mul >= 0.999;
        if opaque {
            for atom in input.molecule.atoms() {
                if let Some(t) = atom.repr.ellipsoid_transparency {
                    if (1.0 - t.clamp(0.0, 1.0)) < 0.999 {
                        opaque = false;
                        break;
                    }
                }
            }
        }
        self.is_opaque_cache = opaque;

        if dirty.is_lut_only() && self.build_input_buffer.is_some() {
            // LUT-only fast path. Render params already updated; nothing
            // else to do for the rep — no compute dispatch needed.
            return;
        }

        // Rebuild the sparse build-input list. CPU iter happens here
        // (rare — only on TOPOLOGY/REPS/COORDS-or-scale change). Note
        // that COORDS-only doesn't actually need axis recomputation, but
        // dirty doesn't separate "scale changed" from "coords changed",
        // so we conservatively rebuild on any non-LUT dirty wave to
        // match existing output.
        self.cpu_build_input.clear();
        for (atom_local, atom) in input.molecule.atoms().enumerate() {
            let local = atom_local as u32;
            if !atom.repr.visible_reps.is_visible(RepMask::ELLIPSOIDS) {
                continue;
            }
            let (a0, a1, a2) = match atom.anisou {
                Some(u) => {
                    let (evals, evecs) = eigen_symmetric_3x3(u);
                    let r0 = evals[0].max(0.0).sqrt() * prob * scale;
                    let r1 = evals[1].max(0.0).sqrt() * prob * scale;
                    let r2 = evals[2].max(0.0).sqrt() * prob * scale;
                    (
                        [evecs[0][0] * r0, evecs[1][0] * r0, evecs[2][0] * r0],
                        [evecs[0][1] * r1, evecs[1][1] * r1, evecs[2][1] * r1],
                        [evecs[0][2] * r2, evecs[1][2] * r2, evecs[2][2] * r2],
                    )
                }
                None => isotropic_axes(atom.b_factor, prob, scale),
            };
            // Skip degenerate ellipsoids — would just discard every
            // fragment in the FS otherwise.
            let len_sq = |v: [f32; 3]| v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
            if len_sq(a0).max(len_sq(a1)).max(len_sq(a2)) < 1e-8 {
                continue;
            }
            self.cpu_build_input.push(EllipsoidBuildEntry {
                axis0: a0,
                atom_local: local,
                axis1: a1,
                _pad0: 0,
                axis2: a2,
                _pad1: 0,
            });
        }

        self.last_entry_count = self.cpu_build_input.len() as u32;
        self.needs_dispatch = self.last_entry_count > 0
            // Always dispatch on first build (which seeds indirect_args)
            || self.gpu.indirect_buffer().is_none();

        // Update build params (entry_count) — used by compute kernel
        // bounds check.
        queue.write_buffer(
            &self.build_params_buffer,
            0,
            bytemuck::bytes_of(&EllipsoidBuildParams {
                entry_count: self.last_entry_count,
                _pad0: 0,
                _pad1: 0,
                _pad2: 0,
            }),
        );
    }

    fn is_opaque(&self) -> bool {
        self.is_opaque_cache
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
        self.ensure_render_bind_group(ctx.device, &ctx.pipelines.ellipsoid_params_layout);
        if !self.needs_dispatch {
            return false;
        }
        let entry_count = self.last_entry_count;
        let compute_pipeline = &ctx.pipelines.ellipsoid_compute;
        let cull_pipeline = &ctx.pipelines.cull_pipeline;
        // Allocate at least one entry's worth of buffers so the indirect
        // exists even when zero ellipsoids are visible (next dispatch
        // can still read a valid indirect with instance_count=0).
        self.ensure_buffers(
            ctx.device,
            entry_count.max(1),
            compute_pipeline,
            Some(cull_pipeline),
        );

        if entry_count > 0 {
            if let Some(buf) = self.build_input_buffer.as_ref() {
                ctx.queue
                    .write_buffer(buf, 0, bytemuck::cast_slice(&self.cpu_build_input));
            }
        }
        self.gpu.reset_raw_count(ctx.queue);
        if entry_count > 0 {
            let bg = match self.compute_bind_group.as_ref() {
                Some(b) => b,
                None => {
                    self.needs_dispatch = false;
                    return false;
                }
            };
            let Some(scene_bg) = ctx.object_coords_scene_bg else {
                return false;
            };
            compute_pipeline.dispatch(
                ctx.encoder,
                scene_bg,
                ctx.obj_dynamic_offset,
                bg,
                entry_count,
            );
        }
        self.needs_dispatch = false;
        true
    }

    fn plan_cull(&mut self, ctx: &CullPlanCtx<'_>) -> Option<CullPlan<'_>> {
        let upper = self.cull_upper_bound()?;
        let seed = indirect_seed(6);
        self.gpu.plan_cull(ctx, upper, 0.0, &seed)
    }

    fn memory_usage(&self) -> GpuMemoryUsage {
        let mut usage = self.gpu.memory_usage();
        usage.add(buffer_usage(&self.build_params_buffer));
        usage.add(buffer_usage(&self.render_params_buffer));
        if let Some(buffer) = self.build_input_buffer.as_ref() {
            usage.add(buffer_usage(buffer));
        }
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
    let entry_count = input
        .molecule
        .atoms()
        .filter(|atom| atom.repr.visible_reps.is_visible(RepMask::ELLIPSOIDS))
        .count() as u32;
    vec![ellipsoid_estimate(entry_count, RepQualityLevel::Full)]
}

fn ellipsoid_estimate(entry_count: u32, quality: RepQualityLevel) -> RepMemoryEstimate {
    let build_input_bytes = build_input_capacity_for(entry_count);
    let instance_capacity_bytes = instance_capacity_for(entry_count);
    let capacity_bytes = CullableBuffers::estimated_memory(instance_capacity_bytes, true)
        .saturating_add(build_input_bytes)
        .saturating_add(EllipsoidBuildParams::SIZE)
        .saturating_add(EllipsoidParams::SIZE);
    RepMemoryEstimate {
        required_bytes: build_input_bytes.max(instance_capacity_bytes),
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
    fn eigen_isotropic_returns_equal_evals() {
        let u = [0.05, 0.05, 0.05, 0.0, 0.0, 0.0];
        let (evals, _evecs) = eigen_symmetric_3x3(u);
        for v in evals {
            assert!((v - 0.05).abs() < 1e-5, "expected 0.05, got {v}");
        }
    }

    #[test]
    fn eigen_diagonal_returns_diagonal_entries() {
        let u = [0.10, 0.20, 0.30, 0.0, 0.0, 0.0];
        let (mut evals, _) = eigen_symmetric_3x3(u);
        evals.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert!((evals[0] - 0.10).abs() < 1e-5);
        assert!((evals[1] - 0.20).abs() < 1e-5);
        assert!((evals[2] - 0.30).abs() < 1e-5);
    }

    #[test]
    fn eigen_general_evecs_are_orthogonal() {
        let u = [0.10, 0.20, 0.30, 0.05, 0.02, 0.04];
        let (_, evecs) = eigen_symmetric_3x3(u);
        let col = |i: usize| [evecs[0][i], evecs[1][i], evecs[2][i]];
        let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let c0 = col(0);
        let c1 = col(1);
        let c2 = col(2);
        assert!(dot(c0, c1).abs() < 1e-4);
        assert!(dot(c0, c2).abs() < 1e-4);
        assert!(dot(c1, c2).abs() < 1e-4);
        for c in [c0, c1, c2] {
            let l2 = dot(c, c);
            assert!((l2 - 1.0).abs() < 1e-4, "|v|² = {l2}");
        }
    }

    #[test]
    fn probability_factor_matches_table() {
        assert!((probability_factor(0.50) - 1.8724).abs() < 1e-3);
        assert!((probability_factor(0.90) - 2.5003).abs() < 1e-3);
    }
}
