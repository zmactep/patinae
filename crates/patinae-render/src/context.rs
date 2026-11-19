//! `RenderContext` — shared GPU resources and bind-group layouts.
//!
//! Every pipeline in this crate references the same four bind-group layouts,
//! cached here. The fixed layout assignment keeps shader interfaces stable
//! across representation pipelines.
//!
//! | Group | Contents                                                |
//! |-------|---------------------------------------------------------|
//! | 0     | `FrameUniforms`                                         |
//! | 1     | Lighting/occlusion depth + sampler + parameters         |
//! | 2     | `SceneStore` (atoms/colors/masks/selection, scene-wide) |
//! | 3     | Geometry (vertex pull / instance buffers)               |
//!
//! The crate accepts an externally-owned `Arc<Device>` / `Arc<Queue>` so it
//! can share a wgpu instance with Slint (or any other host). It NEVER
//! creates its own device.

use std::sync::Arc;

use wgpu::util::DeviceExt;

use crate::passes::lighting::LightingOcclusion;
use crate::uniforms::FrameUniforms;

/// Shared per-frame uniform resources (group 0).
pub struct FrameResources {
    pub bind_group_layout: wgpu::BindGroupLayout,
    pub bind_group: wgpu::BindGroup,
    pub buffer: wgpu::Buffer,
}

impl FrameResources {
    fn new(device: &wgpu::Device) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("patinae.frame_uniforms.layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX
                    | wgpu::ShaderStages::FRAGMENT
                    | wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(FrameUniforms::SIZE),
                },
                count: None,
            }],
        });

        Self::with_layout(device, &bind_group_layout)
    }

    pub(crate) fn with_layout(device: &wgpu::Device, layout: &wgpu::BindGroupLayout) -> Self {
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("patinae.frame_uniforms"),
            contents: bytemuck::bytes_of(&FrameUniforms::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("patinae.frame_uniforms.bind_group"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });

        Self {
            bind_group_layout: layout.clone(),
            bind_group,
            buffer,
        }
    }

    /// Upload a fresh `FrameUniforms` block to the GPU.
    pub fn upload(&self, queue: &wgpu::Queue, uniforms: &FrameUniforms) {
        queue.write_buffer(&self.buffer, 0, bytemuck::bytes_of(uniforms));
    }
}

/// Shared rendering context — borrowed by every pipeline in the crate.
pub struct RenderContext {
    pub device: Arc<wgpu::Device>,
    pub queue: Arc<wgpu::Queue>,
    /// Format of the final color target the host blits to (e.g. swap chain).
    pub color_format: wgpu::TextureFormat,
    pub frame: FrameResources,
    pub lighting: LightingOcclusion,
}

impl RenderContext {
    pub fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        color_format: wgpu::TextureFormat,
    ) -> Self {
        let frame = FrameResources::new(&device);
        let lighting = LightingOcclusion::new(&device);
        Self {
            device,
            queue,
            color_format,
            frame,
            lighting,
        }
    }

    pub fn upload_frame(&self, uniforms: &FrameUniforms) {
        self.frame.upload(&self.queue, uniforms);
    }
}
