use ash::vk;

use patinae_render::{NativeGpuMemoryBudget, NativeMemoryBudgetSource};

#[cfg(target_os = "linux")]
pub(super) fn adapter_budget(adapter: &wgpu::Adapter) -> Option<NativeGpuMemoryBudget> {
    // SAFETY: Query only the physical device belonging to this live HAL adapter.
    let hal = unsafe { adapter.as_hal::<wgpu::hal::api::Vulkan>() }?;
    snapshot(hal.shared_instance(), hal.raw_physical_device())
}

pub(super) fn device_budget(device: &wgpu::Device) -> Option<NativeGpuMemoryBudget> {
    // SAFETY: The guard keeps the instance and physical device alive for queries.
    let hal = unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }?;
    snapshot(hal.shared_instance(), hal.raw_physical_device())
}

fn snapshot(
    shared: &wgpu::hal::vulkan::InstanceShared,
    physical: vk::PhysicalDevice,
) -> Option<NativeGpuMemoryBudget> {
    if shared.instance_api_version() < vk::API_VERSION_1_1 {
        return None;
    }
    let instance = shared.raw_instance();
    // SAFETY: Physical device belongs to this live instance; no resources mutate.
    let extensions = unsafe { instance.enumerate_device_extension_properties(physical) }.ok()?;
    if !extensions.iter().any(|extension| {
        extension.extension_name_as_c_str().ok() == Some(ash::ext::memory_budget::NAME)
    }) {
        return None;
    }
    let mut budget = vk::PhysicalDeviceMemoryBudgetPropertiesEXT::default();
    let mut properties = vk::PhysicalDeviceMemoryProperties2::default().push_next(&mut budget);
    // SAFETY: The supported extension's output structure is valid for this call.
    unsafe { instance.get_physical_device_memory_properties2(physical, &mut properties) };
    let memory = properties.memory_properties;
    // Do not sum heaps: allocations must fit a suitable heap, and shared/device
    // heaps are not interchangeable. Use the largest available device-local heap.
    (0..memory.memory_heap_count as usize)
        .filter(|&index| {
            memory.memory_heaps[index]
                .flags
                .contains(vk::MemoryHeapFlags::DEVICE_LOCAL)
        })
        .filter(|&index| budget.heap_budget[index] > 0)
        .map(|index| NativeGpuMemoryBudget {
            budget_bytes: budget.heap_budget[index].min(memory.memory_heaps[index].size),
            usage_bytes: budget.heap_usage[index],
            source: NativeMemoryBudgetSource::VulkanHeap,
        })
        .max_by_key(|budget| budget.headroom_bytes())
}
