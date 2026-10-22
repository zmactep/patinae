use super::*;
use lin_alg::f32::Vec3;
use patinae_cmd::{AnnotationRequest, MeasurementRequest, MeasurementTarget};
use patinae_mol::{Atom, AtomIndex, Element, ObjectMolecule};
use patinae_render::{
    gib_to_bytes, mib_to_bytes, PERFORMANCE_MAX_BUFFER_SIZE,
    PERFORMANCE_MAX_STORAGE_BUFFER_BINDING_SIZE,
};
use patinae_scene::{KeyCode, Modifiers, MoleculeObject, ObjectType, PickHit};

fn adapter_limits(
    max_buffer_size: u64,
    max_storage_buffer_binding_size: u64,
) -> slint::wgpu_30::wgpu::Limits {
    slint::wgpu_30::wgpu::Limits {
        max_buffer_size,
        max_storage_buffer_binding_size,
        max_texture_dimension_1d: 16_384,
        max_texture_dimension_2d: 32_768,
        max_texture_dimension_3d: 2_048,
        ..slint::wgpu_30::wgpu::Limits::default()
    }
}

fn temp_startup_path(name: &str) -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should be after Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("patinae-{name}-{nonce}"))
}

fn write_startup_file(name: &str) -> PathBuf {
    let path = temp_startup_path(name);
    std::fs::write(&path, "# startup rc\n").expect("startup rc test file should be writable");
    path
}

fn app_with_atoms(atoms: impl IntoIterator<Item = Atom>) -> App {
    let mut app = App::new();
    let mut molecule = ObjectMolecule::new("mol");
    for atom in atoms {
        molecule.add_atom(atom);
    }
    app.kernel
        .session
        .registry
        .add(MoleculeObject::from_raw(molecule));
    app
}

#[test]
fn native_annotation_queue_executes_in_order_without_app_messages() {
    let mut app = app_with_atoms([
        Atom::new("A", Element::Carbon),
        Atom::new("B", Element::Carbon),
    ]);
    app.kernel
        .session
        .registry
        .get_molecule_mut("mol")
        .unwrap()
        .molecule_mut()
        .add_coord_set(patinae_mol::CoordSet::from_vec3(&[
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
        ]));
    for _ in 0..2 {
        app.queue_annotation_request(AnnotationRequest::Measurement(MeasurementRequest::new(
            ["name A", "name B"],
            MeasurementTarget::New,
        )));
    }

    assert_eq!(app.drain_annotation_requests(), 2);

    assert!(app
        .kernel
        .session
        .registry
        .get_measurement("distance01")
        .is_some());
    assert!(app
        .kernel
        .session
        .registry
        .get_measurement("distance02")
        .is_some());
    assert!(app.kernel.bus.drain_outbox().is_empty());
}

fn atom_hit(index: usize) -> PickHit {
    PickHit {
        instance: None,
        object_name: "mol".to_string(),
        object_type: ObjectType::Molecule,
        atom_index: Some(AtomIndex::from(index)),
        position: Vec3::new(0.0, 0.0, 0.0),
        distance: 1.0,
    }
}

fn drain_command_messages(app: &mut App) -> Vec<(String, bool)> {
    app.kernel
        .bus
        .drain_outbox()
        .into_iter()
        .filter_map(|message| match message {
            AppMessage::ExecuteCommand { command, silent } => Some((command, silent)),
            _ => None,
        })
        .collect()
}

fn drain_commands(app: &mut App) -> Vec<String> {
    drain_command_messages(app)
        .into_iter()
        .map(|(command, _)| command)
        .collect()
}

fn drain_warnings(app: &mut App) -> Vec<String> {
    app.kernel
        .bus
        .drain_outbox()
        .into_iter()
        .filter_map(|message| match message {
            AppMessage::PrintWarning(message) => Some(message),
            _ => None,
        })
        .collect()
}

#[test]
fn modified_atom_hit_queues_pick_then_unpick_without_named_selection() {
    let mut app = app_with_atoms([Atom::new("CA", Element::Carbon)]);
    let hit = atom_hit(0);
    let path = canonical_atom_path_for_hit(
        &hit,
        app.kernel
            .session
            .registry
            .get_molecule("mol")
            .unwrap()
            .molecule(),
    )
    .unwrap();

    app.apply_pick_result(Some(&hit), true);
    assert_eq!(
        drain_command_messages(&mut app),
        [(format!("pick {}", quote_command_arg(&path)), false)]
    );
    let limit = app.kernel.session.settings.behavior.recent_pick_limit();
    app.kernel.session.recent_atoms.insert(path.clone(), limit);
    app.apply_pick_result(Some(&hit), true);

    assert!(!app.kernel.session.selections.contains("sele"));
    assert_eq!(
        drain_command_messages(&mut app),
        [(format!("unpick {}", quote_command_arg(&path)), false)]
    );
}

