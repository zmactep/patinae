use super::{NativeGpuMemoryBudget, NativeMemoryBudgetSource};
use objc2_metal::MTLDevice;

pub(super) fn adapter_budget(adapter: &wgpu::Adapter) -> Option<NativeGpuMemoryBudget> {
    // SAFETY: Only read properties while the HAL guard keeps this adapter alive.
    let hal = unsafe { adapter.as_hal::<wgpu::hal::api::Metal>() }?;
    let device = hal.raw_device();
    NativeGpuMemoryBudget::new(
        device.recommendedMaxWorkingSetSize(),
        device.currentAllocatedSize() as u64,
        NativeMemoryBudgetSource::MetalWorkingSet,
    )
}

pub(super) fn device_budget(device: &wgpu::Device) -> Option<NativeGpuMemoryBudget> {
    // SAFETY: Only read properties; do not release or mutate the native device.
    let hal = unsafe { device.as_hal::<wgpu::hal::api::Metal>() }?;
    let device = hal.raw_device();
    NativeGpuMemoryBudget::new(
        device.recommendedMaxWorkingSetSize(),
        device.currentAllocatedSize() as u64,
        NativeMemoryBudgetSource::MetalWorkingSet,
    )
}
