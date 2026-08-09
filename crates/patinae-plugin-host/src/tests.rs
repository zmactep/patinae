use std::ffi::c_void;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use patinae_cmd::{
    parse_command, CommandContext, CommandExecutor, CommandRegistry, CommandRuntimeRequirements,
    OutputMessage,
};
use patinae_framework::atom_stream::{
    AtomColumn, AtomStreamMode, AtomStreamRequest, AtomStreamScope, AtomValue,
};
use patinae_framework::component::SharedContext;
use patinae_framework::message::{AppMessage, MessageBus};
use patinae_framework::plugin_ui::{
    PanelAction, PanelDescriptor, PanelEvent, PanelEventKind, PanelPlacement,
    PanelRuntimeRequirements, PanelSnapshot, PanelValue, PluginPanel,
};
use patinae_mol::{Atom, AtomIndex, ObjectMolecule};
use patinae_plugin::ffi::{
    AbiBytesSinkFn, AbiCommandDescriptor, AbiCommandVTable, AbiPanelDescriptor, AbiPanelVTable,
    AbiSettingDescriptor, AbiSettingValue, AbiStatus, AbiStr, AbiStrSlice, AbiU8Slice,
    HostCallbacks, HostCommandRuntimeCallbacks, HostCommandRuntimeHandle, HostRegistrarHandle,
    PluginCommandHandle, PluginDeclaration, PluginPanelHandle, PluginRegisterFn, ABI_VERSION,
    CAPABILITY_COMMANDS, CAPABILITY_DIAGNOSTICS, CAPABILITY_FORMAT_HANDLERS, CAPABILITY_PANELS,
    CAPABILITY_REGISTRATION, CAPABILITY_SETTINGS, HOST_CALLBACKS_VERSION, MAX_ABI_STRING_LEN,
    PANEL_PLACEMENT_RIGHT, SDK_VERSION, SETTING_TYPE_BOOL, SETTING_VALUE_BOOL,
};
use patinae_plugin::registrar::{
    DynCmdRegistration, FormatHandler, MessageHandler, PluginMetadata, PluginReaderFn,
    PluginRegistrar, PluginWriterFn, PollContext,
};
use patinae_plugin::wire::{
    self, WireCommandInput, WireCommandOutput, WireHostQuery, WireHostQueryValue,
    WirePanelEventOutput, WirePanelSnapshotOutput, RUNTIME_WIRE_VERSION,
};
use patinae_scene::{
    canonical_atom_path_for_hit, GroupObject, KeyBindings, MoleculeObject, ObjectType, PickHit,
    Session, SessionAdapter,
};
use patinae_settings::groups::RecentPickLimit;
use serde::{de::DeserializeOwned, Serialize};

use crate::loader::{
    apply_command_output, command_input_from_context, ensure_runtime_payload_within_limit,
    load_declaration_for_test, shared_input_from_context, validate_declaration,
};
use crate::plugin::{LibraryHandle, LoadedPanel, LoadedPlugin};
use crate::{is_plugin_library_path, validate_declaration_versions, PluginHost};

struct StaticPanel {
    descriptor: PanelDescriptor,
    actions: Vec<PanelAction>,
    snapshot_count: Option<Arc<AtomicUsize>>,
}

impl StaticPanel {
    fn new(descriptor: PanelDescriptor) -> Self {
        Self {
            descriptor,
            actions: Vec::new(),
            snapshot_count: None,
        }
    }

    fn with_actions(descriptor: PanelDescriptor, actions: Vec<PanelAction>) -> Self {
        Self {
            descriptor,
            actions,
            snapshot_count: None,
        }
    }

    fn with_snapshot_count(descriptor: PanelDescriptor, count: Arc<AtomicUsize>) -> Self {
        Self {
            descriptor,
            actions: Vec::new(),
            snapshot_count: Some(count),
        }
    }
}

impl PluginPanel for StaticPanel {
    fn descriptor(&self) -> PanelDescriptor {
        self.descriptor.clone()
    }

    fn snapshot(&mut self, _ctx: &SharedContext<'_>) -> PanelSnapshot {
        if let Some(count) = &self.snapshot_count {
            count.fetch_add(1, Ordering::Relaxed);
        }
        PanelSnapshot::default()
    }

    fn handle_event(
        &mut self,
        _event: PanelEvent,
        _ctx: &SharedContext<'_>,
        _bus: &mut MessageBus,
    ) -> Vec<PanelAction> {
        self.actions.clone()
    }
}

struct SharedFixture {
    session: Session,
    command_registry: CommandRegistry,
    command_names: Vec<String>,
    setting_names: Vec<&'static str>,
}

impl SharedFixture {
    fn new() -> Self {
        Self {
            session: Session::new(),
            command_registry: CommandRegistry::new(),
            command_names: Vec::new(),
            setting_names: Vec::new(),
        }
    }

    fn shared(&self) -> SharedContext<'_> {
        SharedContext {
            registry: &self.session.registry,
            camera: &self.session.camera,
            selections: &self.session.selections,
            recent_atoms: &self.session.recent_atoms,
            named_palette: &self.session.named_palette,
            movie: &self.session.movie,
            settings: &self.session.settings,
            clear_color: self.session.clear_color,
            gpu_device: None,
            gpu_queue: None,
            scene_generation: 0,
            viewport_image: self.session.viewport_image.as_ref(),
            command_names: &self.command_names,
            command_registry: &self.command_registry,
            setting_names: &self.setting_names,
            dynamic_settings: None,
        }
    }
}

fn command_input_for_session(
    session: &mut Session,
    runtime_requirements: CommandRuntimeRequirements,
) -> WireCommandInput {
    let mut needs_redraw = false;
    let mut adapter = SessionAdapter {
        session,
        render_context: None,
        default_size: (800, 600),
        needs_redraw: &mut needs_redraw,
        async_fetch_fn: None,
    };
    let mut ctx = CommandContext::new(&mut adapter);
    let parsed = parse_command("python print('ok')").unwrap();

    command_input_from_context(&mut ctx, &parsed, runtime_requirements).unwrap()
}

fn decoded_wire_session(bytes: &[u8]) -> Session {
    wire::decode_session(bytes).unwrap()
}

fn execute_text(executor: &mut CommandExecutor, command: &str) -> String {
    let mut session = Session::new();
    let mut needs_redraw = false;
    let mut adapter = SessionAdapter {
        session: &mut session,
        render_context: None,
        default_size: (64, 64),
        needs_redraw: &mut needs_redraw,
        async_fetch_fn: None,
    };
    let output = executor
        .do_with_options(&mut adapter, command, false)
        .expect("capabilities command should succeed");
    assert_eq!(output.messages.len(), 1);
    output.messages[0].text.clone()
}

#[test]
fn command_input_without_full_session_omits_registry() {
    let mut session = Session::new();
    session.registry.add(GroupObject::new("big_group"));

    let input = command_input_for_session(&mut session, CommandRuntimeRequirements::NONE);
    let decoded = decoded_wire_session(&input.session);

    assert!(decoded.registry.get("big_group").is_none());
}