#[test]
fn modified_atom_hit_quotes_a_reserved_model_name_via_the_slash_path() {
    let mut app = App::new();
    let mut molecule = ObjectMolecule::new("of");
    molecule.add_atom(Atom::new("CA", Element::Carbon));
    app.kernel
        .session
        .registry
        .add(MoleculeObject::from_raw(molecule));
    let hit = PickHit {
        instance: None,
        object_name: "of".to_string(),
        object_type: ObjectType::Molecule,
        atom_index: Some(AtomIndex::from(0usize)),
        position: Vec3::new(0.0, 0.0, 0.0),
        distance: 1.0,
    };
    let path = canonical_atom_path_for_hit(
        &hit,
        app.kernel
            .session
            .registry
            .get_molecule("of")
            .unwrap()
            .molecule(),
    )
    .unwrap();

    app.apply_pick_result(Some(&hit), true);

    assert_eq!(
        drain_command_messages(&mut app),
        [(format!("pick {}", quote_command_arg(&path)), false)]
    );
}

#[test]
fn plugin_scene_generation_tracks_recent_atom_mutations() {
    let mut app = app_with_atoms([Atom::new("CA", Element::Carbon)]);
    let before = app.plugin_scene_generation();
    let hit = atom_hit(0);
    let path = canonical_atom_path_for_hit(
        &hit,
        app.kernel
            .session
            .registry
            .get_molecule("mol")
            .unwrap()
            .molecule(),
    )
    .unwrap();
    let limit = app.kernel.session.settings.behavior.recent_pick_limit();

    app.kernel.session.recent_atoms.insert(path, limit);

    assert_ne!(app.plugin_scene_generation(), before);
}

#[test]
fn modified_miss_is_no_op_and_ordinary_click_semantics_remain() {
    let mut app = app_with_atoms([Atom::new("CA", Element::Carbon)]);
    app.apply_pick_result(Some(&atom_hit(0)), true);
    assert!(drain_commands(&mut app)[0].starts_with("pick \"/mol/"));

    app.apply_pick_result(None, true);
    assert!(app.kernel.session.recent_atoms.is_empty());
    assert!(drain_commands(&mut app).is_empty());

    app.apply_pick_result(Some(&atom_hit(0)), false);
    assert_eq!(
        drain_commands(&mut app),
        ["select sele, model mol and chain \"\" and resi 0"]
    );

    app.kernel.session.selections.define("sele", "model mol");
    app.apply_pick_result(None, false);
    assert_eq!(drain_commands(&mut app), ["deselect sele"]);
}

#[test]
fn modified_pick_is_always_atomic_regardless_of_mouse_selection_mode() {
    let atoms = ["CA", "CB"].map(|name| Atom::new(name, Element::Carbon));
    let mut app = app_with_atoms(atoms);
    app.kernel.session.settings.ui.mouse_selection_mode = MouseSelectionMode::Objects;

    app.apply_pick_result(Some(&atom_hit(1)), true);

    assert!(drain_commands(&mut app)[0].starts_with("pick \"/mol/"));
}

#[test]
fn stationary_modifier_transitions_reexpand_cached_hover_hit() {
    let mut app = app_with_atoms([
        Atom::new("CA", Element::Carbon),
        Atom::new("CB", Element::Carbon),
    ]);
    app.apply_hover_result(Some(atom_hit(0)));
    assert_eq!(
        app.kernel
            .session
            .hover_target
            .as_ref()
            .unwrap()
            .selection
            .count(),
        2
    );

    app.kernel
        .viewport
        .input
        .handle_modifiers(Modifiers::CONTROL);
    app.refresh_cached_hover_mode();
    assert_eq!(
        app.kernel
            .session
            .hover_target
            .as_ref()
            .unwrap()
            .selection
            .count(),
        1
    );

    app.kernel.viewport.input.handle_modifiers(Modifiers::EMPTY);
    app.refresh_cached_hover_mode();
    assert_eq!(
        app.kernel
            .session
            .hover_target
            .as_ref()
            .unwrap()
            .selection
            .count(),
        2
    );
}

