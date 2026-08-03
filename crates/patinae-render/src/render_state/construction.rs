use std::collections::HashMap;
use std::sync::Arc;

use super::state::*;
use crate::compute::cartoon_extrude::CartoonExtrudeCompute;
use crate::compute::cull::CullPipeline;
use crate::compute::dot_build::DotBuildPipeline;
use crate::compute::ellipsoid_build::EllipsoidBuildPipeline;
use crate::compute::line_build::LineBuildPipeline;
use crate::compute::sphere_build::SphereBuildPipeline;
use crate::compute::sphere_lod_count::SphereLodCountPipeline;
use crate::compute::ssao::{SsaoBlur, SsaoCompute, SsaoResources};
use crate::compute::stick_build::StickBuildPipeline;
use crate::compute::stick_lod_count::StickLodCountPipeline;
use crate::compute::surface_density::SurfaceDensityCompute;
use crate::compute::surface_mc::SurfaceMcCompute;
use crate::compute::surface_ses_morph::SurfaceSesMorphCompute;
use crate::compute::surface_vdw_sdf::SurfaceVdwSdfCompute;
use crate::context::RenderContext;
use crate::frame::FrameTargets;
use crate::passes::atlas_ao::AtlasAoPass;
use crate::passes::lighting::DEFAULT_SHADOW_MAP_SIZE;
use crate::passes::selection_dots::{uses_selection_dots_fallback, SelectionDotsPass};
use crate::passes::shadow::DirectionalShadowPass;
use crate::picking::pass::PickingPass;
use crate::picking::readback::PickingReadback;
use crate::picking::reproject::PickingReproject;
use crate::picking::{PickingMode, RenderConfig};
use crate::pipelines::cartoon::{CartoonParamsLayout, CartoonPipeline};
use crate::pipelines::depth_prepass::{DepthPrepassLayouts, DepthPrepassPipelines};
use crate::pipelines::dot::{DotParamsLayout, DotPipeline};
use crate::pipelines::ellipsoid::{EllipsoidParamsLayout, EllipsoidPipeline};
use crate::pipelines::line::LinePipeline;
use crate::pipelines::map::{MapParamsLayout, MapPipeline};
use crate::pipelines::mesh::{MeshParamsLayout, MeshPipeline};
use crate::pipelines::sphere::{SphereParamsLayout, SpherePipeline};
use crate::pipelines::stick::{StickParamsLayout, StickPipeline};
use crate::pipelines::stroke::StrokePipeline;
use crate::pipelines::surface::{SurfaceParamsLayout, SurfacePipeline};
use crate::pipelines::wboit_composite::WboitComposite;
use crate::postprocess::fxaa::FxaaPass;
use crate::postprocess::marking::MarkingPass;
use crate::postprocess::ssao_compose::SsaoComposePass;
use crate::render_input::SceneLod;
use crate::scene_store::{SceneStore, SceneStoreLayout};
#[cfg(feature = "stats")]
use crate::stats::FrameStatsCollector;
use crate::uniforms::FrameUniforms;

use super::picking_budget::uses_lazy_manual_picking;

impl RenderState {
    pub fn new(
        device: Arc<wgpu::Device>,

        queue: Arc<wgpu::Queue>,

        color_format: wgpu::TextureFormat,

        viewport: (u32, u32),
    ) -> Self {
        Self::with_config(
            device,
            queue,
            color_format,
            viewport,
            RenderConfig::default(),
        )
    }

