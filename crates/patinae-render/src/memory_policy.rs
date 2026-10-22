//! Renderer memory profile selection.
//!
//! The types in this module describe construction-time memory policy. They
//! intentionally stop before representation budgeting: profile-aware rep
//! downgrade, chunking, and skip decisions belong to a later planning phase.

use std::fmt;
use std::str::FromStr;

use crate::byte_units::{gib_to_bytes, mib_to_bytes, BYTES_PER_MIB};
use crate::NativeGpuMemoryBudget;

const LITE_BUFFER_THRESHOLD: u64 = gib_to_bytes(1);
const LITE_STORAGE_BINDING_THRESHOLD: u64 = mib_to_bytes(512);
/// Desired single-buffer target for capable native adapters.
pub const PERFORMANCE_MAX_BUFFER_SIZE: u64 = gib_to_bytes(4);
/// Minimum practical buffer ceiling for the performance policy.
/// WGPU indirect validation caps buffers at u32::MAX, even on large GPUs.
pub const PERFORMANCE_MIN_BUFFER_SIZE: u64 = PERFORMANCE_MAX_BUFFER_SIZE - 1;
// Keep a quarter of the advisory budget outside our startup headroom estimate.
// Eight GiB of remaining headroom permits the eager performance policy; below
// two GiB use lite instead of balanced's two-GiB representation budget.
const PERFORMANCE_MEMORY_HEADROOM: u64 = gib_to_bytes(8);
const BALANCED_MEMORY_HEADROOM: u64 = gib_to_bytes(2);
/// Desired single storage-buffer binding target for capable adapters.
pub const PERFORMANCE_MAX_STORAGE_BUFFER_BINDING_SIZE: u64 = gib_to_bytes(2) - 1;
/// Maximum storage-buffer bindings used by the current compute pipeline layouts.
pub const RENDER_COMPUTE_STORAGE_BUFFERS_PER_STAGE: u32 = 10;

const DEFAULT_PICKING_SCALE: f32 = 0.5;
const LITE_PICKING_SCALE: f32 = 0.25;
const PERFORMANCE_SHADOW_MAP_SIZE: u32 = 4096;
const BALANCED_SHADOW_MAP_SIZE: u32 = 2048;
const LITE_SHADOW_MAP_SIZE: u32 = 1024;
const PERFORMANCE_ATLAS_DIRECTIONS: u32 = 256;
const BALANCED_ATLAS_DIRECTIONS: u32 = 64;
const LITE_ATLAS_DIRECTIONS: u32 = 16;
const PERFORMANCE_SCENE_STORE_ORPHANED_BYTES: u64 = mib_to_bytes(512);
const BALANCED_SCENE_STORE_ORPHANED_BYTES: u64 = mib_to_bytes(128);
const LITE_SCENE_STORE_ORPHANED_BYTES: u64 = mib_to_bytes(32);
const MANUAL_SCENE_STORE_ORPHANED_FLOOR_BYTES: u64 = mib_to_bytes(8);
const BALANCED_SCENE_STORE_SLACK_BYTES: u64 = mib_to_bytes(64);
const LITE_SCENE_STORE_SLACK_BYTES: u64 = mib_to_bytes(16);

/// User-visible renderer memory profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderMemoryProfile {
    /// Preserve current high-quality, eager-allocation behavior.
    Performance,
    /// Prefer lower baseline memory without disabling normal interaction.
    Balanced,
    /// Disable or deny optional full-resolution screen resources.
    Lite,
    /// Apply profile policy from an explicit byte budget.
    Manual { bytes: u64 },
}

impl RenderMemoryProfile {
    /// Returns the configured budget, if this profile is manual.
    pub const fn budget_bytes(self) -> Option<u64> {
        match self {
            Self::Manual { bytes } => Some(bytes),
            Self::Performance | Self::Balanced | Self::Lite => None,
        }
    }
}

impl fmt::Display for RenderMemoryProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Performance => f.write_str("performance"),
            Self::Balanced => f.write_str("balanced"),
            Self::Lite => f.write_str("lite"),
            Self::Manual { bytes } => write!(f, "manual:{}MiB", bytes / BYTES_PER_MIB),
        }
    }
}

impl FromStr for RenderMemoryProfile {
    type Err = RenderMemoryProfileParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let value = value.trim().to_ascii_lowercase();
        match value.as_str() {
            "performance" | "perf" => return Ok(Self::Performance),
            "balanced" | "balance" => return Ok(Self::Balanced),
            "lite" => return Ok(Self::Lite),
            _ => {}
        }

