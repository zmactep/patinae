//! Input bridge: Slint ViewportState globals → patinae-scene InputState.
//!
//! Slint exposes boolean "is pressed" state for mouse buttons. InputState
//! needs press/release events. This bridge tracks previous frame state
//! and emits transitions.

use patinae_scene::{ButtonState, InputState, Modifiers, MouseButton, ScrollDelta};

use crate::ViewportState;

/// Click detection threshold in logical pixels (scale-independent).
const CLICK_THRESHOLD_LP: f32 = 5.0;

/// Slint uses shortcut semantics: on macOS Control means Cmd and Meta means Ctrl.
/// Convert its durable event snapshot to the physical flags used by scene input.
fn physical_slint_modifiers(
    shift: bool,
    control: bool,
    alt: bool,
    meta: bool,
    macos: bool,
) -> Modifiers {
    Modifiers {
        shift,
        ctrl: if macos { meta } else { control },
        alt,
        logo: if macos { control } else { meta },
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PointerSnapshot {
    pub mouse_logical: (f32, f32),
    pub press_logical: (f32, f32),
    pub left_pressed: bool,
    pub middle_pressed: bool,
    pub right_pressed: bool,
    pub suppress_click: bool,
}

/// A qualified viewport click captured at mouse-up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PendingClick {
    /// Click position in physical pixels.
    pub position: (f32, f32),
    /// Whether Ctrl or Cmd was active when the click qualified.
    pub ctrl_or_cmd: bool,
}

/// Translates Slint viewport input globals into patinae-scene [`InputState`] events.
pub struct InputBridge {
    prev_left: bool,
    prev_middle: bool,
    prev_right: bool,
    /// Mouse position at left button press (logical pixels), for click vs drag detection.
    click_start_pos: Option<(f32, f32)>,
    /// Last Slint click event serial consumed. Slint keeps this as a durable
    /// event counter so quick press+release pairs are not lost between frames.
    last_click_serial: Option<i32>,
    /// A click that passed the threshold test, ready to be consumed by the picker.
    pending_click: Option<PendingClick>,
}

impl InputBridge {
    pub fn new() -> Self {
        Self {
            prev_left: false,
            prev_middle: false,
            prev_right: false,
            click_start_pos: None,
            last_click_serial: None,
            pending_click: None,
        }
    }

    /// Consume and return a pending click, if any.
    pub fn take_pending_click(&mut self) -> Option<PendingClick> {
        self.pending_click.take()
    }

    /// Read current state from Slint ViewportState and push events into InputState.
    ///
    /// `winit_modifiers` is `(shift, ctrl, alt, super)` from winit's
    /// `ModifiersChanged` event — always up-to-date, unlike Slint's
    /// `pointer-event` which only fires on mouse button changes.
    pub fn sync(
        &mut self,
        input: &mut InputState,
        vp: &ViewportState,
        scale_factor: f32,
        winit_modifiers: (bool, bool, bool, bool),
        pointer: Option<PointerSnapshot>,
    ) {
        let (mouse_logical, press_logical, left_now, middle_now, right_now, suppress_click) =
            if let Some(pointer) = pointer {
                (
                    pointer.mouse_logical,
                    pointer.press_logical,
                    pointer.left_pressed,
                    pointer.middle_pressed,
                    pointer.right_pressed,
                    pointer.suppress_click,
                )
            } else {
                (
                    (vp.get_mouse_x(), vp.get_mouse_y()),
                    (vp.get_press_x(), vp.get_press_y()),
                    vp.get_left_pressed(),
                    vp.get_middle_pressed(),
                    vp.get_right_pressed(),
                    vp.get_suppress_click(),
                )
            };
        let mouse_phys = (
            mouse_logical.0 * scale_factor,
            mouse_logical.1 * scale_factor,
        );
        let press_phys = (
            press_logical.0 * scale_factor,
            press_logical.1 * scale_factor,
        );
        let click_serial = vp.get_click_serial();
        let slint_click = if self
            .last_click_serial
            .replace(click_serial)
            .is_some_and(|prev| prev != click_serial)
        {
            Some((vp.get_click_x(), vp.get_click_y()))
        } else {
            None
        };

        // Modifiers: use winit state (always current, even mid-drag)
        let (shift, ctrl, alt, logo) = winit_modifiers;
        let modifiers = Modifiers {
            shift,
            ctrl,
            alt,
            logo,
        };
        input.handle_modifiers(modifiers);

        // Button transitions
        let left_started = !self.prev_left && left_now;
        let left_released = self.prev_left && !left_now;
        let any_button_started =
            left_started || (!self.prev_middle && middle_now) || (!self.prev_right && right_now);

        if any_button_started {
            input.handle_mouse_motion((press_phys.0 as f64, press_phys.1 as f64));
        }

        Self::detect_transition(&mut self.prev_left, left_now, input, MouseButton::Left);
        Self::detect_transition(
            &mut self.prev_middle,
            middle_now,
            input,
            MouseButton::Middle,
        );
        Self::detect_transition(&mut self.prev_right, right_now, input, MouseButton::Right);

        input.handle_mouse_motion((mouse_phys.0 as f64, mouse_phys.1 as f64));

        // Click detection: record press position, detect short-distance release.
        // Threshold is in logical pixels (scale-independent).
        if let Some(click_logical) = slint_click {
            self.click_start_pos = None;
            // Slint records these modifiers on the pointer-up event. Use that
            // durable snapshot even if winit has observed a later key release.
            let click_modifiers = physical_slint_modifiers(
                vp.get_shift_held(),
                vp.get_control_held(),
                vp.get_alt_held(),
                vp.get_meta_held(),
                cfg!(target_os = "macos"),
            );
            self.queue_click_if_qualified(
                press_logical,
                click_logical,
                scale_factor,
                suppress_click,
                click_modifiers,
            );
            if suppress_click {
                vp.set_suppress_click(false);
            }
        } else if left_started {
            self.click_start_pos = Some(press_logical);
        } else if left_released {
            if let Some(start) = self.click_start_pos.take() {
                self.queue_click_if_qualified(
                    start,
                    mouse_logical,
                    scale_factor,
                    suppress_click,
                    modifiers,
                );
            }
            if suppress_click {
                vp.set_suppress_click(false);
            }
        } else if !left_now {
            self.click_start_pos = None;
            if suppress_click {
                vp.set_suppress_click(false);
            }
        }

        // Scroll (accumulated by Slint, consumed here)
        let scroll = vp.get_scroll_delta();
        if scroll != 0.0 {
            input.handle_scroll(ScrollDelta::PixelDelta(0.0, scroll as f64));
            vp.set_scroll_delta(0.0);
        }
    }

    fn queue_click_if_qualified(
        &mut self,
        start_logical: (f32, f32),
        end_logical: (f32, f32),
        scale_factor: f32,
        suppress_click: bool,
        modifiers: Modifiers,
    ) {
        let dx = end_logical.0 - start_logical.0;
        let dy = end_logical.1 - start_logical.1;
        if (dx * dx + dy * dy).sqrt() >= CLICK_THRESHOLD_LP || suppress_click {
            return;
        }

        if modifiers.manipulation_chord(cfg!(target_os = "macos")) {
            return;
        }
        self.pending_click = Some(PendingClick {
            position: (end_logical.0 * scale_factor, end_logical.1 * scale_factor),
            ctrl_or_cmd: modifiers.ctrl || modifiers.logo,
        });
    }

    fn detect_transition(
        prev: &mut bool,
        current: bool,
        input: &mut InputState,
        button: MouseButton,
    ) {
        if current != *prev {
            let state = if current {
                ButtonState::Pressed
            } else {
                ButtonState::Released
            };
            input.handle_mouse_button(state, button);
            *prev = current;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slint_shortcut_modifiers_are_converted_to_physical_keys() {
        for macos in [false, true] {
            let primary = physical_slint_modifiers(true, true, false, false, macos);
            assert!(primary.manipulation_chord(macos));
            let secondary = physical_slint_modifiers(true, false, false, true, macos);
            assert!(!secondary.manipulation_chord(macos));
            assert_eq!(primary.ctrl, !macos);
            assert_eq!(primary.logo, macos);
            assert_eq!(secondary.ctrl, macos);
            assert_eq!(secondary.logo, !macos);
        }

        let mut bridge = InputBridge::new();
        bridge.queue_click_if_qualified(
            (10.0, 10.0),
            (11.0, 11.0),
            1.0,
            false,
            physical_slint_modifiers(true, false, false, true, cfg!(target_os = "macos")),
        );
        assert!(bridge.take_pending_click().unwrap().ctrl_or_cmd);
    }

    #[test]
    fn manipulation_click_never_queues_atom_picking_at_any_scale() {
        for scale in [1.0, 1.5, 2.0] {
            let mut bridge = InputBridge::new();
            let macos = cfg!(target_os = "macos");
            bridge.queue_click_if_qualified(
                (10.0, 10.0),
                (11.0, 11.0),
                scale,
                false,
                Modifiers {
                    shift: true,
                    ctrl: !macos,
                    logo: macos,
                    ..Modifiers::EMPTY
                },
            );
            assert_eq!(bridge.take_pending_click(), None);
        }
    }

    #[test]
    fn qualified_click_retains_modifier_captured_at_release() {
        let mut bridge = InputBridge::new();
        let mut input = InputState::new();
        input.handle_modifiers(Modifiers::CONTROL);

        bridge.queue_click_if_qualified((10.0, 10.0), (12.0, 12.0), 2.0, false, Modifiers::CONTROL);
        input.handle_modifiers(Modifiers::EMPTY);
        assert!(!input.ctrl_or_cmd_held());

        let click = bridge.take_pending_click().expect("click should qualify");
        assert_eq!(click.position, (24.0, 24.0));
        assert!(click.ctrl_or_cmd);
    }

    #[test]
    fn drag_never_queues_click_with_or_without_modifier() {
        for modifiers in [Modifiers::EMPTY, Modifiers::CONTROL, Modifiers::SUPER] {
            let mut bridge = InputBridge::new();

            bridge.queue_click_if_qualified(
                (10.0, 10.0),
                (10.0 + CLICK_THRESHOLD_LP, 10.0),
                1.0,
                false,
                modifiers,
            );

            assert_eq!(bridge.take_pending_click(), None, "modifiers={modifiers:?}");
        }
    }

    #[test]
    fn repl_forwarding_snapshots_modifiers_before_publishing_click() {
        let source = include_str!("../../ui/components/panels/repl-pill.slint");
        let click_serial = source
            .find("ViewportState.click-serial += 1;")
            .expect("REPL forwarding should publish viewport clicks");

        for assignment in [
            "ViewportState.shift-held   = event.modifiers.shift;",
            "ViewportState.control-held = event.modifiers.control;",
            "ViewportState.alt-held     = event.modifiers.alt;",
            "ViewportState.meta-held    = event.modifiers.meta;",
        ] {
            let modifier_snapshot = source
                .find(assignment)
                .unwrap_or_else(|| panic!("REPL forwarding should contain `{assignment}`"));
            assert!(
                modifier_snapshot < click_serial,
                "`{assignment}` must precede click publication"
            );
        }
    }
}
