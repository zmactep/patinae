use crate::frame::{DEPTH_FORMAT, PICKING_FORMAT};
use crate::memory::{estimate_texture_2d_bytes, GpuMemoryCategory, GpuMemorySnapshot};
use crate::memory::{RenderMemoryPolicy, RenderMemoryProfile};
use crate::picking::pass::PickingParams;
use crate::picking::readback::PICKING_READBACK_BUFFER_BYTES;

const PICKING_ATOM_PRESSURE_BYTES: u64 = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PickingBudgetDecision {
    pub(super) allowed: bool,
    pub(super) fixed_reserved_bytes: u64,
    pub(super) estimated_bytes: u64,
    pub(super) budget_bytes: Option<u64>,
    pub(super) total_atoms: u64,
    pub(super) active_rep_count: usize,
}

pub(super) fn uses_lazy_manual_picking(policy: RenderMemoryPolicy) -> bool {
    policy.picking.hit_test_enabled
        && matches!(
            policy.profile,
            RenderMemoryProfile::Lite | RenderMemoryProfile::Manual { .. }
        )
}

pub(super) fn effective_picking_budget_bytes(policy: RenderMemoryPolicy) -> Option<u64> {
    if !policy.picking.hit_test_enabled {
        return Some(0);
    }
    policy.budget_bytes
}

pub(super) fn fixed_reserved_without_picking(snapshot: &GpuMemorySnapshot) -> u64 {
    let replaceable = snapshot
        .category_usage(GpuMemoryCategory::Picking)
        .capacity_bytes
        .saturating_add(
            snapshot
                .category_usage(GpuMemoryCategory::Readback)
                .capacity_bytes,
        );
    snapshot.total_capacity_bytes().saturating_sub(replaceable)
}

pub(super) fn plan_picking_budget(
    policy: RenderMemoryPolicy,
    fixed_reserved_bytes: u64,
    viewport: (u32, u32),
    active_rep_count: usize,
    total_atoms: u64,
) -> PickingBudgetDecision {
    let estimated_bytes = estimate_picking_bytes(
        viewport,
        policy.picking.scale,
        active_rep_count,
        total_atoms,
    );
    let budget_bytes = effective_picking_budget_bytes(policy);
    let allowed = budget_bytes
        .is_none_or(|budget| fixed_reserved_bytes.saturating_add(estimated_bytes) <= budget);
    PickingBudgetDecision {
        allowed,
        fixed_reserved_bytes,
        estimated_bytes,
        budget_bytes,
        total_atoms,
        active_rep_count,
    }
}

fn estimate_picking_bytes(
    viewport: (u32, u32),
    picking_scale: f32,
    active_rep_count: usize,
    total_atoms: u64,
) -> u64 {
    let width = viewport.0.max(1);
    let height = viewport.1.max(1);
    let scale = picking_scale.clamp(0.125, 1.0);
    let pick_w = ((width as f32 * scale) as u32).max(1);
    let pick_h = ((height as f32 * scale) as u32).max(1);
    let target_bytes = estimate_texture_2d_bytes(pick_w, pick_h, PICKING_FORMAT)
        .saturating_add(estimate_texture_2d_bytes(pick_w, pick_h, DEPTH_FORMAT));
    let readback_bytes = PICKING_READBACK_BUFFER_BYTES.saturating_mul(2);
    let params_bytes = (active_rep_count as u64).saturating_mul(PickingParams::SIZE);
    let atom_pressure = total_atoms.saturating_mul(PICKING_ATOM_PRESSURE_BYTES);
    target_bytes
        .saturating_add(readback_bytes)
        .saturating_add(params_bytes)
        .saturating_add(atom_pressure)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::gib_to_bytes;
    use crate::memory::mib_to_bytes;

    #[test]
    fn small_scene_fits_lite_headroom() {
        let policy = RenderMemoryPolicy::lite();
        let fixed = mib_to_bytes(256);

        let decision = plan_picking_budget(policy, fixed, (1920, 1080), 8, 50_000);

        assert!(decision.allowed);
        assert_eq!(decision.budget_bytes, Some(gib_to_bytes(1)));
    }

    #[test]
    fn atom_pressure_can_deny_large_scene() {
        let policy = RenderMemoryPolicy::lite();
        let fixed = mib_to_bytes(256);

        let decision = plan_picking_budget(policy, fixed, (640, 480), 8, 2_000_000);

        assert!(!decision.allowed);
        assert!(decision.estimated_bytes > mib_to_bytes(900));
    }

    #[test]
    fn existing_picking_capacity_is_replaceable() {
        let mut snapshot = GpuMemorySnapshot::new();
        snapshot.add_allocation(GpuMemoryCategory::FrameTargets, mib_to_bytes(256));
        snapshot.add_allocation(GpuMemoryCategory::Picking, mib_to_bytes(128));
        snapshot.add_allocation(GpuMemoryCategory::Readback, mib_to_bytes(1));

        assert_eq!(fixed_reserved_without_picking(&snapshot), mib_to_bytes(256));
    }

    #[test]
    fn manual_profile_uses_explicit_budget() {
        let policy = RenderMemoryPolicy::manual(mib_to_bytes(512));
        let fits = plan_picking_budget(policy, mib_to_bytes(128), (1280, 720), 4, 100_000);
        let denied = plan_picking_budget(policy, mib_to_bytes(490), (1280, 720), 4, 100_000);

        assert_eq!(fits.budget_bytes, Some(mib_to_bytes(512)));
        assert!(fits.allowed);
        assert!(!denied.allowed);
    }
}