        let Some((kind, mib)) = value.split_once(':') else {
            return Err(RenderMemoryProfileParseError);
        };
        if kind != "manual" {
            return Err(RenderMemoryProfileParseError);
        }
        let mib = mib
            .parse::<u64>()
            .map_err(|_| RenderMemoryProfileParseError)?;
        if mib == 0 {
            return Err(RenderMemoryProfileParseError);
        }
        Ok(Self::Manual {
            bytes: mib_to_bytes(mib),
        })
    }
}

/// Error returned when a memory profile override is invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderMemoryProfileParseError;

impl fmt::Display for RenderMemoryProfileParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("expected performance, balanced, lite, or manual:<MiB>")
    }
}

impl std::error::Error for RenderMemoryProfileParseError {}

/// Hit-test picking allocation policy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PickingPolicy {
    /// Whether hit-test picking resources may be allocated.
    pub hit_test_enabled: bool,
    /// Fraction of the viewport used for hit-test picking textures.
    pub scale: f32,
    /// Whether the reprojection compute path may be allocated.
    pub reprojection_enabled: bool,
}

/// Per-frame target allocation policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameTargetPolicy {
    /// Allocate SSAO textures during renderer construction.
    pub ssao_eager: bool,
    /// Allocate color scratch during renderer construction.
    pub color_scratch_eager: bool,
    /// Number of native host textures kept in the viewport handoff ring.
    pub viewport_handoff_ring: usize,
}

/// Visual overlay allocation policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayPolicy {
    /// Whether selection and hover marking overlays may allocate resources.
    pub selection_enabled: bool,
    /// Whether silhouette overlays may allocate resources.
    pub silhouette_enabled: bool,
}

/// Full-screen postprocess policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PostprocessPolicy {
    /// Whether SSAO may allocate and run.
    pub ssao_enabled: bool,
    /// Whether FXAA may allocate and run.
    pub fxaa_enabled: bool,
}

/// Shadow and atlas resource caps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShadowPolicy {
    /// Maximum directional shadow-map size.
    pub max_shadow_map_size: u32,
    /// Maximum atlas-AO tile size.
    pub max_atlas_tile_size: u32,
    /// Maximum atlas-AO direction count.
    pub max_atlas_directions: u32,
}

/// Representation allocation policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RepresentationBudgetPolicy {
    /// Whether representation downgrade/skip decisions are active.
    pub enabled: bool,
}

/// SceneStore compaction thresholds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneStoreCompactionPolicy {
    /// Minimum orphaned bytes before compaction is considered.
    pub min_orphaned_bytes: u64,
    /// Minimum orphaned fraction of logical allocation.
    pub min_orphaned_ratio: f32,
}

impl SceneStoreCompactionPolicy {
    /// Returns `true` when fragmentation crosses the policy thresholds.
    pub fn should_compact(self, orphaned_bytes: u64, allocated_bytes: u64) -> bool {
        if allocated_bytes == 0 || orphaned_bytes < self.min_orphaned_bytes {
            return false;
        }
        (orphaned_bytes as f64 / allocated_bytes as f64) >= f64::from(self.min_orphaned_ratio)
    }
}

/// SceneStore buffer growth strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneStoreGrowthPolicy {
    /// Grow to the next power of two.
    PowerOfTwo,
    /// Grow by roughly 1.5x with a byte slack cap.
    Moderate { max_slack_bytes: u64 },
    /// Grow close to the requested size with a byte slack cap.
    Tight { max_slack_bytes: u64 },
}

/// SceneStore memory policy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneStoreMemoryPolicy {
    /// Compaction trigger thresholds.
    pub compaction: SceneStoreCompactionPolicy,
    /// Buffer growth strategy.
    pub growth: SceneStoreGrowthPolicy,
}

/// Construction-time renderer memory policy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderMemoryPolicy {
    /// Selected profile.
    pub profile: RenderMemoryProfile,
    /// Explicit byte budget for manual profiles.
    pub budget_bytes: Option<u64>,
    /// Hit-test picking resource policy.
    pub picking: PickingPolicy,
    /// Frame target resource policy.
    pub frame_targets: FrameTargetPolicy,
    /// Visual overlay policy.
    pub overlays: OverlayPolicy,
    /// Postprocess policy.
    pub postprocess: PostprocessPolicy,
    /// Shadow and atlas caps.
    pub shadows: ShadowPolicy,
    /// Future representation-budgeting policy.
    pub reps: RepresentationBudgetPolicy,
    /// SceneStore growth and compaction policy.
    pub scene_store: SceneStoreMemoryPolicy,
}

