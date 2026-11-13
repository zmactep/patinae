//! Group-2 bind-group layout for [`crate::scene_store::SceneStore`].
//!
//! Bindings (must mirror `shaders/common/scene.wgsl`):
//! - 0: `obj_table` — uniform with `dynamic_offset`. Each slot is one
//!   [`ObjectEntry`] padded to `ObjectEntry::STRIDE` (256 B).
//! - 1: `atoms`         — `storage<read>`  `array<AtomGpu>`
//! - 2: `coords`        — `storage<read>`  `array<vec4<f32>>`
//! - 3: `bonds`         — `storage<read>`  `array<BondGpu>`
//! - 4: `color_lut`     — `storage<read>`  `array<ColorGpu>`
//! - 5: `mask_lut`      — `storage<read>`  `array<u32>`
//! - 6: `marker_lut`    — `storage<read>`  `array<u32>`
//!
//! CSR adjacency remains in the store for export, but shaders do not read it.
//! Keep two storage slots free for compute output and indirect draw counts.

use std::num::NonZeroU64;

use super::ObjectEntry;

const STORAGE_BINDINGS: &[u32] = &[1, 2, 3, 4, 5, 6];
#[cfg(test)]
pub(crate) const FULL_STORAGE_BINDING_COUNT: usize = STORAGE_BINDINGS.len();
#[cfg(test)]
pub(crate) const OBJECT_COORDS_STORAGE_BINDING_COUNT: usize = 1;
#[cfg(test)]
const WEBGPU_MIN_STORAGE_BUFFERS_PER_STAGE: usize = 8;

/// Shared, immutable layout. Created once on `RenderContext`; every
/// pipeline that touches group 2 pulls its BGL from here.
pub struct SceneStoreLayout {
    pub bind_group_layout: wgpu::BindGroupLayout,
    pub object_coords_bind_group_layout: wgpu::BindGroupLayout,
}

impl SceneStoreLayout {
    pub fn new(device: &wgpu::Device) -> Self {
        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("patinae.scene_store.layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX
                            | wgpu::ShaderStages::FRAGMENT
                            | wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: true,
                            min_binding_size: NonZeroU64::new(
                                std::mem::size_of::<ObjectEntry>() as u64
                            ),
                        },
                        count: None,
                    },
                    storage_entry(STORAGE_BINDINGS[0]),
                    storage_entry(STORAGE_BINDINGS[1]),
                    storage_entry(STORAGE_BINDINGS[2]),
                    storage_entry(STORAGE_BINDINGS[3]),
                    storage_entry(STORAGE_BINDINGS[4]),
                    storage_entry(STORAGE_BINDINGS[5]),
                ],
            });
        let object_coords_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("patinae.scene_store.object_coords_layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: true,
                            min_binding_size: NonZeroU64::new(
                                std::mem::size_of::<ObjectEntry>() as u64
                            ),
                        },
                        count: None,
                    },
                    storage_compute_entry(2),
                ],
            });
        Self {
            bind_group_layout,
            object_coords_bind_group_layout,
        }
    }
}

fn storage_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX
            | wgpu::ShaderStages::FRAGMENT
            | wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn storage_compute_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a real GPU; run explicitly for renderer changes"]
    fn gpu_renderer_initializes_with_eight_storage_buffers() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .expect("GPU adapter required; this test must not silently skip");
        let mut limits = crate::required_device_limits(&adapter.limits()).unwrap();
        limits.max_storage_buffers_per_shader_stage = 8;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: limits,
            ..Default::default()
        }))
        .unwrap();
        assert_eq!(device.limits().max_storage_buffers_per_shader_stage, 8);
        let _renderer = crate::RenderState::new(
            std::sync::Arc::new(device),
            std::sync::Arc::new(queue),
            wgpu::TextureFormat::Rgba8Unorm,
            (64, 64),
        );
    }

    #[test]
    fn scene_store_layout_fits_webgpu_storage_buffer_limit() {
        const {
            assert!(
                FULL_STORAGE_BINDING_COUNT + 2 <= WEBGPU_MIN_STORAGE_BUFFERS_PER_STAGE,
                "scene plus compute output/count buffers must fit WebGPU's portable limit"
            );
        }
    }
}
