//! Native memory budgets for the GPU selected by wgpu.
//!
//! Budgets are advisory process working sets, not guaranteed free VRAM. In
//! particular, Metal's recommendation shares physical RAM with the CPU on UMA.
//! Querying a device uses its own native handle, never a newly selected GPU.

#[cfg(target_os = "windows")]
mod dx12;
#[cfg(target_vendor = "apple")]
mod metal;
#[cfg(any(target_os = "linux", target_os = "windows"))]
mod vulkan;

use patinae_render::NativeGpuMemoryBudget;

/// Reads the selected adapter's native budget, if supported by its backend.
#[cfg(not(target_os = "windows"))]
pub(crate) fn adapter_memory_budget(adapter: &wgpu::Adapter) -> Option<NativeGpuMemoryBudget> {
    #[cfg(target_vendor = "apple")]
    if let Some(budget) = metal::adapter_budget(adapter) {
        return Some(budget);
    }
    #[cfg(target_os = "linux")]
    if let Some(budget) = vulkan::adapter_budget(adapter) {
        return Some(budget);
    }
    let _ = adapter;
    None
}

/// Reads the active device's native budget, including host UI allocations.
pub(crate) fn device_memory_budget(device: &wgpu::Device) -> Option<NativeGpuMemoryBudget> {
    #[cfg(target_vendor = "apple")]
    if let Some(budget) = metal::device_budget(device) {
        return Some(budget);
    }
    #[cfg(target_os = "windows")]
    if let Some(budget) = dx12::device_budget(device) {
        return Some(budget);
    }
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    if let Some(budget) = vulkan::device_budget(device) {
        return Some(budget);
    }
    let _ = device;
    None
}
