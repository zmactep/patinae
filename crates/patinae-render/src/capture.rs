//! Headless PNG capture via the live `RenderState`.
//!
//! Reuses the same frame targets, picking, WBOIT, marking, and silhouette
//! passes as the live viewport, so captured images match interactive frames
//! modulo the requested resolution.
//!
//! The function is synchronous: it submits the encoder, blocks on
//! `device.poll(Wait)` until the readback buffer is mapped. Callers can either
//! receive RGBA bytes directly or encode them as PNG.

use std::path::Path;

use crate::render_input::RenderInput;
use crate::render_state::RenderState;
use crate::uniforms::FrameUniforms;

/// Render `input` headlessly into an offscreen Rgba8Unorm texture sized
/// `(width, height)` and write it as PNG to `path`. Resizes the
/// `RenderState`'s frame targets to match — the caller is responsible for
/// resizing back to the live viewport size before the next live frame.
///
/// `frame_uniforms` should already carry view / proj / fog / clip for the
/// requested capture aspect (the caller builds these via the host bridges).
/// `silhouette_params` mirrors `RenderState::set_silhouette` — pass `None`
/// to skip the pass.
pub fn capture_png(
    state: &mut RenderState,
    path: &Path,
    width: u32,
    height: u32,
    frame_uniforms: &FrameUniforms,
    input: &RenderInput,
) -> Result<(), CaptureError> {
    let tightly_packed = capture_rgba(state, width, height, frame_uniforms, input)?;
    let width = width.max(1);
    let height = height.max(1);
    let buffer = image::RgbaImage::from_raw(width, height, tightly_packed).ok_or(
        CaptureError::Image("RgbaImage::from_raw size mismatch".into()),
    )?;
    buffer
        .save_with_format(path, image::ImageFormat::Png)
        .map_err(|e| CaptureError::Image(format!("PNG encode failed: {e}")))?;
    Ok(())
}

/// Render `input` headlessly and return tightly-packed RGBA8 bytes.
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
    state.sync(input);

    // Offscreen colour target — Rgba8Unorm so we can directly read it
    // back as RGBA bytes for PNG encoding.
    let format = wgpu::TextureFormat::Rgba8Unorm;
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
    state.ctx.queue.submit(std::iter::once(encoder.finish()));

    read_texture_rgba(
        &state.ctx.device,
        &state.ctx.queue,
        &color_texture,
        width,
        height,
        "patinae.capture",
    )
}

pub(crate) fn read_texture_rgba(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
    label_prefix: &str,
) -> Result<Vec<u8>, CaptureError> {
    if width == 0 || height == 0 {
        return Err(CaptureError::Gpu(
            "RGBA texture readback dimensions must be non-zero".into(),
        ));
    }
    let unpadded_bpr = width
        .checked_mul(4)
        .ok_or_else(|| CaptureError::Gpu("RGBA texture row size overflow".into()))?;
    let alignment = u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let aligned_bpr_u64 = u64::from(unpadded_bpr).div_ceil(alignment) * alignment;
    let aligned_bpr = u32::try_from(aligned_bpr_u64)
        .map_err(|_| CaptureError::Gpu("RGBA texture aligned row size overflow".into()))?;
    let staging_size = aligned_bpr_u64
        .checked_mul(u64::from(height))
        .ok_or_else(|| CaptureError::Gpu("RGBA texture readback buffer size overflow".into()))?;
    let output_size = u64::from(unpadded_bpr)
        .checked_mul(u64::from(height))
        .ok_or_else(|| CaptureError::Gpu("RGBA texture output size overflow".into()))?;
    let output_capacity = usize::try_from(output_size)
        .map_err(|_| CaptureError::Gpu("RGBA texture output size exceeds usize".into()))?;
    let aligned_bpr_usize = usize::try_from(aligned_bpr_u64)
        .map_err(|_| CaptureError::Gpu("RGBA texture row stride exceeds usize".into()))?;
    let unpadded_bpr_usize = unpadded_bpr as usize;

    let readback_label = format!("{label_prefix}.readback");
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(readback_label.as_str()),
        size: staging_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let encoder_label = format!("{label_prefix}.readback.encoder");
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some(encoder_label.as_str()),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(aligned_bpr),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(std::iter::once(encoder.finish()));

    // Block until the staging buffer is mapped.
    let slice = staging.slice(..);
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
        .map_err(|e| CaptureError::Gpu(format!("buffer map failed: {e:?}")))?;

    // Strip row padding while copying into a tightly packed RGBA8 buffer.
    let mapped = slice
        .get_mapped_range()
        .expect("readback range is mapped and has no overlapping views");
    let mut tightly_packed = Vec::with_capacity(output_capacity);
    for row in 0..height as usize {
        let start = row * aligned_bpr_usize;
        let end = start + unpadded_bpr_usize;
        tightly_packed.extend_from_slice(&mapped[start..end]);
    }
    drop(mapped);
    staging.unmap();

    Ok(tightly_packed)
}

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("GPU error during capture: {0}")]
    Gpu(String),
    #[error("Image encode error: {0}")]
    Image(String),
}
