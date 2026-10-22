//! Plugin configuration actions and native-picker completion.
use super::*;
use crate::app::{
    effects::Effect,
    events::{EventSender, GuiEvent},
    App,
};
use slint::Model;

#[derive(Debug)]
pub(crate) enum Event {
    Open,
    Close,
    Add,
    Remove(PathBuf),
    Save,
    Picked(Result<Vec<PathBuf>, String>),
}

pub(crate) fn setup_callbacks(sender: EventSender, window: &AppWindow) {
    let state = window.global::<PluginListState>();
    let s = sender.clone();
    state.on_close(move || s.send(GuiEvent::PluginList(Event::Close)));
    let s = sender.clone();
    state.on_open(move || s.send(GuiEvent::PluginList(Event::Open)));
    let s = sender.clone();
    state.on_add(move || s.send(GuiEvent::PluginList(Event::Add)));
    let s = sender.clone();
    let weak = window.as_weak();
    state.on_remove(move |i| {
        if let Some(w) = weak.upgrade() {
            if let Some(row) = w
                .global::<PluginListState>()
                .get_rows()
                .row_data(i as usize)
            {
                s.send(GuiEvent::PluginList(Event::Remove(PathBuf::from(
                    row.path.as_str(),
                ))));
            }
        }
    });
    state.on_save(move || sender.send(GuiEvent::PluginList(Event::Save)));
}

pub(in crate::app) fn dispatch(event: Event, app: &mut App, window: &AppWindow) {
    let state = window.global::<PluginListState>();
    match event {
        Event::Close => state.set_visible(false),
        Event::Open => {
            if !app.plugin_runtime.plugin_list.applying() {
                app.plugin_runtime.plugin_list = PluginList::read(
                    &patinae_settings::paths::PathResolver::from_process_env().config_file(),
                    &loaded_entries(&app.plugin_runtime.plugins),
                );
            }
            state.set_visible(true);
            app.sync_plugin_list(window);
        }
        Event::Add => {
            if state.get_picking() || app.plugin_runtime.plugin_list.applying() {
                return;
            }
            state.set_picking(true);
            app.effects.push(Effect::PickPlugins);
        }
        Event::Picked(result) => {
            state.set_picking(false);
            match result {
                Ok(paths) => app.plugin_runtime.plugin_list.add(paths),
                Err(error) => app.plugin_runtime.plugin_list.save_error = error,
            }
        }
        Event::Remove(path) => {
            if app.plugin_runtime.plugin_list.applying() {
                return;
            }
            if let Some(index) = app
                .plugin_runtime
                .plugin_list
                .draft
                .entries
                .iter()
                .position(|p| p == &path)
            {
                app.plugin_runtime.plugin_list.remove(index);
            }
        }
        Event::Save => {
            if app.plugin_runtime.plugin_list.applying() || !state.get_loading_status().is_empty() {
                return;
            }
            app.plugin_runtime.plugin_list.save();
        }
    }
    app.plugin_runtime.plugin_list.publish(window);
}