#[test]
fn modified_non_atom_hit_warns_without_queuing_a_command() {
    let mut app = app_with_atoms([Atom::new("CA", Element::Carbon)]);
    let mut hit = atom_hit(0);
    hit.atom_index = None;

    app.apply_pick_result(Some(&hit), true);

    assert_eq!(drain_warnings(&mut app).len(), 1);
    assert!(drain_commands(&mut app).is_empty());
}

#[test]
fn wgpu_settings_requests_compute_capable_limits() {
    let settings = patinae_wgpu_settings();
    let limits = settings.device_required_limits;

    assert!(limits.max_compute_workgroups_per_dimension > 0);
    assert!(limits.max_compute_workgroup_size_x >= 64);
    assert!(limits.max_compute_invocations_per_workgroup >= 64);
    assert!(limits.max_storage_buffer_binding_size > 0);
    assert!(limits.max_storage_textures_per_shader_stage > 0);
}

#[test]
fn wgpu_required_limits_clamp_to_intel_storage_limit() {
    let adapter_limits = adapter_limits(PERFORMANCE_MAX_BUFFER_SIZE, gib_to_bytes(1) - 4);

    let required = patinae_wgpu_required_limits(&adapter_limits, RenderMemoryPolicy::performance());

    assert_eq!(required.max_storage_buffer_binding_size, 1_073_741_820);
    assert!(
        required.max_storage_buffer_binding_size <= adapter_limits.max_storage_buffer_binding_size
    );
}

#[test]
fn wgpu_required_limits_preserve_high_end_target() {
    let adapter_limits = adapter_limits(gib_to_bytes(8), u64::MAX);

    let required = patinae_wgpu_required_limits(&adapter_limits, RenderMemoryPolicy::performance());

    assert_eq!(required.max_buffer_size, PERFORMANCE_MAX_BUFFER_SIZE);
    assert_eq!(
        required.max_storage_buffer_binding_size,
        PERFORMANCE_MAX_STORAGE_BUFFER_BINDING_SIZE
    );
}

#[test]
fn wgpu_required_limits_use_adapter_resolution_limits() {
    let adapter_buffer_size = mib_to_bytes(512);
    let adapter_storage_binding_size = mib_to_bytes(512);
    let adapter_limits = adapter_limits(adapter_buffer_size, adapter_storage_binding_size);

    let required = patinae_wgpu_required_limits(&adapter_limits, RenderMemoryPolicy::lite());

    assert_eq!(required.max_buffer_size, adapter_buffer_size);
    assert_eq!(
        required.max_storage_buffer_binding_size,
        adapter_storage_binding_size
    );
    assert_eq!(required.max_texture_dimension_1d, 16_384);
    assert_eq!(required.max_texture_dimension_2d, 32_768);
    assert_eq!(required.max_texture_dimension_3d, 2_048);
}

#[test]
fn wgpu_windows_default_backend_prefers_dx12() {
    let backends = patinae_wgpu_backends(None, true);

    assert_eq!(backends, slint::wgpu_30::wgpu::Backends::DX12);
}

#[test]
fn wgpu_windows_backend_env_is_respected_without_gl() {
    let backends = patinae_wgpu_backends(
        Some(slint::wgpu_30::wgpu::Backends::VULKAN | slint::wgpu_30::wgpu::Backends::GL),
        true,
    );

    assert_eq!(backends, slint::wgpu_30::wgpu::Backends::VULKAN);
}

#[test]
fn wgpu_non_windows_default_backend_keeps_wgpu_default_without_gl() {
    let backends = patinae_wgpu_backends(None, false);

    assert_eq!(
        backends,
        slint::wgpu_30::wgpu::Backends::default() & !slint::wgpu_30::wgpu::Backends::GL
    );
}

#[test]
fn startup_renderer_error_message_explains_hardware_requirement() {
    let message = startup_renderer_error_message("adapter failed");

    assert!(message.contains("hardware-accelerated WGPU renderer"));
    assert!(message.contains("adapter failed"));
    assert!(message.contains("VM 3D acceleration"));
    assert!(message.contains("Patinae will now exit"));
}

