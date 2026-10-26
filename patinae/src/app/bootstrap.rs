//! Bootstrap for the native application.

use super::*;

pub(crate) fn run(started: Instant) -> Result<(), Box<dyn std::error::Error>> {
    let wgpu_configuration = match patinae_wgpu_configuration() {
        Ok(configuration) => configuration,
        Err(err) => return abort_startup_for_renderer_error(err),
    };
    let selector = slint::BackendSelector::new().require_wgpu_30(wgpu_configuration);

    #[cfg(target_os = "macos")]
    let selector = selector.with_winit_window_attributes_hook(|attrs| {
        crate::macos::configure_titlebar_attributes(attrs)
    });

    if let Err(err) = selector.select() {
        return abort_startup_for_renderer_error(err);
    }

    let mut app_state = App::new();
    let discovery = patinae_plugin_host::PluginDiscovery::from_process_env();
    let config = discovery.load_config();
    let selected_theme = config
        .as_ref()
        .map(|config| config.theme)
        .unwrap_or_default();
    app_state.kernel.session.settings.ui.theme = selected_theme;
    app_state
        .theme
        .configure(discovery.config_path(), selected_theme);
    let repl_at_top = config
        .as_ref()
        .is_ok_and(|config| config.repl_position == patinae_settings::config::ReplPosition::Top);
    if let Err(error) = &config {
        log::error!("{error}");
        app_state.kernel.output.print_error(error.clone());
    }
    app_state.plugin_runtime.plugin_startup =
        PluginStartup::new(started).with_discovery(discovery.with_config(config));
    app_state.startup_actions = startup_actions(
        patinae_settings::paths::patinaerc_path(),
        std::env::args_os().skip(1).map(PathBuf::from).collect(),
    );
    let app = Rc::new(RefCell::new(app_state));

    let window = match AppWindow::new() {
        Ok(window) => window,
        Err(err) => return abort_startup_for_renderer_error(err),
    };

    let events = app.borrow().events.clone();
    events.bind(&app, &window);
    window.global::<LayoutState>().set_repl_at_top(repl_at_top);

    #[cfg(target_os = "macos")]
    let display_link_heartbeat: Rc<
        RefCell<Option<crate::cv_display_link::DisplayLinkHeartbeat>>,
    > = Rc::new(RefCell::new(None));

    {
        let sender = events.clone();
        window.window().on_close_requested(move || {
            sender.send(events::GuiEvent::Close);
            slint::CloseRequestResponse::KeepWindowShown
        });
    }

    #[cfg(not(target_os = "macos"))]
    window.global::<LayoutState>().set_mod_key("Ctrl".into());

    {
        let mut state = app.borrow_mut();
        let App { kernel, theme, .. } = &mut *state;
        crate::bridges::theme::sync_theme(kernel, &window.global::<Theme>(), &window, theme);
        state.platform.sync(&window);
    }

    // Push fixed solid-color swatches once (theme-independent)
    crate::bridges::theme::push_solid_swatches(&window.global::<Theme>());
    app.borrow().file_actions.recent_files.attach(&window);

    super::window_events::install(events.clone(), &window);
    super::input_events::setup_callbacks(events.clone(), &window);

    // Attach viewport and objects bridge models to Slint globals + wire callbacks
    app.borrow().viewport.attach(&window);
    app.borrow().objects.attach(&window);
    crate::bridges::objects::setup_callbacks(events.clone(), &window);

    // Attach sequence bridge + wire callbacks
    app.borrow().sequence.attach(&window);
    crate::bridges::sequence::setup_callbacks(events.clone(), &window);

    // Attach movie bridge + wire callbacks
    app.borrow().movie.attach(&window);
    crate::bridges::movie::setup_callbacks(events.clone(), &window);

    // Attach REPL bridge + wire callbacks
    app.borrow().repl.attach(&window);
    crate::bridges::repl::setup_callbacks(events.clone(), &window);

    // Attach plugin panel bridge + wire callbacks
    app.borrow().plugin_runtime.plugin_bridge.attach(&window);
    crate::bridges::plugins::setup_callbacks(events.clone(), &window);
    crate::bridges::plugin_list::setup_callbacks(events.clone(), &window);

    // Wire layout panel show/hide callbacks
    crate::bridges::layout::setup_callbacks(events.clone(), &window);

    // Wire native menu and fetch dialog callbacks
    crate::native_menu::setup_callbacks(events.clone(), &window);

    {
        let sender = events.clone();
        app.borrow().host_timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(16),
            move || {
                sender.send(events::GuiEvent::HostTick);
            },
        );
    }

    let app_rc = app.clone();
    let window_weak = window.as_weak();

    let timing_callbacks = std::env::var("PATINAE_TIMING").is_ok();
    let before_end: std::rc::Rc<std::cell::Cell<Option<std::time::Instant>>> =
        std::rc::Rc::new(std::cell::Cell::new(None));
    let before_end_clone = before_end.clone();
    #[cfg(target_os = "macos")]
    let display_link_heartbeat_for_notifier = display_link_heartbeat.clone();
    window
        .window()
        .set_rendering_notifier(move |rs, api| match rs {
            slint::RenderingState::RenderingSetup => {
                if let Some(w) = window_weak.upgrade() {
                    app_rc.borrow_mut().setup_renderer(api, &w);
                    log_window_event(
                        w.window(),
                        "rendering-setup",
                        format_args!("viewport-scheduled"),
                    );
                    request_display_recovery_redraw(w.window());
                    #[cfg(target_os = "macos")]
                    start_display_link_heartbeat(
                        w.as_weak(),
                        display_link_heartbeat_for_notifier.clone(),
                    );
                }
            }
            slint::RenderingState::BeforeRendering => {
                if let Some(w) = window_weak.upgrade() {
                    app_rc.borrow_mut().render_frame(&w);
                    w.window().request_redraw();
                    if timing_callbacks {
                        before_end_clone.set(Some(std::time::Instant::now()));
                    }
                }
            }
            slint::RenderingState::AfterRendering => {
                let mut app = app_rc.borrow_mut();
                app.events.frame_rendered();
                app.plugin_runtime.plugin_startup.after_frame();
                if let Some(startup) = &mut app.rendering.renderer_startup {
                    startup.after_frame();
                }
                if let Some(t0) = before_end_clone.get().filter(|_| timing_callbacks) {
                    let dt = t0.elapsed().as_secs_f32() * 1000.0;
                    eprintln!("[patinae] slint scene-graph render: {dt:.2} ms");
                }
            }
            slint::RenderingState::RenderingTeardown => {
                #[cfg(target_os = "macos")]
                display_link_heartbeat_for_notifier.borrow_mut().take();
                app_rc.borrow_mut().teardown_renderer();
            }
            _ => {}
        })?;

    window.show()?;
    // IPC can hide the last window while tasks continue on the host timer.
    // User close and Quit still explicitly request event-loop termination.
    let result = slint::run_event_loop_until_quit();
    events.close();
    app.borrow().host_timer.stop();
    app.borrow_mut().plugin_runtime.plugin_startup.cancel();
    app.borrow_mut().rendering.renderer_startup = None;
    result?;
    Ok(())
}

