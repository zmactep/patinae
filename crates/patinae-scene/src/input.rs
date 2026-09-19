//! Input handling for camera control
//!
//! Handles mouse and keyboard input for interactive camera manipulation.

use lin_alg::f32::Vec3;

// ---------------------------------------------------------------------------
// Framework-agnostic input types
// ---------------------------------------------------------------------------

/// Whether a button was pressed or released.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonState {
    Pressed,
    Released,
}

/// Logical mouse button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Other(u16),
}

/// Scroll wheel / trackpad delta.
#[derive(Debug, Clone, Copy)]
pub enum ScrollDelta {
    /// Line-based scroll (typical mouse wheel). `(horizontal, vertical)`.
    LineDelta(f32, f32),
    /// Pixel-based scroll (trackpad). `(x, y)` in physical pixels.
    PixelDelta(f64, f64),
}

/// Modifier key flags.
#[derive(Debug, Clone, Copy, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    /// ⌘ on macOS, ⊞ on Windows.
    pub logo: bool,
}

impl Modifiers {
    pub const EMPTY: Self = Self {
        shift: false,
        ctrl: false,
        alt: false,
        logo: false,
    };
    pub const SHIFT: Self = Self {
        shift: true,
        ctrl: false,
        alt: false,
        logo: false,
    };
    pub const CONTROL: Self = Self {
        shift: false,
        ctrl: true,
        alt: false,
        logo: false,
    };
    pub const SUPER: Self = Self {
        shift: false,
        ctrl: false,
        alt: false,
        logo: true,
    };
}

/// Camera movement delta
#[derive(Debug, Clone)]
pub enum CameraDelta {
    /// Rotation around axes (in radians)
    Rotate { x: f32, y: f32 },
    /// Translation in view space
    Translate(Vec3),
    /// Zoom factor (positive = zoom in)
    Zoom(f32),
    /// Clipping plane adjustment
    Clip { front: f32, back: f32 },
    /// Slab scale factor (raw scroll delta; consumer applies mouse_wheel_scale)
    SlabScale(f32),
}

/// Mouse button indices
const LEFT: usize = 0;
const RIGHT: usize = 1;
const MIDDLE: usize = 2;

/// Input state for camera control
///
/// Tracks mouse position, button state, and modifier keys to generate
/// camera movement deltas.
#[derive(Debug, Clone)]
pub struct InputState {
    /// Mouse button states (left, right, middle)
    mouse_buttons: [bool; 3],
    /// Current mouse position (physical pixels)
    mouse_pos: (f32, f32),
    /// Previous mouse position (for delta calculation)
    prev_mouse_pos: (f32, f32),
    /// Accumulated mouse delta since last update
    mouse_delta: (f32, f32),
    /// Accumulated scroll delta since last update
    scroll_delta: f32,
    /// Accumulated pinch-zoom delta since last update
    pinch_zoom_delta: f32,
    /// Current modifier keys
    modifiers: Modifiers,
    /// Rotation sensitivity (radians per pixel)
    pub rotate_sensitivity: f32,
    /// Zoom sensitivity (factor per scroll unit)
    pub zoom_sensitivity: f32,
    /// Clip sensitivity (units per pixel)
    pub clip_sensitivity: f32,
    /// Pan sensitivity multiplier (dimensionless; 1.0 = default behaviour).
    /// Applied on top of screen_vertex_scale, which already accounts for camera
    /// distance and viewport size so that the scene tracks the cursor exactly.
    pub pan_sensitivity: f32,
}

impl Default for InputState {
    fn default() -> Self {
        Self {
            mouse_buttons: [false; 3],
            mouse_pos: (0.0, 0.0),
            prev_mouse_pos: (0.0, 0.0),
            mouse_delta: (0.0, 0.0),
            scroll_delta: 0.0,
            pinch_zoom_delta: 0.0,
            modifiers: Modifiers::EMPTY,
            rotate_sensitivity: 0.005,
            zoom_sensitivity: 0.1,
            clip_sensitivity: 0.1,
            pan_sensitivity: 1.0,
        }
    }
}

