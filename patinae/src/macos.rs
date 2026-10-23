//! macOS title-bar integration.
//!
//! Makes the native title bar transparent so the app's theme background
//! color shows through, and updates it dynamically on theme changes.

use objc2::rc::Id;

use objc2_app_kit::{NSColor, NSView, NSWindow};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use slint::winit_030::winit::platform::macos::WindowAttributesExtMacOS;
use slint::winit_030::winit::window::WindowAttributes;
use slint::winit_030::WinitWindowAccessor;

/// Called once at window-creation time via the `BackendSelector` hook.
/// Enables the transparent title bar and hides the title text.
pub fn configure_titlebar_attributes(attrs: WindowAttributes) -> WindowAttributes {
    attrs
        .with_titlebar_transparent(true)
        .with_title_hidden(true)
}

/// Updates the `NSWindow` background color to match the current theme.
///
/// Retrieves the native window handle via the Slint winit accessor,
/// then calls `setBackgroundColor:` with an sRGB `NSColor`.
pub fn set_background_color(slint_window: &slint::Window, r: f32, g: f32, b: f32) {
    slint_window.with_winit_window(|winit_window| {
        let handle = winit_window.window_handle().ok()?;
        let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
            return None;
        };

        // SAFETY: The AppKit raw window handle comes from the live winit
        // window for this callback, so retaining its NSView pointer is valid.
        let ns_view: Id<NSView> = unsafe { Id::retain(appkit.ns_view.as_ptr().cast()) }?;
        let ns_window: Id<NSWindow> = ns_view.window()?;

        // SAFETY: NSColor construction is an Objective-C class call with
        // primitive sRGB components and returns an owned color object.
        let color = unsafe {
            NSColor::colorWithSRGBRed_green_blue_alpha(r as f64, g as f64, b as f64, 1.0)
        };
        ns_window.setBackgroundColor(Some(&color));

        Some(())
    });
}
