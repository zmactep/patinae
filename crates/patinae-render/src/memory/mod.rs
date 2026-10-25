//! GPU memory budgets, resource accounting, allocation limits, and recovery.
//!
//! Native queries report advisory process budgets. Selection turns those facts
//! into a profile; policy defines its resource settings. Allocation planners
//! enforce device limits independently, while the ledger tracks estimated usage
//! and recovery handles confirmed OOM events.

mod buffer_limits;
mod byte_units;
mod device_limits;
mod ledger;
mod native;
mod policy;
mod recovery;
mod representation_budget;
mod selection;

pub(crate) use buffer_limits::checked_buffer_capacity;
pub use buffer_limits::BufferSizeError;
pub use byte_units::{
    bytes_to_gib, bytes_to_mib, gib_to_bytes, mib_to_bytes, BYTES_PER_GIB, BYTES_PER_MIB,
};
pub use device_limits::{
    required_device_limits, DeviceRequirementsError, DEVICE_BUFFER_TARGET_BYTES,
    DEVICE_STORAGE_BINDING_TARGET_BYTES, RENDER_COMPUTE_STORAGE_BUFFERS_PER_STAGE,
};
pub use ledger::{
    buffer_usage, estimate_texture_2d_bytes, estimate_texture_bytes,
    estimate_texture_descriptor_bytes, GpuAllocationEstimate, GpuMemoryBucket, GpuMemoryCategory,
    GpuMemoryLedger, GpuMemorySnapshot, GpuMemoryUsage, GPU_MEMORY_CATEGORY_COUNT,
};
#[cfg(feature = "native-memory")]
pub use native::{adapter_memory_budget, device_memory_budget};
pub use native::{NativeGpuMemoryBudget, NativeMemoryBudgetSource};
pub use policy::{
    render_memory_policy_from_settings, FrameTargetPolicy, OverlayPolicy, PickingPolicy,
    PostprocessPolicy, RenderMemoryPolicy, RenderMemoryProfile, RenderMemoryProfileParseError,
    SceneStoreCompactionPolicy, SceneStoreGrowthPolicy, SceneStoreMemoryPolicy, ShadowPolicy,
};
pub use recovery::{
    is_wgpu_oom, next_oom_fallback, RenderMemoryRecoveryAction, RenderMemoryRecoveryStage,
};
pub(crate) use representation_budget::{
    current_warning_keys, plan_rep_budget, RepBudgetInput, RepBudgetRequest, RepBudgetWarningKey,
};
pub use representation_budget::{
    RepBudgetDiagnostic, RepBuildDecision, RepMemoryEstimate, RepQualityLevel, RepSkipReason,
};
pub use selection::{
    select_render_memory_policy, RenderMemoryHeadroom, RenderMemorySelection,
    RenderMemorySelectionInput, RenderMemorySelectionReason,
};
