use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory2, IDXGIAdapter3, IDXGIFactory4, DXGI_CREATE_FACTORY_FLAGS,
    DXGI_MEMORY_SEGMENT_GROUP_LOCAL, DXGI_QUERY_VIDEO_MEMORY_INFO,
};

use patinae_render::{NativeGpuMemoryBudget, NativeMemoryBudgetSource};

pub(super) fn device_budget(device: &wgpu::Device) -> Option<NativeGpuMemoryBudget> {
    // SAFETY: Retain the device while reading its adapter identity.
    let hal = unsafe { device.as_hal::<wgpu::hal::api::Dx12>() }?;
    // SAFETY: These read-only queries obtain the exact adapter by its device LUID.
    let adapter: IDXGIAdapter3 = unsafe {
        let factory: IDXGIFactory4 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)).ok()?;
        factory
            .EnumAdapterByLuid(hal.raw_device().GetAdapterLuid())
            .ok()?
    };
    snapshot(&adapter)
}

fn snapshot(adapter: &IDXGIAdapter3) -> Option<NativeGpuMemoryBudget> {
    let mut info = DXGI_QUERY_VIDEO_MEMORY_INFO::default();
    // SAFETY: wgpu uses node zero; the output pointer remains valid for the call.
    unsafe { adapter.QueryVideoMemoryInfo(0, DXGI_MEMORY_SEGMENT_GROUP_LOCAL, &mut info) }.ok()?;
    (info.Budget > 0).then_some(NativeGpuMemoryBudget {
        budget_bytes: info.Budget,
        usage_bytes: info.CurrentUsage,
        source: NativeMemoryBudgetSource::DxgiLocal,
    })
}
