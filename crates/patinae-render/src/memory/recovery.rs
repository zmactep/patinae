//! Renderer memory-pressure recovery state.

use crate::memory::RenderMemoryProfile;

/// Recovery state for a confirmed GPU out-of-memory sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderMemoryRecoveryStage {
    /// Rendering uses the baseline profile selected before this recovery sequence.
    Normal { base_profile: RenderMemoryProfile },
    /// The next frame retries the baseline after forced defragmentation.
    RetryAfterDefrag { base_profile: RenderMemoryProfile },
    /// Rendering uses a lower temporary profile for this recovery sequence.
    Fallback {
        base_profile: RenderMemoryProfile,
        effective_profile: RenderMemoryProfile,
    },
    /// Rendering is blocked for the current scene/profile sequence.
    Blocked {
        base_profile: RenderMemoryProfile,
        last_profile: RenderMemoryProfile,
    },
}

impl RenderMemoryRecoveryStage {
    /// Creates a normal recovery stage for a baseline profile.
    pub const fn normal(base_profile: RenderMemoryProfile) -> Self {
        Self::Normal { base_profile }
    }

    /// Returns the baseline profile captured at sequence start.
    pub const fn base_profile(self) -> RenderMemoryProfile {
        match self {
            Self::Normal { base_profile }
            | Self::RetryAfterDefrag { base_profile }
            | Self::Fallback { base_profile, .. }
            | Self::Blocked { base_profile, .. } => base_profile,
        }
    }

    /// Returns the profile currently used for rendering.
    pub const fn effective_profile(self) -> RenderMemoryProfile {
        match self {
            Self::Normal { base_profile } | Self::RetryAfterDefrag { base_profile } => base_profile,
            Self::Fallback {
                effective_profile, ..
            } => effective_profile,
            Self::Blocked { last_profile, .. } => last_profile,
        }
    }

    /// Returns `true` when no OOM recovery sequence is active.
    pub const fn is_normal(self) -> bool {
        matches!(self, Self::Normal { .. })
    }

    /// Returns whether this scene must wait for a scene or requested-profile change.
    pub const fn is_blocked(self) -> bool {
        matches!(self, Self::Blocked { .. })
    }

    /// Accepts a changed user request without undoing an automatic OOM fallback.
    ///
    /// Compare against the baseline, never the effective fallback profile. Hosts
    /// may call this every frame; an unchanged request preserves recovery state.
    pub fn request_profile(&mut self, requested: RenderMemoryProfile) -> bool {
        if requested == self.base_profile() {
            return false;
        }
        self.reset(requested);
        true
    }

    /// Advances the state after a confirmed OOM.
    pub fn advance_after_oom(&mut self) -> RenderMemoryRecoveryAction {
        match *self {
            Self::Normal { base_profile } => {
                *self = Self::RetryAfterDefrag { base_profile };
                RenderMemoryRecoveryAction::RetryAfterDefrag {
                    profile: base_profile,
                }
            }
            Self::RetryAfterDefrag { base_profile } => {
                self.fallback_or_block(base_profile, base_profile)
            }
            Self::Fallback {
                base_profile,
                effective_profile,
            } => self.fallback_or_block(base_profile, effective_profile),
            Self::Blocked { last_profile, .. } => {
                RenderMemoryRecoveryAction::Blocked { last_profile }
            }
        }
    }

    /// Records a successful frame.
    pub fn record_success(&mut self) {
        if let Self::RetryAfterDefrag { base_profile } = *self {
            *self = Self::Normal { base_profile };
        }
    }

    /// Resets recovery to a new baseline.
    pub fn reset(&mut self, base_profile: RenderMemoryProfile) {
        *self = Self::Normal { base_profile };
    }

    fn fallback_or_block(
        &mut self,
        base_profile: RenderMemoryProfile,
        last_profile: RenderMemoryProfile,
    ) -> RenderMemoryRecoveryAction {
        match next_oom_fallback(last_profile) {
            Some(effective_profile) => {
                *self = Self::Fallback {
                    base_profile,
                    effective_profile,
                };
                RenderMemoryRecoveryAction::SwitchProfile { effective_profile }
            }
            None => {
                *self = Self::Blocked {
                    base_profile,
                    last_profile,
                };
                RenderMemoryRecoveryAction::Blocked { last_profile }
            }
        }
    }
}

/// Recovery action requested by [`RenderMemoryRecoveryStage`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderMemoryRecoveryAction {
    /// Retry the same profile after forced defragmentation.
    RetryAfterDefrag { profile: RenderMemoryProfile },
    /// Rebuild the renderer with a lower effective profile.
    SwitchProfile {
        effective_profile: RenderMemoryProfile,
    },
    /// Stop retrying for this scene/profile sequence.
    Blocked { last_profile: RenderMemoryProfile },
}

/// Returns the next lower OOM fallback profile.
pub const fn next_oom_fallback(profile: RenderMemoryProfile) -> Option<RenderMemoryProfile> {
    match profile {
        RenderMemoryProfile::Performance => Some(RenderMemoryProfile::Balanced),
        RenderMemoryProfile::Balanced => Some(RenderMemoryProfile::Lite),
        RenderMemoryProfile::Lite | RenderMemoryProfile::Manual { .. } => None,
    }
}