#[test]
fn wgpu_no_hardware_adapter_error_disables_cpu_fallbacks() {
    let err = no_hardware_wgpu_adapter_error(slint::wgpu_30::wgpu::Backends::DX12, None);
    let message = err.to_string();

    assert!(message.contains("No hardware-accelerated WGPU adapter"));
    assert!(message.contains("CPU/WARP/llvmpipe software rendering is disabled"));
}

#[test]
fn wgpu_cpu_override_is_rejected() {
    let message = cpu_wgpu_adapter_error().to_string();

    assert!(message.contains("SLINT_WGPU_CPU"));
    assert!(message.contains("hardware-accelerated WGPU renderer"));
}

#[test]
fn viewport_size_scales_actual_slint_bounds() {
    assert_eq!(viewport_size_from_parts(752.0, 500.0, 1.0), (752, 500));
    assert_eq!(viewport_size_from_parts(752.0, 500.0, 1.5), (1128, 750));
    assert_eq!(viewport_size_from_parts(752.0, 500.0, 2.0), (1504, 1000));
}

#[test]
fn viewport_size_clamps_to_one_physical_pixel() {
    assert_eq!(viewport_size_from_parts(0.0, -10.0, 2.0), (1, 1));
}

#[test]
fn annotation_font_size_stays_device_independent_on_hidpi() {
    let projected = patinae_scene::bridge::ProjectedAnnotationLabel {
        x: 200.0,
        y: 100.0,
        text: "6.1 Å".to_string(),
        color: [0.13, 0.83, 0.93, 1.0],
        size: 14.0,
        alignment: patinae_scene::LabelAlignment::Center,
        owner_id: patinae_scene::RenderObjectId::FIRST,
        insertion_ordinal: 0,
        display_order: 0,
    };

    let label = viewport_label_from_projected(&projected, 2.0);

    assert_eq!(label.x, 100.0);
    assert_eq!(label.y, 50.0);
    assert_eq!(label.size, 14.0);
}

#[test]
fn lifecycle_dispatch_quits_event_loop_once() {
    let messages = [
        AppMessage::TogglePanel("objects".into()),
        AppMessage::Quit,
        AppMessage::Quit,
    ];
    let mut calls = 0;

    let quit_requested = dispatch_lifecycle_messages(&messages, || {
        calls += 1;
        Ok(())
    });

    assert!(quit_requested);
    assert_eq!(calls, 1);
}

#[test]
fn lifecycle_dispatch_ignores_non_quit_messages() {
    let messages = [AppMessage::TogglePanel("objects".into())];
    let mut calls = 0;

    let quit_requested = dispatch_lifecycle_messages(&messages, || {
        calls += 1;
        Ok(())
    });

    assert!(!quit_requested);
    assert_eq!(calls, 0);
}

#[test]
fn request_event_loop_quit_calls_delegate_once() {
    let mut calls = 0;

    request_event_loop_quit(|| {
        calls += 1;
        Ok(())
    });

    assert_eq!(calls, 1);
}

#[test]
fn save_file_response_targets_originating_panel() {
    let request = SaveFileRequest {
        panel_id: "fixture-panel".into(),
        reply_control_id: "save_file_selected".into(),
        title: "Save image".into(),
        default_file_name: "image.png".into(),
        allowed_extensions: vec!["png".into()],
    };

    let event = save_file_response_event(&request, "/tmp/render.png".into());

    assert_eq!(event.panel_id, "fixture-panel");
    assert_eq!(event.control_id, "save_file_selected");
    assert_eq!(event.kind, PanelEventKind::TextCommit);
    assert!(matches!(event.value, PanelValue::Text(path) if path == "/tmp/render.png"));
}

#[test]
fn save_file_requests_are_deferred_out_of_render_path() {
    let request = SaveFileRequest {
        panel_id: "fixture-panel".into(),
        reply_control_id: "save_file_selected".into(),
        title: "Save image".into(),
        default_file_name: "image.png".into(),
        allowed_extensions: vec!["png".into()],
    };
    let mut bus = patinae_framework::message::MessageBus::new();
    topics::publish(&mut bus, SAVE_FILE_REQUEST_TOPIC, &request);
    let messages = bus.drain_outbox();

    let mut app = App::new();

    assert!(app.queue_save_file_request(&messages[0]));
    assert_eq!(
        app.file_actions.pending_save_file_requests.pop_front(),
        Some(request)
    );
    assert!(!app.file_actions.save_file_dialog_scheduled);
}