#[test]
fn command_input_with_full_session_preserves_registry() {
    let mut session = Session::new();
    session.registry.add(GroupObject::new("big_group"));

    let input = command_input_for_session(&mut session, CommandRuntimeRequirements::FULL_SESSION);
    let decoded = decoded_wire_session(&input.session);

    assert!(decoded.registry.get("big_group").is_some());
}

#[test]
fn command_input_with_trace_stream_omits_displayed_geometry_payloads() {
    let mut session = Session::new();

    let input = command_input_for_session(
        &mut session,
        CommandRuntimeRequirements::TRACE_GEOMETRY_STREAM,
    );

    assert!(input.displayed_geometry.is_none());
    assert!(input.displayed_geometry_spool.is_none());
}

#[test]
fn panel_input_without_full_session_omits_registry() {
    let mut fixture = SharedFixture::new();
    fixture
        .session
        .registry
        .add(GroupObject::new("panel_group"));

    let input =
        shared_input_from_context(&fixture.shared(), PanelRuntimeRequirements::NONE).unwrap();
    let decoded = decoded_wire_session(&input.session);

    assert!(decoded.registry.get("panel_group").is_none());
}

#[test]
fn panel_input_with_full_session_preserves_registry_and_recent_atoms() {
    let mut fixture = SharedFixture::new();
    add_test_molecule(&mut fixture.session);
    let path = canonical_atom_path_for_hit(
        &PickHit {
            object_name: "obj".to_string(),
            object_type: ObjectType::Molecule,
            atom_index: Some(AtomIndex(0)),
            position: Default::default(),
            distance: 0.0,
        },
        fixture
            .session
            .registry
            .get_molecule("obj")
            .unwrap()
            .molecule(),
    )
    .unwrap();
    fixture
        .session
        .recent_atoms
        .insert(path.clone(), RecentPickLimit::Unlimited);

    let input =
        shared_input_from_context(&fixture.shared(), PanelRuntimeRequirements::FULL_SESSION)
            .unwrap();
    let decoded = decoded_wire_session(&input.session);

    assert!(decoded.registry.get("obj").is_some());
    assert_eq!(decoded.recent_atoms.paths().collect::<Vec<_>>(), vec![path]);
}

#[test]
fn oversized_payload_guard_names_full_scene_limit() {
    let err = ensure_runtime_payload_within_limit(
        "command execute",
        wire::MAX_WIRE_PAYLOAD_LEN + 1,
        "full scene state",
    )
    .unwrap_err();

    assert!(err.contains("full scene state"));
    assert!(err.contains("64 MiB"));
}

#[test]
fn lightweight_command_output_does_not_replace_host_session() {
    let mut session = Session::new();
    session.registry.add(GroupObject::new("keep_group"));
    let replacement = Session::new();
    let output = WireCommandOutput {
        wire_version: RUNTIME_WIRE_VERSION,
        result: Ok(()),
        output: Vec::new(),
        actions: Vec::new(),
        session: wire::encode_session(&replacement).unwrap(),
        viewport_image: None,
        viewport_image_changed: false,
    };

    {
        let mut needs_redraw = false;
        let mut adapter = SessionAdapter {
            session: &mut session,
            render_context: None,
            default_size: (800, 600),
            needs_redraw: &mut needs_redraw,
            async_fetch_fn: None,
        };
        let mut ctx = CommandContext::new(&mut adapter);
        apply_command_output(&mut ctx, output, CommandRuntimeRequirements::NONE).unwrap();
    }

    assert!(session.registry.get("keep_group").is_some());
}

struct ClearCountingRenderer {
    clears: usize,
}

impl patinae_scene::CaptureRenderer for ClearCountingRenderer {
    fn gpu_device(&self) -> &Arc<wgpu::Device> {
        unreachable!("test renderer does not expose a GPU device")
    }

    fn gpu_queue(&self) -> &Arc<wgpu::Queue> {
        unreachable!("test renderer does not expose a GPU queue")
    }

    #[allow(clippy::too_many_arguments)]
    fn capture_png(
        &mut self,
        _path: &Path,
        _width: u32,
        _height: u32,
        _camera: &mut patinae_scene::Camera,
        _registry: &mut patinae_scene::ObjectRegistry,
        _settings: &patinae_settings::Settings,
        _named: &patinae_scene::NamedPalette,
        _themed: &patinae_scene::ThemedPalette,
        _clear_color: [f32; 3],
    ) -> Result<(), patinae_scene::ViewerError> {
        unreachable!("test renderer does not capture PNGs")
    }

    fn clear_viewport_gpu_image(&mut self) {
        self.clears += 1;
    }
}

#[test]
fn command_output_without_viewport_change_keeps_gpu_viewport_image() {
    let mut session = Session::new();
    let mut renderer = ClearCountingRenderer { clears: 0 };
    let output = WireCommandOutput {
        wire_version: RUNTIME_WIRE_VERSION,
        result: Ok(()),
        output: Vec::new(),
        actions: Vec::new(),
        session: wire::encode_session(&Session::new()).unwrap(),
        viewport_image: None,
        viewport_image_changed: false,
    };

    {
        let mut needs_redraw = false;
        let mut adapter = SessionAdapter {
            session: &mut session,
            render_context: Some(&mut renderer),
            default_size: (800, 600),
            needs_redraw: &mut needs_redraw,
            async_fetch_fn: None,
        };
        let mut ctx = CommandContext::new(&mut adapter);
        apply_command_output(&mut ctx, output, CommandRuntimeRequirements::NONE).unwrap();
    }

    assert_eq!(renderer.clears, 0);
}

#[test]
fn command_output_with_viewport_clear_drops_gpu_viewport_image() {
    let mut session = Session::new();
    let mut renderer = ClearCountingRenderer { clears: 0 };
    let output = WireCommandOutput {
        wire_version: RUNTIME_WIRE_VERSION,
        result: Ok(()),
        output: Vec::new(),
        actions: Vec::new(),
        session: wire::encode_session(&Session::new()).unwrap(),
        viewport_image: None,
        viewport_image_changed: true,
    };

    {
        let mut needs_redraw = false;
        let mut adapter = SessionAdapter {
            session: &mut session,
            render_context: Some(&mut renderer),
            default_size: (800, 600),
            needs_redraw: &mut needs_redraw,
            async_fetch_fn: None,
        };
        let mut ctx = CommandContext::new(&mut adapter);
        apply_command_output(&mut ctx, output, CommandRuntimeRequirements::NONE).unwrap();
    }

    assert_eq!(renderer.clears, 1);
}

fn host_with_panels(panels: Vec<LoadedPanel>) -> PluginHost {
    let mut host = PluginHost::new();
    host.plugins.push(LoadedPlugin {
        _library: Arc::new(LibraryHandle::Static),
        metadata: PluginMetadata::new("test-plugin", "0.0.0", "test"),
        message_handler: None,
        panels,
        hotkeys: KeyBindings::new(),
        atom_streams: Default::default(),
        faulted: false,
    });
    host.ensure_single_active_per_placement();
    host
}

