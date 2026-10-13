//! GPU buffer creation, validation, and readback.

use wgpu::util::DeviceExt;

use crate::error::{RaytraceError, RaytraceResult};
use crate::primitive::Primitives;
use patinae_render::bytes_to_mib;

/// Readback buffer layout for tightly extracted RGBA rows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ReadbackLayout {
    pub padded_bytes_per_row: u32,
    pub buffer_size: u64,
}

impl ReadbackLayout {
    /// Create a texture-copy compatible readback layout.
    pub fn new(width: u32, height: u32) -> Self {
        let bytes_per_pixel = 4u32;
        let unpadded_bytes_per_row = width * bytes_per_pixel;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_bytes_per_row = unpadded_bytes_per_row.div_ceil(align) * align;
        let buffer_size = u64::from(padded_bytes_per_row) * u64::from(height);
        Self {
            padded_bytes_per_row,
            buffer_size,
        }
    }
}

/// Persistent MAP_READ target for final raytrace pixels.
pub(crate) struct ReadbackBuffer {
    pub buffer: wgpu::Buffer,
    pub layout: ReadbackLayout,
}

impl ReadbackBuffer {
    /// Create a readback buffer for the final output size.
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let layout = ReadbackLayout::new(width, height);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Raytrace Readback"),
            size: layout.buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Self { buffer, layout }
    }
}

/// Check that primitive buffers don't exceed GPU limits.
pub(crate) fn validate_buffer_sizes(
    device: &wgpu::Device,
    primitives: &Primitives,
) -> RaytraceResult<()> {
    let max_buffer_size = device.limits().max_storage_buffer_binding_size as usize;
    let sphere_size = primitives.spheres.len() * std::mem::size_of::<crate::primitive::GpuSphere>();
    let cylinder_size =
        primitives.cylinders.len() * std::mem::size_of::<crate::primitive::GpuCylinder>();
    let capsule_size =
        primitives.capsules.len() * std::mem::size_of::<crate::primitive::GpuCapsule>();
    let triangle_size =
        primitives.triangles.len() * std::mem::size_of::<crate::primitive::GpuTriangle>();

    log::debug!(
        "Buffer sizes: spheres={:.1}MB, cylinders={:.1}MB, capsules={:.1}MB, triangles={:.1}MB (limit={:.1}MB)",
        bytes_to_mib(sphere_size as u64),
        bytes_to_mib(cylinder_size as u64),
        bytes_to_mib(capsule_size as u64),
        bytes_to_mib(triangle_size as u64),
        bytes_to_mib(max_buffer_size as u64)
    );

    if triangle_size > max_buffer_size {
        return Err(RaytraceError::RenderFailed(format!(
            "Triangle buffer too large ({:.1} MB, limit {:.1} MB). Scene has {} triangles. \
             Try reducing geometry detail with 'set cartoon_sampling' or use a simpler representation.",
            bytes_to_mib(triangle_size as u64),
            bytes_to_mib(max_buffer_size as u64),
            primitives.triangles.len()
        )));
    }
    if sphere_size > max_buffer_size {
        return Err(RaytraceError::RenderFailed(format!(
            "Sphere buffer too large ({:.1} MB, limit {:.1} MB). Scene has {} spheres.",
            bytes_to_mib(sphere_size as u64),
            bytes_to_mib(max_buffer_size as u64),
            primitives.spheres.len()
        )));
    }
    if cylinder_size > max_buffer_size {
        return Err(RaytraceError::RenderFailed(format!(
            "Cylinder buffer too large ({:.1} MB, limit {:.1} MB). Scene has {} cylinders.",
            bytes_to_mib(cylinder_size as u64),
            bytes_to_mib(max_buffer_size as u64),
            primitives.cylinders.len()
        )));
    }
    if capsule_size > max_buffer_size {
        return Err(RaytraceError::RenderFailed(format!(
            "Capsule buffer too large ({:.1} MB, limit {:.1} MB). Scene has {} capsules.",
            bytes_to_mib(capsule_size as u64),
            bytes_to_mib(max_buffer_size as u64),
            primitives.capsules.len()
        )));
    }

    Ok(())
}

/// Create a storage buffer from data.
///
/// When data is empty, creates a buffer with one zeroed element to satisfy
/// WGSL's minimum binding size requirements.
pub(crate) fn create_storage_buffer<T: bytemuck::Pod + bytemuck::Zeroable>(
    device: &wgpu::Device,
    label: &str,
    data: &[T],
) -> wgpu::Buffer {
    if data.is_empty() {
        let dummy = T::zeroed();
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytemuck::bytes_of(&dummy),
            usage: wgpu::BufferUsages::STORAGE,
        })
    } else {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytemuck::cast_slice(data),
            usage: wgpu::BufferUsages::STORAGE,
        })
    }
}

/// Record a texture-to-buffer copy into a reusable readback buffer.
pub(crate) fn record_texture_copy_to_buffer(
    encoder: &mut wgpu::CommandEncoder,
    output_texture: &wgpu::Texture,
    readback_buffer: &ReadbackBuffer,
    width: u32,
    height: u32,
) {
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: output_texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback_buffer.buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(readback_buffer.layout.padded_bytes_per_row),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
}

/// Map and read pixels from a readback buffer (call after queue submission).
pub(crate) fn map_readback_buffer(
    device: &wgpu::Device,
    readback_buffer: &wgpu::Buffer,
    padded_bytes_per_row: u32,
    width: u32,
    height: u32,
) -> RaytraceResult<Vec<u8>> {
    let bytes_per_pixel = 4u32;

    let buffer_slice = readback_buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: None,
    });
    rx.recv()
        .map_err(|e| RaytraceError::RenderFailed(format!("Failed to receive map result: {e}")))?
        .map_err(|e| RaytraceError::RenderFailed(format!("Failed to map buffer: {e:?}")))?;

    let data = buffer_slice
        .get_mapped_range()
        .expect("readback range is mapped and has no overlapping views");
    let mut pixels = Vec::with_capacity((width * height * bytes_per_pixel) as usize);
    for row in 0..height {
        let start = (row * padded_bytes_per_row) as usize;
        let end = start + (width * bytes_per_pixel) as usize;
        pixels.extend_from_slice(&data[start..end]);
    }
    drop(data);
    readback_buffer.unmap();

    Ok(pixels)
}