#[test]
fn startup_work_waits_for_plugins_and_drains_once_in_order() {
    let mut app = App::new();
    let rc = write_startup_file("plugin-startup-rc");
    std::fs::write(&rc, "set sphere_scale, 2\n").unwrap();
    let argv = PathBuf::from("/tmp/startup molecule.pdb");
    let deferred = PathBuf::from("/tmp/user script.fixture");
    app.startup_actions = VecDeque::from([
        StartupAction::RunPatinaerc(rc.clone()),
        StartupAction::RouteArgvFile(argv.clone()),
    ]);
    app.route_native_file(&deferred, NativeFileSource::DragDrop);
    app.kernel.bus.drain_outbox();
    app.run_startup_actions((800, 600));
    assert_eq!(app.startup_actions.len(), 2);
    assert_eq!(app.file_actions.deferred_file_actions.len(), 1);
    app.plugin_runtime.plugin_startup.after_frame();
    app.run_startup_actions((800, 600));
    assert_eq!(app.startup_actions.len(), 2);
    app.plugin_runtime.plugin_startup.finish();
    app.rendering.renderer_startup = Some(crate::renderer_startup::RendererStartup::new(|| {
        panic!("This test must not start GPU preparation")
    }));
    app.run_startup_actions((800, 600));
    assert_eq!(app.startup_actions.len(), 2);
    app.rendering.renderer_startup = None;
    app.run_startup_actions((800, 600));
    assert!(app.startup_actions.is_empty());
    assert!(app.file_actions.deferred_file_actions.is_empty());
    let messages = app.kernel.bus.drain_outbox();
    let commands: Vec<_> = messages
        .iter()
        .filter_map(|message| match message {
            AppMessage::ExecuteCommand { command, .. } => Some(command.as_str()),
            _ => None,
        })
        .collect();
    // The custom format is unsupported in this empty host: the queued file is
    // resolved after readiness, then reports a warning rather than vanishing.
    assert_eq!(
        commands.last().copied(),
        Some("load \"/tmp/startup molecule.pdb\"")
    );
    assert!(messages.iter().any(|message| matches!(message, AppMessage::PrintWarning(text) if text.contains("user script.fixture"))));
    app.run_startup_actions((800, 600));
    assert!(app.kernel.bus.drain_outbox().is_empty());
    std::fs::remove_file(rc).unwrap();
}

#[test]
fn unknown_repl_commands_explain_loading_without_replay() {
    let mut app = App::new();
    app.execute_repl_input("not_registered_yet".into(), (800, 600));
    assert!(app.kernel.bus.drain_outbox().iter().any(
        |message| matches!(message, AppMessage::PrintInfo(text) if text.contains("still loading"))
    ));
    app.execute_repl_input("set sphere_scale, 2".into(), (800, 600));
    assert!(!app.kernel.bus.drain_outbox().iter().any(
        |message| matches!(message, AppMessage::PrintInfo(text) if text.contains("still loading"))
    ));
    app.plugin_runtime.plugin_startup.finish();
    app.execute_repl_input("not_registered_yet".into(), (800, 600));
    assert!(!app.kernel.bus.drain_outbox().iter().any(
        |message| matches!(message, AppMessage::PrintInfo(text) if text.contains("still loading"))
    ));
    assert!(app.startup_actions.is_empty());
}

#[test]
fn startup_actions_skip_missing_default_patinaerc() {
    let path = temp_startup_path("missing-default-patinaerc");
    let actions = startup_actions(
        PatinaercPath {
            path,
            source: PatinaercSource::Default,
        },
        Vec::new(),
    );

    assert!(actions.is_empty());
}

#[test]
fn startup_actions_warn_for_missing_explicit_patinaerc() {
    let path = temp_startup_path("missing-explicit-patinaerc");
    let actions: Vec<_> = startup_actions(
        PatinaercPath {
            path: path.clone(),
            source: PatinaercSource::Env,
        },
        Vec::new(),
    )
    .into_iter()
    .collect();

    assert!(matches!(
        actions.as_slice(),
        [StartupAction::Warning(message)]
            if message.contains("PATINAERC") && message.contains(&path.display().to_string())
    ));
}

