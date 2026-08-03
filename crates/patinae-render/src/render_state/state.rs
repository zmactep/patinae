use std::collections::{HashMap, HashSet};

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
use crate::map_contour::MapEntry;
use crate::memory_policy::RenderMemoryPolicy;
use crate::passes::atlas_ao::AtlasAoPass;
use crate::passes::selection_dots::SelectionDotsPass;
use crate::passes::shadow::DirectionalShadowPass;
use crate::picking::pass::PickingPass;
use crate::picking::readback::PickingReadback;
use crate::picking::reproject::PickingReproject;
use crate::picking::{PickingMode, RepKind};
use crate::pipelines::cartoon::{CartoonParamsLayout, CartoonPipeline};
use crate::pipelines::depth_prepass::DepthPrepassPipelines;
use crate::pipelines::dot::DotPipeline;
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
use crate::postprocess::marking::{MarkingBindGroups, MarkingPass};
use crate::postprocess::silhouette::{SilhouetteParams, SilhouettePass};
use crate::postprocess::ssao_compose::SsaoComposePass;
use crate::render_input::SceneLod;
use crate::representation_budget::{RepBudgetDiagnostic, RepBudgetRequest, RepBudgetWarningKey};
use crate::representations::{DrawPhase, Representation};
use crate::scene_store::{
    SceneStore, SceneStoreCompactionStats, SceneStoreFragmentationStats, SceneStoreLayout,
};
#[cfg(feature = "stats")]
use crate::stats::FrameStatsCollector;
use crate::stroke::StrokeEntry;
use crate::uniforms::FrameUniforms;

/// Per-rep picking state — uniform buffer + bind group for `PickingParams`.
pub(super) struct RepPickingState {
    pub(super) _buffer: wgpu::Buffer,
    pub(super) bind_group: wgpu::BindGroup,
}