fn host_with_message_handler(handler: Box<dyn MessageHandler>) -> PluginHost {
    host_with_message_handlers(vec![handler])
}

fn host_with_message_handlers(handlers: Vec<Box<dyn MessageHandler>>) -> PluginHost {
    let mut host = PluginHost::new();
    for (index, handler) in handlers.into_iter().enumerate() {
        host.plugins.push(LoadedPlugin {
            _library: Arc::new(LibraryHandle::Static),
            metadata: PluginMetadata::new(format!("test-plugin-{index}"), "0.0.0", "test"),
            message_handler: Some(handler),
            panels: Vec::new(),
            hotkeys: KeyBindings::new(),
            atom_streams: Default::default(),
            faulted: false,
        });
    }
    host
}

struct IdleHandler;

impl MessageHandler for IdleHandler {
    fn on_message(&mut self, _msg: &AppMessage, _bus: &mut MessageBus) {}

    fn needs_poll(&self) -> bool {
        true
    }

    fn poll(&mut self, _ctx: &mut PollContext<'_>) {}
}

struct QueryNamesHandler {
    requested: bool,
    seen: Arc<std::sync::Mutex<Option<Vec<String>>>>,
}

impl MessageHandler for QueryNamesHandler {
    fn on_message(&mut self, _msg: &AppMessage, _bus: &mut MessageBus) {}

    fn needs_poll(&self) -> bool {
        true
    }

    fn poll(&mut self, ctx: &mut PollContext<'_>) {
        for result in ctx.host_query_results {
            if let Ok(WireHostQueryValue::ObjectNames(names)) = &result.result {
                *self.seen.lock().unwrap() = Some(names.clone());
            }
        }
        if !self.requested {
            ctx.query_host(WireHostQuery::ObjectNames { id: 7 });
            self.requested = true;
        }
    }
}

struct QueryIsolationHandler {
    query: WireHostQuery,
    requested: bool,
    result_count: Arc<std::sync::Mutex<Option<usize>>>,
}

impl MessageHandler for QueryIsolationHandler {
    fn on_message(&mut self, _msg: &AppMessage, _bus: &mut MessageBus) {}

    fn needs_poll(&self) -> bool {
        true
    }

    fn poll(&mut self, ctx: &mut PollContext<'_>) {
        if self.requested {
            *self.result_count.lock().unwrap() = Some(ctx.host_query_results.len());
            return;
        }
        ctx.query_host(self.query.clone());
        self.requested = true;
    }
}

struct AtomStreamLifecycleHandler {
    phase: u8,
    stream_id: Option<u64>,
    seen: Arc<std::sync::Mutex<Vec<String>>>,
}

impl MessageHandler for AtomStreamLifecycleHandler {
    fn on_message(&mut self, _msg: &AppMessage, _bus: &mut MessageBus) {}

    fn needs_poll(&self) -> bool {
        true
    }

    fn poll(&mut self, ctx: &mut PollContext<'_>) {
        match self.phase {
            0 => {
                ctx.query_host(WireHostQuery::OpenAtomStream {
                    id: 1,
                    request: atom_stream_request("obj"),
                });
                self.phase = 1;
            }
            1 => {
                let stream_id = match &ctx.host_query_results[0].result {
                    Ok(WireHostQueryValue::AtomStreamOpened(opened)) => {
                        assert_eq!(opened.total_count, 2);
                        opened.stream_id
                    }
                    Ok(_) => panic!("unexpected open result kind"),
                    Err(error) => panic!("unexpected open error: {error}"),
                };
                self.stream_id = Some(stream_id);
                ctx.query_host(WireHostQuery::ReadAtomStream {
                    id: 2,
                    stream_id,
                    max_rows: 1,
                });
                self.phase = 2;
            }
            2 => {
                let stream_id = self.stream_id.unwrap();
                let chunk = match &ctx.host_query_results[0].result {
                    Ok(WireHostQueryValue::AtomStreamChunk(chunk)) => chunk,
                    Ok(_) => panic!("unexpected first chunk result kind"),
                    Err(error) => panic!("unexpected first chunk error: {error}"),
                };
                assert_eq!(chunk.rows.len(), 1);
                assert!(!chunk.done);
                self.seen.lock().unwrap().push(row_name(&chunk.rows[0]));
                ctx.query_host(WireHostQuery::ReadAtomStream {
                    id: 3,
                    stream_id,
                    max_rows: 16,
                });
                self.phase = 3;
            }
            3 => {
                let stream_id = self.stream_id.unwrap();
                let chunk = match &ctx.host_query_results[0].result {
                    Ok(WireHostQueryValue::AtomStreamChunk(chunk)) => chunk,
                    Ok(_) => panic!("unexpected final chunk result kind"),
                    Err(error) => panic!("unexpected final chunk error: {error}"),
                };
                assert_eq!(chunk.rows.len(), 1);
                assert!(chunk.done);
                self.seen.lock().unwrap().push(row_name(&chunk.rows[0]));
                ctx.query_host(WireHostQuery::ReadAtomStream {
                    id: 4,
                    stream_id,
                    max_rows: 1,
                });
                self.phase = 4;
            }
            4 => {
                if let Err(error) = &ctx.host_query_results[0].result {
                    self.seen.lock().unwrap().push(error.clone());
                } else {
                    panic!("expected atom stream closed error");
                }
                self.phase = 5;
            }
            _ => {}
        }
    }
}

struct AtomStreamErrorHandler {
    query: WireHostQuery,
    requested: bool,
    error: Arc<std::sync::Mutex<Option<String>>>,
}

impl MessageHandler for AtomStreamErrorHandler {
    fn on_message(&mut self, _msg: &AppMessage, _bus: &mut MessageBus) {}

    fn needs_poll(&self) -> bool {
        true
    }

    fn poll(&mut self, ctx: &mut PollContext<'_>) {
        if self.requested {
            if let Err(error) = &ctx.host_query_results[0].result {
                *self.error.lock().unwrap() = Some(error.clone());
            }
            return;
        }
        ctx.query_host(self.query.clone());
        self.requested = true;
    }
}

struct StaleAtomStreamHandler {
    phase: u8,
    stream_id: Option<u64>,
    error: Arc<std::sync::Mutex<Option<String>>>,
}

impl MessageHandler for StaleAtomStreamHandler {
    fn on_message(&mut self, _msg: &AppMessage, _bus: &mut MessageBus) {}

    fn needs_poll(&self) -> bool {
        true
    }

