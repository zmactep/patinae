//! Viewport bridge: FPS tracking, texture push, frame dimensions.

use patinae_scene::manipulation::{
    label_target, ViewportBounds, ViewportManipulation, ViewportTarget,
};
use patinae_scene::{
    ButtonState, InputState, Modifiers, MouseButton, ObjectRegistry, RenderObjectId,
};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;
use std::time::Instant;

use slint::{ComponentHandle, Model, ModelRc, VecModel};

use crate::{AppWindow, ViewportLabel, ViewportState};

type LabelSizes = HashMap<(String, i32), [f32; 2]>;

#[derive(Debug, Clone, Copy)]
pub enum ViewportPointerKind {
    Press,
    Move,
    Release,
    Cancel,
}

#[derive(Debug, Clone, Copy)]
pub struct ViewportPointerEvent {
    pub kind: ViewportPointerKind,
    pub point: [f32; 2],
    pub modifiers: Modifiers,
}

/// Click suppression belongs to the latest release, camera reservation to the
/// latest gesture. They may differ when several gestures arrive between frames.
#[derive(Debug, Default)]
pub struct PointerDispatch {
    camera_reserved: Option<bool>,
    latest_press: Option<[f32; 2]>,
    pub suppress_click: bool,
}

impl PointerDispatch {
    pub fn reconcile_camera_input(&self, input: &mut InputState, scale: f32) {
        if let Some(point) = self.latest_press {
            // A frame snapshot cannot see up/down between two held states.
            // Rebase its delta to the latest press, excluding the previous drag.
            let position = input.mouse_position();
            let left_pressed = input.left_pressed();
            input.handle_mouse_motion(((point[0] * scale) as f64, (point[1] * scale) as f64));
            input.handle_mouse_button(ButtonState::Pressed, MouseButton::Left);
            input.handle_mouse_motion((position.0 as f64, position.1 as f64));
            if !left_pressed {
                input.handle_mouse_button(ButtonState::Released, MouseButton::Left);
            }
        }
        if let Some(reserved) = self.camera_reserved {
            // A new left gesture cannot release another held button's reservation.
            if reserved || (!input.middle_pressed() && !input.right_pressed()) {
                input.set_pointer_gesture_reserved(reserved);
            }
        }
    }
}

/// Tracks frame dimensions and FPS, pushes rendered images to Slint.
pub struct ViewportBridge {
    pub width: u32,
    pub height: u32,
    frame_count: u64,
    last_fps_time: Instant,
    labels: Rc<VecModel<ViewportLabel>>,
    labels_fingerprint: Option<u64>,
    label_sizes: Rc<RefCell<LabelSizes>>,
    pointer_events: Rc<RefCell<Vec<ViewportPointerEvent>>>,
}

