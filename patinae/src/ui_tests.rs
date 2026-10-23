//! Offscreen interaction regressions for the real Slint component tree.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

use slint::{
    platform::{
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
        Platform, WindowAdapter, WindowEvent,
    },
    ComponentHandle, Model, ModelRc, SharedPixelBuffer, VecModel,
};

use crate::{
    AppWindow, ComponentMetrics, DockRow, LayoutState, ObjectsState, PluginState,
    PluginToolbarItem, SeqChainRow, SequenceState, Spacing, Theme, TooltipState,
};

struct TestPlatform {
    window: Rc<MinimalSoftwareWindow>,
    clock: Rc<Cell<Duration>>,
}
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }
    fn duration_since_start(&self) -> Duration {
        self.clock.get()
    }
}

#[test]
fn actual_ui_hover_tooltip_and_responsive_bounds() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    let clock = Rc::new(Cell::new(Duration::ZERO));
    slint::platform::set_platform(Box::new(TestPlatform {
        window: window.clone(),
        clock: clock.clone(),
    }))
    .unwrap();
    let app = AppWindow::new().unwrap();
    app.window()
        .set_size(slint::LogicalSize::new(1512.0, 945.0));
    let layout = app.global::<LayoutState>();
    layout.set_empty_mode(false);
    layout.set_sequence_visible(true);
    let objects = app.global::<ObjectsState>();
    objects.set_dock_rows(ModelRc::from(Rc::new(VecModel::from(vec![
        DockRow {
            kind: "header".into(),
            key: "objects".into(),
            title: "OBJECTS".into(),
            count: 1,
            ..Default::default()
        },
        DockRow {
            kind: "object".into(),
            key: "object:long".into(),
            object: crate::ObjectItem {
                name: "long_structure_name_for_layout".into(),
                enabled: true,
                has_representations: true,
                ..Default::default()
            },
            ..Default::default()
        },
        DockRow {
            kind: "header".into(),
            key: "recent".into(),
            title: "RECENT ATOMS".into(),
            ..Default::default()
        },
        DockRow {
            kind: "empty".into(),
            key: "empty:recent".into(),
            title: "no recent atoms yet".into(),
            ..Default::default()
        },
    ]))));
    let sequence = app.global::<SequenceState>();
    sequence.set_has_data(true);
    sequence.set_max_row_width(250.0);
    sequence.set_footer_text("drag to add · Cmd+drag to exclude".into());
    let metrics = app.global::<ComponentMetrics>().get_sequence();
    let buffer = SharedPixelBuffer::<slint::Rgba8Pixel>::new(250, metrics.height as u32);
    sequence.set_chains(ModelRc::from(Rc::new(VecModel::from(vec![SeqChainRow {
        chain_id: "A".into(),
        object_name: "fixture".into(),
        is_first_in_object: true,
        polymer_length: 5,
        strip_width_px: 250.0,
        strip_image: slint::Image::from_rgba8(buffer),
        ..Default::default()
    }]))));
    let hovered = Rc::new(Cell::new(false));
    let seen = hovered.clone();
    sequence.on_pointer(move |_kind, chain, x, _sx, _sy, _exclude, _generation| {
        if chain == 0 && x >= 0.0 {
            seen.set(true);
        }
    });
    app.show().unwrap();
    let frame = Cell::new(0);
    let render = || {
        app.window().request_redraw();
        window.draw_if_needed(|renderer| {
            let size = window.size();
            let mut pixels = vec![
                slint::platform::software_renderer::Rgb565Pixel::default();
                (size.width * size.height) as usize
            ];
            renderer.render(&mut pixels, size.width as usize);
            if let Ok(directory) = std::env::var("PATINAE_UI_TEST_ARTIFACTS") {
                std::fs::create_dir_all(&directory).unwrap();
                let bytes: Vec<u8> = pixels
                    .into_iter()
                    .flat_map(|pixel| {
                        let rgb: slint::Rgb8Pixel = pixel.into();
                        [rgb.r, rgb.g, rgb.b]
                    })
                    .collect();
                image::save_buffer(
                    std::path::Path::new(&directory).join(format!("offscreen-{}.png", frame.get())),
                    &bytes,
                    size.width,
                    size.height,
                    image::ColorType::Rgb8,
                )
                .unwrap();
                frame.set(frame.get() + 1);
            }
        });
    };
    clock.set(Duration::from_secs(1));
    slint::platform::update_timers_and_animations();
    render();
    clock.set(Duration::from_secs(2));
    slint::platform::update_timers_and_animations();
    render();
    let y = app.get_scene_y()
        + app.get_scene_height()
        + app.global::<ComponentMetrics>().get_tab_bar_h()
        + app.global::<ComponentMetrics>().get_row_h()
        + 22.0
        + metrics.ruler
        + 2.0;
    app.window().dispatch_event(WindowEvent::PointerMoved {
        position: slint::LogicalPosition::new(app.get_scene_x() + 4.0 + metrics.offset + 5.0, y),
    });
    assert!(
        hovered.get(),
        "hover must reach the residue strip without a pressed button, scene={}, y={y}",
        app.get_scene_height()
    );
    let tooltip = app.global::<TooltipState>();
    tooltip.invoke_show("A.THR 5".into(), 1500.0, 1510.0, 940.0, 945.0, true);
    assert!(tooltip.get_visible());
    render();
    tooltip.invoke_show("Delayed action".into(), 16.0, 48.0, 0.0, 32.0, false);
    assert!(!tooltip.get_visible());
    render();
    slint::platform::update_timers_and_animations();
    clock.set(Duration::from_secs(3));
    slint::platform::update_timers_and_animations();
    render();
    assert!(
        tooltip.get_visible(),
        "delayed tooltip must become visible; pending={}, ticket={}, clock={:?}",
        tooltip.get_pending(),
        tooltip.get_ticket(),
        clock.get()
    );
    tooltip.invoke_dismiss();
    assert!(!tooltip.get_visible());
    let clicked = Rc::new(Cell::new(0));
    let seen_clicks = clicked.clone();
    let weak = app.as_weak();
    objects.on_row_btn_clicked(move |kind, source, name, chain, label, _, _, _| {
        seen_clicks.set(seen_clicks.get() + 1);
        let app = weak.unwrap();
        let state = app.global::<ObjectsState>();
        state.set_popover_kind(kind);
        state.set_popover_source(source);
        state.set_popover_obj_name(name);
        state.set_popover_chain_id(chain);
        state.set_popover_subchain_label(label);
    });
    let weak = app.as_weak();
    objects.on_popover_close(move || {
        weak.unwrap()
            .global::<ObjectsState>()
            .set_popover_kind("".into());
    });
    let geometry = app.global::<ComponentMetrics>();
    let row_y = app.get_scene_y() + geometry.get_section_header_h() + geometry.get_row_h() / 2.0;
    let rep_x = 1512.0 - geometry.get_row_actions_w()
        + geometry.get_row_btn_group_pad()
        + geometry.get_row_btn_w() / 2.0;
    let color_x = rep_x + geometry.get_row_btn_w() + geometry.get_row_btn_spacing();
    for (index, (x, kind)) in [(rep_x, "R"), (color_x, "C"), (rep_x, "R")]
        .into_iter()
        .enumerate()
    {
        let position = slint::LogicalPosition::new(x, row_y);
        app.window()
            .dispatch_event(WindowEvent::PointerMoved { position });
        render();
        clock.set(clock.get() + Duration::from_secs(1));
        slint::platform::update_timers_and_animations();
        render();
        app.window().dispatch_event(WindowEvent::PointerPressed {
            position,
            button: slint::platform::PointerEventButton::Left,
        });
        app.window().dispatch_event(WindowEvent::PointerReleased {
            position,
            button: slint::platform::PointerEventButton::Left,
        });
        render();
        clock.set(clock.get() + Duration::from_secs(1));
        slint::platform::update_timers_and_animations();
        render();
        assert_eq!(
            clicked.get(),
            index + 1,
            "each R/C click must reach its action"
        );
        assert_eq!(
            objects.get_popover_kind(),
            kind,
            "R/C must switch on the first click"
        );
    }
    objects.set_popover_kind("".into());
    tooltip.invoke_dismiss();

    // The pill's overflow menu must open beside its trigger, above the pill.
    objects.set_selected_count(1);
    objects.set_action_can_focus(true);
    objects.set_action_can_orient(true);
    objects.set_action_can_center(true);
    objects.set_action_can_toggle(true);
    objects.set_action_can_overflow(true);
    objects.set_overflow_menu_items(ModelRc::from(Rc::new(VecModel::from(
        ["Rename", "Copy", "Remove", "Color"]
            .into_iter()
            .map(|label| crate::OverflowMenuItem {
                action: label.to_lowercase().into(),
                label: label.into(),
                ..Default::default()
            })
            .collect::<Vec<_>>(),
    ))));
    let weak = app.as_weak();
    objects.on_action_overflow(move || {
        let app = weak.unwrap();
        let state = app.global::<ObjectsState>();
        state.set_popover_kind(if state.get_popover_kind() == "M" {
            "".into()
        } else {
            "M".into()
        });
    });
    let menu_action = Rc::new(RefCell::new(String::new()));
    let seen_action = menu_action.clone();
    objects.on_overflow_action(move |action| {
        *seen_action.borrow_mut() = action.to_string();
    });
    render();
    let pill_button = slint::LogicalPosition::new(
        1512.0 - 64.0,
        app.get_scene_y() + app.get_scene_height()
            - geometry.get_action_pill_bottom_gap()
            - geometry.get_action_pill_h() / 2.0,
    );
    for event in [
        WindowEvent::PointerMoved {
            position: pill_button,
        },
        WindowEvent::PointerPressed {
            position: pill_button,
            button: slint::platform::PointerEventButton::Left,
        },
        WindowEvent::PointerReleased {
            position: pill_button,
            button: slint::platform::PointerEventButton::Left,
        },
    ] {
        app.window().dispatch_event(event);
    }
    render();
    assert_eq!(objects.get_popover_kind(), "M");
    assert_eq!(objects.get_popover_source(), "pill");
    assert!((objects.get_popover_anchor_x() - pill_button.x).abs() < geometry.get_action_btn_h());
    assert_eq!(
        objects.get_popover_anchor_y(),
        pill_button.y - geometry.get_action_pill_h() / 2.0
    );
    for (width, height) in [(1512.0, 945.0), (800.0, 600.0), (1512.0, 945.0)] {
        app.window()
            .set_size(slint::LogicalSize::new(width, height));
        clock.set(clock.get() + Duration::from_secs(1));
        slint::platform::update_timers_and_animations();
        render();
        menu_action.borrow_mut().clear();
        let menu_position = slint::LogicalPosition::new(
            objects.get_popover_anchor_x() - geometry.get_overflow_menu_w() / 2.0,
            objects.get_popover_anchor_y() - app.global::<Spacing>().get_sm() - 18.0,
        );
        app.window().dispatch_event(WindowEvent::PointerMoved {
            position: menu_position,
        });
        app.window().dispatch_event(WindowEvent::PointerPressed {
            position: menu_position,
            button: slint::platform::PointerEventButton::Left,
        });
        app.window().dispatch_event(WindowEvent::PointerReleased {
            position: menu_position,
            button: slint::platform::PointerEventButton::Left,
        });
        assert_eq!(
            &*menu_action.borrow(),
            "color",
            "menu must stay above the pill after resizing to {width}x{height}"
        );
    }
    objects.set_popover_kind("".into());
    objects.set_selected_count(0);
    let spacing = app.global::<Spacing>();
    app.global::<PluginState>()
        .set_toolbar_items(ModelRc::from(Rc::new(VecModel::from(vec![
            PluginToolbarItem {
                id: "python_scripting".into(),
                title: "Python Scripting".into(),
                icon: "Py".into(),
                ..Default::default()
            },
        ]))));
    render();
    // Six sidebar buttons have five small gaps; the top and separator use three medium gaps.
    let plugin_position = slint::LogicalPosition::new(
        geometry.get_sidebar_w() / 2.0,
        app.get_scene_y()
            + 3.0 * spacing.get_md()
            + 5.0 * spacing.get_sm()
            + 6.5 * geometry.get_icon_size(),
    );
    app.window().dispatch_event(WindowEvent::PointerMoved {
        position: plugin_position,
    });
    render();
    clock.set(clock.get() + Duration::from_secs(1));
    slint::platform::update_timers_and_animations();
    render();
    assert_eq!(tooltip.get_text(), "Python Scripting");
    assert!(
        tooltip.get_visible(),
        "plugin tooltips must use the display title"
    );

    let toggle_position = slint::LogicalPosition::new(
        app.get_scene_x() + 40.0,
        app.get_scene_y()
            + app.get_scene_height()
            + geometry.get_tab_bar_h()
            + geometry.get_row_h() / 2.0,
    );
    app.window().dispatch_event(WindowEvent::PointerMoved {
        position: toggle_position,
    });
    render();
    clock.set(clock.get() + Duration::from_secs(1));
    slint::platform::update_timers_and_animations();
    render();
    assert!(
        !tooltip.get_visible(),
        "Viewer colors already has a visible label"
    );
    app.window().dispatch_event(WindowEvent::PointerPressed {
        position: toggle_position,
        button: slint::platform::PointerEventButton::Left,
    });
    app.window().dispatch_event(WindowEvent::PointerReleased {
        position: toggle_position,
        button: slint::platform::PointerEventButton::Left,
    });
    app.window().dispatch_event(WindowEvent::PointerMoved {
        position: slint::LogicalPosition::new(500.0, 300.0),
    });
    render();
    clock.set(clock.get() + Duration::from_secs(1));
    slint::platform::update_timers_and_animations();
    render();
    assert!(
        !tooltip.get_visible(),
        "clicking a toggle must not leave a focus tooltip"
    );

    // Exercise the sidebar through the production callbacks and derived dock model.
    let host = Rc::new(RefCell::new(crate::app::App::new()));
    let sender = host.borrow().events.clone();
    sender.bind(&host, &app);
    let theme_sender = sender.clone();
    app.global::<Theme>().on_toggle_theme(move || {
        theme_sender.send(crate::app::events::GuiEvent::Input(
            crate::app::input_events::Event::ToggleTheme,
        ));
    });
    let theme_position = slint::LogicalPosition::new(
        geometry.get_sidebar_w() / 2.0,
        app.window().size().height as f32 / app.window().scale_factor()
            - spacing.get_md()
            - spacing.get_sm()
            - 1.5 * geometry.get_icon_size(),
    );
    for (current, next, expected) in [
        ("System", "Dark", patinae_settings::ThemeMode::Dark),
        ("Dark", "Light", patinae_settings::ThemeMode::Light),
        ("Light", "System", patinae_settings::ThemeMode::System),
    ] {
        app.window().dispatch_event(WindowEvent::PointerMoved {
            position: theme_position,
        });
        render();
        clock.set(clock.get() + Duration::from_secs(1));
        slint::platform::update_timers_and_animations();
        render();
        assert_eq!(
            tooltip.get_text(),
            format!("Theme: {current}. Switch to {next}")
        );
        for event in [
            WindowEvent::PointerPressed {
                position: theme_position,
                button: slint::platform::PointerEventButton::Left,
            },
            WindowEvent::PointerReleased {
                position: theme_position,
                button: slint::platform::PointerEventButton::Left,
            },
        ] {
            app.window().dispatch_event(event);
        }
        sender.drain();
        let mut host = host.borrow_mut();
        assert_eq!(host.kernel.session.settings.ui.theme, expected);
        let crate::app::App { kernel, theme, .. } = &mut *host;
        crate::bridges::theme::sync_theme(kernel, &app.global::<Theme>(), &app, theme);
        drop(host);
        app.window().dispatch_event(WindowEvent::PointerMoved {
            position: slint::LogicalPosition::new(500.0, 300.0),
        });
        render();
    }
    crate::bridges::layout::setup_callbacks(sender.clone(), &app);
    host.borrow().objects.attach(&app);
    let sync_dock = || {
        sender.drain();
        let mut host = host.borrow_mut();
        let messages = host.kernel.process_messages(None, (800, 600));
        crate::bridges::layout::dispatch_messages(&messages, &app);
        let host = &mut *host;
        host.objects.sync(&mut host.kernel, &app);
    };
    sync_dock();
    render();
    let spacing = app.global::<Spacing>();
    for (index, section) in [(1, "objects"), (2, "selections"), (3, "recent")] {
        for visible in [false, true, false, true] {
            let position = slint::LogicalPosition::new(
                geometry.get_sidebar_w() / 2.0,
                app.get_scene_y()
                    + spacing.get_md()
                    + index as f32 * (geometry.get_icon_size() + spacing.get_sm())
                    + geometry.get_icon_size() / 2.0,
            );
            app.window()
                .dispatch_event(WindowEvent::PointerMoved { position });
            app.window().dispatch_event(WindowEvent::PointerPressed {
                position,
                button: slint::platform::PointerEventButton::Left,
            });
            app.window().dispatch_event(WindowEvent::PointerReleased {
                position,
                button: slint::platform::PointerEventButton::Left,
            });
            sync_dock();
            render();
            assert_eq!(
                objects.get_dock_rows().iter().any(|row| row.key == section),
                visible,
                "sidebar toggle must change section {section} on every click"
            );
            assert_eq!(
                objects.get_dock_scroll_y(),
                0.0,
                "section toggles must not scroll a short dock out of view"
            );
        }
    }
    {
        let mut host = host.borrow_mut();
        host.kernel
            .execute_command("mset 1 x41", false, None, (800, 600))
            .unwrap();
        host.movie.attach(&app);
        let host = &mut *host;
        host.movie.sync(&host.kernel, &app, None);
    }
    layout.set_movie_visible(true);
    layout.set_bottom_active_tab(1);
    render();
    for dark in [false, true] {
        app.global::<Theme>().set_dark_mode(dark);
        app.window().set_size(slint::LogicalSize::new(800.0, 600.0));
        clock.set(clock.get() + Duration::from_secs(1));
        slint::platform::update_timers_and_animations();
        render();
        assert!(app.get_scene_width() >= app.global::<ComponentMetrics>().get_viewport_min_w());
        assert!(app.get_scene_height() >= app.global::<ComponentMetrics>().get_viewport_min_h());
    }

    // A release must commit the selection and its UI models without another
    // pointer move, host timer tick, or viewport rendering callback.
    {
        let mut host = host.borrow_mut();
        let mut molecule = patinae_mol::MoleculeBuilder::new("drag_fixture");
        for residue in 1..=10 {
            molecule = molecule.add_atom(
                patinae_mol::AtomBuilder::new()
                    .name("CA")
                    .element_symbol("C")
                    .resn("ALA")
                    .resv(residue)
                    .chain("A")
                    .flags(patinae_mol::AtomFlags::PROTEIN | patinae_mol::AtomFlags::POLYMER)
                    .build(),
                lin_alg::f32::Vec3::new(residue as f32, 0.0, 0.0),
            );
        }
        host.kernel
            .session
            .registry
            .add(patinae_scene::MoleculeObject::with_name(
                molecule.build(),
                "drag_fixture",
            ));
        let host = &mut *host;
        host.sequence.sync(&host.kernel, &app);
    }
    crate::bridges::sequence::setup_callbacks(sender.clone(), &app);
    layout.set_bottom_active_tab(0);
    app.window()
        .set_size(slint::LogicalSize::new(1512.0, 945.0));
    render();
    let y = app.get_scene_y()
        + app.get_scene_height()
        + geometry.get_tab_bar_h()
        + geometry.get_row_h()
        + 22.0
        + metrics.ruler
        + 2.0;
    let start = slint::LogicalPosition::new(
        app.get_scene_x() + spacing.get_xs() + metrics.offset + 5.0,
        y,
    );
    let end = slint::LogicalPosition::new(start.x + 50.0, y);
    let previous_highlight = sequence.get_chains().row_data(0).unwrap().highlight_image;
    app.window()
        .dispatch_event(WindowEvent::PointerMoved { position: start });
    app.window().dispatch_event(WindowEvent::PointerPressed {
        position: start,
        button: slint::platform::PointerEventButton::Left,
    });
    sender.drain();
    assert!(sequence.get_drag_active());
    // Release at a new position, with no intervening move event.
    app.window().dispatch_event(WindowEvent::PointerReleased {
        position: end,
        button: slint::platform::PointerEventButton::Left,
    });
    sender.drain();
    assert!(sequence.get_commit_pending());
    assert!(!host.borrow().kernel.session.selections.contains("sele"));
    sender.drain();
    assert!(!sequence.get_commit_pending());
    assert!(!sequence.get_drag_active());
    assert!(sequence.get_drag_end() > sequence.get_drag_start());
    assert!(host.borrow().kernel.session.selections.contains("sele"));
    assert!(objects.get_selections().row_count() > 0);
    assert_ne!(
        sequence.get_chains().row_data(0).unwrap().highlight_image,
        previous_highlight,
        "the selection band must replace the drag preview on release"
    );

    // Exercise the real input, drawer hit testing, and completion routing in both
    // placements, including a resized viewport with the bottom dock open.
    crate::bridges::repl::setup_callbacks(sender.clone(), &app);
    let repl = app.global::<crate::ReplState>();
    repl.set_output_items(ModelRc::new(VecModel::from(
        (0..40)
            .map(|index| crate::OutputItem {
                plain_text: format!("Output line {index}").into(),
                kind: "normal".into(),
                ..Default::default()
            })
            .collect::<Vec<_>>(),
    )));
    let accepted = Rc::new(Cell::new(-1));
    let seen = accepted.clone();
    let weak = app.as_weak();
    repl.on_completion_accept(move |index| {
        seen.set(index);
        weak.unwrap()
            .global::<crate::ReplState>()
            .set_completion_visible(false);
    });
    let click = |x, y| {
        let position = slint::LogicalPosition::new(x, y);
        app.window()
            .dispatch_event(WindowEvent::PointerMoved { position });
        app.window().dispatch_event(WindowEvent::PointerPressed {
            position,
            button: slint::platform::PointerEventButton::Left,
        });
        app.window().dispatch_event(WindowEvent::PointerReleased {
            position,
            button: slint::platform::PointerEventButton::Left,
        });
        sender.drain();
    };
    let settle = || {
        sender.drain();
        render();
        clock.set(clock.get() + Duration::from_secs(1));
        slint::platform::update_timers_and_animations();
        render();
    };
    for top in [false, true] {
        for (width, height, bottom_dock) in [(1512.0, 945.0, false), (800.0, 600.0, true)] {
            app.invoke_dismiss_repl();
            layout.set_repl_at_top(top);
            layout.set_sequence_visible(bottom_dock);
            layout.set_movie_visible(false);
            layout.set_repl_visible(true);
            objects.set_popover_kind("".into());
            app.window()
                .set_size(slint::LogicalSize::new(width, height));
            settle();
            let cx = app.get_scene_width() / 2.0;
            let pill_h = geometry.get_repl_pill_h();
            let edge = geometry.get_repl_pill_edge_gap();
            let pill_y = if top {
                edge
            } else {
                app.get_scene_height() - edge - pill_h
            };
            assert!(app.invoke_overlay_contains_point(cx, pill_y + pill_h / 2.0));
            let opposite_y = if top {
                app.get_scene_height() - edge - pill_h / 2.0
            } else {
                edge + pill_h / 2.0
            };
            assert!(!app.invoke_overlay_contains_point(cx, opposite_y));
            click(
                app.get_scene_x() + cx,
                app.get_scene_y() + pill_y + pill_h / 2.0,
            );
            assert!(
                repl.get_focused(),
                "input must be clickable at its configured edge"
            );
            repl.set_drawer_open(false);
            settle();
            let pill_w = geometry
                .get_repl_pill_w()
                .min(app.get_scene_width() - 2.0 * spacing.get_md());
            click(
                app.get_scene_x() + cx + pill_w / 2.0 - 18.0,
                app.get_scene_y() + pill_y + pill_h / 2.0,
            );
            assert!(repl.get_drawer_open(), "toggle must open the drawer");
            settle();
            let drawer_point = if top {
                pill_y + pill_h + 30.0
            } else {
                pill_y - 30.0
            };
            assert!(app.invoke_overlay_contains_point(cx, drawer_point));
            click(app.get_scene_x() + cx, app.get_scene_y() + drawer_point);
            assert!(
                repl.get_drawer_open(),
                "output clicks must not dismiss the drawer"
            );
            repl.set_completion_items(ModelRc::new(VecModel::from(vec![crate::CompletionItem {
                text: "help".into(),
                ..Default::default()
            }])));
            repl.set_completion_visible(true);
            accepted.set(-1);
            settle();
            let completion_point = if top {
                pill_y
                    + pill_h
                    + geometry.get_repl_drawer_gap()
                    + spacing.get_xs()
                    + geometry.get_completion_row_h() / 2.0
            } else {
                pill_y
                    - geometry.get_repl_drawer_gap()
                    - spacing.get_xs()
                    - geometry.get_completion_row_h() / 2.0
            };
            assert!(app.invoke_overlay_contains_point(cx, completion_point));
            click(app.get_scene_x() + cx, app.get_scene_y() + completion_point);
            assert_eq!(
                accepted.get(),
                0,
                "completion must receive clicks above the output layer"
            );
            click(app.get_scene_x() + 12.0, app.get_scene_y() + 100.0);
            assert!(!repl.get_drawer_open());
            assert!(!repl.get_focused());
        }
    }
    // Rename through real pointer/key events, production callbacks, and commands.
    crate::bridges::objects::setup_callbacks(sender.clone(), &app);
    app.invoke_dismiss_repl();
    objects.set_popover_kind("".into());
    app.window()
        .set_size(slint::LogicalSize::new(1512.0, 945.0));
    sync_dock();
    objects.set_dock_scroll_y(0.0);
    render();
    let key = |text: slint::SharedString| {
        app.window()
            .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        app.window()
            .dispatch_event(WindowEvent::KeyReleased { text });
    };
    for (kind, old_name, new_name) in [
        ("object", "drag_fixture", "renamed_object"),
        ("selection", "sele", "renamed_selection"),
    ] {
        let mut row_y = app.get_scene_y();
        let mut found = false;
        for row in objects.get_dock_rows().iter() {
            if row.kind == kind && (row.object.name == old_name || row.selection.name == old_name) {
                found = true;
                break;
            }
            row_y += if row.kind == "header" {
                geometry.get_section_header_h()
            } else {
                geometry.get_row_h()
            };
        }
        assert!(found, "rename fixture must be visible: {old_name}");
        let row_x = 1512.0 - 180.0;
        row_y += geometry.get_row_h() / 2.0;
        for cancel in [true, false] {
            clock.set(clock.get() + Duration::from_secs(1));
            slint::platform::update_timers_and_animations();
            click(row_x, row_y);
            render();
            assert_eq!(objects.get_name_input_action(), "");
            clock.set(clock.get() + Duration::from_millis(100));
            slint::platform::update_timers_and_animations();
            click(row_x, row_y);
            render();
            assert_eq!(objects.get_name_input_action(), "rename");
            assert_eq!(objects.get_name_input_target(), old_name);
            assert_eq!(objects.get_name_input_text(), old_name);
            for character in new_name.chars() {
                key(character.to_string().into());
            }
            assert_eq!(
                objects.get_name_input_text(),
                new_name,
                "the editor must receive focus and select the old name"
            );
            key(if cancel {
                slint::platform::Key::Escape.into()
            } else {
                slint::platform::Key::Return.into()
            });
            sync_dock();
            render();
            assert_eq!(objects.get_name_input_action(), "");
            let host = host.borrow();
            let contains = |name| {
                if kind == "object" {
                    host.kernel.session.registry.contains(name)
                } else {
                    host.kernel.session.selections.contains(name)
                }
            };
            assert_eq!(contains(old_name), cancel);
            assert_eq!(contains(new_name), !cancel);
        }
    }
}

