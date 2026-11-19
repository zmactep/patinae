//! SSAO compute + bilateral blur.
//!
//! Depth-only SSAO keeps the pass independent of material and lighting state.
//!
//! Owns one `SsaoCompute` (ssao.wgsl) + one `SsaoBlur` (ssao_blur.wgsl).
//! Both write to `R8Unorm` storage textures held by `FrameTargets`
//! (`ssao_texture` / `ssao_blurred_texture`). The compose post-pass
//! lives in `postprocess::ssao_compose`.

use std::num::NonZeroU64;

use bytemuck::{Pod, Zeroable};

use crate::context::RenderContext;
use crate::frame::{FrameTargets, SSAO_FORMAT};
use crate::shader_source;

/// Number of hemisphere samples per pixel. Must match `N_SAMPLES` in
/// `ssao.wgsl`.
pub const SSAO_SAMPLES: usize = 32;

/// `SsaoParams` mirror in WGSL. 64 B base + 32 vec4 samples = 576 B.
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct SsaoParams {
    pub radius: f32,
    pub bias: f32,
    pub frame_phase: f32,
    pub _pad: f32,
    pub samples: [[f32; 4]; SSAO_SAMPLES],
}

impl SsaoParams {
    pub const SIZE: u64 = std::mem::size_of::<Self>() as u64;

    /// Build per-pixel hemisphere samples. Pure Rust pseudo-random
    /// (deterministic) so headless tests render the same AO pattern.
    /// Each sample is in the hemisphere `z >= 0`, length-biased so
    /// shorter samples dominate (smoother gradient).
    pub fn default_samples() -> [[f32; 4]; SSAO_SAMPLES] {
        let mut out = [[0.0_f32; 4]; SSAO_SAMPLES];
        for (i, slot) in out.iter_mut().enumerate() {
            // Halton sequence in 3D (bases 2/3/5).
            let h2 = halton(i as u32 + 1, 2);
            let h3 = halton(i as u32 + 1, 3);
            let h5 = halton(i as u32 + 1, 5);
            let phi = h2 * std::f32::consts::TAU;
            let cos_theta = h3;
            let sin_theta = (1.0 - cos_theta * cos_theta).max(0.0).sqrt();
            let mut x = sin_theta * phi.cos();
            let mut y = sin_theta * phi.sin();
            let mut z = cos_theta;
            // Length bias: scale by t² with t ∈ [0.1, 1] so most samples
            // are near the surface.
            let t = 0.1 + h5 * 0.9;
            let scale = t * t;
            x *= scale;
            y *= scale;
            z *= scale;
            *slot = [x, y, z, 0.0];
        }
        out
    }

    pub fn new(radius: f32, bias: f32, frame_phase: f32) -> Self {
        Self {
            radius,
            bias,
            frame_phase,
            _pad: 0.0,
            samples: Self::default_samples(),
        }
    }
}

fn halton(mut index: u32, base: u32) -> f32 {
    let mut f = 1.0_f32;
    let mut result = 0.0_f32;
    while index > 0 {
        f /= base as f32;
        result += f * (index % base) as f32;
        index /= base;
    }
    result
}

pub struct SsaoCompute {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
}

impl SsaoCompute {
    pub fn new(device: &wgpu::Device) -> Self {
        // Group 0: FrameUniforms (binding 0) + SsaoParams (binding 1) +
        // depth (binding 2) + storage AO (binding 3). FrameUniforms is
        // already exposed via `RenderContext::frame.bind_group`, which
        // uses its own BGL — we declare our own here matching the same
        // FrameUniforms binding so the shader sees `frame` at @group(0).
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("patinae.ssao.bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(crate::uniforms::FrameUniforms::SIZE),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(SsaoParams::SIZE),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: SSAO_FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("patinae.ssao.pl"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("patinae.ssao.wgsl"),
            source: wgpu::ShaderSource::Wgsl(
                shader_source::expand(shader_source::SSAO_WGSL).into(),
            ),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("patinae.ssao.pipeline"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("cs_ssao"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        Self { pipeline, layout }
    }

    pub fn make_bind_group(
        &self,
        device: &wgpu::Device,
        frame_buf: &wgpu::Buffer,
        params_buf: &wgpu::Buffer,
        depth_view: &wgpu::TextureView,
        out_view: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("patinae.ssao.bg"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: frame_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: params_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(out_view),
                },
            ],
        })
    }