impl RenderMemoryPolicy {
    /// Returns the current high-quality policy.
    pub const fn performance() -> Self {
        Self {
            profile: RenderMemoryProfile::Performance,
            budget_bytes: None,
            picking: PickingPolicy {
                hit_test_enabled: true,
                scale: DEFAULT_PICKING_SCALE,
                reprojection_enabled: true,
            },
            frame_targets: FrameTargetPolicy {
                ssao_eager: true,
                color_scratch_eager: true,
                viewport_handoff_ring: 3,
            },
            overlays: OverlayPolicy {
                selection_enabled: true,
                silhouette_enabled: true,
            },
            postprocess: PostprocessPolicy {
                ssao_enabled: true,
                fxaa_enabled: true,
            },
            shadows: ShadowPolicy {
                max_shadow_map_size: PERFORMANCE_SHADOW_MAP_SIZE,
                max_atlas_tile_size: PERFORMANCE_SHADOW_MAP_SIZE,
                max_atlas_directions: PERFORMANCE_ATLAS_DIRECTIONS,
            },
            reps: RepresentationBudgetPolicy { enabled: false },
            scene_store: SceneStoreMemoryPolicy {
                compaction: SceneStoreCompactionPolicy {
                    min_orphaned_bytes: PERFORMANCE_SCENE_STORE_ORPHANED_BYTES,
                    min_orphaned_ratio: 0.5,
                },
                growth: SceneStoreGrowthPolicy::PowerOfTwo,
            },
        }
    }

    /// Returns a lower-baseline-memory policy.
    pub const fn balanced() -> Self {
        Self {
            profile: RenderMemoryProfile::Balanced,
            budget_bytes: None,
            picking: PickingPolicy {
                hit_test_enabled: true,
                scale: DEFAULT_PICKING_SCALE,
                reprojection_enabled: true,
            },
            frame_targets: FrameTargetPolicy {
                ssao_eager: false,
                color_scratch_eager: false,
                viewport_handoff_ring: 2,
            },
            overlays: OverlayPolicy {
                selection_enabled: true,
                silhouette_enabled: true,
            },
            postprocess: PostprocessPolicy {
                ssao_enabled: true,
                fxaa_enabled: true,
            },
            shadows: ShadowPolicy {
                max_shadow_map_size: BALANCED_SHADOW_MAP_SIZE,
                max_atlas_tile_size: 1024,
                max_atlas_directions: BALANCED_ATLAS_DIRECTIONS,
            },
            reps: RepresentationBudgetPolicy { enabled: true },
            scene_store: SceneStoreMemoryPolicy {
                compaction: SceneStoreCompactionPolicy {
                    min_orphaned_bytes: BALANCED_SCENE_STORE_ORPHANED_BYTES,
                    min_orphaned_ratio: 0.25,
                },
                growth: SceneStoreGrowthPolicy::Moderate {
                    max_slack_bytes: BALANCED_SCENE_STORE_SLACK_BYTES,
                },
            },
        }
    }

    /// Returns the constrained lite policy.
    pub const fn lite() -> Self {
        Self {
            profile: RenderMemoryProfile::Lite,
            budget_bytes: None,
            picking: PickingPolicy {
                hit_test_enabled: true,
                scale: LITE_PICKING_SCALE,
                reprojection_enabled: false,
            },
            frame_targets: FrameTargetPolicy {
                ssao_eager: false,
                color_scratch_eager: false,
                viewport_handoff_ring: 1,
            },
            overlays: OverlayPolicy {
                selection_enabled: false,
                silhouette_enabled: false,
            },
            postprocess: PostprocessPolicy {
                ssao_enabled: false,
                fxaa_enabled: false,
            },
            shadows: ShadowPolicy {
                max_shadow_map_size: LITE_SHADOW_MAP_SIZE,
                max_atlas_tile_size: 512,
                max_atlas_directions: LITE_ATLAS_DIRECTIONS,
            },
            reps: RepresentationBudgetPolicy { enabled: true },
            scene_store: SceneStoreMemoryPolicy {
                compaction: SceneStoreCompactionPolicy {
                    min_orphaned_bytes: LITE_SCENE_STORE_ORPHANED_BYTES,
                    min_orphaned_ratio: 0.1,
                },
                growth: SceneStoreGrowthPolicy::Tight {
                    max_slack_bytes: LITE_SCENE_STORE_SLACK_BYTES,
                },
            },
        }
    }

    /// Returns the constrained policy with an explicit representation budget.
    pub const fn manual(bytes: u64) -> Self {
        let mut policy = Self::lite();
        policy.profile = RenderMemoryProfile::Manual { bytes };
        policy.budget_bytes = Some(bytes);
        policy.reps.enabled = true;
        let budget_threshold = bytes / 20;
        let min_orphaned_bytes = if budget_threshold < MANUAL_SCENE_STORE_ORPHANED_FLOOR_BYTES {
            MANUAL_SCENE_STORE_ORPHANED_FLOOR_BYTES
        } else if budget_threshold > LITE_SCENE_STORE_ORPHANED_BYTES {
            LITE_SCENE_STORE_ORPHANED_BYTES
        } else {
            budget_threshold
        };
        policy.scene_store.compaction = SceneStoreCompactionPolicy {
            min_orphaned_bytes,
            min_orphaned_ratio: 0.1,
        };
        policy
    }

