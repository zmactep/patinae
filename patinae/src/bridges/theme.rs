//! Theme bridge: settings → kernel palette → Slint Theme global.

use std::rc::Rc;

use slint::{ComponentHandle, ModelRc, VecModel};

use patinae_color::Color;
use patinae_framework::kernel::AppKernel;
use patinae_settings::{SettingEnum as _, ThemeMode};

use crate::{AppWindow, NamedColor, Theme};

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
pub fn sync_theme(
    kernel: &mut AppKernel,
    theme_global: &Theme,
    app_window: &AppWindow,
    prev_theme: &mut ThemeMode,
) -> bool {
    use patinae_color::ThemedPalette;

    let mode = kernel.session.settings.ui.theme;
    let changed = mode != *prev_theme;
    *prev_theme = mode;

    // UI Foundations own the desktop viewport background; scientific colors
    // still come from the molecular palette.
    theme_global.set_dark_mode(mode == ThemeMode::Dark);
    let bg = theme_global.get_surface_viewport();
    let mut palette = ThemedPalette::by_name(mode.name());
    palette.viewport_bg = Color::from_rgb8(bg.red(), bg.green(), bg.blue());
    kernel.session.palette = palette;

    let bg = kernel.session.palette.viewport_bg;
    kernel.sync_clear_color([bg.r, bg.g, bg.b]);

    app_window.invoke_sync_widget_theme();
    sync_window_theme(app_window.window(), mode);

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

fn sync_window_theme(window: &slint::Window, mode: ThemeMode) {
    use slint::winit_030::winit::window::Theme as WinitTheme;
    use slint::winit_030::WinitWindowAccessor;

    let preferred = match mode {
        ThemeMode::Dark => WinitTheme::Dark,
        ThemeMode::Light => WinitTheme::Light,
    };

    let _ = window.with_winit_window(|winit_window| {
        if winit_window.theme() != Some(preferred) {
            winit_window.set_theme(Some(preferred));
        }
    });
}