#[cfg(target_os = "macos")]
pub(super) fn start_display_link_heartbeat(
    window: slint::Weak<AppWindow>,
    heartbeat: Rc<RefCell<Option<crate::cv_display_link::DisplayLinkHeartbeat>>>,
) {
    if heartbeat.borrow().is_some() {
        return;
    }

    match crate::cv_display_link::DisplayLinkHeartbeat::start(window) {
        Ok(link) => {
            *heartbeat.borrow_mut() = Some(link);
            log::info!("Started macOS CVDisplayLink render heartbeat");
        }
        Err(err) => {
            log::warn!("Failed to start macOS CVDisplayLink render heartbeat: {err}");
        }
    }
}

pub(super) fn abort_startup_for_renderer_error(
    err: impl std::fmt::Display,
) -> Result<(), Box<dyn std::error::Error>> {
    let message = startup_renderer_error_message(&err.to_string());
    startup_alert::show_error(STARTUP_RENDERER_ALERT_TITLE, &message);
    Err(std::io::Error::other(message).into())
}

pub(super) fn startup_renderer_error_message(reason: &str) -> String {
    format!(
        "Patinae could not start the hardware-accelerated WGPU renderer required \
         for the interactive viewport.\n\n{reason}\n\nEnable VM 3D acceleration, \
         install a Vulkan/DirectX 12-capable graphics driver, or run Patinae on \
         a machine with supported GPU acceleration. Patinae will now exit."
    )
}

#[cfg(any(not(target_os = "windows"), test))]
pub(super) fn no_hardware_wgpu_adapter_error(
    backends: slint::wgpu_30::wgpu::Backends,
    selected_cpu_adapter: Option<&slint::wgpu_30::wgpu::AdapterInfo>,
) -> std::io::Error {
    let selected = selected_cpu_adapter
        .map(|info| format!("; selected adapter was {} ({:?})", info.name, info.backend))
        .unwrap_or_default();
    std::io::Error::other(format!(
        "No hardware-accelerated WGPU adapter is available for {backends:?}{selected}. \
         CPU/WARP/llvmpipe software rendering is disabled."
    ))
}

#[cfg(any(target_os = "windows", test))]
pub(super) fn cpu_wgpu_adapter_error() -> std::io::Error {
    std::io::Error::other(
        "Software WGPU rendering was requested via SLINT_WGPU_CPU, but Patinae requires \
         a hardware-accelerated WGPU renderer.",
    )
}