impl InputState {
    /// Create a new input state with default settings
    pub fn new() -> Self {
        Self::default()
    }

    /// Handle a mouse button event
    pub fn handle_mouse_button(&mut self, state: ButtonState, button: MouseButton) {
        let pressed = state == ButtonState::Pressed;

        // Reset delta when starting a new drag to prevent accumulated
        // passive mouse movement from being applied
        if pressed {
            self.mouse_delta = (0.0, 0.0);
        }

        match button {
            MouseButton::Left => self.mouse_buttons[LEFT] = pressed,
            MouseButton::Right => self.mouse_buttons[RIGHT] = pressed,
            MouseButton::Middle => self.mouse_buttons[MIDDLE] = pressed,
            _ => {}
        }
    }

    /// Handle mouse movement
    pub fn handle_mouse_motion(&mut self, position: (f64, f64)) {
        let new_pos = (position.0 as f32, position.1 as f32);

        // Accumulate delta (only if we have a previous valid position)
        if self.mouse_pos != (0.0, 0.0) || self.prev_mouse_pos != (0.0, 0.0) {
            self.mouse_delta.0 += new_pos.0 - self.mouse_pos.0;
            self.mouse_delta.1 += new_pos.1 - self.mouse_pos.1;
        }

        self.prev_mouse_pos = self.mouse_pos;
        self.mouse_pos = new_pos;
    }

    /// Handle mouse scroll
    pub fn handle_scroll(&mut self, delta: ScrollDelta) {
        let scroll = match delta {
            ScrollDelta::LineDelta(_, y) => y,
            ScrollDelta::PixelDelta(_, y) => y as f32 / 100.0,
        };
        self.scroll_delta += scroll;
    }

    /// Handle trackpad pinch-zoom gesture
    pub fn handle_pinch_zoom(&mut self, delta: f64) {
        self.pinch_zoom_delta += delta as f32;
    }

    /// Handle modifier key changes
    pub fn handle_modifiers(&mut self, modifiers: Modifiers) {
        self.modifiers = modifiers;
    }

    /// Check if left mouse button is pressed
    pub fn left_pressed(&self) -> bool {
        self.mouse_buttons[LEFT]
    }

    /// Check if right mouse button is pressed
    pub fn right_pressed(&self) -> bool {
        self.mouse_buttons[RIGHT]
    }

    /// Check if middle mouse button is pressed
    pub fn middle_pressed(&self) -> bool {
        self.mouse_buttons[MIDDLE]
    }

    /// Check if shift is held
    pub fn shift_held(&self) -> bool {
        self.modifiers.shift
    }

    /// Check if ctrl is held
    pub fn ctrl_held(&self) -> bool {
        self.modifiers.ctrl
    }

    /// Check if ctrl or cmd (macOS) is held.
    ///
    /// On macOS, ⌘ maps to the `logo` modifier.
    /// We treat both as the "pan" modifier for Ctrl+drag panning.
    pub fn ctrl_or_cmd_held(&self) -> bool {
        self.modifiers.ctrl || self.modifiers.logo
    }

    /// Check if alt is held
    pub fn alt_held(&self) -> bool {
        self.modifiers.alt
    }

    /// Get current mouse position
    pub fn mouse_position(&self) -> (f32, f32) {
        self.mouse_pos
    }

