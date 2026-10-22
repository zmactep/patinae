use objc2_metal::MTLDevice;
use patinae_render::{NativeGpuMemoryBudget, NativeMemoryBudgetSource};

pub(super) fn adapter_budget(adapter: &wgpu::Adapter) -> Option<NativeGpuMemoryBudget> {
    // SAFETY: Only read properties while the HAL guard keeps this adapter alive.
    let hal = unsafe { adapter.as_hal::<wgpu::hal::api::Metal>() }?;
    let device = hal.raw_device();
    snapshot(
        device.recommendedMaxWorkingSetSize(),
        device.currentAllocatedSize() as u64,
    )
}

pub(super) fn device_budget(device: &wgpu::Device) -> Option<NativeGpuMemoryBudget> {
    // SAFETY: Only read properties; do not release or mutate the native device.
    let hal = unsafe { device.as_hal::<wgpu::hal::api::Metal>() }?;
    let device = hal.raw_device();
    snapshot(
        device.recommendedMaxWorkingSetSize(),
        device.currentAllocatedSize() as u64,
    )
}

fn snapshot(budget_bytes: u64, usage_bytes: u64) -> Option<NativeGpuMemoryBudget> {
    (budget_bytes > 0).then_some(NativeGpuMemoryBudget {
        budget_bytes,
        usage_bytes,
        source: NativeMemoryBudgetSource::MetalWorkingSet,
    })
}
