//! Pipeline construction helpers.
//!
//! Two color-target shapes are supported:
//!
//! - **Opaque fast-path** — single host-format target with `depth_write =
//!   true` and no blending. Reps default to this when their material LUT
//!   contains no per-atom α < 1 and no global transparency setting. This
//!   skips the WBOIT accum/reveal bandwidth and the composite pass for
//!   the common all-opaque scene.
//! - **WBOIT translucent** — twin (accum, reveal) targets with the
//!   blend modes documented below. Activated per-rep when transparency
//!   is present; final composite pass alpha-blends the result on top of
//!   the opaque content already written to the host target.
//!
//! Use `opaque_color_target(host_format)` for the opaque path and
//! `translucent_color_targets()` for the WBOIT path.

pub mod cartoon;
pub mod depth_prepass;
pub mod dot;
pub mod ellipsoid;
pub mod line;
pub mod map;
pub mod mesh;
pub mod sphere;
pub mod stick;
pub mod stroke;
pub mod surface;
pub mod wboit_composite;

use crate::context::RenderContext;
use crate::frame::{ACCUM_FORMAT, DEPTH_FORMAT, REVEAL_FORMAT};
use crate::shader_source;

/// Twin draw pipelines emitted by [`build_draw_pair`]. Every geometric
/// representation owns one of these; opaque / translucent reps choose at
/// draw time which to bind.
pub(crate) struct DrawPipelinePair {
    /// WBOIT pipeline — used when the rep contains any per-atom α < 1.
    /// Writes (accum, reveal); depth is read-only against the prepass.
    pub translucent: wgpu::RenderPipeline,
    /// Opaque fast-path — single host-format target, writes depth.
    /// Selected when every per-atom alpha is fully opaque.
    pub opaque: wgpu::RenderPipeline,
}

/// Shared scaffolding for every per-rep draw pipeline. Emits one shader
/// module, one [`wgpu::PipelineLayout`] (over the supplied BGLs), and a
/// translucent / opaque pair that differ only in the four fields needed
/// for the depth-writing opaque path: `depth_write_enabled`, `depth_compare`,
/// `fragment.entry_point` (`fs_main` vs `fs_opaque`) and the colour
/// targets.
///
/// `label_prefix` follows the existing `patinae.<rep>` convention; the
/// downstream labels are `<prefix>.shader`, `<prefix>.pipeline_layout`,
/// `<prefix>.pipeline`, `<prefix>.pipeline_opaque` — kept bit-identical
/// to the pre-extraction call sites so wgpu state caches don't churn.
pub(crate) fn build_draw_pair(
    ctx: &RenderContext,
    label_prefix: &str,
    wgsl: &str,
    bgls: &[&wgpu::BindGroupLayout],
    vertex_layout: wgpu::VertexBufferLayout<'static>,
    topology: wgpu::PrimitiveTopology,
) -> DrawPipelinePair {
    let device = &ctx.device;

    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(&format!("{label_prefix}.shader")),
        source: wgpu::ShaderSource::Wgsl(shader_source::expand(wgsl).into()),
    });
    let bind_group_layouts: Vec<_> = bgls.iter().map(|layout| Some(*layout)).collect();

    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(&format!("{label_prefix}.pipeline_layout")),
        bind_group_layouts: &bind_group_layouts,
        immediate_size: 0,
    });

    let vertex_buffers = std::slice::from_ref(&vertex_layout);

    let translucent_targets = translucent_color_targets();
    let translucent = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(&format!("{label_prefix}.pipeline")),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: vertex_buffers,
        },
        primitive: wgpu::PrimitiveState {
            topology,
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
            targets: &translucent_targets,
        }),
        multiview_mask: None,
        cache: None,
    });

    let opaque_targets = opaque_color_target(ctx.color_format);
    let opaque = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(&format!("{label_prefix}.pipeline_opaque")),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: vertex_buffers,
        },
        primitive: wgpu::PrimitiveState {
            topology,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_opaque"),
            compilation_options: Default::default(),
            targets: &opaque_targets,
        }),
        multiview_mask: None,
        cache: None,
    });

    DrawPipelinePair {
        translucent,
        opaque,
    }
}

/// Single-target overlay pipeline. Used by visual surface samples such as
/// dots: depth-tested against solid geometry, no WBOIT targets, no depth write.
pub(crate) fn build_fast_overlay_pipeline(
    ctx: &RenderContext,
    label_prefix: &str,
    wgsl: &str,
    bgls: &[&wgpu::BindGroupLayout],
    vertex_layout: wgpu::VertexBufferLayout<'static>,
    topology: wgpu::PrimitiveTopology,
) -> wgpu::RenderPipeline {
    let device = &ctx.device;
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(&format!("{label_prefix}.shader")),
        source: wgpu::ShaderSource::Wgsl(shader_source::expand(wgsl).into()),
    });
    let bind_group_layouts: Vec<_> = bgls.iter().map(|layout| Some(*layout)).collect();
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(&format!("{label_prefix}.pipeline_layout")),
        bind_group_layouts: &bind_group_layouts,
        immediate_size: 0,
    });
    let vertex_buffers = [vertex_layout];
    let color_targets = opaque_color_target(ctx.color_format);
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(&format!("{label_prefix}.pipeline")),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &vertex_buffers,
        },
        primitive: wgpu::PrimitiveState {
            topology,
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
            entry_point: Some("fs_opaque"),
            compilation_options: Default::default(),
            targets: &color_targets,
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// Single host-format target with no blending and full color/depth write.
/// Use for the opaque pass — direct write into the swap-chain texture.
pub fn opaque_color_target(
    host_format: wgpu::TextureFormat,
) -> [Option<wgpu::ColorTargetState>; 1] {
    [Some(wgpu::ColorTargetState {
        format: host_format,
        blend: None,
        write_mask: wgpu::ColorWrites::ALL,
    })]
}

/// Color-target descriptors for the translucent pass. Order matters and must
/// match `TranslucentOut` in `shaders/common/wboit.wgsl`:
///   @location(0) = accum
///   @location(1) = reveal
pub fn translucent_color_targets() -> [Option<wgpu::ColorTargetState>; 2] {
    [
        Some(wgpu::ColorTargetState {
            format: ACCUM_FORMAT,
            blend: Some(wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
            }),
            write_mask: wgpu::ColorWrites::ALL,
        }),
        Some(wgpu::ColorTargetState {
            format: REVEAL_FORMAT,
            blend: Some(wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::Zero,
                    dst_factor: wgpu::BlendFactor::OneMinusSrc,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::Zero,
                    dst_factor: wgpu::BlendFactor::OneMinusSrc,
                    operation: wgpu::BlendOperation::Add,
                },
            }),
            write_mask: wgpu::ColorWrites::RED,
        }),
    ]
}
