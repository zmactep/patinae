//! Frame rendering — drives `patinae_render::RenderState` via the shared
//! `patinae_scene::bridge` helpers (same path patinae uses).

use patinae_render::RenderSyncTimings;
use patinae_scene::bridge::{
    configure_frame_effects, frame_uniforms_from_session, CachedRenderScene,
};
use patinae_scene::Session;

use crate::gpu::GpuState;

#[path = "capture.rs"]
pub(crate) mod capture;

#[derive(Debug, Clone, Copy, Default)]
pub struct WebRenderTimings {
    pub uniforms_ms: f32,
    pub prepare_ms: f32,
    pub sync_ms: f32,
    pub settings_ms: f32,
    pub acquire_ms: f32,
    pub encode_ms: f32,
    pub submit_present_ms: f32,
    pub sync_detail: RenderSyncTimings,
}

#[derive(Debug, Clone, Copy)]
pub enum WebSurfaceStatus {
    Lost,
    Outdated,
    Timeout,
    Occluded,
    Validation,
    /// Scene data exceeds a hard device limit; another frame cannot fix it.
    ResourceLimit,
}

/// Render one frame to the canvas surface.
pub fn render_frame(
    gpu: &mut GpuState,
    session: &mut Session,
    render_scene: &mut CachedRenderScene,
) -> Result<WebRenderTimings, WebSurfaceStatus> {
    let t0 = performance_now_ms();
    let size = (gpu.surface_config.width, gpu.surface_config.height);
    gpu.state.uniforms = frame_uniforms_from_session(session, size, session.clear_color);
    gpu.state.ctx.upload_frame(&gpu.state.uniforms);
    gpu.state.set_clear_color(session.clear_color, true);
    let uniforms_ms = elapsed_ms(t0);
    let mut timings =
        prepare_scene(gpu, session, render_scene).map_err(|_| WebSurfaceStatus::ResourceLimit)?;
    timings.uniforms_ms = uniforms_ms;

    // Acquire surface texture and run the FrameGraph.
    let t0 = performance_now_ms();
    let output = match gpu.surface.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(output)
        | wgpu::CurrentSurfaceTexture::Suboptimal(output) => output,
        wgpu::CurrentSurfaceTexture::Timeout => return Err(WebSurfaceStatus::Timeout),
        wgpu::CurrentSurfaceTexture::Occluded => return Err(WebSurfaceStatus::Occluded),
        wgpu::CurrentSurfaceTexture::Outdated => return Err(WebSurfaceStatus::Outdated),
        wgpu::CurrentSurfaceTexture::Lost => return Err(WebSurfaceStatus::Lost),
        wgpu::CurrentSurfaceTexture::Validation => return Err(WebSurfaceStatus::Validation),
    };
    let output_view = output
        .texture
        .create_view(&wgpu::TextureViewDescriptor::default());
    timings.acquire_ms = elapsed_ms(t0);

    let t0 = performance_now_ms();
    let mut encoder =
        gpu.state
            .ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Web Render Encoder"),
            });

    gpu.state.render(&output_view, &mut encoder);
    timings.encode_ms = elapsed_ms(t0);
    let t0 = performance_now_ms();
    gpu.state
        .ctx
        .queue
        .submit(std::iter::once(encoder.finish()));
    gpu.state.ctx.queue.present(output);
    timings.submit_present_ms = elapsed_ms(t0);

    Ok(timings)
}

fn prepare_scene(
    gpu: &mut GpuState,
    session: &mut Session,
    render_scene: &mut CachedRenderScene,
) -> Result<WebRenderTimings, patinae_render::BufferSizeError> {
    let mut timings = WebRenderTimings::default();

    gpu.state
        .set_marking_width(session.settings.ui.selection_width);

    // Cached host-side render input. This keeps the web path aligned with
    // desktop and avoids per-frame color/marker rebuilds on hover redraws.
    let t0 = performance_now_ms();
    let frame = render_scene.prepare(session);
    timings.prepare_ms = elapsed_ms(t0);
    let t0 = performance_now_ms();
    gpu.state
        .sync_with_timer(&frame.render_input(), performance_now_ms)?;
    timings.sync_ms = elapsed_ms(t0);
    timings.sync_detail = gpu.state.last_sync_timings();
    drop(frame);
    session.registry.clear_all_dirty_objects();

    let t0 = performance_now_ms();
    configure_frame_effects(&mut gpu.state, &session.settings, &session.named_palette);
    timings.settings_ms = elapsed_ms(t0);

    Ok(timings)
}

fn performance_now_ms() -> f64 {
    web_sys::window()
        .and_then(|window| window.performance())
        .map(|performance| performance.now())
        .unwrap_or(0.0)
}

fn elapsed_ms(start_ms: f64) -> f32 {
    (performance_now_ms() - start_ms).max(0.0) as f32
}
