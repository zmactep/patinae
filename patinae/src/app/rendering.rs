//! Rendering for the native application.

use super::*;

impl App {
    pub(super) fn setup_renderer(
        &mut self,
        api: &slint::GraphicsAPI,
        app: &AppWindow,
    ) -> Option<()> {
        self.rendering.renderer_startup = ViewportRenderer::schedule(api);
        self.sync_notifications(app);
        self.rendering.renderer_startup.as_ref().map(|_| ())
    }

    pub(super) fn poll_renderer_startup(&mut self, app: &AppWindow) {
        let Some(startup) = self.rendering.renderer_startup.as_mut() else {
            return;
        };
        let prepared = match startup.tick() {
            Ok(Some(result)) => result,
            Ok(None) => return,
            Err(error) => {
                self.rendering.renderer_startup = None;
                log::error!("{error}");
                self.kernel.output.print_error(error);
                return;
            }
        };
        self.rendering.renderer_startup = None;
        let started = Instant::now();
        let (renderer, info) = prepared.finish();
        let (vw, vh) = Self::viewport_size(app);
        self.viewport.resize(vw, vh);
        self.kernel.resize(vw, vh);
        self.rendering.renderer = Some(renderer);
        self.mark_display_transition();

        if let Some(info) = info {
            log::info!("GPU: {} ({})", info.device_name, info.backend);
            let vp = app.global::<ViewportState>();
            vp.set_device_name(info.device_name.into());
            vp.set_engine_name(info.backend.into());
        }

        log::info!(
            "Viewport attached: {:.3} ms",
            started.elapsed().as_secs_f64() * 1000.0
        );
        app.window().request_redraw();
    }

    pub(super) fn teardown_renderer(&mut self) {
        log::info!("Rendering teardown — dropping viewport");
        self.rendering.renderer_startup = None;
        self.rendering.renderer = None;
    }

    pub(super) fn mark_display_transition(&self) {
        let mut recovery = self.rendering.display_recovery.get();
        recovery.mark_transition();
        self.rendering.display_recovery.set(recovery);
    }

    pub(super) fn take_live_texture_invalidation(&self) -> bool {
        let mut recovery = self.rendering.display_recovery.get();
        let invalid = recovery.take_live_texture_invalidation();
        self.rendering.display_recovery.set(recovery);
        invalid
    }
}

/// Owns rendering state.
pub(super) struct Rendering {
    pub(super) renderer: Option<ViewportRenderer>,
    pub(super) renderer_startup: Option<
        crate::renderer_startup::RendererStartup<crate::components::viewport::PreparedViewport>,
    >,
    pub(super) display_recovery: Cell<DisplayRecovery>,
}