    fn poll(&mut self, ctx: &mut PollContext<'_>) {
        match self.phase {
            0 => {
                ctx.query_host(WireHostQuery::OpenAtomStream {
                    id: 1,
                    request: atom_stream_request("obj"),
                });
                self.phase = 1;
            }
            1 => {
                if let Ok(WireHostQueryValue::AtomStreamOpened(opened)) =
                    &ctx.host_query_results[0].result
                {
                    self.stream_id = Some(opened.stream_id);
                    ctx.query_host(WireHostQuery::ReadAtomStream {
                        id: 2,
                        stream_id: opened.stream_id,
                        max_rows: 1,
                    });
                    self.phase = 2;
                }
            }
            2 => {
                if let Err(error) = &ctx.host_query_results[0].result {
                    *self.error.lock().unwrap() = Some(error.clone());
                }
                self.phase = 3;
            }
            _ => {}
        }
    }
}

fn atom_stream_request(object: &str) -> AtomStreamRequest {
    AtomStreamRequest {
        scope: AtomStreamScope::Object(object.to_string()),
        mode: AtomStreamMode::Read,
        columns: vec![AtomColumn::Name],
        chunk_size: 1,
    }
}

fn row_name(row: &patinae_framework::atom_stream::AtomRow) -> String {
    match row.get(AtomColumn::Name) {
        Some(AtomValue::Str(name)) => name.clone(),
        _ => panic!("unexpected row name value"),
    }
}

fn add_test_molecule(session: &mut Session) {
    let mut molecule = ObjectMolecule::new("obj");
    molecule.add_atom(Atom {
        name: "CA".into(),
        ..Atom::default()
    });
    molecule.add_atom(Atom {
        name: "CB".into(),
        ..Atom::default()
    });
    session.registry.add(MoleculeObject::new(molecule));
}

fn loaded_panel(descriptor: PanelDescriptor) -> LoadedPanel {
    let visible = descriptor.default_visible;
    LoadedPanel {
        active: visible,
        visible,
        cached_snapshot_generation: None,
        cached_snapshot: PanelSnapshot::default(),
        panel: Box::new(StaticPanel::new(descriptor.clone())),
        descriptor,
    }
}

fn test_capabilities() -> u64 {
    CAPABILITY_REGISTRATION
        | CAPABILITY_COMMANDS
        | CAPABILITY_PANELS
        | CAPABILITY_SETTINGS
        | CAPABILITY_DIAGNOSTICS
        | CAPABILITY_FORMAT_HANDLERS
}

fn test_declaration(register: Option<PluginRegisterFn>) -> PluginDeclaration {
    PluginDeclaration {
        abi_version: ABI_VERSION,
        sdk_version: AbiStr::from_static(SDK_VERSION),
        capabilities: test_capabilities(),
        init: Some(ok_init),
        register,
    }
}

unsafe extern "C" fn ok_init(_callbacks: *const HostCallbacks) -> AbiStatus {
    AbiStatus::OK
}

fn fixture_command_handle() -> PluginCommandHandle {
    PluginCommandHandle(
        std::ptr::NonNull::<u8>::dangling()
            .as_ptr()
            .cast::<c_void>(),
    )
}

fn fixture_panel_handle() -> PluginPanelHandle {
    PluginPanelHandle(
        std::ptr::NonNull::<u8>::dangling()
            .as_ptr()
            .cast::<c_void>(),
    )
}

fn register_fixture_metadata(
    handle: HostRegistrarHandle,
    callbacks: &HostCallbacks,
    name: &'static str,
    version: &'static str,
    description: &'static str,
) -> AbiStatus {
    let Some(register_metadata) = callbacks.register_metadata else {
        return AbiStatus::INVALID;
    };
    // SAFETY: The host supplied `handle`; all string views remain static.
    unsafe {
        register_metadata(
            handle,
            AbiStr::from_static(name),
            AbiStr::from_static(version),
            AbiStr::from_static(description),
        )
    }
}

fn fixture_format_reader(_reader: Box<dyn std::io::Read>) -> Result<Vec<ObjectMolecule>, String> {
    Ok(Vec::new())
}

fn fixture_format_writer(
    _writer: Box<dyn std::io::Write>,
    _molecules: &[ObjectMolecule],
) -> Result<(), String> {
    Ok(())
}

fn fixture_format(name: &str, extension: &str, readable: bool, writable: bool) -> FormatHandler {
    let reader = readable.then(|| Arc::new(fixture_format_reader) as PluginReaderFn);
    let writer = writable.then(|| Arc::new(fixture_format_writer) as PluginWriterFn);
    FormatHandler {
        name: name.to_string(),
        extensions: vec![extension.to_string()],
        reader,
        writer,
    }
}

fn fixture_decode<T: DeserializeOwned>(input: AbiU8Slice) -> Result<T, AbiStatus> {
    if input.ptr.is_null() && input.len != 0 {
        return Err(AbiStatus::INVALID);
    }
    // SAFETY: Fixture callbacks only read the host-provided runtime payload
    // during this callback. Null and empty-slice handling were checked above.
    let bytes = unsafe { std::slice::from_raw_parts(input.ptr, input.len) };
    wire::decode(bytes).map_err(|_| AbiStatus::INVALID)
}

fn fixture_send<T: Serialize>(
    sink: AbiBytesSinkFn,
    user_data: *mut c_void,
    output: &T,
) -> AbiStatus {
    let Ok(bytes) = wire::encode(output) else {
        return AbiStatus::INVALID;
    };
    let slice = AbiU8Slice {
        ptr: bytes.as_ptr(),
        len: bytes.len(),
    };
    // SAFETY: `slice` points into `bytes`, which remains alive until the sink
    // returns. The host sink copies runtime output before returning.
    unsafe { sink(user_data, slice) }
}

unsafe extern "C" fn fixture_command_execute(
    handle: PluginCommandHandle,
    input: AbiU8Slice,
    _runtime_callbacks: *const HostCommandRuntimeCallbacks,
    _runtime_handle: HostCommandRuntimeHandle,
    sink: AbiBytesSinkFn,
    user_data: *mut c_void,
) -> AbiStatus {
    if handle.0.is_null() {
        return AbiStatus::INVALID;
    }
    let Ok(input): Result<WireCommandInput, _> = fixture_decode(input) else {
        return AbiStatus::INVALID;
    };
    if input.wire_version != RUNTIME_WIRE_VERSION {
        return AbiStatus::INVALID;
    }
    let output = WireCommandOutput {
        wire_version: RUNTIME_WIRE_VERSION,
        result: Ok(()),
        output: vec![OutputMessage::info("fixture command executed")],
        actions: Vec::new(),
        session: input.session,
        viewport_image: input.viewport_image,
        viewport_image_changed: false,
    };
    fixture_send(sink, user_data, &output)
}

unsafe extern "C" fn fixture_command_destroy(handle: PluginCommandHandle) -> AbiStatus {
    if handle.0.is_null() {
        AbiStatus::INVALID
    } else {
        AbiStatus::OK
    }
}

unsafe extern "C" fn fixture_panel_snapshot(
    handle: PluginPanelHandle,
    input: AbiU8Slice,
    sink: AbiBytesSinkFn,
    user_data: *mut c_void,
) -> AbiStatus {
    if handle.0.is_null() {
        return AbiStatus::INVALID;
    }
    let Ok(input): Result<patinae_plugin::wire::WireSharedInput, _> = fixture_decode(input) else {
        return AbiStatus::INVALID;
    };
    if input.wire_version != RUNTIME_WIRE_VERSION {
        return AbiStatus::INVALID;
    }
    let output = WirePanelSnapshotOutput {
        wire_version: RUNTIME_WIRE_VERSION,
        snapshot: PanelSnapshot::default(),
    };
    fixture_send(sink, user_data, &output)
}

unsafe extern "C" fn fixture_panel_event(
    handle: PluginPanelHandle,
    input: AbiU8Slice,
    sink: AbiBytesSinkFn,
    user_data: *mut c_void,
) -> AbiStatus {
    if handle.0.is_null() {
        return AbiStatus::INVALID;
    }
    let Ok(_input): Result<patinae_plugin::wire::WirePanelEventInput, _> = fixture_decode(input)
    else {
        return AbiStatus::INVALID;
    };
    let output = WirePanelEventOutput {
        wire_version: RUNTIME_WIRE_VERSION,
        actions: Vec::new(),
    };
    fixture_send(sink, user_data, &output)
}

unsafe extern "C" fn fixture_panel_destroy(handle: PluginPanelHandle) -> AbiStatus {
    if handle.0.is_null() {
        AbiStatus::INVALID
    } else {
        AbiStatus::OK
    }
}

unsafe extern "C" fn register_fixture(
    handle: HostRegistrarHandle,
    callbacks: *const HostCallbacks,
) -> AbiStatus {
    if callbacks.is_null() {
        return AbiStatus::INVALID;
    }
    // SAFETY: The host passes a valid callback table for this synchronous
    // registration call; null was checked above.
    let callbacks = unsafe { &*callbacks };
    if callbacks.table_version != HOST_CALLBACKS_VERSION {
        return AbiStatus::INVALID;
    }

    let Some(register_metadata) = callbacks.register_metadata else {
        return AbiStatus::INVALID;
    };
    // SAFETY: `handle` and all string views are valid for this callback call.
    let status = unsafe {
        register_metadata(
            handle,
            AbiStr::from_static("abi-fixture"),
            AbiStr::from_static("1.2.3"),
            AbiStr::from_static("ABI fixture plugin"),
        )
    };
    if !status.is_ok() {
        return status;
    }

    let Some(register_command) = callbacks.register_command else {
        return AbiStatus::INVALID;
    };
    let aliases = [AbiStr::from_static("af")];
    let arg_hints = [patinae_plugin::ffi::ARG_HINT_NONE];
    let command = AbiCommandDescriptor {
        handle: fixture_command_handle(),
        vtable: AbiCommandVTable {
            execute: Some(fixture_command_execute),
            destroy: Some(fixture_command_destroy),
        },
        name: AbiStr::from_static("abi_fixture"),
        description: AbiStr::from_static("registered through ABI v2"),
        usage: AbiStr::from_static("abi_fixture"),
        arguments: AbiStr::EMPTY,
        help: AbiStr::from_static("ABI fixture help"),
        aliases: AbiStrSlice {
            ptr: aliases.as_ptr(),
            len: aliases.len(),
        },
        arg_hints: AbiU8Slice {
            ptr: arg_hints.as_ptr(),
            len: arg_hints.len(),
        },
        runtime_requirements: 0,
    };
    // SAFETY: `command` and its borrowed slices live until the callback returns.
    let status = unsafe { register_command(handle, &command) };
    if !status.is_ok() {
        return status;
    }

    let Some(register_panel) = callbacks.register_panel else {
        return AbiStatus::INVALID;
    };
    let panel = AbiPanelDescriptor {
        handle: fixture_panel_handle(),
        vtable: AbiPanelVTable {
            snapshot: Some(fixture_panel_snapshot),
            handle_event: Some(fixture_panel_event),
            destroy: Some(fixture_panel_destroy),
        },
        id: AbiStr::from_static("abi_panel"),
        title: AbiStr::from_static("ABI Panel"),
        icon: AbiStr::from_static("A"),
        placement: PANEL_PLACEMENT_RIGHT,
        default_visible: 1,
        runtime_requirements: 0,
    };
    // SAFETY: `panel` and its borrowed strings live until the callback returns.
    let status = unsafe { register_panel(handle, &panel) };
    if !status.is_ok() {
        return status;
    }

    let Some(register_setting) = callbacks.register_setting else {
        return AbiStatus::INVALID;
    };
    let setting = AbiSettingDescriptor {
        name: AbiStr::from_static("abi_enabled"),
        setting_type: SETTING_TYPE_BOOL,
        default_value: AbiSettingValue {
            tag: SETTING_VALUE_BOOL,
            bool_value: 1,
            int_value: 0,
            float_values: [0.0; 3],
            string_value: AbiStr::EMPTY,
        },
        has_min: 0,
        min: 0.0,
        has_max: 0,
        max: 0.0,
        value_hints: patinae_plugin::ffi::AbiSettingValueHintSlice::EMPTY,
        side_effects: AbiU8Slice::EMPTY,
        object_overridable: 0,
    };
    // SAFETY: `setting` and its borrowed slices live until the callback returns.
    unsafe { register_setting(handle, &setting) }
}

unsafe extern "C" fn failing_register(
    _handle: HostRegistrarHandle,
    _callbacks: *const HostCallbacks,
) -> AbiStatus {
    AbiStatus::INVALID
}

unsafe extern "C" fn partially_failing_register(
    handle: HostRegistrarHandle,
    callbacks: *const HostCallbacks,
) -> AbiStatus {
    if callbacks.is_null() {
        return AbiStatus::INVALID;
    }
    // SAFETY: The host supplies a valid callback table for this registration call.
    let callbacks = unsafe { &*callbacks };
    let status = register_fixture_metadata(
        handle,
        callbacks,
        "partial-fixture",
        "0.1.0",
        "Must not be reported",
    );
    if status.is_ok() {
        AbiStatus::INVALID
    } else {
        status
    }
}

unsafe extern "C" fn register_reader_format_fixture(
    handle: HostRegistrarHandle,
    callbacks: *const HostCallbacks,
) -> AbiStatus {
    // SAFETY: The host supplies matching live ABI inputs for this registration call.
    let Ok(mut registrar) = (unsafe { PluginRegistrar::from_abi(handle, callbacks) }) else {
        return AbiStatus::INVALID;
    };
    registrar.set_metadata(PluginMetadata::new(
        "reader-fixture",
        "1.0.0",
        "Reader format fixture",
    ));
    registrar.register_format_handler(fixture_format("reader-only", "reader_only", true, false));
    registrar.register_format_handler(fixture_format(
        "reader-collision",
        "runtime_collision",
        true,
        false,
    ));
    registrar.register_format_handler(fixture_format(
        "load-built-in-collision",
        "xtc",
        true,
        false,
    ));
    registrar.finish()
}

unsafe extern "C" fn register_writer_format_fixture(
    handle: HostRegistrarHandle,
    callbacks: *const HostCallbacks,
) -> AbiStatus {
    // SAFETY: The host supplies matching live ABI inputs for this registration call.
    let Ok(mut registrar) = (unsafe { PluginRegistrar::from_abi(handle, callbacks) }) else {
        return AbiStatus::INVALID;
    };
    registrar.set_metadata(PluginMetadata::new(
        "writer-fixture",
        "2.0.0",
        "Writer format fixture",
    ));
    registrar.register_format_handler(fixture_format("writer-only", "writer_only", false, true));
    registrar.register_format_handler(fixture_format(
        "writer-collision",
        "runtime_collision",
        false,
        true,
    ));
    registrar.register_format_handler(fixture_format(
        "save-built-in-collision",
        "bcif",
        false,
        true,
    ));
    registrar.register_format_handler(fixture_format(
        "save-advertised-built-in-collision",
        "pdb",
        false,
        true,
    ));
    registrar.finish()
}

#[test]
fn plugin_extension_filter_matches_platform() {
    assert_eq!(
        is_plugin_library_path(Path::new("libhello_plugin.dylib")),
        cfg!(target_os = "macos")
    );
    assert_eq!(
        is_plugin_library_path(Path::new("libhello_plugin.so")),
        cfg!(target_os = "linux")
    );
    assert_eq!(
        is_plugin_library_path(Path::new("hello_plugin.dll")),
        cfg!(target_os = "windows")
    );
    assert!(!is_plugin_library_path(Path::new("hello_plugin.deps")));
}

#[test]
fn validates_abi_and_sdk_versions() {
    assert!(validate_declaration_versions(ABI_VERSION, SDK_VERSION).is_ok());
    assert!(validate_declaration_versions(ABI_VERSION + 1, SDK_VERSION).is_err());
    assert!(validate_declaration_versions(ABI_VERSION, "0.0.0").is_err());
}

#[test]
fn validates_full_abi_declaration_shape() {
    let declaration = test_declaration(Some(register_fixture));

    assert!(validate_declaration(&declaration).is_ok());
}

#[test]
fn rejects_invalid_abi_declarations_without_dereferencing_bad_lengths() {
    let mut declaration = test_declaration(Some(register_fixture));
    declaration.sdk_version = AbiStr {
        ptr: std::ptr::null(),
        len: 1,
    };
    assert!(validate_declaration(&declaration)
        .unwrap_err()
        .contains("sdk_version pointer was null"));

    declaration.sdk_version = AbiStr {
        ptr: std::ptr::NonNull::<u8>::dangling().as_ptr(),
        len: MAX_ABI_STRING_LEN + 1,
    };
    assert!(validate_declaration(&declaration)
        .unwrap_err()
        .contains("sdk_version length exceeds ABI limit"));

    static INVALID_UTF8: [u8; 1] = [0xff];
    declaration.sdk_version = AbiStr {
        ptr: INVALID_UTF8.as_ptr(),
        len: INVALID_UTF8.len(),
    };
    assert!(validate_declaration(&declaration)
        .unwrap_err()
        .contains("sdk_version was not valid UTF-8"));
}

#[test]
fn rejects_missing_register_callback_and_capability_mismatch() {
    let mut declaration = test_declaration(None);
    assert!(validate_declaration(&declaration)
        .unwrap_err()
        .contains("missing register callback"));

    declaration = test_declaration(Some(register_fixture));
    declaration.capabilities = 0;
    assert!(validate_declaration(&declaration)
        .unwrap_err()
        .contains("missing registration capability"));

    declaration.capabilities = test_capabilities() | (1 << 62);
    assert!(validate_declaration(&declaration)
        .unwrap_err()
        .contains("unknown capabilities"));
}

#[test]
fn loads_abi_fixture_into_host_owned_registrations() {
    let declaration = test_declaration(Some(register_fixture));
    let mut executor = CommandExecutor::new();
    let mut host = PluginHost::new();

    let name = load_declaration_for_test(&mut host, &mut executor, declaration).unwrap();

    assert_eq!(name, "abi-fixture v1.2.3");
    assert_eq!(host.plugin_count(), 1);
    assert!(executor.registry().contains("abi_fixture"));
    assert!(executor.registry().contains("af"));
    assert!(executor.dynamic_settings().lookup("abi_enabled").is_some());
    assert!(host.has_panel("abi_panel"));
}

#[test]
fn reports_only_successfully_retained_plugin_instances_with_exact_metadata() {
    let mut executor = CommandExecutor::new();
    let mut host = PluginHost::new();

    assert_eq!(
        execute_text(&mut executor, "capabilities plugins"),
        "Loaded plugins:\n(none)"
    );

    let error = load_declaration_for_test(
        &mut host,
        &mut executor,
        test_declaration(Some(failing_register)),
    )
    .unwrap_err();
    assert!(error.contains("Plugin register failed: invalid ABI input"));
    let error = load_declaration_for_test(
        &mut host,
        &mut executor,
        test_declaration(Some(partially_failing_register)),
    )
    .unwrap_err();
    assert!(error.contains("Plugin register failed: invalid ABI input"));
    assert_eq!(host.plugin_count(), 0);
    assert_eq!(
        execute_text(&mut executor, "capabilities plugins"),
        "Loaded plugins:\n(none)"
    );

    for _ in 0..2 {
        load_declaration_for_test(
            &mut host,
            &mut executor,
            test_declaration(Some(register_fixture)),
        )
        .unwrap();
    }

    assert_eq!(host.plugin_count(), 2);
    assert_eq!(
        execute_text(&mut executor, "capabilities plugins"),
        concat!(
            "Loaded plugins:\n",
            "abi-fixture\t1.2.3\tABI fixture plugin\n",
            "abi-fixture\t1.2.3\tABI fixture plugin"
        )
    );
}

#[test]
fn reports_effective_format_access_and_current_plugin_winner() {
    let mut executor = CommandExecutor::new();
    let mut host = PluginHost::new();

    load_declaration_for_test(
        &mut host,
        &mut executor,
        test_declaration(Some(register_reader_format_fixture)),
    )
    .unwrap();

    let reader_load = execute_text(&mut executor, "capabilities formats load");
    let reader_save = execute_text(&mut executor, "capabilities formats save");
    assert!(reader_load.lines().any(|line| line == ".reader_only"));
    assert!(reader_load.lines().any(|line| line == ".runtime_collision"));
    assert!(!reader_load.lines().any(|line| line == ".xtc"));
    assert!(!reader_save.lines().any(|line| line == ".reader_only"));
    assert!(!reader_save.lines().any(|line| line == ".runtime_collision"));

    load_declaration_for_test(
        &mut host,
        &mut executor,
        test_declaration(Some(register_writer_format_fixture)),
    )
    .unwrap();

    let load = execute_text(&mut executor, "capabilities formats load");
    let save = execute_text(&mut executor, "capabilities formats save");
    assert!(load.lines().any(|line| line == ".reader_only"));
    assert!(!load.lines().any(|line| line == ".writer_only"));
    assert!(!load.lines().any(|line| line == ".runtime_collision"));
    assert!(!save.lines().any(|line| line == ".reader_only"));
    assert!(save.lines().any(|line| line == ".writer_only"));
    assert!(save.lines().any(|line| line == ".runtime_collision"));
    assert_eq!(save.lines().filter(|line| *line == ".pdb").count(), 1);
    assert_eq!(save.lines().filter(|line| *line == ".pdb.gz").count(), 1);
    assert!(save.lines().any(|line| line == ".pml"));
    assert!(save.lines().any(|line| line == ".prs"));
    for unsupported in [
        ".pml.gz", ".prs.gz", ".bcif", ".bcif.gz", ".map", ".map.gz", ".xtc", ".xtc.gz",
    ] {
        assert!(!save.lines().any(|line| line == unsupported));
    }
}

#[test]
fn plugin_register_failure_is_reported() {
    let declaration = test_declaration(Some(failing_register));
    let mut executor = CommandExecutor::new();
    let mut host = PluginHost::new();

    let error = load_declaration_for_test(&mut host, &mut executor, declaration).unwrap_err();

    assert!(error.contains("Plugin register failed: invalid ABI input"));
}

#[test]
fn panel_visibility_keeps_one_active_tab_per_placement() {
    let right_a = PanelDescriptor::right("right-a", "Right A").default_visible(true);
    let right_b = PanelDescriptor::right("right-b", "Right B").default_visible(false);
    let bottom = PanelDescriptor::bottom("bottom", "Bottom").default_visible(true);
    let mut host = host_with_panels(vec![
        loaded_panel(right_a),
        loaded_panel(right_b),
        loaded_panel(bottom),
    ]);

    assert_panel(&host, "right-a", true, true);
    assert_panel(&host, "right-b", false, false);
    assert_panel(&host, "bottom", true, true);

    host.show_panel("right-b");
    assert_panel(&host, "right-a", true, false);
    assert_panel(&host, "right-b", true, true);
    assert_panel(&host, "bottom", true, true);

    host.hide_panel("right-b");
    assert_panel(&host, "right-a", true, true);
    assert_panel(&host, "right-b", false, false);
}

#[test]
fn panel_generation_increments_only_on_real_visibility_changes() {
    let right = PanelDescriptor::right("right", "Right").default_visible(false);
    let mut host = host_with_panels(vec![loaded_panel(right)]);
    let initial = host.panel_ui_generation();

    assert!(!host.hide_panel("right"));
    assert_eq!(host.panel_ui_generation(), initial);

    assert!(host.toggle_panel("right"));
    let opened = host.panel_ui_generation();
    assert!(opened > initial);
    assert_panel(&host, "right", true, true);

    assert!(!host.show_panel("right"));
    assert_eq!(host.panel_ui_generation(), opened);

    assert!(host.toggle_panel("right"));
    assert!(host.panel_ui_generation() > opened);
    assert_panel(&host, "right", false, false);
}

#[test]
fn transient_panel_events_do_not_invalidate_snapshots() {
    let panel = PanelDescriptor::right("panel", "Panel").default_visible(true);
    let mut host = host_with_panels(vec![loaded_panel(panel)]);
    let fixture = SharedFixture::new();
    let mut bus = MessageBus::new();
    let initial = host.panel_ui_generation();

    host.queue_panel_event(PanelEvent {
        panel_id: "panel".into(),
        control_id: "script".into(),
        kind: PanelEventKind::TextEdit,
        value: PanelValue::Text("pri".into()),
    });
    host.poll_all(&fixture.shared(), &mut bus);

    assert_eq!(host.panel_ui_generation(), initial);
}

#[test]
fn text_area_panel_events_invalidate_snapshots() {
    let panel = PanelDescriptor::right("panel", "Panel").default_visible(true);
    let mut host = host_with_panels(vec![loaded_panel(panel)]);
    let fixture = SharedFixture::new();
    let mut bus = MessageBus::new();
    let initial = host.panel_ui_generation();

    host.queue_panel_event(PanelEvent {
        panel_id: "panel".into(),
        control_id: "script".into(),
        kind: PanelEventKind::TextAreaEdit,
        value: PanelValue::Text("print('hi')".into()),
    });
    host.poll_all(&fixture.shared(), &mut bus);

    assert!(host.panel_ui_generation() > initial);
}

#[test]
fn panel_snapshots_are_reused_for_same_generation() {
    let count = Arc::new(AtomicUsize::new(0));
    let descriptor = PanelDescriptor::right("right", "Right").default_visible(true);
    let mut host = host_with_panels(vec![LoadedPanel {
        active: true,
        visible: true,
        cached_snapshot_generation: None,
        cached_snapshot: PanelSnapshot::default(),
        panel: Box::new(StaticPanel::with_snapshot_count(
            descriptor.clone(),
            count.clone(),
        )),
        descriptor,
    }]);
    let fixture = SharedFixture::new();

    host.panel_frames(&fixture.shared(), 42);
    host.panel_frames(&fixture.shared(), 42);
    assert_eq!(count.load(Ordering::Relaxed), 1);

    host.panel_frames(&fixture.shared(), 43);
    assert_eq!(count.load(Ordering::Relaxed), 2);
}

#[test]
fn deactivating_bottom_plugins_does_not_touch_right_plugins() {
    let right = PanelDescriptor::right("right", "Right").default_visible(true);
    let bottom = PanelDescriptor::bottom("bottom", "Bottom").default_visible(true);
    let mut host = host_with_panels(vec![loaded_panel(right), loaded_panel(bottom)]);

    host.deactivate_placement(PanelPlacement::Bottom);

    assert_panel(&host, "right", true, true);
    assert_panel(&host, "bottom", false, false);
}

#[test]
fn panel_events_emit_commands_and_setting_actions() {
    let panel = PanelDescriptor::right("panel", "Panel").default_visible(true);
    let actions = vec![
        PanelAction::ExecuteCommand {
            command: "do_work".into(),
            silent: false,
        },
        PanelAction::SetSetting {
            name: "test_setting".into(),
            value: PanelValue::Bool(true),
        },
    ];
    let mut host = host_with_panels(vec![LoadedPanel {
        active: true,
        visible: true,
        cached_snapshot_generation: None,
        cached_snapshot: PanelSnapshot::default(),
        panel: Box::new(StaticPanel::with_actions(panel.clone(), actions)),
        descriptor: panel,
    }]);
    let fixture = SharedFixture::new();
    let mut bus = MessageBus::new();

    host.queue_panel_event(PanelEvent {
        panel_id: "panel".into(),
        control_id: "button".into(),
        kind: PanelEventKind::Click,
        value: PanelValue::None,
    });
    host.poll_all(&fixture.shared(), &mut bus);

    let messages = bus.drain_outbox();
    assert_eq!(messages.len(), 2);
    assert!(matches!(
        &messages[0],
        AppMessage::ExecuteCommand { command, silent: false } if command == "do_work"
    ));
    assert!(matches!(
        &messages[1],
        AppMessage::ExecuteCommand { command, silent: true } if command == "set test_setting, 1"
    ));
}

#[test]
fn dynamic_command_registration_and_unregistration_drain_into_executor() {
    let mut host = PluginHost::new();
    let mut executor = CommandExecutor::new();

    host.pending_registrations.push(DynCmdRegistration {
        name: "dynamic_test".into(),
        description: "Dynamic test".into(),
        usage: "dynamic_test".into(),
        arguments: String::new(),
    });
    host.apply_dynamic_command_changes(&mut executor);
    assert!(executor.registry().contains("dynamic_test"));

    host.pending_unregistrations.push("dynamic_test".into());
    host.apply_dynamic_command_changes(&mut executor);
    assert!(!executor.registry().contains("dynamic_test"));
}

#[test]
fn idle_poll_does_not_queue_full_session_mutation() {
    let mut host = host_with_message_handler(Box::new(IdleHandler));
    let fixture = SharedFixture::new();
    let registry_generation = fixture.session.registry.generation();
    let selection_generation = fixture.session.selections.generation();
    let mut bus = MessageBus::new();

    host.poll_all(&fixture.shared(), &mut bus);

    assert!(host.take_pending_mutations().is_empty());
    assert!(host.notification_messages().is_empty());
    assert_eq!(fixture.session.registry.generation(), registry_generation);
    assert_eq!(
        fixture.session.selections.generation(),
        selection_generation
    );
}

#[test]
fn host_queries_return_results_on_next_poll() {
    let seen = Arc::new(std::sync::Mutex::new(None));
    let mut host = host_with_message_handler(Box::new(QueryNamesHandler {
        requested: false,
        seen: seen.clone(),
    }));
    let mut fixture = SharedFixture::new();
    fixture
        .session
        .registry
        .add(GroupObject::new("query_group"));
    let mut bus = MessageBus::new();

    host.poll_all(&fixture.shared(), &mut bus);
    assert!(seen.lock().unwrap().is_none());

    host.poll_all(&fixture.shared(), &mut bus);
    assert_eq!(*seen.lock().unwrap(), Some(vec!["query_group".to_string()]));
}

#[test]
fn host_query_results_are_scoped_to_requesting_plugin() {
    let first_count = Arc::new(std::sync::Mutex::new(None));
    let second_count = Arc::new(std::sync::Mutex::new(None));
    let mut host = host_with_message_handlers(vec![
        Box::new(QueryIsolationHandler {
            query: WireHostQuery::ObjectNames { id: 1 },
            requested: false,
            result_count: first_count.clone(),
        }),
        Box::new(QueryIsolationHandler {
            query: WireHostQuery::View { id: 1 },
            requested: false,
            result_count: second_count.clone(),
        }),
    ]);
    let fixture = SharedFixture::new();
    let mut bus = MessageBus::new();

    host.poll_all(&fixture.shared(), &mut bus);
    host.poll_all(&fixture.shared(), &mut bus);

    assert_eq!(*first_count.lock().unwrap(), Some(1));
    assert_eq!(*second_count.lock().unwrap(), Some(1));
}

#[test]
fn host_atom_stream_reads_chunks_and_closes_on_done() {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut host = host_with_message_handler(Box::new(AtomStreamLifecycleHandler {
        phase: 0,
        stream_id: None,
        seen: seen.clone(),
    }));
    let mut fixture = SharedFixture::new();
    add_test_molecule(&mut fixture.session);
    let mut bus = MessageBus::new();

    for _ in 0..5 {
        host.poll_all(&fixture.shared(), &mut bus);
    }

    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen[0], "CA");
    assert_eq!(seen[1], "CB");
    assert!(seen[2].contains("is not open"));
}

