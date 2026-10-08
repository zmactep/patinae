//! Offscreen interaction regressions for the real Slint component tree.

use std::{cell::Cell, rc::Rc, time::Duration};

use slint::{
    platform::{
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
        Platform, WindowAdapter, WindowEvent,
    },
    ComponentHandle, ModelRc, SharedPixelBuffer, VecModel,
};

use crate::{
    AppWindow, ComponentMetrics, DockRow, LayoutState, ObjectsState, SeqChainRow, SequenceState,
    Theme, TooltipState,
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
    sequence.on_row_hover(move |chain, x| {
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
    tooltip.invoke_show("A.THR 5".into(), 1500.0, 940.0, 945.0, true);
    assert!(tooltip.get_visible());
    render();
    tooltip.invoke_show("Delayed action".into(), 48.0, 0.0, 32.0, false);
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
    for dark in [false, true] {
        app.global::<Theme>().set_dark_mode(dark);
        app.window().set_size(slint::LogicalSize::new(800.0, 600.0));
        clock.set(clock.get() + Duration::from_secs(1));
        slint::platform::update_timers_and_animations();
        render();
        assert!(app.get_scene_width() >= app.global::<ComponentMetrics>().get_viewport_min_w());
        assert!(app.get_scene_height() >= app.global::<ComponentMetrics>().get_viewport_min_h());
    }
}