/// One drawable per (object_id, rep_kind). Heterogeneous reps live behind a
/// trait object so shared passes can dispatch by `rep.kind()`.
pub(super) struct RepEntry {
    pub(super) rep: Box<dyn Representation>,
    /// Per-rep id bind group (group 2 = `PickingParams`). `None` until a
    /// hit-test pick or visual overlay first needs an id-rendering pass.
    pub(super) picking: Option<RepPickingState>,
    /// Visible draw phase for this rep. Refreshed every `sync` from
    /// `Representation::draw_phase`.
    pub(super) draw_phase: DrawPhase,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct SceneBounds {
    pub(super) center: [f32; 3],
    pub(super) radius: f32,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RenderSyncTimings {
    pub scene_store_object_sync_ms: f32,
    pub scene_store_flush_ms: f32,
    pub marking_resources_ms: f32,
    pub rep_sync_ms: f32,
    pub map_sync_ms: f32,
    pub stroke_sync_ms: f32,
    pub order_bounds_ms: f32,
    pub compute_dispatch_ms: f32,
    pub marker_lut_upload_bytes: u64,
    pub marker_lut_upload_ranges: u32,
    pub marker_lut_reallocated: bool,
    pub scene_store_fragmentation: SceneStoreFragmentationStats,
    pub scene_store_compaction: SceneStoreCompactionStats,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ShadowPassMode {
    Disabled,
    Directional,
    AtlasAo,
}

pub(super) struct SceneRuntime {
    pub(super) scene_layout: SceneStoreLayout,
    pub(super) scene_store: SceneStore,
    pub(super) reps: HashMap<(u32, RepKind), RepEntry>,
    pub(super) maps: HashMap<u32, MapEntry>,
    pub(super) strokes: HashMap<u32, StrokeEntry>,
    /// Stable draw order grouped by `RepKind`, so passes (picking,
    /// depth_prepass, translucent) hit the same pipeline in long runs and
    /// behave deterministically across frames. `HashMap::values()` order is
    /// unspecified — relying on it produced flickering picks at overlaps.
    /// Refreshed in `sync`.
    pub(super) draw_order: Vec<(u32, RepKind)>,
    /// Stable draw order for first-class map contour objects.
    pub(super) map_draw_order: Vec<u32>,
    /// Stable registry order for semantic objects that own visible strokes.
    pub(super) stroke_draw_order: Vec<u32>,
    /// Fingerprint of all annotation-owner bounds, including label-only owners.
    pub(super) stroke_bounds_hash: u64,
    /// Set by `sync` and viewport LOD transitions when scene-affecting data
    /// invalidates cached render-side state.
    pub(super) scene_dirty: bool,
    /// True after the compacted per-rep instance buffers were culled for the
    /// current scene and view-projection hash.
    pub(super) cull_pass_initialized: bool,
    /// View-projection hash that produced the cached compacted cull buffers.
    pub(super) last_cull_view_proj_hash: u64,
    /// Cheap CPU heuristic refreshed in `sync` — `true` if any object
    /// supplied a non-zero `atom_markers` entry (selected or hovered atom).
    /// Used to skip selection overlay work over scenes with no selection /
    /// hover state.
    pub(super) has_any_marker: bool,
    /// Hash of object ids from the last marker scan. Lets `sync` avoid an
    /// O(N atoms) marker walk on camera-only frames while still noticing
    /// object add/remove waves.
    pub(super) marker_object_hash: u64,
    pub(super) scene_bounds: Option<SceneBounds>,
    pub(super) lod: SceneLod,
}

pub struct GeometryRuntime {
    pub(crate) sphere_params_layout: SphereParamsLayout,
    pub(crate) stick_params_layout: StickParamsLayout,
    pub(crate) ellipsoid_params_layout: EllipsoidParamsLayout,
    pub(crate) cartoon_params_layout: CartoonParamsLayout,
    pub(crate) surface_params_layout: SurfaceParamsLayout,
    pub(crate) mesh_params_layout: MeshParamsLayout,
    pub(crate) dot_params_layout: crate::pipelines::dot::DotParamsLayout,
    pub(crate) sphere_compute: SphereBuildPipeline,
    pub(crate) sphere_lod_count: SphereLodCountPipeline,
    pub(crate) stick_compute: StickBuildPipeline,
    pub(crate) stick_lod_count: StickLodCountPipeline,
    pub(crate) line_compute: LineBuildPipeline,
    pub(crate) dot_compute: DotBuildPipeline,
    pub(crate) ellipsoid_compute: EllipsoidBuildPipeline,
    pub(crate) cartoon_compute: CartoonExtrudeCompute,
    pub(crate) surface_density_compute: SurfaceDensityCompute,
    pub(crate) surface_vdw_sdf_compute: SurfaceVdwSdfCompute,
    pub(crate) surface_ses_morph_compute: SurfaceSesMorphCompute,
    pub(crate) surface_mc_compute: SurfaceMcCompute,
    pub(crate) sphere_pipeline: SpherePipeline,
    pub(crate) stick_pipeline: StickPipeline,
    pub(crate) line_pipeline: LinePipeline,
    pub(crate) dot_pipeline: DotPipeline,
    pub(crate) map_params_layout: MapParamsLayout,
    pub(crate) map_pipeline: MapPipeline,
    pub(crate) stroke_pipeline: StrokePipeline,
    pub(crate) mesh_pipeline: MeshPipeline,
    pub(crate) ellipsoid_pipeline: EllipsoidPipeline,
    pub(crate) cartoon_pipeline: CartoonPipeline,
    pub(crate) surface_pipeline: SurfacePipeline,
    /// Depth-only sibling pipelines used by the shadow-map pass.
    pub(crate) depth_prepass: DepthPrepassPipelines,
    /// Per-rep cull pipeline. Owns the sphere/stick/... compute pipelines
    /// under a shared bind-group layout.
    pub(crate) cull_pipeline: CullPipeline,
}

pub(super) struct PickingRuntime {
    /// Construction-time hit-test picking policy. `Disabled` ⇒ readbacks and
    /// reprojection resources are absent; visual overlays use a separate id
    /// target and can still be enabled.
    pub(super) picking_mode: PickingMode,
    /// Runtime budget gate for hit-test picking resources.
    pub(super) budget_allowed: bool,
    /// Id-rendering pass used by hit-test picking and visual overlays. Created
    /// lazily when either feature first needs visible object/atom ids.
    pub(super) id_pass: Option<PickingPass>,
    /// Click-pick readback. Also used by the blocking `pick()` helper.
    /// `None` when `picking_mode == Disabled`.
    pub(super) readback: Option<PickingReadback>,
    /// Async hover-pick readback. Separate from click readback so clicks do
    /// not wait for a pending hover query to release the staging buffer.
    pub(super) hover_readback: Option<PickingReadback>,
    /// Reprojection compute infra. Only allocated when
    /// `picking_mode == PickingMode::Reprojected`.
    pub(super) picking_reproject: Option<PickingReproject>,
    /// `view_proj` at the moment of the last full re-record. Used both as
    /// the "previous" matrix for reprojection and as the baseline against
    /// which the angular-drift watchdog measures.
    pub(super) last_full_record_view_proj: Option<[[f32; 4]; 4]>,
    /// Frames since the last full re-record. Used by the watchdog to force
    /// a periodic full re-record bounded by `MAX_REPROJECT_FRAMES`.
    pub(super) reproject_count: u32,
    /// Picking texture is the heaviest extra GPU pass we run every frame
    /// (full geometry rasterization). It only needs to be rewritten when
    /// either geometry changed (`scene_dirty`) or the camera moved. If
    /// neither has happened, the texture from the previous frame is still
    /// pixel-correct and we skip the pass entirely. `last_view_proj_hash`
    /// is bumped only when `uniforms.view_proj` differs from the value
    /// that produced the cached picking texture.
    pub(super) last_view_proj_hash: u64,
    /// True until at least one picking pass has executed against the
    /// current `FrameTargets`. Forces a write on the first frame after
    /// `new`/`resize` regardless of camera/scene dirtyness.
    pub(super) picking_pass_initialized: bool,
    /// Provenance of the cached picking texture. A reprojected texture is
    /// good enough for readback, but visible overlays need a full raster pass
    /// to avoid half-res reprojection artifacts.
    pub(super) picking_texture_source: Option<PickingTextureSource>,
    /// Full-resolution overlay id target cache state. Marker-only updates
    /// refresh the marker LUT and marking mask, but keep this expensive
    /// scene id pass valid until geometry, visibility, resize, or camera
    /// movement invalidates it.
    pub(super) overlay_id_pass_initialized: bool,
    /// View-projection hash that produced the cached full-resolution overlay
    /// id target.
    pub(super) overlay_id_view_proj_hash: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PickingTextureSource {
    FullRecord,
    Reprojected,
}

pub(super) struct ScreenRuntime {
    pub(super) composite: WboitComposite,
    /// Lazily created when silhouettes are enabled. Silhouettes read the
    /// overlay id texture and are independent from hit-test picking.
    pub(super) silhouette: Option<SilhouettePass>,
    /// Lazily created when the selection overlay is enabled. Marking reads the
    /// overlay id texture and marker LUT.
    pub(super) marking: Option<MarkingPass>,
    /// Lite selection-only fallback. Draws compact selected atom dots
    /// without allocating overlay id, marking mask, or color scratch targets.
    pub(super) selection_dots: Option<SelectionDotsPass>,
    pub(super) composite_bind_group: wgpu::BindGroup,
    /// `None` until overlay id resources exist and silhouettes are enabled.
    pub(super) silhouette_bind_group: Option<wgpu::BindGroup>,
    /// `None` when marking is unavailable, disabled, or overlay targets do not
    /// exist.
    pub(super) marking_bind_groups: Option<MarkingBindGroups>,
    pub(super) marking_bindings_dirty: bool,
    pub(super) marking_params_dirty: bool,
    pub(super) marking_offsets_dirty: bool,
    /// Host requested selection visuals, but the memory policy denied the full
    /// selection/hover overlay and selected atom dots are used instead.
    pub(super) selection_dots_enabled: bool,
    pub(super) selection_dots_rebuild_all: bool,
    pub(super) selection_overlay_enabled: bool,
    /// Screen-space selection / hover outline width, in target pixels.
    pub(super) marking_width: f32,
    /// Set by the host via `set_silhouette`. When `None`, the silhouette pass
    /// is skipped.
    pub(super) silhouette_params: Option<SilhouetteParams>,
    /// SSAO compute + blur + compose. Pipelines + resources are always
    /// present; whether SSAO runs is driven by `ssao_enabled` from the host.
    pub(super) ssao_compute: SsaoCompute,
    pub(super) ssao_blur: SsaoBlur,
    pub(super) ssao_compose: SsaoComposePass,
    pub(super) ssao_resources: SsaoResources,
    pub(super) ssao_bind_group: Option<wgpu::BindGroup>,
    pub(super) ssao_blur_h_bind_group: Option<wgpu::BindGroup>,
    pub(super) ssao_blur_v_bind_group: Option<wgpu::BindGroup>,
    pub(super) ssao_compose_bind_group: Option<wgpu::BindGroup>,
    /// Cached SSAO settings from the host. `enabled=false` skips the
    /// entire SSAO chain (no compute dispatches, no compose pass).
    pub(super) ssao_enabled: bool,
    pub(super) ssao_intensity: f32,
    /// Frame counter used to jitter SSAO sample pattern each frame.
    pub(super) ssao_frame_counter: u32,
    /// When enabled, every colour-writing pass writes to
    /// `targets.color_scratch_view`; the final FXAA pass then writes to the
    /// host target.
    pub(super) fxaa_pass: FxaaPass,
    pub(super) fxaa_bind_group: Option<wgpu::BindGroup>,
    pub(super) fxaa_overlay_bind_group: Option<wgpu::BindGroup>,
    pub(super) fxaa_enabled: bool,
    /// Host clear colour for the visible colour target. Alpha is driven by
    /// `opaque_background`; transparent captures keep alpha at 0.
    pub(super) clear_color: wgpu::Color,
}

pub(super) struct LightingRuntime {
    pub(super) directional_shadow: DirectionalShadowPass,
    pub(super) atlas_ao: AtlasAoPass,
    pub(super) shadow_mode: ShadowPassMode,
    pub(super) shadow_map_size: u32,
    pub(super) shadow_bias: f32,
    pub(super) shadow_intensity: f32,
    pub(super) shadow_pcf_radius: f32,
    pub(super) skripkin_directions: u32,
    pub(super) skripkin_map_size: u32,
    pub(super) skripkin_bias: f32,
    pub(super) skripkin_intensity: f32,
}

pub(super) struct MemoryRuntime {
    pub(super) policy: RenderMemoryPolicy,
    pub(super) pending_warnings: Vec<String>,
    pub(super) rep_budget_diagnostics: Vec<RepBudgetDiagnostic>,
    pub(super) rep_budget_request_cache: HashMap<(u32, RepKind), RepBudgetRequest>,
    pub(super) warned_rep_budget: HashSet<RepBudgetWarningKey>,
    pub(super) warned_ssao_denied: bool,
    pub(super) warned_fxaa_denied: bool,
    pub(super) warned_selection_denied: bool,
    pub(super) warned_silhouette_denied: bool,
    pub(super) warned_picking_budget_denied: bool,
    pub(super) warned_shadow_clamped: bool,
    pub(super) warned_atlas_clamped: bool,
}

pub struct RenderState {
    pub ctx: RenderContext,
    pub targets: FrameTargets,
    pub uniforms: FrameUniforms,

    pub(super) scene: SceneRuntime,
    pub(super) geometry: GeometryRuntime,
    pub(super) picking: PickingRuntime,
    pub(super) screen: ScreenRuntime,
    pub(super) lighting: LightingRuntime,
    pub(super) memory: MemoryRuntime,
    pub(super) last_sync_timings: RenderSyncTimings,

    /// Per-frame instrumentation collector. Always present when the
    /// `stats` feature is compiled in; falls back to CPU-only markers
    /// when the adapter doesn't expose `TIMESTAMP_QUERY`.
    #[cfg(feature = "stats")]
    pub(super) stats: FrameStatsCollector,
}
