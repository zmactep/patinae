//! Picking pass — `Rg32Uint` @ low-res, async readback.
//!
//! Owns:
//! - bind-group-2 layout for `PickingParams` (per-draw rep_kind|object_id)
//! - picking pipelines for drawable representations
//! - shared readback staging buffer
//!
//! Per-rep picking state (the `PickingParams` uniform buffer + bind group)
//! lives on the representation itself — see `SphereRep`.

use std::num::NonZeroU64;

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::context::RenderContext;
use crate::frame::{DEPTH_FORMAT, PICKING_FORMAT};
use crate::picking::{ObjectId, PackedId, PickHit, RepKind};
use crate::pipelines::dot::DotParamsLayout;
use crate::representations::dot::DotAtomInstance;
use crate::representations::ellipsoid::EllipsoidInstance;
use crate::representations::line::LineInstance;
use crate::representations::mesh::StdVertex;
use crate::representations::sphere::SphereInstance;
use crate::representations::stick::StickInstance;
use crate::scene_store::SceneStoreLayout;
use crate::shader_source::{
    self, DOT_PICKING_WGSL, ELLIPSOID_PICKING_WGSL, LINE_PICKING_WGSL, SPHERE_PICKING_WGSL,
    STD_VERTEX_PICKING_WGSL, STICK_PICKING_WGSL,
};

/// Mirrors `PickingParams` in `shaders/common/picking.wgsl` (16 B).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable, Default)]
pub struct PickingParams {
    /// `(rep_kind << 28) | (object_id << 16)` — pre-shifted on the CPU so the
    /// shader does no work beyond OR-ing the atom_id low 16 bits.
    pub rep_object: u32,
    pub _pad0: u32,
    pub _pad1: u32,
    pub _pad2: u32,
}

impl PickingParams {
    pub(crate) const SIZE: u64 = std::mem::size_of::<Self>() as u64;

    pub fn new(rep_kind: RepKind, object_id: ObjectId) -> Self {
        let kind = (rep_kind.as_raw() as u32) & 0xF;
        let obj = object_id.0 & 0xFFF;
        Self {
            rep_object: (kind << 28) | (obj << 16),
            ..Default::default()
        }
    }
}

pub struct PickingPass {
    pub params_layout: wgpu::BindGroupLayout,
    pub sphere_pipeline: wgpu::RenderPipeline,
    pub stick_pipeline: wgpu::RenderPipeline,
    pub line_pipeline: wgpu::RenderPipeline,
    pub dot_pipeline: wgpu::RenderPipeline,
    /// `StdVertex` triangle picking — shared by `RepKind::Surface`,
    /// `Cartoon`, `Ribbon`. Mesh has its own LineList sibling below.
    pub std_vertex_pipeline: wgpu::RenderPipeline,
    /// `StdVertex` LineList picking — used by `RepKind::Mesh` (atom-driven
    /// wireframe surface). Same shader as `std_vertex_pipeline`, only
    /// topology differs. Picking texture is at `PICKING_SCALE` (0.5×) so
    /// 1-pixel lines effectively cover ~0.5 px — clients should apply
    /// tolerance at hit-test time.
    pub std_vertex_line_pipeline: wgpu::RenderPipeline,
    pub ellipsoid_pipeline: wgpu::RenderPipeline,
}

