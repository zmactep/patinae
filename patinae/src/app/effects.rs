use slint::ComponentHandle;
// Native operations run after releasing application and queue borrows.

use super::{
    events::{EventSender, GuiEvent},
    AppWindow,
};

pub(super) enum Effect {
    Quit,
    ResetModifiers,
    Visibility(bool),
    PluginSave(super::SaveFileRequest),
    OpenUrl(String),
    OpenFile {
        script: bool,
    },
    SaveFile {
        title: String,
        default_name: String,
        command: String,
    },
    FocusRepl,
    Key {
        text: slint::SharedString,
        modifiers: patinae_scene::Modifiers,
        pressed: bool,
        repeat: bool,
    },
    DismissRepl,
    PickPlugins,
    Clipboard {
        text: String,
        message: String,
    },
}

impl Effect {
    pub(super) fn execute(self, window: &AppWindow, sender: &EventSender) {
        match self {
            Self::ResetModifiers => {
                sender.reset_captured_modifiers();
                super::bridges::repl::release_stale_keyboard_modifiers(window);
            }
            Self::Quit => {
                sender.close();
                super::request_event_loop_quit(slint::quit_event_loop);
            }
            Self::Visibility(visible) => {
                let result = if visible {
                    window.show()
                } else {
                    window.hide()
                };
                if let Err(error) = result {
                    sender.send(GuiEvent::Warning(format!(
                        "Window visibility failed: {error}"
                    )));
                }
            }
            Self::PluginSave(request) => {
                #[cfg(target_os = "macos")]
                let path = super::files::choose_save_file_path(window, &request);
                #[cfg(not(target_os = "macos"))]
                let path = None;
                #[cfg(not(target_os = "macos"))]
                sender.send(GuiEvent::Warning(
                    "Save dialog is unavailable on this platform; use ray with a filename.".into(),
                ));
                sender.send(GuiEvent::SaveResponse { request, path });
            }
            Self::OpenUrl(url) => {
                if let Err(error) = webbrowser::open(&url) {
                    sender.send(GuiEvent::Warning(format!("Could not open link: {error}")));
                }
            }

            Self::Key {
                text,
                modifiers,
                pressed,
                repeat,
            } => {
                use slint::platform::Key;
                use slint::platform::WindowEvent;
                let (control, meta) = if cfg!(target_os = "macos") {
                    (modifiers.logo, modifiers.ctrl)
                } else {
                    (modifiers.ctrl, modifiers.logo)
                };
                for (key, held) in [
                    (Key::Shift, modifiers.shift),
                    (Key::Control, control),
                    (Key::Alt, modifiers.alt),
                    (Key::Meta, meta),
                ] {
                    let text = key.into();
                    window.window().dispatch_event(if held {
                        WindowEvent::KeyPressed { text }
                    } else {
                        WindowEvent::KeyReleased { text }
                    });
                }
                window.window().dispatch_event(if !pressed {
                    WindowEvent::KeyReleased { text }
                } else if repeat {
                    WindowEvent::KeyPressRepeated { text }
                } else {
                    WindowEvent::KeyPressed { text }
                });
            }
            Self::Clipboard { text, message } => {
                match crate::clipboard::set_clipboard_text(&text) {
                    Ok(()) => sender.send(GuiEvent::Notification(message)),
                    Err(error) => sender.send(GuiEvent::Warning(format!(
                        "Could not copy sequence to clipboard: {error}"
                    ))),
                }
            }
            Self::FocusRepl => window.invoke_focus_repl(),
            Self::DismissRepl => window.invoke_dismiss_repl(),
            Self::OpenFile { script } => {
                #[cfg(target_os = "macos")]
                if let Some(path) =
                    crate::macos::open_file_path(if script { "Run Script" } else { "Open" })
                {
                    sender.send(GuiEvent::FileChosen { path, script });
                }
                #[cfg(not(target_os = "macos"))]
                {
                    let _ = script;
                    sender.send(GuiEvent::Warning(
                        "Open dialog is unavailable on this platform; use the REPL.".into(),
                    ));
                }
            }
            Self::SaveFile {
                title,
                default_name,
                command,
            } => {
                #[cfg(target_os = "macos")]
                if let Some(path) = crate::macos::save_file_path_with_default(&title, &default_name)
                {
                    sender.send(GuiEvent::Command(format!(
                        "{command} {}",
                        crate::native_file_actions::quote_command_arg(&path.to_string_lossy())
                    )));
                }
                #[cfg(not(target_os = "macos"))]
                {
                    let _ = default_name;
                    sender.send(GuiEvent::Warning(format!("{title} dialog is unavailable on this platform; use {command} in the REPL.")));
                }
            }
            Self::PickPlugins => {
                #[cfg(target_os = "macos")]
                let result = Ok(crate::macos::open_plugin_paths().unwrap_or_default());
                #[cfg(not(target_os = "macos"))]
                let result = Err("Plugin file picker is unavailable on this platform.".into());
                sender.send(GuiEvent::PluginList(
                    super::bridges::plugin_list::events::Event::Picked(result),
                ));
            }
        }
    }
}
