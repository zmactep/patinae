//! Asynchronous native file pickers shared by all desktop platforms.

use slint::winit_030::WinitWindowAccessor;
use std::path::PathBuf;
use std::sync::Arc;

async fn dialog(
    window: &slint::Window,
    title: &str,
) -> (
    rfd::AsyncFileDialog,
    Option<Arc<slint::winit_030::winit::window::Window>>,
) {
    let dialog = rfd::AsyncFileDialog::new().set_title(title);
    // rfd stores raw handles; retain the native window until the picker completes.
    let parent = if window.has_winit_window() {
        window.winit_window().await.ok()
    } else {
        None
    };
    let dialog = match parent.as_deref() {
        Some(parent) => dialog.set_parent(parent),
        None => dialog,
    };
    (dialog, parent)
}

/// Selects one file without restricting formats registered by plugins.
pub(crate) async fn open_file(window: &slint::Window, title: &str) -> Option<PathBuf> {
    let (dialog, parent) = dialog(window, title).await;
    let file = dialog.pick_file().await;
    drop(parent);
    file.map(|file| file.path().to_path_buf())
}

/// Selects a destination with the caller's proposed filename.
pub(crate) async fn save_file(
    window: &slint::Window,
    title: &str,
    default_name: &str,
) -> Option<PathBuf> {
    let (dialog, parent) = dialog(window, title).await;
    let file = dialog
        .set_file_name(default_name)
        .set_can_create_directories(true)
        .save_file()
        .await;
    drop(parent);
    file.map(|file| file.path().to_path_buf())
}

/// Selects multiple native plugin libraries for the current platform.
pub(crate) async fn pick_plugins(window: &slint::Window) -> Option<Vec<PathBuf>> {
    let (dialog, parent) = dialog(window, "Add Plugins").await;
    let files = dialog
        .add_filter("Plugin libraries", &[std::env::consts::DLL_EXTENSION])
        .pick_files()
        .await;
    drop(parent);
    files.map(|files| {
        files
            .into_iter()
            .map(|file| file.path().to_path_buf())
            .collect()
    })
}
