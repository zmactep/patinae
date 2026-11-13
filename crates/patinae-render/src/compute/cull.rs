//! Per-rep GPU culling pipeline.
//!
//! For each culled representation kind, one compute pipeline reads the raw
//! instance buffer that was emitted by the corresponding build kernel and
//! writes a frustum-tested compacted buffer + the indirect-draw args.
//!
//! Bind groups:
//! - Group 0: per-rep CullParams uniform + per-rep storage buffers.

use std::num::NonZeroU64;

use bytemuck::{Pod, Zeroable};

use crate::shader_source;

pub const WORKGROUP: u32 = 64;

/// Mirror of `CullParams` in `cull_common.wgsl`. 176 B (16-aligned).
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct CullParams {
    pub view_proj: [[f32; 4]; 4],
    /// World-space frustum planes (l, r, b, t, n, f). `dot(plane.xyz, p)
    /// + plane.w >= 0` ⇔ p inside.
    pub frustum_planes: [[f32; 4]; 6],
    pub raw_capacity: u32,
    /// Per-kind bounding-sphere pad. Used by kinds with no per-instance
    /// radius (line: half line-width in NDC units, converted per instance;
    /// dot: dot radius). Sphere cull ignores
    /// this, but sphere viewport-count reuses the same uniform slot for
    /// `sphere_scale`.
    pub kind_radius: f32,
    pub _pad0: u32,
    pub _pad1: u32,
}

impl CullParams {
    pub const SIZE: u64 = std::mem::size_of::<Self>() as u64;
}

const _: () = assert!(std::mem::size_of::<CullParams>() == 176);

/// Extract 6 world-space frustum planes from a row-major view-projection
/// matrix in WGSL convention (`v * VP`). Planes follow the Gribb-Hartmann
/// formulation; each is normalized so the `plane.xyz` is a unit normal
/// and `plane.w` is the signed distance offset such that
/// `dot(plane.xyz, p) + plane.w >= 0` ⇔ p inside.
///
/// `vp` is stored row-major as `[[f32; 4]; 4]` where outer index = row.
pub fn frustum_planes_from_view_proj(vp: &[[f32; 4]; 4]) -> [[f32; 4]; 6] {
    // wgpu / WGSL stores matrices column-major in shaders. `FrameUniforms`
    // already encodes view_proj as `[[f32; 4]; 4]` rows-of-columns
    // (each `[f32;4]` is one column). To use Gribb-Hartmann the standard
    // way (rows of the row-major matrix), index transposed.
    let m = |row: usize, col: usize| -> f32 { vp[col][row] };
    let row = |r: usize| -> [f32; 4] { [m(r, 0), m(r, 1), m(r, 2), m(r, 3)] };
    let r0 = row(0);
    let r1 = row(1);
    let r2 = row(2);
    let r3 = row(3);
    let add = |a: [f32; 4], b: [f32; 4]| -> [f32; 4] {
        [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]]
    };
    let sub = |a: [f32; 4], b: [f32; 4]| -> [f32; 4] {
        [a[0] - b[0], a[1] - b[1], a[2] - b[2], a[3] - b[3]]
    };
    // Left = r3 + r0, Right = r3 - r0, Bottom = r3 + r1, Top = r3 - r1.
    // Near = r2 (clip-z in [0,1] convention; wgpu = D3D), Far = r3 - r2.
    let planes = [
        add(r3, r0),
        sub(r3, r0),
        add(r3, r1),
        sub(r3, r1),
        r2,
        sub(r3, r2),
    ];
    let mut out = [[0.0_f32; 4]; 6];
    for (i, p) in planes.iter().enumerate() {
        let n_len = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt().max(1e-9);
        out[i] = [p[0] / n_len, p[1] / n_len, p[2] / n_len, p[3] / n_len];
    }
    out
}

pub struct CullPipeline {
    pub pipeline_sphere: wgpu::ComputePipeline,
    pub pipeline_stick: wgpu::ComputePipeline,
    pub pipeline_line: wgpu::ComputePipeline,
    pub pipeline_dot: wgpu::ComputePipeline,
    pub pipeline_ellipsoid: wgpu::ComputePipeline,
    pub rep_layout: wgpu::BindGroupLayout,
}

impl CullPipeline {
    pub fn new(device: &wgpu::Device) -> Self {
        // Shared across all per-kind cull pipelines. The two
        // instance-buffer entries (1 = raw, 3 = compacted) use
        // `min_binding_size: None` so the same BGL accommodates every
        // kind's instance struct (Sphere=32 / Stick=Line=48 / Dot=32 /
        // Ellipsoid=64). Each pipeline's WGSL declares the typed
        // struct.
        let rep_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("patinae.cull.rep.layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(CullParams::SIZE),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(4),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(16),
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("patinae.cull.pipeline_layout"),
            bind_group_layouts: &[Some(&rep_layout)],
            immediate_size: 0,
        });

        let make_pipeline = |label: &str, src: &str, entry: &str| {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(shader_source::expand(src).into()),
            });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        };

        let pipeline_sphere = make_pipeline(
            "patinae.cull.pipeline.sphere",
            shader_source::CULL_INSTANCES_WGSL,
            "cs_cull_sphere",
        );
        let pipeline_stick = make_pipeline(
            "patinae.cull.pipeline.stick",
            shader_source::CULL_STICK_WGSL,
            "cs_cull_stick",
        );
        let pipeline_line = make_pipeline(
            "patinae.cull.pipeline.line",
            shader_source::CULL_LINE_WGSL,
            "cs_cull_line",
        );
        let pipeline_dot = make_pipeline(
            "patinae.cull.pipeline.dot",
            shader_source::CULL_DOT_WGSL,
            "cs_cull_dot",
        );
        let pipeline_ellipsoid = make_pipeline(
            "patinae.cull.pipeline.ellipsoid",
            shader_source::CULL_ELLIPSOID_WGSL,
            "cs_cull_ellipsoid",
        );

        Self {
            pipeline_sphere,
            pipeline_stick,
            pipeline_line,
            pipeline_dot,
            pipeline_ellipsoid,
            rep_layout,
        }
    }

    /// Build the per-rep bind group. Lifetime ties to the rep's buffers;
    /// recreated when the rep grows its raw/compacted buffers.
    pub fn make_rep_bind_group(
        &self,
        device: &wgpu::Device,
        params_buf: &wgpu::Buffer,
        raw_inst: &wgpu::Buffer,
        raw_count: &wgpu::Buffer,
        compacted: &wgpu::Buffer,
        indirect: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("patinae.cull.rep.bg"),
            layout: &self.rep_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: raw_inst.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: raw_count.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: compacted.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: indirect.as_entire_binding(),
                },
            ],
        })
    }

    /// Dispatch one kind's cull kernel. `instance_upper_bound` is the
    /// worst-case number of raw instances; the shader bails on each
    /// thread whose index ≥ actual GPU-side `raw_count`.
    pub fn dispatch_kind(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::ComputePipeline,
        rep_bg: &wgpu::BindGroup,
        instance_upper_bound: u32,
        label: &str,
        timestamp_writes: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        if instance_upper_bound == 0 {
            return;
        }
        let groups = instance_upper_bound.div_ceil(WORKGROUP);
        let (wg_x, wg_y) = super::split_1d_dispatch(groups);
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some(label),
            timestamp_writes,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, rep_bg, &[]);
        pass.dispatch_workgroups(wg_x, wg_y, 1);
    }
}
