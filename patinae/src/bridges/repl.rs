use std::cell::RefCell;
use std::rc::Rc;

use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, Model, ModelRc, VecModel};

use patinae_framework::completion::{self, CompletionSource};
use patinae_framework::kernel::AppKernel;
use patinae_framework::model::output::OutputKind;

mod markdown;

use crate::{AppWindow, CompletionItem, OutputItem, ReplState};

// ---------------------------------------------------------------------------
// ReplBridge
// ---------------------------------------------------------------------------

pub struct ReplBridge {
    output_model: Rc<VecModel<OutputItem>>,
    pub(crate) completion_model: Rc<VecModel<CompletionItem>>,
    last_output_generation: u64,
}

impl ReplBridge {
    pub fn new() -> Self {
        Self {
            output_model: Rc::new(VecModel::default()),
            completion_model: Rc::new(VecModel::default()),
            last_output_generation: 0,
        }
    }

    /// Attach the VecModel to the Slint global (call once after window creation).
    pub fn attach(&self, window: &AppWindow) {
        let rs = window.global::<ReplState>();
        rs.set_output_items(ModelRc::from(self.output_model.clone()));
        rs.set_completion_items(ModelRc::from(self.completion_model.clone()));
    }

    /// Sync kernel output buffer to the Slint model. Called each frame.
    pub fn sync(&mut self, kernel: &AppKernel, window: &AppWindow) {
        let output_generation = kernel.output.generation();
        let output_changed = output_generation != self.last_output_generation;
        let rs = window.global::<ReplState>();

        if output_changed {
            let items = build_output_items(kernel.output.buffer.iter());
            self.output_model.set_vec(items);
            self.last_output_generation = output_generation;

            // Bump generation to trigger scroll-to-bottom
            rs.set_output_generation(rs.get_output_generation().wrapping_add(1));
        }
    }
}

/// Show the icon on "command" lines always, and on the first line of a
/// consecutive run of the same non-command kind.
fn should_show_icon(kind: OutputKind, prev_kind: Option<OutputKind>) -> bool {
    if kind == OutputKind::Command || kind == OutputKind::Timing {
        return true;
    }
    match prev_kind {
        None => true,
        Some(prev) => prev != kind,
    }
}

fn to_output_item(
    msg: &patinae_framework::model::output::OutputMessage,
    show_icon: bool,
) -> OutputItem {
    OutputItem {
        text: slint::StyledText::default(),
        plain_text: msg.text.clone().into(),
        markdown: false,
        heading_level: 0,
        code_block: false,
        kind: match msg.kind {
            OutputKind::Normal => "normal",
            OutputKind::Info => "info",
            OutputKind::Warning => "warning",
            OutputKind::Error => "error",
            OutputKind::Command => "command",
            OutputKind::Timing => "timing",
        }
        .into(),
        show_icon,
    }
}

/// Build a complete list of OutputItems with correct show_icon flags.
fn build_output_items<'a>(
    iter: impl Iterator<Item = &'a patinae_framework::model::output::OutputMessage>,
) -> Vec<OutputItem> {
    let mut items = Vec::new();
    let mut prev_kind: Option<OutputKind> = None;
    for msg in iter {
        let show_icon = should_show_icon(msg.kind, prev_kind);
        if msg.format == patinae_cmd::OutputFormat::Markdown {
            for (index, block) in markdown::render(&msg.text).into_iter().enumerate() {
                let mut item = to_output_item(msg, show_icon && index == 0);
                item.text = block.text;
                item.plain_text = block.plain_text.into();
                item.markdown = true;
                item.heading_level = block.heading_level;
                item.code_block = block.code_block;
                items.push(item);
            }
        } else {
            items.push(to_output_item(msg, show_icon));
        }
        prev_kind = Some(msg.kind);
    }
    items
}

// ---------------------------------------------------------------------------
// Callbacks
// ---------------------------------------------------------------------------

