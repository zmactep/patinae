//! Typed repl events and their UI-thread handler.

use super::*;
use crate::app::{
    events::{EventSender, GuiEvent},
    App,
};

#[derive(Debug)]
pub(crate) enum Event {
    OpenLink(slint::SharedString),
    Submit(slint::SharedString),
    HistoryPrevious,
    HistoryNext,
    ToggleDrawer,
    RequestFocus,
    RequestDismiss,
    RequestKeyboardModifierReset,
    CompletionTextChanged(slint::SharedString),
    CompletionAccept {
        text: slint::SharedString,
        current: String,
        start: usize,
    },
}

pub(crate) fn setup_callbacks(sender: EventSender, window: &AppWindow) {
    let state = window.global::<ReplState>();
    {
        let sender = sender.clone();
        state.on_open_link(move |arg0| sender.send(GuiEvent::Repl(Event::OpenLink(arg0))));
    }
    {
        let sender = sender.clone();
        state.on_submit(move |arg0| sender.send(GuiEvent::Repl(Event::Submit(arg0))));
    }
    {
        let sender = sender.clone();
        state.on_history_previous(move || sender.send(GuiEvent::Repl(Event::HistoryPrevious)));
    }
    {
        let sender = sender.clone();
        state.on_history_next(move || sender.send(GuiEvent::Repl(Event::HistoryNext)));
    }
    {
        let sender = sender.clone();
        state.on_toggle_drawer(move || sender.send(GuiEvent::Repl(Event::ToggleDrawer)));
    }
    {
        let sender = sender.clone();
        state.on_request_focus(move || sender.send(GuiEvent::Repl(Event::RequestFocus)));
    }
    {
        let sender = sender.clone();
        state.on_request_dismiss(move || sender.send(GuiEvent::Repl(Event::RequestDismiss)));
    }
    {
        let sender = sender.clone();
        state.on_request_keyboard_modifier_reset(move || {
            sender.send(GuiEvent::Repl(Event::RequestKeyboardModifierReset))
        });
    }
    {
        let sender = sender.clone();
        state.on_completion_text_changed(move |arg0| {
            sender.send(GuiEvent::Repl(Event::CompletionTextChanged(arg0)))
        });
    }
    {
        let sender = sender.clone();
        let weak = window.as_weak();
        state.on_completion_accept(move |index| {
            if let Some(w) = weak.upgrade() {
                let rs = w.global::<ReplState>();
                if let Some(item) = rs.get_completion_items().row_data(index as usize) {
                    sender.send(GuiEvent::Repl(Event::CompletionAccept {
                        text: item.text,
                        current: rs.get_input_text().to_string(),
                        start: rs.get_completion_start_pos().max(0) as usize,
                    }));
                }
            }
        });
    }
}

pub(in crate::app) fn dispatch(event: Event, app: &mut App, window: &AppWindow) {
    let weak = window.as_weak();
    match event {
        Event::OpenLink(url) => {
            if url.starts_with("https://") || url.starts_with("http://") {
                app.effects
                    .push(crate::app::effects::Effect::OpenUrl(url.to_string()));
            }
        }
        Event::Submit(text) => {
            let cmd = text.to_string();
            if cmd.is_empty() {
                return;
            }
            if let Some(w) = weak.upgrade() {
                let a = &mut *app;
                a.submit_repl_command(cmd, &w);

                let rs = w.global::<ReplState>();
                rs.set_input_text(slint::SharedString::default());
                rs.set_completion_visible(false);
            }
        }
        Event::HistoryPrevious => {
            let a = &mut *app;
            a.kernel.command_line.history_previous();
            if let Some(w) = weak.upgrade() {
                let rs = w.global::<ReplState>();
                rs.set_input_text(a.kernel.command_line.input.clone().into());
                rs.set_completion_visible(false);
                w.invoke_repl_cursor_to_end();
            }
        }
        Event::HistoryNext => {
            let a = &mut *app;
            a.kernel.command_line.history_next();
            if let Some(w) = weak.upgrade() {
                let rs = w.global::<ReplState>();
                rs.set_input_text(a.kernel.command_line.input.clone().into());
                rs.set_completion_visible(false);
                w.invoke_repl_cursor_to_end();
            }
        }
        Event::ToggleDrawer => {
            if let Some(w) = weak.upgrade() {
                let rs = w.global::<ReplState>();
                rs.set_drawer_open(!rs.get_drawer_open());
            }
        }
        Event::RequestFocus => {
            if weak.upgrade().is_some() {
                app.effects.push(crate::app::effects::Effect::FocusRepl);
            }
        }
        Event::RequestDismiss => {
            if weak.upgrade().is_some() {
                app.effects.push(crate::app::effects::Effect::DismissRepl);
            }
        }
        Event::RequestKeyboardModifierReset => {
            app.interaction
                .winit_modifiers
                .set((false, false, false, false));
            if let Some(w) = weak.upgrade() {
                let _ = w;
                app.effects
                    .push(crate::app::effects::Effect::ResetModifiers);
            }
        }
        Event::CompletionTextChanged(text) => {
            if let Some(window) = weak.upgrade() {
                let app = &*app;
                app.repl
                    .update_completions(&app.kernel, &window, text.as_str());
            }
        }
        Event::CompletionAccept {
            text,
            current,
            start: start_pos,
        } => {
            if let Some(w) = weak.upgrade() {
                let rs = w.global::<ReplState>();
                let mut new_input = current[..start_pos.min(current.len())].to_string();
                new_input.push_str(text.as_str());

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
        }
    }
}