#[test]
fn host_atom_streams_are_isolated_per_plugin() {
    let error = Arc::new(std::sync::Mutex::new(None));
    let mut host = host_with_message_handlers(vec![
        Box::new(IdleHandler),
        Box::new(AtomStreamErrorHandler {
            query: WireHostQuery::ReadAtomStream {
                id: 1,
                stream_id: 1,
                max_rows: 1,
            },
            requested: false,
            error: error.clone(),
        }),
    ]);
    let mut fixture = SharedFixture::new();
    add_test_molecule(&mut fixture.session);
    let mut bus = MessageBus::new();

    host.poll_all(&fixture.shared(), &mut bus);
    host.poll_all(&fixture.shared(), &mut bus);

    let error = error.lock().unwrap().clone().unwrap();
    assert!(error.contains("atom stream 1 is not open"));
}

#[test]
fn host_atom_stream_reports_stale_topology() {
    let error = Arc::new(std::sync::Mutex::new(None));
    let mut host = host_with_message_handler(Box::new(StaleAtomStreamHandler {
        phase: 0,
        stream_id: None,
        error: error.clone(),
    }));
    let mut fixture = SharedFixture::new();
    add_test_molecule(&mut fixture.session);
    let mut bus = MessageBus::new();

    host.poll_all(&fixture.shared(), &mut bus);
    fixture
        .session
        .registry
        .get_molecule_mut("obj")
        .unwrap()
        .molecule_mut()
        .add_atom(Atom::default());
    host.poll_all(&fixture.shared(), &mut bus);
    host.poll_all(&fixture.shared(), &mut bus);

    let error = error.lock().unwrap().clone().unwrap();
    assert!(error.contains("stale"));
}

