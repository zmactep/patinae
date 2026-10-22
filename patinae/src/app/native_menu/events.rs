//! Native menu ingress and dispatch.
use super::*;
use crate::app::{
    events::{EventSender, GuiEvent},
    App,
};
#[derive(Debug)]
pub(crate) enum Event {
    FetchClose,
    FetchFormat(slint::SharedString),
    MenuAction(slint::SharedString),
    MenuOpenRecent(slint::SharedString),
    MenuFetchPdbEdited(slint::SharedString),
    MenuFetchPreview(slint::SharedString),
    MenuFetchSubmit(
        slint::SharedString,
        slint::SharedString,
        slint::SharedString,
    ),
    MenuFetchOpenDoi(slint::SharedString),
    StartFetchPdbEdited(slint::SharedString),
    StartFetchSubmit(slint::SharedString),
    StartOpenExample(slint::SharedString),
}
pub(crate) fn setup_callbacks(sender: EventSender, window: &AppWindow) {
    let menu = window.global::<MenuState>();
    let s = sender.clone();
    menu.on_fetch_close(move || s.send(GuiEvent::Menu(Event::FetchClose)));
    let s = sender.clone();
    menu.on_fetch_format_selected(move |v| s.send(GuiEvent::Menu(Event::FetchFormat(v))));
    let start = window.global::<StartState>();
    {
        let sender = sender.clone();
        menu.on_action(move |action| sender.send(GuiEvent::Menu(Event::MenuAction(action))));
    }
    {
        let sender = sender.clone();
        menu.on_open_recent(move |path| sender.send(GuiEvent::Menu(Event::MenuOpenRecent(path))));
    }
    {
        let sender = sender.clone();
        menu.on_fetch_pdb_edited(move |pdb_id| {
            sender.send(GuiEvent::Menu(Event::MenuFetchPdbEdited(pdb_id)))
        });
    }
    {
        let sender = sender.clone();
        menu.on_fetch_preview(move |pdb_id| {
            sender.send(GuiEvent::Menu(Event::MenuFetchPreview(pdb_id)))
        });
    }
    {
        let sender = sender.clone();
        menu.on_fetch_submit(move |pdb_id, object_name, format| {
            sender.send(GuiEvent::Menu(Event::MenuFetchSubmit(
                pdb_id,
                object_name,
                format,
            )))
        });
    }
    {
        let sender = sender.clone();
        menu.on_fetch_open_doi(move |doi| {
            sender.send(GuiEvent::Menu(Event::MenuFetchOpenDoi(doi)))
        });
    }
    {
        let sender = sender.clone();
        start.on_fetch_pdb_edited(move |pdb_id| {
            sender.send(GuiEvent::Menu(Event::StartFetchPdbEdited(pdb_id)))
        });
    }
    {
        let sender = sender.clone();
        start.on_fetch_submit(move |pdb_id| {
            sender.send(GuiEvent::Menu(Event::StartFetchSubmit(pdb_id)))
        });
    }
    {
        let sender = sender.clone();
        start.on_open_example(move |pdb_id| {
            sender.send(GuiEvent::Menu(Event::StartOpenExample(pdb_id)))
        });
    }
}
pub(in crate::app) fn dispatch(event: Event, app: &mut App, window: &AppWindow) {
    let weak = window.as_weak();
    match event {
        Event::FetchClose => window.global::<MenuState>().set_fetch_visible(false),
        Event::FetchFormat(value) => window.global::<MenuState>().set_fetch_format(value),
        Event::MenuAction(action) => {
            let Some(window) = weak.upgrade() else {
                return;
            };
            if window.global::<crate::PluginListState>().get_visible() {
                return;
            }
            dispatch_menu_action(&mut *app, &window, action.as_str());
        }
        Event::MenuOpenRecent(path) => {
            let Some(window) = weak.upgrade() else {
                return;
            };
            if window.global::<crate::PluginListState>().get_visible() {
                return;
            }
            app.open_recent_file(&window, path.as_str());
        }
        Event::MenuFetchPdbEdited(pdb_id) => {
            let Some(window) = weak.upgrade() else {
                return;
            };
            normalize_fetch_pdb_edit(&window, pdb_id.as_str());
        }
        Event::MenuFetchPreview(pdb_id) => {
            let Some(window) = weak.upgrade() else {
                return;
            };
            request_fetch_preview(&mut *app, &window, pdb_id.as_str());
        }
        Event::MenuFetchSubmit(pdb_id, object_name, format) => {
            let Some(window) = weak.upgrade() else {
                return;
            };
            submit_fetch(
                &mut *app,
                &window,
                pdb_id.as_str(),
                object_name.as_str(),
                format.as_str(),
            );
        }
        Event::MenuFetchOpenDoi(doi) => {
            let Some(window) = weak.upgrade() else {
                return;
            };
            open_doi_link(&mut *app, &window, doi.as_str());
        }
        Event::StartFetchPdbEdited(pdb_id) => {
            let Some(window) = weak.upgrade() else {
                return;
            };
            normalize_start_fetch_edit(&window, pdb_id.as_str());
        }
        Event::StartFetchSubmit(pdb_id) => {
            let Some(window) = weak.upgrade() else {
                return;
            };
            submit_start_fetch(&mut *app, &window, pdb_id.as_str());
        }
        Event::StartOpenExample(pdb_id) => {
            let Some(window) = weak.upgrade() else {
                return;
            };
            submit_start_fetch(&mut *app, &window, pdb_id.as_str());
        }
    }
}
