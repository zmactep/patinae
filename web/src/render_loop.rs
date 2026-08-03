//! Frame rendering — drives `patinae_render::RenderState` via the shared
//! `patinae_scene::bridge` helpers (same path patinae uses).

use patinae_render::RenderSyncTimings;
use patinae_scene::bridge::{
    frame_uniforms_from_session, resolve_setting_color, CachedRenderScene,
};
use patinae_scene::Session;
use patinae_settings::{Settings, ShadingMode};

use crate::gpu::GpuState;

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
}

/// Render one frame to the canvas surface.
///
pub fn render_frame(
    gpu: &mut GpuState,
    session: &mut Session,
    render_scene: &mut CachedRenderScene,
) -> Result<WebRenderTimings, WebSurfaceStatus> {
    let mut timings = WebRenderTimings::default();
    let width = gpu.surface_config.width;
    let height = gpu.surface_config.height;

    let t0 = performance_now_ms();
    // Frame uniforms (camera/light/fog/clip) — same math as patinae.
    let frame = frame_uniforms_from_session(session, (width, height), session.clear_color);
    gpu.state.uniforms = frame;
    gpu.state.ctx.upload_frame(&gpu.state.uniforms);
    // Live viewports should show the RGB background; PNG capture keeps
    // using `opaque_background` to decide exported alpha.
    gpu.state.set_clear_color(session.clear_color, true);
    gpu.state
        .set_marking_width(session.settings.ui.selection_width);
    timings.uniforms_ms = elapsed_ms(t0);

    // Cached host-side render input. This keeps the web path aligned with
    // desktop and avoids per-frame color/marker rebuilds on hover redraws.
    let t0 = performance_now_ms();
    let frame = render_scene.prepare(session);
    timings.prepare_ms = elapsed_ms(t0);
    let t0 = performance_now_ms();
    gpu.state
        .sync_with_timer(&frame.render_input(), performance_now_ms);
    timings.sync_ms = elapsed_ms(t0);
    timings.sync_detail = gpu.state.last_sync_timings();
    drop(frame);
    session.registry.clear_all_dirty_objects();

    let t0 = performance_now_ms();
    // Silhouette setting.
    let common = &session.settings.shading.common;
    let silhouette_ink = resolve_setting_color(
        common.silhouette_color,
        &session.named_palette,
        [0.0, 0.0, 0.0, 1.0],
    );
    if common.silhouettes {
        gpu.state
            .set_silhouette(true, common.silhouette_width, silhouette_ink);
    } else {
        gpu.state.set_silhouette(false, 1.0, silhouette_ink);
    }
    let full = &session.settings.shading.full;
    gpu.state.set_shadows(
        session.settings.shading.mode == ShadingMode::Full,
        full.shadow_map_size.max(64) as u32,
        full.shadow_bias,
        full.shadow_intensity,
        full.shadow_pcf.max(1) as u32,
    );
    configure_ambient_occlusion(gpu, &session.settings);
    gpu.state.set_fxaa(effective_fxaa(&session.settings));
    timings.settings_ms = elapsed_ms(t0);

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
    output.present();
    timings.submit_present_ms = elapsed_ms(t0);

    Ok(timings)
}

fn effective_fxaa(settings: &Settings) -> bool {
    settings.ui.antialias > 0 && settings.fxaa.enabled
}

fn configure_ambient_occlusion(gpu: &mut GpuState, settings: &Settings) {
    let ssao = &settings.ssao;
    if settings.shading.mode == ShadingMode::Skripkin {
        let skripkin = &settings.shading.skripkin;
        gpu.state
            .set_ssao(false, ssao.radius, ssao.intensity, ssao.bias);
        gpu.state.set_skripkin_ao(
            true,
            skripkin.directions.max(1) as u32,
            skripkin.map_size.max(32) as u32,
            skripkin.bias,
            skripkin.intensity,
        );
    } else {
        gpu.state.set_skripkin_ao(false, 1, 32, 0.0, 0.0);
        gpu.state
            .set_ssao(ssao.enabled, ssao.radius, ssao.intensity, ssao.bias);
    }
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