impl ViewportBridge {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            frame_count: 0,
            last_fps_time: Instant::now(),
            labels: Rc::new(VecModel::default()),
            labels_fingerprint: None,
            label_sizes: Rc::default(),
            pointer_events: Rc::default(),
        }
    }

    /// Attaches the persistent annotation label model to Slint.
    pub fn attach(&self, window: &AppWindow, modifiers: Rc<Cell<(bool, bool, bool, bool)>>) {
        let sizes = self.label_sizes.clone();
        window
            .global::<ViewportState>()
            .on_label_size(move |owner, entity, width, height| {
                sizes
                    .borrow_mut()
                    .insert((owner.to_string(), entity), [width, height]);
            });
        let events = self.pointer_events.clone();
        window
            .global::<ViewportState>()
            .on_manipulation_pointer(move |kind, x, y| {
                // Slint maps macOS Cmd to control and physical Ctrl to meta.
                // Snapshot raw winit modifiers for every event, including press,
                // before subsequent key releases can change the frame's state.
                let (shift, ctrl, alt, logo) = modifiers.get();
                events.borrow_mut().push(ViewportPointerEvent {
                    kind: match kind {
                        0 => ViewportPointerKind::Press,
                        1 => ViewportPointerKind::Move,
                        2 => ViewportPointerKind::Release,
                        _ => ViewportPointerKind::Cancel,
                    },
                    point: [x, y],
                    modifiers: Modifiers {
                        shift,
                        ctrl,
                        alt,
                        logo,
                    },
                });
            });

        window
            .global::<ViewportState>()
            .set_labels(ModelRc::from(self.labels.clone()));
    }

    /// Drains durable pointer events, including complete gestures between frames.
    pub fn take_pointer_events(&self) -> Vec<ViewportPointerEvent> {
        std::mem::take(&mut *self.pointer_events.borrow_mut())
    }

    /// Routes queued Slint events through the shared manipulation controller.
    pub fn dispatch_pointer_events(
        &self,
        manipulation: &mut ViewportManipulation,
        registry: &mut ObjectRegistry,
    ) -> PointerDispatch {
        let mut dispatch = PointerDispatch::default();
        for event in self.take_pointer_events() {
            manipulation.set_modifiers(event.modifiers, cfg!(target_os = "macos"));
            match event.kind {
                ViewportPointerKind::Press => {
                    dispatch.latest_press = Some(event.point);
                    dispatch.camera_reserved = Some(manipulation.press(event.point, registry));
                }
                ViewportPointerKind::Move | ViewportPointerKind::Release => {
                    if manipulation.is_reserved() {
                        dispatch.camera_reserved = Some(true);
                    }
                    manipulation.motion(event.point, registry);
                    if matches!(event.kind, ViewportPointerKind::Release) {
                        dispatch.suppress_click = manipulation.release();
                        if dispatch.suppress_click {
                            dispatch.camera_reserved = Some(true);
                        }
                    }
                }
                ViewportPointerKind::Cancel => {
                    manipulation.cancel();
                    dispatch.suppress_click = true;
                    dispatch.camera_reserved = Some(true);
                }
            }
        }
        dispatch
    }

    /// Uses the same text extents Slint rendered, without highlight padding.
    pub fn manipulation_targets(&self, registry: &ObjectRegistry) -> Vec<ViewportTarget> {
        let sizes = self.label_sizes.borrow();
        self.labels
            .iter()
            .filter_map(|label| {
                let [width, height] =
                    *sizes.get(&(label.owner_id.to_string(), label.insertion_ordinal))?;
                let owner = RenderObjectId::new(label.owner_id.parse().ok()?)?;
                label_target(
                    registry,
                    owner,
                    label.insertion_ordinal.try_into().ok()?,
                    ViewportBounds {
                        x: label.x - width * label.anchor_x,
                        y: label.y - height * label.anchor_y,
                        width,
                        height,
                    },
                    label.display_order.parse().ok()?,
                )
            })
            .collect()
    }

    /// Update viewport dimensions.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width.max(1);
        self.height = height.max(1);
    }

    /// Push a rendered frame image and update FPS display in Slint.
    pub fn push_frame(&mut self, image: Option<slint::Image>, vp: &ViewportState) {
        if let Some(image) = image {
            vp.set_viewport_texture(image);
        }

        self.frame_count += 1;
        let elapsed = self.last_fps_time.elapsed();
        if elapsed.as_secs() >= 1 {
            let fps = self.frame_count as f64 / elapsed.as_secs_f64();
            vp.set_fps_text(format!("{:.0} fps", fps).into());
            vp.set_fps_value(fps as f32);
            self.frame_count = 0;
            self.last_fps_time = Instant::now();
        }
    }

    /// Replaces projected annotation labels shown above the viewport image.
    pub fn set_labels(&mut self, labels: Vec<ViewportLabel>) {
        let fingerprint = label_fingerprint(&labels);
        if self.labels_fingerprint == Some(fingerprint) {
            return;
        }
        let visible: HashSet<_> = labels
            .iter()
            .map(|label| (label.owner_id.as_str(), label.insertion_ordinal))
            .collect();
        self.label_sizes
            .borrow_mut()
            .retain(|(owner, entity), _| visible.contains(&(owner.as_str(), *entity)));
        self.labels_fingerprint = Some(fingerprint);
        update_label_rows(&self.labels, labels);
    }
}

fn update_label_rows(model: &VecModel<ViewportLabel>, labels: Vec<ViewportLabel>) {
    let previous_count = model.row_count();
    let next_count = labels.len();

    for (row, label) in labels.into_iter().enumerate() {
        if row < previous_count {
            model.set_row_data(row, label);
        } else {
            model.push(label);
        }
    }

    // Removing from the tail preserves the surviving Slint repeater instances.
    for row in (next_count..previous_count).rev() {
        model.remove(row);
    }
}

