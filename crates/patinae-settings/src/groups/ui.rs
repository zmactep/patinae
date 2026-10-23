//! UI / interface settings (global-only).

use crate::define_settings_group;
use crate::enums::MouseSelectionMode;

define_settings_group! {
    /// User interface and interaction settings.
    group_global UiSettings {
        theme: crate::enums::ThemeMode = crate::enums::ThemeMode::System,
            name = "theme",
            hints = crate::enums::ThemeMode,
            side_effects = [ColorRebuild, SceneInvalidate];
        mouse_selection_mode: MouseSelectionMode = MouseSelectionMode::Residues,
            name = "mouse_selection_mode",
            hints = MouseSelectionMode,
            side_effects = [OrthoDirty];
        mouse_wheel_scale: f32 = 0.5,
            name = "mouse_wheel_scale",
            min = 0.01, max = 10.0;
        antialias: i32 = 1,
            name = "antialias",
            side_effects = [SceneInvalidate];
        // Outline thickness (in pixels) of the screen-space selection / hover
        // highlight. Was historically the dot-indicator radius before the
        // GPU-id highlight pass replaced it.
        selection_width: f32 = 1.0,
            name = "selection_width",
            min = 0.5, max = 20.0,
            side_effects = [SceneInvalidate];
        transparent_panels: bool = false,
            name = "transparent_panels",
            side_effects = [OrthoDirty];
        opaque_background: bool = false,
            name = "opaque_background",
            side_effects = [SceneInvalidate];
        orthoscopic: bool = false,
            name = "orthoscopic",
            side_effects = [SceneInvalidate];
    }
}
