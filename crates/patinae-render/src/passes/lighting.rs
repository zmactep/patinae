//! Visible-pass lighting/occlusion bind-group contract.
//!
//! Group 1 is intentionally neutral: visible representation shaders bind a
//! depth texture, comparison sampler, and uniforms describing whether that
//! texture currently means directional shadows, atlas AO, or disabled lighting
//! occlusion.

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::memory::{buffer_usage, estimate_texture_2d_bytes, GpuMemoryUsage};

pub const OCCLUSION_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
pub const DEFAULT_SHADOW_MAP_SIZE: u32 = 1024;
/// Keep the lighting uniform within wgpu's 16 KiB downlevel binding limit.
///
/// `LightingOcclusionUniforms` has a 96-byte fixed header and one 64-byte
/// matrix per atlas direction, so 254 directions occupy 16,352 bytes.
pub const MAX_ATLAS_DIRECTIONS: usize = 254;

pub const OCCLUSION_MODE_DISABLED: u32 = 0;
pub const OCCLUSION_MODE_ATLAS_AO: u32 = 1;
pub const OCCLUSION_MODE_DIRECTIONAL: u32 = 2;

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct LightingOcclusionUniforms {
    pub view_proj: [[f32; 4]; 4],
    /// `(bias, intensity, atlas_texel_size, pcf_radius)`.
    pub params: [f32; 4],
    /// `(direction_count, atlas_grid, mode, tile_size)`.
    pub atlas: [u32; 4],
    pub matrices: [[[f32; 4]; 4]; MAX_ATLAS_DIRECTIONS],
}

impl Default for LightingOcclusionUniforms {
    fn default() -> Self {
        Self {
            view_proj: identity_mat4(),
            params: [0.002, 0.0, 1.0 / DEFAULT_SHADOW_MAP_SIZE as f32, 1.0],
            atlas: [0, 1, OCCLUSION_MODE_DISABLED, DEFAULT_SHADOW_MAP_SIZE],
            matrices: [identity_mat4(); MAX_ATLAS_DIRECTIONS],
        }
    }
}

impl LightingOcclusionUniforms {
    pub fn disabled(size: u32) -> Self {
        Self {
            params: [0.002, 0.0, 1.0 / size.max(1) as f32, 1.0],
            atlas: [0, 1, OCCLUSION_MODE_DISABLED, size.max(1)],
            ..Self::default()
        }
    }

    pub fn directional(
        view_proj: [[f32; 4]; 4],
        bias: f32,
        intensity: f32,
        map_size: u32,
        pcf_radius: f32,
    ) -> Self {
        let mut uniforms = Self {
            view_proj,
            params: [
                bias,
                intensity.clamp(0.0, 1.0),
                1.0 / map_size.max(1) as f32,
                pcf_radius,
            ],
            atlas: [1, 1, OCCLUSION_MODE_DIRECTIONAL, map_size.max(1)],
            ..Self::default()
        };
        uniforms.matrices[0] = view_proj;
        uniforms
    }

    pub fn atlas_ao(
        matrices: &[[[f32; 4]; 4]],
        grid: u32,
        tile_size: u32,
        bias: f32,
        intensity: f32,
    ) -> Self {
        let mut uniforms = Self {
            params: [
                bias,
                intensity.clamp(0.0, 2.0),
                1.0 / (grid.max(1) * tile_size.max(1)) as f32,
                1.0,
            ],
            atlas: [
                matrices.len().min(MAX_ATLAS_DIRECTIONS) as u32,
                grid.max(1),
                OCCLUSION_MODE_ATLAS_AO,
                tile_size.max(1),
            ],
            ..Self::default()
        };
        for (dst, src) in uniforms.matrices.iter_mut().zip(matrices.iter()) {
            *dst = *src;
        }
        uniforms
    }
}

pub struct LightingOcclusion {
    pub bind_group_layout: wgpu::BindGroupLayout,
    pub bind_group: wgpu::BindGroup,
    pub depth_pass_bind_group: wgpu::BindGroup,
    pub uniform_buffer: wgpu::Buffer,
    sampler: wgpu::Sampler,
    dummy_depth_texture: wgpu::Texture,
}

impl LightingOcclusion {
    pub fn new(device: &wgpu::Device) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("patinae.lighting_occlusion.layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<
                            LightingOcclusionUniforms,
                        >() as u64),
                    },
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("patinae.lighting_occlusion.sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("patinae.lighting_occlusion.uniforms"),
            contents: bytemuck::bytes_of(&LightingOcclusionUniforms::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let (dummy_depth_texture, dummy_depth_view) = create_depth_texture(device, 1);
        let bind_group = create_bind_group(
            device,
            &bind_group_layout,
            &dummy_depth_view,
            &sampler,
            &uniform_buffer,
        );
        let depth_pass_bind_group = create_bind_group(
            device,
            &bind_group_layout,
            &dummy_depth_view,
            &sampler,
            &uniform_buffer,
        );
        Self {
            bind_group_layout,
            bind_group,
            depth_pass_bind_group,
            uniform_buffer,
            sampler,
            dummy_depth_texture,
        }
    }

    pub fn set_depth_view(&mut self, device: &wgpu::Device, depth_view: &wgpu::TextureView) {
        self.bind_group = create_bind_group(
            device,
            &self.bind_group_layout,
            depth_view,
            &self.sampler,
            &self.uniform_buffer,
        );
    }

    pub fn upload_uniforms(&self, queue: &wgpu::Queue, uniforms: &LightingOcclusionUniforms) {
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(uniforms));
    }

    pub fn reset_disabled(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, size: u32) {
        let view = self
            .dummy_depth_texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.set_depth_view(device, &view);
        self.upload_uniforms(queue, &LightingOcclusionUniforms::disabled(size));
    }

    pub(crate) fn memory_usage(&self) -> GpuMemoryUsage {
        let mut usage = GpuMemoryUsage::default();
        usage.add(buffer_usage(&self.uniform_buffer));
        usage.add(GpuMemoryUsage::allocation(estimate_texture_2d_bytes(
            1,
            1,
            OCCLUSION_DEPTH_FORMAT,
        )));
        usage
    }
}

pub fn create_depth_texture(
    device: &wgpu::Device,
    size: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("patinae.lighting_occlusion.depth"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: OCCLUSION_DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

fn create_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    depth_view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    uniform_buffer: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("patinae.lighting_occlusion.bind_group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(depth_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: uniform_buffer.as_entire_binding(),
            },
        ],
    })
}

pub(crate) const fn identity_mat4() -> [[f32; 4]; 4] {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lighting_occlusion_uniforms_fit_downlevel_binding_limit() {
        let size = std::mem::size_of::<LightingOcclusionUniforms>() as u64;
        let limit = wgpu::Limits::downlevel_defaults().max_uniform_buffer_binding_size;

        assert!(
            size <= limit,
            "lighting occlusion uniform is {size} bytes, exceeding the downlevel {limit}-byte binding limit"
        );
    }

    #[test]
    fn lighting_shader_matches_atlas_direction_capacity() {
        let shader = include_str!("../shaders/common/lighting.wgsl");
        let declaration = format!("const MAX_ATLAS_DIRECTIONS: u32 = {MAX_ATLAS_DIRECTIONS}u;");

        assert!(
            shader.contains(&declaration),
            "lighting WGSL must keep MAX_ATLAS_DIRECTIONS in sync with Rust"
        );
    }
}
