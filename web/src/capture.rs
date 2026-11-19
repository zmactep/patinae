//! Offscreen WebGPU capture with asynchronous readback.

use std::{future::Future, pin::Pin};

use futures_channel::oneshot;
use patinae_render::capture::TextureReadback;
use patinae_scene::{
    bridge::{frame_uniforms_from_session, CachedRenderScene},
    Session,
};

use crate::gpu::GpuState;

pub(crate) struct PendingCapture {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) view_proj: [[f32; 4]; 4],
    mapped: Pin<Box<dyn Future<Output = Result<TextureReadback, String>>>>,
}

impl PendingCapture {
    pub(crate) async fn rgba(self, cancelled: impl FnOnce() -> bool) -> Result<Vec<u8>, String> {
        // Mapping completion fences the submitted GPU copy. Keep the capture
        // slot until then, even on cancellation, but skip the CPU readback.
        let readback = self.mapped.await?;
        if cancelled() {
            readback.buffer().unmap();
            return Err("PNG export cancelled".into());
        }
        readback
            .take_mapped_straight_rgba()
            .map_err(|e| e.to_string())
    }
}

fn dimensions(
    live: (u32, u32),
    width: Option<u32>,
    height: Option<u32>,
) -> Result<(u32, u32), String> {
    if width == Some(0) || height == Some(0) {
        return Err("PNG dimensions must be positive".into());
    }
    let scaled = |specified, numerator, denominator| {
        let value = (u64::from(specified) * u64::from(numerator) + u64::from(denominator) / 2)
            / u64::from(denominator);
        u32::try_from(value.max(1)).map_err(|_| "PNG dimensions exceed the pixel range".to_owned())
    };
    match (width, height) {
        (Some(w), Some(h)) => Ok((w, h)),
        (Some(w), None) => Ok((w, scaled(w, live.1, live.0)?)),
        (None, Some(h)) => Ok((scaled(h, live.0, live.1)?, h)),
        (None, None) => Ok(live),
    }
}

/// Submit the capture before restoring live uniforms or yielding to JavaScript.
pub(crate) fn begin(
    gpu: &mut GpuState,
    session: &mut Session,
    render_scene: &mut CachedRenderScene,
    width: Option<u32>,
    height: Option<u32>,
    transparent: Option<bool>,
) -> Result<PendingCapture, String> {
    let live = (gpu.surface_config.width, gpu.surface_config.height);
    let (width, height) = dimensions(live, width, height)?;
    let opaque = transparent.map_or(session.settings.ui.opaque_background, |value| !value);
    let device = gpu.state.ctx.device.clone();
    let oom = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);

    let size = (width, height);
    let frame = frame_uniforms_from_session(session, size, session.clear_color);
    let background = patinae_render::RenderState::background_color(session.clear_color, opaque);
    let readback = super::prepare_scene(gpu, session, render_scene)
        .map_err(|error| error.to_string())
        .and_then(|_| {
            gpu.state
                .submit_capture(size, frame, background)
                .map_err(|error| error.to_string())
        });
    let validation = validation.pop();
    let oom = oom.pop();
    let readback = readback?;
    let mapped = Box::pin(async move {
        if let Some(error) = validation.await.or(oom.await) {
            return Err(format!("PNG {width}x{height}: {error}"));
        }
        let (tx, rx) = oneshot::channel();
        readback
            .buffer()
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = tx.send(result);
            });
        rx.await
            .map_err(|_| "GPU closed during PNG readback".to_owned())?
            .map_err(|error| format!("PNG buffer map failed: {error}"))?;
        Ok(readback)
    });
    Ok(PendingCapture {
        width,
        height,
        view_proj: frame.view_proj,
        mapped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_physical_pixel_dimensions() {
        let live = (800, 600);
        assert_eq!(dimensions(live, None, None), Ok(live));
        assert_eq!(dimensions(live, Some(1600), None), Ok((1600, 1200)));
        assert_eq!(dimensions(live, None, Some(300)), Ok((400, 300)));
        assert_eq!(dimensions(live, Some(801), Some(509)), Ok((801, 509)));
        assert_eq!(dimensions((1, 9), None, Some(1)), Ok((1, 1)));
        assert!(dimensions(live, Some(0), None).is_err());
        assert!(dimensions((1, 10), Some(u32::MAX), None).is_err());
    }
}
