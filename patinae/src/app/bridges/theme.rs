//! Theme bridge: settings → kernel palette → Slint Theme global.

use std::{path::PathBuf, rc::Rc};

use slint::{ComponentHandle, ModelRc, VecModel};

use patinae_color::Color;
use patinae_framework::kernel::AppKernel;
use patinae_settings::{SettingEnum as _, ThemeMode};

use crate::{AppWindow, NamedColor, Theme};

/// Tracks the selected preference independently of the resolved native appearance.
#[derive(Debug, Default)]
pub(crate) struct ThemeBridge {
    config_path: Option<PathBuf>,
    observed: ThemeMode,
    native_mode: Option<ThemeMode>,
    system_dark: Option<bool>,
    system_dirty: bool,
    previous_dark: Option<bool>,
}

impl ThemeBridge {
    pub(crate) fn configure(&mut self, path: PathBuf, selected: ThemeMode) {
        self.config_path = Some(path);
        self.observed = selected;
    }

    /// Attempts one save per preference change, including after a failed save.
    pub(crate) fn persist(
        &mut self,
        selected: ThemeMode,
    ) -> Result<(), patinae_settings::config::ConfigError> {
        if self.observed == selected {
            return Ok(());
        }
        self.observed = selected;
        if let Some(path) = &self.config_path {
            patinae_settings::config::save_theme(path, selected)?;
        }
        Ok(())
    }

    pub(crate) fn system_changed(&mut self) {
        // Re-read the native window at dispatch time; queued notifications can be stale.
        self.system_dirty = true;
    }

    fn resolve(&mut self, window: &slint::Window, selected: ThemeMode) -> bool {
        use slint::winit_030::{winit::window::Theme as WinitTheme, WinitWindowAccessor};

        let mode_changed = self.native_mode != Some(selected);
        if mode_changed || self.system_dirty {
            let resolved = window.with_winit_window(|native| {
                if mode_changed {
                    native.set_theme(match selected {
                        ThemeMode::Dark => Some(WinitTheme::Dark),
                        ThemeMode::Light => Some(WinitTheme::Light),
                        ThemeMode::System => None,
                    });
                }
                native.theme().map(|theme| theme == WinitTheme::Dark)
            });
            if let Some(system_dark) = resolved {
                self.native_mode = Some(selected);
                self.system_dark = system_dark;
                self.system_dirty = false;
            }
        }
        resolved_dark(selected, self.system_dark)
    }
}

fn resolved_dark(selected: ThemeMode, system_dark: Option<bool>) -> bool {
    match selected {
        ThemeMode::Dark => true,
        ThemeMode::Light => false,
        // Preserve the historical dark fallback on backends without theme detection.
        ThemeMode::System => system_dark.unwrap_or(true),
    }
}

fn to_slint(c: Color) -> slint::Color {
    slint::Color::from_rgb_f32(c.r, c.g, c.b)
}

fn named(name: &str, color: Color) -> NamedColor {
    NamedColor {
        name: name.into(),
        swatch: to_slint(color),
    }
}

/// Push the fixed solid-color swatch rows once (they are theme-independent).
pub fn push_solid_swatches(theme_global: &Theme) {
    use patinae_color::*;

    // Row 1 — bold spectrum
    let row1 = Rc::new(VecModel::from(vec![
        named("red", RED),
        named("orange", ORANGE),
        named("yellow", YELLOW),
        named("green", GREEN),
        named("cyan", CYAN),
        named("blue", BLUE),
        named("teal", TEAL),
        named("marine", MARINE),
        named("emerald", EMERALD),
        named("white", WHITE),
    ]));

    // Row 2 — soft / pastel
    let row2 = Rc::new(VecModel::from(vec![
        named("lightorange", LIGHT_ORANGE),
        named("wheat", WHEAT),
        named("lime", LIME),
        named("palegreen", PALE_GREEN),
        named("palecyan", PALE_CYAN),
        named("lightblue", LIGHT_BLUE),
        named("aquamarine", AQUAMARINE),
        named("paleyellow", PALE_YELLOW),
        named("gray80", GRAY80),
        named("slate", SLATE),
    ]));

    // Row 3 — deep / rich
    let row3 = Rc::new(VecModel::from(vec![
        named("firebrick", FIREBRICK),
        named("chocolate", CHOCOLATE),
        named("olive", OLIVE),
        named("forest", FOREST),
        named("teal", TEAL),
        named("deepteal", DEEP_TEAL),
        named("deepolive", DEEP_OLIVE),
        named("burntorange", BURNT_ORANGE),
        named("terracotta", TERRACOTTA),
        named("gray50", GRAY50),
    ]));

    theme_global.set_solid_row1(ModelRc::from(row1));
    theme_global.set_solid_row2(ModelRc::from(row2));
    theme_global.set_solid_row3(ModelRc::from(row3));
}

