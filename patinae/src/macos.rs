//! macOS title-bar integration.
//!
//! Makes the native title bar transparent so the app's theme background
//! color shows through, and updates it dynamically on theme changes.

use objc2::rc::Id;
use std::path::PathBuf;

use objc2_app_kit::{
    NSApplication, NSColor, NSModalResponseOK, NSOpenPanel, NSSavePanel, NSView, NSWindow,
};
use objc2_foundation::{MainThreadMarker, NSArray, NSString};
use patinae_framework::topics::SaveFileRequest;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use slint::winit_030::winit::platform::macos::WindowAttributesExtMacOS;
use slint::winit_030::winit::window::WindowAttributes;
use slint::winit_030::WinitWindowAccessor;

/// Returns `true` if the system appearance is dark mode.
pub fn is_system_dark_mode() -> bool {
    let mtm = MainThreadMarker::new().expect("must be called from main thread");
    let app = NSApplication::sharedApplication(mtm);
    let appearance = app.effectiveAppearance();
    // SAFETY: effectiveAppearance returns a live NSAppearance for the shared
    // application, and this function is constrained to the main thread above.
    let name = unsafe { appearance.name() };
    name.to_string().contains("Dark")
}

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

/// Opens a native macOS save-file picker.
pub fn save_file_path(_slint_window: &slint::Window, request: &SaveFileRequest) -> Option<String> {
    save_file_path_with_default(&request.title, &request.default_file_name)
        .map(|path| path.to_string_lossy().into_owned())
}

/// Opens a native macOS open-file picker.
pub fn open_file_path(title: &str) -> Option<PathBuf> {
    open_file_paths(title, false, &[])?.into_iter().next()
}

/// Selects multiple plugin libraries without opening or loading them.
pub fn open_plugin_paths() -> Option<Vec<PathBuf>> {
    open_file_paths("Add Plugins", true, &["dylib"])
}

fn open_file_paths(title: &str, multiple: bool, extensions: &[&str]) -> Option<Vec<PathBuf>> {
    let mtm = MainThreadMarker::new()?;

    let panel = unsafe {
        // SAFETY: `NSOpenPanel` is main-thread-only and `mtm` proves this
        // function is running on the AppKit main thread.
        NSOpenPanel::openPanel(mtm)
    };

    let title = NSString::from_str(title);
    unsafe {
        // SAFETY: The panel and title are live AppKit/Foundation objects on
        // the main thread for the duration of these Objective-C calls.
        panel.setTitle(Some(&title));
        panel.setCanChooseFiles(true);
        panel.setCanChooseDirectories(false);
        panel.setAllowsMultipleSelection(multiple);
        if !extensions.is_empty() {
            let types = NSArray::from_vec(
                extensions
                    .iter()
                    .map(|ext| NSString::from_str(ext))
                    .collect(),
            );
            // The project's AppKit 0.2 bindings support extension-based filters
            // without an additional UniformTypeIdentifiers dependency.
            #[expect(
                deprecated,
                reason = "Use the extension filter available in existing AppKit bindings"
            )]
            panel.setAllowedFileTypes(Some(&types));
            panel.setAllowsOtherFileTypes(false);
        }
    }

    let response = unsafe {
        // SAFETY: Running the modal panel is an AppKit main-thread operation;
        // `mtm` above guarantees that precondition for this function.
        panel.runModal()
    };
    if response != NSModalResponseOK {
        return None;
    }

    let urls = unsafe {
        // SAFETY: The modal completed with OK; AppKit returns a retained array
        // of selected file URLs, whose ownership is tracked by objc2.
        panel.URLs()
    };
    urls.iter().map(path_from_url).collect()
}

/// Opens a native macOS save-file picker with a default filename.
pub fn save_file_path_with_default(title: &str, default_file_name: &str) -> Option<PathBuf> {
    let mtm = MainThreadMarker::new()?;

    let panel = unsafe {
        // SAFETY: `NSSavePanel` is main-thread-only and `mtm` proves this
        // function is running on the AppKit main thread.
        NSSavePanel::savePanel(mtm)
    };

    let title = NSString::from_str(title);
    let default_file_name = NSString::from_str(default_file_name);

    unsafe {
        // SAFETY: The panel and strings are live AppKit/Foundation objects on
        // the main thread for the duration of these Objective-C calls.
        panel.setTitle(Some(&title));
        panel.setNameFieldStringValue(&default_file_name);
        panel.setCanCreateDirectories(true);
    }

    let response = unsafe {
        // SAFETY: Running the modal panel is an AppKit main-thread operation;
        // `mtm` above guarantees that precondition for this function.
        panel.runModal()
    };
    if response != NSModalResponseOK {
        return None;
    }

    let url = unsafe {
        // SAFETY: The modal completed with OK, and AppKit returns either a
        // retained URL or `None`; `objc2` models that ownership.
        panel.URL()?
    };
    path_from_url(&url)
}

fn path_from_url(url: &objc2_foundation::NSURL) -> Option<PathBuf> {
    let path = unsafe {
        // SAFETY: The selected file URL is still retained here. `path` returns
        // an owned NSString when the URL can be represented as a file path.
        url.path()?
    };
    Some(PathBuf::from(path.to_string()))
}
