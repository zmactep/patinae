use std::cell::RefCell;
use std::rc::Rc;

use slint::ComponentHandle;

use patinae_framework::message::AppMessage;

use crate::{AppWindow, LayoutState, ReplState};

// ---------------------------------------------------------------------------
// Callbacks
// ---------------------------------------------------------------------------

pub fn setup_callbacks(app: Rc<RefCell<crate::app::App>>, window: &AppWindow) {
    let ls = window.global::<LayoutState>();

    // --- Show panel ---
    {
        let app = app.clone();
        ls.on_show_panel(move |id| {
            let mut a = app.borrow_mut();
            a.kernel.bus.send(AppMessage::ShowPanel(id.to_string()));
        });
    }

    // --- Hide panel ---
    {
        let app = app.clone();
        ls.on_hide_panel(move |id| {
            let mut a = app.borrow_mut();
            a.kernel.bus.send(AppMessage::HidePanel(id.to_string()));
        });
    }
}

// ---------------------------------------------------------------------------
// Dispatch unhandled messages to Slint layout state
// ---------------------------------------------------------------------------

pub fn dispatch_messages(messages: &[AppMessage], window: &AppWindow) {
    let ls = window.global::<LayoutState>();

    for msg in messages {
        match msg {
            AppMessage::ShowPanel(id) => set_panel_visible(id, true, window, &ls),
            AppMessage::HidePanel(id) => set_panel_visible(id, false, window, &ls),
            AppMessage::TogglePanel(id) => {
                let visible = get_panel_visible(id, &ls);
                set_panel_visible(id, !visible, window, &ls);
            }
            _ => {}
        }
    }
}

fn get_panel_visible(id: &str, ls: &LayoutState) -> bool {
    match id {
        "objects" => ls.get_objects_visible(),
        "selections" => ls.get_selections_visible(),
        "recent-atoms" => ls.get_recent_atoms_visible(),
        "sequence" => ls.get_sequence_visible(),
        "movie" => ls.get_movie_visible(),
        "repl" => ls.get_repl_visible(),
        _ => false,
    }
}

fn set_panel_visible(id: &str, visible: bool, window: &AppWindow, ls: &LayoutState) {
    match id {
        "objects" => ls.set_objects_visible(visible),
        "selections" => ls.set_selections_visible(visible),
        "recent-atoms" => ls.set_recent_atoms_visible(visible),
        "sequence" => {
            ls.set_sequence_visible(visible);
            if visible {
                ls.set_bottom_active_tab(0);
            } else if ls.get_movie_visible() {
                ls.set_bottom_active_tab(1);
            }
        }
        "movie" => {
            ls.set_movie_visible(visible);
            if visible {
                ls.set_bottom_active_tab(1);
            } else if ls.get_sequence_visible() {
                ls.set_bottom_active_tab(0);
            }
        }
        "repl" => {
            ls.set_repl_visible(visible);
            if visible {
                window.global::<ReplState>().invoke_request_focus();
            }
        }
        _ => log::warn!("Unknown panel id: {}", id),
    }
}
