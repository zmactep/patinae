//! Input bridge: Slint ViewportState globals → patinae-scene InputState.
//!
//! Qualifies each ordered gesture independently of frame cadence.

#[cfg(test)]
use patinae_scene::InputState;
use patinae_scene::Modifiers;

/// Click detection threshold in logical pixels (scale-independent).
const CLICK_THRESHOLD_LP: f32 = 5.0;

/// Slint uses shortcut semantics: on macOS Control means Cmd and Meta means Ctrl.
/// Convert its durable event snapshot to the physical flags used by scene input.
pub(in crate::app) fn physical_slint_modifiers(
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
    /// Mouse position at left button press (logical pixels), for click vs drag detection.
    click_start_pos: Option<(f32, f32)>,
    suppressed: bool,
    /// A click that passed the threshold test, ready to be consumed by the picker.
    pending_click: Option<PendingClick>,
}

impl InputBridge {
    pub fn new() -> Self {
        Self {
            prev_left: false,
            click_start_pos: None,
            suppressed: false,
            pending_click: None,
        }
    }

    /// Consume and return a pending click, if any.
    pub fn take_pending_click(&mut self) -> Option<PendingClick> {
        self.pending_click.take()
    }

    pub(crate) fn pointer(
        &mut self,
        kind: i32,
        button: i32,
        point: (f32, f32),
        scale: f32,
        suppress: bool,
        modifiers: Modifiers,
    ) {
        if button != 0 {
            return;
        }
        if kind == 0 {
            self.click_start_pos = Some(point);
            self.prev_left = true;
            self.suppressed = suppress;
        } else if self.prev_left {
            self.suppressed |= suppress
                || self.click_start_pos.is_some_and(|start| {
                    (point.0 - start.0).hypot(point.1 - start.1) >= CLICK_THRESHOLD_LP
                });
            if kind == 2 {
                self.prev_left = false;
                if let Some(start) = self.click_start_pos.take() {
                    self.queue_click_if_qualified(start, point, scale, self.suppressed, modifiers);
                }
            }
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
}

#[cfg(test)]
mod gesture_tests {
    use super::*;
    #[test]
    fn each_click_has_one_release_and_suppression_is_sticky() {
        let mut input = InputBridge::new();
        for scale in [1.0, 1.5, 2.0] {
            input.pointer(0, 0, (10.0, 10.0), scale, false, Modifiers::EMPTY);
            input.pointer(2, 0, (11.0, 11.0), scale, false, Modifiers::SUPER);
            assert_eq!(
                input.take_pending_click(),
                Some(PendingClick {
                    position: (11.0 * scale, 11.0 * scale),
                    ctrl_or_cmd: true
                })
            );
            input.pointer(2, 0, (11.0, 11.0), scale, false, Modifiers::EMPTY);
            assert_eq!(input.take_pending_click(), None);
            input.pointer(0, 0, (10.0, 10.0), scale, true, Modifiers::EMPTY);
            input.pointer(2, 0, (11.0, 11.0), scale, false, Modifiers::EMPTY);
            assert_eq!(input.take_pending_click(), None);
            input.pointer(0, 0, (10.0, 10.0), scale, false, Modifiers::EMPTY);
            input.pointer(1, 0, (40.0, 40.0), scale, false, Modifiers::EMPTY);
            input.pointer(2, 0, (10.0, 10.0), scale, false, Modifiers::EMPTY);
            assert_eq!(
                input.take_pending_click(),
                None,
                "returning to press after a drag is not a click"
            );
        }
    }
}