/// Sync theme from settings (source of truth) to kernel palette and Slint UI.
///
/// Returns `true` if the theme changed since the last call, so the caller can
/// invalidate GPU representations and the scene model.
pub(crate) fn sync_theme(
    kernel: &mut AppKernel,
    theme_global: &Theme,
    app_window: &AppWindow,
    bridge: &mut ThemeBridge,
) -> bool {
    use patinae_color::ThemedPalette;

    let mode = kernel.session.settings.ui.theme;
    let dark = bridge.resolve(app_window.window(), mode);
    let changed = bridge.previous_dark != Some(dark);
    bridge.previous_dark = Some(dark);

    // UI Foundations own the desktop viewport background; scientific colors
    // still come from the molecular palette.
    theme_global.set_mode(mode.name().into());
    theme_global.set_dark_mode(dark);
    let bg = theme_global.get_surface_viewport();
    let mut palette = if dark {
        ThemedPalette::dark()
    } else {
        ThemedPalette::light()
    };
    palette.viewport_bg = Color::from_rgb8(bg.red(), bg.green(), bg.blue());
    kernel.session.palette = palette;

    let bg = kernel.session.palette.viewport_bg;
    kernel.sync_clear_color([bg.r, bg.g, bg.b]);

    app_window.invoke_sync_widget_theme();

    // Push chain palette colors for scheme card icons
    let ch = &kernel.session.palette.chains;
    theme_global.set_chain_a(to_slint(ch.get("A")));
    theme_global.set_chain_b(to_slint(ch.get("B")));
    theme_global.set_chain_c(to_slint(ch.get("C")));
    theme_global.set_chain_d(to_slint(ch.get("D")));
    theme_global.set_chain_e(to_slint(ch.get("E")));
    theme_global.set_chain_f(to_slint(ch.get("F")));

    // Push secondary structure colors
    let ss = &kernel.session.palette.ss;
    theme_global.set_ss_helix(to_slint(ss.helix));
    theme_global.set_ss_loop(to_slint(ss.coil));
    theme_global.set_ss_sheet(to_slint(ss.sheet));

    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_resolution_changes_palette_without_changing_preference() {
        i_slint_backend_testing::init_no_event_loop();
        let window = AppWindow::new().unwrap();
        let theme = window.global::<Theme>();
        let mut kernel = AppKernel::new();
        let mut bridge = ThemeBridge {
            native_mode: Some(ThemeMode::System),
            system_dark: Some(false),
            ..Default::default()
        };
        assert!(sync_theme(&mut kernel, &theme, &window, &mut bridge));
        assert!(!theme.get_dark_mode());
        assert_eq!(theme.get_mode(), "system");
        let light_chain = kernel.session.palette.chains.get("A");
        let light_bg = kernel.session.palette.viewport_bg;
        bridge.system_dark = Some(true);
        assert!(sync_theme(&mut kernel, &theme, &window, &mut bridge));
        assert!(theme.get_dark_mode());
        assert_ne!(kernel.session.palette.chains.get("A"), light_chain);
        assert_ne!(kernel.session.palette.viewport_bg, light_bg);
        assert_eq!(kernel.session.settings.ui.theme, ThemeMode::System);
        assert_eq!(theme.get_mode(), "system");
        assert!(!sync_theme(&mut kernel, &theme, &window, &mut bridge));
        for system in [None, Some(false), Some(true)] {
            assert!(resolved_dark(ThemeMode::Dark, system));
            assert!(!resolved_dark(ThemeMode::Light, system));
        }
        assert!(resolved_dark(ThemeMode::System, None));
    }
}