impl PickingPass {
    pub fn new(
        ctx: &RenderContext,
        scene_layout: &SceneStoreLayout,
        dot_layout: &DotParamsLayout,
    ) -> Self {
        let device = &ctx.device;

        let params_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("patinae.picking.params.layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(std::mem::size_of::<PickingParams>() as u64),
                },
                count: None,
            }],
        });

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("patinae.picking.sphere.shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source::expand(SPHERE_PICKING_WGSL).into()),
        });

        let sphere_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("patinae.picking.sphere.pipeline_layout"),
            bind_group_layouts: &[
                Some(&ctx.frame.bind_group_layout),
                Some(&ctx.lighting.bind_group_layout),
                Some(&params_layout),
                Some(&scene_layout.bind_group_layout),
            ],
            immediate_size: 0,
        });

        let sphere_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("patinae.picking.sphere.pipeline"),
            layout: Some(&sphere_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[SphereInstance::vertex_layout()],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(picking_depth_state()),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: PICKING_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::RED | wgpu::ColorWrites::GREEN,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("patinae.picking.pipeline_layout"),
            bind_group_layouts: &[
                Some(&ctx.frame.bind_group_layout),
                Some(&ctx.lighting.bind_group_layout),
                Some(&params_layout),
                Some(&scene_layout.bind_group_layout),
            ],
            immediate_size: 0,
        });

        let stick_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("patinae.picking.stick.shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source::expand(STICK_PICKING_WGSL).into()),
        });
        let stick_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("patinae.picking.stick.pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &stick_module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[StickInstance::vertex_layout()],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(picking_depth_state()),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &stick_module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: PICKING_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::RED | wgpu::ColorWrites::GREEN,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let line_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("patinae.picking.line.shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source::expand(LINE_PICKING_WGSL).into()),
        });
        let line_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("patinae.picking.line.pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &line_module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[LineInstance::vertex_layout()],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(picking_depth_state()),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &line_module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: PICKING_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::RED | wgpu::ColorWrites::GREEN,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let dot_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("patinae.picking.dot.shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source::expand(DOT_PICKING_WGSL).into()),
        });
        let dot_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("patinae.picking.dot.pipeline_layout"),
            bind_group_layouts: &[
                Some(&ctx.frame.bind_group_layout),
                Some(&scene_layout.bind_group_layout),
                Some(&params_layout),
                Some(&dot_layout.bind_group_layout),
            ],
            immediate_size: 0,
        });
        let dot_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("patinae.picking.dot.pipeline"),
            layout: Some(&dot_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &dot_module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[DotAtomInstance::vertex_layout()],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(picking_depth_state()),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &dot_module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: PICKING_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::RED | wgpu::ColorWrites::GREEN,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let std_vertex_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("patinae.picking.std_vertex.shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source::expand(STD_VERTEX_PICKING_WGSL).into()),
        });
        let std_vertex_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("patinae.picking.std_vertex.pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &std_vertex_module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[StdVertex::vertex_layout()],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(picking_depth_state()),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &std_vertex_module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: PICKING_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::RED | wgpu::ColorWrites::GREEN,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        // Wireframe-mesh picking — same shader, LineList topology.
        let std_vertex_line_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("patinae.picking.std_vertex_line.pipeline"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &std_vertex_module,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[StdVertex::vertex_layout()],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::LineList,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(picking_depth_state()),
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &std_vertex_module,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: PICKING_FORMAT,
                        blend: None,
                        write_mask: wgpu::ColorWrites::RED | wgpu::ColorWrites::GREEN,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });

        let ellipsoid_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("patinae.picking.ellipsoid.shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source::expand(ELLIPSOID_PICKING_WGSL).into()),
        });
        let ellipsoid_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("patinae.picking.ellipsoid.pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &ellipsoid_module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[EllipsoidInstance::vertex_layout()],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(picking_depth_state()),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &ellipsoid_module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: PICKING_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::RED | wgpu::ColorWrites::GREEN,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        Self {
            params_layout,
            sphere_pipeline,
            stick_pipeline,
            line_pipeline,
            dot_pipeline,
            std_vertex_pipeline,
            std_vertex_line_pipeline,
            ellipsoid_pipeline,
        }
    }

    /// Allocate a per-rep picking params buffer + bind group.
    pub fn make_params(
        &self,
        device: &wgpu::Device,
        params: PickingParams,
    ) -> (wgpu::Buffer, wgpu::BindGroup) {
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("patinae.picking.params"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("patinae.picking.params.bind_group"),
            layout: &self.params_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });
        (buffer, bind_group)
    }
}

/// Shared depth state for every picking pipeline. Less + write enabled — the
/// closest writer wins. Format must match `FrameTargets::picking_depth`.
fn picking_depth_state() -> wgpu::DepthStencilState {
    wgpu::DepthStencilState {
        format: DEPTH_FORMAT,
        depth_write_enabled: Some(true),
        depth_compare: Some(wgpu::CompareFunction::Less),
        stencil: wgpu::StencilState::default(),
        bias: wgpu::DepthBiasState::default(),
    }
}

/// Decode an `Rg32Uint` pair from the picking texture into a `PickHit`.
pub fn decode_pixel(r: u32, g: u32) -> Option<PickHit> {
    let (rep_kind, object_id, atom_id) = PackedId { r, g }.unpack()?;
    Some(PickHit {
        instance: PackedId { r, g }.instance(),
        rep_kind,
        object_id,
        atom_id,
    })
}
