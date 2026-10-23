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
                let failure = GuiEvent::SaveResponse {
                    request: request.clone(),
                    path: None,
                };
                let window = window.clone_strong();
                spawn_dialog(sender, Some(failure), async move {
                    let path = crate::native_file_dialogs::save_file(
                        window.window(),
                        &request.title,
                        &request.default_file_name,
                    )
                    .await
                    .map(|path| path.to_string_lossy().into_owned());
                    Some(GuiEvent::SaveResponse { request, path })
                });
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
                let window = window.clone_strong();
                spawn_dialog(sender, None, async move {
                    crate::native_file_dialogs::open_file(
                        window.window(),
                        if script { "Run Script" } else { "Open" },
                    )
                    .await
                    .map(|path| GuiEvent::FileChosen { path, script })
                });
            }
            Self::SaveFile {
                title,
                default_name,
                command,
            } => {
                let window = window.clone_strong();
                spawn_dialog(sender, None, async move {
                    crate::native_file_dialogs::save_file(window.window(), &title, &default_name)
                        .await
                        .map(|path| {
                            GuiEvent::Command(format!(
                                "{command} {}",
                                crate::native_file_actions::quote_command_arg(
                                    &path.to_string_lossy()
                                )
                            ))
                        })
                });
            }
            Self::PickPlugins => {
                let failure =
                    GuiEvent::PluginList(super::bridges::plugin_list::events::Event::Picked(Err(
                        "Could not start the plugin file picker.".into(),
                    )));
                let window = window.clone_strong();
                spawn_dialog(sender, Some(failure), async move {
                    let result = Ok(crate::native_file_dialogs::pick_plugins(window.window())
                        .await
                        .unwrap_or_default());
                    Some(GuiEvent::PluginList(
                        super::bridges::plugin_list::events::Event::Picked(result),
                    ))
                });
            }
        }
    }
}

/// Returns dialog completion to the ordered inbox without holding app borrows.
pub(super) fn spawn_dialog(
    sender: &EventSender,
    failure: Option<GuiEvent>,
    future: impl std::future::Future<Output = Option<GuiEvent>> + 'static,
) {
    let next = sender.clone();
    if let Err(error) = slint::spawn_local(async move {
        if let Some(event) = future.await {
            next.send(event);
        }
    }) {
        if let Some(event) = failure {
            sender.send(event);
        }
        sender.send(GuiEvent::Warning(format!(
            "Could not start file dialog: {error}"
        )));
    }
}
