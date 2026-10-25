//! Checked GPU buffer sizing, independent of memory profiles.

use crate::memory::SceneStoreGrowthPolicy;

/// A buffer cannot fit within the active device's resource limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BufferSizeError {
    /// Resource that could not be allocated.
    pub label: &'static str,
    /// Required size, or `None` if arithmetic overflowed.
    pub required_bytes: Option<u64>,
    /// Maximum permitted allocation size.
    pub limit_bytes: u64,
}

impl std::fmt::Display for BufferSizeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.required_bytes {
            Some(bytes) => write!(
                f,
                "GPU buffer {} requires {} bytes; device limit is {} bytes",
                self.label, bytes, self.limit_bytes
            ),
            None => write!(
                f,
                "GPU buffer {} size calculation overflowed; device limit is {} bytes",
                self.label, self.limit_bytes
            ),
        }
    }
}

impl std::error::Error for BufferSizeError {}

/// Computes capacity without allocating, respecting element and copy alignment.
pub(crate) fn checked_buffer_capacity(
    label: &'static str,
    needed: usize,
    current: usize,
    item_size: usize,
    growth: SceneStoreGrowthPolicy,
    limit_bytes: u64,
) -> Result<usize, BufferSizeError> {
    let required_bytes = needed
        .max(1)
        .checked_mul(item_size)
        .map(|bytes| bytes as u64);
    let error = BufferSizeError {
        label,
        required_bytes,
        limit_bytes,
    };
    if item_size == 0 || required_bytes.is_none_or(|bytes| bytes > limit_bytes) {
        return Err(error);
    }
    // Round allocation sizes in whole elements to four-byte copy alignment.
    // Callers must also align individual queue writes.
    let quantum = match item_size % 4 {
        0 => 1,
        2 => 2,
        _ => 4,
    };
    let max_entries = usize::try_from(limit_bytes / item_size as u64)
        .unwrap_or(usize::MAX)
        .min(usize::MAX / item_size);
    let max_entries = max_entries - max_entries % quantum;
    let aligned_needed = needed
        .max(1)
        .checked_next_multiple_of(quantum)
        .ok_or(error)?;
    if aligned_needed > max_entries {
        return Err(BufferSizeError {
            required_bytes: aligned_needed
                .checked_mul(item_size)
                .map(|bytes| bytes as u64),
            ..error
        });
    }
    let desired = growable_capacity_entries(aligned_needed, current, item_size, growth);
    let desired = desired
        .checked_next_multiple_of(quantum)
        .unwrap_or(max_entries);
    Ok(desired.min(max_entries))
}

fn growable_capacity_entries(
    needed: usize,
    current: usize,
    item_size: usize,
    policy: SceneStoreGrowthPolicy,
) -> usize {
    let needed = needed.max(1);
    let min_capacity = 64;
    match policy {
        SceneStoreGrowthPolicy::PowerOfTwo => needed
            .checked_next_power_of_two()
            .unwrap_or(usize::MAX)
            .max(min_capacity),
        SceneStoreGrowthPolicy::Moderate { max_slack_bytes } => {
            let grown = current.saturating_add(current.div_ceil(2));
            clamp_growth_capacity(needed, grown, item_size, max_slack_bytes).max(min_capacity)
        }
        SceneStoreGrowthPolicy::Tight { max_slack_bytes } => {
            let grown = needed.saturating_add(needed.div_ceil(8));
            clamp_growth_capacity(needed, grown, item_size, max_slack_bytes).max(min_capacity)
        }
    }
}

fn clamp_growth_capacity(
    needed: usize,
    grown: usize,
    item_size: usize,
    max_slack_bytes: u64,
) -> usize {
    let item_size = item_size.max(1) as u64;
    let slack_entries = (max_slack_bytes / item_size)
        .try_into()
        .unwrap_or(usize::MAX);
    let cap = needed.saturating_add(slack_entries);
    grown.max(needed).min(cap)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dx12_growth_clamps_rounding_without_rejecting_live_data() {
        let limit = i32::MAX as u64;
        let max_entries = (limit / 16) as usize;
        for growth in [
            SceneStoreGrowthPolicy::PowerOfTwo,
            SceneStoreGrowthPolicy::Moderate {
                max_slack_bytes: 1 << 30,
            },
            SceneStoreGrowthPolicy::Tight {
                max_slack_bytes: 1 << 30,
            },
        ] {
            assert_eq!(
                checked_buffer_capacity("atoms", max_entries, 64, 16, growth, limit).unwrap(),
                max_entries
            );
            assert!(
                checked_buffer_capacity("atoms", max_entries + 1, 64, 16, growth, limit).is_err()
            );
        }
    }

    #[test]
    fn capacity_checks_overflow_alignment_and_small_limits_without_allocating() {
        let growth = SceneStoreGrowthPolicy::PowerOfTwo;
        assert_eq!(
            checked_buffer_capacity("overflow", usize::MAX, 0, 16, growth, u64::MAX)
                .unwrap_err()
                .required_bytes,
            None
        );
        assert!(checked_buffer_capacity("zero", 1, 0, 0, growth, 100).is_err());
        assert_eq!(
            checked_buffer_capacity("small", 2, 0, 4, growth, 11).unwrap(),
            2
        );
        assert!(checked_buffer_capacity("alignment", 1, 0, 3, growth, 11).is_err());
        assert_eq!(
            checked_buffer_capacity("alignment", 1, 0, 3, growth, 12).unwrap(),
            4
        );
        assert_eq!(
            checked_buffer_capacity("empty", 0, 0, 16, growth, 1024).unwrap(),
            64
        );
        assert!(checked_buffer_capacity("empty", 0, 0, 16, growth, 0).is_err());
    }

    #[test]
    fn power_of_two_growth_preserves_previous_capacity_strategy() {
        let capacity = growable_capacity_entries(65, 64, 4, SceneStoreGrowthPolicy::PowerOfTwo);

        assert_eq!(capacity, 128);
    }

    #[test]
    fn moderate_growth_uses_one_and_half_x_with_slack_cap() {
        let capacity = growable_capacity_entries(
            1_100,
            1_024,
            4,
            SceneStoreGrowthPolicy::Moderate {
                max_slack_bytes: 128,
            },
        );

        assert_eq!(capacity, 1_132);
    }

    #[test]
    fn tight_growth_uses_requested_size_plus_small_slack() {
        let capacity = growable_capacity_entries(
            1_000,
            512,
            4,
            SceneStoreGrowthPolicy::Tight {
                max_slack_bytes: 10_000,
            },
        );

        assert_eq!(capacity, 1_125);
    }
}