fn assert_panel(host: &PluginHost, id: &str, visible: bool, active: bool) {
    let status = host
        .panel_statuses()
        .into_iter()
        .find(|panel| panel.descriptor.id == id)
        .unwrap_or_else(|| panic!("missing panel {id}"));
    assert_eq!(status.visible, visible, "visibility for panel {id}");
    assert_eq!(status.active, active, "active state for panel {id}");
}

#[test]
#[ignore = "requires PATINAE_PLUGIN_TEST_DIR pointing at built plugin libraries"]
fn loads_built_reference_plugins() {
    let dir = std::env::var("PATINAE_PLUGIN_TEST_DIR")
        .expect("set PATINAE_PLUGIN_TEST_DIR to target/release/plugins");
    let mut executor = CommandExecutor::new();
    let mut host = PluginHost::new();
    let mut errors = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("plugin test dir exists") {
        let path = entry.expect("plugin dir entry").path();
        if !is_plugin_library_path(&path) {
            continue;
        }
        if let Err(e) = host.load_library(&path, &mut executor) {
            errors.push(format!("{}: {e}", path.display()));
        }
    }

    assert!(
        errors.is_empty(),
        "plugin load errors:\n{}",
        errors.join("\n")
    );
    assert!(host.plugin_count() >= 3);
    assert!(executor.registry().contains("hello"));
    assert!(executor.registry().contains("ray"));
    assert!(host.panel_statuses().iter().any(|panel| {
        panel.descriptor.id == "rt_toolbar" && panel.descriptor.placement == PanelPlacement::Right
    }));
    assert!(host.toggle_panel("rt_toolbar"));
    assert_panel(&host, "rt_toolbar", true, true);
    assert!(host.toggle_panel("rt_toolbar"));
    assert_panel(&host, "rt_toolbar", false, false);
}