    /// Construct with an explicit [`RenderConfig`]. Hosts that want no
    /// hit-test readback resources can pass `PickingMode::Disabled` while
    /// keeping visual overlays controlled separately.
    pub fn with_config(
        device: Arc<wgpu::Device>,

        queue: Arc<wgpu::Queue>,

        color_format: wgpu::TextureFormat,

        viewport: (u32, u32),

        config: RenderConfig,
    ) -> Self {
        let memory_policy = config.memory;
        let mut memory_warnings = Vec::new();
        let policy_allows_picking = memory_policy.picking.hit_test_enabled;
        if config.picking != PickingMode::Disabled && !policy_allows_picking {
            let message = format!(
                "Hit-test picking disabled by render memory profile {}.",
                memory_policy.profile
            );
            log::warn!("patinae-render: {message}");
            memory_warnings.push(message);
        }
        if config.selection_overlay && !memory_policy.overlays.selection_enabled {
            if uses_selection_dots_fallback(memory_policy) {
                let message = format!(
                    "Full selection and hover overlays disabled by render memory profile {}; using selected atom dots.",
                    memory_policy.profile
                );
                log::warn!("patinae-render: {message}");
                memory_warnings.push(message);
            } else {
                let message = format!(
                    "Selection and hover overlays disabled by render memory profile {}.",
                    memory_policy.profile
                );
                log::warn!("patinae-render: {message}");
                memory_warnings.push(message);
            }
        }
        let picking_mode = if policy_allows_picking {
            config.picking
        } else {
            PickingMode::Disabled
        };
        let picking_mode = if picking_mode == PickingMode::Reprojected
            && !memory_policy.picking.reprojection_enabled
        {
            let message = format!(
                "Picking reprojection disabled by render memory profile {}; using full-record hit-test picking.",
                memory_policy.profile
            );
            log::warn!("patinae-render: {message}");
            memory_warnings.push(message);
            PickingMode::FullRecord
        } else {
            picking_mode
        };
        let lazy_manual_picking = uses_lazy_manual_picking(memory_policy);

        let with_picking = picking_mode != PickingMode::Disabled && !lazy_manual_picking;

        let ctx = RenderContext::new(device, queue, color_format);

        let targets = FrameTargets::new_with_policy(
            &ctx.device,
            viewport.0,
            viewport.1,
            with_picking,
            ctx.color_format,
            memory_policy.frame_targets,
            memory_policy.picking.scale,
        );

        let mut uniforms = FrameUniforms::default();

        uniforms.set_viewport(viewport.0, viewport.1);

        ctx.upload_frame(&uniforms);

        let scene_layout = SceneStoreLayout::new(&ctx.device);

        let sphere_params_layout = SphereParamsLayout::new(&ctx.device);

        let stick_params_layout = StickParamsLayout::new(&ctx.device);

        let ellipsoid_params_layout = EllipsoidParamsLayout::new(&ctx.device);

        let cartoon_params_layout = CartoonParamsLayout::new(&ctx.device);

        let surface_params_layout = SurfaceParamsLayout::new(&ctx.device);

        let dot_params_layout = DotParamsLayout::new(&ctx.device);

        let scene_store = SceneStore::new();

        let sphere_compute = SphereBuildPipeline::new(&ctx.device, &scene_layout);

        let sphere_lod_count = SphereLodCountPipeline::new(&ctx.device, &scene_layout);

        let stick_compute = StickBuildPipeline::new(&ctx.device, &scene_layout);

        let stick_lod_count = StickLodCountPipeline::new(&ctx.device, &scene_layout);

        let line_compute = LineBuildPipeline::new(&ctx.device, &scene_layout);

        let dot_compute = DotBuildPipeline::new(&ctx.device, &scene_layout);

        let ellipsoid_compute = EllipsoidBuildPipeline::new(&ctx.device, &scene_layout);

        let cartoon_compute = CartoonExtrudeCompute::new(&ctx.device);

        let surface_density_compute = SurfaceDensityCompute::new(&ctx.device, &scene_layout);

        let surface_vdw_sdf_compute = SurfaceVdwSdfCompute::new(&ctx.device, &scene_layout);

        let surface_ses_morph_compute = SurfaceSesMorphCompute::new(&ctx.device);

        let surface_mc_compute = SurfaceMcCompute::new(&ctx.device);

        let sphere_pipeline = SpherePipeline::new(&ctx, &scene_layout, &sphere_params_layout);

        let stick_pipeline = StickPipeline::new(&ctx, &scene_layout, &stick_params_layout);

        let line_pipeline = LinePipeline::new(&ctx, &scene_layout);

        let dot_pipeline = DotPipeline::new(&ctx, &scene_layout, &dot_params_layout);

        let map_params_layout = MapParamsLayout::new(&ctx.device);

        let map_pipeline = MapPipeline::new(&ctx, &map_params_layout);

        let stroke_pipeline = StrokePipeline::new(&ctx);

        let mesh_params_layout = MeshParamsLayout::new(&ctx.device);

        let mesh_pipeline = MeshPipeline::new(&ctx, &scene_layout, &mesh_params_layout);

        let ellipsoid_pipeline =
            EllipsoidPipeline::new(&ctx, &scene_layout, &ellipsoid_params_layout);

        let cartoon_pipeline = CartoonPipeline::new(&ctx, &scene_layout, &cartoon_params_layout);

        let surface_pipeline = SurfacePipeline::new(&ctx, &scene_layout, &surface_params_layout);

        let depth_prepass = DepthPrepassPipelines::new(
            &ctx,
            &DepthPrepassLayouts {
                scene: &scene_layout,
                sphere: &sphere_params_layout,
                stick: &stick_params_layout,
                ellipsoid: &ellipsoid_params_layout,
                cartoon: &cartoon_params_layout,
                surface: &surface_params_layout,
                mesh: &mesh_params_layout,
                dot: &dot_params_layout,
            },
        );

        let composite = WboitComposite::new(&ctx);

        // Hit-test picking owns only readback/reprojection resources. Visual
        // overlays use the same id-rendering pipelines but a separate
        // full-resolution id target, so they can stay independent from
        // `PickingMode`.
        let id_pass =
            with_picking.then(|| PickingPass::new(&ctx, &scene_layout, &dot_params_layout));

        let silhouette = None;

        let selection_overlay_enabled =
            config.selection_overlay && memory_policy.overlays.selection_enabled;
        let marking = selection_overlay_enabled.then(|| MarkingPass::new(&ctx));
        let selection_dots_enabled =
            config.selection_overlay && uses_selection_dots_fallback(memory_policy);
        let selection_dots =
            selection_dots_enabled.then(|| SelectionDotsPass::new(&ctx, &scene_layout));

        let readback = with_picking.then(|| PickingReadback::new(&ctx.device));

        let hover_readback = with_picking.then(|| PickingReadback::new(&ctx.device));

        let picking_reproject = if picking_mode == PickingMode::Reprojected
            && memory_policy.picking.reprojection_enabled
        {
            let (pw, ph) = targets.picking_dims();

            Some(PickingReproject::new(&ctx, pw, ph))
        } else {
            None
        };

        let cull_pipeline = CullPipeline::new(&ctx.device);

        let directional_shadow =
            DirectionalShadowPass::new(&ctx.device, &ctx.frame.bind_group_layout);

        let atlas_ao = AtlasAoPass::new(&ctx.device, &ctx.frame.bind_group_layout);

        // SSAO compute / blur / compose. Wired even when disabled so

        // `set_ssao(true)` is a zero-rebuild toggle.

        let ssao_compute = SsaoCompute::new(&ctx.device);

        let ssao_blur = SsaoBlur::new(&ctx.device);

        let ssao_compose = SsaoComposePass::new(&ctx);

        let ssao_resources = SsaoResources::new(&ctx);

        let (
            ssao_bind_group,
            ssao_blur_h_bind_group,
            ssao_blur_v_bind_group,
            ssao_compose_bind_group,
        ) = match (
            targets.ssao_view.as_ref(),
            targets.ssao_blurred_view.as_ref(),
        ) {
            (Some(ssao_view), Some(ssao_blurred_view)) => (
                Some(ssao_compute.make_bind_group(
                    &ctx.device,
                    &ctx.frame.buffer,
                    &ssao_resources.ssao_params_buffer,
                    &targets.depth,
                    ssao_view,
                )),
                Some(ssao_blur.make_bind_group(
                    &ctx.device,
                    &ssao_resources.blur_h_params_buffer,
                    &targets.depth,
                    ssao_view,
                    ssao_blurred_view,
                )),
                Some(ssao_blur.make_bind_group(
                    &ctx.device,
                    &ssao_resources.blur_v_params_buffer,
                    &targets.depth,
                    ssao_blurred_view,
                    ssao_view,
                )),
                Some(ssao_compose.make_bind_group(&ctx.device, ssao_view)),
            ),
            _ => (None, None, None, None),
        };

        let fxaa_pass = FxaaPass::new(&ctx);

        let fxaa_bind_group = targets
            .color_scratch_view
            .as_ref()
            .map(|view| fxaa_pass.make_bind_group(&ctx.device, view));

        let composite_bind_group =
            composite.make_bind_group(&ctx.device, &targets.accum, &targets.reveal);

        let silhouette_bind_group = None;

        #[cfg(feature = "stats")]
        let stats = FrameStatsCollector::new(&ctx.device, &ctx.queue);

        Self {
            ctx,

            targets,

            uniforms,

            scene: SceneRuntime {
                scene_layout,
                scene_store,
                reps: HashMap::new(),
                maps: HashMap::new(),
                strokes: HashMap::new(),
                draw_order: Vec::new(),
                map_draw_order: Vec::new(),
                stroke_draw_order: Vec::new(),
                stroke_bounds_hash: 0,
                scene_dirty: true,
                cull_pass_initialized: false,
                last_cull_view_proj_hash: 0,
                has_any_marker: false,
                marker_object_hash: 0,
                scene_bounds: None,
                lod: SceneLod::Auto,
            },

            geometry: GeometryRuntime {
                sphere_params_layout,
                stick_params_layout,
                ellipsoid_params_layout,
                cartoon_params_layout,
                surface_params_layout,
                mesh_params_layout,
                dot_params_layout,
                sphere_compute,
                sphere_lod_count,
                stick_compute,
                stick_lod_count,
                line_compute,
                dot_compute,
                ellipsoid_compute,
                cartoon_compute,
                surface_density_compute,
                surface_vdw_sdf_compute,
                surface_ses_morph_compute,
                surface_mc_compute,
                sphere_pipeline,
                stick_pipeline,
                line_pipeline,
                dot_pipeline,
                map_params_layout,
                map_pipeline,
                stroke_pipeline,
                mesh_pipeline,
                ellipsoid_pipeline,
                cartoon_pipeline,
                surface_pipeline,
                depth_prepass,
                cull_pipeline,
            },

            picking: PickingRuntime {
                picking_mode,
                budget_allowed: with_picking,
                id_pass,
                readback,
                hover_readback,
                picking_reproject,
                last_full_record_view_proj: None,
                reproject_count: 0,
                last_view_proj_hash: 0,
                picking_pass_initialized: false,
                picking_texture_source: None,
                overlay_id_pass_initialized: false,
                overlay_id_view_proj_hash: 0,
            },

            screen: ScreenRuntime {
                composite,
                silhouette,
                marking,
                selection_dots,
                composite_bind_group,
                silhouette_bind_group,
                marking_bind_groups: None,
                marking_bindings_dirty: true,
                marking_params_dirty: true,
                marking_offsets_dirty: true,
                selection_dots_enabled,
                selection_dots_rebuild_all: selection_dots_enabled,
                selection_overlay_enabled,
                marking_width: 1.0,
                silhouette_params: None,
                ssao_compute,
                ssao_blur,
                ssao_compose,
                ssao_resources,
                ssao_bind_group,
                ssao_blur_h_bind_group,
                ssao_blur_v_bind_group,
                ssao_compose_bind_group,
                ssao_enabled: false,
                ssao_intensity: 1.0,
                ssao_frame_counter: 0,
                fxaa_pass,
                fxaa_bind_group,
                fxaa_overlay_bind_group: None,
                fxaa_enabled: false,
                clear_color: wgpu::Color::TRANSPARENT,
            },

            lighting: LightingRuntime {
                directional_shadow,
                atlas_ao,
                shadow_mode: ShadowPassMode::Disabled,
                shadow_map_size: DEFAULT_SHADOW_MAP_SIZE,
                shadow_bias: 0.01,
                shadow_intensity: 0.0,
                shadow_pcf_radius: 1.0,
                skripkin_directions: 64,
                skripkin_map_size: 128,
                skripkin_bias: 0.025,
                skripkin_intensity: 1.0,
            },

            memory: MemoryRuntime {
                policy: memory_policy,
                pending_warnings: memory_warnings,
                rep_budget_diagnostics: Vec::new(),
                rep_budget_request_cache: HashMap::new(),
                warned_rep_budget: std::collections::HashSet::new(),
                warned_ssao_denied: false,
                warned_fxaa_denied: false,
                warned_selection_denied: false,
                warned_silhouette_denied: false,
                warned_picking_budget_denied: false,
                warned_shadow_clamped: false,
                warned_atlas_clamped: false,
            },

            last_sync_timings: RenderSyncTimings::default(),

            #[cfg(feature = "stats")]
            stats,
        }
    }
}