    pub fn dispatch(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        bg: &wgpu::BindGroup,
        targets: &FrameTargets,
        timestamp_writes: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("patinae.ssao.dispatch"),
            timestamp_writes,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bg, &[]);
        let (w, h) = (targets.width, targets.height);
        pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
    }
}

#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct SsaoBlurParams {
    pub dir: [i32; 2],
    pub depth_sigma: f32,
    pub _pad: f32,
}

impl SsaoBlurParams {
    pub const SIZE: u64 = std::mem::size_of::<Self>() as u64;
}

pub struct SsaoBlur {
    pipeline: wgpu::ComputePipeline,
    pub layout: wgpu::BindGroupLayout,
}

impl SsaoBlur {
    pub fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("patinae.ssao_blur.bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(SsaoBlurParams::SIZE),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: SSAO_FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("patinae.ssao_blur.pl"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("patinae.ssao_blur.wgsl"),
            source: wgpu::ShaderSource::Wgsl(shader_source::SSAO_BLUR_WGSL.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("patinae.ssao_blur.pipeline"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("cs_blur"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        Self { pipeline, layout }
    }

    pub fn make_bind_group(
        &self,
        device: &wgpu::Device,
        params_buf: &wgpu::Buffer,
        depth_view: &wgpu::TextureView,
        src_view: &wgpu::TextureView,
        dst_view: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("patinae.ssao_blur.bg"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(src_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(dst_view),
                },
            ],
        })
    }

    pub fn dispatch(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        bg: &wgpu::BindGroup,
        targets: &FrameTargets,
        timestamp_writes: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("patinae.ssao_blur.dispatch"),
            timestamp_writes,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bg, &[]);
        let (w, h) = (targets.width, targets.height);
        pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
    }
}

/// Per-`RenderState` resources. Owns the params uniform buffers + lazy
/// bind groups (rebuilt on resize / settings change).
pub struct SsaoResources {
    pub ssao_params_buffer: wgpu::Buffer,
    pub blur_h_params_buffer: wgpu::Buffer,
    pub blur_v_params_buffer: wgpu::Buffer,
}

impl SsaoResources {
    pub fn new(ctx: &RenderContext) -> Self {
        let device = &ctx.device;
        let ssao_params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("patinae.ssao.params"),
            size: SsaoParams::SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // The sampling kernel is constant. Each frame updates only radius, bias and phase.
        ctx.queue.write_buffer(
            &ssao_params_buffer,
            0,
            bytemuck::bytes_of(&SsaoParams::new(0.0, 0.0, 0.0)),
        );
        let blur_h_params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("patinae.ssao_blur.params.h"),
            size: SsaoBlurParams::SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let blur_v_params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("patinae.ssao_blur.params.v"),
            size: SsaoBlurParams::SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // Pre-fill blur dirs.
        ctx.queue.write_buffer(
            &blur_h_params_buffer,
            0,
            bytemuck::bytes_of(&SsaoBlurParams {
                dir: [1, 0],
                depth_sigma: 0.02,
                _pad: 0.0,
            }),
        );
        ctx.queue.write_buffer(
            &blur_v_params_buffer,
            0,
            bytemuck::bytes_of(&SsaoBlurParams {
                dir: [0, 1],
                depth_sigma: 0.02,
                _pad: 0.0,
            }),
        );
        Self {
            ssao_params_buffer,
            blur_h_params_buffer,
            blur_v_params_buffer,
        }
    }
}
