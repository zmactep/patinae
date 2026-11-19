//! Headless PNG capture via the live `RenderState`.
//!
//! PNG uses offscreen targets and the viewport's scene, lighting and postprocessing.
//! Raw viewport capture also runs the interactive passes for callers that inspect picking.
//!
//! The function is synchronous: it submits the encoder, blocks on
//! `device.poll(Wait)` until the readback buffer is mapped. Callers can either
//! receive RGBA bytes directly or encode them as PNG.

use std::path::Path;

use crate::render_input::RenderInput;
use crate::render_state::RenderState;
use crate::uniforms::FrameUniforms;

/// Render `input` into independent offscreen targets and write a straight-alpha PNG.
///
/// `frame_uniforms` should already carry view / proj / fog / clip for the
/// requested capture aspect (the caller builds these via the host bridges).
pub fn capture_png(
    state: &mut RenderState,
    path: &Path,
    width: u32,
    height: u32,
    frame_uniforms: &FrameUniforms,
    input: &RenderInput,
) -> Result<(), CaptureError> {
    let width = width.max(1);
    let height = height.max(1);
    state
        .sync(input)
        .map_err(|err| CaptureError::Gpu(err.to_string()))?;
    let readback = state.submit_capture((width, height), *frame_uniforms, state.clear_color())?;
    readback.map_blocking(&state.ctx.device)?;
    let tightly_packed = readback.take_mapped_straight_rgba()?;
    let buffer = image::RgbaImage::from_raw(width, height, tightly_packed).ok_or(
        CaptureError::Image("RgbaImage::from_raw size mismatch".into()),
    )?;
    buffer
        .save_with_format(path, image::ImageFormat::Png)
        .map_err(|e| CaptureError::Image(format!("PNG encode failed: {e}")))?;
    Ok(())
}

/// Render `input` headlessly and return tightly packed premultiplied RGBA8 bytes.
///
/// The output is row-major, top-to-bottom, and has exactly
/// `width.max(1) * height.max(1) * 4` bytes.
pub fn capture_rgba(
    state: &mut RenderState,
    width: u32,
    height: u32,
    frame_uniforms: &FrameUniforms,
    input: &RenderInput,
) -> Result<Vec<u8>, CaptureError> {
    let width = width.max(1);
    let height = height.max(1);

    state.uniforms = *frame_uniforms;
    state.uniforms.set_viewport(width, height);
    // Use the normal resize path so all target-dependent bind groups
    // (WBOIT composite, SSAO, FXAA, overlay resources) point at the capture
    // targets instead of stale live-viewport targets.
    state.resize((width, height));
    state
        .sync(input)
        .map_err(|error| CaptureError::Gpu(error.to_string()))?;

    let format = state.ctx.color_format;
    let color_texture = state.ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("patinae.capture.color"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let color_view = color_texture.create_view(&wgpu::TextureViewDescriptor::default());

    // Render into an offscreen texture, then reuse the shared RGBA readback
    // helper so capture and viewport GPU images stay byte-for-byte aligned.
    let mut encoder = state
        .ctx
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("patinae.capture.encoder"),
        });
    state.render(&color_view, &mut encoder);
    let readback = TextureReadback::record(
        &state.ctx.device,
        &mut encoder,
        &color_texture,
        width,
        height,
        "patinae.capture",
    )?;
    state.ctx.queue.submit([encoder.finish()]);
    readback.map_blocking(&state.ctx.device)?;
    readback.take_mapped_rgba(ReadbackAlpha::Preserve)
}

pub(crate) fn read_texture_rgba(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
    label_prefix: &str,
) -> Result<Vec<u8>, CaptureError> {
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some(label_prefix),
    });
    let readback =
        TextureReadback::record(device, &mut encoder, texture, width, height, label_prefix)?;
    queue.submit([encoder.finish()]);

    readback.map_blocking(device)?;
    // External viewport images already contain the producer's RGBA bytes.
    readback.take_mapped_rgba(ReadbackAlpha::Preserve)
}

/// A four-byte-per-pixel texture copy with WebGPU-aligned rows.
#[derive(Debug)]
pub struct TextureReadback {
    buffer: std::sync::Arc<wgpu::Buffer>,
    width: u32,
    height: u32,
    stride: u32,
    bgra: bool,
}

impl TextureReadback {
    /// Compute the padded row stride and staging allocation size.
    ///
    /// # Errors
    /// Returns an error for empty dimensions or an unrepresentable buffer.
    pub fn layout(width: u32, height: u32) -> Result<(u32, u64), CaptureError> {
        if width == 0 || height == 0 {
            return Err(CaptureError::Gpu("Empty texture readback".into()));
        }
        let alignment = u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let stride = (u64::from(width) * 4).div_ceil(alignment) * alignment;
        let stride = u32::try_from(stride)
            .map_err(|_| CaptureError::Gpu("Readback row exceeds u32".into()))?;
        let size = u64::from(stride) * u64::from(height);
        if size > isize::MAX as u64 {
            return Err(CaptureError::Gpu(
                "Readback exceeds addressable memory".into(),
            ));
        }
        Ok((stride, size))
    }

