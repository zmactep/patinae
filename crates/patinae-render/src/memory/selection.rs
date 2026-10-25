//! Pure startup decisions; platform queries and resource allocation stay outside.

use super::{RenderMemoryPolicy, RenderMemoryProfile};
use crate::memory::{DEVICE_BUFFER_TARGET_BYTES, DEVICE_STORAGE_BINDING_TARGET_BYTES};
use crate::{gib_to_bytes, mib_to_bytes, NativeGpuMemoryBudget};

const LITE_BUFFER_THRESHOLD: u64 = gib_to_bytes(1);
const LITE_STORAGE_BINDING_THRESHOLD: u64 = mib_to_bytes(512);
// Keep a quarter of the advisory budget outside our startup headroom estimate.
// Eight GiB of remaining headroom permits the eager performance policy; below
// two GiB use lite instead of balanced's two-GiB representation budget.
const PERFORMANCE_MEMORY_HEADROOM: u64 = gib_to_bytes(8);
const BALANCED_MEMORY_HEADROOM: u64 = gib_to_bytes(2);

/// Synthetic adapter inputs for profile selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderMemorySelectionInput {
    /// Adapter type.
    pub adapter_type: wgpu::DeviceType,
    /// Adapter backend.
    pub backend: wgpu::Backend,
    /// Adapter maximum buffer size.
    pub max_buffer_size: u64,
    /// Adapter maximum storage-buffer binding size.
    pub max_storage_buffer_binding_size: u64,
    /// Adapter maximum two-dimensional texture dimension.
    pub max_texture_dimension_2d: u32,
    /// Whether this is a browser-hosted renderer.
    pub is_web: bool,
}

impl RenderMemorySelectionInput {
    /// Builds selector input from `wgpu` adapter data.
    pub fn from_wgpu(info: &wgpu::AdapterInfo, limits: &wgpu::Limits, is_web: bool) -> Self {
        Self {
            adapter_type: info.device_type,
            backend: info.backend,
            max_buffer_size: limits.max_buffer_size,
            max_storage_buffer_binding_size: limits.max_storage_buffer_binding_size,
            max_texture_dimension_2d: limits.max_texture_dimension_2d,
            is_web,
        }
    }
}

/// The decision path used to select a startup profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderMemorySelectionReason {
    /// Explicit user or host override.
    ExplicitOverride,
    /// Native process budget and the startup reserve.
    NativeBudget,
    /// No usable native budget; conservative adapter capabilities.
    CapabilityFallback,
}

/// Native headroom after the startup reserve, computed once for policy and logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderMemoryHeadroom {
    /// Raw advisory snapshot from the selected adapter/device.
    pub budget: NativeGpuMemoryBudget,
    /// Budget reserved for allocations outside renderer estimates.
    pub reserved_bytes: u64,
    /// Remaining usable budget after process usage and the reserve.
    pub available_bytes: u64,
}

impl RenderMemoryHeadroom {
    fn from_budget(budget: NativeGpuMemoryBudget) -> Option<Self> {
        if budget.budget_bytes == 0 {
            return None;
        }
        // Leave one quarter of the advisory budget for untracked allocations.
        let reserved_bytes = budget.budget_bytes / 4;
        Some(Self {
            budget,
            reserved_bytes,
            available_bytes: budget.headroom_bytes().saturating_sub(reserved_bytes),
        })
    }
}

/// Complete startup decision, including the evidence used for diagnostics.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderMemorySelection {
    /// Effective construction-time policy.
    pub policy: RenderMemoryPolicy,
    /// Why this profile was selected.
    pub reason: RenderMemorySelectionReason,
    /// Native budget calculation, if a usable snapshot was available.
    pub headroom: Option<RenderMemoryHeadroom>,
}

/// Selects a profile without querying the OS, allocating, or mutating settings.
///
/// Native headroom determines quality; device limits constrain allocations.
/// An explicit override wins. Missing or zero budgets use conservative limits.
pub fn select_render_memory_policy(
    input: RenderMemorySelectionInput,
    budget: Option<NativeGpuMemoryBudget>,
    override_profile: Option<RenderMemoryProfile>,
) -> RenderMemorySelection {
    use RenderMemorySelectionReason as Reason;
    let headroom = budget.and_then(RenderMemoryHeadroom::from_budget);
    let (profile, reason) = if let Some(profile) = override_profile {
        (profile, Reason::ExplicitOverride)
    } else if let Some(headroom) = headroom {
        let profile = if headroom.available_bytes >= PERFORMANCE_MEMORY_HEADROOM {
            RenderMemoryProfile::Performance
        } else if headroom.available_bytes >= BALANCED_MEMORY_HEADROOM {
            RenderMemoryProfile::Balanced
        } else {
            RenderMemoryProfile::Lite
        };
        (profile, Reason::NativeBudget)
    } else {
        (capability_fallback(input), Reason::CapabilityFallback)
    };
    RenderMemorySelection {
        policy: RenderMemoryPolicy::from_profile(profile),
        reason,
        headroom,
    }
}

fn capability_fallback(input: RenderMemorySelectionInput) -> RenderMemoryProfile {
    if input.adapter_type == wgpu::DeviceType::Cpu
        || input.adapter_type == wgpu::DeviceType::VirtualGpu
        || input.max_buffer_size < LITE_BUFFER_THRESHOLD
        || input.max_storage_buffer_binding_size < LITE_STORAGE_BINDING_THRESHOLD
        || input.max_texture_dimension_2d < 8192
    {
        return RenderMemoryProfile::Lite;
    }
    if input.is_web
        || matches!(input.backend, wgpu::Backend::BrowserWebGpu)
        || input.max_buffer_size < DEVICE_BUFFER_TARGET_BYTES
        || input.max_storage_buffer_binding_size < DEVICE_STORAGE_BINDING_TARGET_BYTES
    {
        return RenderMemoryProfile::Balanced;
    }
    RenderMemoryProfile::Performance
}
