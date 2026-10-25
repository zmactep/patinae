//! Representation memory preflight decisions.
//!
//! This module keeps budget planning separate from representation builders.
//! The planner works on conservative estimates and returns decisions that the
//! sync path can apply before large GPU resources are allocated.

use std::collections::HashSet;

use crate::memory::RenderMemoryPolicy;
use crate::picking::{ObjectId, RepKind};

/// Conservative GPU-memory estimate for one representation build option.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RepMemoryEstimate {
    /// Largest required single GPU buffer or binding, in bytes.
    pub required_bytes: u64,
    /// Temporary GPU memory needed while building, in bytes.
    pub scratch_bytes: u64,
    /// Persistent capacity allocated by this representation, in bytes.
    pub capacity_bytes: u64,
    /// Visual or geometric quality represented by this estimate.
    pub quality: RepQualityLevel,
    /// Whether this representation could be built in chunks.
    pub can_chunk: bool,
    /// Whether this representation can be skipped without failing sync.
    pub can_skip: bool,
}

impl RepMemoryEstimate {
    /// Returns persistent plus scratch bytes reserved by this option.
    pub const fn reserved_bytes(self) -> u64 {
        self.capacity_bytes.saturating_add(self.scratch_bytes)
    }
}

/// Representation quality selected by the budget planner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RepQualityLevel {
    /// Preserve the currently requested full-detail behavior.
    Full,
    /// Build only a deterministic source sample.
    Sampled {
        /// Right-shift used to derive the source sampling stride.
        sample_shift: u32,
    },
    /// Use an existing coarser representation quality.
    Coarsened,
}

/// Build action selected by the budget planner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RepBuildDecision {
    /// Build with the requested full quality.
    Build,
    /// Build at a lower quality described by the diagnostic estimate.
    Downgrade,
    /// Build in bounded chunks.
    Chunk {
        /// Maximum GPU bytes per chunk.
        max_chunk_bytes: u64,
    },
    /// Do not build this representation.
    Skip {
        /// Why the representation was skipped.
        reason: RepSkipReason,
    },
}

impl RepBuildDecision {
    /// Returns `true` when this decision skips representation allocation.
    pub const fn is_skip(self) -> bool {
        matches!(self, Self::Skip { .. })
    }
}

/// Reason a representation preflight denied allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RepSkipReason {
    /// The active memory budget could not reserve this representation.
    BudgetExceeded,
    /// A required buffer exceeds the device limits.
    DeviceLimitExceeded,
    /// The planner selected chunking, but the rep cannot apply it yet.
    UnsupportedChunking,
}

/// Last budget decision for one object representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RepBudgetDiagnostic {
    /// Object whose representation was planned.
    pub object_id: ObjectId,
    /// Representation kind that was planned.
    pub kind: RepKind,
    /// Decision selected for this representation.
    pub decision: RepBuildDecision,
    /// Estimate associated with the selected or smallest attempted option.
    pub estimate: RepMemoryEstimate,
}

