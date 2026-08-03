//! patinae-render — GPU-first molecular renderer.
//!
//! Core renderer contracts include fixed representation bind groups,
//! indirect draws, settings-as-uniforms, and GPU-first compute criteria.
//!
//! Public API:
//! - `RenderState::new` / `resize` / `sync` / `render` / `pick`
//! - `RenderInput` + `RenderObjectInput` for pull-based host integration
//! - `RepKind`, `ObjectId`, `PickHit` for picking round-trips

mod byte_units;
pub mod capture;
#[allow(dead_code)]
pub(crate) mod compute;
mod context;
pub mod frame;
mod geometry_export;
pub(crate) mod lut_buffer;
mod map_contour;
pub mod memory;
mod memory_policy;
mod memory_recovery;
pub(crate) mod passes;
pub mod picking;
pub(crate) mod pipelines;
pub(crate) mod postprocess;
mod render_artifacts;
mod render_input;
mod render_state;
mod representation_budget;
#[allow(dead_code)]
pub(crate) mod representations;
pub mod scene_store;
mod shader_source;
#[cfg(feature = "stats")]
mod stats;
/// `FrameStatsHistory` is compiled unconditionally so hosts can surface
/// fps + percentile tail-latency in a HUD even when the `stats` cargo
/// feature (per-pass GPU timestamps) is off.
mod stats_history;
mod stroke;
mod uniforms;
mod viewport_image;

#[doc(inline)]
pub use byte_units::{
    bytes_to_gib, bytes_to_mib, gib_to_bytes, mib_to_bytes, BYTES_PER_GIB, BYTES_PER_MIB,
};
pub use context::RenderContext;
pub use frame::FrameTargets;
pub use geometry_export::{
    DisplayedGeometry, DisplayedMaterial, DisplayedMesh, DisplayedMeshVertex,
    DisplayedObjectGeometry, DisplayedPrimitive, GeometryExportError, GeometryExportOptions,
    TraceCylinder, TraceGeometryChunk, TraceLineSegment, TraceMaterial, TracePointSample,
    TraceSphere, TraceTriangle,
};
pub use memory::{
    estimate_texture_2d_bytes, estimate_texture_bytes, estimate_texture_descriptor_bytes,
    GpuAllocationEstimate, GpuMemoryBucket, GpuMemoryCategory, GpuMemoryLedger, GpuMemorySnapshot,
    GpuMemoryUsage,
};
pub use memory_policy::{
    render_memory_policy_from_settings, required_limits_for_memory_policy,
    select_render_memory_policy, FrameTargetPolicy, OverlayPolicy, PickingPolicy,
    PostprocessPolicy, RenderAdapterType, RenderBackend, RenderMemoryPolicy, RenderMemoryProfile,
    RenderMemoryProfileParseError, RenderMemorySelectionInput, RepresentationBudgetPolicy,
    SceneStoreCompactionPolicy, SceneStoreGrowthPolicy, SceneStoreMemoryPolicy, ShadowPolicy,
    PERFORMANCE_MAX_BUFFER_SIZE, PERFORMANCE_MAX_STORAGE_BUFFER_BINDING_SIZE,
    RENDER_COMPUTE_STORAGE_BUFFERS_PER_STAGE,
};
pub use memory_recovery::{
    is_wgpu_oom, next_oom_fallback, RenderMemoryRecoveryAction, RenderMemoryRecoveryStage,
};
pub use picking::{ObjectId, PackedId, PickHit, PickingMode, RenderConfig, RepKind};
pub use render_artifacts::{
    RenderArtifactBufferRef, RenderArtifactBufferRole, RenderArtifactPrimitiveTopology,
    RenderArtifactRep, RenderArtifactSnapshot, RENDER_ARTIFACT_LAYOUT_VERSION,
};
pub use render_input::{
    pack_rep_rgb8, ColorLutEntry, MarkerUpdate, RenderInput, RenderMapInput, RenderMapMode,
    RenderObjectInput, RenderStrokeInput, RepColorLutEntry, SceneLod, StrokeSegment,
    IDENTITY_TRANSFORM, REP_COLOR_INHERIT,
};
pub use render_state::{RenderState, RenderSyncTimings};
pub use representation_budget::{
    RepBudgetDiagnostic, RepBuildDecision, RepMemoryEstimate, RepQualityLevel, RepSkipReason,
};
pub use representations::sphere::SphereLodDiagnostics;
pub use representations::stick::StickLodDiagnostics;
pub use scene_store::{
    SceneStoreBufferKind, SceneStoreBufferStats, SceneStoreCompactionStats,
    SceneStoreFragmentationStats, SceneStoreMemoryStats,
};
#[cfg(feature = "stats")]
pub use stats::{FrameStats, FrameStatsCollector};
pub use stats_history::FrameStatsHistory;
pub use uniforms::FrameUniforms;
#[doc(inline)]
pub use viewport_image::{
    copy_rgba_buffer_to_viewport_texture, GpuViewportImage, GpuViewportImageError,
};
