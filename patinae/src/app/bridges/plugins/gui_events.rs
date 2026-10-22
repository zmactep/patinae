//! Typed plugins events and their UI-thread handler.

use super::*;
use crate::app::{
    events::{EventSender, GuiEvent},
    App,
};

#[derive(Debug)]
pub(crate) enum Event {
    TogglePanel(slint::SharedString),
    ActivatePanel(slint::SharedString),
    DeactivatePlacement(slint::SharedString),
    Control(
        slint::SharedString,
        slint::SharedString,
        slint::SharedString,
        slint::SharedString,
        f32,
        bool,
    ),
}

pub(crate) fn setup_callbacks(sender: EventSender, window: &AppWindow) {
    let state = window.global::<PluginState>();
    {
        let sender = sender.clone();
        state.on_toggle_panel(move |arg0| sender.send(GuiEvent::Plugins(Event::TogglePanel(arg0))));
    }
    {
        let sender = sender.clone();
        state.on_activate_panel(move |arg0| {
            sender.send(GuiEvent::Plugins(Event::ActivatePanel(arg0)))
        });
    }
    {
        let sender = sender.clone();
        state.on_deactivate_placement(move |arg0| {
            sender.send(GuiEvent::Plugins(Event::DeactivatePlacement(arg0)))
        });
    }
    {
        let sender = sender.clone();
        state.on_control_event(move |arg0, arg1, arg2, arg3, arg4, arg5| {
            sender.send(GuiEvent::Plugins(Event::Control(
                arg0, arg1, arg2, arg3, arg4, arg5,
            )))
        });
    }
}

pub(in crate::app) fn dispatch(event: Event, app: &mut App, window: &AppWindow) {
    let weak = window.as_weak();
    match event {
        Event::TogglePanel(id) => {
            if let Some(window) = weak.upgrade() {
                let app = &mut *app;
                if app.plugin_runtime.plugins.toggle_panel(id.as_str()) {
                    app.sync_plugins(&window);
                    window.window().request_redraw();
                }
            }
        }
        Event::ActivatePanel(id) => {
            if let Some(window) = weak.upgrade() {
                let app = &mut *app;
                if app.plugin_runtime.plugins.activate_panel(id.as_str()) {
                    app.sync_plugins(&window);
                    window.window().request_redraw();
                }
            }
        }
        Event::DeactivatePlacement(placement) => {
            let placement = match placement.as_str() {
                "right" => Some(PanelPlacement::Right),
                "bottom" => Some(PanelPlacement::Bottom),
                _ => None,
            };
            if let (Some(placement), Some(window)) = (placement, weak.upgrade()) {
                let app = &mut *app;
                if app.plugin_runtime.plugins.deactivate_placement(placement) {
                    app.sync_plugins(&window);
                    window.window().request_redraw();
                }
            }
        }
        Event::Control(panel_id, control_id, kind, text, number, boolean) => {
            let Some((kind, value)) =
                panel_event_payload(kind.as_str(), text.as_str(), number, boolean)
            else {
                log::warn!("Unknown plugin panel event kind '{}'", kind);
                return;
            };

            app.plugin_runtime.plugins.queue_panel_event(PanelEvent {
                panel_id: panel_id.to_string(),
                control_id: control_id.to_string(),
                kind,
                value,
            });
            if let Some(window) = weak.upgrade() {
                window.window().request_redraw();
            }
        }
    }
}
