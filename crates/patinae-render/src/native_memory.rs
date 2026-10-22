//! Native GPU memory snapshots supplied by the host.

/// Origin and semantics of a native memory budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeMemoryBudgetSource {
    /// Metal's recommended working set and device resource allocations.
    MetalWorkingSet,
    /// Budget and process usage of one device-local Vulkan heap.
    VulkanHeap,
    /// OS-assigned budget and process usage of the local DXGI segment.
    DxgiLocal,
}

/// Advisory budget and process usage sampled from a native GPU API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeGpuMemoryBudget {
    /// Total recommended budget, including existing process allocations.
    pub budget_bytes: u64,
    /// Current process usage reported by the same native API.
    pub usage_bytes: u64,
    /// API providing this snapshot; unsupported queries return `None`.
    pub source: NativeMemoryBudgetSource,
}

impl NativeGpuMemoryBudget {
    /// Remaining advisory budget, saturating when the process is over budget.
    pub fn headroom_bytes(self) -> u64 {
        self.budget_bytes.saturating_sub(self.usage_bytes)
    }
}
