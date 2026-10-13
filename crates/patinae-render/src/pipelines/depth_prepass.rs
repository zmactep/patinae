//! Depth pre-pass pipelines (FrameGraph slot `3`).
//!
//! For every geometric representation we compile a sister pipeline that
//! shares the rep's vertex stage but binds a stripped fragment entry point
//! (`fs_depth`) which only does the visibility + alpha gate, then discards
//! when `alpha < 0.999`. The pipeline has no color targets and writes
//! depth (`depth_write_enabled = true, depth_compare = Less`). The
//! resulting depth shield is consumed by the WBOIT translucent pass via
//! `depth_compare = LessEqual`, occluding translucent fragments behind
//! opaque ones. The pre-pass is the explicit depth-writing path for the
//! otherwise translucent representation pipeline.

use crate::context::RenderContext;
use crate::frame::DEPTH_FORMAT;
use crate::pipelines::cartoon::CartoonParamsLayout;
use crate::pipelines::dot::DotParamsLayout;
use crate::pipelines::ellipsoid::EllipsoidParamsLayout;
use crate::pipelines::mesh::MeshParamsLayout;
use crate::pipelines::sphere::SphereParamsLayout;
use crate::pipelines::stick::StickParamsLayout;
use crate::pipelines::surface::SurfaceParamsLayout;
use crate::representations::dot::DotAtomInstance;
use crate::representations::ellipsoid::EllipsoidInstance;
use crate::representations::line::LineInstance;
use crate::representations::mesh::StdVertex;
use crate::representations::sphere::SphereInstance;
use crate::representations::stick::StickInstance;
use crate::scene_store::SceneStoreLayout;
use crate::shader_source::{
    self, CARTOON_WGSL, DOT_WGSL, ELLIPSOID_WGSL, LINE_WGSL, MESH_WGSL, SPHERE_WGSL, STICK_WGSL,
    SURFACE_WGSL,
};

/// Borrow-bundle of every bind-group layout `DepthPrepassPipelines::new`
/// needs. Avoids a 7-arg constructor; each field is just a reference into
/// resources owned by `GeometryRuntime`.
pub struct DepthPrepassLayouts<'a> {
    pub scene: &'a SceneStoreLayout,
    pub sphere: &'a SphereParamsLayout,
    pub stick: &'a StickParamsLayout,
    pub ellipsoid: &'a EllipsoidParamsLayout,
    pub cartoon: &'a CartoonParamsLayout,
    pub surface: &'a SurfaceParamsLayout,
    pub mesh: &'a MeshParamsLayout,
    pub dot: &'a DotParamsLayout,
}

pub struct DepthPrepassPipelines {
    pub sphere: wgpu::RenderPipeline,
    pub stick: wgpu::RenderPipeline,
    pub line: wgpu::RenderPipeline,
    pub dot: wgpu::RenderPipeline,
    pub mesh: wgpu::RenderPipeline,
    pub ellipsoid: wgpu::RenderPipeline,
    pub cartoon: wgpu::RenderPipeline,
    pub surface: wgpu::RenderPipeline,
}

