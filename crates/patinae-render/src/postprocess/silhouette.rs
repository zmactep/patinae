//! Silhouette pass — full-screen visible-id edge detect, alpha-blended onto
//! the host color target. Skipped when the host disables silhouettes.
//!
//! Algorithm: 4-tap kernel over the overlay id texture. An edge pixel is one
//! where the centre is non-empty and at least one of its cardinal neighbours
//! is empty or belongs to a different (rep_kind, object_id) pair. Reads the
//! visible-id texture populated earlier in the frame; does not depend on the
//! hit-test picking mode.
//!
//! Why ids, not depth: the translucent geometric path keeps
//! `depth_write_enabled = false`; the depth pre-pass is the only depth writer
//! and is optional. A depth-driven silhouette would either need that pre-pass
//! enabled for all silhouette frames or violate the invariant. Id-driven
//! silhouette keeps the outline source aligned with representation ids, which
//! also prevents edges between triangles of the same object representation.

use bytemuck::{Pod, Zeroable};

use crate::context::RenderContext;
use crate::shader_source::{self, SILHOUETTE_WGSL};

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct SilhouetteParams {
    pub color: [f32; 4],
    /// Outline thickness in overlay texture pixels.
    pub thickness: f32,
    pub(crate) padding: [f32; 3],
}

pub struct SilhouettePass {
    pub pipeline: wgpu::RenderPipeline,
    pub bind_group_layout: wgpu::BindGroupLayout,
    pub uniform_buffer: wgpu::Buffer,
}

impl SilhouettePass {
    pub fn new(ctx: &RenderContext) -> Self {
        let device = &ctx.device;

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("patinae.silhouette.shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source::expand(SILHOUETTE_WGSL).into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("patinae.silhouette.layout"),
            entries: &[
                // Overlay id texture (Rg32Uint), refreshed before this pass
                // when silhouettes are enabled.
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("patinae.silhouette.pipeline_layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("patinae.silhouette.pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: ctx.color_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("patinae.silhouette.params"),
            size: std::mem::size_of::<SilhouetteParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            bind_group_layout,
            uniform_buffer,
        }
    }

    pub fn make_bind_group(
        &self,
        device: &wgpu::Device,
        overlay_id: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("patinae.silhouette.bind_group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(overlay_id),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.uniform_buffer.as_entire_binding(),
                },
            ],
        })
    }

    pub fn upload_params(&self, queue: &wgpu::Queue, params: SilhouetteParams) {
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&params));
    }

    pub fn record(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        bind_group: &wgpu::BindGroup,
        timestamp_writes: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("patinae.silhouette_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}
