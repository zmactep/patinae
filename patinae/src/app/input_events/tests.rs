use super::*;
use patinae_mol::AtomIndex;
use patinae_scene::{AtomAnchor, LabelEntity, LabelObject};

#[test]
fn callbacks_preserve_annotation_release_endpoint_and_modifiers_at_hidpi() {
    i_slint_backend_testing::init_no_event_loop();
    for scale in [1.0, 1.5, 2.0] {
        let window = AppWindow::new().unwrap();
        window
            .window()
            .dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged {
                scale_factor: scale,
            });
        let host = Rc::new(RefCell::new(App::new()));
        let sender = host.borrow().events.clone();
        sender.bind(&host, &window);
        setup_callbacks(sender.clone(), &window);
        {
            let mut app = host.borrow_mut();
            app.kernel.session.registry.add(LabelObject::with_entities(
                "labels",
                vec![LabelEntity::new(
                    AtomAnchor::new("mol", AtomIndex(0)),
                    "drag me",
                )],
            ));
            let owner = app
                .kernel
                .session
                .registry
                .render_id("labels")
                .unwrap()
                .get()
                .to_string();
            app.viewport.set_labels(vec![ViewportLabel {
                x: 100.0,
                y: 100.0,
                text: "drag me".into(),
                size: 14.0,
                owner_id: owner.clone().into(),
                insertion_ordinal: 0,
                display_order: "0".into(),
                ..Default::default()
            }]);
            app.viewport.set_label_size(owner, 0, [80.0, 20.0]);
        }
        let vp = window.global::<ViewportState>();
        vp.invoke_pointer(1, 0, 110.0, 105.0, true, true, false, false, false);
        sender.drain();
        assert!(host
            .borrow()
            .interaction
            .manipulation
            .active_target()
            .is_some());
        vp.invoke_pointer(0, 0, 110.0, 105.0, true, true, false, false, false);
        vp.invoke_pointer(2, 0, 150.0, 85.0, true, true, false, false, false);
        // Keys can be released before the queued gesture is dispatched.
        sender.send(events::GuiEvent::Input(Event::Modifiers(Modifiers::EMPTY)));
        sender.drain();
        let app = host.borrow();
        assert_eq!(
            app.kernel
                .session
                .registry
                .get_label("labels")
                .unwrap()
                .entities()[0]
                .label_offset(),
            [40.0, -20.0],
            "scale={scale}"
        );
        assert!(!app.kernel.viewport.input.any_button_pressed());
        assert!(!app.interaction.manipulation.is_reserved());
        drop(app);
        // A duplicate native/toolkit release must not move the annotation again.
        vp.invoke_pointer(2, 0, 160.0, 80.0, true, true, false, false, false);
        sender.drain();
        assert_eq!(
            host.borrow()
                .kernel
                .session
                .registry
                .get_label("labels")
                .unwrap()
                .entities()[0]
                .label_offset(),
            [40.0, -20.0]
        );
    }
}

#[test]
fn several_camera_gestures_and_focus_loss_do_not_wait_for_frames() {
    i_slint_backend_testing::init_no_event_loop();
    let window = AppWindow::new().unwrap();
    let host = Rc::new(RefCell::new(App::new()));
    let sender = host.borrow().events.clone();
    sender.bind(&host, &window);
    setup_callbacks(sender.clone(), &window);
    let vp = window.global::<ViewportState>();
    for _ in 0..3 {
        vp.invoke_pointer(0, 0, 10.0, 10.0, false, false, false, false, false);
        vp.invoke_pointer(2, 0, 40.0, 25.0, false, false, false, false, false);
    }
    vp.invoke_pointer(0, 0, 10.0, 10.0, false, false, false, false, false);
    sender.send(events::GuiEvent::Input(Event::Focus(false)));
    sender.drain();
    assert!(!host.borrow().kernel.viewport.input.any_button_pressed());
    assert!(!vp.get_left_pressed());
}