impl RepBudgetDiagnostic {
    pub(crate) const fn warning_key(self) -> Option<RepBudgetWarningKey> {
        match self.decision {
            RepBuildDecision::Build => None,
            decision => Some(RepBudgetWarningKey {
                object_id: self.object_id.0,
                kind: self.kind,
                decision,
                quality: self.estimate.quality,
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct RepBudgetWarningKey {
    object_id: u32,
    kind: RepKind,
    decision: RepBuildDecision,
    quality: RepQualityLevel,
}

/// One representation request passed to the planner.
#[derive(Debug, Clone)]
pub(crate) struct RepBudgetRequest {
    pub(crate) object_id: ObjectId,
    pub(crate) kind: RepKind,
    pub(crate) estimates: Vec<RepMemoryEstimate>,
}

impl RepBudgetRequest {
    pub(crate) fn new(
        object_id: ObjectId,
        kind: RepKind,
        estimates: Vec<RepMemoryEstimate>,
    ) -> Self {
        Self {
            object_id,
            kind,
            estimates,
        }
    }
}

/// Inputs required to plan representation allocation.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RepBudgetInput<'a> {
    pub(crate) policy: RenderMemoryPolicy,
    pub(crate) fixed_reserved_bytes: u64,
    pub(crate) device_limits: &'a wgpu::Limits,
    pub(crate) requests: &'a [RepBudgetRequest],
    pub(crate) active_rep_count: usize,
}

/// Plans representation allocation decisions in deterministic order.
pub(crate) fn plan_rep_budget(input: RepBudgetInput<'_>) -> Vec<RepBudgetDiagnostic> {
    let mut decisions = vec![None; input.requests.len()];
    let mut order: Vec<usize> = (0..input.requests.len()).collect();
    order.sort_by_key(|&index| (rep_priority(input.requests[index].kind), index));

    let mut used_bytes = input.fixed_reserved_bytes;
    let budget_bytes = input.policy.budget_bytes;

    for index in order {
        let request = &input.requests[index];
        let diagnostic = plan_constrained_request(
            request,
            input.device_limits,
            budget_bytes.unwrap_or(u64::MAX),
            &mut used_bytes,
        );
        decisions[index] = Some(diagnostic);
    }

    let mut diagnostics = Vec::with_capacity(input.active_rep_count.max(input.requests.len()));
    diagnostics.extend(decisions.into_iter().flatten());
    diagnostics
}

pub(crate) fn current_warning_keys(
    diagnostics: &[RepBudgetDiagnostic],
) -> HashSet<RepBudgetWarningKey> {
    let mut keys = HashSet::with_capacity(diagnostics.len());
    keys.extend(
        diagnostics
            .iter()
            .filter_map(|diagnostic| diagnostic.warning_key()),
    );
    keys
}

fn plan_constrained_request(
    request: &RepBudgetRequest,
    device_limits: &wgpu::Limits,
    budget_bytes: u64,
    used_bytes: &mut u64,
) -> RepBudgetDiagnostic {
    for estimate in &request.estimates {
        if device_skip_reason(*estimate, device_limits).is_some() {
            continue;
        }
        let next_used = used_bytes.saturating_add(estimate.reserved_bytes());
        if next_used <= budget_bytes {
            *used_bytes = next_used;
            return RepBudgetDiagnostic {
                object_id: request.object_id,
                kind: request.kind,
                decision: decision_for_quality(estimate.quality),
                estimate: *estimate,
            };
        }
    }

    let fallback = smallest_estimate(&request.estimates).unwrap_or_else(empty_full_estimate);
    let reason =
        device_skip_reason(fallback, device_limits).unwrap_or(RepSkipReason::BudgetExceeded);
    RepBudgetDiagnostic {
        object_id: request.object_id,
        kind: request.kind,
        decision: RepBuildDecision::Skip { reason },
        estimate: fallback,
    }
}

fn decision_for_quality(quality: RepQualityLevel) -> RepBuildDecision {
    match quality {
        RepQualityLevel::Full => RepBuildDecision::Build,
        RepQualityLevel::Sampled { .. } | RepQualityLevel::Coarsened => RepBuildDecision::Downgrade,
    }
}

fn device_skip_reason(estimate: RepMemoryEstimate, limits: &wgpu::Limits) -> Option<RepSkipReason> {
    let storage_limit = limits.max_storage_buffer_binding_size;
    let single_buffer_limit = limits.max_buffer_size.min(storage_limit);
    (estimate.required_bytes > single_buffer_limit).then_some(RepSkipReason::DeviceLimitExceeded)
}

fn smallest_estimate(estimates: &[RepMemoryEstimate]) -> Option<RepMemoryEstimate> {
    estimates
        .iter()
        .copied()
        .min_by_key(|estimate| estimate.reserved_bytes())
}

fn empty_full_estimate() -> RepMemoryEstimate {
    RepMemoryEstimate {
        required_bytes: 0,
        scratch_bytes: 0,
        capacity_bytes: 0,
        quality: RepQualityLevel::Full,
        can_chunk: false,
        can_skip: true,
    }
}

fn rep_priority(kind: RepKind) -> u8 {
    match kind {
        RepKind::Line => 0,
        RepKind::Cartoon | RepKind::Ribbon => 1,
        RepKind::Dot => 2,
        RepKind::Surface | RepKind::Mesh => 3,
        RepKind::Sphere | RepKind::Stick => 4,
        RepKind::Ellipsoid => 5,
        RepKind::None => 6,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{gib_to_bytes, mib_to_bytes};

    fn estimate(mib: u64, quality: RepQualityLevel) -> RepMemoryEstimate {
        RepMemoryEstimate {
            required_bytes: mib_to_bytes(mib),
            scratch_bytes: 0,
            capacity_bytes: mib_to_bytes(mib),
            quality,
            can_chunk: false,
            can_skip: true,
        }
    }

    fn limits() -> wgpu::Limits {
        wgpu::Limits {
            max_buffer_size: gib_to_bytes(4),
            max_storage_buffer_binding_size: gib_to_bytes(2),
            ..wgpu::Limits::default()
        }
    }

    fn request(kind: RepKind, estimates: Vec<RepMemoryEstimate>) -> RepBudgetRequest {
        RepBudgetRequest::new(ObjectId(kind as u32), kind, estimates)
    }

    #[test]
    fn performance_profile_checks_device_limits_without_a_total_budget() {
        let requests = [
            request(RepKind::Sphere, vec![estimate(4096, RepQualityLevel::Full)]),
            request(RepKind::Line, vec![estimate(1024, RepQualityLevel::Full)]),
        ];

        let diagnostics = plan_rep_budget(RepBudgetInput {
            policy: RenderMemoryPolicy::performance(),
            fixed_reserved_bytes: gib_to_bytes(20),
            device_limits: &limits(),
            requests: &requests,
            active_rep_count: requests.len(),
        });

        assert_eq!(
            diagnostics[0].decision,
            RepBuildDecision::Skip {
                reason: RepSkipReason::DeviceLimitExceeded
            }
        );
        assert_eq!(diagnostics[1].decision, RepBuildDecision::Build);
    }

    #[test]
    fn performance_downgrades_only_when_resource_limits_require_it() {
        let requests = [request(
            RepKind::Sphere,
            vec![
                estimate(4096, RepQualityLevel::Full),
                estimate(512, RepQualityLevel::Sampled { sample_shift: 1 }),
            ],
        )];
        let diagnostics = plan_rep_budget(RepBudgetInput {
            policy: RenderMemoryPolicy::performance(),
            fixed_reserved_bytes: 0,
            device_limits: &limits(),
            requests: &requests,
            active_rep_count: 1,
        });
        assert_eq!(diagnostics[0].decision, RepBuildDecision::Downgrade);
    }

    #[test]
    fn constrained_profile_reserves_lower_priority_reps_after_fallbacks() {
        let requests = [
            request(RepKind::Sphere, vec![estimate(900, RepQualityLevel::Full)]),
            request(RepKind::Line, vec![estimate(128, RepQualityLevel::Full)]),
            request(RepKind::Cartoon, vec![estimate(256, RepQualityLevel::Full)]),
        ];

        let diagnostics = plan_rep_budget(RepBudgetInput {
            policy: RenderMemoryPolicy::lite(),
            fixed_reserved_bytes: mib_to_bytes(128),
            device_limits: &limits(),
            requests: &requests,
            active_rep_count: requests.len(),
        });

        assert_eq!(
            diagnostics[0].decision,
            RepBuildDecision::Skip {
                reason: RepSkipReason::BudgetExceeded,
            }
        );
        assert_eq!(diagnostics[1].decision, RepBuildDecision::Build);
        assert_eq!(diagnostics[2].decision, RepBuildDecision::Build);
    }

    #[test]
    fn constrained_profile_selects_sampled_candidate_when_full_does_not_fit() {
        let requests = [request(
            RepKind::Sphere,
            vec![
                estimate(1200, RepQualityLevel::Full),
                estimate(256, RepQualityLevel::Sampled { sample_shift: 3 }),
            ],
        )];

        let diagnostics = plan_rep_budget(RepBudgetInput {
            policy: RenderMemoryPolicy::lite(),
            fixed_reserved_bytes: mib_to_bytes(512),
            device_limits: &limits(),
            requests: &requests,
            active_rep_count: requests.len(),
        });

        assert_eq!(diagnostics[0].decision, RepBuildDecision::Downgrade);
        assert_eq!(
            diagnostics[0].estimate.quality,
            RepQualityLevel::Sampled { sample_shift: 3 }
        );
    }

    #[test]
    fn planner_reports_device_limit_before_budget_pressure() {
        let mut limits = limits();
        limits.max_storage_buffer_binding_size = mib_to_bytes(64);
        let requests = [request(
            RepKind::Dot,
            vec![estimate(128, RepQualityLevel::Full)],
        )];

        let diagnostics = plan_rep_budget(RepBudgetInput {
            policy: RenderMemoryPolicy::lite(),
            fixed_reserved_bytes: 0,
            device_limits: &limits,
            requests: &requests,
            active_rep_count: requests.len(),
        });

        assert_eq!(
            diagnostics[0].decision,
            RepBuildDecision::Skip {
                reason: RepSkipReason::DeviceLimitExceeded
            }
        );
    }

    #[test]
    fn warning_keys_dedupe_equivalent_non_build_decisions() {
        let diagnostic = RepBudgetDiagnostic {
            object_id: ObjectId(7),
            kind: RepKind::Sphere,
            decision: RepBuildDecision::Skip {
                reason: RepSkipReason::BudgetExceeded,
            },
            estimate: estimate(128, RepQualityLevel::Sampled { sample_shift: 2 }),
        };
        let build = RepBudgetDiagnostic {
            object_id: ObjectId(8),
            kind: RepKind::Line,
            decision: RepBuildDecision::Build,
            estimate: estimate(1, RepQualityLevel::Full),
        };

        let keys = current_warning_keys(&[diagnostic, diagnostic, build]);

        assert_eq!(keys.len(), 1);
        assert!(keys.contains(&diagnostic.warning_key().expect("warning key")));
    }
}