    /// Process accumulated input and return camera deltas
    ///
    /// This consumes the accumulated deltas and returns the corresponding
    /// camera movements. Call this once per frame.
    ///
    /// The default mapping follows standard three-button viewing conventions:
    /// - Left drag:             Rotate (X/Y rotation)
    /// - Ctrl+Left drag:        Pan XY
    /// - Cmd+Left drag (macOS): Pan XY (⌘ maps to logo modifier)
    /// - Shift+Left drag:       Pan XY
    /// - Middle drag:           Pan XY
    /// - Right drag:            Zoom (Y)
    /// - Shift+Right drag:      Clip planes
    /// - Ctrl+Right drag:       Slab (both clip planes)
    /// - Scroll:                Slab scale (or Zoom while button held)
    pub fn take_camera_deltas(&mut self) -> Vec<CameraDelta> {
        let mut deltas = Vec::new();
        let (dx, dy) = self.mouse_delta;

        // Handle pinch-zoom gesture (trackpad)
        if self.pinch_zoom_delta.abs() > 0.001 {
            deltas.push(CameraDelta::Zoom(self.pinch_zoom_delta * 5.0));
        }

        // Handle scroll: zoom while dragging, slab scale otherwise
        if self.scroll_delta.abs() > 0.001 {
            if self.any_button_pressed() {
                deltas.push(CameraDelta::Zoom(self.scroll_delta * self.zoom_sensitivity));
            } else {
                deltas.push(CameraDelta::SlabScale(self.scroll_delta));
            }
        }

        // Handle mouse drag
        if dx.abs() > 0.001 || dy.abs() > 0.001 {
            if self.mouse_buttons[LEFT] {
                if self.modifiers.shift {
                    // Shift+Left: Pan (negate: moving origin opposite to drag direction)
                    deltas.push(CameraDelta::Translate(Vec3::new(-dx, dy, 0.0)));
                } else if self.ctrl_or_cmd_held() {
                    // Ctrl+Left / Cmd+Left (macOS): Pan XY
                    deltas.push(CameraDelta::Translate(Vec3::new(-dx, dy, 0.0)));
                } else {
                    // Left: Rotate
                    deltas.push(CameraDelta::Rotate {
                        x: dy * self.rotate_sensitivity,
                        y: dx * self.rotate_sensitivity,
                    });
                }
            } else if self.mouse_buttons[MIDDLE] {
                // Middle: Pan
                deltas.push(CameraDelta::Translate(Vec3::new(-dx, dy, 0.0)));
            } else if self.mouse_buttons[RIGHT] {
                if self.modifiers.shift {
                    // Shift+Right: Clip planes
                    deltas.push(CameraDelta::Clip {
                        front: -dy * self.clip_sensitivity,
                        back: dy * self.clip_sensitivity,
                    });
                } else if self.modifiers.ctrl {
                    // Ctrl+Right: Slab (move both clip planes together)
                    deltas.push(CameraDelta::Clip {
                        front: -dy * self.clip_sensitivity,
                        back: -dy * self.clip_sensitivity,
                    });
                } else {
                    // Right: Zoom
                    deltas.push(CameraDelta::Zoom(dy * self.zoom_sensitivity * 0.1));
                }
            }
        }

        // Reset accumulated deltas
        self.mouse_delta = (0.0, 0.0);
        self.scroll_delta = 0.0;
        self.pinch_zoom_delta = 0.0;

        deltas
    }

    /// Reset all state
    pub fn reset(&mut self) {
        self.mouse_buttons = [false; 3];
        self.mouse_delta = (0.0, 0.0);
        self.scroll_delta = 0.0;
        self.pinch_zoom_delta = 0.0;
        self.modifiers = Modifiers::EMPTY;
    }

