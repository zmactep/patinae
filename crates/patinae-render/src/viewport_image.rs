//! Viewport texture helpers for externally generated GPU images.

use wgpu::util::DeviceExt;

use crate::capture::{read_texture_rgba, CaptureError};
use crate::shader_source;

const VIEWPORT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// GPU texture ready for host viewport presentation.
#[must_use]
pub struct GpuViewportImage {
    /// Texture containing tightly packed RGBA8 image data.
    pub texture: wgpu::Texture,
    /// Image width in physical pixels.
    pub width: u32,
    /// Image height in physical pixels.
    pub height: u32,
}

impl GpuViewportImage {
    /// Read this viewport texture as tightly packed RGBA8 bytes.
    ///
    /// The output is row-major, top-to-bottom, and has exactly
    /// `width * height * 4` bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if GPU readback, polling, or buffer mapping fails.
    pub fn read_rgba(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Result<Vec<u8>, CaptureError> {
        read_texture_rgba(
            device,
            queue,
            &self.texture,
            self.width,
            self.height,
            "patinae.ray.viewport",
        )
    }
}

/// Convert a tightly packed RGBA8 storage buffer into a viewport texture.
///
/// The source buffer is interpreted as little-endian packed `u32` RGBA pixels.
///
/// # Errors
///
/// Returns an error when dimensions are zero, image byte size overflows, or
/// `buffer_size` is smaller than `width * height * 4`.
pub fn copy_rgba_buffer_to_viewport_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    buffer: &wgpu::Buffer,
    buffer_size: u64,
    width: u32,
    height: u32,
) -> Result<GpuViewportImage, GpuViewportImageError> {
    validate_rgba_buffer_size(width, height, buffer_size)?;

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("patinae.ray.viewport"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: VIEWPORT_FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::STORAGE_BINDING
            // Slint's WGPU texture import requires render-attachment usage.
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("patinae.ray.buffer_to_texture.shader"),
        source: wgpu::ShaderSource::Wgsl(shader_source::RAY_BUFFER_TO_TEXTURE_WGSL.into()),
    });
    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("patinae.ray.buffer_to_texture.layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format: VIEWPORT_FORMAT,
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("patinae.ray.buffer_to_texture.pipeline_layout"),
        bind_group_layouts: &[Some(&bind_group_layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("patinae.ray.buffer_to_texture.pipeline"),
        layout: Some(&pipeline_layout),
        module: &shader,
        entry_point: Some("main"),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    });
    let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("patinae.ray.buffer_to_texture.params"),
        contents: &viewport_copy_params(width, height),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("patinae.ray.buffer_to_texture.bind_group"),
        layout: &bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: params_buffer.as_entire_binding(),
            },
        ],
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("patinae.ray.buffer_to_texture.encoder"),
    });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("patinae.ray.buffer_to_texture.pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, Some(&bind_group), &[]);
        pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(8), 1);
    }
    queue.submit(std::iter::once(encoder.finish()));

    Ok(GpuViewportImage {
        texture,
        width,
        height,
    })
}

fn validate_rgba_buffer_size(
    width: u32,
    height: u32,
    buffer_size: u64,
) -> Result<(), GpuViewportImageError> {
    let required = required_rgba_bytes(width, height)?;
    if buffer_size < required {
        return Err(GpuViewportImageError::BufferTooSmall {
            actual: buffer_size,
            required,
        });
    }
    Ok(())
}

fn required_rgba_bytes(width: u32, height: u32) -> Result<u64, GpuViewportImageError> {
    if width == 0 || height == 0 {
        return Err(GpuViewportImageError::InvalidDimensions);
    }
    u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(GpuViewportImageError::SizeOverflow)
}

fn viewport_copy_params(width: u32, height: u32) -> [u8; 16] {
    let mut bytes = [0_u8; 16];
    bytes[0..4].copy_from_slice(&width.to_ne_bytes());
    bytes[4..8].copy_from_slice(&height.to_ne_bytes());
    bytes
}

/// Errors while preparing an externally generated viewport image.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum GpuViewportImageError {
    /// The requested viewport image has a zero dimension.
    #[error("ray viewport image dimensions must be non-zero")]
    InvalidDimensions,
    /// The requested viewport image is too large to address safely.
    #[error("ray viewport image size overflow")]
    SizeOverflow,
    /// The source buffer is smaller than the requested image.
    #[error("ray output buffer is {actual} bytes, expected at least {required}")]
    BufferTooSmall {
        /// Actual source buffer size in bytes.
        actual: u64,
        /// Required source buffer size in bytes.
        required: u64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_rgba_bytes_rejects_zero_dimensions() {
        assert_eq!(
            required_rgba_bytes(0, 1),
            Err(GpuViewportImageError::InvalidDimensions)
        );
        assert_eq!(
            required_rgba_bytes(1, 0),
            Err(GpuViewportImageError::InvalidDimensions)
        );
    }

    #[test]
    fn required_rgba_bytes_rejects_overflow() {
        assert_eq!(
            required_rgba_bytes(u32::MAX, u32::MAX),
            Err(GpuViewportImageError::SizeOverflow)
        );
    }

    #[test]
    fn validate_rgba_buffer_size_reports_short_buffer() {
        assert_eq!(
            validate_rgba_buffer_size(2, 2, 15),
            Err(GpuViewportImageError::BufferTooSmall {
                actual: 15,
                required: 16,
            })
        );
    }

    #[test]
    fn viewport_copy_params_packs_width_and_height() {
        let bytes = viewport_copy_params(640, 480);
        assert_eq!(u32::from_ne_bytes(bytes[0..4].try_into().unwrap()), 640);
        assert_eq!(u32::from_ne_bytes(bytes[4..8].try_into().unwrap()), 480);
        assert_eq!(&bytes[8..16], &[0; 8]);
    }
}
