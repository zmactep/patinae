//! Ordered UI ingress, independent of rendering and application borrows.

use super::{bridges, App, AppWindow};
use slint::ComponentHandle;
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    rc::{Rc, Weak},
    time::Instant,
};

/// Bound each turn so a burst of input cannot starve the native event loop.
const EVENTS_PER_TURN: usize = 256;

#[derive(Debug)]
pub(crate) enum GuiEvent {
    Close,
    HostTick,
    SaveResponse {
        request: super::SaveFileRequest,
        path: Option<String>,
    },
    Input(super::input_events::Event),
    Objects(bridges::objects::events::Event),
    Movie(bridges::movie::events::Event),
    Repl(bridges::repl::events::Event),
    Plugins(bridges::plugins::gui_events::Event),
    Layout(bridges::layout::events::Event),
    Sequence(bridges::sequence::events::Event),
    Notification(String),
    Menu(super::native_menu::events::Event),
    PluginList(bridges::plugin_list::events::Event),
    FileChosen {
        path: std::path::PathBuf,
        script: bool,
    },
    Command(String),
    Warning(String),
}

type Target = (Weak<RefCell<App>>, slint::Weak<AppWindow>);

struct QueuedEvent {
    event: GuiEvent,
    timing: Option<(&'static str, Instant)>,
}

fn input_timing(event: &GuiEvent) -> Option<(&'static str, Instant)> {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if !*ENABLED.get_or_init(|| std::env::var_os("PATINAE_INPUT_TIMING").is_some()) {
        return None;
    }
    let kind = match event {
        GuiEvent::Input(super::input_events::Event::Pointer(_)) => "pointer",
        GuiEvent::Input(super::input_events::Event::Key { .. }) => "key",
        GuiEvent::Input(super::input_events::Event::Scroll { .. }) => "scroll",
        GuiEvent::Sequence(bridges::sequence::events::Event::Pointer { .. }) => "sequence-pointer",
        GuiEvent::Sequence(bridges::sequence::events::Event::ClearHover) => "sequence-refresh",
        _ => return None,
    };
    Some((kind, Instant::now()))
}

#[derive(Default)]
struct Inbox {
    pending: RefCell<VecDeque<QueuedEvent>>,
    frame_timings: RefCell<Vec<(&'static str, Instant, f64)>>,
    native_modifiers: Cell<Option<patinae_scene::Modifiers>>,
    scheduled: Cell<bool>,
    draining: Cell<bool>,
    closed: Cell<bool>,
    target: RefCell<Option<Target>>,
}

/// Callbacks can submit events but cannot read or mutate application state.
#[derive(Clone, Default)]
pub(crate) struct EventSender(Rc<Inbox>);

impl EventSender {
    pub(super) fn bind(&self, app: &Rc<RefCell<App>>, window: &AppWindow) {
        *self.0.target.borrow_mut() = Some((Rc::downgrade(app), window.as_weak()));
        self.schedule();
    }

    pub(crate) fn send(&self, event: GuiEvent) {
        if self.0.closed.get() {
            return;
        }
        match &event {
            GuiEvent::Input(super::input_events::Event::Modifiers(value)) => {
                self.0.native_modifiers.set(Some(*value));
            }
            GuiEvent::Input(super::input_events::Event::Focus(false)) => {
                self.reset_captured_modifiers();
            }
            _ => {}
        }
        let timing = input_timing(&event);
        self.0
            .pending
            .borrow_mut()
            .push_back(QueuedEvent { event, timing });
        self.schedule();
    }

    pub(super) fn reset_captured_modifiers(&self) {
        self.0
            .native_modifiers
            .set(Some(patinae_scene::Modifiers::EMPTY));
    }

    pub(super) fn captured_modifiers(&self) -> Option<patinae_scene::Modifiers> {
        self.0.native_modifiers.get()
    }

    fn schedule(&self) {
        if self.0.closed.get()
            || self.0.draining.get()
            || self.0.pending.borrow().is_empty()
            || self.0.target.borrow().is_none()
            || self.0.scheduled.replace(true)
        {
            return;
        }
        let weak = Rc::downgrade(&self.0);
        // Native dialogs run a nested event loop. Dispatching from a Slint timer
        // would reenter its timer activation and panic while a dialog is open.
        let scheduled = slint::spawn_local(async move {
            if let Some(inbox) = weak.upgrade() {
                let sender = Self(inbox);
                sender.0.scheduled.set(false);
                sender.drain();
            }
        });
        if scheduled.is_err() {
            // A headless test may intentionally have no event-loop provider.
            // Keep the pending events available to explicit dispatch/teardown.
            self.0.scheduled.set(false);
        }
    }

