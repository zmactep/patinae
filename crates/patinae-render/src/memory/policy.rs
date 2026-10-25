//! Renderer memory profile selection.
//!
//! Profiles expand into resource policies and one shared allocation budget here.
//! `selection` makes pure startup decisions from an optional native snapshot;
//! `native` owns platform queries and `device_limits` owns hard device
//! requirements. Planners enforce each allocation; `recovery` owns OOM
//! transitions separately from the user's requested profile.

use std::fmt;
use std::str::FromStr;

use crate::memory::{gib_to_bytes, mib_to_bytes, BYTES_PER_MIB};

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
    /// Optional allocation budget shared by representation and picking planning.
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
            budget_bytes: Some(gib_to_bytes(2)),
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
            budget_bytes: Some(gib_to_bytes(1)),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{gib_to_bytes, mib_to_bytes};
    use crate::memory::{
        select_render_memory_policy, RenderMemorySelectionInput, RenderMemorySelectionReason,
    };
    use crate::{
        required_device_limits, NativeGpuMemoryBudget, DEVICE_STORAGE_BINDING_TARGET_BYTES,
    };

    fn native_budget(budget: u64, usage: u64) -> NativeGpuMemoryBudget {
        NativeGpuMemoryBudget {
            budget_bytes: budget,
            usage_bytes: usage,
            source: crate::NativeMemoryBudgetSource::MetalWorkingSet,
        }
    }

    #[test]
    fn selection_reason_and_diagnostics_share_the_policy_calculation() {
        let adapter = input(
            wgpu::DeviceType::DiscreteGpu,
            gib_to_bytes(4),
            gib_to_bytes(2),
        );
        for source in [
            crate::NativeMemoryBudgetSource::MetalWorkingSet,
            crate::NativeMemoryBudgetSource::VulkanHeap,
            crate::NativeMemoryBudgetSource::DxgiLocal,
        ] {
            let budget =
                NativeGpuMemoryBudget::new(gib_to_bytes(16), gib_to_bytes(4), source).unwrap();
            let selection = select_render_memory_policy(adapter, Some(budget), None);
            assert_eq!(selection.reason, RenderMemorySelectionReason::NativeBudget);
            assert_eq!(selection.policy.profile, RenderMemoryProfile::Performance);
            let headroom = selection.headroom.unwrap();
            assert_eq!(headroom.budget, budget);
            assert_eq!(headroom.reserved_bytes, gib_to_bytes(4));
            assert_eq!(headroom.available_bytes, gib_to_bytes(8));

            let selection =
                select_render_memory_policy(adapter, Some(budget), Some(RenderMemoryProfile::Lite));
            assert_eq!(
                selection.reason,
                RenderMemorySelectionReason::ExplicitOverride
            );
            assert_eq!(selection.headroom, Some(headroom));
            assert_eq!(selection.policy.profile, RenderMemoryProfile::Lite);
        }
        for budget in [None, Some(native_budget(0, u64::MAX))] {
            let selection = select_render_memory_policy(adapter, budget, None);
            assert_eq!(
                selection.reason,
                RenderMemorySelectionReason::CapabilityFallback
            );
            assert!(selection.headroom.is_none());
        }
        let selection =
            select_render_memory_policy(adapter, Some(native_budget(u64::MAX, u64::MAX)), None);
        assert_eq!(selection.policy.profile, RenderMemoryProfile::Lite);
        assert_eq!(selection.headroom.unwrap().available_bytes, 0);
    }

    #[test]
    fn profiles_define_one_allocation_budget_for_all_planners() {
        for (profile, expected) in [
            (RenderMemoryProfile::Performance, None),
            (RenderMemoryProfile::Balanced, Some(gib_to_bytes(2))),
            (RenderMemoryProfile::Lite, Some(gib_to_bytes(1))),
            (RenderMemoryProfile::Manual { bytes: 1234 }, Some(1234)),
        ] {
            assert_eq!(
                RenderMemoryPolicy::from_profile(profile).budget_bytes,
                expected
            );
        }
    }

    #[test]
    fn dx12_reported_budget_selects_performance_with_two_gib_resource_limits() {
        let mut adapter = input(
            wgpu::DeviceType::DiscreteGpu,
            i32::MAX as u64,
            i32::MAX as u64,
        );
        adapter.backend = wgpu::Backend::Dx12;
        let budget = NativeGpuMemoryBudget {
            budget_bytes: mib_to_bytes(15_227),
            usage_bytes: mib_to_bytes(99),
            source: crate::NativeMemoryBudgetSource::DxgiLocal,
        };
        let policy = select_render_memory_policy(adapter, Some(budget), None).policy;
        assert_eq!(policy.profile, RenderMemoryProfile::Performance);
        assert!(policy.overlays.selection_enabled);
        assert!(policy.picking.reprojection_enabled);
        assert!(policy.postprocess.fxaa_enabled);
        assert_eq!(
            select_render_memory_policy(adapter, None, None)
                .policy
                .profile,
            RenderMemoryProfile::Balanced
        );
    }

    #[test]
    fn native_budget_thresholds_include_boundary_and_exclude_one_byte_below() {
        let adapter = input(
            wgpu::DeviceType::DiscreteGpu,
            i32::MAX as u64,
            i32::MAX as u64,
        );
        for (used, expected) in [
            (gib_to_bytes(4), RenderMemoryProfile::Performance),
            (gib_to_bytes(4) + 1, RenderMemoryProfile::Balanced),
            (gib_to_bytes(10), RenderMemoryProfile::Balanced),
            (gib_to_bytes(10) + 1, RenderMemoryProfile::Lite),
        ] {
            assert_eq!(
                select_render_memory_policy(
                    adapter,
                    Some(native_budget(gib_to_bytes(16), used)),
                    None
                )
                .policy
                .profile,
                expected
            );
        }
    }

    #[test]
    fn native_budget_selects_profiles_independently_of_resource_limits() {
        for (buffer, storage) in [
            (mib_to_bytes(256), mib_to_bytes(128)),
            (gib_to_bytes(2), gib_to_bytes(1)),
            (gib_to_bytes(4), gib_to_bytes(2)),
        ] {
            let mut adapter = input(wgpu::DeviceType::IntegratedGpu, buffer, storage);
            adapter.max_texture_dimension_2d = 4096;
            for (used, expected) in [
                (gib_to_bytes(4), RenderMemoryProfile::Performance),
                (gib_to_bytes(4) + 1, RenderMemoryProfile::Balanced),
                (gib_to_bytes(10), RenderMemoryProfile::Balanced),
                (gib_to_bytes(10) + 1, RenderMemoryProfile::Lite),
            ] {
                assert_eq!(
                    select_render_memory_policy(
                        adapter,
                        Some(native_budget(gib_to_bytes(16), used)),
                        None
                    )
                    .policy
                    .profile,
                    expected,
                    "buffer={buffer} storage={storage} usage={used}"
                );
            }
            for budget in [None, Some(native_budget(0, 0))] {
                assert_eq!(
                    select_render_memory_policy(adapter, budget, None)
                        .policy
                        .profile,
                    RenderMemoryProfile::Lite
                );
            }
        }
    }

    #[test]
    fn explicit_performance_is_independent_of_resource_limits_and_budget() {
        let adapter = input(
            wgpu::DeviceType::DiscreteGpu,
            mib_to_bytes(256),
            mib_to_bytes(128),
        );
        for budget in [None, Some(native_budget(gib_to_bytes(1), 0))] {
            assert_eq!(
                select_render_memory_policy(
                    adapter,
                    budget,
                    Some(RenderMemoryProfile::Performance)
                )
                .policy
                .profile,
                RenderMemoryProfile::Performance
            );
        }
    }

    #[test]
    fn native_budget_selects_performance_under_indirect_validation_limits() {
        let adapter = input(
            wgpu::DeviceType::IntegratedGpu,
            u64::from(u32::MAX),
            DEVICE_STORAGE_BINDING_TARGET_BYTES,
        );
        let budget = native_budget(gib_to_bytes(100), gib_to_bytes(1));
        let policy = select_render_memory_policy(adapter, Some(budget), None).policy;
        assert_eq!(policy.profile, RenderMemoryProfile::Performance);
        let required = required_device_limits(&limits(
            adapter.max_buffer_size,
            adapter.max_storage_buffer_binding_size,
        ))
        .expect("GPU must support renderer compute requirements");
        assert_eq!(required.max_buffer_size, u64::from(u32::MAX));
        assert_eq!(
            required.max_storage_buffer_binding_size,
            adapter.max_storage_buffer_binding_size
        );
    }

    #[test]
    fn native_headroom_reserves_margin_and_accounts_for_process_usage() {
        let adapter = input(wgpu::DeviceType::DiscreteGpu, gib_to_bytes(8), u64::MAX);
        for (total, used, expected) in [
            (16, 4, RenderMemoryProfile::Performance),
            (16, 5, RenderMemoryProfile::Balanced),
            (8, 4, RenderMemoryProfile::Balanced),
            (8, 5, RenderMemoryProfile::Lite),
            (8, 9, RenderMemoryProfile::Lite),
        ] {
            let policy = select_render_memory_policy(
                adapter,
                Some(native_budget(gib_to_bytes(total), gib_to_bytes(used))),
                None,
            )
            .policy;
            assert_eq!(policy.profile, expected, "budget={total} usage={used}");
        }
    }

    #[test]
    fn native_budget_does_not_override_explicit_profiles() {
        let adapter = input(wgpu::DeviceType::IntegratedGpu, gib_to_bytes(8), u64::MAX);
        for profile in [
            RenderMemoryProfile::Balanced,
            RenderMemoryProfile::Lite,
            RenderMemoryProfile::Performance,
            RenderMemoryProfile::Manual {
                bytes: gib_to_bytes(3),
            },
        ] {
            assert_eq!(
                select_render_memory_policy(
                    adapter,
                    Some(native_budget(gib_to_bytes(1), 0)),
                    Some(profile)
                )
                .policy
                .profile,
                profile
            );
        }
        let constrained = input(
            wgpu::DeviceType::IntegratedGpu,
            gib_to_bytes(2),
            gib_to_bytes(1),
        );
        assert_eq!(
            select_render_memory_policy(
                constrained,
                Some(native_budget(gib_to_bytes(100), 0)),
                None
            )
            .policy
            .profile,
            RenderMemoryProfile::Performance
        );
    }

    #[test]
    fn missing_native_budget_preserves_capability_fallback() {
        let adapter = input(
            wgpu::DeviceType::IntegratedGpu,
            u64::from(u32::MAX),
            u64::MAX,
        );
        for budget in [None, Some(native_budget(0, 0))] {
            assert_eq!(
                select_render_memory_policy(adapter, budget, None)
                    .policy
                    .profile,
                RenderMemoryProfile::Balanced
            );
        }
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
        adapter_type: wgpu::DeviceType,
        max_buffer_size: u64,
        max_storage_buffer_binding_size: u64,
    ) -> RenderMemorySelectionInput {
        RenderMemorySelectionInput {
            adapter_type,
            backend: wgpu::Backend::Metal,
            max_buffer_size,
            max_storage_buffer_binding_size,
            max_texture_dimension_2d: 16_384,
            is_web: false,
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
            input(wgpu::DeviceType::IntegratedGpu, gib_to_bytes(8), u64::MAX),
            None,
            None,
        )
        .policy;

        assert_eq!(policy.profile, RenderMemoryProfile::Performance);
        assert_eq!(policy.frame_targets.viewport_handoff_ring, 3);
        assert!(policy.postprocess.fxaa_enabled);
        assert!(policy.postprocess.ssao_enabled);
    }

    #[test]
    fn moderate_adapter_selects_balanced() {
        let policy = select_render_memory_policy(
            input(
                wgpu::DeviceType::IntegratedGpu,
                gib_to_bytes(2),
                gib_to_bytes(1),
            ),
            None,
            None,
        )
        .policy;

        assert_eq!(policy.profile, RenderMemoryProfile::Balanced);
        assert!(!policy.frame_targets.ssao_eager);
        assert!(!policy.frame_targets.color_scratch_eager);
        assert_eq!(policy.frame_targets.viewport_handoff_ring, 2);
    }

    #[test]
    fn constrained_adapter_selects_lite() {
        let policy = select_render_memory_policy(
            input(
                wgpu::DeviceType::IntegratedGpu,
                mib_to_bytes(512),
                mib_to_bytes(256),
            ),
            None,
            None,
        )
        .policy;

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
            input(wgpu::DeviceType::DiscreteGpu, gib_to_bytes(8), u64::MAX),
            None,
            Some(RenderMemoryProfile::Lite),
        )
        .policy;

        assert_eq!(policy.profile, RenderMemoryProfile::Lite);
        assert!(!policy.postprocess.ssao_enabled);
        assert!(policy.budget_bytes.is_some());
    }

    #[test]
    fn performance_keeps_rep_budgeting_pass_through() {
        let policy = RenderMemoryPolicy::performance();

        assert!(!policy.budget_bytes.is_some());
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
        assert!(policy.budget_bytes.is_some());
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