    /// Returns policy for a profile.
    pub const fn from_profile(profile: RenderMemoryProfile) -> Self {
        match profile {
            RenderMemoryProfile::Performance => Self::performance(),
            RenderMemoryProfile::Balanced => Self::balanced(),
            RenderMemoryProfile::Lite => Self::lite(),
            RenderMemoryProfile::Manual { bytes } => Self::manual(bytes),
        }
    }

    /// Returns the preferred `wgpu` memory hint for this policy.
    pub const fn wgpu_memory_hints(self) -> wgpu::MemoryHints {
        match self.profile {
            RenderMemoryProfile::Performance => wgpu::MemoryHints::Performance,
            RenderMemoryProfile::Balanced
            | RenderMemoryProfile::Lite
            | RenderMemoryProfile::Manual { .. } => wgpu::MemoryHints::MemoryUsage,
        }
    }
}

impl Default for RenderMemoryPolicy {
    fn default() -> Self {
        Self::performance()
    }
}

/// Adapter class used by policy selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderAdapterType {
    /// Unknown or uncategorized adapter.
    Other,
    /// Integrated GPU.
    IntegratedGpu,
    /// Discrete GPU.
    DiscreteGpu,
    /// Virtual GPU.
    VirtualGpu,
    /// CPU fallback adapter.
    Cpu,
}

impl From<wgpu::DeviceType> for RenderAdapterType {
    fn from(value: wgpu::DeviceType) -> Self {
        match value {
            wgpu::DeviceType::IntegratedGpu => Self::IntegratedGpu,
            wgpu::DeviceType::DiscreteGpu => Self::DiscreteGpu,
            wgpu::DeviceType::VirtualGpu => Self::VirtualGpu,
            wgpu::DeviceType::Cpu => Self::Cpu,
            wgpu::DeviceType::Other => Self::Other,
        }
    }
}

/// Backend class used by policy selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderBackend {
    /// Native Vulkan backend.
    Vulkan,
    /// Native Metal backend.
    Metal,
    /// Native DirectX 12 backend.
    Dx12,
    /// Browser WebGPU backend.
    BrowserWebGpu,
    /// Browser WebGL backend.
    BrowserWebGl,
    /// OpenGL backend.
    Gl,
    /// Unknown backend.
    Other,
}

impl From<wgpu::Backend> for RenderBackend {
    fn from(value: wgpu::Backend) -> Self {
        match value {
            wgpu::Backend::Vulkan => Self::Vulkan,
            wgpu::Backend::Metal => Self::Metal,
            wgpu::Backend::Dx12 => Self::Dx12,
            wgpu::Backend::BrowserWebGpu => Self::BrowserWebGpu,
            wgpu::Backend::Gl => Self::Gl,
            _ => Self::Other,
        }
    }
}

/// Synthetic adapter inputs for profile selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderMemorySelectionInput {
    /// Adapter type.
    pub adapter_type: RenderAdapterType,
    /// Adapter backend.
    pub backend: RenderBackend,
    /// Adapter maximum buffer size.
    pub max_buffer_size: u64,
    /// Adapter maximum storage-buffer binding size.
    pub max_storage_buffer_binding_size: u64,
    /// Adapter maximum two-dimensional texture dimension.
    pub max_texture_dimension_2d: u32,
    /// Whether this is a browser-hosted renderer.
    pub is_web: bool,
    /// Whether prior runtime state requested a downgrade.
    pub observed_downgrade: bool,
}

impl RenderMemorySelectionInput {
    /// Builds selector input from `wgpu` adapter data.
    pub fn from_wgpu(info: &wgpu::AdapterInfo, limits: &wgpu::Limits, is_web: bool) -> Self {
        Self {
            adapter_type: info.device_type.into(),
            backend: info.backend.into(),
            max_buffer_size: limits.max_buffer_size,
            max_storage_buffer_binding_size: limits.max_storage_buffer_binding_size,
            max_texture_dimension_2d: limits.max_texture_dimension_2d,
            is_web,
            observed_downgrade: false,
        }
    }
}

/// Selects a memory policy from adapter facts and an optional override.
pub fn select_render_memory_policy(
    input: RenderMemorySelectionInput,
    override_profile: Option<RenderMemoryProfile>,
) -> RenderMemoryPolicy {
    select_render_memory_policy_with_budget(input, None, override_profile)
}