    /// Check if any mouse button is pressed
    pub fn any_button_pressed(&self) -> bool {
        self.mouse_buttons.iter().any(|&b| b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_input_state_default() {
        let state = InputState::new();
        assert!(!state.left_pressed());
        assert!(!state.right_pressed());
        assert!(!state.middle_pressed());
    }

    #[test]
    fn test_mouse_button_handling() {
        let mut state = InputState::new();

        state.handle_mouse_button(ButtonState::Pressed, MouseButton::Left);
        assert!(state.left_pressed());

        state.handle_mouse_button(ButtonState::Released, MouseButton::Left);
        assert!(!state.left_pressed());
    }

    #[test]
    fn test_mouse_motion() {
        let mut state = InputState::new();

        state.handle_mouse_motion((100.0, 200.0));
        assert_eq!(state.mouse_position(), (100.0, 200.0));

        state.handle_mouse_motion((110.0, 205.0));
        assert_eq!(state.mouse_position(), (110.0, 205.0));
        assert!((state.mouse_delta.0 - 10.0).abs() < 0.001);
        assert!((state.mouse_delta.1 - 5.0).abs() < 0.001);
    }

    #[test]
    fn test_camera_deltas_rotation() {
        let mut state = InputState::new();

        // Press left button and move
        state.handle_mouse_button(ButtonState::Pressed, MouseButton::Left);
        state.handle_mouse_motion((100.0, 100.0));
        state.handle_mouse_motion((110.0, 105.0));

        let deltas = state.take_camera_deltas();
        assert_eq!(deltas.len(), 1);

        if let CameraDelta::Rotate { x, y } = &deltas[0] {
            assert!(x.abs() > 0.0);
            assert!(y.abs() > 0.0);
        } else {
            panic!("Expected Rotate delta");
        }
    }

    #[test]
    fn test_camera_deltas_scroll_slab_scale() {
        let mut state = InputState::new();

        // Scroll without any button pressed → SlabScale
        state.handle_scroll(ScrollDelta::LineDelta(0.0, 1.0));

        let deltas = state.take_camera_deltas();
        assert_eq!(deltas.len(), 1);

        if let CameraDelta::SlabScale(s) = &deltas[0] {
            assert!(*s > 0.0);
        } else {
            panic!("Expected SlabScale delta, got {:?}", deltas[0]);
        }
    }

    #[test]
    fn test_camera_deltas_scroll_while_dragging_zooms() {
        let mut state = InputState::new();

        // Press left button, then scroll → should produce Zoom
        state.handle_mouse_button(ButtonState::Pressed, MouseButton::Left);
        state.handle_scroll(ScrollDelta::LineDelta(0.0, 1.0));

        let deltas = state.take_camera_deltas();
        let has_zoom = deltas.iter().any(|d| matches!(d, CameraDelta::Zoom(_)));
        let has_slab = deltas
            .iter()
            .any(|d| matches!(d, CameraDelta::SlabScale(_)));
        assert!(has_zoom, "Scroll while dragging should produce Zoom");
        assert!(
            !has_slab,
            "Scroll while dragging should not produce SlabScale"
        );
    }

    #[test]
    fn test_ctrl_left_drag_pans() {
        let mut state = InputState::new();

        // Simulate Ctrl+Left drag
        state.handle_mouse_button(ButtonState::Pressed, MouseButton::Left);
        state.handle_modifiers(Modifiers::CONTROL);
        state.handle_mouse_motion((100.0, 100.0));
        state.handle_mouse_motion((110.0, 105.0)); // dx=10, dy=5

        let deltas = state.take_camera_deltas();

        assert_eq!(
            deltas.len(),
            1,
            "Ctrl+Left drag should produce exactly one delta"
        );
        match &deltas[0] {
            CameraDelta::Translate(v) => {
                assert!(
                    (v.x - (-10.0)).abs() < 0.001,
                    "Raw pixel dx=10 negated for origin shift"
                );
                assert!(
                    (v.y - 5.0).abs() < 0.001,
                    "Raw pixel dy=5 negated for origin shift"
                );
                assert!((v.z).abs() < 0.001, "No Z component");
            }
            other => panic!("Expected Translate, got {:?}", other),
        }
    }

    #[test]
    fn test_super_left_drag_pans_on_macos() {
        let mut state = InputState::new();

        state.handle_mouse_button(ButtonState::Pressed, MouseButton::Left);
        state.handle_modifiers(Modifiers::SUPER);
        state.handle_mouse_motion((100.0, 100.0));
        state.handle_mouse_motion((110.0, 105.0));

        let deltas = state.take_camera_deltas();

        assert!(
            deltas
                .iter()
                .any(|d| matches!(d, CameraDelta::Translate(_))),
            "Cmd+Left (SUPER) should pan on macOS"
        );
    }
}
