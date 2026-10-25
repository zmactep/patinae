//! Device creation requirements, independent of quality and memory profiles.

use crate::memory::gib_to_bytes;

/// Desired single-buffer ceiling; actual allocations use the created device's limits.
pub const DEVICE_BUFFER_TARGET_BYTES: u64 = gib_to_bytes(4);
/// Desired storage binding ceiling, compatible with WGPU's DX12 backend.
pub const DEVICE_STORAGE_BINDING_TARGET_BYTES: u64 = gib_to_bytes(2) - 1;
/// Storage bindings required by the current compute pipeline layouts.
pub const RENDER_COMPUTE_STORAGE_BUFFERS_PER_STAGE: u32 = 10;

/// An adapter cannot support the renderer's mandatory compute layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error(
    "GPU supports only {available} storage buffers per shader stage; Patinae requires {required}"
)]
pub struct DeviceRequirementsError {
    /// Adapter capability.
    pub available: u32,
    /// Mandatory renderer requirement.
    pub required: u32,
}

/// Negotiates resource ceilings while preserving mandatory compute requirements.
///
/// # Errors
/// Returns an error when the adapter cannot support the compute layout.
pub fn required_device_limits(
    adapter: &wgpu::Limits,
) -> Result<wgpu::Limits, DeviceRequirementsError> {
    if adapter.max_storage_buffers_per_shader_stage < RENDER_COMPUTE_STORAGE_BUFFERS_PER_STAGE {
        return Err(DeviceRequirementsError {
            available: adapter.max_storage_buffers_per_shader_stage,
            required: RENDER_COMPUTE_STORAGE_BUFFERS_PER_STAGE,
        });
    }
    Ok(wgpu::Limits {
        max_buffer_size: DEVICE_BUFFER_TARGET_BYTES.min(adapter.max_buffer_size),
        max_storage_buffer_binding_size: DEVICE_STORAGE_BINDING_TARGET_BYTES
            .min(adapter.max_storage_buffer_binding_size)
            .min(adapter.max_buffer_size),
        max_storage_buffers_per_shader_stage: RENDER_COMPUTE_STORAGE_BUFFERS_PER_STAGE,
        ..wgpu::Limits::default()
    }
    .using_resolution(adapter.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negotiates_dx12_and_weaker_storage_limits() {
        for (buffer, storage) in [
            (i32::MAX as u64, i32::MAX as u64),
            (1 << 30, 512 << 20),
            (512 << 20, 1 << 30),
            (8 << 30, 4 << 30),
        ] {
            let adapter = wgpu::Limits {
                max_buffer_size: buffer,
                max_storage_buffer_binding_size: storage,
                max_storage_buffers_per_shader_stage: 32,
                max_texture_dimension_2d: 16384,
                ..Default::default()
            };
            let requested = required_device_limits(&adapter).unwrap();
            assert_eq!(
                requested.max_buffer_size,
                buffer.min(DEVICE_BUFFER_TARGET_BYTES)
            );
            assert_eq!(
                requested.max_storage_buffer_binding_size,
                storage.min(buffer).min(DEVICE_STORAGE_BINDING_TARGET_BYTES)
            );
            assert_eq!(requested.max_storage_buffers_per_shader_stage, 10);
            assert_eq!(requested.max_texture_dimension_2d, 16384);
        }
    }

    #[test]
    fn never_silently_lowers_the_compute_requirement() {
        for available in [8, 9, 10] {
            let adapter = wgpu::Limits {
                max_storage_buffers_per_shader_stage: available,
                ..Default::default()
            };
            match required_device_limits(&adapter) {
                Ok(limits) => assert_eq!(limits.max_storage_buffers_per_shader_stage, 10),
                Err(error) => {
                    assert!(available < 10);
                    assert_eq!(error.available, available);
                    assert_eq!(error.required, 10);
                }
            }
        }
    }
}
