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

    /// Recomputes suggestions after input edits or newly attached plugin commands.
    pub(crate) fn update_completions(&self, kernel: &AppKernel, window: &AppWindow, input: &str) {
        // Build completion context
        let registry = kernel.executor.registry();
        let command_names: Vec<&str> = registry.names().collect();
        let setting_names = patinae_settings::setting_names();
        let setting_name_refs: Vec<&str> = setting_names.to_vec();
        // Only palette names — SCHEME_NAMES are added by the engine for ArgHint::Color
        let color_names: Vec<String> = kernel
            .session
            .named_palette
            .names()
            .into_iter()
            .map(|s| s.to_string())
            .collect();
        let object_names: Vec<String> = kernel
            .session
            .registry
            .names()
            .map(|s| s.to_string())
            .collect();
        let selection_names = kernel.session.selections.names();
        let dynamic_settings = kernel.executor.dynamic_settings();

        let ctx = completion::CompletionContext {
            command_names: &command_names,
            registry,
            setting_names: &setting_name_refs,
            color_names: &color_names,
            object_names: &object_names,
            selection_names: &selection_names,
            dynamic_settings: Some(dynamic_settings),
        };

        let result = completion::generate_completions(input, input.len(), &ctx);

        if result.suggestions.is_empty() {
            self.completion_model.set_vec(Vec::new());
            window.global::<ReplState>().set_completion_visible(false);
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

        self.completion_model.set_vec(items);

        let rs = window.global::<ReplState>();
        rs.set_completion_start_pos(result.start_pos as i32);
        rs.set_completion_selected(0);
        rs.set_completion_visible(!input.is_empty());
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

pub(crate) mod events;
pub(crate) use events::setup_callbacks;

pub(in crate::app) fn release_stale_keyboard_modifiers(window: &AppWindow) {
    for key in keyboard_modifier_release_keys() {
        if let Err(err) = window
            .window()
            .dispatch_event_with_result(WindowEvent::KeyReleased {
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
