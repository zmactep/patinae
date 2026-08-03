//! Screen-space pipeline for reusable annotation strokes.

use crate::context::RenderContext;
use crate::frame::DEPTH_FORMAT;
use crate::render_input::StrokeSegment;
use crate::shader_source;

/// Alpha-blended stroke pipeline with depth testing and no picking output.
pub struct StrokePipeline {
    pub pipeline: wgpu::RenderPipeline,
}

impl StrokePipeline {
    pub fn new(ctx: &RenderContext) -> Self {
        let device = &ctx.device;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("patinae.stroke.shader"),
            source: wgpu::ShaderSource::Wgsl(
                shader_source::expand(shader_source::STROKE_WGSL).into(),
            ),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("patinae.stroke.pipeline_layout"),
            bind_group_layouts: &[Some(&ctx.frame.bind_group_layout)],
            immediate_size: 0,
        });
        let color_targets = [Some(wgpu::ColorTargetState {
            format: ctx.color_format,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        })];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("patinae.stroke.pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[StrokeSegment::vertex_layout()],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &color_targets,
            }),
            multiview_mask: None,
            cache: None,
        });
        Self { pipeline }
    }
}