impl DepthPrepassPipelines {
    pub fn new(ctx: &RenderContext, layouts: &DepthPrepassLayouts<'_>) -> Self {
        let device = &ctx.device;
        let frame_bgl = &ctx.frame.bind_group_layout;
        let light_bgl = &ctx.lighting.bind_group_layout;
        let scene_bgl = &layouts.scene.bind_group_layout;

        // Sphere migrated to SceneStore (group 2) + per-rep params (group 3).
        let sphere_pl = make_layout(
            device,
            "patinae.depth_prepass.sphere.layout",
            &[
                frame_bgl,
                light_bgl,
                scene_bgl,
                &layouts.sphere.bind_group_layout,
            ],
        );
        let sphere = build_pipeline(
            device,
            &sphere_pl,
            "patinae.depth_prepass.sphere",
            SPHERE_WGSL,
            SphereInstance::vertex_layout(),
            wgpu::PrimitiveTopology::TriangleList,
        );

        // Stick migrated to SceneStore + per-rep params (group 3) too.
        let stick_pl = make_layout(
            device,
            "patinae.depth_prepass.stick.layout",
            &[
                frame_bgl,
                light_bgl,
                scene_bgl,
                &layouts.stick.bind_group_layout,
            ],
        );
        let stick = build_pipeline(
            device,
            &stick_pl,
            "patinae.depth_prepass.stick",
            STICK_WGSL,
            StickInstance::vertex_layout(),
            wgpu::PrimitiveTopology::TriangleList,
        );

        // Line migrated to SceneStore; no group 3 (lines are always opaque).
        // Lines are too thin to produce useful occlusion. Run them through
        // the prepass anyway so the pipeline existence is uniform; an empty
        // rasterisation (zero-pixel coverage at the typical 1 px line width)
        // costs nothing and keeps the shield honest if anyone bumps line
        // thickness later.
        let line_pl = make_layout(
            device,
            "patinae.depth_prepass.line.layout",
            &[frame_bgl, light_bgl, scene_bgl],
        );
        let line = build_pipeline(
            device,
            &line_pl,
            "patinae.depth_prepass.line",
            LINE_WGSL,
            LineInstance::vertex_layout(),
            wgpu::PrimitiveTopology::LineList,
        );

        // Dot uses group 3 for procedural sample-count params + direction LUT.
        let dot_pl = make_layout(
            device,
            "patinae.depth_prepass.dot.layout",
            &[
                frame_bgl,
                light_bgl,
                scene_bgl,
                &layouts.dot.bind_group_layout,
            ],
        );
        let dot = build_pipeline(
            device,
            &dot_pl,
            "patinae.depth_prepass.dot",
            DOT_WGSL,
            DotAtomInstance::vertex_layout(),
            wgpu::PrimitiveTopology::TriangleList,
        );

        // Mesh = atom-driven wireframe of molecular surface; SceneStore (group 2)
        // + mesh params (group 3); LineList topology emitted by surface_mc compute.
        let mesh_pl = make_layout(
            device,
            "patinae.depth_prepass.mesh.layout",
            &[
                frame_bgl,
                light_bgl,
                scene_bgl,
                &layouts.mesh.bind_group_layout,
            ],
        );
        let mesh = build_pipeline(
            device,
            &mesh_pl,
            "patinae.depth_prepass.mesh",
            MESH_WGSL,
            StdVertex::vertex_layout(),
            wgpu::PrimitiveTopology::LineList,
        );

        // Ellipsoid migrated to SceneStore + per-rep params (group 3).
        let ellipsoid_pl = make_layout(
            device,
            "patinae.depth_prepass.ellipsoid.layout",
            &[
                frame_bgl,
                light_bgl,
                scene_bgl,
                &layouts.ellipsoid.bind_group_layout,
            ],
        );
        let ellipsoid = build_pipeline(
            device,
            &ellipsoid_pl,
            "patinae.depth_prepass.ellipsoid",
            ELLIPSOID_WGSL,
            EllipsoidInstance::vertex_layout(),
            wgpu::PrimitiveTopology::TriangleList,
        );

        // Cartoon migrated to SceneStore (group 2) + cartoon params (group 3).
        let cartoon_pl = make_layout(
            device,
            "patinae.depth_prepass.cartoon.layout",
            &[
                frame_bgl,
                light_bgl,
                scene_bgl,
                &layouts.cartoon.bind_group_layout,
            ],
        );
        let cartoon = build_pipeline(
            device,
            &cartoon_pl,
            "patinae.depth_prepass.cartoon",
            CARTOON_WGSL,
            StdVertex::vertex_layout(),
            wgpu::PrimitiveTopology::TriangleList,
        );

        // Surface migrated to SceneStore (group 2) + surface params (group 3).
        let surface_pl = make_layout(
            device,
            "patinae.depth_prepass.surface.layout",
            &[
                frame_bgl,
                light_bgl,
                scene_bgl,
                &layouts.surface.bind_group_layout,
            ],
        );
        let surface = build_pipeline(
            device,
            &surface_pl,
            "patinae.depth_prepass.surface",
            SURFACE_WGSL,
            StdVertex::vertex_layout(),
            wgpu::PrimitiveTopology::TriangleList,
        );

        Self {
            sphere,
            stick,
            line,
            dot,
            mesh,
            ellipsoid,
            cartoon,
            surface,
        }
    }
}

fn make_layout(
    device: &wgpu::Device,
    label: &str,
    bgls: &[&wgpu::BindGroupLayout],
) -> wgpu::PipelineLayout {
    let bind_group_layouts: Vec<_> = bgls.iter().map(|layout| Some(*layout)).collect();
    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &bind_group_layouts,
        immediate_size: 0,
    })
}

fn build_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    label: &str,
    wgsl: &str,
    vertex_layout: wgpu::VertexBufferLayout<'static>,
    topology: wgpu::PrimitiveTopology,
) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(shader_source::expand(wgsl).into()),
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[Some(vertex_layout)],
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
            entry_point: Some("fs_depth"),
            compilation_options: Default::default(),
            // Empty `targets` — pre-pass writes only depth.
            targets: &[],
        }),
        multiview_mask: None,
        cache: None,
    })
}
