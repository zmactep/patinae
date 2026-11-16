//! Native one-pixel and adjustable-width line color pipelines.
//! Wider lines use an eight-vertex cap/body strip per bond.
//!
//! Bind groups:
//! - 0: `FrameUniforms` (`RenderContext`)
//! - 1: shadow placeholder (`RenderContext`)
//! - 2: [`SceneStoreLayout`] — scene-wide LUTs + `obj` dynamic-uniform
//!
//! No group 3: lines have no per-rep transparency. The WBOIT path keeps
//! `depth_write = false`; the opaque fast-path writes depth.

use crate::context::RenderContext;
use crate::pipelines::build_draw_pair;
use crate::representations::line::{LineInstance, LINE_TOPOLOGY};
use crate::scene_store::SceneStoreLayout;
use crate::shader_source::{LINE_NATIVE_WGSL, LINE_WGSL};

pub struct LinePipeline {
    pub(crate) native: crate::pipelines::DrawPipelinePair,
    pub pipeline: wgpu::RenderPipeline,
    pub pipeline_opaque: wgpu::RenderPipeline,
}

impl LinePipeline {
    pub fn new(ctx: &RenderContext, scene_layout: &SceneStoreLayout) -> Self {
        let pair = build_draw_pair(
            ctx,
            "patinae.line",
            LINE_WGSL,
            &[
                &ctx.frame.bind_group_layout,
                &ctx.lighting.bind_group_layout,
                &scene_layout.bind_group_layout,
            ],
            LineInstance::vertex_layout(),
            LINE_TOPOLOGY,
        );
        let native = build_draw_pair(
            ctx,
            "patinae.line.native",
            LINE_NATIVE_WGSL,
            &[
                &ctx.frame.bind_group_layout,
                &ctx.lighting.bind_group_layout,
                &scene_layout.bind_group_layout,
            ],
            LineInstance::vertex_layout(),
            wgpu::PrimitiveTopology::LineList,
        );
        Self {
            native,
            pipeline: pair.translucent,
            pipeline_opaque: pair.opaque,
        }
    }
}
