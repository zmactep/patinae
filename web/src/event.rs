//! DOM event to InputState mapping.
//!
//! Converts JS mouse/keyboard event parameters into the framework-agnostic
//! input types used by `patinae_scene::InputState`.

use patinae_scene::manipulation::ViewportManipulation;
use patinae_scene::{ButtonState, InputState, Modifiers, MouseButton, ObjectRegistry, ScrollDelta};

/// Modifier flag bits matching the JS `MouseEvent.buttons` / modifier booleans.
pub const MOD_SHIFT: u32 = 1;
pub const MOD_CTRL: u32 = 2;
pub const MOD_ALT: u32 = 4;
pub const MOD_META: u32 = 8;

/// Convert a JS modifier bitmask to `Modifiers`.
pub fn modifiers_from_bits(bits: u32) -> Modifiers {
    Modifiers {
        shift: bits & MOD_SHIFT != 0,
        ctrl: bits & MOD_CTRL != 0,
        alt: bits & MOD_ALT != 0,
        logo: bits & MOD_META != 0,
    }
}

/// Convert a JS mouse button index (0=left, 1=middle, 2=right) to `MouseButton`.
pub fn mouse_button_from_js(button: u32) -> MouseButton {
    match button {
        0 => MouseButton::Left,
        1 => MouseButton::Middle,
        2 => MouseButton::Right,
        n => MouseButton::Other(n as u16),
    }
}

/// Apply a mouse-down event to the input state.
pub fn handle_mouse_down(input: &mut InputState, x: f32, y: f32, button: u32, mods: u32) {
    input.handle_modifiers(modifiers_from_bits(mods));
    input.handle_mouse_motion((x as f64, y as f64));
    input.handle_mouse_button(ButtonState::Pressed, mouse_button_from_js(button));
}

/// Apply a mouse-move event to the input state.
pub fn handle_mouse_move(input: &mut InputState, x: f32, y: f32, mods: u32) {
    input.handle_modifiers(modifiers_from_bits(mods));
    input.handle_mouse_motion((x as f64, y as f64));
}

/// Updates a captured drag while hiding affordances outside the logical viewport.
/// Pointer capture can deliver motion outside the canvas without a leave event.
pub fn handle_manipulation_motion(
    manipulation: &mut ViewportManipulation,
    registry: &mut ObjectRegistry,
    point: [f32; 2],
    viewport: [f32; 2],
) {
    manipulation.motion(point, registry);
    if !(0.0..viewport[0]).contains(&point[0]) || !(0.0..viewport[1]).contains(&point[1]) {
        manipulation.leave();
    }
}

/// Apply a mouse-up event to the input state.
pub fn handle_mouse_up(input: &mut InputState, x: f32, y: f32, button: u32) {
    input.handle_mouse_motion((x as f64, y as f64));
    input.handle_mouse_button(ButtonState::Released, mouse_button_from_js(button));
}

/// Apply a wheel event to the input state.
pub fn handle_wheel(input: &mut InputState, delta_y: f32, mods: u32) {
    input.handle_modifiers(modifiers_from_bits(mods));
    // Normalize: browsers typically send delta in pixels; treat as line-based for consistency.
    input.handle_scroll(ScrollDelta::LineDelta(0.0, -delta_y / 100.0));
}

#[cfg(test)]
mod tests {
    use super::*;
    use patinae_mol::AtomIndex;
    use patinae_scene::manipulation::{label_target, ViewportBounds, ViewportCursor};
    use patinae_scene::{AtomAnchor, LabelEntity, LabelObject};

    #[test]
    fn captured_pointer_outside_canvas_hides_highlights_until_reentry() {
        let mut registry = ObjectRegistry::new();
        registry.add(LabelObject::with_entities(
            "labels",
            vec![LabelEntity::new(AtomAnchor::new("mol", AtomIndex(0)), "C")],
        ));
        let target = label_target(
            &registry,
            registry.render_id("labels").unwrap(),
            0,
            ViewportBounds {
                x: 40.0,
                y: 40.0,
                width: 10.0,
                height: 10.0,
            },
            0,
        )
        .unwrap();
        let mut manipulation = ViewportManipulation::default();
        manipulation.set_targets(vec![target]);
        manipulation.set_modifiers(modifiers_from_bits(MOD_CTRL | MOD_SHIFT), false);
        manipulation.press([45.0, 45.0], &registry);
        handle_manipulation_motion(&mut manipulation, &mut registry, [-1.0, 45.0], [100.0; 2]);
        assert!(!manipulation.feedback().highlights_visible);
        assert_eq!(manipulation.feedback().cursor, ViewportCursor::Grabbing);
        assert_eq!(
            registry.get_label("labels").unwrap().entities()[0].label_offset(),
            [-46.0, 0.0]
        );
        handle_manipulation_motion(&mut manipulation, &mut registry, [30.0, 45.0], [100.0; 2]);
        assert!(manipulation.feedback().highlights_visible);
        assert_eq!(manipulation.active_target(), Some(target.id));
        assert!(manipulation.release());
        handle_manipulation_motion(&mut manipulation, &mut registry, [100.0, 45.0], [100.0; 2]);
        assert!(!manipulation.feedback().highlights_visible);
        assert_eq!(
            registry.get_label("labels").unwrap().entities()[0].label_offset(),
            [-15.0, 0.0]
        );
    }
}