/// Selects a profile using native memory headroom and separate resource limits.
///
/// Explicit overrides win. Without a usable native budget, retain the existing
/// capability-based fallback. The advisory snapshot is sampled at startup;
/// runtime allocation failures still use the renderer's OOM recovery policy.
pub fn select_render_memory_policy_with_budget(
    input: RenderMemorySelectionInput,
    budget: Option<NativeGpuMemoryBudget>,
    override_profile: Option<RenderMemoryProfile>,
) -> RenderMemoryPolicy {
    if let Some(profile) = override_profile {
        return RenderMemoryPolicy::from_profile(profile);
    }
    if input.observed_downgrade
        || input.adapter_type == RenderAdapterType::Cpu
        || input.adapter_type == RenderAdapterType::VirtualGpu
        || input.max_buffer_size < LITE_BUFFER_THRESHOLD
        || input.max_storage_buffer_binding_size < LITE_STORAGE_BINDING_THRESHOLD
        || input.max_texture_dimension_2d < 8192
    {
        return RenderMemoryPolicy::lite();
    }
    if input.is_web
        || matches!(
            input.backend,
            RenderBackend::BrowserWebGpu | RenderBackend::BrowserWebGl
        )
    {
        return RenderMemoryPolicy::balanced();
    }
    if let Some(budget) = budget.filter(|budget| budget.budget_bytes > 0) {
        let headroom = budget
            .headroom_bytes()
            .saturating_sub(budget.budget_bytes / 4);
        if headroom < BALANCED_MEMORY_HEADROOM {
            return RenderMemoryPolicy::lite();
        }
        if headroom < PERFORMANCE_MEMORY_HEADROOM
            || input.max_buffer_size < PERFORMANCE_MIN_BUFFER_SIZE
            || input.max_storage_buffer_binding_size < PERFORMANCE_MAX_STORAGE_BUFFER_BINDING_SIZE
        {
            return RenderMemoryPolicy::balanced();
        }
        return RenderMemoryPolicy::performance();
    }
    if input.max_buffer_size < PERFORMANCE_MAX_BUFFER_SIZE
        || input.max_storage_buffer_binding_size < PERFORMANCE_MAX_STORAGE_BUFFER_BINDING_SIZE
    {
        return RenderMemoryPolicy::balanced();
    }
    RenderMemoryPolicy::performance()
}

/// Resolves user settings into an effective memory policy.
///
/// `auto_policy` is the startup policy selected from adapter facts and
/// environment or host initialization overrides.
pub fn render_memory_policy_from_settings(
    settings: &patinae_settings::Settings,
    auto_policy: RenderMemoryPolicy,
) -> RenderMemoryPolicy {
    match settings.renderer.memory_profile {
        patinae_settings::RenderMemoryProfileSetting::Auto => auto_policy,
        patinae_settings::RenderMemoryProfileSetting::Performance => {
            RenderMemoryPolicy::performance()
        }
        patinae_settings::RenderMemoryProfileSetting::Balanced => RenderMemoryPolicy::balanced(),
        patinae_settings::RenderMemoryProfileSetting::Lite => RenderMemoryPolicy::lite(),
        patinae_settings::RenderMemoryProfileSetting::Manual => {
            match u64::try_from(settings.renderer.memory_budget_mib) {
                Ok(mib) if mib > 0 => RenderMemoryPolicy::manual(mib_to_bytes(mib)),
                _ => auto_policy,
            }
        }
    }
}

