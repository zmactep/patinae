//! Platform bridge: OS-specific appearance sync.

#[cfg(target_os = "macos")]
use slint::ComponentHandle;

use crate::AppWindow;

/// Syncs platform-specific appearance (e.g. macOS title bar color).
pub struct PlatformBridge {
    #[cfg(target_os = "macos")]
    prev_titlebar_bg: [f32; 3],
}

impl PlatformBridge {
    pub fn new() -> Self {
        Self {
            #[cfg(target_os = "macos")]
            prev_titlebar_bg: [0.0; 3],
        }
    }

    /// Sync platform appearance with Slint theme. Call once per frame.
    pub fn sync(&mut self, _app: &AppWindow) {
        #[cfg(target_os = "macos")]
        {
            let bg = _app
                .global::<crate::Theme>()
                .get_surface_base()
                .to_argb_f32();
            let new_bg = [bg.red, bg.green, bg.blue];
            if new_bg != self.prev_titlebar_bg {
                crate::macos::set_background_color(_app.window(), new_bg[0], new_bg[1], new_bg[2]);
                self.prev_titlebar_bg = new_bg;
            }
        }
    }
}
