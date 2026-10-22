//! Native input capture and ordered viewport interaction.
use super::*;
use patinae_scene::{ButtonState, Modifiers, MouseButton, ScrollDelta};

#[derive(Clone, Copy, Debug)]
pub(crate) struct PointerEvent {
    pub kind: i32,
    pub button: i32,
    pub point: (f32, f32),
    pub modifiers: Modifiers,
    pub scale: f32,
    pub viewport: (u32, u32),
    pub suppress: bool,
}

#[derive(Debug)]
pub(crate) enum Event {
    Pointer(PointerEvent),
    Scroll {
        delta: f32,
        viewport: (u32, u32),
    },
    Cursor {
        blocked: bool,
        point: (f32, f32),
        local: (f32, f32),
        scale: f32,
        viewport: (u32, u32),
    },
    Modifiers(Modifiers),
    Focus(bool),
    Display,
    Pinch(f64),
    Drop(PathBuf),
    Key {
        binding: KeyBinding,
        pressed: bool,
        repeat: bool,
        text: slint::SharedString,
        modifiers: Modifiers,
    },
    LabelSize {
        owner: String,
        entity: i32,
        size: [f32; 2],
    },
    CancelTask(String),
    ToggleTheme,
}

pub(crate) fn setup_callbacks(sender: events::EventSender, window: &AppWindow) {
    let vp = window.global::<ViewportState>();
    let weak = window.as_weak();
    let s = sender.clone();
    vp.on_pointer(
        move |kind, button, x, y, shift, control, alt, meta, suppress| {
            let Some(w) = weak.upgrade() else { return };
            let mut modifiers = bridges::input::physical_slint_modifiers(
                shift,
                control,
                alt,
                meta,
                cfg!(target_os = "macos"),
            );
            if kind == 1 {
                modifiers = s.captured_modifiers().unwrap_or(modifiers);
            }
            s.send(events::GuiEvent::Input(Event::Pointer(PointerEvent {
                kind,
                button,
                point: (x, y),
                modifiers,
                scale: w.window().scale_factor(),
                viewport: App::viewport_size(&w),
                suppress,
            })))
        },
    );
    let weak = window.as_weak();
    let s = sender.clone();
    vp.on_scroll(move |delta| {
        if let Some(w) = weak.upgrade() {
            s.send(events::GuiEvent::Input(Event::Scroll {
                delta,
                viewport: App::viewport_size(&w),
            }));
        }
    });
    let s = sender.clone();
    vp.on_label_size(move |owner, entity, width, height| {
        s.send(events::GuiEvent::Input(Event::LabelSize {
            owner: owner.to_string(),
            entity,
            size: [width, height],
        }))
    });
    let s = sender.clone();
    window
        .global::<NotificationState>()
        .on_cancel_task(move |id| {
            s.send(events::GuiEvent::Input(Event::CancelTask(id.to_string())))
        });
    window
        .global::<Theme>()
        .on_toggle_theme(move || sender.send(events::GuiEvent::Input(Event::ToggleTheme)));
}

pub(super) fn dispatch(event: Event, app: &mut App, window: &AppWindow) {
    match event {
        Event::Pointer(event) => app.pointer_event(event, window),
        Event::Scroll { delta, viewport } => {
            app.kernel
                .viewport
                .input
                .handle_scroll(ScrollDelta::PixelDelta(0.0, delta as f64));
            if app.kernel.process_input(viewport.1 as f32) {
                app.dismiss_viewport_image();
            }
        }
        Event::Cursor {
            blocked,
            point,
            local,
            scale,
            viewport,
        } => {
            app.interaction.cursor_pos.set(point);
            app.interaction.cursor_known.set(!blocked);
            if !blocked
                || app.kernel.viewport.input.any_button_pressed()
                || app.interaction.manipulation.is_reserved()
            {
                let (shift, ctrl, alt, logo) = app.interaction.winit_modifiers.get();
                app.pointer_event(
                    PointerEvent {
                        kind: 1,
                        button: 0,
                        point: local,
                        modifiers: Modifiers {
                            shift,
                            ctrl,
                            alt,
                            logo,
                        },
                        scale,
                        viewport,
                        suppress: false,
                    },
                    window,
                );
            } else {
                app.interaction.manipulation.cancel();
                app.clear_hover_indicators();
            }
        }
        Event::Modifiers(modifiers) => {
            app.interaction.winit_modifiers.set((
                modifiers.shift,
                modifiers.ctrl,
                modifiers.alt,
                modifiers.logo,
            ));
            app.kernel.viewport.input.handle_modifiers(modifiers);
            app.interaction
                .manipulation
                .set_modifiers(modifiers, cfg!(target_os = "macos"));
        }
        Event::Focus(focused) => {
            app.reset_pointer(window);
            if focused {
                app.mark_display_transition();
            } else {
                app.interaction.cursor_known.set(false);
            }
        }
        Event::Display => {
            app.reset_pointer(window);
            app.mark_display_transition();
        }
        Event::Pinch(delta) => {
            app.kernel.viewport.input.handle_pinch_zoom(delta);
            if app
                .kernel
                .process_input(App::viewport_size(window).1 as f32)
            {
                app.dismiss_viewport_image();
            }
        }
        Event::Drop(path) => app.route_dropped_file(&path),
        Event::Key {
            binding,
            pressed,
            repeat,
            text,
            modifiers,
        } => {
            if pressed
                && app
                    .plugin_runtime
                    .plugins
                    .handle_hotkey(binding, &mut app.kernel.bus)
            {
                return;
            }
            if !text.is_empty() {
                app.effects.push(effects::Effect::Key {
                    text,
                    modifiers,
                    pressed,
                    repeat,
                });
            }
        }
        Event::LabelSize {
            owner,
            entity,
            size,
        } => app.viewport.set_label_size(owner, entity, size),
        Event::CancelTask(id) => {
            if let Ok(id) = id.parse() {
                let _ = app.kernel.tasks.cancel(id);
            }
        }
        Event::ToggleTheme => {
            let next = if app.kernel.session.settings.ui.theme == ThemeMode::Dark {
                "light"
            } else {
                "dark"
            };
            app.kernel
                .bus
                .execute_command_silent(format!("set theme, {next}"));
        }
    }
}