pub fn setup_callbacks(app: Rc<RefCell<crate::app::App>>, window: &AppWindow) {
    let rs = window.global::<ReplState>();

    rs.on_open_link(|url| {
        if url.starts_with("https://") || url.starts_with("http://") {
            if let Err(error) = webbrowser::open(url.as_str()) {
                log::warn!("Failed to open REPL link: {error}");
            }
        }
    });

    // --- Submit command ---
    {
        let app = app.clone();
        let weak = window.as_weak();
        rs.on_submit(move |text| {
            let cmd = text.to_string();
            if cmd.is_empty() {
                return;
            }
            if let Some(w) = weak.upgrade() {
                let mut a = app.borrow_mut();
                a.submit_repl_command(cmd, &w);

                let rs = w.global::<ReplState>();
                rs.set_input_text(slint::SharedString::default());
                rs.set_completion_visible(false);
            }
        });
    }

    // --- History previous (up arrow) ---
    {
        let app = app.clone();
        let weak = window.as_weak();
        rs.on_history_previous(move || {
            let mut a = app.borrow_mut();
            a.kernel.command_line.history_previous();
            if let Some(w) = weak.upgrade() {
                let rs = w.global::<ReplState>();
                rs.set_input_text(a.kernel.command_line.input.clone().into());
                rs.set_completion_visible(false);
                w.invoke_repl_cursor_to_end();
            }
        });
    }

    // --- History next (down arrow) ---
    {
        let app = app.clone();
        let weak = window.as_weak();
        rs.on_history_next(move || {
            let mut a = app.borrow_mut();
            a.kernel.command_line.history_next();
            if let Some(w) = weak.upgrade() {
                let rs = w.global::<ReplState>();
                rs.set_input_text(a.kernel.command_line.input.clone().into());
                rs.set_completion_visible(false);
                w.invoke_repl_cursor_to_end();
            }
        });
    }

    // --- Toggle drawer ---
    {
        let weak = window.as_weak();
        rs.on_toggle_drawer(move || {
            if let Some(w) = weak.upgrade() {
                let rs = w.global::<ReplState>();
                rs.set_drawer_open(!rs.get_drawer_open());
            }
        });
    }

    // --- Request focus (from sidebar button or keyboard shortcut) ---
    {
        let weak = window.as_weak();
        rs.on_request_focus(move || {
            if let Some(w) = weak.upgrade() {
                w.invoke_focus_repl();
            }
        });
    }

    // --- Request dismiss (from viewport click or other outside interaction) ---
    {
        let weak = window.as_weak();
        rs.on_request_dismiss(move || {
            if let Some(w) = weak.upgrade() {
                w.invoke_dismiss_repl();
            }
        });
    }

    // --- Reset stale modifier state before editing text ---
    {
        let winit_modifiers = app.borrow().winit_modifiers.clone();
        let weak = window.as_weak();
        rs.on_request_keyboard_modifier_reset(move || {
            winit_modifiers.set((false, false, false, false));
            if let Some(w) = weak.upgrade() {
                release_stale_keyboard_modifiers(&w);
            }
        });
    }

    // --- Completion: text changed ---
    {
        let app = app.clone();
        let weak = window.as_weak();
        let completion_model = app.borrow().repl.completion_model.clone();
        rs.on_completion_text_changed(move |text| {
            let input = text.to_string();

            let a = app.borrow();

            // Build completion context
            let registry = a.kernel.executor.registry();
            let command_names: Vec<&str> = registry.names().collect();
            let setting_names = patinae_settings::setting_names();
            let setting_name_refs: Vec<&str> = setting_names.to_vec();
            // Only palette names — SCHEME_NAMES are added by the engine for ArgHint::Color
            let color_names: Vec<String> = a
                .kernel
                .session
                .named_palette
                .names()
                .into_iter()
                .map(|s| s.to_string())
                .collect();
            let object_names: Vec<String> = a
                .kernel
                .session
                .registry
                .names()
                .map(|s| s.to_string())
                .collect();
            let selection_names = a.kernel.session.selections.names();
            let dynamic_settings = a.kernel.executor.dynamic_settings();

            let ctx = completion::CompletionContext {
                command_names: &command_names,
                registry,
                setting_names: &setting_name_refs,
                color_names: &color_names,
                object_names: &object_names,
                selection_names: &selection_names,
                dynamic_settings: Some(dynamic_settings),
            };

            let result = completion::generate_completions(&input, input.len(), &ctx);

            if result.suggestions.is_empty() {
                completion_model.set_vec(Vec::new());
                if let Some(w) = weak.upgrade() {
                    w.global::<ReplState>().set_completion_visible(false);
                }
                return;
            }

            // Convert to Slint CompletionItem
            let items: Vec<CompletionItem> = result
                .suggestions
                .iter()
                .map(|item| CompletionItem {
                    text: item.text.clone().into(),
                    description: item.description.clone().into(),
                    source: match item.source {
                        CompletionSource::Plugin => "plugin",
                        _ => "",
                    }
                    .into(),
                })
                .collect();

            completion_model.set_vec(items);

            if let Some(w) = weak.upgrade() {
                let rs = w.global::<ReplState>();
                rs.set_completion_start_pos(result.start_pos as i32);
                rs.set_completion_selected(0);
                rs.set_completion_visible(true);
            }
        });
    }

    // --- Completion: accept ---
    {
        let app = app.clone();
        let weak = window.as_weak();
        let completion_model = app.borrow().repl.completion_model.clone();
        rs.on_completion_accept(move |index| {
            let idx = index as usize;
            let item = match completion_model.row_data(idx) {
                Some(item) => item,
                None => return,
            };

            if let Some(w) = weak.upgrade() {
                let rs = w.global::<ReplState>();
                let start_pos = rs.get_completion_start_pos() as usize;
                let current = rs.get_input_text().to_string();

                let mut new_input = current[..start_pos.min(current.len())].to_string();
                new_input.push_str(item.text.as_str());

                // Add trailing space for command-name completions (start_pos == 0)
                if start_pos == 0 {
                    new_input.push(' ');
                }

                let is_command_completion = start_pos == 0;

                let new_input: slint::SharedString = new_input.into();
                rs.set_input_text(new_input.clone());
                rs.set_completion_visible(false);
                w.invoke_repl_cursor_to_end();

                // Re-trigger only for command-name completions so argument
                // suggestions appear immediately (e.g., "load " → path hints).
                // For argument completions the value is final — don't re-trigger
                // or the popup reappears and blocks Enter/Return.
                if is_command_completion {
                    rs.invoke_completion_text_changed(new_input);
                }
            }
        });
    }
}

