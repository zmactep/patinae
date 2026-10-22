//! Sequence gestures retain their preview until the queued commit is acknowledged.
use super::*;
use crate::app::{
    events::{EventSender, GuiEvent},
    App,
};

#[derive(Debug)]
pub(crate) enum Event {
    Pointer {
        kind: i32,
        row: i32,
        x: f32,
        screen: (f32, f32),
        exclude: bool,
        generation: i32,
    },
    Commit {
        id: u64,
        row: i32,
        start: i32,
        end: i32,
        exclude: bool,
        generation: i32,
    },
    ViewerColors,
    CopyChain(i32),
    CopyAll,
    ClearHover,
    Cancel,
    CloseContext,
}

#[derive(Debug, Default)]
pub(super) enum Gesture {
    #[default]
    Idle,
    Dragging {
        id: u64,
        row: i32,
        start: i32,
        exclude: bool,
        generation: i32,
    },
    CommitPending {
        id: u64,
    },
}

pub(crate) fn setup_callbacks(sender: EventSender, window: &AppWindow) {
    let state = window.global::<SequenceState>();
    let s = sender.clone();
    state.on_pointer(move |kind, row, x, sx, sy, exclude, generation| {
        s.send(GuiEvent::Sequence(Event::Pointer {
            kind,
            row,
            x,
            screen: (sx, sy),
            exclude,
            generation,
        }))
    });
    let s = sender.clone();
    state.on_close_context(move || s.send(GuiEvent::Sequence(Event::CloseContext)));
    let s = sender.clone();
    state.on_viewer_colors_toggled(move || s.send(GuiEvent::Sequence(Event::ViewerColors)));
    let s = sender.clone();
    state.on_copy_chain_fasta(move |row| s.send(GuiEvent::Sequence(Event::CopyChain(row))));
    let s = sender.clone();
    state.on_copy_all_fasta(move || s.send(GuiEvent::Sequence(Event::CopyAll)));
    state.on_hover_left(move || sender.send(GuiEvent::Sequence(Event::ClearHover)));
}

fn finish_preview(bridge: &mut SequenceBridge, state: &SequenceState) {
    bridge.gesture = Gesture::Idle;
    state.set_drag_active(false);
    state.set_commit_pending(false);
}