/// Returns `true` when a WGPU error is a confirmed out-of-memory signal.
pub const fn is_wgpu_oom(error: &wgpu::Error) -> bool {
    matches!(error, wgpu::Error::OutOfMemory { .. })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unchanged_user_request_preserves_fallback_until_a_real_change() {
        let requested = RenderMemoryProfile::Performance;
        let mut state = RenderMemoryRecoveryStage::normal(requested);
        for expected in [
            requested,
            RenderMemoryProfile::Balanced,
            RenderMemoryProfile::Lite,
            RenderMemoryProfile::Lite,
        ] {
            state.advance_after_oom();
            for _ in 0..100 {
                assert!(!state.request_profile(requested));
                assert_eq!(state.effective_profile(), expected);
            }
        }
        assert!(state.is_blocked());
        assert!(state.request_profile(RenderMemoryProfile::Balanced));
        assert!(state.is_normal());
        assert_eq!(state.effective_profile(), RenderMemoryProfile::Balanced);
        assert!(state.request_profile(RenderMemoryProfile::Manual { bytes: 1024 }));
        assert!(state.request_profile(RenderMemoryProfile::Manual { bytes: 2048 }));
        assert!(!state.request_profile(RenderMemoryProfile::Manual { bytes: 2048 }));
    }
    fn test_error_source(message: &'static str) -> wgpu::ErrorSource {
        Box::new(std::io::Error::other(message))
    }

    #[test]
    fn recovery_performance_baseline_retries_then_balanced_then_lite_then_blocks() {
        let mut stage = RenderMemoryRecoveryStage::normal(RenderMemoryProfile::Performance);

        assert_eq!(
            stage.advance_after_oom(),
            RenderMemoryRecoveryAction::RetryAfterDefrag {
                profile: RenderMemoryProfile::Performance
            }
        );
        assert_eq!(
            stage.advance_after_oom(),
            RenderMemoryRecoveryAction::SwitchProfile {
                effective_profile: RenderMemoryProfile::Balanced
            }
        );
        assert_eq!(
            stage.advance_after_oom(),
            RenderMemoryRecoveryAction::SwitchProfile {
                effective_profile: RenderMemoryProfile::Lite
            }
        );
        assert_eq!(
            stage.advance_after_oom(),
            RenderMemoryRecoveryAction::Blocked {
                last_profile: RenderMemoryProfile::Lite
            }
        );
    }

    #[test]
    fn recovery_balanced_baseline_skips_performance() {
        let mut stage = RenderMemoryRecoveryStage::normal(RenderMemoryProfile::Balanced);

        assert_eq!(
            stage.advance_after_oom(),
            RenderMemoryRecoveryAction::RetryAfterDefrag {
                profile: RenderMemoryProfile::Balanced
            }
        );
        assert_eq!(
            stage.advance_after_oom(),
            RenderMemoryRecoveryAction::SwitchProfile {
                effective_profile: RenderMemoryProfile::Lite
            }
        );
        assert_eq!(
            stage.advance_after_oom(),
            RenderMemoryRecoveryAction::Blocked {
                last_profile: RenderMemoryProfile::Lite
            }
        );
    }

    #[test]
    fn recovery_lite_baseline_only_retries_then_blocks() {
        let mut stage = RenderMemoryRecoveryStage::normal(RenderMemoryProfile::Lite);

        assert_eq!(
            stage.advance_after_oom(),
            RenderMemoryRecoveryAction::RetryAfterDefrag {
                profile: RenderMemoryProfile::Lite
            }
        );
        assert_eq!(
            stage.advance_after_oom(),
            RenderMemoryRecoveryAction::Blocked {
                last_profile: RenderMemoryProfile::Lite
            }
        );
    }

    #[test]
    fn recovery_manual_baseline_does_not_invent_lower_budget() {
        let profile = RenderMemoryProfile::Manual { bytes: 512 };
        let mut stage = RenderMemoryRecoveryStage::normal(profile);

        assert_eq!(
            stage.advance_after_oom(),
            RenderMemoryRecoveryAction::RetryAfterDefrag { profile }
        );
        assert_eq!(
            stage.advance_after_oom(),
            RenderMemoryRecoveryAction::Blocked {
                last_profile: profile
            }
        );
    }

    #[test]
    fn recovery_success_after_defrag_preserves_baseline() {
        let mut stage = RenderMemoryRecoveryStage::normal(RenderMemoryProfile::Balanced);

        assert_eq!(
            stage.advance_after_oom(),
            RenderMemoryRecoveryAction::RetryAfterDefrag {
                profile: RenderMemoryProfile::Balanced
            }
        );
        stage.record_success();

        assert_eq!(
            stage,
            RenderMemoryRecoveryStage::Normal {
                base_profile: RenderMemoryProfile::Balanced
            }
        );
    }

    #[test]
    fn recovery_success_after_fallback_preserves_baseline_and_effective_profile() {
        let mut stage = RenderMemoryRecoveryStage::normal(RenderMemoryProfile::Performance);

        assert_eq!(
            stage.advance_after_oom(),
            RenderMemoryRecoveryAction::RetryAfterDefrag {
                profile: RenderMemoryProfile::Performance
            }
        );
        assert_eq!(
            stage.advance_after_oom(),
            RenderMemoryRecoveryAction::SwitchProfile {
                effective_profile: RenderMemoryProfile::Balanced
            }
        );
        stage.record_success();

        assert_eq!(
            stage,
            RenderMemoryRecoveryStage::Fallback {
                base_profile: RenderMemoryProfile::Performance,
                effective_profile: RenderMemoryProfile::Balanced,
            }
        );
    }

    #[test]
    fn wgpu_classifier_only_accepts_out_of_memory() {
        let oom = wgpu::Error::OutOfMemory {
            source: test_error_source("oom"),
        };
        let validation = wgpu::Error::Validation {
            source: test_error_source("validation"),
            description: "validation".to_string(),
        };
        let internal = wgpu::Error::Internal {
            source: test_error_source("internal"),
            description: "internal".to_string(),
        };

        assert!(is_wgpu_oom(&oom));
        assert!(!is_wgpu_oom(&validation));
        assert!(!is_wgpu_oom(&internal));
    }
}