impl App {
    fn reset_pointer(&mut self, window: &AppWindow) {
        bridges::sequence::events::dispatch(bridges::sequence::events::Event::Cancel, self, window);
        self.kernel.viewport.input.reset();
        self.interaction.manipulation.cancel();
        self.interaction.input = InputBridge::new();
        self.interaction
            .winit_modifiers
            .set((false, false, false, false));
        let vp = window.global::<ViewportState>();
        vp.set_left_pressed(false);
        vp.set_middle_pressed(false);
        vp.set_right_pressed(false);
        self.clear_hover_indicators();
    }

    fn pointer_event(&mut self, event: PointerEvent, window: &AppWindow) {
        if event.kind == 3 {
            self.reset_pointer(window);
            return;
        }
        if !event.point.0.is_finite() || !event.point.1.is_finite() {
            return;
        }
        let vp = window.global::<ViewportState>();
        let input = &mut self.kernel.viewport.input;
        // Native release and toolkit capture can both report the same edge.
        // Only the currently held button owns its release and final motion.
        if event.kind == 2
            && !match event.button {
                1 => input.middle_pressed(),
                2 => input.right_pressed(),
                _ => input.left_pressed(),
            }
        {
            return;
        }

        input.handle_modifiers(event.modifiers);
        self.interaction.winit_modifiers.set((
            event.modifiers.shift,
            event.modifiers.ctrl,
            event.modifiers.alt,
            event.modifiers.logo,
        ));
        self.interaction.manipulation.set_targets(
            self.viewport
                .manipulation_targets(&self.kernel.session.registry),
        );
        self.interaction
            .manipulation
            .set_modifiers(event.modifiers, cfg!(target_os = "macos"));
        let point = [event.point.0, event.point.1];
        let mut reserved = self.interaction.manipulation.is_reserved();
        if event.kind == 0 && event.button == 0 {
            reserved |= self
                .interaction
                .manipulation
                .press(point, &self.kernel.session.registry);
        } else {
            self.interaction
                .manipulation
                .motion(point, &mut self.kernel.session.registry);
        }
        if event.kind == 2 && event.button == 0 {
            reserved |= self.interaction.manipulation.release();
        }
        // Update motion while the previous button is still held, including release endpoints.
        input.handle_mouse_motion((
            (event.point.0 * event.scale) as f64,
            (event.point.1 * event.scale) as f64,
        ));
        if reserved {
            input.set_pointer_gesture_reserved(true);
        }
        if self.kernel.process_input(event.viewport.1 as f32) {
            self.dismiss_viewport_image();
        }
        let button = match event.button {
            1 => MouseButton::Middle,
            2 => MouseButton::Right,
            _ => MouseButton::Left,
        };
        if event.kind == 0 || event.kind == 2 {
            self.kernel.viewport.input.handle_mouse_button(
                if event.kind == 0 {
                    ButtonState::Pressed
                } else {
                    ButtonState::Released
                },
                button,
            );
        }
        self.interaction.input.pointer(
            event.kind,
            event.button,
            event.point,
            event.scale,
            event.suppress || reserved,
            event.modifiers,
        );
        vp.set_shift_held(event.modifiers.shift);
        vp.set_control_held(if cfg!(target_os = "macos") {
            event.modifiers.logo
        } else {
            event.modifiers.ctrl
        });
        vp.set_meta_held(if cfg!(target_os = "macos") {
            event.modifiers.ctrl
        } else {
            event.modifiers.logo
        });
        vp.set_alt_held(event.modifiers.alt);
        if event.kind == 0 {
            vp.set_press_x(event.point.0);
            vp.set_press_y(event.point.1);
        }
        vp.set_mouse_x(event.point.0);
        vp.set_mouse_y(event.point.1);
        vp.set_left_pressed(self.kernel.viewport.input.left_pressed());
        vp.set_middle_pressed(self.kernel.viewport.input.middle_pressed());
        vp.set_right_pressed(self.kernel.viewport.input.right_pressed());
        if let Some(click) = self.interaction.input.take_pending_click() {
            if self.process_click(click, event.viewport) {
                self.clear_hover_indicators();
            }
        }
        if reserved {
            self.clear_hover_indicators();
        }
    }
}

#[cfg(test)]
mod tests;