    /// Processes only the batch present at entry. Reentrant events wait their turn.
    pub(crate) fn drain(&self) {
        if self.0.closed.get() || self.0.draining.replace(true) {
            return;
        }
        let target = self.0.target.borrow().clone();
        let Some((app, window)) = target.and_then(|(a, w)| Some((a.upgrade()?, w.upgrade()?)))
        else {
            self.close();
            return;
        };
        let Ok(mut state) = app.try_borrow_mut() else {
            self.0.draining.set(false);
            self.schedule();
            return;
        };
        let count = self.0.pending.borrow().len().min(EVENTS_PER_TURN);
        for _ in 0..count {
            let event = self.0.pending.borrow_mut().pop_front();
            if let Some(QueuedEvent { event, timing }) = event {
                if let Some((kind, started)) = timing {
                    let queue_ms = started.elapsed().as_secs_f64() * 1000.0;
                    self.0
                        .frame_timings
                        .borrow_mut()
                        .push((kind, started, queue_ms));
                }
                let closing = matches!(event, GuiEvent::Close);
                state.dispatch(event, &window);
                if !closing {
                    if let Some(visible) = state.process_commands(&window) {
                        state
                            .effects
                            .push(super::effects::Effect::Visibility(visible));
                    }
                }
                if closing {
                    self.0.closed.set(true);
                    self.0.pending.borrow_mut().clear();
                    break;
                }
            }
        }
        let effects = std::mem::take(&mut state.effects);
        drop(state);
        for effect in effects {
            effect.execute(&window, self);
        }
        self.0.draining.set(false);
        window.window().request_redraw();
        self.schedule();
    }

    pub(crate) fn close(&self) {
        self.0.closed.set(true);
        self.0.draining.set(false);
        self.0.pending.borrow_mut().clear();
        self.0.target.borrow_mut().take();
        self.0.frame_timings.borrow_mut().clear();
    }

    /// Measures Rust ingress to Slint AfterRendering, not display scan-out.
    pub(super) fn frame_rendered(&self) {
        let now = Instant::now();
        for (kind, started, queue_ms) in self.0.frame_timings.borrow_mut().drain(..) {
            let frame_ms = now.duration_since(started).as_secs_f64() * 1000.0;
            eprintln!("[patinae-input] kind={kind} queue_ms={queue_ms:.3} frame_ms={frame_ms:.3}");
        }
    }
}

impl App {
    fn dispatch(&mut self, event: GuiEvent, window: &AppWindow) {
        match event {
            GuiEvent::Close => {
                self.plugin_runtime.plugin_startup.cancel();
                self.rendering.renderer_startup = None;
                self.host_timer.stop();
                self.effects.push(super::effects::Effect::Quit);
            }
            GuiEvent::HostTick => {
                if let Some(visible) = self.pump_host(window) {
                    self.effects
                        .push(super::effects::Effect::Visibility(visible));
                }
                if !self.file_actions.save_file_dialog_scheduled {
                    if let Some(request) = self.file_actions.pending_save_file_requests.pop_front()
                    {
                        self.file_actions.save_file_dialog_scheduled = true;
                        self.effects
                            .push(super::effects::Effect::PluginSave(request));
                    }
                }
            }
            GuiEvent::SaveResponse { request, path } => {
                self.file_actions.save_file_dialog_scheduled = false;
                if let Some(path) = path {
                    self.plugin_runtime
                        .plugins
                        .queue_panel_event(super::files::save_file_response_event(&request, path));
                }
            }

            GuiEvent::Input(event) => super::input_events::dispatch(event, self, window),
            GuiEvent::Sequence(event) => bridges::sequence::events::dispatch(event, self, window),
            GuiEvent::Notification(message) => {
                self.kernel.bus.print_info(message.clone());
                self.notify_short(message);
            }
            GuiEvent::Menu(event) => super::native_menu::events::dispatch(event, self, window),
            GuiEvent::PluginList(event) => {
                bridges::plugin_list::events::dispatch(event, self, window)
            }
            GuiEvent::Command(command) => self.kernel.bus.execute_command(command),
            GuiEvent::Warning(message) => self.kernel.bus.print_warning(message),
            GuiEvent::FileChosen { path, script } => {
                if script {
                    self.route_menu_run_script(path);
                } else {
                    self.route_menu_open_file(path);
                }
            }
            GuiEvent::Objects(event) => bridges::objects::events::dispatch(
                event,
                &mut self.objects,
                &mut self.kernel,
                &mut self.file_actions.pending_annotation_requests,
                window,
            ),
            GuiEvent::Movie(event) => bridges::movie::events::dispatch(event, self, window),
            GuiEvent::Repl(event) => bridges::repl::events::dispatch(event, self, window),
            GuiEvent::Plugins(event) => bridges::plugins::gui_events::dispatch(event, self, window),
            GuiEvent::Layout(event) => bridges::layout::events::dispatch(event, self, window),
        }
    }
}

#[cfg(test)]
mod tests;