fn label_fingerprint(labels: &[ViewportLabel]) -> u64 {
    let mut hasher = DefaultHasher::new();
    labels.len().hash(&mut hasher);
    for label in labels {
        label.x.to_bits().hash(&mut hasher);
        label.y.to_bits().hash(&mut hasher);
        label.text.as_str().hash(&mut hasher);
        label.color.red().hash(&mut hasher);
        label.color.green().hash(&mut hasher);
        label.color.blue().hash(&mut hasher);
        label.color.alpha().hash(&mut hasher);
        label.size.to_bits().hash(&mut hasher);
        label.anchor_x.to_bits().hash(&mut hasher);
        label.anchor_y.to_bits().hash(&mut hasher);
        label.alignment.as_str().hash(&mut hasher);
        label.owner_id.as_str().hash(&mut hasher);
        label.insertion_ordinal.hash(&mut hasher);
        label.display_order.as_str().hash(&mut hasher);
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use slint::{Color, Model, SharedString, VecModel};

    use super::*;
    use patinae_scene::manipulation::ViewportCursor;

    #[test]
    fn queued_gestures_keep_click_suppression_separate_from_camera_reservation() {
        use patinae_scene::{ButtonState, CameraDelta, MouseButton};

        // A second gesture can be held or already released before the next frame.
        for (release_ordinary, scale) in [false, true]
            .into_iter()
            .flat_map(|released| [1.0_f32, 1.5, 2.0].map(|scale| (released, scale)))
        {
            let mut registry = ObjectRegistry::new();
            let bridge = ViewportBridge::new(800, 600);
            let mut manipulation = ViewportManipulation::default();
            let macos = cfg!(target_os = "macos");
            let chord = Modifiers {
                shift: true,
                ctrl: !macos,
                logo: macos,
                ..Modifiers::EMPTY
            };
            let mut input = InputState::new();
            input.handle_modifiers(chord);
            input.handle_mouse_button(ButtonState::Pressed, MouseButton::Left);
            assert!(input.take_camera_deltas().is_empty());

            // The snapshot still sees left down, so the intervening up/down is
            // available only in Slint's durable queue. Camera input is latched.
            input.handle_modifiers(Modifiers::EMPTY);
            input.handle_mouse_motion(((100.0 * scale) as f64, (100.0 * scale) as f64));
            input.handle_mouse_motion(((20.0 * scale) as f64, (15.0 * scale) as f64));
            bridge.pointer_events.borrow_mut().extend([
                ViewportPointerEvent {
                    kind: ViewportPointerKind::Press,
                    point: [100.0; 2],
                    modifiers: chord,
                },
                ViewportPointerEvent {
                    kind: ViewportPointerKind::Release,
                    point: [100.0; 2],
                    modifiers: chord,
                },
                ViewportPointerEvent {
                    kind: ViewportPointerKind::Press,
                    point: [10.0; 2],
                    modifiers: Modifiers::EMPTY,
                },
                ViewportPointerEvent {
                    kind: ViewportPointerKind::Move,
                    point: [20.0, 15.0],
                    modifiers: Modifiers::EMPTY,
                },
            ]);
            if release_ordinary {
                bridge
                    .pointer_events
                    .borrow_mut()
                    .push(ViewportPointerEvent {
                        kind: ViewportPointerKind::Release,
                        point: [20.0, 15.0],
                        modifiers: Modifiers::EMPTY,
                    });
                input.handle_mouse_button(ButtonState::Released, MouseButton::Left);
            }
            let dispatch = bridge.dispatch_pointer_events(&mut manipulation, &mut registry);
            dispatch.reconcile_camera_input(&mut input, scale);
            assert_eq!(dispatch.suppress_click, !release_ordinary);
            assert!(!input.manipulation_reserved());
            if !release_ordinary {
                assert!(
                    matches!(input.take_camera_deltas().as_slice(), [CameraDelta::Rotate { x, y }]
                    if *x == 5.0 * scale * input.rotate_sensitivity && *y == 10.0 * scale * input.rotate_sensitivity)
                );
                input.handle_mouse_motion(((30.0 * scale) as f64, (20.0 * scale) as f64));
                assert!(matches!(
                    input.take_camera_deltas().as_slice(),
                    [CameraDelta::Rotate { .. }]
                ));
            }
        }
    }

    #[test]
    fn slint_pointer_drag_uses_event_time_physical_modifiers() {
        use super::*;
        use crate::LayoutState;
        use patinae_mol::AtomIndex;
        use patinae_scene::{AtomAnchor, LabelEntity, LabelObject};
        use slint::platform::{Key, PointerEventButton, WindowEvent};

        i_slint_backend_testing::init_no_event_loop();
        for scale in [1.0, 1.5, 2.0] {
            let window = AppWindow::new().unwrap();
            window
                .window()
                .dispatch_event(WindowEvent::ScaleFactorChanged {
                    scale_factor: scale,
                });
            window
                .window()
                .set_size(slint::LogicalSize::new(800.0, 600.0));
            let layout = window.global::<LayoutState>();
            layout.set_empty_mode(false);
            layout.set_objects_visible(false);
            layout.set_selections_visible(false);
            layout.set_recent_atoms_visible(false);
            layout.set_repl_visible(false);

            let mut registry = ObjectRegistry::new();
            registry.add(LabelObject::with_entities(
                "labels",
                vec![
                    LabelEntity::new(AtomAnchor::new("mol", AtomIndex(0)), "drag me"),
                    LabelEntity::new(AtomAnchor::new("mol", AtomIndex(1)), "stay"),
                ],
            ));
            let macos = cfg!(target_os = "macos");
            let raw_modifiers = Rc::new(Cell::new((true, !macos, false, macos)));
            let mut bridge = super::ViewportBridge::new(800, 600);
            bridge.attach(&window, raw_modifiers.clone());
            let mut projected = label(100.0, "drag me");
            projected.y = 100.0;
            projected.owner_id = registry
                .render_id("labels")
                .unwrap()
                .get()
                .to_string()
                .into();
            bridge.set_labels(vec![projected]);
            window.show().unwrap();
            i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_secs(1));

            // Locate the viewport through its real pointer callback. The
            // headless menu occupies window space unlike the macOS menu bar.
            let probe = slint::LogicalPosition::new(150.0, 150.0);
            window
                .window()
                .dispatch_event(WindowEvent::PointerMoved { position: probe });
            window.window().dispatch_event(WindowEvent::PointerPressed {
                position: probe,
                button: PointerEventButton::Left,
            });
            let vp = window.global::<ViewportState>();
            assert!(vp.get_left_pressed());
            let origin =
                slint::LogicalPosition::new(probe.x - vp.get_press_x(), probe.y - vp.get_press_y());
            window
                .window()
                .dispatch_event(WindowEvent::PointerReleased {
                    position: probe,
                    button: PointerEventButton::Left,
                });
            bridge.take_pointer_events();

            // Winit maps physical Cmd to Slint Control on macOS. Windows/Linux
            // physical Ctrl also arrives as Slint Control, but raw bits differ.
            for key in [Key::Control, Key::Shift] {
                window
                    .window()
                    .dispatch_event(WindowEvent::KeyPressed { text: key.into() });
            }
            let start = slint::LogicalPosition::new(origin.x + 100.0, origin.y + 100.0);
            let end = slint::LogicalPosition::new(start.x + 40.0, start.y - 20.0);
            window
                .window()
                .dispatch_event(WindowEvent::PointerMoved { position: start });
            let mut manipulation = ViewportManipulation::default();
            let targets = bridge.manipulation_targets(&registry);
            assert_eq!(
                targets.len(),
                1,
                "Slint must report actual rendered text extents"
            );
            manipulation.set_targets(targets);
            // Native passive hover uses raw winit coordinates; Slint's moved
            // callback runs only while its TouchArea holds pointer capture.
            manipulation.set_modifiers(
                Modifiers {
                    shift: true,
                    ctrl: !macos,
                    logo: macos,
                    ..Modifiers::EMPTY
                },
                macos,
            );
            manipulation.motion([100.0, 100.0], &mut registry);
            assert!(manipulation.active_target().is_some());

            window.window().dispatch_event(WindowEvent::PointerPressed {
                position: start,
                button: PointerEventButton::Left,
            });
            window
                .window()
                .dispatch_event(WindowEvent::PointerMoved { position: end });
            window
                .window()
                .dispatch_event(WindowEvent::PointerReleased {
                    position: end,
                    button: PointerEventButton::Left,
                });
            assert!(window.global::<ViewportState>().get_control_held());
            assert!(!window.global::<ViewportState>().get_meta_held());
            // The frame may run after the user has already released the keys.
            raw_modifiers.set((false, false, false, false));
            for key in [Key::Shift, Key::Control] {
                window
                    .window()
                    .dispatch_event(WindowEvent::KeyReleased { text: key.into() });
            }
            assert!(
                bridge
                    .dispatch_pointer_events(&mut manipulation, &mut registry)
                    .suppress_click
            );
            let entities = registry.get_label("labels").unwrap().entities();
            assert_eq!(entities[0].label_offset(), [40.0, -20.0], "scale={scale}");
            assert_eq!(entities[1].label_offset(), [0.0; 2]);
            assert_ne!(manipulation.feedback().cursor, ViewportCursor::Grabbing);
            window
                .window()
                .dispatch_event(WindowEvent::PointerMoved { position: start });
            bridge.dispatch_pointer_events(&mut manipulation, &mut registry);
            assert_eq!(
                registry.get_label("labels").unwrap().entities()[0].label_offset(),
                [40.0, -20.0]
            );

            // The non-primary modifier (physical Ctrl on macOS) must still
            // allow a short Shift-click through the real TouchArea and bridge.
            let mut input_bridge = crate::bridges::input::InputBridge::new();
            let mut input = InputState::new();
            input_bridge.sync(&mut input, &vp, scale, raw_modifiers.get(), None);
            raw_modifiers.set((true, macos, false, !macos));
            for key in [Key::Meta, Key::Shift] {
                window
                    .window()
                    .dispatch_event(WindowEvent::KeyPressed { text: key.into() });
            }
            window.window().dispatch_event(WindowEvent::PointerPressed {
                position: start,
                button: PointerEventButton::Left,
            });
            window
                .window()
                .dispatch_event(WindowEvent::PointerReleased {
                    position: start,
                    button: PointerEventButton::Left,
                });
            raw_modifiers.set((false, false, false, false));
            for key in [Key::Shift, Key::Meta] {
                window
                    .window()
                    .dispatch_event(WindowEvent::KeyReleased { text: key.into() });
            }
            input_bridge.sync(&mut input, &vp, scale, raw_modifiers.get(), None);
            assert!(
                input_bridge
                    .take_pending_click()
                    .expect("non-primary Shift-click must qualify")
                    .ctrl_or_cmd
            );
            assert!(
                !bridge
                    .dispatch_pointer_events(&mut manipulation, &mut registry)
                    .suppress_click
            );
            window.hide().unwrap();
        }
    }

    fn label(x: f32, text: &str) -> ViewportLabel {
        ViewportLabel {
            x,
            y: 20.0,
            text: SharedString::from(text),
            color: Color::from_rgb_u8(34, 211, 238),
            size: 14.0,
            anchor_x: 0.5,
            anchor_y: 0.5,
            alignment: SharedString::from("center"),
            owner_id: SharedString::from("1"),
            insertion_ordinal: 0,
            display_order: SharedString::from("0"),
        }
    }

    #[test]
    fn label_rows_are_updated_without_replacing_the_model() {
        let model = VecModel::from(vec![label(10.0, "old"), label(20.0, "remove")]);

        update_label_rows(&model, vec![label(30.0, "updated")]);

        assert_eq!(model.row_count(), 1);
        let updated = model.row_data(0).expect("updated label row");
        assert_eq!(updated.x, 30.0);
        assert_eq!(updated.text.as_str(), "updated");

        update_label_rows(&model, vec![label(40.0, "first"), label(50.0, "added")]);

        assert_eq!(model.row_count(), 2);
        assert_eq!(model.row_data(0).unwrap().text.as_str(), "first");
        assert_eq!(model.row_data(1).unwrap().text.as_str(), "added");
    }
}
