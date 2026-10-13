use std::cell::Cell;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct DisplayRecovery {
    live_textures_invalid: bool,
}

impl DisplayRecovery {
    pub(crate) fn mark_transition(&mut self) {
        self.live_textures_invalid = true;
    }

    pub(crate) fn take_live_texture_invalidation(&mut self) -> bool {
        let invalid = self.live_textures_invalid;
        self.live_textures_invalid = false;
        invalid
    }
}

fn window_event_diagnostics_enabled() -> bool {
    std::env::var("PATINAE_WINDOW_EVENTS")
        .ok()
        .is_some_and(|value| value == "1")
}

pub(crate) fn request_display_recovery_redraw(slint_window: &slint::Window) {
    use slint::winit_030::WinitWindowAccessor;

    slint_window.request_redraw();
    let _ = slint_window.with_winit_window(|winit_window| {
        winit_window.request_redraw();
    });
}

pub(crate) fn mark_display_transition_and_redraw(
    recovery: &Cell<DisplayRecovery>,
    slint_window: &slint::Window,
) {
    let mut state = recovery.get();
    state.mark_transition();
    recovery.set(state);
    request_display_recovery_redraw(slint_window);
}

pub(crate) fn log_window_event(
    slint_window: &slint::Window,
    event: &str,
    detail: std::fmt::Arguments<'_>,
) {
    use slint::winit_030::WinitWindowAccessor;

    if !window_event_diagnostics_enabled() {
        return;
    }

    let slint_size = slint_window.size();
    let slint_scale = slint_window.scale_factor();
    let winit_info = slint_window.with_winit_window(|winit_window| {
        let size = winit_window.inner_size();
        let monitor = winit_window.current_monitor();
        let monitor_name = monitor
            .as_ref()
            .and_then(|monitor| monitor.name())
            .unwrap_or_else(|| "unknown".to_string());
        let refresh_millihertz = monitor.and_then(|monitor| monitor.refresh_rate_millihertz());
        (
            size.width,
            size.height,
            winit_window.scale_factor(),
            monitor_name,
            refresh_millihertz,
        )
    });

    if let Some((winit_width, winit_height, winit_scale, monitor_name, refresh_millihertz)) =
        winit_info
    {
        log::info!(
            "window event {event}: {detail}; slint_size={}x{} slint_scale={:.3} \
             winit_inner={}x{} winit_scale={:.3} monitor={} refresh_millihertz={:?}",
            slint_size.width,
            slint_size.height,
            slint_scale,
            winit_width,
            winit_height,
            winit_scale,
            monitor_name,
            refresh_millihertz,
        );
    } else {
        log::info!(
            "window event {event}: {detail}; slint_size={}x{} slint_scale={:.3} winit=unavailable",
            slint_size.width,
            slint_size.height,
            slint_scale,
        );
    }
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn log_selected_gpu(info: &slint::wgpu_30::wgpu::AdapterInfo) {
    if window_event_diagnostics_enabled() {
        log::info!(
            "selected GPU: {} ({:?}) vendor={} device={}",
            info.name,
            info.backend,
            info.vendor,
            info.device,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_recovery_coalesces_repeated_texture_invalidations() {
        let mut recovery = DisplayRecovery::default();

        recovery.mark_transition();
        recovery.mark_transition();

        assert!(recovery.take_live_texture_invalidation());
        assert!(!recovery.take_live_texture_invalidation());
    }
}