#[test]
fn startup_actions_run_patinaerc_before_argv_load() {
    let path = write_startup_file("existing-patinaerc");
    let actions: Vec<_> = startup_actions(
        PatinaercPath {
            path: path.clone(),
            source: PatinaercSource::Default,
        },
        vec![PathBuf::from("input.pdb")],
    )
    .into_iter()
    .collect();
    let _ = std::fs::remove_file(&path);

    assert_eq!(
        actions,
        vec![
            StartupAction::RunPatinaerc(path),
            StartupAction::RouteArgvFile(PathBuf::from("input.pdb"))
        ]
    );
}

#[test]
fn startup_actions_keep_multiple_argv_files_in_order() {
    let actions: Vec<_> = startup_actions(
        PatinaercPath {
            path: temp_startup_path("missing-default-patinaerc"),
            source: PatinaercSource::Default,
        },
        vec![PathBuf::from("first.pdb"), PathBuf::from("second.pml")],
    )
    .into_iter()
    .collect();

    assert_eq!(
        actions,
        vec![
            StartupAction::RouteArgvFile(PathBuf::from("first.pdb")),
            StartupAction::RouteArgvFile(PathBuf::from("second.pml")),
        ]
    );
}

#[test]
fn startup_command_paths_with_spaces_are_quoted() {
    assert_eq!(
        quote_command_arg("/tmp/patinae startup/patinaerc"),
        "\"/tmp/patinae startup/patinaerc\""
    );
}

#[test]
fn recent_open_uses_shared_file_router() {
    let mut app = App::new();
    let path = PathBuf::from("/tmp/recent files/script.py");
    app.route_native_file(&path, NativeFileSource::RecentFile);
    assert_eq!(app.file_actions.deferred_file_actions.len(), 1);
}

#[test]
fn standard_panel_preset_only_applies_when_empty_scene_becomes_active() {
    assert!(should_apply_standard_panel_preset(true, false));
    assert!(!should_apply_standard_panel_preset(true, true));
    assert!(!should_apply_standard_panel_preset(false, false));
    assert!(!should_apply_standard_panel_preset(false, true));
}

fn platform_modifiers() -> (bool, bool) {
    #[cfg(target_os = "macos")]
    {
        (false, true)
    }
    #[cfg(not(target_os = "macos"))]
    {
        (true, false)
    }
}

#[test]
fn normalizes_common_platform_shortcuts() {
    let (ctrl, super_key) = platform_modifiers();

    for (key, text) in [
        (KeyCode::KeyA, "a"),
        (KeyCode::KeyC, "c"),
        (KeyCode::KeyL, "l"),
        (KeyCode::KeyV, "v"),
        (KeyCode::KeyX, "x"),
        (KeyCode::KeyZ, "z"),
        (KeyCode::KeyU, "u"),
    ] {
        assert_eq!(
            normalized_slint_shortcut_text(key, false, ctrl, false, super_key),
            Some(text)
        );
    }
}

#[test]
fn normalizes_shifted_printable_shortcuts() {
    let (ctrl, super_key) = platform_modifiers();

    assert_eq!(
        normalized_slint_shortcut_text(KeyCode::KeyZ, true, ctrl, false, super_key),
        Some("Z")
    );
    assert_eq!(
        normalized_slint_shortcut_text(KeyCode::Digit1, true, ctrl, false, super_key),
        Some("!")
    );
    assert_eq!(
        normalized_slint_shortcut_text(KeyCode::Slash, true, ctrl, false, super_key),
        Some("?")
    );
}

#[test]
fn skips_shortcuts_without_platform_modifier() {
    assert_eq!(
        normalized_slint_shortcut_text(KeyCode::KeyA, false, false, false, false),
        None
    );

    #[cfg(target_os = "macos")]
    assert_eq!(
        normalized_slint_shortcut_text(KeyCode::KeyA, false, true, false, false),
        None
    );

    #[cfg(not(target_os = "macos"))]
    assert_eq!(
        normalized_slint_shortcut_text(KeyCode::KeyA, false, false, false, true),
        None
    );
}

#[test]
fn skips_alt_and_non_printable_shortcuts() {
    let (ctrl, super_key) = platform_modifiers();

    assert_eq!(
        normalized_slint_shortcut_text(KeyCode::KeyA, false, ctrl, true, super_key),
        None
    );
    assert_eq!(
        normalized_slint_shortcut_text(KeyCode::Escape, false, ctrl, false, super_key),
        None
    );
    assert_eq!(
        normalized_slint_shortcut_text(KeyCode::F1, false, ctrl, false, super_key),
        None
    );
}