/// Builds `wgpu` required limits for a memory policy.
pub fn required_limits_for_memory_policy(
    adapter_limits: &wgpu::Limits,
    _policy: RenderMemoryPolicy,
) -> wgpu::Limits {
    wgpu::Limits {
        max_buffer_size: PERFORMANCE_MAX_BUFFER_SIZE.min(adapter_limits.max_buffer_size),
        max_storage_buffer_binding_size: PERFORMANCE_MAX_STORAGE_BUFFER_BINDING_SIZE
            .min(adapter_limits.max_storage_buffer_binding_size),
        max_storage_buffers_per_shader_stage: RENDER_COMPUTE_STORAGE_BUFFERS_PER_STAGE
            .min(adapter_limits.max_storage_buffers_per_shader_stage),
        ..wgpu::Limits::default()
    }
    .using_resolution(adapter_limits.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::byte_units::{gib_to_bytes, mib_to_bytes};

    fn native_budget(budget: u64, usage: u64) -> NativeGpuMemoryBudget {
        NativeGpuMemoryBudget {
            budget_bytes: budget,
            usage_bytes: usage,
            source: crate::NativeMemoryBudgetSource::MetalWorkingSet,
        }
    }

    #[test]
    fn native_budget_selects_performance_under_indirect_validation_limits() {
        let adapter = input(
            RenderAdapterType::IntegratedGpu,
            u64::from(u32::MAX),
            PERFORMANCE_MAX_STORAGE_BUFFER_BINDING_SIZE,
        );
        let budget = native_budget(gib_to_bytes(100), gib_to_bytes(1));
        let policy = select_render_memory_policy_with_budget(adapter, Some(budget), None);
        assert_eq!(policy.profile, RenderMemoryProfile::Performance);
        let required = required_limits_for_memory_policy(
            &limits(
                adapter.max_buffer_size,
                adapter.max_storage_buffer_binding_size,
            ),
            policy,
        );
        assert_eq!(required.max_buffer_size, u64::from(u32::MAX));
        assert_eq!(
            required.max_storage_buffer_binding_size,
            adapter.max_storage_buffer_binding_size
        );
    }

    #[test]
    fn native_headroom_reserves_margin_and_accounts_for_process_usage() {
        let adapter = input(RenderAdapterType::DiscreteGpu, gib_to_bytes(8), u64::MAX);
        for (total, used, expected) in [
            (16, 4, RenderMemoryProfile::Performance),
            (16, 5, RenderMemoryProfile::Balanced),
            (8, 4, RenderMemoryProfile::Balanced),
            (8, 5, RenderMemoryProfile::Lite),
            (8, 9, RenderMemoryProfile::Lite),
        ] {
            let policy = select_render_memory_policy_with_budget(
                adapter,
                Some(native_budget(gib_to_bytes(total), gib_to_bytes(used))),
                None,
            );
            assert_eq!(policy.profile, expected, "budget={total} usage={used}");
        }
    }

    #[test]
    fn native_budget_does_not_override_explicit_profiles_or_resource_limits() {
        let adapter = input(RenderAdapterType::IntegratedGpu, gib_to_bytes(8), u64::MAX);
        for profile in [
            RenderMemoryProfile::Balanced,
            RenderMemoryProfile::Lite,
            RenderMemoryProfile::Performance,
            RenderMemoryProfile::Manual {
                bytes: gib_to_bytes(3),
            },
        ] {
            assert_eq!(
                select_render_memory_policy_with_budget(
                    adapter,
                    Some(native_budget(gib_to_bytes(1), 0)),
                    Some(profile)
                )
                .profile,
                profile
            );
        }
        let constrained = input(
            RenderAdapterType::IntegratedGpu,
            gib_to_bytes(2),
            gib_to_bytes(1),
        );
        assert_eq!(
            select_render_memory_policy_with_budget(
                constrained,
                Some(native_budget(gib_to_bytes(100), 0)),
                None
            )
            .profile,
            RenderMemoryProfile::Balanced
        );
    }

    #[test]
    fn missing_native_budget_preserves_fallback_and_oom_downgrade() {
        let mut adapter = input(
            RenderAdapterType::IntegratedGpu,
            u64::from(u32::MAX),
            u64::MAX,
        );
        for budget in [None, Some(native_budget(0, 0))] {
            assert_eq!(
                select_render_memory_policy_with_budget(adapter, budget, None).profile,
                RenderMemoryProfile::Balanced
            );
        }
        adapter.observed_downgrade = true;
        assert_eq!(
            select_render_memory_policy_with_budget(
                adapter,
                Some(native_budget(gib_to_bytes(100), 0)),
                None
            )
            .profile,
            RenderMemoryProfile::Lite
        );
    }

    fn limits(max_buffer_size: u64, max_storage_buffer_binding_size: u64) -> wgpu::Limits {
        wgpu::Limits {
            max_buffer_size,
            max_storage_buffer_binding_size,
            max_storage_buffers_per_shader_stage: 32,
            max_texture_dimension_2d: 16_384,
            ..wgpu::Limits::default()
        }
    }

    fn input(
        adapter_type: RenderAdapterType,
        max_buffer_size: u64,
        max_storage_buffer_binding_size: u64,
    ) -> RenderMemorySelectionInput {
        RenderMemorySelectionInput {
            adapter_type,
            backend: RenderBackend::Metal,
            max_buffer_size,
            max_storage_buffer_binding_size,
            max_texture_dimension_2d: 16_384,
            is_web: false,
            observed_downgrade: false,
        }
    }

    #[test]
    fn profile_override_parses_expected_values() {
        assert_eq!(
            "performance".parse::<RenderMemoryProfile>(),
            Ok(RenderMemoryProfile::Performance)
        );
        assert_eq!(
            "balanced".parse::<RenderMemoryProfile>(),
            Ok(RenderMemoryProfile::Balanced)
        );
        assert_eq!(
            "lite".parse::<RenderMemoryProfile>(),
            Ok(RenderMemoryProfile::Lite)
        );
        assert_eq!(
            "manual:1024".parse::<RenderMemoryProfile>(),
            Ok(RenderMemoryProfile::Manual {
                bytes: gib_to_bytes(1)
            })
        );
    }

    #[test]
    fn profile_override_rejects_legacy_profile_values() {
        let legacy_values = [
            ["lo", "w"].concat(),
            ["lo", "w-memory"].concat(),
            ["lo", "w_memory"].concat(),
            ["lo", "w:1024"].concat(),
            ["li", "te:1024"].concat(),
            ["bud", "get"].concat(),
            ["bud", "geted"].concat(),
        ];

        for value in legacy_values {
            assert_eq!(
                value.parse::<RenderMemoryProfile>(),
                Err(RenderMemoryProfileParseError)
            );
        }
    }

    #[test]
    fn high_end_adapter_selects_performance() {
        let policy = select_render_memory_policy(
            input(RenderAdapterType::IntegratedGpu, gib_to_bytes(8), u64::MAX),
            None,
        );

        assert_eq!(policy.profile, RenderMemoryProfile::Performance);
        assert_eq!(policy.frame_targets.viewport_handoff_ring, 3);
        assert!(policy.postprocess.fxaa_enabled);
        assert!(policy.postprocess.ssao_enabled);
    }

    #[test]
    fn moderate_adapter_selects_balanced() {
        let policy = select_render_memory_policy(
            input(
                RenderAdapterType::IntegratedGpu,
                gib_to_bytes(2),
                gib_to_bytes(1),
            ),
            None,
        );

        assert_eq!(policy.profile, RenderMemoryProfile::Balanced);
        assert!(!policy.frame_targets.ssao_eager);
        assert!(!policy.frame_targets.color_scratch_eager);
        assert_eq!(policy.frame_targets.viewport_handoff_ring, 2);
    }

    #[test]
    fn constrained_adapter_selects_lite() {
        let policy = select_render_memory_policy(
            input(
                RenderAdapterType::IntegratedGpu,
                mib_to_bytes(512),
                mib_to_bytes(256),
            ),
            None,
        );

        assert_eq!(policy.profile, RenderMemoryProfile::Lite);
        assert!(policy.picking.hit_test_enabled);
        assert!(!policy.picking.reprojection_enabled);
        assert!(!policy.postprocess.fxaa_enabled);
        assert!(!policy.overlays.selection_enabled);
        assert_eq!(policy.frame_targets.viewport_handoff_ring, 1);
    }

    #[test]
    fn forced_lite_changes_policy_without_weak_hardware() {
        let policy = select_render_memory_policy(
            input(RenderAdapterType::DiscreteGpu, gib_to_bytes(8), u64::MAX),
            Some(RenderMemoryProfile::Lite),
        );

        assert_eq!(policy.profile, RenderMemoryProfile::Lite);
        assert!(!policy.postprocess.ssao_enabled);
        assert!(policy.reps.enabled);
    }

    #[test]
    fn performance_keeps_rep_budgeting_pass_through() {
        let policy = RenderMemoryPolicy::performance();

        assert!(!policy.reps.enabled);
        assert_eq!(
            policy.scene_store.compaction.min_orphaned_bytes,
            mib_to_bytes(512)
        );
        assert_eq!(policy.scene_store.compaction.min_orphaned_ratio, 0.5);
        assert_eq!(
            policy.scene_store.growth,
            SceneStoreGrowthPolicy::PowerOfTwo
        );
    }

    #[test]
    fn manual_profile_enforces_rep_budgeting_even_on_large_budget() {
        let policy = RenderMemoryPolicy::manual(gib_to_bytes(8));

        assert_eq!(
            policy.profile,
            RenderMemoryProfile::Manual {
                bytes: gib_to_bytes(8)
            }
        );
        assert!(policy.reps.enabled);
        assert!(!policy.postprocess.fxaa_enabled);
        assert!(!policy.postprocess.ssao_enabled);
        assert!(policy.picking.hit_test_enabled);
        assert!(!policy.picking.reprojection_enabled);
        assert!(!policy.overlays.selection_enabled);
        assert_eq!(
            policy.scene_store.compaction.min_orphaned_bytes,
            mib_to_bytes(32)
        );
        assert_eq!(policy.scene_store.compaction.min_orphaned_ratio, 0.1);
        assert_eq!(
            policy.scene_store.growth,
            SceneStoreGrowthPolicy::Tight {
                max_slack_bytes: mib_to_bytes(16)
            }
        );
    }

    #[test]
    fn lite_scene_store_policy_uses_aggressive_compaction() {
        let policy = RenderMemoryPolicy::lite();

        assert_eq!(
            policy.scene_store.compaction.min_orphaned_bytes,
            mib_to_bytes(32)
        );
        assert!(policy
            .scene_store
            .compaction
            .should_compact(mib_to_bytes(32), mib_to_bytes(256)));
        assert!(!policy
            .scene_store
            .compaction
            .should_compact(mib_to_bytes(16), mib_to_bytes(256)));
    }

    #[test]
    fn small_budget_caps_scene_store_threshold_at_budget_fraction() {
        let policy = RenderMemoryPolicy::manual(mib_to_bytes(128));

        assert_eq!(
            policy.scene_store.compaction.min_orphaned_bytes,
            mib_to_bytes(8)
        );
    }

    #[test]
    fn required_limits_never_exceed_adapter_limits() {
        let adapter = limits(mib_to_bytes(512), mib_to_bytes(384));
        let required = required_limits_for_memory_policy(&adapter, RenderMemoryPolicy::lite());

        assert!(required.max_buffer_size <= adapter.max_buffer_size);
        assert!(
            required.max_storage_buffer_binding_size <= adapter.max_storage_buffer_binding_size
        );
        assert!(
            required.max_storage_buffers_per_shader_stage
                <= adapter.max_storage_buffers_per_shader_stage
        );
        assert_eq!(required.max_texture_dimension_2d, 16_384);
    }

    #[test]
    fn required_limits_preserve_performance_targets_on_capable_adapter() {
        let adapter = limits(gib_to_bytes(8), u64::MAX);
        let required =
            required_limits_for_memory_policy(&adapter, RenderMemoryPolicy::performance());

        assert_eq!(required.max_buffer_size, PERFORMANCE_MAX_BUFFER_SIZE);
        assert_eq!(
            required.max_storage_buffer_binding_size,
            PERFORMANCE_MAX_STORAGE_BUFFER_BINDING_SIZE
        );
        assert_eq!(
            required.max_storage_buffers_per_shader_stage,
            RENDER_COMPUTE_STORAGE_BUFFERS_PER_STAGE
        );
    }

    #[test]
    fn required_limits_raise_compute_storage_buffer_binding_count() {
        let adapter = limits(gib_to_bytes(8), u64::MAX);

        let required =
            required_limits_for_memory_policy(&adapter, RenderMemoryPolicy::performance());

        assert_eq!(
            required.max_storage_buffers_per_shader_stage,
            RENDER_COMPUTE_STORAGE_BUFFERS_PER_STAGE
        );
        assert!(
            required.max_storage_buffers_per_shader_stage
                > wgpu::Limits::default().max_storage_buffers_per_shader_stage
        );
    }

    #[test]
    fn required_limits_clamp_compute_storage_buffer_binding_count() {
        let mut adapter = limits(gib_to_bytes(8), u64::MAX);
        adapter.max_storage_buffers_per_shader_stage = 9;

        let required =
            required_limits_for_memory_policy(&adapter, RenderMemoryPolicy::performance());

        assert_eq!(required.max_storage_buffers_per_shader_stage, 9);
    }

    #[test]
    fn render_memory_settings_auto_uses_startup_policy() {
        let settings = patinae_settings::Settings::default();
        let policy = render_memory_policy_from_settings(&settings, RenderMemoryPolicy::balanced());

        assert_eq!(policy.profile, RenderMemoryProfile::Balanced);
    }

    #[test]
    fn render_memory_settings_budget_value_does_not_force_manual() {
        let mut settings = patinae_settings::Settings::default();
        settings.renderer.memory_budget_mib = 1024;

        let policy = render_memory_policy_from_settings(&settings, RenderMemoryPolicy::balanced());

        assert_eq!(policy.profile, RenderMemoryProfile::Balanced);
    }

    #[test]
    fn render_memory_settings_explicit_profile_ignores_stale_budget() {
        let mut settings = patinae_settings::Settings::default();
        settings.renderer.memory_profile = patinae_settings::RenderMemoryProfileSetting::Lite;
        settings.renderer.memory_budget_mib = 1024;

        let policy =
            render_memory_policy_from_settings(&settings, RenderMemoryPolicy::performance());

        assert_eq!(policy.profile, RenderMemoryProfile::Lite);
    }

    #[test]
    fn render_memory_settings_manual_uses_mib_budget() {
        let mut settings = patinae_settings::Settings::default();
        settings.renderer.memory_profile = patinae_settings::RenderMemoryProfileSetting::Manual;
        settings.renderer.memory_budget_mib = 1024;

        let policy =
            render_memory_policy_from_settings(&settings, RenderMemoryPolicy::performance());

        assert_eq!(
            policy.profile,
            RenderMemoryProfile::Manual {
                bytes: gib_to_bytes(1)
            }
        );
    }

    #[test]
    fn render_memory_settings_manual_without_budget_falls_back_to_auto() {
        let mut settings = patinae_settings::Settings::default();
        settings.renderer.memory_profile = patinae_settings::RenderMemoryProfileSetting::Manual;

        let policy = render_memory_policy_from_settings(&settings, RenderMemoryPolicy::balanced());

        assert_eq!(policy.profile, RenderMemoryProfile::Balanced);
    }
}
