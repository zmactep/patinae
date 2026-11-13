//! Compute pipeline that builds the ellipsoid instance buffer on the GPU.
//!
//! **Build-input pattern.** Unlike sphere/stick/line/dot which iterate
//! every atom on the GPU and gate by `repr_flags`, ellipsoid axes
//! require a 3×3 Jacobi eigendecomp per anisou-bearing atom. That's
//! impractical to broadcast across all atoms (most carry no anisou) and
//! the eigendecomp is rare (TOPOLOGY-frequency). Instead, the host walks
//! ELLIPSOIDS-visible atoms once on non-LUT dirty, runs eigendecomp on
//! CPU, and uploads a packed sparse list of `EllipsoidBuildEntry`. The
//! compute kernel runs one thread per entry and just reads coords +
//! emits instance.

use std::num::NonZeroU64;

use bytemuck::{Pod, Zeroable};

use crate::scene_store::SceneStoreLayout;
use crate::shader_source;

pub const WORKGROUP: u32 = 64;
#[cfg(test)]
const BUILD_STORAGE_BINDING_COUNT: usize = 3;
#[cfg(test)]
const WEBGPU_MAX_STORAGE_BUFFERS_PER_COMPUTE_STAGE: usize = 8;

/// Mirror of `EllipsoidBuildEntry` in `build_ellipsoid.wgsl`. **48 B**.
/// Each entry corresponds to one atom whose ellipsoid axes have been
/// pre-computed (Jacobi for anisou, isotropic fallback for b_factor).
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct EllipsoidBuildEntry {
    pub axis0: [f32; 3],
    pub atom_local: u32,
    pub axis1: [f32; 3],
    pub _pad0: u32,
    pub axis2: [f32; 3],
    pub _pad1: u32,
}

impl EllipsoidBuildEntry {
    pub const SIZE: u64 = std::mem::size_of::<Self>() as u64;
}

const _: () = assert!(std::mem::size_of::<EllipsoidBuildEntry>() == 48);

/// Mirror of `EllipsoidBuildParams` in `build_ellipsoid.wgsl`. 16 B.
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct EllipsoidBuildParams {
    pub entry_count: u32,
    pub _pad0: u32,
    pub _pad1: u32,
    pub _pad2: u32,
}

impl EllipsoidBuildParams {
    pub const SIZE: u64 = std::mem::size_of::<Self>() as u64;
}

pub use crate::compute::indirect_seed;

pub struct EllipsoidBuildPipeline {
    pub pipeline: wgpu::ComputePipeline,
    pub build_layout: wgpu::BindGroupLayout,
}

impl EllipsoidBuildPipeline {
    pub fn new(device: &wgpu::Device, scene_layout: &SceneStoreLayout) -> Self {
        let build_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("patinae.ellipsoid_build.layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(EllipsoidBuildParams::SIZE),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(EllipsoidBuildEntry::SIZE),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        // EllipsoidInstance = 64 B.
                        min_binding_size: NonZeroU64::new(64),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        // raw_count: array<atomic<u32>, 1> — 4 B
                        min_binding_size: NonZeroU64::new(4),
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("patinae.ellipsoid_build.pipeline_layout"),
            bind_group_layouts: &[
                Some(&scene_layout.object_coords_bind_group_layout),
                Some(&build_layout),
            ],
            immediate_size: 0,
        });

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("patinae.ellipsoid_build.wgsl"),
            source: wgpu::ShaderSource::Wgsl(
                shader_source::expand(shader_source::BUILD_ELLIPSOID_WGSL).into(),
            ),
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("patinae.ellipsoid_build.pipeline"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("cs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        Self {
            pipeline,
            build_layout,
        }
    }

    pub fn make_bind_group(
        &self,
        device: &wgpu::Device,
        params_buf: &wgpu::Buffer,
        build_input_buf: &wgpu::Buffer,
        raw_instance_buf: &wgpu::Buffer,
        raw_count_buf: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("patinae.ellipsoid_build.bg"),
            layout: &self.build_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: build_input_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: raw_instance_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: raw_count_buf.as_entire_binding(),
                },
            ],
        })
    }

    pub fn dispatch(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        object_coords_scene_bg: &wgpu::BindGroup,
        obj_dynamic_offset: u32,
        build_bg: &wgpu::BindGroup,
        entry_count: u32,
    ) {
        if entry_count == 0 {
            return;
        }
        let groups = entry_count.div_ceil(WORKGROUP);
        let (wg_x, wg_y) = super::split_1d_dispatch(groups);
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("patinae.ellipsoid_build.dispatch"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, object_coords_scene_bg, &[obj_dynamic_offset]);
        pass.set_bind_group(1, build_bg, &[]);
        pass.dispatch_workgroups(wg_x, wg_y, 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene_store::layout::{
        FULL_STORAGE_BINDING_COUNT, OBJECT_COORDS_STORAGE_BINDING_COUNT,
    };

    #[test]
    fn ellipsoid_build_uses_slim_scene_layout_under_webgpu_compute_limit() {
        const {
            assert!(
                OBJECT_COORDS_STORAGE_BINDING_COUNT + BUILD_STORAGE_BINDING_COUNT
                    <= WEBGPU_MAX_STORAGE_BUFFERS_PER_COMPUTE_STAGE,
                "ellipsoid build must stay within WebGPU's portable 8 storage-buffer limit"
            );
        }
        const {
            assert!(
                FULL_STORAGE_BINDING_COUNT + BUILD_STORAGE_BINDING_COUNT
                    > WEBGPU_MAX_STORAGE_BUFFERS_PER_COMPUTE_STAGE,
                "the full SceneStore layout would regress Chrome/WebGPU startup validation"
            );
        }
    }
}