fn release_stale_keyboard_modifiers(window: &AppWindow) {
    for key in keyboard_modifier_release_keys() {
        if let Err(err) = window
            .window()
            .try_dispatch_event(WindowEvent::KeyReleased {
                text: (*key).into(),
            })
        {
            log::warn!("Failed to reset REPL keyboard modifier: {err}");
        }
    }
}

fn keyboard_modifier_release_keys() -> &'static [Key] {
    &[
        Key::Control,
        Key::ControlR,
        Key::Meta,
        Key::MetaR,
        Key::Alt,
        Key::AltGr,
        Key::Shift,
        Key::ShiftR,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_explicit_markdown_is_formatted() {
        let messages = [
            patinae_cmd::OutputMessage::info("**literal**").into(),
            patinae_cmd::OutputMessage::markdown("# Heading\n\n**formatted**").into(),
        ];
        let items = build_output_items(messages.iter());
        assert_eq!(items.len(), 3);
        assert!(!items[0].markdown);
        assert_eq!(items[0].plain_text, "**literal**");
        assert!(items[1].markdown);
        assert_eq!(items[1].heading_level, 1);
        assert_eq!(
            items[2].text,
            slint::StyledText::from_markdown("**formatted**").unwrap()
        );
        assert!(!items[2].show_icon);
    }

    #[test]
    fn modifier_reset_releases_left_and_right_modifier_keys() {
        let keys = keyboard_modifier_release_keys();

        assert_eq!(keys.len(), 8);
        for key in [
            Key::Control,
            Key::ControlR,
            Key::Meta,
            Key::MetaR,
            Key::Alt,
            Key::AltGr,
            Key::Shift,
            Key::ShiftR,
        ] {
            assert!(keys.contains(&key));
        }
    }
}
