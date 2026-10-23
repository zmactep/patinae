use super::*;
use patinae_mol::AtomIndex;
use patinae_scene::{AtomAnchor, LabelEntity, LabelObject};

#[test]
fn theme_button_commands_and_restart_share_persisted_preference() {
    use patinae_settings::{config::read_config, ThemeMode};
    i_slint_backend_testing::init_no_event_loop();
    let window = AppWindow::new().unwrap();
    let mut app = App::new();
    let path = super::super::tests::temp_startup_path("theme-config");
    app.theme.configure(path.clone(), ThemeMode::System);
    for expected in [ThemeMode::Dark, ThemeMode::Light, ThemeMode::System] {
        dispatch(Event::ToggleTheme, &mut app, &window);
        app.process_commands(&window);
        assert_eq!(app.kernel.session.settings.ui.theme, expected);
        let config = read_config(&path).unwrap().unwrap();
        assert_eq!(config.theme, expected);
        let mut restarted = App::new();
        restarted.kernel.session.settings.ui.theme = config.theme;
        restarted.theme.configure(path.clone(), config.theme);
        assert_eq!(restarted.kernel.session.settings.ui.theme, expected);
    }
    app.kernel.bus.execute_command_silent("set theme, light");
    app.process_commands(&window);
    assert_eq!(read_config(&path).unwrap().unwrap().theme, ThemeMode::Light);
    app.kernel.bus.execute_command_silent("set theme, system");
    app.process_commands(&window);
    dispatch(Event::ThemeChanged, &mut app, &window);
    app.process_commands(&window);
    assert_eq!(
        read_config(&path).unwrap().unwrap().theme,
        ThemeMode::System
    );
    std::fs::write(&path, "invalid = [").unwrap();
    dispatch(Event::ToggleTheme, &mut app, &window);
    app.process_commands(&window);
    assert_eq!(app.kernel.session.settings.ui.theme, ThemeMode::Dark);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "invalid = [");
    let warnings = app
        .kernel
        .output
        .buffer
        .iter()
        .filter(|m| m.text.contains("could not be saved"))
        .count();
    assert_eq!(warnings, 1);
    app.process_commands(&window);
    assert_eq!(
        app.kernel
            .output
            .buffer
            .iter()
            .filter(|m| m.text.contains("could not be saved"))
            .count(),
        warnings
    );
    assert!(
        app.theme.persist(ThemeMode::Dark).is_ok(),
        "failed saves must not repeat every host tick"
    );
    std::fs::remove_file(path).unwrap();
}

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