    /// Record a copy into a new staging buffer.
    ///
    /// # Errors
    /// Returns an error when the dimensions exceed host or device limits.
    pub fn record(
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        texture: &wgpu::Texture,
        width: u32,
        height: u32,
        label: &str,
    ) -> Result<Self, CaptureError> {
        let bgra = match texture.format() {
            wgpu::TextureFormat::Bgra8Unorm => true,
            wgpu::TextureFormat::Rgba8Unorm => false,
            format => {
                return Err(CaptureError::Gpu(format!(
                    "Unsupported readback format {format:?}"
                )))
            }
        };
        let (stride, size) = Self::layout(width, height)?;

        if size > device.limits().max_buffer_size {
            return Err(CaptureError::Gpu(format!(
                "{width}x{height} needs {size} readback bytes, exceeding the {} byte device buffer limit",
                device.limits().max_buffer_size
            )));
        }
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        Ok(Self {
            buffer: std::sync::Arc::new(buffer),
            width,
            height,
            stride,
            bgra,
        })
    }

    /// Return the staging buffer for host-specific asynchronous mapping.
    pub fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }

    pub(crate) fn weak_buffer(&self) -> std::sync::Weak<wgpu::Buffer> {
        std::sync::Arc::downgrade(&self.buffer)
    }

    fn map_blocking(&self, device: &wgpu::Device) -> Result<(), CaptureError> {
        // Block until the staging buffer is mapped.
        let slice = self.buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .map_err(|e| CaptureError::Gpu(format!("device poll failed: {e:?}")))?;
        rx.recv()
            .map_err(|_| CaptureError::Gpu("staging buffer map channel closed".into()))?
            .map_err(|e| CaptureError::Gpu(format!("buffer map failed: {e:?}")))
    }

    /// Unpack a premultiplied render target into straight RGBA8 for PNG encoding.
    ///
    /// # Errors
    /// Returns an error if mapping has not completed or allocation fails.
    pub fn take_mapped_straight_rgba(self) -> Result<Vec<u8>, CaptureError> {
        self.take_mapped_rgba(ReadbackAlpha::Straight)
    }

    fn take_mapped_rgba(self, alpha: ReadbackAlpha) -> Result<Vec<u8>, CaptureError> {
        let row_bytes = self.width as usize * 4;
        let size = row_bytes * self.height as usize;
        let mut pixels = Vec::new();
        pixels.try_reserve_exact(size).map_err(|error| {
            CaptureError::Gpu(format!("Cannot allocate {size} readback bytes: {error}"))
        })?;
        // The browser backend copies each mapped view into WASM memory on
        // dereference. Limit that temporary copy to about 1 MiB per block.
        let stride = self.stride as usize;
        let rows_per_block = ((1024 * 1024) / stride).max(1);
        let height = self.height as usize;
        for first in (0..height).step_by(rows_per_block) {
            let last = (first + rows_per_block).min(height);
            let range = (first * stride) as u64..(last * stride) as u64;
            let mapped = self
                .buffer
                .slice(range)
                .get_mapped_range()
                .map_err(|error| {
                    CaptureError::Gpu(format!("Readback mapping unavailable: {error}"))
                })?;
            for row in mapped.chunks_exact(stride) {
                append_rgba_row(&mut pixels, &row[..row_bytes], self.bgra, alpha);
            }
        }
        self.buffer.unmap();
        Ok(pixels)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReadbackAlpha {
    Preserve,
    Straight,
}

/// Convert channel order and alpha while copying the unpadded row.
fn append_rgba_row(output: &mut Vec<u8>, row: &[u8], bgra: bool, alpha: ReadbackAlpha) {
    let start = output.len();
    output.extend_from_slice(row);
    if !bgra && alpha == ReadbackAlpha::Preserve {
        return;
    }
    for pixel in output[start..].as_chunks_mut::<4>().0 {
        if bgra {
            pixel.swap(0, 2);
        }
        if alpha == ReadbackAlpha::Straight {
            match pixel[3] {
                0 => pixel[..3].fill(0),
                255 => {}
                a => {
                    let a = u32::from(a);
                    for channel in &mut pixel[..3] {
                        *channel = ((u32::from(*channel) * 255 + a / 2) / a).min(255) as u8;
                    }
                }
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("GPU error during capture: {0}")]
    Gpu(String),
    #[error("Image encode error: {0}")]
    Image(String),
}

#[cfg(test)]
mod tests;
