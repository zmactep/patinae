//! Typed layout events and their UI-thread handler.

use super::*;
use crate::app::{
    events::{EventSender, GuiEvent},
    App,
};

#[derive(Debug)]
pub(crate) enum Event {
    ResizeRight(f32),
    ResizeBottom(f32),
    SelectTab(i32),
    ShowPanel(slint::SharedString),
    HidePanel(slint::SharedString),
}

pub(crate) fn setup_callbacks(sender: EventSender, window: &AppWindow) {
    let state = window.global::<LayoutState>();
    let s = sender.clone();
    state.on_resize_right(move |x| s.send(GuiEvent::Layout(Event::ResizeRight(x))));
    let s = sender.clone();
    state.on_resize_bottom(move |x| s.send(GuiEvent::Layout(Event::ResizeBottom(x))));
    let s = sender.clone();
    state.on_select_tab(move |x| s.send(GuiEvent::Layout(Event::SelectTab(x))));
    {
        let sender = sender.clone();
        state.on_show_panel(move |arg0| sender.send(GuiEvent::Layout(Event::ShowPanel(arg0))));
    }
    {
        let sender = sender.clone();
        state.on_hide_panel(move |arg0| sender.send(GuiEvent::Layout(Event::HidePanel(arg0))));
    }
}

pub(in crate::app) fn dispatch(event: Event, app: &mut App, window: &AppWindow) {
    match event {
        Event::ResizeRight(x) => window.global::<LayoutState>().set_right_dock_width(x),
        Event::ResizeBottom(x) => window.global::<LayoutState>().set_bottom_dock_height(x),
        Event::SelectTab(x) => window.global::<LayoutState>().set_bottom_active_tab(x),
        Event::ShowPanel(id) => {
            let a = &mut *app;
            a.kernel.bus.send(AppMessage::ShowPanel(id.to_string()));
        }
        Event::HidePanel(id) => {
            let a = &mut *app;
            a.kernel.bus.send(AppMessage::HidePanel(id.to_string()));
        }
    }
}
