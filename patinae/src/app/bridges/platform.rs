//! Platform bridge: OS-specific appearance sync.

#[cfg(target_os = "macos")]
use slint::ComponentHandle;

use crate::AppWindow;

/// Syncs platform-specific appearance (e.g. macOS title bar color).
pub struct PlatformBridge {
    #[cfg(target_os = "macos")]
    prev_titlebar_bg: Option<[f32; 3]>,
}

impl PlatformBridge {
    pub fn new() -> Self {
        Self {
            #[cfg(target_os = "macos")]
            prev_titlebar_bg: None,
        }
    }

    /// Sync platform appearance with Slint theme. Call once per frame.
    pub fn sync(&mut self, _app: &AppWindow) {
        #[cfg(target_os = "macos")]
        {
            let bg = _app
                .global::<crate::Theme>()
                .get_surface_panel()
                .to_argb_f32();
            let new_bg = [bg.red, bg.green, bg.blue];
            // Bootstrap runs before the native window exists. Retry until the
            // color is applied instead of caching an unsuccessful attempt.
            if Some(new_bg) != self.prev_titlebar_bg
                && crate::macos::set_background_color(
                    _app.window(),
                    new_bg[0],
                    new_bg[1],
                    new_bg[2],
                )
            {
                self.prev_titlebar_bg = Some(new_bg);
            }
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn unavailable_native_window_does_not_cache_titlebar_color() {
        i_slint_backend_testing::init_no_event_loop();
        let app = AppWindow::new().unwrap();
        let mut bridge = PlatformBridge::new();

        for dark in [true, false, true] {
            app.global::<crate::Theme>().set_dark_mode(dark);
            bridge.sync(&app);
            assert_eq!(bridge.prev_titlebar_bg, None);
        }
    }
}
