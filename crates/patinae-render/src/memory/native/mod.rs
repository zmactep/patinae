//! Advisory process budgets, not physical VRAM capacity or allocation guarantees.
//!
//! Optional native queries use the selected adapter/device's own handle. Sample
//! at startup, never per frame. Platform code reports facts; profile selection
//! owns reserve margins and thresholds. UMA budgets may include shared CPU RAM.

#[cfg(all(feature = "native-memory", target_os = "windows"))]
mod dx12;
#[cfg(all(feature = "native-memory", target_vendor = "apple"))]
mod metal;
#[cfg(all(
    feature = "native-memory",
    any(target_os = "linux", target_os = "windows")
))]
mod vulkan;

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
    /// Normalizes an unavailable zero budget to `None`.
    pub fn new(
        budget_bytes: u64,
        usage_bytes: u64,
        source: NativeMemoryBudgetSource,
    ) -> Option<Self> {
        (budget_bytes > 0).then_some(Self {
            budget_bytes,
            usage_bytes,
            source,
        })
    }
    /// Remaining advisory budget, saturating when the process is over budget.
    pub fn headroom_bytes(self) -> u64 {
        self.budget_bytes.saturating_sub(self.usage_bytes)
    }
}

/// Samples the selected adapter before device creation, when supported.
///
/// This provisional snapshot can guide allocation hints and process startup.
/// Use the created device's snapshot for the renderer's final auto profile.
#[cfg(feature = "native-memory")]
pub fn adapter_memory_budget(adapter: &wgpu::Adapter) -> Option<NativeGpuMemoryBudget> {
    match adapter.get_info().backend {
        #[cfg(target_vendor = "apple")]
        wgpu::Backend::Metal => metal::adapter_budget(adapter),
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        wgpu::Backend::Vulkan => vulkan::adapter_budget(adapter),
        _ => None,
    }
}

/// Samples the created device, including allocations made by the host UI.
#[cfg(feature = "native-memory")]
pub fn device_memory_budget(device: &wgpu::Device) -> Option<NativeGpuMemoryBudget> {
    match device.adapter_info().backend {
        #[cfg(target_vendor = "apple")]
        wgpu::Backend::Metal => metal::device_budget(device),
        #[cfg(target_os = "windows")]
        wgpu::Backend::Dx12 => dx12::device_budget(device),
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        wgpu::Backend::Vulkan => vulkan::device_budget(device),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "native-memory")]
    #[test]
    #[ignore = "requires a real native GPU; queries its actual API budget"]
    fn gpu_native_budget_matches_created_device_backend() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
        let info = adapter.get_info();
        let provisional = adapter_memory_budget(&adapter);
        let (device, _) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: crate::required_device_limits(&adapter.limits()).unwrap(),
            ..Default::default()
        }))
        .unwrap();
        let snapshot = device_memory_budget(&device);
        eprintln!(
            "backend={:?} provisional={provisional:?} created_device={snapshot:?}",
            info.backend
        );
        let expected = match info.backend {
            wgpu::Backend::Metal => NativeMemoryBudgetSource::MetalWorkingSet,
            wgpu::Backend::Dx12 => NativeMemoryBudgetSource::DxgiLocal,
            wgpu::Backend::Vulkan => {
                // VK_EXT_memory_budget is optional; its absence is a valid fallback.
                if snapshot.is_none() {
                    return;
                }
                NativeMemoryBudgetSource::VulkanHeap
            }
            other => panic!("native memory test requires a native backend, got {other:?}"),
        };
        let snapshot = snapshot.expect("native budget must be readable on this backend");
        assert_eq!(snapshot.source, expected);
        assert!(snapshot.budget_bytes > 0);
        assert_eq!(device.adapter_info().name, info.name);
    }

    #[test]
    fn unknown_budget_and_over_budget_usage_are_distinct() {
        for source in [
            NativeMemoryBudgetSource::MetalWorkingSet,
            NativeMemoryBudgetSource::VulkanHeap,
            NativeMemoryBudgetSource::DxgiLocal,
        ] {
            assert!(NativeGpuMemoryBudget::new(0, 10, source).is_none());
            let budget = NativeGpuMemoryBudget::new(100, 110, source).unwrap();
            assert_eq!(budget.headroom_bytes(), 0);
            assert_eq!(budget.usage_bytes, 110);
        }
    }
}