#[test]
fn color_popover_reset_hover_and_click_preserve_scope() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    let clock = Rc::new(Cell::new(Duration::ZERO));
    slint::platform::set_platform(Box::new(TestPlatform {
        window: window.clone(),
        clock: clock.clone(),
    }))
    .unwrap();
    let app = AppWindow::new().unwrap();
    app.window()
        .set_size(slint::LogicalSize::new(1512.0, 945.0));
    app.global::<LayoutState>().set_empty_mode(false);
    let objects = app.global::<ObjectsState>();
    objects.set_popover_source("object".into());
    objects.set_popover_anchor_y(100.0);
    let geometry = app.global::<ComponentMetrics>();
    let tooltip = app.global::<TooltipState>();
    app.show().unwrap();
    let render = || {
        app.window().request_redraw();
        window.draw_if_needed(|renderer| {
            let size = window.size();
            let mut pixels = vec![
                slint::platform::software_renderer::Rgb565Pixel::default();
                (size.width * size.height) as usize
            ];
            renderer.render(&mut pixels, size.width as usize);
        });
    };
    clock.set(Duration::from_secs(1));
    slint::platform::update_timers_and_animations();
    render();
    objects.set_popover_kind("C".into());
    objects.set_popover_scope("surface".into());
    objects.set_popover_target("fixture and chain A and polymer".into());
    objects.set_popover_color_cartoon(true);
    let executed = Rc::new(RefCell::new(Vec::new()));
    let commands = executed.clone();
    let weak = app.as_weak();
    objects.on_popover_execute(move || {
        commands.borrow_mut().push(
            weak.unwrap()
                .global::<ObjectsState>()
                .get_popover_cmd_preview()
                .to_string(),
        );
    });
    render();
    let reset_command = "unset cartoon_color, fixture and chain A and polymer";
    // Locate the real reset hit area by its hover behavior, independent of tile layout.
    let popover_left = app.get_scene_x() + app.get_scene_width()
        - geometry.get_popover_w()
        - app.global::<Spacing>().get_sm();
    let mut reset_position = None;
    'find_reset: for y in (0..300).step_by(4) {
        for x in (0..geometry.get_popover_w() as usize).step_by(4) {
            let position = slint::LogicalPosition::new(popover_left + x as f32, y as f32);
            objects.set_popover_cmd_preview("".into());
            app.window()
                .dispatch_event(WindowEvent::PointerMoved { position });
            if objects.get_popover_cmd_preview() == reset_command {
                reset_position = Some(position);
                break 'find_reset;
            }
        }
    }
    let reset_position = reset_position.expect("cartoon reset must have a hoverable hit area");
    clock.set(clock.get() + Duration::from_secs(1));
    slint::platform::update_timers_and_animations();
    render();
    assert_eq!(tooltip.get_text(), reset_command);
    assert!(tooltip.get_visible());
    // Clicking must set its own command even if another preview was left behind.
    objects.set_popover_cmd_preview("color blue, fixture".into());
    app.window().dispatch_event(WindowEvent::PointerPressed {
        position: reset_position,
        button: slint::platform::PointerEventButton::Left,
    });
    app.window().dispatch_event(WindowEvent::PointerReleased {
        position: reset_position,
        button: slint::platform::PointerEventButton::Left,
    });
    assert_eq!(&*executed.borrow(), &[reset_command]);
    assert_eq!(objects.get_popover_scope(), "surface");
    assert_eq!(objects.get_popover_kind(), "C");
    objects.set_popover_color_cartoon(false);
}