pub(in crate::app) fn dispatch(event: Event, app: &mut App, window: &AppWindow) {
    let state = window.global::<SequenceState>();
    match event {
        Event::Cancel => {
            app.sequence.pending_commits.clear();
            finish_preview(&mut app.sequence, &state);
            state.invoke_clear_hover();
        }
        Event::CloseContext => state.set_context_menu_open(false),
        Event::ViewerColors => {
            app.sequence.toggle_viewer_colors();
            app.sequence.invalidate();
        }
        Event::ClearHover => state.invoke_clear_hover(),
        Event::CopyChain(row) => {
            state.set_context_menu_open(false);
            let text = app.sequence.chain_fasta(row).map(|(text, count, chain)| {
                (text, format!("Copied {count} residues from chain {chain}"))
            });
            copy_text(text, app);
        }
        Event::CopyAll => {
            state.set_context_menu_open(false);
            let text = app
                .sequence
                .all_fasta()
                .map(|(text, count)| (text, format!("Copied {count} residues to clipboard")));
            copy_text(text, app);
        }
        Event::Commit {
            id,
            row,
            start,
            end,
            exclude,
            generation,
        } => {
            if app
                .sequence
                .model
                .needs_rebuild(&app.kernel.session.registry, app.sequence.viewer_colors)
            {
                app.sequence.sync(&app.kernel, window);
            }

            if !app.sequence.pending_commits.remove(&id) {
                return;
            }
            if generation == app.sequence.generation {
                let Some(command) = app.sequence.build_drag_command(
                    row,
                    start,
                    end,
                    exclude,
                    app.kernel.session.selections.contains("sele"),
                ) else {
                    if matches!(app.sequence.gesture, Gesture::CommitPending { id: pending } if pending == id)
                    {
                        finish_preview(&mut app.sequence, &state);
                    }
                    return;
                };
                let viewport = App::viewport_size(window);
                if let Err(error) = app.kernel.execute_command(&command, false, None, viewport) {
                    app.kernel
                        .bus
                        .print_warning(format!("Sequence selection failed: {error}"));
                }
                app.objects.sync(&mut app.kernel, window);
                app.sequence.sync(&app.kernel, window);
            }
            if matches!(app.sequence.gesture,Gesture::CommitPending {id: pending} if pending==id) {
                finish_preview(&mut app.sequence, &state);
            }
        }
        Event::Pointer {
            kind,
            row,
            x,
            screen,
            exclude,
            generation,
        } => {
            // A command ahead of this event may have replaced the row model.
            if app
                .sequence
                .model
                .needs_rebuild(&app.kernel.session.registry, app.sequence.viewer_colors)
            {
                app.sequence.sync(&app.kernel, window);
            }
            if generation != app.sequence.generation || !x.is_finite() {
                finish_preview(&mut app.sequence, &state);
                state.invoke_clear_hover();
                return;
            }
            if kind == 3 {
                app.sequence.pending_commits.clear();
                finish_preview(&mut app.sequence, &state);
                state.invoke_clear_hover();
                return;
            }
            if kind == 5 {
                state.invoke_clear_hover();
                state.set_context_menu_chain(row);
                state.set_context_menu_chain_copy_enabled(app.sequence.chain_fasta(row).is_some());
                state.set_context_menu_x(screen.0);
                state.set_context_menu_y(screen.1);
                state.set_context_menu_open(true);
                return;
            }
            if kind == 0 {
                let (index, valid) = app.sequence.x_to_residue(row, x);
                if !valid {
                    return;
                }
                app.sequence.next_gesture = app.sequence.next_gesture.wrapping_add(1);
                app.sequence.gesture = Gesture::Dragging {
                    id: app.sequence.next_gesture,
                    row,
                    start: index,
                    exclude,
                    generation,
                };
                state.invoke_clear_hover();
                state.set_drag_chain(row);
                state.set_drag_start(index);
                state.set_drag_end(index);
                state.set_ctrl_held(exclude);
                state.set_drag_active(true);
                state.set_commit_pending(false);
                app.sequence.write_drag_band(window, row, index, index);
            } else if kind == 1 || kind == 2 {
                if let Gesture::Dragging {
                    id,
                    row: active,
                    start,
                    exclude: initial,
                    generation: epoch,
                    ..
                } = app.sequence.gesture
                {
                    if active != row {
                        return;
                    }
                    let (end, valid) = app.sequence.x_to_residue(row, x);
                    if !valid {
                        finish_preview(&mut app.sequence, &state);
                        return;
                    }
                    app.sequence.gesture = Gesture::Dragging {
                        id,
                        row,
                        start,
                        exclude: initial,
                        generation: epoch,
                    };
                    state.set_drag_end(end);
                    app.sequence.write_drag_band(window, row, start, end);
                    if kind == 2 {
                        app.sequence.pending_commits.insert(id);
                        app.sequence.gesture = Gesture::CommitPending { id };
                        state.set_drag_active(false);
                        state.set_commit_pending(true);
                        app.events.send(GuiEvent::Sequence(Event::Commit {
                            id,
                            row,
                            start,
                            end,
                            exclude,
                            generation,
                        }));
                    }
                } else if kind == 1 && !state.get_context_menu_open() {
                    if let Some((index, start, end)) = app.sequence.hit_residue(row, x) {
                        if let Some((text, _)) = app.sequence.resolve_residue(row, index) {
                            state.set_hover_px_start(start);
                            state.set_hover_px_end(end);
                            state.set_hover_chain(row);
                            state.set_hover_residue(index);
                            state.set_hover_tooltip(text.clone().into());
                            let ticket = window.global::<crate::TooltipState>().invoke_show(
                                text.into(),
                                screen.0,
                                screen.0,
                                screen.1,
                                screen.1,
                                true,
                            );
                            state.set_hover_ticket(ticket);
                            state.set_hover_active(true);
                        }
                    } else {
                        state.invoke_clear_hover();
                    }
                }
            }
        }
    }
}

fn copy_text(text: Option<(String, String)>, app: &mut App) {
    match text {
        Some((text, message)) => app
            .effects
            .push(crate::app::effects::Effect::Clipboard { text, message }),
        None => {
            app.kernel
                .bus
                .print_warning("No polymer sequence available.");
            app.notify_short("No polymer sequence available.");
        }
    }
}
