use std::collections::{HashMap, VecDeque};
use std::ffi::c_void;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use libloading::Library;
use wgpu::util::DeviceExt;

use patinae_cmd::{
    ArgHint, CmdError, CmdResult, Command, CommandAction, CommandContext, CommandExecutor,
    CommandRuntimeRequirements, DynamicSettingRegistry, FormatHandler, LoadedPluginCapability,
    MessageKind, ParsedCommand, PluginReaderFn, PluginWriterFn, ScriptHandler,
};
use patinae_framework::component::SharedContext;
use patinae_framework::message::{AppMessage, MessageBus};
use patinae_framework::plugin_ui::{
    PanelControl, PanelDescriptor, PanelEvent, PanelPlacement, PanelRuntimeRequirements,
    PanelSnapshot, PluginPanel,
};
use patinae_mol::ObjectMolecule;
use patinae_plugin::ffi::{
    AbiBytesSinkFn, AbiCommandDescriptor, AbiCommandVTable, AbiFormatHandlerDescriptor,
    AbiFormatHandlerVTable, AbiHotkeyDescriptor, AbiHotkeyVTable, AbiMessageHandlerDescriptor,
    AbiMessageHandlerVTable, AbiPanelDescriptor, AbiPanelVTable, AbiReadError,
    AbiScriptHandlerDescriptor, AbiScriptHandlerVTable, AbiSettingDescriptor, AbiSettingValue,
    AbiSettingValueHint, AbiStatus, AbiStr, AbiStrSlice, AbiU8Slice, HostCallbacks,
    HostCommandRuntimeCallbacks, HostCommandRuntimeHandle, HostRegistrarHandle,
    PluginCommandHandle, PluginDeclaration, PluginFormatHandlerHandle, PluginHotkeyHandle,
    PluginMessageHandlerHandle, PluginPanelHandle, PluginScriptHandlerHandle,
    ABI_STATUS_HOST_ERROR, ABI_STATUS_INVALID, ABI_STATUS_PANIC, ABI_STATUS_UNSUPPORTED,
    ABI_VERSION, ARG_HINT_COLOR, ARG_HINT_COMMAND, ARG_HINT_LABEL_PROPERTY,
    ARG_HINT_NAMED_SELECTION, ARG_HINT_OBJECT, ARG_HINT_PATH, ARG_HINT_REPRESENTATION,
    ARG_HINT_SELECTION, ARG_HINT_SETTING, ARG_HINT_SETTING_VALUE, CAPABILITY_REGISTRATION,
    HOST_CALLBACKS_VERSION, HOST_COMMAND_RUNTIME_CALLBACKS_VERSION, KNOWN_CAPABILITIES,
    KNOWN_COMMAND_RUNTIME_REQUIREMENTS, KNOWN_PANEL_RUNTIME_REQUIREMENTS, MAX_ABI_SLICE_LEN,
    MAX_ABI_STRING_LEN, PANEL_PLACEMENT_BOTTOM, PANEL_PLACEMENT_RIGHT, SDK_VERSION,
    SETTING_TYPE_BLANK, SETTING_TYPE_BOOL, SETTING_TYPE_COLOR, SETTING_TYPE_FLOAT,
    SETTING_TYPE_FLOAT3, SETTING_TYPE_INT, SETTING_TYPE_STRING, SETTING_VALUE_BOOL,
    SETTING_VALUE_COLOR, SETTING_VALUE_FLOAT, SETTING_VALUE_FLOAT3, SETTING_VALUE_INT,
    SETTING_VALUE_NONE, SETTING_VALUE_STRING, SIDE_EFFECT_COLOR_REBUILD, SIDE_EFFECT_FULL_REBUILD,
    SIDE_EFFECT_ORTHO_DIRTY, SIDE_EFFECT_REPRESENTATION_REBUILD, SIDE_EFFECT_SCENE_CHANGED,
    SIDE_EFFECT_SCENE_INVALIDATE, SIDE_EFFECT_SEQ_CHANGED, SIDE_EFFECT_SHADER_COMPUTE_LIGHTING,
    SIDE_EFFECT_SHADER_RELOAD, SIDE_EFFECT_STEREO_UPDATE, SIDE_EFFECT_SURFACE_TRANSPARENCY,
    SIDE_EFFECT_VIEWPORT_UPDATE,
};
use patinae_plugin::registrar::{MessageHandler, PluginKeyAction, PluginMetadata, PollContext};
use patinae_plugin::wire::{
    self, WireCommandInput, WireCommandOutput, WireCommandRuntimeRequest,
    WireCommandRuntimeResponse, WireCommandRuntimeValue, WireFormatReadInput, WireFormatReadOutput,
    WireFormatWriteInput, WireFormatWriteOutput, WireHotkeyAction, WireMessageInput,
    WireMessageOutput, WirePanelEventInput, WirePanelEventOutput, WirePanelSnapshotOutput,
    WirePollInput, WirePollOutput, WireScriptInput, WireScriptOutput, WireSharedInput,
    WireTraceGeometryOpened, WireViewerAction, RUNTIME_WIRE_VERSION,
};
use patinae_scene::{
    parse_key_string, GpuAddressMode, GpuBatchCommand, GpuBatchResult, GpuBindGroupDescriptor,
    GpuBindGroupEntry, GpuBindGroupLayoutDescriptor, GpuBindGroupLayoutEntry, GpuBindingResource,
    GpuBindingType, GpuBlendState, GpuBufferBindingType, GpuBufferDescriptor, GpuBufferUsage,
    GpuCacheStats, GpuCacheStatus, GpuCachedHandle, GpuColor, GpuColorLoadOp, GpuColorOperations,
    GpuColorTargetState, GpuColorWriteMask, GpuCompareFunction, GpuComputePipelineDescriptor,
    GpuDepthLoadOp, GpuDepthOperations, GpuDepthStencilState, GpuDeviceLimits, GpuExtent3d,
    GpuFace, GpuFilterMode, GpuFrontFace, GpuHandle, GpuHandleKind, GpuIndexFormat,
    GpuMultisampleState, GpuOrigin3d, GpuPipelineLayoutDescriptor, GpuPrimitiveState,
    GpuPrimitiveTopology, GpuRenderPassCommand, GpuRenderPassDepthStencilAttachment,
    GpuRenderPassDescriptor, GpuRenderPipelineDescriptor, GpuSamplerBindingType,
    GpuSamplerDescriptor, GpuShaderModuleDescriptor, GpuShaderStages, GpuStorageTextureAccess,
    GpuStoreOp, GpuSubmitBatch, GpuTexelCopyBufferLayout, GpuTexelCopyTextureInfo,
    GpuTextureAspect, GpuTextureDescriptor, GpuTextureDimension, GpuTextureFormat,
    GpuTextureSampleType, GpuTextureUsage, GpuTextureViewDescriptor, GpuTextureViewDimension,
    GpuVertexAttribute, GpuVertexBufferLayout, GpuVertexFormat, GpuVertexState, GpuVertexStepMode,
    KeyBindings, RenderArtifactBufferDescriptor, RenderArtifactBufferRole,
    RenderArtifactPrimitiveTopology, RenderArtifactRepDescriptor, RenderArtifactRepKind,
    RenderArtifactSnapshotDescriptor, Session, ViewerLike,
};
use patinae_settings::{
    DynamicSettingDescriptor, DynamicSettingStore, SettingType, SettingValue, SharedSettingStore,
    SideEffectCategory,
};
use serde::{de::DeserializeOwned, Serialize};

use crate::host::PluginHost;
use crate::panic::panic_payload_to_string;
use crate::paths::{is_plugin_library_path, PluginDiscovery};
use crate::plugin::{LibraryHandle, LoadedPanel, LoadedPlugin};
use crate::runtime::apply_atom_property_change_batch;

mod gpu_validation;
mod render_artifacts;

use gpu_validation::*;
use render_artifacts::*;

/// Vertices per spooled mesh chunk.
///
/// `DisplayedMesh` is a triangle list. The value is divisible by three so
/// chunks never split a triangle, and it keeps encoded chunks far below the
/// 64 MiB ABI payload ceiling.
const DISPLAYED_GEOMETRY_SPOOL_MESH_VERTICES: usize = 24_576;

/// Non-mesh primitives per spooled chunk.
///
/// Analytic primitives encode much smaller than mesh vertices; this still
/// leaves wide headroom below the ABI payload ceiling.
const DISPLAYED_GEOMETRY_SPOOL_PRIMITIVES: usize = 32_768;

static DISPLAYED_GEOMETRY_SPOOL_COUNTER: AtomicU64 = AtomicU64::new(0);

type SharedGpuPluginCache = Arc<Mutex<GpuPluginCache>>;

const GPU_CACHE_LAYOUT_VERSION: u32 = 1;
const DRAW_INDIRECT_SIZE: u64 = 16;
const DRAW_INDEXED_INDIRECT_SIZE: u64 = 20;
const DISPATCH_INDIRECT_SIZE: u64 = 12;

fn rt_profile_enabled() -> bool {
    std::env::var_os("PATINAE_RT_PROFILE").is_some()
}

fn rt_profile_stages_enabled() -> bool {
    rt_profile_enabled() && std::env::var_os("PATINAE_RT_PROFILE_STAGES").is_some()
}

#[derive(Debug, Clone)]
struct GpuStageTimestamp {
    scope: String,
    begin_index: u32,
    end_index: u32,
}

struct GpuStageProfile {
    unavailable: Option<&'static str>,
    query_set: Option<wgpu::QuerySet>,
    resolve_buffer: Option<wgpu::Buffer>,
    readback_buffer: Option<wgpu::Buffer>,
    timestamp_period_ns: f32,
    next_query_index: u32,
    timestamps: Vec<GpuStageTimestamp>,
    current_scope: String,
}

impl GpuStageProfile {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, commands: &[GpuBatchCommand]) -> Self {
        if !rt_profile_stages_enabled() {
            return Self::inactive();
        }
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return Self::unavailable("timestamp_query_missing");
        }
        let dispatch_count = commands
            .iter()
            .filter(|command| {
                matches!(
                    command,
                    GpuBatchCommand::DispatchCompute { .. }
                        | GpuBatchCommand::DispatchComputeIndirect { .. }
                )
            })
            .count();
        if dispatch_count == 0 {
            return Self::unavailable("no_compute_dispatches");
        }
        let Some(query_count) = dispatch_count
            .checked_mul(2)
            .and_then(|count| u32::try_from(count).ok())
        else {
            return Self::unavailable("too_many_compute_dispatches");
        };
        let size = u64::from(query_count) * std::mem::size_of::<u64>() as u64;
        let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("patinae.plugin.gpu.batch.stage_timestamps"),
            ty: wgpu::QueryType::Timestamp,
            count: query_count,
        });
        let resolve_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("patinae.plugin.gpu.batch.stage_timestamp_resolve"),
            size,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("patinae.plugin.gpu.batch.stage_timestamp_readback"),
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Self {
            unavailable: None,
            query_set: Some(query_set),
            resolve_buffer: Some(resolve_buffer),
            readback_buffer: Some(readback_buffer),
            timestamp_period_ns: queue.get_timestamp_period(),
            next_query_index: 0,
            timestamps: Vec::with_capacity(dispatch_count),
            current_scope: "unscoped".to_string(),
        }
    }

    fn inactive() -> Self {
        Self {
            unavailable: None,
            query_set: None,
            resolve_buffer: None,
            readback_buffer: None,
            timestamp_period_ns: 0.0,
            next_query_index: 0,
            timestamps: Vec::new(),
            current_scope: "unscoped".to_string(),
        }
    }

    fn unavailable(reason: &'static str) -> Self {
        let mut profile = Self::inactive();
        profile.unavailable = Some(reason);
        profile
    }

    fn set_scope(&mut self, name: String) {
        self.current_scope = name;
    }

    fn timestamp_writes(&mut self) -> Option<wgpu::ComputePassTimestampWrites<'_>> {
        let query_set = self.query_set.as_ref()?;
        let begin_index = self.next_query_index;
        let end_index = begin_index.checked_add(1)?;
        self.next_query_index = end_index.checked_add(1)?;
        self.timestamps.push(GpuStageTimestamp {
            scope: self.current_scope.clone(),
            begin_index,
            end_index,
        });
        Some(wgpu::ComputePassTimestampWrites {
            query_set,
            beginning_of_pass_write_index: Some(begin_index),
            end_of_pass_write_index: Some(end_index),
        })
    }

    fn resolve(&self, encoder: &mut wgpu::CommandEncoder) {
        let (Some(query_set), Some(resolve_buffer), Some(readback_buffer)) = (
            self.query_set.as_ref(),
            self.resolve_buffer.as_ref(),
            self.readback_buffer.as_ref(),
        ) else {
            return;
        };
        if self.next_query_index == 0 {
            return;
        }
        let size = u64::from(self.next_query_index) * std::mem::size_of::<u64>() as u64;
        encoder.resolve_query_set(query_set, 0..self.next_query_index, resolve_buffer, 0);
        encoder.copy_buffer_to_buffer(resolve_buffer, 0, readback_buffer, 0, size);
    }

    fn readback_bytes(&self) -> u64 {
        if self.query_set.is_none() {
            return 0;
        }
        u64::from(self.next_query_index) * std::mem::size_of::<u64>() as u64
    }

    fn log_after_submit(&self, device: &wgpu::Device) -> Result<u64, String> {
        if !rt_profile_stages_enabled() {
            return Ok(0);
        }
        if let Some(reason) = self.unavailable {
            log::info!("patinae.rt_profile host.gpu_stage_ms unavailable={reason}");
            return Ok(0);
        }
        let Some(readback_buffer) = self.readback_buffer.as_ref() else {
            return Ok(0);
        };
        let readback_bytes = self.readback_bytes();
        if readback_bytes == 0 {
            log::info!("patinae.rt_profile host.gpu_stage_ms unavailable=no_timestamp_records");
            return Ok(0);
        }
        let bytes = map_readback_buffer(device, readback_buffer.clone(), readback_bytes)?;
        let ticks = decode_timestamp_ticks(&bytes);
        let by_scope =
            aggregate_stage_timestamp_ms(&ticks, &self.timestamps, self.timestamp_period_ns);
        let stage = |name: &str| by_scope.get(name).copied().unwrap_or(0.0);
        log::info!(
            "patinae.rt_profile host.gpu_stage_ms primitive_build={:.3} surface_compact={:.3} metadata_finalize={:.3} bvh_build={:.3} raytrace={:.3} downsample={:.3} unscoped={:.3} dispatches={} timestamp_readback_bytes={}",
            stage("primitive_build"),
            stage("surface_compact"),
            stage("metadata_finalize"),
            stage("bvh_build"),
            stage("raytrace"),
            stage("downsample"),
            stage("unscoped"),
            self.timestamps.len(),
            readback_bytes
        );
        Ok(readback_bytes)
    }
}

fn decode_timestamp_ticks(bytes: &[u8]) -> Vec<u64> {
    bytes
        .as_chunks::<{ std::mem::size_of::<u64>() }>()
        .0
        .iter()
        .map(|&chunk| u64::from_ne_bytes(chunk))
        .collect()
}

fn aggregate_stage_timestamp_ms(
    ticks: &[u64],
    timestamps: &[GpuStageTimestamp],
    timestamp_period_ns: f32,
) -> HashMap<String, f64> {
    let mut by_scope = HashMap::<String, f64>::new();
    for timestamp in timestamps {
        let Some(begin) = ticks.get(timestamp.begin_index as usize) else {
            continue;
        };
        let Some(end) = ticks.get(timestamp.end_index as usize) else {
            continue;
        };
        let elapsed_ticks = end.saturating_sub(*begin);
        let elapsed_ms = (elapsed_ticks as f64) * f64::from(timestamp_period_ns) / 1_000_000.0;
        *by_scope.entry(timestamp.scope.clone()).or_insert(0.0) += elapsed_ms;
    }
    by_scope
}

static HOST_CALLBACKS: HostCallbacks = HostCallbacks {
    table_version: HOST_CALLBACKS_VERSION,
    register_metadata: Some(host_register_metadata),
    register_command: Some(host_register_command),
    register_panel: Some(host_register_panel),
    register_setting: Some(host_register_setting),
    register_message_handler: Some(host_register_message_handler),
    register_script_handler: Some(host_register_script_handler),
    register_format_handler: Some(host_register_format_handler),
    register_hotkey: Some(host_register_hotkey),
    report_unsupported: Some(host_report_unsupported),
};

static HOST_COMMAND_RUNTIME_CALLBACKS: HostCommandRuntimeCallbacks = HostCommandRuntimeCallbacks {
    table_version: HOST_COMMAND_RUNTIME_CALLBACKS_VERSION,
    request: Some(host_command_runtime_request),
};

impl PluginHost {
    pub fn load_standard_dirs(&mut self, executor: &mut CommandExecutor) {
        self.load_discovered_dirs(&PluginDiscovery::from_process_env(), executor);
    }

    pub fn load_discovered_dirs(
        &mut self,
        discovery: &PluginDiscovery,
        executor: &mut CommandExecutor,
    ) {
        for dir in discovery.standard_plugin_dirs() {
            self.load_dir(&dir, executor);
        }
    }

    pub fn load_dir(&mut self, dir: &Path, executor: &mut CommandExecutor) {
        self.add_plugin_dir(dir);

        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) => {
                log::debug!("Plugin directory {:?}: {}", dir, e);
                return;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !is_plugin_library_path(&path) {
                continue;
            }
            match self.load_library(&path, executor) {
                Ok(name) => log::info!("Loaded plugin: {}", name),
                Err(e) => log::warn!("Failed to load plugin {:?}: {}", path, e),
            }
        }
    }

    pub fn load_library(
        &mut self,
        path: &Path,
        executor: &mut CommandExecutor,
    ) -> Result<String, String> {
        #[cfg(target_os = "windows")]
        crate::paths::apply_deps_search_paths(path);

        // SAFETY: Loading a plugin is the explicit native-code extension point.
        // The resulting handle is kept alive in LoadedPlugin while needed.
        let library =
            unsafe { Library::new(path).map_err(|e| format!("Failed to load library: {e}"))? };

        let declaration = load_declaration(&library)?;
        validate_declaration(&declaration)?;
        initialize_plugin(&declaration)?;

        let mut registration = RegistrationSink::new();
        register_plugin(&declaration, &mut registration)?;

        finish_registration(
            self,
            executor,
            LibraryHandle::Dynamic(library),
            registration,
        )
    }
}

#[cfg(test)]
pub(crate) fn load_declaration_for_test(
    host: &mut PluginHost,
    executor: &mut CommandExecutor,
    declaration: PluginDeclaration,
) -> Result<String, String> {
    validate_declaration(&declaration)?;
    initialize_plugin(&declaration)?;

    let mut registration = RegistrationSink::new();
    register_plugin(&declaration, &mut registration)?;

    finish_registration(host, executor, LibraryHandle::Static, registration)
}

fn finish_registration(
    host: &mut PluginHost,
    executor: &mut CommandExecutor,
    library_handle: LibraryHandle,
    mut registration: RegistrationSink,
) -> Result<String, String> {
    let metadata = registration
        .metadata
        .take()
        .ok_or("Plugin did not set metadata")?;
    let plugin_name = format!("{} v{}", metadata.name, metadata.version);
    let plugin_id = format!("{}@{}", metadata.name, metadata.version);
    let gpu_cache = Arc::new(Mutex::new(GpuPluginCache::new(plugin_id)));
    let library = Arc::new(library_handle);

    install_registration_assets(executor, &mut registration, library.clone(), gpu_cache);

    let mut panels = Vec::new();
    for panel in registration.panels {
        if host.panel_exists(&panel.descriptor.id)
            || panels
                .iter()
                .any(|p: &LoadedPanel| p.descriptor.id == panel.descriptor.id)
        {
            log::warn!(
                "Skipping duplicate plugin panel id '{}'",
                panel.descriptor.id
            );
            continue;
        }
        let descriptor = panel.descriptor.clone();
        panels.push(LoadedPanel::new(
            descriptor,
            Box::new(AbiPanelProxy::new(panel, library.clone())),
        ));
    }

    let mut hotkeys = KeyBindings::new();
    for hotkey in registration.hotkeys {
        let key_str = hotkey.key.clone();
        let action = hotkey.into_action(library.clone());
        match parse_key_string(&key_str) {
            Ok(key) => hotkeys.bind(key, action),
            Err(error) => log::warn!("Invalid plugin hotkey string '{}': {}", key_str, error),
        }
    }

    let message_handler = registration.message_handler.map(|handler| {
        Box::new(AbiMessageHandlerProxy::new(handler, library.clone())) as Box<dyn MessageHandler>
    });

    for diagnostic in &registration.diagnostics {
        log::warn!("Plugin '{}': {}", metadata.name, diagnostic);
    }

    host.plugins.push(LoadedPlugin {
        _library: library,
        metadata,
        message_handler,
        panels,
        hotkeys,
        atom_streams: Default::default(),
        faulted: false,
    });
    let retained_metadata = &host
        .plugins
        .last()
        .expect("plugin was just retained by the host")
        .metadata;
    executor.record_loaded_plugin_capability(LoadedPluginCapability {
        name: retained_metadata.name.clone(),
        version: retained_metadata.version.clone(),
        description: retained_metadata.description.clone(),
    });
    host.ensure_single_active_per_placement();
    host.bump_panel_ui_generation();

    Ok(plugin_name)
}

fn load_declaration(library: &Library) -> Result<PluginDeclaration, String> {
    // SAFETY: The plugin is expected to export PATINAE_PLUGIN_DECLARATION as a
    // data symbol containing a pointer to a static PluginDeclaration.
    unsafe {
        let symbol = library
            .get::<*const PluginDeclaration>(b"PATINAE_PLUGIN_DECLARATION")
            .map_err(|e| format!("Missing PATINAE_PLUGIN_DECLARATION symbol: {e}"))?;
        let ptr = *symbol;
        if ptr.is_null() {
            return Err("PATINAE_PLUGIN_DECLARATION symbol was null".to_string());
        }
        Ok(*ptr)
    }
}

fn initialize_plugin(declaration: &PluginDeclaration) -> Result<(), String> {
    let Some(init) = declaration.init else {
        return Ok(());
    };
    let init_result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: The declaration passed validation, and HOST_CALLBACKS is a
        // process-static callback table valid for the duration of this call.
        unsafe { init(&HOST_CALLBACKS) }
    }));
    match init_result {
        Ok(status) => status_to_result("init", status),
        Err(panic_info) => Err(format!(
            "Plugin panicked during init: {}",
            panic_payload_to_string(&panic_info)
        )),
    }
}

fn register_plugin(
    declaration: &PluginDeclaration,
    registration: &mut RegistrationSink,
) -> Result<(), String> {
    let register = declaration
        .register
        .ok_or("Plugin declaration missing register callback")?;
    let handle = HostRegistrarHandle(registration as *mut RegistrationSink as *mut c_void);
    let register_result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: The declaration passed validation. The opaque handle points to
        // `registration`, which outlives this call, and HOST_CALLBACKS is static.
        unsafe { register(handle, &HOST_CALLBACKS) }
    }));
    match register_result {
        Ok(status) => status_to_result("register", status),
        Err(panic_info) => Err(format!(
            "Plugin panicked during register: {}",
            panic_payload_to_string(&panic_info)
        )),
    }
}

fn install_registration_assets(
    executor: &mut CommandExecutor,
    registration: &mut RegistrationSink,
    library: Arc<LibraryHandle>,
    gpu_cache: SharedGpuPluginCache,
) {
    for command in std::mem::take(&mut registration.commands) {
        executor
            .registry_mut()
            .register_boxed(Box::new(AbiCommandProxy::new(
                command,
                library.clone(),
                gpu_cache.clone(),
            )));
    }

    for script in std::mem::take(&mut registration.script_handlers) {
        executor.register_script_handler(
            script.extension.clone(),
            AbiScriptHandlerProxy::into_handler(script, library.clone()),
        );
    }

    for format in std::mem::take(&mut registration.format_handlers) {
        executor
            .register_format_handler(AbiFormatHandlerProxy::into_handler(format, library.clone()));
    }

    for (descriptor, store) in std::mem::take(&mut registration.settings) {
        if let Err(e) = executor.dynamic_settings_mut().register(descriptor, store) {
            log::warn!("Failed to register plugin setting: {}", e);
        }
    }
}

pub fn validate_declaration_versions(abi_version: u32, sdk_version: &str) -> Result<(), String> {
    if abi_version != ABI_VERSION {
        return Err(format!(
            "ABI version mismatch: plugin has {}, host expects {}",
            abi_version, ABI_VERSION
        ));
    }
    if sdk_version != SDK_VERSION {
        return Err(format!(
            "SDK version mismatch: plugin has {}, host expects {}",
            sdk_version, SDK_VERSION
        ));
    }
    Ok(())
}

pub(crate) fn validate_declaration(declaration: &PluginDeclaration) -> Result<(), String> {
    validate_declaration_versions(declaration.abi_version, &plugin_sdk_version(declaration)?)?;
    if declaration.register.is_none() {
        return Err("Plugin declaration missing register callback".to_string());
    }
    if declaration.capabilities & CAPABILITY_REGISTRATION == 0 {
        return Err("Plugin declaration missing registration capability".to_string());
    }
    let unknown = declaration.capabilities & !KNOWN_CAPABILITIES;
    if unknown != 0 {
        return Err(format!(
            "Plugin declaration has unknown capabilities: 0x{unknown:x}"
        ));
    }
    Ok(())
}

pub(crate) fn plugin_sdk_version(declaration: &PluginDeclaration) -> Result<String, String> {
    copy_str(declaration.sdk_version, "sdk_version")
}

fn status_to_result(phase: &str, status: AbiStatus) -> Result<(), String> {
    if status.is_ok() {
        return Ok(());
    }
    let message = match status.code {
        ABI_STATUS_PANIC => "panic",
        ABI_STATUS_INVALID => "invalid ABI input",
        ABI_STATUS_UNSUPPORTED => "unsupported feature",
        ABI_STATUS_HOST_ERROR => "host callback error",
        other => {
            return Err(format!(
                "Plugin {phase} returned unknown ABI status {other}"
            ))
        }
    };
    Err(format!("Plugin {phase} failed: {message}"))
}

#[derive(Default)]
struct RegistrationSink {
    metadata: Option<PluginMetadata>,
    commands: Vec<RegisteredCommand>,
    panels: Vec<RegisteredPanel>,
    message_handler: Option<RegisteredMessageHandler>,
    script_handlers: Vec<RegisteredScriptHandler>,
    format_handlers: Vec<RegisteredFormatHandler>,
    hotkeys: Vec<RegisteredHotkey>,
    settings: Vec<(DynamicSettingDescriptor, SharedSettingStore)>,
    diagnostics: Vec<String>,
}

impl RegistrationSink {
    fn new() -> Self {
        Self::default()
    }
}

struct RegisteredPanel {
    descriptor: PanelDescriptor,
    runtime_requirements: PanelRuntimeRequirements,
    handle: PluginPanelHandle,
    vtable: AbiPanelVTable,
}

unsafe extern "C" fn host_register_metadata(
    handle: HostRegistrarHandle,
    name: AbiStr,
    version: AbiStr,
    description: AbiStr,
) -> AbiStatus {
    callback_status(|| {
        let sink = sink_mut(handle)?;
        sink.metadata = Some(PluginMetadata::new(
            copy_str(name, "metadata.name")?,
            copy_str(version, "metadata.version")?,
            copy_str(description, "metadata.description")?,
        ));
        Ok(())
    })
}

unsafe extern "C" fn host_register_command(
    handle: HostRegistrarHandle,
    descriptor: *const AbiCommandDescriptor,
) -> AbiStatus {
    callback_status(|| {
        let sink = sink_mut(handle)?;
        if descriptor.is_null() {
            return Err("command descriptor pointer was null".to_string());
        }
        // SAFETY: The plugin provided a non-null descriptor pointer for the
        // duration of this callback. The host copies all borrowed fields here.
        let descriptor = unsafe { &*descriptor };
        sink.commands
            .push(RegisteredCommand::from_descriptor(descriptor)?);
        Ok(())
    })
}

unsafe extern "C" fn host_register_panel(
    handle: HostRegistrarHandle,
    descriptor: *const AbiPanelDescriptor,
) -> AbiStatus {
    callback_status(|| {
        let sink = sink_mut(handle)?;
        if descriptor.is_null() {
            return Err("panel descriptor pointer was null".to_string());
        }
        // SAFETY: The plugin provided a non-null descriptor pointer for the
        // duration of this callback. The host copies all borrowed fields here.
        let descriptor = unsafe { &*descriptor };
        validate_panel_runtime(descriptor)?;
        let panel_descriptor = panel_descriptor_from_abi(descriptor)?;
        sink.panels.push(RegisteredPanel {
            descriptor: panel_descriptor,
            runtime_requirements: PanelRuntimeRequirements::from_bits(
                descriptor.runtime_requirements,
            ),
            handle: descriptor.handle,
            vtable: descriptor.vtable,
        });
        Ok(())
    })
}

unsafe extern "C" fn host_register_setting(
    handle: HostRegistrarHandle,
    descriptor: *const AbiSettingDescriptor,
) -> AbiStatus {
    callback_status(|| {
        let sink = sink_mut(handle)?;
        if descriptor.is_null() {
            return Err("setting descriptor pointer was null".to_string());
        }
        // SAFETY: The plugin provided a non-null descriptor pointer for the
        // duration of this callback. The host copies all borrowed fields here.
        let descriptor = unsafe { &*descriptor };
        sink.settings.push(setting_from_abi(descriptor)?);
        Ok(())
    })
}

unsafe extern "C" fn host_register_message_handler(
    handle: HostRegistrarHandle,
    descriptor: *const AbiMessageHandlerDescriptor,
) -> AbiStatus {
    callback_status(|| {
        let sink = sink_mut(handle)?;
        if descriptor.is_null() {
            return Err("message handler descriptor pointer was null".to_string());
        }
        // SAFETY: The plugin provided a non-null descriptor pointer for the
        // duration of this callback. The host copies handle and vtable here.
        let descriptor = unsafe { &*descriptor };
        validate_message_handler_runtime(descriptor)?;
        sink.message_handler = Some(RegisteredMessageHandler {
            handle: descriptor.handle,
            vtable: descriptor.vtable,
        });
        Ok(())
    })
}

unsafe extern "C" fn host_register_script_handler(
    handle: HostRegistrarHandle,
    descriptor: *const AbiScriptHandlerDescriptor,
) -> AbiStatus {
    callback_status(|| {
        let sink = sink_mut(handle)?;
        if descriptor.is_null() {
            return Err("script handler descriptor pointer was null".to_string());
        }
        // SAFETY: The plugin provided a non-null descriptor pointer for the
        // duration of this callback. The host copies all borrowed fields here.
        let descriptor = unsafe { &*descriptor };
        validate_script_handler_runtime(descriptor)?;
        sink.script_handlers.push(RegisteredScriptHandler {
            extension: copy_str(descriptor.extension, "script.extension")?,
            handle: descriptor.handle,
            vtable: descriptor.vtable,
        });
        Ok(())
    })
}

unsafe extern "C" fn host_register_format_handler(
    handle: HostRegistrarHandle,
    descriptor: *const AbiFormatHandlerDescriptor,
) -> AbiStatus {
    callback_status(|| {
        let sink = sink_mut(handle)?;
        if descriptor.is_null() {
            return Err("format handler descriptor pointer was null".to_string());
        }
        // SAFETY: The plugin provided a non-null descriptor pointer for the
        // duration of this callback. The host copies all borrowed fields here.
        let descriptor = unsafe { &*descriptor };
        validate_format_handler_runtime(descriptor)?;
        sink.format_handlers.push(RegisteredFormatHandler {
            name: copy_str(descriptor.name, "format.name")?,
            extensions: copy_str_slice(descriptor.extensions, "format.extensions")?,
            handle: descriptor.handle,
            vtable: descriptor.vtable,
        });
        Ok(())
    })
}

unsafe extern "C" fn host_register_hotkey(
    handle: HostRegistrarHandle,
    descriptor: *const AbiHotkeyDescriptor,
) -> AbiStatus {
    callback_status(|| {
        let sink = sink_mut(handle)?;
        if descriptor.is_null() {
            return Err("hotkey descriptor pointer was null".to_string());
        }
        // SAFETY: The plugin provided a non-null descriptor pointer for the
        // duration of this callback. The host copies all borrowed fields here.
        let descriptor = unsafe { &*descriptor };
        sink.hotkeys
            .push(RegisteredHotkey::from_descriptor(descriptor)?);
        Ok(())
    })
}

unsafe extern "C" fn host_report_unsupported(
    handle: HostRegistrarHandle,
    feature: AbiStr,
    detail: AbiStr,
) -> AbiStatus {
    callback_status(|| {
        let sink = sink_mut(handle)?;
        let feature = copy_str(feature, "unsupported.feature")?;
        let detail = copy_str(detail, "unsupported.detail")?;
        sink.diagnostics.push(format!(
            "{feature} is unsupported by ABI v2 runtime: {detail}"
        ));
        Ok(())
    })
}

fn callback_status(f: impl FnOnce() -> Result<(), String>) -> AbiStatus {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(())) => AbiStatus::OK,
        Ok(Err(error)) => {
            log::warn!("Plugin registration callback failed: {error}");
            AbiStatus::HOST_ERROR
        }
        Err(panic_info) => {
            log::error!(
                "Plugin registration callback panicked: {}",
                panic_payload_to_string(&panic_info)
            );
            AbiStatus::PANIC
        }
    }
}

fn sink_mut<'a>(handle: HostRegistrarHandle) -> Result<&'a mut RegistrationSink, String> {
    if handle.0.is_null() {
        return Err("host registrar handle was null".to_string());
    }
    // SAFETY: The handle is created from a live `RegistrationSink` in
    // `register_plugin` and is only used during that synchronous registration.
    Ok(unsafe { &mut *(handle.0.cast::<RegistrationSink>()) })
}

fn copy_str(value: AbiStr, field: &str) -> Result<String, String> {
    // SAFETY: The ABI contract requires string views to point to initialized
    // UTF-8 bytes. This call validates null and length before reading.
    let bytes = unsafe { value.as_bytes_checked(MAX_ABI_STRING_LEN) }
        .map_err(|error| abi_read_error(field, error))?;
    let value =
        std::str::from_utf8(bytes).map_err(|_| abi_read_error(field, AbiReadError::InvalidUtf8))?;
    Ok(value.to_string())
}

fn copy_str_slice(value: AbiStrSlice, field: &str) -> Result<Vec<String>, String> {
    if value.len > MAX_ABI_SLICE_LEN {
        return Err(format!("{field} slice length exceeds ABI limit"));
    }
    if value.len == 0 {
        return Ok(Vec::new());
    }
    if value.ptr.is_null() {
        return Err(format!("{field} slice pointer was null"));
    }
    // SAFETY: The ABI contract requires non-null slices to point to `len`
    // initialized AbiStr entries for the duration of the callback.
    let values = unsafe { std::slice::from_raw_parts(value.ptr, value.len) };
    values
        .iter()
        .enumerate()
        .map(|(index, value)| copy_str(*value, &format!("{field}[{index}]")))
        .collect()
}

fn copy_u8_slice(value: AbiU8Slice, field: &str) -> Result<Vec<u8>, String> {
    if value.len > MAX_ABI_SLICE_LEN {
        return Err(format!("{field} slice length exceeds ABI limit"));
    }
    if value.len == 0 {
        return Ok(Vec::new());
    }
    if value.ptr.is_null() {
        return Err(format!("{field} slice pointer was null"));
    }
    // SAFETY: The ABI contract requires non-null slices to point to `len`
    // initialized bytes for the duration of the callback.
    Ok(unsafe { std::slice::from_raw_parts(value.ptr, value.len) }.to_vec())
}

fn copy_runtime_u8_slice(value: AbiU8Slice, field: &str) -> Result<Vec<u8>, String> {
    if value.len > wire::MAX_WIRE_PAYLOAD_LEN {
        return Err(format!("{field} runtime payload exceeds ABI limit"));
    }
    if value.len == 0 {
        return Ok(Vec::new());
    }
    if value.ptr.is_null() {
        return Err(format!("{field} runtime payload pointer was null"));
    }
    // SAFETY: The ABI contract requires non-null runtime byte slices to point
    // to `len` initialized bytes for the duration of the callback.
    Ok(unsafe { std::slice::from_raw_parts(value.ptr, value.len) }.to_vec())
}

fn copy_hint_slice(
    value: patinae_plugin::ffi::AbiSettingValueHintSlice,
    field: &str,
) -> Result<Vec<(String, SettingValue)>, String> {
    if value.len > MAX_ABI_SLICE_LEN {
        return Err(format!("{field} slice length exceeds ABI limit"));
    }
    if value.len == 0 {
        return Ok(Vec::new());
    }
    if value.ptr.is_null() {
        return Err(format!("{field} slice pointer was null"));
    }
    // SAFETY: The ABI contract requires non-null slices to point to `len`
    // initialized hint entries for the duration of the callback.
    let values = unsafe { std::slice::from_raw_parts(value.ptr, value.len) };
    values
        .iter()
        .enumerate()
        .map(|(index, value)| setting_hint_from_abi(value, &format!("{field}[{index}]")))
        .collect()
}

fn abi_read_error(field: &str, error: AbiReadError) -> String {
    match error {
        AbiReadError::NullPointer => format!("{field} pointer was null"),
        AbiReadError::TooLong => format!("{field} length exceeds ABI limit"),
        AbiReadError::InvalidUtf8 => format!("{field} was not valid UTF-8"),
    }
}

#[derive(Default)]
struct RuntimeBytes {
    bytes: Vec<u8>,
}

unsafe extern "C" fn host_runtime_bytes_sink(
    user_data: *mut c_void,
    bytes: AbiU8Slice,
) -> AbiStatus {
    if user_data.is_null() {
        return AbiStatus::INVALID;
    }
    let copied = match copy_runtime_u8_slice(bytes, "runtime.output") {
        Ok(bytes) => bytes,
        Err(error) => {
            log::warn!("Plugin runtime byte sink rejected output: {}", error);
            return AbiStatus::INVALID;
        }
    };
    // SAFETY: The caller passes a pointer to a live `RuntimeBytes` for the
    // duration of this sink call. Null was checked above.
    unsafe { &mut *(user_data.cast::<RuntimeBytes>()) }.bytes = copied;
    AbiStatus::OK
}

fn invoke_runtime<I, O>(
    phase: &str,
    input: &I,
    call: impl FnOnce(AbiU8Slice, AbiBytesSinkFn, *mut c_void) -> AbiStatus,
) -> Result<O, String>
where
    I: Serialize,
    O: DeserializeOwned,
{
    let input_bytes = wire::encode(input)?;
    ensure_runtime_payload_within_limit(phase, input_bytes.len(), "runtime input")?;
    let input_slice = AbiU8Slice {
        ptr: input_bytes.as_ptr(),
        len: input_bytes.len(),
    };
    let mut output = RuntimeBytes::default();
    let output_ptr = (&mut output as *mut RuntimeBytes).cast::<c_void>();
    let status = catch_unwind(AssertUnwindSafe(|| {
        call(input_slice, host_runtime_bytes_sink, output_ptr)
    }))
    .map_err(|panic_info| {
        format!(
            "plugin {phase} panicked across host boundary: {}",
            panic_payload_to_string(&panic_info)
        )
    })?;
    status_to_result(phase, status)?;
    wire::decode(&output.bytes)
        .map_err(|error| format!("plugin {phase} returned malformed MessagePack: {error}"))
}

unsafe extern "C" fn host_command_runtime_request(
    handle: HostCommandRuntimeHandle,
    input: AbiU8Slice,
    sink: AbiBytesSinkFn,
    sink_user_data: *mut c_void,
) -> AbiStatus {
    if handle.0.is_null() {
        return AbiStatus::INVALID;
    }

    let result = catch_unwind(AssertUnwindSafe(|| {
        let request: WireCommandRuntimeRequest = decode_runtime_input(input)?;
        // SAFETY: `AbiCommandProxy::execute` passes a pointer to a live
        // `HostCommandRuntimeState` for the duration of the plugin command
        // call. Null was checked above.
        let state = unsafe { &mut *(handle.0.cast::<HostCommandRuntimeState<'static>>()) };
        let response = state.handle_request(request);
        send_runtime_output(sink, sink_user_data, &response)
    }));

    match result {
        Ok(Ok(())) => AbiStatus::OK,
        Ok(Err(error)) => {
            log::warn!("Plugin command runtime request failed: {}", error);
            AbiStatus::HOST_ERROR
        }
        Err(panic_info) => {
            log::warn!(
                "Plugin command runtime request panicked: {}",
                panic_payload_to_string(&panic_info)
            );
            AbiStatus::PANIC
        }
    }
}

fn decode_runtime_input<T: DeserializeOwned>(input: AbiU8Slice) -> Result<T, String> {
    if input.len == 0 {
        return wire::decode(&[]);
    }
    if input.ptr.is_null() {
        return Err("runtime request pointer was null".to_string());
    }
    if input.len > wire::MAX_WIRE_PAYLOAD_LEN {
        return Err("runtime request exceeds ABI limit".to_string());
    }
    // SAFETY: The plugin ABI contract requires non-null byte slices to remain
    // initialized and readable for the duration of this callback.
    let bytes = unsafe { std::slice::from_raw_parts(input.ptr, input.len) };
    wire::decode(bytes)
}

fn send_runtime_output<T: Serialize>(
    sink: AbiBytesSinkFn,
    sink_user_data: *mut c_void,
    output: &T,
) -> Result<(), String> {
    let bytes = wire::encode(output)?;
    ensure_runtime_payload_within_limit("command runtime request", bytes.len(), "runtime output")?;
    let slice = AbiU8Slice {
        ptr: bytes.as_ptr(),
        len: bytes.len(),
    };
    // SAFETY: `slice` points to `bytes`, which lives until the sink returns.
    // The plugin sink copies the bytes synchronously.
    let status = unsafe { sink(sink_user_data, slice) };
    status_to_result("command runtime request", status)
}

fn validate_runtime_wire_version(version: u32) -> Result<(), String> {
    if version == RUNTIME_WIRE_VERSION {
        Ok(())
    } else {
        Err(format!(
            "runtime wire version mismatch: plugin has {}, host expects {}",
            version, RUNTIME_WIRE_VERSION
        ))
    }
}

fn validate_command_runtime(descriptor: &AbiCommandDescriptor) -> Result<(), String> {
    if descriptor.handle.0.is_null() {
        return Err("command handle was null".to_string());
    }
    if descriptor.vtable.execute.is_none() {
        return Err("command execute callback was null".to_string());
    }
    if descriptor.vtable.destroy.is_none() {
        return Err("command destroy callback was null".to_string());
    }
    if descriptor.runtime_requirements & !KNOWN_COMMAND_RUNTIME_REQUIREMENTS != 0 {
        return Err("command runtime requirements contain unknown bits".to_string());
    }
    Ok(())
}

fn validate_panel_runtime(descriptor: &AbiPanelDescriptor) -> Result<(), String> {
    if descriptor.handle.0.is_null() {
        return Err("panel handle was null".to_string());
    }
    if descriptor.vtable.snapshot.is_none() {
        return Err("panel snapshot callback was null".to_string());
    }
    if descriptor.vtable.handle_event.is_none() {
        return Err("panel event callback was null".to_string());
    }
    if descriptor.vtable.destroy.is_none() {
        return Err("panel destroy callback was null".to_string());
    }
    if descriptor.runtime_requirements & !KNOWN_PANEL_RUNTIME_REQUIREMENTS != 0 {
        return Err("panel runtime requirements contain unknown bits".to_string());
    }
    Ok(())
}

fn validate_message_handler_runtime(
    descriptor: &AbiMessageHandlerDescriptor,
) -> Result<(), String> {
    if descriptor.handle.0.is_null() {
        return Err("message handler handle was null".to_string());
    }
    if descriptor.vtable.on_message.is_none() {
        return Err("message handler callback was null".to_string());
    }
    if descriptor.vtable.poll.is_none() {
        return Err("message poll callback was null".to_string());
    }
    if descriptor.vtable.destroy.is_none() {
        return Err("message handler destroy callback was null".to_string());
    }
    Ok(())
}

fn validate_script_handler_runtime(descriptor: &AbiScriptHandlerDescriptor) -> Result<(), String> {
    if descriptor.handle.0.is_null() {
        return Err("script handler handle was null".to_string());
    }
    if descriptor.vtable.run.is_none() {
        return Err("script handler callback was null".to_string());
    }
    if descriptor.vtable.destroy.is_none() {
        return Err("script handler destroy callback was null".to_string());
    }
    Ok(())
}

fn validate_format_handler_runtime(descriptor: &AbiFormatHandlerDescriptor) -> Result<(), String> {
    if descriptor.handle.0.is_null() {
        return Err("format handler handle was null".to_string());
    }
    if descriptor.vtable.read.is_none() && descriptor.vtable.write.is_none() {
        return Err("format handler has no reader or writer callback".to_string());
    }
    if descriptor.vtable.destroy.is_none() {
        return Err("format handler destroy callback was null".to_string());
    }
    Ok(())
}

fn validate_hotkey_callback_runtime(descriptor: &AbiHotkeyDescriptor) -> Result<(), String> {
    if descriptor.handle.0.is_null() {
        return Err("hotkey action and callback handle were both empty".to_string());
    }
    if descriptor.vtable.invoke.is_none() {
        return Err("hotkey callback was null".to_string());
    }
    if descriptor.vtable.destroy.is_none() {
        return Err("hotkey destroy callback was null".to_string());
    }
    Ok(())
}

struct RegisteredCommand {
    name: String,
    description: String,
    usage: String,
    arguments: String,
    help: String,
    aliases: &'static [&'static str],
    arg_hints: &'static [ArgHint],
    runtime_requirements: CommandRuntimeRequirements,
    handle: PluginCommandHandle,
    vtable: AbiCommandVTable,
}

impl RegisteredCommand {
    fn from_descriptor(descriptor: &AbiCommandDescriptor) -> Result<Self, String> {
        validate_command_runtime(descriptor)?;
        Ok(Self {
            name: copy_str(descriptor.name, "command.name")?,
            description: copy_str(descriptor.description, "command.description")?,
            usage: copy_str(descriptor.usage, "command.usage")?,
            arguments: copy_str(descriptor.arguments, "command.arguments")?,
            help: copy_str(descriptor.help, "command.help")?,
            aliases: leak_aliases(copy_str_slice(descriptor.aliases, "command.aliases")?),
            arg_hints: leak_arg_hints(
                copy_u8_slice(descriptor.arg_hints, "command.arg_hints")?
                    .into_iter()
                    .map(arg_hint_from_abi)
                    .collect(),
            ),
            runtime_requirements: CommandRuntimeRequirements::from_bits(
                descriptor.runtime_requirements,
            ),
            handle: descriptor.handle,
            vtable: descriptor.vtable,
        })
    }
}

struct RegisteredMessageHandler {
    handle: PluginMessageHandlerHandle,
    vtable: AbiMessageHandlerVTable,
}

struct RegisteredScriptHandler {
    extension: String,
    handle: PluginScriptHandlerHandle,
    vtable: AbiScriptHandlerVTable,
}

struct RegisteredFormatHandler {
    name: String,
    extensions: Vec<String>,
    handle: PluginFormatHandlerHandle,
    vtable: AbiFormatHandlerVTable,
}

struct RegisteredHotkey {
    key: String,
    action: Option<WireHotkeyAction>,
    handle: PluginHotkeyHandle,
    vtable: AbiHotkeyVTable,
}

impl RegisteredHotkey {
    fn from_descriptor(descriptor: &AbiHotkeyDescriptor) -> Result<Self, String> {
        let action_bytes = copy_runtime_u8_slice(descriptor.action, "hotkey.action")?;
        let action = if action_bytes.is_empty() {
            validate_hotkey_callback_runtime(descriptor)?;
            None
        } else {
            Some(
                wire::decode(&action_bytes)
                    .map_err(|error| format!("hotkey.action was not valid MessagePack: {error}"))?,
            )
        };
        Ok(Self {
            key: copy_str(descriptor.key, "hotkey.key")?,
            action,
            handle: descriptor.handle,
            vtable: descriptor.vtable,
        })
    }

    fn into_action(self, library: Arc<LibraryHandle>) -> PluginKeyAction {
        match self.action {
            Some(action) => plugin_key_action_from_wire(action),
            None => {
                let proxy = AbiHotkeyProxy::new(self.handle, self.vtable, library);
                PluginKeyAction::Callback(Box::new(move |ctx| {
                    if let Err(error) = proxy.invoke(ctx) {
                        ctx.set_notification(format!("plugin hotkey callback failed: {error}"));
                    }
                }))
            }
        }
    }
}

struct AbiCommandProxy {
    command: RegisteredCommand,
    _library: Arc<LibraryHandle>,
    gpu_cache: SharedGpuPluginCache,
}

impl AbiCommandProxy {
    fn new(
        command: RegisteredCommand,
        library: Arc<LibraryHandle>,
        gpu_cache: SharedGpuPluginCache,
    ) -> Self {
        Self {
            command,
            _library: library,
            gpu_cache,
        }
    }
}

// SAFETY: The proxy never dereferences the raw plugin handle locally. All
// access goes through the plugin vtable while the library is kept alive. Plugin
// commands implement `Command: Send + Sync` before the SDK exports a handle.
unsafe impl Send for AbiCommandProxy {}
// SAFETY: See the `Send` rationale; command callbacks are immutable command
// invocations and plugin command objects are required to be `Sync`.
unsafe impl Sync for AbiCommandProxy {}

impl Command for AbiCommandProxy {
    fn name(&self) -> &str {
        &self.command.name
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let input =
            command_runtime_input_from_context(ctx, args, self.command.runtime_requirements)
                .map_err(|error| CmdError::execution(format!("plugin command input: {error}")))?;
        let execute = self
            .command
            .vtable
            .execute
            .ok_or_else(|| CmdError::execution("plugin command execute callback was null"))?;
        let output: WireCommandOutput = {
            let mut runtime_state = HostCommandRuntimeState::with_cache(
                ctx.viewer,
                self.command.runtime_requirements,
                self.gpu_cache.clone(),
            );
            invoke_runtime("command execute", &input.input, |slice, sink, user_data| {
                // SAFETY: The descriptor was validated during registration and the
                // library is kept alive by this proxy while the callback runs.
                unsafe {
                    execute(
                        self.command.handle,
                        slice,
                        &HOST_COMMAND_RUNTIME_CALLBACKS,
                        runtime_state.handle(),
                        sink,
                        user_data,
                    )
                }
            })
        }
        .map_err(CmdError::execution)?;
        apply_command_output(ctx, output, self.command.runtime_requirements)
    }

    fn description(&self) -> &str {
        &self.command.description
    }

    fn usage(&self) -> &str {
        &self.command.usage
    }

    fn arguments(&self) -> &str {
        &self.command.arguments
    }

    fn help(&self) -> &str {
        &self.command.help
    }

    fn aliases(&self) -> &[&str] {
        self.command.aliases
    }

    fn arg_hints(&self) -> &[ArgHint] {
        self.command.arg_hints
    }

    fn runtime_requirements(&self) -> CommandRuntimeRequirements {
        self.command.runtime_requirements
    }
}

impl Drop for AbiCommandProxy {
    fn drop(&mut self) {
        destroy_command_handle(self.command.handle, self.command.vtable);
    }
}

struct AbiPanelProxy {
    panel: RegisteredPanel,
    _library: Arc<LibraryHandle>,
}

impl AbiPanelProxy {
    fn new(panel: RegisteredPanel, library: Arc<LibraryHandle>) -> Self {
        Self {
            panel,
            _library: library,
        }
    }
}

// SAFETY: The proxy keeps the dynamic library alive and only calls validated
// vtable entries with ABI byte payloads. Panel methods require mutable access.
unsafe impl Send for AbiPanelProxy {}

impl PluginPanel for AbiPanelProxy {
    fn descriptor(&self) -> PanelDescriptor {
        self.panel.descriptor.clone()
    }

    fn snapshot(&mut self, ctx: &SharedContext<'_>) -> PanelSnapshot {
        match self.snapshot_inner(ctx) {
            Ok(snapshot) => snapshot,
            Err(error) => PanelSnapshot::new(vec![PanelControl::Text {
                id: "abi_v2_error".to_string(),
                text: format!("Plugin panel failed: {error}"),
            }]),
        }
    }

    fn handle_event(
        &mut self,
        event: PanelEvent,
        ctx: &SharedContext<'_>,
        _bus: &mut MessageBus,
    ) -> Vec<patinae_framework::plugin_ui::PanelAction> {
        match self.handle_event_inner(event, ctx) {
            Ok(actions) => actions,
            Err(error) => {
                log::warn!(
                    "Plugin panel '{}' event failed: {}",
                    self.panel.descriptor.id,
                    error
                );
                Vec::new()
            }
        }
    }
}

impl AbiPanelProxy {
    fn snapshot_inner(&mut self, ctx: &SharedContext<'_>) -> Result<PanelSnapshot, String> {
        let snapshot = shared_input_from_context(ctx, self.panel.runtime_requirements)?;
        let callback = self
            .panel
            .vtable
            .snapshot
            .ok_or("plugin panel snapshot callback was null")?;
        let output: WirePanelSnapshotOutput =
            invoke_runtime("panel snapshot", &snapshot, |slice, sink, user_data| {
                // SAFETY: The descriptor was validated during registration and
                // this proxy keeps the library alive while the callback runs.
                unsafe { callback(self.panel.handle, slice, sink, user_data) }
            })?;
        validate_runtime_wire_version(output.wire_version)?;
        Ok(output.snapshot)
    }

    fn handle_event_inner(
        &mut self,
        event: PanelEvent,
        ctx: &SharedContext<'_>,
    ) -> Result<Vec<patinae_framework::plugin_ui::PanelAction>, String> {
        let input = WirePanelEventInput {
            shared: shared_input_from_context(ctx, self.panel.runtime_requirements)?,
            event,
        };
        let callback = self
            .panel
            .vtable
            .handle_event
            .ok_or("plugin panel event callback was null")?;
        let output: WirePanelEventOutput =
            invoke_runtime("panel event", &input, |slice, sink, user_data| {
                // SAFETY: The descriptor was validated during registration and
                // this proxy keeps the library alive while the callback runs.
                unsafe { callback(self.panel.handle, slice, sink, user_data) }
            })?;
        validate_runtime_wire_version(output.wire_version)?;
        Ok(output.actions)
    }
}

impl Drop for AbiPanelProxy {
    fn drop(&mut self) {
        destroy_panel_handle(self.panel.handle, self.panel.vtable);
    }
}

struct AbiMessageHandlerProxy {
    handle: PluginMessageHandlerHandle,
    vtable: AbiMessageHandlerVTable,
    _library: Arc<LibraryHandle>,
}

impl AbiMessageHandlerProxy {
    fn new(handler: RegisteredMessageHandler, library: Arc<LibraryHandle>) -> Self {
        Self {
            handle: handler.handle,
            vtable: handler.vtable,
            _library: library,
        }
    }
}

// SAFETY: The proxy keeps the dynamic library alive and only calls validated
// vtable entries with ABI byte payloads. `MessageHandler` only requires `Send`.
unsafe impl Send for AbiMessageHandlerProxy {}

impl MessageHandler for AbiMessageHandlerProxy {
    fn on_message(&mut self, msg: &AppMessage, bus: &mut MessageBus) {
        if let Err(error) = self.on_message_inner(msg, bus) {
            log::warn!("Plugin message handler failed: {}", error);
        }
    }

    fn needs_poll(&self) -> bool {
        self.vtable.needs_poll != 0
    }

    fn poll(&mut self, ctx: &mut PollContext<'_>) {
        if let Err(error) = self.poll_inner(ctx) {
            ctx.set_notification(format!("plugin poll failed: {error}"));
        }
    }
}

impl AbiMessageHandlerProxy {
    fn on_message_inner(&mut self, msg: &AppMessage, bus: &mut MessageBus) -> Result<(), String> {
        let input = WireMessageInput {
            wire_version: RUNTIME_WIRE_VERSION,
            message: msg.clone(),
        };
        let callback = self
            .vtable
            .on_message
            .ok_or("plugin message callback was null")?;
        let output: WireMessageOutput =
            invoke_runtime("message handler", &input, |slice, sink, user_data| {
                // SAFETY: The descriptor was validated during registration and
                // this proxy keeps the library alive while the callback runs.
                unsafe { callback(self.handle, slice, sink, user_data) }
            })?;
        validate_runtime_wire_version(output.wire_version)?;
        send_bus_messages(output.messages, bus);
        Ok(())
    }

    fn poll_inner(&mut self, ctx: &mut PollContext<'_>) -> Result<(), String> {
        let input = poll_input_from_context(ctx)?;
        let callback = self.vtable.poll.ok_or("plugin poll callback was null")?;
        let output: WirePollOutput =
            invoke_runtime("message poll", &input, |slice, sink, user_data| {
                // SAFETY: The descriptor was validated during registration and
                // this proxy keeps the library alive while the callback runs.
                unsafe { callback(self.handle, slice, sink, user_data) }
            })?;
        apply_poll_output(ctx, output)
    }
}

impl Drop for AbiMessageHandlerProxy {
    fn drop(&mut self) {
        destroy_message_handler_handle(self.handle, self.vtable);
    }
}

struct AbiScriptHandlerProxy {
    extension: String,
    handle: PluginScriptHandlerHandle,
    vtable: AbiScriptHandlerVTable,
    _library: Arc<LibraryHandle>,
}

impl AbiScriptHandlerProxy {
    fn into_handler(script: RegisteredScriptHandler, library: Arc<LibraryHandle>) -> ScriptHandler {
        let proxy = Arc::new(Self {
            extension: script.extension,
            handle: script.handle,
            vtable: script.vtable,
            _library: library,
        });
        Arc::new(move |path: &str| proxy.run(path))
    }

    fn run(&self, path: &str) -> Result<(), String> {
        let input = WireScriptInput {
            wire_version: RUNTIME_WIRE_VERSION,
            path: path.to_string(),
        };
        let callback = self.vtable.run.ok_or("plugin script callback was null")?;
        let output: WireScriptOutput =
            invoke_runtime("script handler", &input, |slice, sink, user_data| {
                // SAFETY: The descriptor was validated during registration and
                // this proxy keeps the library alive while the callback runs.
                unsafe { callback(self.handle, slice, sink, user_data) }
            })?;
        validate_runtime_wire_version(output.wire_version)?;
        output
            .result
            .map_err(|error| format!("plugin script .{} failed: {error}", self.extension))
    }
}

// SAFETY: The proxy never dereferences the raw plugin handle locally and the
// SDK only exports script handlers from `Send + Sync` closures.
unsafe impl Send for AbiScriptHandlerProxy {}
// SAFETY: See the `Send` rationale; calls go through validated ABI callbacks.
unsafe impl Sync for AbiScriptHandlerProxy {}

impl Drop for AbiScriptHandlerProxy {
    fn drop(&mut self) {
        destroy_script_handler_handle(self.handle, self.vtable);
    }
}

struct AbiFormatHandlerProxy {
    name: String,
    handle: PluginFormatHandlerHandle,
    vtable: AbiFormatHandlerVTable,
    _library: Arc<LibraryHandle>,
}

impl AbiFormatHandlerProxy {
    fn into_handler(format: RegisteredFormatHandler, library: Arc<LibraryHandle>) -> FormatHandler {
        let name = format.name;
        let extensions = format.extensions;
        let vtable = format.vtable;
        let proxy = Arc::new(Self {
            name: name.clone(),
            handle: format.handle,
            vtable,
            _library: library,
        });
        let reader: Option<PluginReaderFn> = if vtable.read.is_some() {
            let proxy = proxy.clone();
            Some(Arc::new(move |reader: Box<dyn Read>| proxy.read(reader)))
        } else {
            None
        };
        let writer: Option<PluginWriterFn> = if vtable.write.is_some() {
            let proxy = proxy.clone();
            Some(Arc::new(
                move |writer: Box<dyn Write>, molecules: &[ObjectMolecule]| {
                    proxy.write(writer, molecules)
                },
            ))
        } else {
            None
        };
        FormatHandler {
            name,
            extensions,
            reader,
            writer,
        }
    }

    fn read(&self, mut reader: Box<dyn Read>) -> Result<Vec<ObjectMolecule>, String> {
        let mut bytes = Vec::new();
        reader
            .read_to_end(&mut bytes)
            .map_err(|error| format!("failed to read input bytes: {error}"))?;
        let input = WireFormatReadInput {
            wire_version: RUNTIME_WIRE_VERSION,
            extension: String::new(),
            bytes,
        };
        let callback = self
            .vtable
            .read
            .ok_or("plugin format reader callback was null")?;
        let output: WireFormatReadOutput =
            invoke_runtime("format reader", &input, |slice, sink, user_data| {
                // SAFETY: The descriptor was validated during registration and
                // this proxy keeps the library alive while the callback runs.
                unsafe { callback(self.handle, slice, sink, user_data) }
            })?;
        validate_runtime_wire_version(output.wire_version)?;
        output
            .result
            .map_err(|error| format!("plugin format '{}' read failed: {error}", self.name))
    }

    fn write(
        &self,
        mut writer: Box<dyn Write>,
        molecules: &[ObjectMolecule],
    ) -> Result<(), String> {
        let input = WireFormatWriteInput {
            wire_version: RUNTIME_WIRE_VERSION,
            extension: String::new(),
            molecules: molecules.to_vec(),
        };
        let callback = self
            .vtable
            .write
            .ok_or("plugin format writer callback was null")?;
        let output: WireFormatWriteOutput =
            invoke_runtime("format writer", &input, |slice, sink, user_data| {
                // SAFETY: The descriptor was validated during registration and
                // this proxy keeps the library alive while the callback runs.
                unsafe { callback(self.handle, slice, sink, user_data) }
            })?;
        validate_runtime_wire_version(output.wire_version)?;
        let bytes = output
            .result
            .map_err(|error| format!("plugin format '{}' write failed: {error}", self.name))?;
        writer
            .write_all(&bytes)
            .map_err(|error| format!("failed to write plugin output bytes: {error}"))
    }
}

// SAFETY: The proxy never dereferences the raw plugin handle locally and the
// SDK only exports format handlers from `Send + Sync` reader/writer closures.
unsafe impl Send for AbiFormatHandlerProxy {}
// SAFETY: See the `Send` rationale; calls go through validated ABI callbacks.
unsafe impl Sync for AbiFormatHandlerProxy {}

impl Drop for AbiFormatHandlerProxy {
    fn drop(&mut self) {
        destroy_format_handler_handle(self.handle, self.vtable);
    }
}

struct AbiHotkeyProxy {
    handle: PluginHotkeyHandle,
    vtable: AbiHotkeyVTable,
    _library: Arc<LibraryHandle>,
}

impl AbiHotkeyProxy {
    fn new(
        handle: PluginHotkeyHandle,
        vtable: AbiHotkeyVTable,
        library: Arc<LibraryHandle>,
    ) -> Self {
        Self {
            handle,
            vtable,
            _library: library,
        }
    }

    fn invoke(&self, ctx: &mut PollContext<'_>) -> Result<(), String> {
        let input = poll_input_from_context(ctx)?;
        let callback = self
            .vtable
            .invoke
            .ok_or("plugin hotkey callback was null")?;
        let output: WirePollOutput =
            invoke_runtime("hotkey callback", &input, |slice, sink, user_data| {
                // SAFETY: The descriptor was validated during registration and
                // this proxy keeps the library alive while the callback runs.
                unsafe { callback(self.handle, slice, sink, user_data) }
            })?;
        apply_poll_output(ctx, output)
    }
}

// SAFETY: The proxy is moved into a host-owned callback and only calls through
// validated ABI callbacks while keeping the dynamic library alive.
unsafe impl Send for AbiHotkeyProxy {}

impl Drop for AbiHotkeyProxy {
    fn drop(&mut self) {
        destroy_hotkey_handle(self.handle, self.vtable);
    }
}

struct CommandRuntimeInput {
    input: WireCommandInput,
    _spools: Vec<RuntimePayloadSpool>,
}

struct HostCommandRuntimeState<'a> {
    viewer: &'a mut dyn ViewerLike,
    runtime_requirements: CommandRuntimeRequirements,
    streams: Vec<HostTraceGeometryStream>,
    gpu_handles: GpuHandleRegistry,
    gpu_cache: SharedGpuPluginCache,
    snapshots: Vec<HostRenderArtifactSnapshot>,
    next_stream_id: u64,
    next_snapshot_id: u64,
}

impl<'a> HostCommandRuntimeState<'a> {
    #[cfg(test)]
    fn new(
        viewer: &'a mut dyn ViewerLike,
        runtime_requirements: CommandRuntimeRequirements,
    ) -> Self {
        Self::with_cache(
            viewer,
            runtime_requirements,
            Arc::new(Mutex::new(GpuPluginCache::new("test".to_string()))),
        )
    }

    fn with_cache(
        viewer: &'a mut dyn ViewerLike,
        runtime_requirements: CommandRuntimeRequirements,
        gpu_cache: SharedGpuPluginCache,
    ) -> Self {
        Self {
            viewer,
            runtime_requirements,
            streams: Vec::new(),
            gpu_handles: GpuHandleRegistry::new(),
            gpu_cache,
            snapshots: Vec::new(),
            next_stream_id: 1,
            next_snapshot_id: 1,
        }
    }

    fn handle(&mut self) -> HostCommandRuntimeHandle {
        HostCommandRuntimeHandle((self as *mut Self).cast::<c_void>())
    }

    fn handle_request(&mut self, request: WireCommandRuntimeRequest) -> WireCommandRuntimeResponse {
        match request {
            WireCommandRuntimeRequest::OpenTraceGeometryStream { id } => {
                let result = self.open_trace_geometry_stream();
                Self::response(id, result)
            }
            WireCommandRuntimeRequest::ReadTraceGeometryStream { id, stream_id } => {
                let result = self.read_trace_geometry_stream(stream_id);
                Self::response(id, result)
            }
            WireCommandRuntimeRequest::CloseTraceGeometryStream { id, stream_id } => {
                let result = self.close_trace_geometry_stream(stream_id);
                Self::response(id, result)
            }
            WireCommandRuntimeRequest::OpenRenderArtifactSnapshot { id } => {
                let result = self.open_render_artifact_snapshot();
                Self::response(id, result)
            }
            WireCommandRuntimeRequest::CloseRenderArtifactSnapshot { id, snapshot_id } => {
                let result = self.close_render_artifact_snapshot(snapshot_id);
                Self::response(id, result)
            }
            WireCommandRuntimeRequest::GpuDeviceLimits { id } => {
                let result = self.gpu_device_limits();
                Self::response(id, result)
            }
            WireCommandRuntimeRequest::GpuCreateBuffer {
                id,
                descriptor,
                initial_data,
            } => {
                let result = self.gpu_create_buffer(descriptor, initial_data);
                Self::response(id, result)
            }
            WireCommandRuntimeRequest::GpuCreateTexture { id, descriptor } => {
                let result = self.gpu_create_texture(descriptor);
                Self::response(id, result)
            }
            WireCommandRuntimeRequest::GpuCreateTextureView { id, descriptor } => {
                let result = self.gpu_create_texture_view(descriptor);
                Self::response(id, result)
            }
            WireCommandRuntimeRequest::GpuCreateSampler { id, descriptor } => {
                let result = self.gpu_create_sampler(descriptor);
                Self::response(id, result)
            }
            WireCommandRuntimeRequest::GpuWriteBuffer {
                id,
                buffer,
                offset,
                data,
            } => {
                let result = self.gpu_write_buffer(buffer, offset, &data);
                Self::response(id, result)
            }
            WireCommandRuntimeRequest::GpuCopyBufferToBuffer {
                id,
                source,
                source_offset,
                destination,
                destination_offset,
                size,
            } => {
                let result = self.gpu_copy_buffer_to_buffer(
                    source,
                    source_offset,
                    destination,
                    destination_offset,
                    size,
                );
                Self::response(id, result)
            }
            WireCommandRuntimeRequest::GpuReadBuffer {
                id,
                buffer,
                offset,
                size,
            } => {
                let result = self.gpu_read_buffer(buffer, offset, size);
                Self::response(id, result)
            }
            WireCommandRuntimeRequest::GpuCreateShaderModule { id, descriptor } => {
                let result = self.gpu_create_shader_module(descriptor);
                Self::response(id, result)
            }
            WireCommandRuntimeRequest::GpuCreateBindGroupLayout { id, descriptor } => {
                let result = self.gpu_create_bind_group_layout(descriptor);
                Self::response(id, result)
            }
            WireCommandRuntimeRequest::GpuCreatePipelineLayout { id, descriptor } => {
                let result = self.gpu_create_pipeline_layout(descriptor);
                Self::response(id, result)
            }
            WireCommandRuntimeRequest::GpuCreateComputePipeline { id, descriptor } => {
                let result = self.gpu_create_compute_pipeline(descriptor);
                Self::response(id, result)
            }
            WireCommandRuntimeRequest::GpuCreateRenderPipeline { id, descriptor } => {
                let result = self.gpu_create_render_pipeline(descriptor);
                Self::response(id, result)
            }
            WireCommandRuntimeRequest::GpuCreateCachedShaderModule { id, descriptor } => {
                let result = self.gpu_create_cached_shader_module(descriptor);
                Self::response(id, result)
            }
            WireCommandRuntimeRequest::GpuCreateCachedBindGroupLayout { id, descriptor } => {
                let result = self.gpu_create_cached_bind_group_layout(descriptor);
                Self::response(id, result)
            }
            WireCommandRuntimeRequest::GpuCreateCachedPipelineLayout { id, descriptor } => {
                let result = self.gpu_create_cached_pipeline_layout(descriptor);
                Self::response(id, result)
            }
            WireCommandRuntimeRequest::GpuCreateCachedComputePipeline { id, descriptor } => {
                let result = self.gpu_create_cached_compute_pipeline(descriptor);
                Self::response(id, result)
            }
            WireCommandRuntimeRequest::GpuCreateCachedRenderPipeline { id, descriptor } => {
                let result = self.gpu_create_cached_render_pipeline(descriptor);
                Self::response(id, result)
            }
            WireCommandRuntimeRequest::GpuCacheStats { id } => {
                let result = self.gpu_cache_stats();
                Self::response(id, result)
            }
            WireCommandRuntimeRequest::GpuDropPluginCache { id } => {
                let result = self.gpu_drop_plugin_cache();
                Self::response(id, result)
            }
            WireCommandRuntimeRequest::GpuCreateBindGroup { id, descriptor } => {
                let result = self.gpu_create_bind_group(descriptor);
                Self::response(id, result)
            }
            WireCommandRuntimeRequest::GpuDispatchCompute {
                id,
                pipeline,
                bind_groups,
                workgroups,
            } => {
                let result = self.gpu_dispatch_compute(pipeline, &bind_groups, workgroups);
                Self::response(id, result)
            }
            WireCommandRuntimeRequest::GpuSubmitBatch { id, batch } => {
                let result = self.gpu_submit_batch(batch);
                Self::response(id, result)
            }
            WireCommandRuntimeRequest::SetViewportGpuImageFromBuffer {
                id,
                buffer,
                width,
                height,
            } => {
                let result = self.set_viewport_gpu_image_from_buffer(buffer, width, height);
                Self::response(id, result)
            }
            WireCommandRuntimeRequest::GpuDropHandles { id, handles } => {
                let result = self.gpu_drop_handles(&handles);
                Self::response(id, result)
            }
        }
    }

    fn response(
        id: u64,
        result: Result<WireCommandRuntimeValue, String>,
    ) -> WireCommandRuntimeResponse {
        WireCommandRuntimeResponse {
            wire_version: RUNTIME_WIRE_VERSION,
            id,
            result,
        }
    }

    fn ensure_requirement(&self, requirement: CommandRuntimeRequirements) -> Result<(), String> {
        if self.runtime_requirements.contains(requirement) {
            Ok(())
        } else {
            Err("plugin command did not declare the required runtime input".to_string())
        }
    }

    fn open_trace_geometry_stream(&mut self) -> Result<WireCommandRuntimeValue, String> {
        self.ensure_requirement(CommandRuntimeRequirements::TRACE_GEOMETRY_STREAM)?;
        let stream_id = self.next_stream_id;
        self.next_stream_id += 1;
        let start = std::time::Instant::now();
        let mut chunks = VecDeque::new();
        let mut chunk_count = 0usize;
        let mut primitive_count = 0usize;

        self.viewer.for_each_trace_geometry_chunk(
            &patinae_render::GeometryExportOptions::default(),
            &mut |chunk| {
                primitive_count += chunk.primitive_count();
                let wire_chunk = wire::trace_geometry_chunk_to_wire(chunk);
                let encoded = wire::encode(&wire_chunk)?;
                ensure_runtime_payload_within_limit(
                    "command trace geometry stream",
                    encoded.len(),
                    "trace geometry chunk",
                )?;
                chunks.push_back(encoded);
                chunk_count += 1;
                Ok(())
            },
        )?;

        if rt_profile_enabled() {
            log::info!(
                "patinae.rt_profile host.trace_export_ms={} chunks={} primitives={}",
                start.elapsed().as_millis(),
                chunk_count,
                primitive_count
            );
        }

        self.streams.push(HostTraceGeometryStream {
            id: stream_id,
            chunks,
        });
        Ok(WireCommandRuntimeValue::TraceGeometryOpened(
            WireTraceGeometryOpened { stream_id },
        ))
    }

    fn read_trace_geometry_stream(
        &mut self,
        stream_id: u64,
    ) -> Result<WireCommandRuntimeValue, String> {
        self.ensure_requirement(CommandRuntimeRequirements::TRACE_GEOMETRY_STREAM)?;
        let stream = self
            .streams
            .iter_mut()
            .find(|stream| stream.id == stream_id)
            .ok_or_else(|| format!("trace geometry stream {stream_id} was not open"))?;
        Ok(WireCommandRuntimeValue::TraceGeometryChunkBytes(
            stream.chunks.pop_front(),
        ))
    }

    fn close_trace_geometry_stream(
        &mut self,
        stream_id: u64,
    ) -> Result<WireCommandRuntimeValue, String> {
        self.ensure_requirement(CommandRuntimeRequirements::TRACE_GEOMETRY_STREAM)?;
        let index = self
            .streams
            .iter()
            .position(|stream| stream.id == stream_id)
            .ok_or_else(|| format!("trace geometry stream {stream_id} was not open"))?;
        self.streams.swap_remove(index);
        Ok(WireCommandRuntimeValue::TraceGeometryClosed)
    }

    fn open_render_artifact_snapshot(&mut self) -> Result<WireCommandRuntimeValue, String> {
        self.ensure_requirement(CommandRuntimeRequirements::RENDER_ARTIFACTS)?;
        let snapshot_id = self.next_snapshot_id;
        self.next_snapshot_id += 1;

        let limits = gpu_device_limits_from_viewer(self.viewer)?;
        let scene_generation = self
            .viewer
            .objects()
            .generation()
            .wrapping_add(self.viewer.selections().generation().rotate_left(1))
            .wrapping_add(self.viewer.movie().generation().rotate_left(2));
        let mut captured = None;
        self.viewer.visit_render_artifacts(&mut |snapshot| {
            let buffers = snapshot
                .buffers
                .iter()
                .map(|buffer| HostRenderArtifactBuffer {
                    role: map_render_artifact_buffer_role(buffer.role),
                    buffer: buffer.buffer.clone(),
                    size: buffer.size,
                    stride: buffer.stride,
                    element_count: buffer.element_count,
                })
                .collect();
            let reps = snapshot
                .reps
                .iter()
                .copied()
                .map(|rep| HostRenderArtifactRep {
                    object_id: rep.object_id,
                    rep_kind: map_render_artifact_rep_kind(rep.rep_kind),
                    topology: map_render_artifact_topology(rep.topology),
                    geometry_buffer_index: rep.geometry_buffer_index,
                    count_buffer_index: rep.count_buffer_index,
                    indirect_buffer_index: rep.indirect_buffer_index,
                    element_count: rep.element_count,
                    max_element_count: rep.max_element_count,
                    atom_offset: rep.atom_offset,
                    atom_count: rep.atom_count,
                    material_rgba: rep.material_rgba,
                    transparency: rep.transparency,
                })
                .collect();
            captured = Some(HostRenderArtifactSnapshot {
                id: snapshot_id,
                layout_version: snapshot.layout_version,
                scene_generation,
                scene_bounds_min: snapshot.scene_bounds_min,
                scene_bounds_max: snapshot.scene_bounds_max,
                cull_pass_initialized: snapshot.cull_pass_initialized,
                buffers,
                reps,
            });
            Ok(())
        })?;

        let snapshot = captured
            .ok_or_else(|| "renderer did not provide a render artifact snapshot".to_string())?;
        let mut buffer_descriptors = Vec::with_capacity(snapshot.buffers.len());
        for buffer in &snapshot.buffers {
            let handle = self.gpu_handles.insert_buffer(
                buffer.buffer.clone(),
                buffer.size,
                render_artifact_buffer_usage(buffer.role),
            );
            buffer_descriptors.push(RenderArtifactBufferDescriptor {
                handle,
                role: buffer.role,
                size: buffer.size,
                stride: buffer.stride,
                element_count: buffer.element_count,
            });
        }

        let mut rep_descriptors = Vec::with_capacity(snapshot.reps.len());
        for rep in &snapshot.reps {
            let geometry = buffer_descriptors
                .get(rep.geometry_buffer_index)
                .ok_or_else(|| {
                    "render artifact rep referenced missing geometry buffer".to_string()
                })?
                .handle;
            let count = rep
                .count_buffer_index
                .map(|index| {
                    buffer_descriptors
                        .get(index)
                        .map(|descriptor| descriptor.handle)
                        .ok_or_else(|| {
                            "render artifact rep referenced missing count buffer".to_string()
                        })
                })
                .transpose()?;
            let indirect = rep
                .indirect_buffer_index
                .map(|index| {
                    buffer_descriptors
                        .get(index)
                        .map(|descriptor| descriptor.handle)
                        .ok_or_else(|| {
                            "render artifact rep referenced missing indirect buffer".to_string()
                        })
                })
                .transpose()?;
            rep_descriptors.push(RenderArtifactRepDescriptor {
                object_id: rep.object_id,
                rep_kind: rep.rep_kind,
                topology: rep.topology,
                geometry,
                count,
                indirect,
                element_count: rep.element_count,
                max_element_count: rep.max_element_count,
                atom_offset: rep.atom_offset,
                atom_count: rep.atom_count,
                material_rgba: rep.material_rgba,
                transparency: rep.transparency,
            });
        }

        let descriptor = RenderArtifactSnapshotDescriptor {
            snapshot_id,
            layout_version: snapshot.layout_version,
            scene_generation: snapshot.scene_generation,
            scene_bounds_min: snapshot.scene_bounds_min,
            scene_bounds_max: snapshot.scene_bounds_max,
            cull_pass_initialized: snapshot.cull_pass_initialized,
            device_limits: limits,
            buffers: buffer_descriptors,
            reps: rep_descriptors,
        };
        self.snapshots.push(snapshot);
        Ok(WireCommandRuntimeValue::RenderArtifactSnapshotOpened(
            descriptor,
        ))
    }

    fn close_render_artifact_snapshot(
        &mut self,
        snapshot_id: u64,
    ) -> Result<WireCommandRuntimeValue, String> {
        self.ensure_requirement(CommandRuntimeRequirements::RENDER_ARTIFACTS)?;
        let index = self
            .snapshots
            .iter()
            .position(|snapshot| snapshot.id == snapshot_id)
            .ok_or_else(|| format!("render artifact snapshot {snapshot_id} was not open"))?;
        self.snapshots.swap_remove(index);
        Ok(WireCommandRuntimeValue::RenderArtifactSnapshotClosed)
    }

    fn gpu_device_limits(&mut self) -> Result<WireCommandRuntimeValue, String> {
        self.ensure_requirement(CommandRuntimeRequirements::GPU_COMMANDS)?;
        Ok(WireCommandRuntimeValue::GpuDeviceLimits(
            gpu_device_limits_from_viewer(self.viewer)?,
        ))
    }

    fn gpu_create_buffer(
        &mut self,
        descriptor: GpuBufferDescriptor,
        initial_data: Option<Vec<u8>>,
    ) -> Result<WireCommandRuntimeValue, String> {
        self.ensure_requirement(CommandRuntimeRequirements::GPU_COMMANDS)?;
        let device = self
            .viewer
            .gpu_device()
            .ok_or_else(|| "GPU command runtime requires an active device".to_string())?;
        let usage = wgpu_buffer_usage(descriptor.usage)?;
        let buffer = if let Some(data) = initial_data {
            if data.len() as u64 > descriptor.size {
                return Err("initial GPU buffer data exceeds descriptor size".to_string());
            }
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: descriptor.label.as_deref(),
                contents: &data,
                usage,
            })
        } else {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: descriptor.label.as_deref(),
                size: descriptor.size,
                usage,
                mapped_at_creation: false,
            })
        };
        let handle = self
            .gpu_handles
            .insert_buffer(buffer, descriptor.size, descriptor.usage);
        Ok(WireCommandRuntimeValue::GpuHandle(handle))
    }

    fn gpu_create_texture(
        &mut self,
        descriptor: GpuTextureDescriptor,
    ) -> Result<WireCommandRuntimeValue, String> {
        self.ensure_requirement(CommandRuntimeRequirements::GPU_COMMANDS)?;
        validate_texture_descriptor(&descriptor)?;
        let device = self
            .viewer
            .gpu_device()
            .ok_or_else(|| "GPU command runtime requires an active device".to_string())?;
        let size = wgpu_extent3d(descriptor.size);
        let format = wgpu_texture_format(descriptor.format);
        let usage = wgpu_texture_usage(descriptor.usage)?;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: descriptor.label.as_deref(),
            size,
            mip_level_count: descriptor.mip_level_count,
            sample_count: descriptor.sample_count,
            dimension: wgpu_texture_dimension(descriptor.dimension),
            format,
            usage,
            view_formats: &[],
        });
        let handle = self.gpu_handles.insert_texture(texture, &descriptor);
        Ok(WireCommandRuntimeValue::GpuHandle(handle))
    }

    fn gpu_create_texture_view(
        &mut self,
        descriptor: GpuTextureViewDescriptor,
    ) -> Result<WireCommandRuntimeValue, String> {
        self.ensure_requirement(CommandRuntimeRequirements::GPU_COMMANDS)?;
        let texture = self.gpu_handles.texture(descriptor.texture)?;
        validate_texture_view_descriptor(texture, &descriptor)?;
        let view_format = descriptor.format.unwrap_or(texture.format);
        let view_usage = descriptor.usage.unwrap_or(texture.usage);
        let view = texture.texture.create_view(&wgpu::TextureViewDescriptor {
            label: descriptor.label.as_deref(),
            format: descriptor.format.map(wgpu_texture_format),
            dimension: descriptor.dimension.map(wgpu_texture_view_dimension),
            usage: descriptor.usage.map(wgpu_texture_usage).transpose()?,
            aspect: wgpu_texture_aspect(descriptor.aspect),
            base_mip_level: descriptor.base_mip_level,
            mip_level_count: descriptor.mip_level_count,
            base_array_layer: descriptor.base_array_layer,
            array_layer_count: descriptor.array_layer_count,
        });
        let handle = self
            .gpu_handles
            .insert_texture_view(view, view_format, view_usage);
        Ok(WireCommandRuntimeValue::GpuHandle(handle))
    }

    fn gpu_create_sampler(
        &mut self,
        descriptor: GpuSamplerDescriptor,
    ) -> Result<WireCommandRuntimeValue, String> {
        self.ensure_requirement(CommandRuntimeRequirements::GPU_COMMANDS)?;
        validate_sampler_descriptor(&descriptor)?;
        let device = self
            .viewer
            .gpu_device()
            .ok_or_else(|| "GPU command runtime requires an active device".to_string())?;
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: descriptor.label.as_deref(),
            address_mode_u: wgpu_address_mode(descriptor.address_mode_u),
            address_mode_v: wgpu_address_mode(descriptor.address_mode_v),
            address_mode_w: wgpu_address_mode(descriptor.address_mode_w),
            mag_filter: wgpu_filter_mode(descriptor.mag_filter),
            min_filter: wgpu_filter_mode(descriptor.min_filter),
            mipmap_filter: wgpu_mipmap_filter_mode(descriptor.mipmap_filter),
            lod_min_clamp: descriptor.lod_min_clamp,
            lod_max_clamp: descriptor.lod_max_clamp,
            compare: descriptor.compare.map(wgpu_compare_function),
            anisotropy_clamp: descriptor.anisotropy_clamp,
            border_color: None,
        });
        let handle = self.gpu_handles.insert_sampler(sampler);
        Ok(WireCommandRuntimeValue::GpuHandle(handle))
    }

    fn gpu_write_buffer(
        &mut self,
        buffer: GpuHandle,
        offset: u64,
        data: &[u8],
    ) -> Result<WireCommandRuntimeValue, String> {
        self.ensure_requirement(CommandRuntimeRequirements::GPU_COMMANDS)?;
        let queue = self
            .viewer
            .gpu_queue()
            .ok_or_else(|| "GPU command runtime requires an active queue".to_string())?;
        let target = self.gpu_handles.buffer(buffer)?;
        validate_buffer_range(target.size, offset, data.len() as u64)?;
        queue.write_buffer(&target.buffer, offset, data);
        Ok(WireCommandRuntimeValue::GpuOk)
    }

    fn gpu_copy_buffer_to_buffer(
        &mut self,
        source: GpuHandle,
        source_offset: u64,
        destination: GpuHandle,
        destination_offset: u64,
        size: u64,
    ) -> Result<WireCommandRuntimeValue, String> {
        self.ensure_requirement(CommandRuntimeRequirements::GPU_COMMANDS)?;
        let device = self
            .viewer
            .gpu_device()
            .ok_or_else(|| "GPU command runtime requires an active device".to_string())?;
        let queue = self
            .viewer
            .gpu_queue()
            .ok_or_else(|| "GPU command runtime requires an active queue".to_string())?;
        let source = self.gpu_handles.buffer(source)?;
        let destination = self.gpu_handles.buffer(destination)?;
        validate_buffer_range(source.size, source_offset, size)?;
        validate_buffer_range(destination.size, destination_offset, size)?;
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("patinae.plugin.gpu.copy_buffer_to_buffer"),
        });
        encoder.copy_buffer_to_buffer(
            &source.buffer,
            source_offset,
            &destination.buffer,
            destination_offset,
            size,
        );
        queue.submit(std::iter::once(encoder.finish()));
        Ok(WireCommandRuntimeValue::GpuOk)
    }

    fn gpu_read_buffer(
        &mut self,
        buffer: GpuHandle,
        offset: u64,
        size: u64,
    ) -> Result<WireCommandRuntimeValue, String> {
        self.ensure_requirement(CommandRuntimeRequirements::GPU_COMMANDS)?;
        let device = self
            .viewer
            .gpu_device()
            .ok_or_else(|| "GPU command runtime requires an active device".to_string())?;
        let queue = self
            .viewer
            .gpu_queue()
            .ok_or_else(|| "GPU command runtime requires an active queue".to_string())?;
        let source = self.gpu_handles.buffer(buffer)?;
        validate_buffer_range(source.size, offset, size)?;
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("patinae.plugin.gpu.readback"),
            size: size.max(4),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("patinae.plugin.gpu.readback.copy"),
        });
        encoder.copy_buffer_to_buffer(&source.buffer, offset, &staging, 0, size);
        queue.submit(std::iter::once(encoder.finish()));

        let bytes = map_readback_buffer(device, staging, size)?;
        Ok(WireCommandRuntimeValue::GpuBytes(bytes))
    }

    fn gpu_create_shader_module(
        &mut self,
        descriptor: GpuShaderModuleDescriptor,
    ) -> Result<WireCommandRuntimeValue, String> {
        self.ensure_requirement(CommandRuntimeRequirements::GPU_COMMANDS)?;
        let fingerprint = shader_module_fingerprint(&descriptor);
        let device = self
            .viewer
            .gpu_device()
            .ok_or_else(|| "GPU command runtime requires an active device".to_string())?;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: descriptor.label.as_deref(),
            source: wgpu::ShaderSource::Wgsl(descriptor.wgsl.into()),
        });
        let handle = self.gpu_handles.insert_shader_module(module, fingerprint);
        Ok(WireCommandRuntimeValue::GpuHandle(handle))
    }

    fn gpu_create_bind_group_layout(
        &mut self,
        descriptor: GpuBindGroupLayoutDescriptor,
    ) -> Result<WireCommandRuntimeValue, String> {
        self.ensure_requirement(CommandRuntimeRequirements::GPU_COMMANDS)?;
        let fingerprint = bind_group_layout_fingerprint(&descriptor);
        let device = self
            .viewer
            .gpu_device()
            .ok_or_else(|| "GPU command runtime requires an active device".to_string())?;
        let entries = descriptor
            .entries
            .iter()
            .copied()
            .map(wgpu_bind_group_layout_entry)
            .collect::<Result<Vec<_>, _>>()?;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: descriptor.label.as_deref(),
            entries: &entries,
        });
        let handle =
            self.gpu_handles
                .insert_bind_group_layout(layout, fingerprint, descriptor.entries);
        Ok(WireCommandRuntimeValue::GpuHandle(handle))
    }

    fn gpu_create_pipeline_layout(
        &mut self,
        descriptor: GpuPipelineLayoutDescriptor,
    ) -> Result<WireCommandRuntimeValue, String> {
        self.ensure_requirement(CommandRuntimeRequirements::GPU_COMMANDS)?;
        let fingerprint = self.pipeline_layout_fingerprint(&descriptor)?;
        let device = self
            .viewer
            .gpu_device()
            .ok_or_else(|| "GPU command runtime requires an active device".to_string())?;
        let layout = {
            let layouts = descriptor
                .bind_group_layouts
                .iter()
                .map(|handle| self.gpu_handles.bind_group_layout(*handle))
                .collect::<Result<Vec<_>, _>>()?;
            let bind_group_layouts: Vec<_> = layouts.iter().map(|layout| Some(*layout)).collect();
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: descriptor.label.as_deref(),
                bind_group_layouts: &bind_group_layouts,
                immediate_size: 0,
            })
        };
        let handle = self.gpu_handles.insert_pipeline_layout(layout, fingerprint);
        Ok(WireCommandRuntimeValue::GpuHandle(handle))
    }

    fn gpu_create_compute_pipeline(
        &mut self,
        descriptor: GpuComputePipelineDescriptor,
    ) -> Result<WireCommandRuntimeValue, String> {
        self.ensure_requirement(CommandRuntimeRequirements::GPU_COMMANDS)?;
        let fingerprint = self.compute_pipeline_fingerprint(&descriptor)?;
        let device = self
            .viewer
            .gpu_device()
            .ok_or_else(|| "GPU command runtime requires an active device".to_string())?;
        let pipeline = {
            let layout = self.gpu_handles.pipeline_layout(descriptor.layout)?;
            let module = self.gpu_handles.shader_module(descriptor.module)?;
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: descriptor.label.as_deref(),
                layout: Some(layout),
                module,
                entry_point: Some(&descriptor.entry_point),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        };
        let handle = self
            .gpu_handles
            .insert_compute_pipeline(pipeline, fingerprint);
        Ok(WireCommandRuntimeValue::GpuHandle(handle))
    }

    fn gpu_create_render_pipeline(
        &mut self,
        descriptor: GpuRenderPipelineDescriptor,
    ) -> Result<WireCommandRuntimeValue, String> {
        self.ensure_requirement(CommandRuntimeRequirements::GPU_COMMANDS)?;
        validate_render_pipeline_descriptor(&descriptor)?;
        let fingerprint = self.render_pipeline_fingerprint(&descriptor)?;
        let device = self
            .viewer
            .gpu_device()
            .ok_or_else(|| "GPU command runtime requires an active device".to_string())?;
        let pipeline = self.create_render_pipeline(device, &descriptor)?;
        let handle = self.gpu_handles.insert_render_pipeline(
            pipeline,
            render_pipeline_metadata(&descriptor),
            fingerprint,
        );
        Ok(WireCommandRuntimeValue::GpuHandle(handle))
    }

    fn gpu_create_cached_shader_module(
        &mut self,
        descriptor: GpuShaderModuleDescriptor,
    ) -> Result<WireCommandRuntimeValue, String> {
        self.ensure_requirement(CommandRuntimeRequirements::GPU_COMMANDS)?;
        let fingerprint = shader_module_fingerprint(&descriptor);
        let device = self
            .viewer
            .gpu_device()
            .ok_or_else(|| "GPU command runtime requires an active device".to_string())?;
        let start = std::time::Instant::now();
        let mut cache = self.lock_gpu_cache()?;
        let key = cache.resource_key(device, fingerprint);
        let (module, status) = if let Some(object) = cache.cached_object(&key) {
            let module = object.into_shader_module()?;
            cache.record_hit();
            (module, GpuCacheStatus::Hit)
        } else {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: descriptor.label.as_deref(),
                source: wgpu::ShaderSource::Wgsl(descriptor.wgsl.clone().into()),
            });
            cache.insert(key, GpuCachedObject::ShaderModule(module.clone()));
            (module, GpuCacheStatus::Miss)
        };
        cache.record_profile_event(status, start.elapsed().as_millis());
        drop(cache);
        let handle = self.gpu_handles.insert_shader_module(module, fingerprint);
        Ok(WireCommandRuntimeValue::GpuCachedHandle(GpuCachedHandle {
            handle,
            status,
        }))
    }

    fn gpu_create_cached_bind_group_layout(
        &mut self,
        descriptor: GpuBindGroupLayoutDescriptor,
    ) -> Result<WireCommandRuntimeValue, String> {
        self.ensure_requirement(CommandRuntimeRequirements::GPU_COMMANDS)?;
        let fingerprint = bind_group_layout_fingerprint(&descriptor);
        let entries = descriptor
            .entries
            .iter()
            .copied()
            .map(wgpu_bind_group_layout_entry)
            .collect::<Result<Vec<_>, _>>()?;
        let device = self
            .viewer
            .gpu_device()
            .ok_or_else(|| "GPU command runtime requires an active device".to_string())?;
        let start = std::time::Instant::now();
        let mut cache = self.lock_gpu_cache()?;
        let key = cache.resource_key(device, fingerprint);
        let (layout, status) = if let Some(object) = cache.cached_object(&key) {
            let layout = object.into_bind_group_layout()?;
            cache.record_hit();
            (layout, GpuCacheStatus::Hit)
        } else {
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: descriptor.label.as_deref(),
                entries: &entries,
            });
            cache.insert(key, GpuCachedObject::BindGroupLayout(layout.clone()));
            (layout, GpuCacheStatus::Miss)
        };
        cache.record_profile_event(status, start.elapsed().as_millis());
        drop(cache);
        let handle =
            self.gpu_handles
                .insert_bind_group_layout(layout, fingerprint, descriptor.entries);
        Ok(WireCommandRuntimeValue::GpuCachedHandle(GpuCachedHandle {
            handle,
            status,
        }))
    }

    fn gpu_create_cached_pipeline_layout(
        &mut self,
        descriptor: GpuPipelineLayoutDescriptor,
    ) -> Result<WireCommandRuntimeValue, String> {
        self.ensure_requirement(CommandRuntimeRequirements::GPU_COMMANDS)?;
        let fingerprint = self.pipeline_layout_fingerprint(&descriptor)?;
        let device = self
            .viewer
            .gpu_device()
            .ok_or_else(|| "GPU command runtime requires an active device".to_string())?;
        let start = std::time::Instant::now();
        let mut cache = self.lock_gpu_cache()?;
        let key = cache.resource_key(device, fingerprint);
        let (layout, status) = if let Some(object) = cache.cached_object(&key) {
            let layout = object.into_pipeline_layout()?;
            cache.record_hit();
            (layout, GpuCacheStatus::Hit)
        } else {
            let layout = {
                let layouts = descriptor
                    .bind_group_layouts
                    .iter()
                    .map(|handle| self.gpu_handles.bind_group_layout(*handle))
                    .collect::<Result<Vec<_>, _>>()?;
                let bind_group_layouts: Vec<_> =
                    layouts.iter().map(|layout| Some(*layout)).collect();
                device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: descriptor.label.as_deref(),
                    bind_group_layouts: &bind_group_layouts,
                    immediate_size: 0,
                })
            };
            cache.insert(key, GpuCachedObject::PipelineLayout(layout.clone()));
            (layout, GpuCacheStatus::Miss)
        };
        cache.record_profile_event(status, start.elapsed().as_millis());
        drop(cache);
        let handle = self.gpu_handles.insert_pipeline_layout(layout, fingerprint);
        Ok(WireCommandRuntimeValue::GpuCachedHandle(GpuCachedHandle {
            handle,
            status,
        }))
    }

    fn gpu_create_cached_compute_pipeline(
        &mut self,
        descriptor: GpuComputePipelineDescriptor,
    ) -> Result<WireCommandRuntimeValue, String> {
        self.ensure_requirement(CommandRuntimeRequirements::GPU_COMMANDS)?;
        let fingerprint = self.compute_pipeline_fingerprint(&descriptor)?;
        let device = self
            .viewer
            .gpu_device()
            .ok_or_else(|| "GPU command runtime requires an active device".to_string())?;
        let start = std::time::Instant::now();
        let mut cache = self.lock_gpu_cache()?;
        let key = cache.resource_key(device, fingerprint);
        let (pipeline, status) = if let Some(object) = cache.cached_object(&key) {
            let pipeline = object.into_compute_pipeline()?;
            cache.record_hit();
            (pipeline, GpuCacheStatus::Hit)
        } else {
            let pipeline = {
                let layout = self.gpu_handles.pipeline_layout(descriptor.layout)?;
                let module = self.gpu_handles.shader_module(descriptor.module)?;
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: descriptor.label.as_deref(),
                    layout: Some(layout),
                    module,
                    entry_point: Some(&descriptor.entry_point),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    cache: None,
                })
            };
            cache.insert(key, GpuCachedObject::ComputePipeline(pipeline.clone()));
            (pipeline, GpuCacheStatus::Miss)
        };
        cache.record_profile_event(status, start.elapsed().as_millis());
        drop(cache);
        let handle = self
            .gpu_handles
            .insert_compute_pipeline(pipeline, fingerprint);
        Ok(WireCommandRuntimeValue::GpuCachedHandle(GpuCachedHandle {
            handle,
            status,
        }))
    }

    fn gpu_create_cached_render_pipeline(
        &mut self,
        descriptor: GpuRenderPipelineDescriptor,
    ) -> Result<WireCommandRuntimeValue, String> {
        self.ensure_requirement(CommandRuntimeRequirements::GPU_COMMANDS)?;
        validate_render_pipeline_descriptor(&descriptor)?;
        let fingerprint = self.render_pipeline_fingerprint(&descriptor)?;
        let device = self
            .viewer
            .gpu_device()
            .ok_or_else(|| "GPU command runtime requires an active device".to_string())?;
        let start = std::time::Instant::now();
        let mut cache = self.lock_gpu_cache()?;
        let key = cache.resource_key(device, fingerprint);
        let (pipeline, status) = if let Some(object) = cache.cached_object(&key) {
            let pipeline = object.into_render_pipeline()?;
            cache.record_hit();
            (pipeline, GpuCacheStatus::Hit)
        } else {
            let pipeline = self.create_render_pipeline(device, &descriptor)?;
            cache.insert(key, GpuCachedObject::RenderPipeline(pipeline.clone()));
            (pipeline, GpuCacheStatus::Miss)
        };
        cache.record_profile_event(status, start.elapsed().as_millis());
        drop(cache);
        let handle = self.gpu_handles.insert_render_pipeline(
            pipeline,
            render_pipeline_metadata(&descriptor),
            fingerprint,
        );
        Ok(WireCommandRuntimeValue::GpuCachedHandle(GpuCachedHandle {
            handle,
            status,
        }))
    }

    fn gpu_cache_stats(&mut self) -> Result<WireCommandRuntimeValue, String> {
        self.ensure_requirement(CommandRuntimeRequirements::GPU_COMMANDS)?;
        let cache = self.lock_gpu_cache()?;
        Ok(WireCommandRuntimeValue::GpuCacheStats(cache.stats()))
    }

    fn gpu_drop_plugin_cache(&mut self) -> Result<WireCommandRuntimeValue, String> {
        self.ensure_requirement(CommandRuntimeRequirements::GPU_COMMANDS)?;
        self.lock_gpu_cache()?.clear();
        Ok(WireCommandRuntimeValue::GpuOk)
    }

    fn gpu_create_bind_group(
        &mut self,
        descriptor: GpuBindGroupDescriptor,
    ) -> Result<WireCommandRuntimeValue, String> {
        self.ensure_requirement(CommandRuntimeRequirements::GPU_COMMANDS)?;
        let device = self
            .viewer
            .gpu_device()
            .ok_or_else(|| "GPU command runtime requires an active device".to_string())?;
        let (bind_group, has_dynamic_offsets) = {
            let layout = self
                .gpu_handles
                .bind_group_layout_descriptor(descriptor.layout)?;
            let has_dynamic_offsets = bind_group_layout_has_dynamic_offsets(&layout.entries);
            let mut entries = Vec::with_capacity(descriptor.entries.len());
            for entry in descriptor.entries {
                let layout_entry = layout
                    .entries
                    .iter()
                    .find(|layout_entry| layout_entry.binding == entry.binding)
                    .ok_or_else(|| {
                        format!(
                            "GPU bind group entry {} is not declared by its layout",
                            entry.binding
                        )
                    })?;
                entries.push(wgpu_bind_group_entry(
                    &self.gpu_handles,
                    entry,
                    layout_entry.ty,
                )?);
            }
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: descriptor.label.as_deref(),
                layout: &layout.layout,
                entries: &entries,
            });
            (bind_group, has_dynamic_offsets)
        };
        let handle = self
            .gpu_handles
            .insert_bind_group(bind_group, has_dynamic_offsets);
        Ok(WireCommandRuntimeValue::GpuHandle(handle))
    }

    fn gpu_dispatch_compute(
        &mut self,
        pipeline: GpuHandle,
        bind_groups: &[GpuHandle],
        workgroups: [u32; 3],
    ) -> Result<WireCommandRuntimeValue, String> {
        self.ensure_requirement(CommandRuntimeRequirements::GPU_COMMANDS)?;
        let device = self
            .viewer
            .gpu_device()
            .ok_or_else(|| "GPU command runtime requires an active device".to_string())?;
        let queue = self
            .viewer
            .gpu_queue()
            .ok_or_else(|| "GPU command runtime requires an active queue".to_string())?;
        validate_dispatch_workgroups(
            workgroups,
            device.limits().max_compute_workgroups_per_dimension,
        )?;
        let pipeline = self.gpu_handles.compute_pipeline(pipeline)?;
        let bind_groups = bind_groups
            .iter()
            .map(|handle| self.gpu_handles.bind_group(*handle))
            .collect::<Result<Vec<_>, _>>()?;
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("patinae.plugin.gpu.dispatch_compute"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("patinae.plugin.gpu.dispatch_compute.pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(pipeline);
            for (index, bind_group) in bind_groups.iter().enumerate() {
                pass.set_bind_group(index as u32, Some(&bind_group.bind_group), &[]);
            }
            pass.dispatch_workgroups(workgroups[0], workgroups[1], workgroups[2]);
        }
        queue.submit(std::iter::once(encoder.finish()));
        Ok(WireCommandRuntimeValue::GpuOk)
    }

    fn gpu_submit_batch(
        &mut self,
        batch: GpuSubmitBatch,
    ) -> Result<WireCommandRuntimeValue, String> {
        self.ensure_requirement(CommandRuntimeRequirements::GPU_COMMANDS)?;
        let device = self
            .viewer
            .gpu_device()
            .ok_or_else(|| "GPU command runtime requires an active device".to_string())?;
        let queue = self
            .viewer
            .gpu_queue()
            .ok_or_else(|| "GPU command runtime requires an active queue".to_string())?;
        let record_start = std::time::Instant::now();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: batch.label.as_deref().or(Some("patinae.plugin.gpu.batch")),
        });
        let mut readbacks = Vec::new();
        let mut upload_staging = Vec::new();
        let mut stage_profile = GpuStageProfile::new(device, queue, &batch.commands);

        for command in batch.commands {
            match command {
                GpuBatchCommand::SetProfileScope { name } => {
                    stage_profile.set_scope(name);
                }
                GpuBatchCommand::WriteBuffer {
                    buffer,
                    offset,
                    data,
                } => {
                    let target = self.gpu_handles.buffer(buffer)?;
                    validate_buffer_range(target.size, offset, data.len() as u64)?;
                    let staging = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("patinae.plugin.gpu.batch.upload"),
                        contents: &data,
                        usage: wgpu::BufferUsages::COPY_SRC,
                    });
                    encoder.copy_buffer_to_buffer(
                        &staging,
                        0,
                        &target.buffer,
                        offset,
                        data.len() as u64,
                    );
                    upload_staging.push(staging);
                }
                GpuBatchCommand::DispatchCompute {
                    pipeline,
                    bind_groups,
                    workgroups,
                } => {
                    validate_dispatch_workgroups(
                        workgroups,
                        device.limits().max_compute_workgroups_per_dimension,
                    )?;
                    let pipeline = self.gpu_handles.compute_pipeline(pipeline)?;
                    let bind_groups = bind_groups
                        .iter()
                        .map(|handle| self.gpu_handles.bind_group(*handle))
                        .collect::<Result<Vec<_>, _>>()?;
                    let timestamp_writes = stage_profile.timestamp_writes();
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("patinae.plugin.gpu.batch.compute"),
                        timestamp_writes,
                    });
                    pass.set_pipeline(pipeline);
                    for (index, bind_group) in bind_groups.iter().enumerate() {
                        pass.set_bind_group(index as u32, Some(&bind_group.bind_group), &[]);
                    }
                    pass.dispatch_workgroups(workgroups[0], workgroups[1], workgroups[2]);
                }
                GpuBatchCommand::DispatchComputeIndirect {
                    pipeline,
                    bind_groups,
                    indirect_buffer,
                    indirect_offset,
                } => {
                    let indirect_buffer = self.gpu_handles.buffer(indirect_buffer)?;
                    validate_indirect_buffer(
                        indirect_buffer,
                        indirect_offset,
                        DISPATCH_INDIRECT_SIZE,
                        "compute indirect buffer",
                    )?;
                    let pipeline = self.gpu_handles.compute_pipeline(pipeline)?;
                    let bind_groups = bind_groups
                        .iter()
                        .map(|handle| self.gpu_handles.bind_group(*handle))
                        .collect::<Result<Vec<_>, _>>()?;
                    let timestamp_writes = stage_profile.timestamp_writes();
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("patinae.plugin.gpu.batch.compute_indirect"),
                        timestamp_writes,
                    });
                    pass.set_pipeline(pipeline);
                    for (index, bind_group) in bind_groups.iter().enumerate() {
                        pass.set_bind_group(index as u32, Some(&bind_group.bind_group), &[]);
                    }
                    pass.dispatch_workgroups_indirect(&indirect_buffer.buffer, indirect_offset);
                }
                GpuBatchCommand::CopyBufferToBuffer {
                    source,
                    source_offset,
                    destination,
                    destination_offset,
                    size,
                } => {
                    let source = self.gpu_handles.buffer(source)?;
                    let destination = self.gpu_handles.buffer(destination)?;
                    validate_buffer_range(source.size, source_offset, size)?;
                    validate_buffer_range(destination.size, destination_offset, size)?;
                    encoder.copy_buffer_to_buffer(
                        &source.buffer,
                        source_offset,
                        &destination.buffer,
                        destination_offset,
                        size,
                    );
                }
                GpuBatchCommand::CopyBufferToTexture {
                    source,
                    source_layout,
                    destination,
                    size,
                } => {
                    let source = self.gpu_handles.buffer(source)?;
                    let destination_texture = self.gpu_handles.texture(destination.texture)?;
                    validate_texture_usage(
                        destination_texture,
                        GpuTextureUsage::COPY_DST,
                        "copy_buffer_to_texture destination",
                    )?;
                    validate_texture_copy_range(destination_texture, &destination, size)?;
                    validate_texel_copy_buffer_layout(
                        destination_texture.format,
                        source_layout,
                        size,
                        source.size,
                    )?;
                    encoder.copy_buffer_to_texture(
                        wgpu::TexelCopyBufferInfo {
                            buffer: &source.buffer,
                            layout: wgpu_texel_copy_buffer_layout(source_layout),
                        },
                        wgpu_texel_copy_texture_info(destination_texture, destination),
                        wgpu_extent3d(size),
                    );
                }
                GpuBatchCommand::CopyTextureToBuffer {
                    source,
                    destination,
                    destination_layout,
                    size,
                } => {
                    let source_texture = self.gpu_handles.texture(source.texture)?;
                    let destination = self.gpu_handles.buffer(destination)?;
                    validate_texture_usage(
                        source_texture,
                        GpuTextureUsage::COPY_SRC,
                        "copy_texture_to_buffer source",
                    )?;
                    validate_texture_copy_range(source_texture, &source, size)?;
                    validate_texel_copy_buffer_layout(
                        source_texture.format,
                        destination_layout,
                        size,
                        destination.size,
                    )?;
                    encoder.copy_texture_to_buffer(
                        wgpu_texel_copy_texture_info(source_texture, source),
                        wgpu::TexelCopyBufferInfo {
                            buffer: &destination.buffer,
                            layout: wgpu_texel_copy_buffer_layout(destination_layout),
                        },
                        wgpu_extent3d(size),
                    );
                }
                GpuBatchCommand::CopyTextureToTexture {
                    source,
                    destination,
                    size,
                } => {
                    let source_texture = self.gpu_handles.texture(source.texture)?;
                    let destination_texture = self.gpu_handles.texture(destination.texture)?;
                    validate_texture_usage(
                        source_texture,
                        GpuTextureUsage::COPY_SRC,
                        "copy_texture_to_texture source",
                    )?;
                    validate_texture_usage(
                        destination_texture,
                        GpuTextureUsage::COPY_DST,
                        "copy_texture_to_texture destination",
                    )?;
                    if source_texture.format != destination_texture.format {
                        return Err(
                            "texture-to-texture copies require matching formats".to_string()
                        );
                    }
                    validate_texture_copy_range(source_texture, &source, size)?;
                    validate_texture_copy_range(destination_texture, &destination, size)?;
                    encoder.copy_texture_to_texture(
                        wgpu_texel_copy_texture_info(source_texture, source),
                        wgpu_texel_copy_texture_info(destination_texture, destination),
                        wgpu_extent3d(size),
                    );
                }
                GpuBatchCommand::ReadBuffer {
                    buffer,
                    offset,
                    size,
                } => {
                    let source = self.gpu_handles.buffer(buffer)?;
                    validate_buffer_range(source.size, offset, size)?;
                    let staging = device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("patinae.plugin.gpu.batch.readback"),
                        size: size.max(4),
                        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                        mapped_at_creation: false,
                    });
                    encoder.copy_buffer_to_buffer(&source.buffer, offset, &staging, 0, size);
                    readbacks.push((staging, size));
                }
                GpuBatchCommand::RenderPass {
                    descriptor,
                    commands,
                } => {
                    self.record_render_pass(&mut encoder, descriptor, &commands)?;
                }
            }
        }
        stage_profile.resolve(&mut encoder);

        let wait_for_completion = batch.wait_for_completion;
        let record_ms = record_start.elapsed().as_millis();
        let submit_start = std::time::Instant::now();
        let submission_index = queue.submit(std::iter::once(encoder.finish()));
        let submit_ms = submit_start.elapsed().as_millis();
        let completion_wait_start = std::time::Instant::now();
        if wait_for_completion {
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(submission_index),
                    timeout: None,
                })
                .map_err(|error| format!("device poll failed: {error:?}"))?;
        }
        let completion_wait_ms = completion_wait_start.elapsed().as_millis();
        let read_start = std::time::Instant::now();
        let mut bytes = Vec::with_capacity(readbacks.len());
        for (buffer, size) in readbacks {
            bytes.push(map_readback_buffer(device, buffer, size)?);
        }
        let stage_profile_readback_bytes = stage_profile.log_after_submit(device)?;
        if rt_profile_enabled() {
            let cache_profile = self.take_gpu_cache_profile()?;
            log::info!(
                "patinae.rt_profile host.batch_record_ms={} host.batch_submit_ms={} host.batch_completion_wait_ms={} host.batch_readback_ms={} readbacks={} stage_profile_readback_bytes={} cache_hits={} cache_misses={} cache_entries={} cache_lazy_create_ms={}",
                record_ms,
                submit_ms,
                completion_wait_ms,
                read_start.elapsed().as_millis(),
                bytes.len(),
                stage_profile_readback_bytes,
                cache_profile.hits,
                cache_profile.misses,
                cache_profile.entries,
                cache_profile.lazy_create_ms
            );
        }
        Ok(WireCommandRuntimeValue::GpuBatchResult(GpuBatchResult {
            readbacks: bytes,
        }))
    }

    fn gpu_drop_handles(
        &mut self,
        handles: &[GpuHandle],
    ) -> Result<WireCommandRuntimeValue, String> {
        self.ensure_requirement(CommandRuntimeRequirements::GPU_COMMANDS)?;
        for handle in handles {
            self.gpu_handles.drop_handle(*handle)?;
        }
        Ok(WireCommandRuntimeValue::GpuOk)
    }

    fn set_viewport_gpu_image_from_buffer(
        &mut self,
        buffer: GpuHandle,
        width: u32,
        height: u32,
    ) -> Result<WireCommandRuntimeValue, String> {
        self.ensure_requirement(CommandRuntimeRequirements::GPU_COMMANDS)?;
        let source = self.gpu_handles.buffer(buffer)?;
        let buffer = source.buffer.clone();
        let buffer_size = source.size;
        self.viewer
            .set_viewport_gpu_image_from_wgpu_buffer(&buffer, buffer_size, width, height)?;
        self.viewer.request_redraw();
        Ok(WireCommandRuntimeValue::GpuOk)
    }

    fn lock_gpu_cache(&self) -> Result<std::sync::MutexGuard<'_, GpuPluginCache>, String> {
        self.gpu_cache
            .lock()
            .map_err(|_| "GPU plugin cache was poisoned".to_string())
    }

    fn take_gpu_cache_profile(&self) -> Result<GpuCacheProfileDelta, String> {
        Ok(self.lock_gpu_cache()?.take_profile_delta())
    }

    fn create_render_pipeline(
        &self,
        device: &wgpu::Device,
        descriptor: &GpuRenderPipelineDescriptor,
    ) -> Result<wgpu::RenderPipeline, String> {
        let layout = self.gpu_handles.pipeline_layout(descriptor.layout)?;
        let vertex_module = self.gpu_handles.shader_module(descriptor.vertex.module)?;
        let vertex_attributes = descriptor
            .vertex
            .buffers
            .iter()
            .map(|layout| {
                layout
                    .attributes
                    .iter()
                    .copied()
                    .map(wgpu_vertex_attribute)
                    .collect::<Result<Vec<_>, _>>()
            })
            .collect::<Result<Vec<_>, _>>()?;
        let vertex_buffers = descriptor
            .vertex
            .buffers
            .iter()
            .zip(vertex_attributes.iter())
            .map(|(layout, attributes)| wgpu::VertexBufferLayout {
                array_stride: layout.array_stride,
                step_mode: wgpu_vertex_step_mode(layout.step_mode),
                attributes,
            })
            .collect::<Vec<_>>();
        let fragment_module = descriptor
            .fragment
            .as_ref()
            .map(|fragment| self.gpu_handles.shader_module(fragment.module))
            .transpose()?;
        let fragment_targets = descriptor
            .fragment
            .as_ref()
            .map(|fragment| {
                fragment
                    .targets
                    .iter()
                    .copied()
                    .map(|target| target.map(wgpu_color_target_state).transpose())
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_default();
        let fragment_state = descriptor
            .fragment
            .as_ref()
            .map(|fragment| wgpu::FragmentState {
                module: fragment_module.expect("fragment module was loaded above"),
                entry_point: Some(&fragment.entry_point),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &fragment_targets,
            });

        Ok(
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: descriptor.label.as_deref(),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: vertex_module,
                    entry_point: Some(&descriptor.vertex.entry_point),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &vertex_buffers,
                },
                primitive: wgpu_primitive_state(descriptor.primitive),
                depth_stencil: descriptor.depth_stencil.map(wgpu_depth_stencil_state),
                multisample: wgpu_multisample_state(descriptor.multisample),
                fragment: fragment_state,
                multiview_mask: None,
                cache: None,
            }),
        )
    }

    fn record_render_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        descriptor: GpuRenderPassDescriptor,
        commands: &[GpuRenderPassCommand],
    ) -> Result<(), String> {
        if descriptor.color_attachments.iter().all(Option::is_none)
            && descriptor.depth_stencil_attachment.is_none()
        {
            return Err("GPU render pass requires at least one attachment".to_string());
        }

        let mut pass_color_formats = Vec::with_capacity(descriptor.color_attachments.len());
        let color_attachments = descriptor
            .color_attachments
            .iter()
            .map(|attachment| {
                let Some(attachment) = attachment else {
                    pass_color_formats.push(None);
                    return Ok(None);
                };
                let view = self.gpu_handles.texture_view(attachment.view)?;
                validate_render_color_attachment(view)?;
                pass_color_formats.push(Some(view.format));
                Ok(Some(wgpu::RenderPassColorAttachment {
                    view: &view.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu_color_operations(attachment.ops),
                }))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let (depth_stencil_attachment, depth_format) =
            self.render_pass_depth_attachment(descriptor.depth_stencil_attachment)?;
        let attachment_formats = RenderPassAttachmentFormats {
            color_targets: pass_color_formats,
            depth_stencil: depth_format,
        };

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: descriptor.label.as_deref(),
            color_attachments: &color_attachments,
            depth_stencil_attachment,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        let mut state = RenderPassRecordingState::default();
        for command in commands {
            match *command {
                GpuRenderPassCommand::SetPipeline { pipeline } => {
                    let pipeline = self.gpu_handles.render_pipeline(pipeline)?;
                    validate_render_pipeline_compatible_with_pass(pipeline, &attachment_formats)?;
                    pass.set_pipeline(&pipeline.pipeline);
                    state.pipeline_set = true;
                }
                GpuRenderPassCommand::SetBindGroup { index, bind_group } => {
                    let bind_group = self.gpu_handles.bind_group(bind_group)?;
                    if bind_group.has_dynamic_offsets {
                        return Err(
                            "GPU render pass bind groups do not support dynamic offsets in v1"
                                .to_string(),
                        );
                    }
                    pass.set_bind_group(index, Some(&bind_group.bind_group), &[]);
                }
                GpuRenderPassCommand::SetVertexBuffer {
                    slot,
                    buffer,
                    offset,
                    size,
                } => {
                    let buffer = self.gpu_handles.buffer(buffer)?;
                    validate_buffer_usage(buffer, GpuBufferUsage::VERTEX, "render vertex buffer")?;
                    let len = validate_buffer_slice(buffer.size, offset, size)?;
                    pass.set_vertex_buffer(slot, buffer.buffer.slice(offset..offset + len));
                }
                GpuRenderPassCommand::SetIndexBuffer {
                    buffer,
                    format,
                    offset,
                    size,
                } => {
                    let buffer = self.gpu_handles.buffer(buffer)?;
                    validate_buffer_usage(buffer, GpuBufferUsage::INDEX, "render index buffer")?;
                    let len = validate_buffer_slice(buffer.size, offset, size)?;
                    let index_size = gpu_index_format_size(format);
                    if len % index_size != 0 {
                        return Err(
                            "GPU render index buffer range must contain whole indices".to_string()
                        );
                    }
                    pass.set_index_buffer(
                        buffer.buffer.slice(offset..offset + len),
                        wgpu_index_format(format),
                    );
                    state.index_buffer = Some(RenderPassIndexState {
                        format,
                        index_count: len / index_size,
                    });
                }
                GpuRenderPassCommand::SetViewport {
                    x,
                    y,
                    width,
                    height,
                    min_depth,
                    max_depth,
                } => {
                    validate_viewport(width, height, min_depth, max_depth)?;
                    pass.set_viewport(x, y, width, height, min_depth, max_depth);
                }
                GpuRenderPassCommand::SetScissorRect {
                    x,
                    y,
                    width,
                    height,
                } => {
                    validate_scissor_rect(width, height)?;
                    pass.set_scissor_rect(x, y, width, height);
                }
                GpuRenderPassCommand::Draw {
                    vertex_start,
                    vertex_count,
                    instance_start,
                    instance_count,
                } => {
                    validate_draw_counts(vertex_count, instance_count)?;
                    state.ensure_pipeline_set()?;
                    let vertex_end = checked_draw_range("vertex", vertex_start, vertex_count)?;
                    let instance_end =
                        checked_draw_range("instance", instance_start, instance_count)?;
                    pass.draw(vertex_start..vertex_end, instance_start..instance_end);
                }
                GpuRenderPassCommand::DrawIndexed {
                    index_start,
                    index_count,
                    base_vertex,
                    instance_start,
                    instance_count,
                } => {
                    validate_draw_counts(index_count, instance_count)?;
                    state.ensure_pipeline_set()?;
                    state.validate_index_range(index_start, index_count)?;
                    let index_end = checked_draw_range("index", index_start, index_count)?;
                    let instance_end =
                        checked_draw_range("instance", instance_start, instance_count)?;
                    pass.draw_indexed(
                        index_start..index_end,
                        base_vertex,
                        instance_start..instance_end,
                    );
                }
                GpuRenderPassCommand::DrawIndirect { buffer, offset } => {
                    state.ensure_pipeline_set()?;
                    let buffer = self.gpu_handles.buffer(buffer)?;
                    validate_indirect_buffer(
                        buffer,
                        offset,
                        DRAW_INDIRECT_SIZE,
                        "render indirect buffer",
                    )?;
                    pass.draw_indirect(&buffer.buffer, offset);
                }
                GpuRenderPassCommand::DrawIndexedIndirect { buffer, offset } => {
                    state.ensure_pipeline_set()?;
                    state.ensure_index_buffer_set()?;
                    let buffer = self.gpu_handles.buffer(buffer)?;
                    validate_indirect_buffer(
                        buffer,
                        offset,
                        DRAW_INDEXED_INDIRECT_SIZE,
                        "render indirect buffer",
                    )?;
                    pass.draw_indexed_indirect(&buffer.buffer, offset);
                }
            }
        }
        Ok(())
    }

    fn render_pass_depth_attachment(
        &self,
        attachment: Option<GpuRenderPassDepthStencilAttachment>,
    ) -> Result<
        (
            Option<wgpu::RenderPassDepthStencilAttachment<'_>>,
            Option<GpuTextureFormat>,
        ),
        String,
    > {
        let Some(attachment) = attachment else {
            return Ok((None, None));
        };
        let depth_ops = attachment
            .depth_ops
            .ok_or_else(|| "GPU render pass depth attachment requires depth ops".to_string())?;
        let view = self.gpu_handles.texture_view(attachment.view)?;
        validate_render_depth_attachment(view)?;
        Ok((
            Some(wgpu::RenderPassDepthStencilAttachment {
                view: &view.view,
                depth_ops: Some(wgpu_depth_operations(depth_ops)),
                stencil_ops: None,
            }),
            Some(view.format),
        ))
    }

    fn pipeline_layout_fingerprint(
        &self,
        descriptor: &GpuPipelineLayoutDescriptor,
    ) -> Result<GpuResourceFingerprint, String> {
        let layouts = descriptor
            .bind_group_layouts
            .iter()
            .map(|handle| {
                self.gpu_handles
                    .fingerprint(*handle, GpuHandleKind::BindGroupLayout)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(pipeline_layout_fingerprint(&layouts))
    }

    fn compute_pipeline_fingerprint(
        &self,
        descriptor: &GpuComputePipelineDescriptor,
    ) -> Result<GpuResourceFingerprint, String> {
        let layout = self
            .gpu_handles
            .fingerprint(descriptor.layout, GpuHandleKind::PipelineLayout)?;
        let module = self
            .gpu_handles
            .fingerprint(descriptor.module, GpuHandleKind::ShaderModule)?;
        Ok(compute_pipeline_fingerprint(
            layout,
            module,
            &descriptor.entry_point,
        ))
    }

    fn render_pipeline_fingerprint(
        &self,
        descriptor: &GpuRenderPipelineDescriptor,
    ) -> Result<GpuResourceFingerprint, String> {
        let layout = self
            .gpu_handles
            .fingerprint(descriptor.layout, GpuHandleKind::PipelineLayout)?;
        let vertex_module = self
            .gpu_handles
            .fingerprint(descriptor.vertex.module, GpuHandleKind::ShaderModule)?;
        let fragment = descriptor
            .fragment
            .as_ref()
            .map(|fragment| {
                self.gpu_handles
                    .fingerprint(fragment.module, GpuHandleKind::ShaderModule)
                    .map(|module| (module, fragment.entry_point.as_str(), &fragment.targets))
            })
            .transpose()?;
        Ok(render_pipeline_fingerprint(
            RenderPipelineFingerprintInput {
                layout,
                vertex_module,
                vertex_entry_point: &descriptor.vertex.entry_point,
                vertex_buffers: &descriptor.vertex.buffers,
                fragment,
                primitive: descriptor.primitive,
                depth_stencil: descriptor.depth_stencil,
                multisample: descriptor.multisample,
            },
        ))
    }
}

struct HostTraceGeometryStream {
    id: u64,
    chunks: VecDeque<Vec<u8>>,
}

struct HostRenderArtifactSnapshot {
    id: u64,
    layout_version: u32,
    scene_generation: u64,
    scene_bounds_min: [f32; 3],
    scene_bounds_max: [f32; 3],
    cull_pass_initialized: bool,
    buffers: Vec<HostRenderArtifactBuffer>,
    reps: Vec<HostRenderArtifactRep>,
}

struct HostRenderArtifactBuffer {
    role: RenderArtifactBufferRole,
    buffer: wgpu::Buffer,
    size: u64,
    stride: u64,
    element_count: u64,
}

struct HostRenderArtifactRep {
    object_id: u32,
    rep_kind: RenderArtifactRepKind,
    topology: RenderArtifactPrimitiveTopology,
    geometry_buffer_index: usize,
    count_buffer_index: Option<usize>,
    indirect_buffer_index: Option<usize>,
    element_count: u64,
    max_element_count: u64,
    atom_offset: u32,
    atom_count: u32,
    material_rgba: [f32; 4],
    transparency: f32,
}

#[derive(Clone)]
struct HostGpuBuffer {
    buffer: wgpu::Buffer,
    size: u64,
    usage: GpuBufferUsage,
}

#[derive(Clone)]
struct HostGpuTexture {
    texture: wgpu::Texture,
    size: GpuExtent3d,
    mip_level_count: u32,
    dimension: GpuTextureDimension,
    format: GpuTextureFormat,
    usage: GpuTextureUsage,
}

#[derive(Clone)]
struct HostGpuTextureView {
    view: wgpu::TextureView,
    format: GpuTextureFormat,
    usage: GpuTextureUsage,
}

#[derive(Clone)]
struct HostGpuBindGroupLayout {
    layout: wgpu::BindGroupLayout,
    entries: Vec<GpuBindGroupLayoutEntry>,
}

#[derive(Clone)]
struct HostGpuBindGroup {
    bind_group: wgpu::BindGroup,
    has_dynamic_offsets: bool,
}

#[derive(Clone)]
struct HostGpuRenderPipeline {
    pipeline: wgpu::RenderPipeline,
    color_targets: Vec<Option<GpuTextureFormat>>,
    depth_stencil: Option<GpuTextureFormat>,
}

struct RenderPipelineMetadata {
    color_targets: Vec<Option<GpuTextureFormat>>,
    depth_stencil: Option<GpuTextureFormat>,
}

struct RenderPassAttachmentFormats {
    color_targets: Vec<Option<GpuTextureFormat>>,
    depth_stencil: Option<GpuTextureFormat>,
}

#[derive(Default)]
struct RenderPassRecordingState {
    pipeline_set: bool,
    index_buffer: Option<RenderPassIndexState>,
}

struct RenderPassIndexState {
    format: GpuIndexFormat,
    index_count: u64,
}

impl RenderPassRecordingState {
    fn ensure_pipeline_set(&self) -> Result<(), String> {
        if self.pipeline_set {
            Ok(())
        } else {
            Err("GPU render draw requires a render pipeline".to_string())
        }
    }

    fn ensure_index_buffer_set(&self) -> Result<(), String> {
        if self.index_buffer.is_some() {
            Ok(())
        } else {
            Err("GPU indexed render draw requires an index buffer".to_string())
        }
    }

    fn validate_index_range(&self, index_start: u32, index_count: u32) -> Result<(), String> {
        let index_buffer = self
            .index_buffer
            .as_ref()
            .ok_or_else(|| "GPU indexed render draw requires an index buffer".to_string())?;
        let end = u64::from(index_start)
            .checked_add(u64::from(index_count))
            .ok_or_else(|| "GPU render index range overflow".to_string())?;
        if end > index_buffer.index_count {
            return Err(format!(
                "GPU render index range {}..{} exceeds {:?} index buffer count {}",
                index_start, end, index_buffer.format, index_buffer.index_count
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct GpuResourceFingerprint {
    kind: GpuHandleKind,
    hash: u64,
}

#[derive(Clone)]
enum GpuHandleObject {
    Buffer(HostGpuBuffer),
    Texture(HostGpuTexture),
    TextureView(HostGpuTextureView),
    Sampler(wgpu::Sampler),
    ShaderModule(wgpu::ShaderModule),
    BindGroupLayout(HostGpuBindGroupLayout),
    PipelineLayout(wgpu::PipelineLayout),
    BindGroup(HostGpuBindGroup),
    ComputePipeline(wgpu::ComputePipeline),
    RenderPipeline(HostGpuRenderPipeline),
}

struct GpuHandleEntry {
    generation: u64,
    kind: GpuHandleKind,
    fingerprint: Option<GpuResourceFingerprint>,
    object: GpuHandleObject,
}

struct GpuHandleRegistry {
    next_id: u64,
    generation: u64,
    entries: HashMap<u64, GpuHandleEntry>,
}

impl GpuHandleRegistry {
    fn new() -> Self {
        Self {
            next_id: 1,
            generation: 1,
            entries: HashMap::new(),
        }
    }

    fn insert_buffer(
        &mut self,
        buffer: wgpu::Buffer,
        size: u64,
        usage: GpuBufferUsage,
    ) -> GpuHandle {
        self.insert(
            GpuHandleKind::Buffer,
            GpuHandleObject::Buffer(HostGpuBuffer {
                buffer,
                size,
                usage,
            }),
            None,
        )
    }

    fn insert_texture(
        &mut self,
        texture: wgpu::Texture,
        descriptor: &GpuTextureDescriptor,
    ) -> GpuHandle {
        self.insert(
            GpuHandleKind::Texture,
            GpuHandleObject::Texture(HostGpuTexture {
                texture,
                size: descriptor.size,
                mip_level_count: descriptor.mip_level_count,
                dimension: descriptor.dimension,
                format: descriptor.format,
                usage: descriptor.usage,
            }),
            None,
        )
    }

    fn insert_texture_view(
        &mut self,
        view: wgpu::TextureView,
        format: GpuTextureFormat,
        usage: GpuTextureUsage,
    ) -> GpuHandle {
        self.insert(
            GpuHandleKind::TextureView,
            GpuHandleObject::TextureView(HostGpuTextureView {
                view,
                format,
                usage,
            }),
            None,
        )
    }

    fn insert_sampler(&mut self, sampler: wgpu::Sampler) -> GpuHandle {
        self.insert(
            GpuHandleKind::Sampler,
            GpuHandleObject::Sampler(sampler),
            None,
        )
    }

    fn insert_shader_module(
        &mut self,
        module: wgpu::ShaderModule,
        fingerprint: GpuResourceFingerprint,
    ) -> GpuHandle {
        self.insert(
            GpuHandleKind::ShaderModule,
            GpuHandleObject::ShaderModule(module),
            Some(fingerprint),
        )
    }

    fn insert_bind_group_layout(
        &mut self,
        layout: wgpu::BindGroupLayout,
        fingerprint: GpuResourceFingerprint,
        entries: Vec<GpuBindGroupLayoutEntry>,
    ) -> GpuHandle {
        self.insert(
            GpuHandleKind::BindGroupLayout,
            GpuHandleObject::BindGroupLayout(HostGpuBindGroupLayout { layout, entries }),
            Some(fingerprint),
        )
    }

    fn insert_pipeline_layout(
        &mut self,
        layout: wgpu::PipelineLayout,
        fingerprint: GpuResourceFingerprint,
    ) -> GpuHandle {
        self.insert(
            GpuHandleKind::PipelineLayout,
            GpuHandleObject::PipelineLayout(layout),
            Some(fingerprint),
        )
    }

    fn insert_bind_group(
        &mut self,
        bind_group: wgpu::BindGroup,
        has_dynamic_offsets: bool,
    ) -> GpuHandle {
        self.insert(
            GpuHandleKind::BindGroup,
            GpuHandleObject::BindGroup(HostGpuBindGroup {
                bind_group,
                has_dynamic_offsets,
            }),
            None,
        )
    }

    fn insert_compute_pipeline(
        &mut self,
        pipeline: wgpu::ComputePipeline,
        fingerprint: GpuResourceFingerprint,
    ) -> GpuHandle {
        self.insert(
            GpuHandleKind::ComputePipeline,
            GpuHandleObject::ComputePipeline(pipeline),
            Some(fingerprint),
        )
    }

    fn insert_render_pipeline(
        &mut self,
        pipeline: wgpu::RenderPipeline,
        metadata: RenderPipelineMetadata,
        fingerprint: GpuResourceFingerprint,
    ) -> GpuHandle {
        self.insert(
            GpuHandleKind::RenderPipeline,
            GpuHandleObject::RenderPipeline(HostGpuRenderPipeline {
                pipeline,
                color_targets: metadata.color_targets,
                depth_stencil: metadata.depth_stencil,
            }),
            Some(fingerprint),
        )
    }

    fn insert(
        &mut self,
        kind: GpuHandleKind,
        object: GpuHandleObject,
        fingerprint: Option<GpuResourceFingerprint>,
    ) -> GpuHandle {
        let id = self.next_id;
        self.next_id += 1;
        self.entries.insert(
            id,
            GpuHandleEntry {
                generation: self.generation,
                kind,
                fingerprint,
                object,
            },
        );
        GpuHandle {
            id,
            kind,
            generation: self.generation,
        }
    }

    fn drop_handle(&mut self, handle: GpuHandle) -> Result<(), String> {
        self.validate(handle)?;
        self.entries.remove(&handle.id);
        Ok(())
    }

    fn buffer(&self, handle: GpuHandle) -> Result<&HostGpuBuffer, String> {
        match &self.entry(handle, GpuHandleKind::Buffer)?.object {
            GpuHandleObject::Buffer(buffer) => Ok(buffer),
            _ => Err("GPU handle kind mismatch for buffer".to_string()),
        }
    }

    fn texture(&self, handle: GpuHandle) -> Result<&HostGpuTexture, String> {
        match &self.entry(handle, GpuHandleKind::Texture)?.object {
            GpuHandleObject::Texture(texture) => Ok(texture),
            _ => Err("GPU handle kind mismatch for texture".to_string()),
        }
    }

    fn texture_view(&self, handle: GpuHandle) -> Result<&HostGpuTextureView, String> {
        match &self.entry(handle, GpuHandleKind::TextureView)?.object {
            GpuHandleObject::TextureView(view) => Ok(view),
            _ => Err("GPU handle kind mismatch for texture view".to_string()),
        }
    }

    fn sampler(&self, handle: GpuHandle) -> Result<&wgpu::Sampler, String> {
        match &self.entry(handle, GpuHandleKind::Sampler)?.object {
            GpuHandleObject::Sampler(sampler) => Ok(sampler),
            _ => Err("GPU handle kind mismatch for sampler".to_string()),
        }
    }

    fn shader_module(&self, handle: GpuHandle) -> Result<&wgpu::ShaderModule, String> {
        match &self.entry(handle, GpuHandleKind::ShaderModule)?.object {
            GpuHandleObject::ShaderModule(module) => Ok(module),
            _ => Err("GPU handle kind mismatch for shader module".to_string()),
        }
    }

    fn bind_group_layout(&self, handle: GpuHandle) -> Result<&wgpu::BindGroupLayout, String> {
        match &self.entry(handle, GpuHandleKind::BindGroupLayout)?.object {
            GpuHandleObject::BindGroupLayout(layout) => Ok(&layout.layout),
            _ => Err("GPU handle kind mismatch for bind-group layout".to_string()),
        }
    }

    fn bind_group_layout_descriptor(
        &self,
        handle: GpuHandle,
    ) -> Result<&HostGpuBindGroupLayout, String> {
        match &self.entry(handle, GpuHandleKind::BindGroupLayout)?.object {
            GpuHandleObject::BindGroupLayout(layout) => Ok(layout),
            _ => Err("GPU handle kind mismatch for bind-group layout".to_string()),
        }
    }

    fn pipeline_layout(&self, handle: GpuHandle) -> Result<&wgpu::PipelineLayout, String> {
        match &self.entry(handle, GpuHandleKind::PipelineLayout)?.object {
            GpuHandleObject::PipelineLayout(layout) => Ok(layout),
            _ => Err("GPU handle kind mismatch for pipeline layout".to_string()),
        }
    }

    fn bind_group(&self, handle: GpuHandle) -> Result<&HostGpuBindGroup, String> {
        match &self.entry(handle, GpuHandleKind::BindGroup)?.object {
            GpuHandleObject::BindGroup(bind_group) => Ok(bind_group),
            _ => Err("GPU handle kind mismatch for bind group".to_string()),
        }
    }

    fn compute_pipeline(&self, handle: GpuHandle) -> Result<&wgpu::ComputePipeline, String> {
        match &self.entry(handle, GpuHandleKind::ComputePipeline)?.object {
            GpuHandleObject::ComputePipeline(pipeline) => Ok(pipeline),
            _ => Err("GPU handle kind mismatch for compute pipeline".to_string()),
        }
    }

    fn render_pipeline(&self, handle: GpuHandle) -> Result<&HostGpuRenderPipeline, String> {
        match &self.entry(handle, GpuHandleKind::RenderPipeline)?.object {
            GpuHandleObject::RenderPipeline(pipeline) => Ok(pipeline),
            _ => Err("GPU handle kind mismatch for render pipeline".to_string()),
        }
    }

    fn fingerprint(
        &self,
        handle: GpuHandle,
        expected: GpuHandleKind,
    ) -> Result<GpuResourceFingerprint, String> {
        let entry = self.entry(handle, expected)?;
        entry.fingerprint.ok_or_else(|| {
            format!(
                "GPU handle {} of kind {:?} cannot be used in cached descriptor",
                handle.id, expected
            )
        })
    }

    fn entry(&self, handle: GpuHandle, expected: GpuHandleKind) -> Result<&GpuHandleEntry, String> {
        self.validate(handle)?;
        let entry = self
            .entries
            .get(&handle.id)
            .ok_or_else(|| format!("GPU handle {} was not found", handle.id))?;
        if entry.kind != expected {
            return Err(format!(
                "GPU handle {} has kind {:?}, expected {:?}",
                handle.id, entry.kind, expected
            ));
        }
        Ok(entry)
    }

    fn validate(&self, handle: GpuHandle) -> Result<(), String> {
        let entry = self
            .entries
            .get(&handle.id)
            .ok_or_else(|| format!("GPU handle {} was not found", handle.id))?;
        if entry.generation != handle.generation {
            return Err(format!(
                "GPU handle {} generation mismatch: got {}, expected {}",
                handle.id, handle.generation, entry.generation
            ));
        }
        if entry.kind != handle.kind {
            return Err(format!(
                "GPU handle {} kind mismatch: got {:?}, expected {:?}",
                handle.id, handle.kind, entry.kind
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct GpuCacheKey {
    plugin_id_hash: u64,
    device_generation: u64,
    layout_version: u32,
    fingerprint: GpuResourceFingerprint,
}

#[derive(Clone)]
enum GpuCachedObject {
    ShaderModule(wgpu::ShaderModule),
    BindGroupLayout(wgpu::BindGroupLayout),
    PipelineLayout(wgpu::PipelineLayout),
    ComputePipeline(wgpu::ComputePipeline),
    RenderPipeline(wgpu::RenderPipeline),
}

impl GpuCachedObject {
    fn into_shader_module(self) -> Result<wgpu::ShaderModule, String> {
        match self {
            Self::ShaderModule(module) => Ok(module),
            _ => Err("GPU cache entry kind mismatch for shader module".to_string()),
        }
    }

    fn into_bind_group_layout(self) -> Result<wgpu::BindGroupLayout, String> {
        match self {
            Self::BindGroupLayout(layout) => Ok(layout),
            _ => Err("GPU cache entry kind mismatch for bind-group layout".to_string()),
        }
    }

    fn into_pipeline_layout(self) -> Result<wgpu::PipelineLayout, String> {
        match self {
            Self::PipelineLayout(layout) => Ok(layout),
            _ => Err("GPU cache entry kind mismatch for pipeline layout".to_string()),
        }
    }

    fn into_compute_pipeline(self) -> Result<wgpu::ComputePipeline, String> {
        match self {
            Self::ComputePipeline(pipeline) => Ok(pipeline),
            _ => Err("GPU cache entry kind mismatch for compute pipeline".to_string()),
        }
    }

    fn into_render_pipeline(self) -> Result<wgpu::RenderPipeline, String> {
        match self {
            Self::RenderPipeline(pipeline) => Ok(pipeline),
            _ => Err("GPU cache entry kind mismatch for render pipeline".to_string()),
        }
    }
}

struct GpuCacheEntry {
    object: GpuCachedObject,
}

#[derive(Clone, Copy, Default)]
struct GpuCacheProfileDelta {
    hits: u64,
    misses: u64,
    entries: u64,
    lazy_create_ms: u128,
}

struct GpuPluginCache {
    plugin_id_hash: u64,
    device_identity: Option<usize>,
    device_generation: u64,
    hits: u64,
    misses: u64,
    profile_hits: u64,
    profile_misses: u64,
    profile_lazy_create_ms: u128,
    entries: HashMap<GpuCacheKey, GpuCacheEntry>,
}

impl GpuPluginCache {
    fn new(plugin_id: String) -> Self {
        Self {
            plugin_id_hash: semantic_hash(&plugin_id),
            device_identity: None,
            device_generation: 0,
            hits: 0,
            misses: 0,
            profile_hits: 0,
            profile_misses: 0,
            profile_lazy_create_ms: 0,
            entries: HashMap::new(),
        }
    }

    fn resource_key(
        &mut self,
        device: &wgpu::Device,
        fingerprint: GpuResourceFingerprint,
    ) -> GpuCacheKey {
        self.resource_key_for_identity(device as *const wgpu::Device as usize, fingerprint)
    }

    fn resource_key_for_identity(
        &mut self,
        identity: usize,
        fingerprint: GpuResourceFingerprint,
    ) -> GpuCacheKey {
        let device_generation = self.ensure_device_identity(identity);
        GpuCacheKey {
            plugin_id_hash: self.plugin_id_hash,
            device_generation,
            layout_version: GPU_CACHE_LAYOUT_VERSION,
            fingerprint,
        }
    }

    fn cached_object(&self, key: &GpuCacheKey) -> Option<GpuCachedObject> {
        self.entries.get(key).map(|entry| entry.object.clone())
    }

    fn insert(&mut self, key: GpuCacheKey, object: GpuCachedObject) {
        self.entries.insert(key, GpuCacheEntry { object });
        self.record_miss();
    }

    fn record_hit(&mut self) {
        self.hits = self.hits.saturating_add(1);
    }

    fn record_miss(&mut self) {
        self.misses = self.misses.saturating_add(1);
    }

    fn record_profile_event(&mut self, status: GpuCacheStatus, elapsed_ms: u128) {
        match status {
            GpuCacheStatus::Hit => {
                self.profile_hits = self.profile_hits.saturating_add(1);
            }
            GpuCacheStatus::Miss => {
                self.profile_misses = self.profile_misses.saturating_add(1);
            }
        }
        self.profile_lazy_create_ms = self.profile_lazy_create_ms.saturating_add(elapsed_ms);
    }

    fn take_profile_delta(&mut self) -> GpuCacheProfileDelta {
        let delta = GpuCacheProfileDelta {
            hits: self.profile_hits,
            misses: self.profile_misses,
            entries: self.entries.len() as u64,
            lazy_create_ms: self.profile_lazy_create_ms,
        };
        self.profile_hits = 0;
        self.profile_misses = 0;
        self.profile_lazy_create_ms = 0;
        delta
    }

    fn stats(&self) -> GpuCacheStats {
        GpuCacheStats {
            hits: self.hits,
            misses: self.misses,
            entries: self.entries.len() as u64,
        }
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.profile_hits = 0;
        self.profile_misses = 0;
        self.profile_lazy_create_ms = 0;
    }

    fn ensure_device_identity(&mut self, identity: usize) -> u64 {
        if self.device_identity != Some(identity) {
            self.device_identity = Some(identity);
            self.device_generation = self.device_generation.saturating_add(1);
            self.entries.clear();
        }
        self.device_generation
    }
}

fn validate_render_color_attachment(view: &HostGpuTextureView) -> Result<(), String> {
    validate_texture_view_usage_bits(view, GpuTextureUsage::RENDER_ATTACHMENT, "color attachment")?;
    validate_color_format(view.format)
}

fn validate_render_depth_attachment(view: &HostGpuTextureView) -> Result<(), String> {
    validate_texture_view_usage_bits(view, GpuTextureUsage::RENDER_ATTACHMENT, "depth attachment")?;
    validate_depth_format(view.format)
}

fn validate_render_pipeline_compatible_with_pass(
    pipeline: &HostGpuRenderPipeline,
    pass: &RenderPassAttachmentFormats,
) -> Result<(), String> {
    validate_render_pipeline_formats(&pipeline.color_targets, pipeline.depth_stencil, pass)
}

fn validate_render_pipeline_formats(
    pipeline_color_targets: &[Option<GpuTextureFormat>],
    pipeline_depth_stencil: Option<GpuTextureFormat>,
    pass: &RenderPassAttachmentFormats,
) -> Result<(), String> {
    if pipeline_color_targets != pass.color_targets {
        return Err(format!(
            "GPU render pipeline color targets {:?} do not match pass attachments {:?}",
            pipeline_color_targets, pass.color_targets
        ));
    }
    if pipeline_depth_stencil != pass.depth_stencil {
        return Err(format!(
            "GPU render pipeline depth target {:?} does not match pass attachment {:?}",
            pipeline_depth_stencil, pass.depth_stencil
        ));
    }
    Ok(())
}

fn validate_buffer_usage(
    buffer: &HostGpuBuffer,
    required: GpuBufferUsage,
    context: &str,
) -> Result<(), String> {
    validate_buffer_usage_bits(buffer.usage, required, context)
}

fn validate_buffer_usage_bits(
    usage: GpuBufferUsage,
    required: GpuBufferUsage,
    context: &str,
) -> Result<(), String> {
    if !usage.contains(required) {
        return Err(format!(
            "{context} requires buffer usage bits {}",
            required.bits
        ));
    }
    Ok(())
}

fn validate_buffer_slice(size: u64, offset: u64, requested: Option<u64>) -> Result<u64, String> {
    let len = requested.unwrap_or_else(|| size.saturating_sub(offset));
    if len == 0 {
        return Err("GPU buffer slice size must be non-zero".to_string());
    }
    validate_buffer_range(size, offset, len)?;
    Ok(len)
}

fn validate_indirect_buffer(
    buffer: &HostGpuBuffer,
    offset: u64,
    command_size: u64,
    context: &str,
) -> Result<(), String> {
    validate_indirect_buffer_range(buffer.usage, buffer.size, offset, command_size, context)
}

fn validate_indirect_buffer_range(
    usage: GpuBufferUsage,
    buffer_size: u64,
    offset: u64,
    command_size: u64,
    context: &str,
) -> Result<(), String> {
    validate_buffer_usage_bits(usage, GpuBufferUsage::INDIRECT, context)?;
    if !offset.is_multiple_of(4) {
        return Err(format!("GPU {context} offset must be aligned to 4"));
    }
    validate_buffer_range(buffer_size, offset, command_size)
}

fn validate_viewport(
    width: f32,
    height: f32,
    min_depth: f32,
    max_depth: f32,
) -> Result<(), String> {
    if !(width.is_finite() && height.is_finite() && min_depth.is_finite() && max_depth.is_finite())
    {
        return Err("GPU render viewport values must be finite".to_string());
    }
    if width <= 0.0 || height <= 0.0 {
        return Err("GPU render viewport dimensions must be positive".to_string());
    }
    if min_depth < 0.0 || max_depth > 1.0 || min_depth > max_depth {
        return Err(
            "GPU render viewport depth range must satisfy 0 <= min <= max <= 1".to_string(),
        );
    }
    Ok(())
}

fn validate_scissor_rect(width: u32, height: u32) -> Result<(), String> {
    if width == 0 || height == 0 {
        return Err("GPU render scissor dimensions must be non-zero".to_string());
    }
    Ok(())
}

fn validate_draw_counts(primary_count: u32, instance_count: u32) -> Result<(), String> {
    if primary_count == 0 || instance_count == 0 {
        return Err("GPU render draw counts must be non-zero".to_string());
    }
    Ok(())
}

fn checked_draw_range(label: &str, start: u32, count: u32) -> Result<u32, String> {
    start
        .checked_add(count)
        .ok_or_else(|| format!("GPU render {label} range overflow"))
}

fn gpu_device_limits_from_viewer(viewer: &dyn ViewerLike) -> Result<GpuDeviceLimits, String> {
    let device = viewer
        .gpu_device()
        .ok_or_else(|| "GPU command runtime requires an active device".to_string())?;
    let limits = device.limits();
    let features = device.features();
    Ok(GpuDeviceLimits {
        max_buffer_size: limits.max_buffer_size,
        max_storage_buffer_binding_size: limits.max_storage_buffer_binding_size,
        max_compute_workgroups_per_dimension: limits.max_compute_workgroups_per_dimension,
        max_compute_invocations_per_workgroup: limits.max_compute_invocations_per_workgroup,
        max_compute_workgroup_size_x: limits.max_compute_workgroup_size_x,
        max_compute_workgroup_size_y: limits.max_compute_workgroup_size_y,
        max_compute_workgroup_size_z: limits.max_compute_workgroup_size_z,
        buffer_binding_array: features.contains(wgpu::Features::BUFFER_BINDING_ARRAY),
        storage_resource_binding_array: features
            .contains(wgpu::Features::STORAGE_RESOURCE_BINDING_ARRAY),
    })
}

fn validate_buffer_range(size: u64, offset: u64, len: u64) -> Result<(), String> {
    let end = offset
        .checked_add(len)
        .ok_or_else(|| "GPU buffer range overflow".to_string())?;
    if end > size {
        return Err(format!(
            "GPU buffer range {}..{} exceeds buffer size {}",
            offset, end, size
        ));
    }
    Ok(())
}

fn validate_texture_descriptor(descriptor: &GpuTextureDescriptor) -> Result<(), String> {
    if descriptor.size.width == 0
        || descriptor.size.height == 0
        || descriptor.size.depth_or_array_layers == 0
    {
        return Err("GPU texture dimensions must be non-zero".to_string());
    }
    if descriptor.mip_level_count == 0 {
        return Err("GPU texture mip_level_count must be non-zero".to_string());
    }
    if descriptor.sample_count == 0 {
        return Err("GPU texture sample_count must be non-zero".to_string());
    }
    wgpu_texture_usage(descriptor.usage)?;
    Ok(())
}

fn validate_texture_view_descriptor(
    texture: &HostGpuTexture,
    descriptor: &GpuTextureViewDescriptor,
) -> Result<(), String> {
    if let Some(format) = descriptor.format {
        if format != texture.format {
            return Err("GPU texture view format must match the texture format".to_string());
        }
    }
    validate_mip_range(
        descriptor.base_mip_level,
        descriptor.mip_level_count,
        texture.mip_level_count,
    )?;
    validate_layer_range(
        descriptor.base_array_layer,
        descriptor.array_layer_count,
        texture.size.depth_or_array_layers,
    )?;
    if let Some(usage) = descriptor.usage {
        wgpu_texture_usage(usage)?;
        if usage.bits & !texture.usage.bits != 0 {
            return Err("GPU texture view usage must be a subset of texture usage".to_string());
        }
    }
    Ok(())
}

fn validate_mip_range(base: u32, count: Option<u32>, total: u32) -> Result<(), String> {
    if base >= total {
        return Err(format!(
            "GPU texture mip base {} exceeds mip level count {}",
            base, total
        ));
    }
    if let Some(count) = count {
        if count == 0 {
            return Err("GPU texture mip count must be non-zero".to_string());
        }
        let end = base
            .checked_add(count)
            .ok_or_else(|| "GPU texture mip range overflow".to_string())?;
        if end > total {
            return Err(format!(
                "GPU texture mip range {}..{} exceeds mip level count {}",
                base, end, total
            ));
        }
    }
    Ok(())
}

fn validate_layer_range(base: u32, count: Option<u32>, total: u32) -> Result<(), String> {
    if base >= total {
        return Err(format!(
            "GPU texture layer base {} exceeds layer count {}",
            base, total
        ));
    }
    if let Some(count) = count {
        if count == 0 {
            return Err("GPU texture layer count must be non-zero".to_string());
        }
        let end = base
            .checked_add(count)
            .ok_or_else(|| "GPU texture layer range overflow".to_string())?;
        if end > total {
            return Err(format!(
                "GPU texture layer range {}..{} exceeds layer count {}",
                base, end, total
            ));
        }
    }
    Ok(())
}

fn validate_sampler_descriptor(descriptor: &GpuSamplerDescriptor) -> Result<(), String> {
    if descriptor.lod_min_clamp > descriptor.lod_max_clamp {
        return Err("GPU sampler lod_min_clamp must not exceed lod_max_clamp".to_string());
    }
    if descriptor.anisotropy_clamp > 1
        && (descriptor.mag_filter != GpuFilterMode::Linear
            || descriptor.min_filter != GpuFilterMode::Linear
            || descriptor.mipmap_filter != GpuFilterMode::Linear)
    {
        return Err("GPU sampler anisotropy requires linear filters".to_string());
    }
    Ok(())
}

fn validate_texture_usage(
    texture: &HostGpuTexture,
    required: GpuTextureUsage,
    context: &str,
) -> Result<(), String> {
    if !texture.usage.contains(required) {
        return Err(format!(
            "{context} requires texture usage bits {}",
            required.bits
        ));
    }
    Ok(())
}

fn validate_texture_copy_range(
    texture: &HostGpuTexture,
    info: &GpuTexelCopyTextureInfo,
    size: GpuExtent3d,
) -> Result<(), String> {
    validate_mip_range(info.mip_level, Some(1), texture.mip_level_count)?;
    if size.width == 0 || size.height == 0 || size.depth_or_array_layers == 0 {
        return Err("GPU texture copy size must be non-zero".to_string());
    }
    let mip_size = texture_mip_extent(texture, info.mip_level);
    validate_texture_axis_range("x", info.origin.x, size.width, mip_size.width)?;
    validate_texture_axis_range("y", info.origin.y, size.height, mip_size.height)?;
    validate_texture_axis_range(
        "z",
        info.origin.z,
        size.depth_or_array_layers,
        mip_size.depth_or_array_layers,
    )?;
    Ok(())
}

fn validate_texture_axis_range(
    axis: &str,
    origin: u32,
    size: u32,
    limit: u32,
) -> Result<(), String> {
    let end = origin
        .checked_add(size)
        .ok_or_else(|| format!("GPU texture copy {axis} range overflow"))?;
    if end > limit {
        return Err(format!(
            "GPU texture copy {axis} range {}..{} exceeds extent {}",
            origin, end, limit
        ));
    }
    Ok(())
}

fn texture_mip_extent(texture: &HostGpuTexture, mip_level: u32) -> GpuExtent3d {
    let size = texture.size;
    GpuExtent3d {
        width: mip_dimension(size.width, mip_level),
        height: mip_dimension(size.height, mip_level),
        depth_or_array_layers: match texture.dimension {
            GpuTextureDimension::D3 => mip_dimension(size.depth_or_array_layers, mip_level),
            GpuTextureDimension::D1 | GpuTextureDimension::D2 => size.depth_or_array_layers,
        },
    }
}

fn mip_dimension(value: u32, mip_level: u32) -> u32 {
    value.checked_shr(mip_level).unwrap_or(0).max(1)
}

fn validate_texel_copy_buffer_layout(
    format: GpuTextureFormat,
    layout: GpuTexelCopyBufferLayout,
    size: GpuExtent3d,
    buffer_size: u64,
) -> Result<(), String> {
    let bytes_per_texel = texture_format_bytes_per_texel(format).ok_or_else(|| {
        format!(
            "GPU texture format {:?} cannot be copied through buffers",
            format
        )
    })?;
    let row_bytes = u64::from(size.width)
        .checked_mul(u64::from(bytes_per_texel))
        .ok_or_else(|| "GPU texture copy row byte size overflow".to_string())?;
    let multiple_rows = size.height > 1 || size.depth_or_array_layers > 1;
    let bytes_per_row_was_provided = layout.bytes_per_row.is_some();
    let bytes_per_row = match layout.bytes_per_row {
        Some(bytes_per_row) => u64::from(bytes_per_row),
        None if multiple_rows => {
            return Err(
                "GPU texture copy bytes_per_row is required for multi-row copies".to_string(),
            );
        }
        None => row_bytes,
    };
    if bytes_per_row < row_bytes {
        return Err(format!(
            "GPU texture copy bytes_per_row {} is smaller than row size {}",
            bytes_per_row, row_bytes
        ));
    }
    if bytes_per_row_was_provided
        && bytes_per_row % u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) != 0
    {
        return Err(format!(
            "GPU texture copy bytes_per_row {} must be aligned to {}",
            bytes_per_row,
            wgpu::COPY_BYTES_PER_ROW_ALIGNMENT
        ));
    }

    let rows_per_image = match layout.rows_per_image {
        Some(rows_per_image) => u64::from(rows_per_image),
        None if size.depth_or_array_layers > 1 => {
            return Err(
                "GPU texture copy rows_per_image is required for multi-layer copies".to_string(),
            );
        }
        None => u64::from(size.height),
    };
    if rows_per_image < u64::from(size.height) {
        return Err(format!(
            "GPU texture copy rows_per_image {} is smaller than copy height {}",
            rows_per_image, size.height
        ));
    }

    let image_stride = rows_per_image
        .checked_mul(bytes_per_row)
        .ok_or_else(|| "GPU texture copy image stride overflow".to_string())?;
    let last_image_offset = u64::from(size.depth_or_array_layers.saturating_sub(1))
        .checked_mul(image_stride)
        .ok_or_else(|| "GPU texture copy last image offset overflow".to_string())?;
    let last_row_offset = u64::from(size.height.saturating_sub(1))
        .checked_mul(bytes_per_row)
        .ok_or_else(|| "GPU texture copy last row offset overflow".to_string())?;
    let required = last_image_offset
        .checked_add(last_row_offset)
        .and_then(|value| value.checked_add(row_bytes))
        .ok_or_else(|| "GPU texture copy required byte size overflow".to_string())?;
    validate_buffer_range(buffer_size, layout.offset, required)
}

fn texture_format_bytes_per_texel(format: GpuTextureFormat) -> Option<u32> {
    let bytes = match format {
        GpuTextureFormat::R8Unorm | GpuTextureFormat::R8Uint | GpuTextureFormat::R8Sint => 1,
        GpuTextureFormat::R16Uint
        | GpuTextureFormat::R16Sint
        | GpuTextureFormat::R16Float
        | GpuTextureFormat::Rg8Unorm
        | GpuTextureFormat::Rg8Uint
        | GpuTextureFormat::Rg8Sint => 2,
        GpuTextureFormat::R32Uint
        | GpuTextureFormat::R32Sint
        | GpuTextureFormat::R32Float
        | GpuTextureFormat::Rg16Float
        | GpuTextureFormat::Rgba8Unorm
        | GpuTextureFormat::Rgba8UnormSrgb
        | GpuTextureFormat::Rgba8Uint
        | GpuTextureFormat::Rgba8Sint
        | GpuTextureFormat::Bgra8Unorm
        | GpuTextureFormat::Bgra8UnormSrgb
        | GpuTextureFormat::Depth32Float => 4,
        GpuTextureFormat::Rg32Float | GpuTextureFormat::Rgba16Float => 8,
        GpuTextureFormat::Rgba32Float => 16,
    };
    Some(bytes)
}

fn validate_dispatch_workgroups(
    workgroups: [u32; 3],
    max_workgroups_per_dimension: u32,
) -> Result<(), String> {
    for (axis, count) in ["x", "y", "z"].into_iter().zip(workgroups) {
        if count > max_workgroups_per_dimension {
            return Err(format!(
                "compute dispatch workgroups.{axis}={} exceeds GPU limit {}",
                count, max_workgroups_per_dimension
            ));
        }
    }
    Ok(())
}

fn map_readback_buffer(
    device: &wgpu::Device,
    buffer: wgpu::Buffer,
    size: u64,
) -> Result<Vec<u8>, String> {
    let slice = buffer.slice(0..size);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .map_err(|error| error.to_string())?;
    rx.recv()
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;
    let mapped = slice.get_mapped_range();
    let bytes = mapped.to_vec();
    drop(mapped);
    buffer.unmap();
    Ok(bytes)
}

fn wgpu_buffer_usage(usage: GpuBufferUsage) -> Result<wgpu::BufferUsages, String> {
    let known = GpuBufferUsage::MAP_READ
        .union(GpuBufferUsage::COPY_SRC)
        .union(GpuBufferUsage::COPY_DST)
        .union(GpuBufferUsage::INDEX)
        .union(GpuBufferUsage::VERTEX)
        .union(GpuBufferUsage::UNIFORM)
        .union(GpuBufferUsage::STORAGE)
        .union(GpuBufferUsage::INDIRECT)
        .union(GpuBufferUsage::QUERY_RESOLVE);
    if usage.bits & !known.bits != 0 {
        return Err(format!("unknown GPU buffer usage bits: {}", usage.bits));
    }
    let mut out = wgpu::BufferUsages::empty();
    if usage.contains(GpuBufferUsage::MAP_READ) {
        out |= wgpu::BufferUsages::MAP_READ;
    }
    if usage.contains(GpuBufferUsage::COPY_SRC) {
        out |= wgpu::BufferUsages::COPY_SRC;
    }
    if usage.contains(GpuBufferUsage::COPY_DST) {
        out |= wgpu::BufferUsages::COPY_DST;
    }
    if usage.contains(GpuBufferUsage::INDEX) {
        out |= wgpu::BufferUsages::INDEX;
    }
    if usage.contains(GpuBufferUsage::VERTEX) {
        out |= wgpu::BufferUsages::VERTEX;
    }
    if usage.contains(GpuBufferUsage::UNIFORM) {
        out |= wgpu::BufferUsages::UNIFORM;
    }
    if usage.contains(GpuBufferUsage::STORAGE) {
        out |= wgpu::BufferUsages::STORAGE;
    }
    if usage.contains(GpuBufferUsage::INDIRECT) {
        out |= wgpu::BufferUsages::INDIRECT;
    }
    if usage.contains(GpuBufferUsage::QUERY_RESOLVE) {
        out |= wgpu::BufferUsages::QUERY_RESOLVE;
    }
    Ok(out)
}

fn wgpu_texture_usage(usage: GpuTextureUsage) -> Result<wgpu::TextureUsages, String> {
    let known = GpuTextureUsage::COPY_SRC
        .union(GpuTextureUsage::COPY_DST)
        .union(GpuTextureUsage::TEXTURE_BINDING)
        .union(GpuTextureUsage::STORAGE_BINDING)
        .union(GpuTextureUsage::RENDER_ATTACHMENT);
    if usage.bits & !known.bits != 0 {
        return Err(format!("unknown GPU texture usage bits: {}", usage.bits));
    }
    let mut out = wgpu::TextureUsages::empty();
    if usage.contains(GpuTextureUsage::COPY_SRC) {
        out |= wgpu::TextureUsages::COPY_SRC;
    }
    if usage.contains(GpuTextureUsage::COPY_DST) {
        out |= wgpu::TextureUsages::COPY_DST;
    }
    if usage.contains(GpuTextureUsage::TEXTURE_BINDING) {
        out |= wgpu::TextureUsages::TEXTURE_BINDING;
    }
    if usage.contains(GpuTextureUsage::STORAGE_BINDING) {
        out |= wgpu::TextureUsages::STORAGE_BINDING;
    }
    if usage.contains(GpuTextureUsage::RENDER_ATTACHMENT) {
        out |= wgpu::TextureUsages::RENDER_ATTACHMENT;
    }
    Ok(out)
}

fn wgpu_extent3d(extent: GpuExtent3d) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: extent.width,
        height: extent.height,
        depth_or_array_layers: extent.depth_or_array_layers,
    }
}

fn wgpu_origin3d(origin: GpuOrigin3d) -> wgpu::Origin3d {
    wgpu::Origin3d {
        x: origin.x,
        y: origin.y,
        z: origin.z,
    }
}

fn wgpu_texture_dimension(dimension: GpuTextureDimension) -> wgpu::TextureDimension {
    match dimension {
        GpuTextureDimension::D1 => wgpu::TextureDimension::D1,
        GpuTextureDimension::D2 => wgpu::TextureDimension::D2,
        GpuTextureDimension::D3 => wgpu::TextureDimension::D3,
    }
}

fn wgpu_texture_view_dimension(dimension: GpuTextureViewDimension) -> wgpu::TextureViewDimension {
    match dimension {
        GpuTextureViewDimension::D1 => wgpu::TextureViewDimension::D1,
        GpuTextureViewDimension::D2 => wgpu::TextureViewDimension::D2,
        GpuTextureViewDimension::D2Array => wgpu::TextureViewDimension::D2Array,
        GpuTextureViewDimension::Cube => wgpu::TextureViewDimension::Cube,
        GpuTextureViewDimension::CubeArray => wgpu::TextureViewDimension::CubeArray,
        GpuTextureViewDimension::D3 => wgpu::TextureViewDimension::D3,
    }
}

fn wgpu_texture_aspect(aspect: GpuTextureAspect) -> wgpu::TextureAspect {
    match aspect {
        GpuTextureAspect::All => wgpu::TextureAspect::All,
        GpuTextureAspect::StencilOnly => wgpu::TextureAspect::StencilOnly,
        GpuTextureAspect::DepthOnly => wgpu::TextureAspect::DepthOnly,
    }
}

fn wgpu_texture_format(format: GpuTextureFormat) -> wgpu::TextureFormat {
    match format {
        GpuTextureFormat::R8Unorm => wgpu::TextureFormat::R8Unorm,
        GpuTextureFormat::R8Uint => wgpu::TextureFormat::R8Uint,
        GpuTextureFormat::R8Sint => wgpu::TextureFormat::R8Sint,
        GpuTextureFormat::R16Uint => wgpu::TextureFormat::R16Uint,
        GpuTextureFormat::R16Sint => wgpu::TextureFormat::R16Sint,
        GpuTextureFormat::R16Float => wgpu::TextureFormat::R16Float,
        GpuTextureFormat::R32Uint => wgpu::TextureFormat::R32Uint,
        GpuTextureFormat::R32Sint => wgpu::TextureFormat::R32Sint,
        GpuTextureFormat::R32Float => wgpu::TextureFormat::R32Float,
        GpuTextureFormat::Rg8Unorm => wgpu::TextureFormat::Rg8Unorm,
        GpuTextureFormat::Rg8Uint => wgpu::TextureFormat::Rg8Uint,
        GpuTextureFormat::Rg8Sint => wgpu::TextureFormat::Rg8Sint,
        GpuTextureFormat::Rg16Float => wgpu::TextureFormat::Rg16Float,
        GpuTextureFormat::Rg32Float => wgpu::TextureFormat::Rg32Float,
        GpuTextureFormat::Rgba8Unorm => wgpu::TextureFormat::Rgba8Unorm,
        GpuTextureFormat::Rgba8UnormSrgb => wgpu::TextureFormat::Rgba8UnormSrgb,
        GpuTextureFormat::Rgba8Uint => wgpu::TextureFormat::Rgba8Uint,
        GpuTextureFormat::Rgba8Sint => wgpu::TextureFormat::Rgba8Sint,
        GpuTextureFormat::Bgra8Unorm => wgpu::TextureFormat::Bgra8Unorm,
        GpuTextureFormat::Bgra8UnormSrgb => wgpu::TextureFormat::Bgra8UnormSrgb,
        GpuTextureFormat::Rgba16Float => wgpu::TextureFormat::Rgba16Float,
        GpuTextureFormat::Rgba32Float => wgpu::TextureFormat::Rgba32Float,
        GpuTextureFormat::Depth32Float => wgpu::TextureFormat::Depth32Float,
    }
}

fn wgpu_address_mode(mode: GpuAddressMode) -> wgpu::AddressMode {
    match mode {
        GpuAddressMode::ClampToEdge => wgpu::AddressMode::ClampToEdge,
        GpuAddressMode::Repeat => wgpu::AddressMode::Repeat,
        GpuAddressMode::MirrorRepeat => wgpu::AddressMode::MirrorRepeat,
        GpuAddressMode::ClampToBorder => wgpu::AddressMode::ClampToBorder,
    }
}

fn wgpu_filter_mode(mode: GpuFilterMode) -> wgpu::FilterMode {
    match mode {
        GpuFilterMode::Nearest => wgpu::FilterMode::Nearest,
        GpuFilterMode::Linear => wgpu::FilterMode::Linear,
    }
}

fn wgpu_mipmap_filter_mode(mode: GpuFilterMode) -> wgpu::MipmapFilterMode {
    match mode {
        GpuFilterMode::Nearest => wgpu::MipmapFilterMode::Nearest,
        GpuFilterMode::Linear => wgpu::MipmapFilterMode::Linear,
    }
}

fn wgpu_compare_function(function: GpuCompareFunction) -> wgpu::CompareFunction {
    match function {
        GpuCompareFunction::Never => wgpu::CompareFunction::Never,
        GpuCompareFunction::Less => wgpu::CompareFunction::Less,
        GpuCompareFunction::Equal => wgpu::CompareFunction::Equal,
        GpuCompareFunction::LessEqual => wgpu::CompareFunction::LessEqual,
        GpuCompareFunction::Greater => wgpu::CompareFunction::Greater,
        GpuCompareFunction::NotEqual => wgpu::CompareFunction::NotEqual,
        GpuCompareFunction::GreaterEqual => wgpu::CompareFunction::GreaterEqual,
        GpuCompareFunction::Always => wgpu::CompareFunction::Always,
    }
}

fn wgpu_vertex_step_mode(mode: GpuVertexStepMode) -> wgpu::VertexStepMode {
    match mode {
        GpuVertexStepMode::Vertex => wgpu::VertexStepMode::Vertex,
        GpuVertexStepMode::Instance => wgpu::VertexStepMode::Instance,
    }
}

fn wgpu_vertex_attribute(attribute: GpuVertexAttribute) -> Result<wgpu::VertexAttribute, String> {
    Ok(wgpu::VertexAttribute {
        format: wgpu_vertex_format(attribute.format),
        offset: attribute.offset,
        shader_location: attribute.shader_location,
    })
}

fn wgpu_vertex_format(format: GpuVertexFormat) -> wgpu::VertexFormat {
    match format {
        GpuVertexFormat::Uint8 => wgpu::VertexFormat::Uint8,
        GpuVertexFormat::Uint8x2 => wgpu::VertexFormat::Uint8x2,
        GpuVertexFormat::Uint8x4 => wgpu::VertexFormat::Uint8x4,
        GpuVertexFormat::Sint8 => wgpu::VertexFormat::Sint8,
        GpuVertexFormat::Sint8x2 => wgpu::VertexFormat::Sint8x2,
        GpuVertexFormat::Sint8x4 => wgpu::VertexFormat::Sint8x4,
        GpuVertexFormat::Unorm8 => wgpu::VertexFormat::Unorm8,
        GpuVertexFormat::Unorm8x2 => wgpu::VertexFormat::Unorm8x2,
        GpuVertexFormat::Unorm8x4 => wgpu::VertexFormat::Unorm8x4,
        GpuVertexFormat::Snorm8 => wgpu::VertexFormat::Snorm8,
        GpuVertexFormat::Snorm8x2 => wgpu::VertexFormat::Snorm8x2,
        GpuVertexFormat::Snorm8x4 => wgpu::VertexFormat::Snorm8x4,
        GpuVertexFormat::Uint16 => wgpu::VertexFormat::Uint16,
        GpuVertexFormat::Uint16x2 => wgpu::VertexFormat::Uint16x2,
        GpuVertexFormat::Uint16x4 => wgpu::VertexFormat::Uint16x4,
        GpuVertexFormat::Sint16 => wgpu::VertexFormat::Sint16,
        GpuVertexFormat::Sint16x2 => wgpu::VertexFormat::Sint16x2,
        GpuVertexFormat::Sint16x4 => wgpu::VertexFormat::Sint16x4,
        GpuVertexFormat::Unorm16 => wgpu::VertexFormat::Unorm16,
        GpuVertexFormat::Unorm16x2 => wgpu::VertexFormat::Unorm16x2,
        GpuVertexFormat::Unorm16x4 => wgpu::VertexFormat::Unorm16x4,
        GpuVertexFormat::Snorm16 => wgpu::VertexFormat::Snorm16,
        GpuVertexFormat::Snorm16x2 => wgpu::VertexFormat::Snorm16x2,
        GpuVertexFormat::Snorm16x4 => wgpu::VertexFormat::Snorm16x4,
        GpuVertexFormat::Float16 => wgpu::VertexFormat::Float16,
        GpuVertexFormat::Float16x2 => wgpu::VertexFormat::Float16x2,
        GpuVertexFormat::Float16x4 => wgpu::VertexFormat::Float16x4,
        GpuVertexFormat::Float32 => wgpu::VertexFormat::Float32,
        GpuVertexFormat::Float32x2 => wgpu::VertexFormat::Float32x2,
        GpuVertexFormat::Float32x3 => wgpu::VertexFormat::Float32x3,
        GpuVertexFormat::Float32x4 => wgpu::VertexFormat::Float32x4,
        GpuVertexFormat::Uint32 => wgpu::VertexFormat::Uint32,
        GpuVertexFormat::Uint32x2 => wgpu::VertexFormat::Uint32x2,
        GpuVertexFormat::Uint32x3 => wgpu::VertexFormat::Uint32x3,
        GpuVertexFormat::Uint32x4 => wgpu::VertexFormat::Uint32x4,
        GpuVertexFormat::Sint32 => wgpu::VertexFormat::Sint32,
        GpuVertexFormat::Sint32x2 => wgpu::VertexFormat::Sint32x2,
        GpuVertexFormat::Sint32x3 => wgpu::VertexFormat::Sint32x3,
        GpuVertexFormat::Sint32x4 => wgpu::VertexFormat::Sint32x4,
        GpuVertexFormat::Unorm10_10_10_2 => wgpu::VertexFormat::Unorm10_10_10_2,
        GpuVertexFormat::Unorm8x4Bgra => wgpu::VertexFormat::Unorm8x4Bgra,
    }
}

fn gpu_vertex_format_size(format: GpuVertexFormat) -> u64 {
    wgpu_vertex_format(format).size()
}

fn wgpu_primitive_state(state: GpuPrimitiveState) -> wgpu::PrimitiveState {
    wgpu::PrimitiveState {
        topology: wgpu_primitive_topology(state.topology),
        strip_index_format: state.strip_index_format.map(wgpu_index_format),
        front_face: wgpu_front_face(state.front_face),
        cull_mode: state.cull_mode.map(wgpu_face),
        unclipped_depth: false,
        polygon_mode: wgpu::PolygonMode::Fill,
        conservative: false,
    }
}

fn wgpu_primitive_topology(topology: GpuPrimitiveTopology) -> wgpu::PrimitiveTopology {
    match topology {
        GpuPrimitiveTopology::PointList => wgpu::PrimitiveTopology::PointList,
        GpuPrimitiveTopology::LineList => wgpu::PrimitiveTopology::LineList,
        GpuPrimitiveTopology::LineStrip => wgpu::PrimitiveTopology::LineStrip,
        GpuPrimitiveTopology::TriangleList => wgpu::PrimitiveTopology::TriangleList,
        GpuPrimitiveTopology::TriangleStrip => wgpu::PrimitiveTopology::TriangleStrip,
    }
}

fn wgpu_index_format(format: GpuIndexFormat) -> wgpu::IndexFormat {
    match format {
        GpuIndexFormat::Uint16 => wgpu::IndexFormat::Uint16,
        GpuIndexFormat::Uint32 => wgpu::IndexFormat::Uint32,
    }
}

fn gpu_index_format_size(format: GpuIndexFormat) -> u64 {
    match format {
        GpuIndexFormat::Uint16 => 2,
        GpuIndexFormat::Uint32 => 4,
    }
}

fn wgpu_front_face(face: GpuFrontFace) -> wgpu::FrontFace {
    match face {
        GpuFrontFace::Ccw => wgpu::FrontFace::Ccw,
        GpuFrontFace::Cw => wgpu::FrontFace::Cw,
    }
}

fn wgpu_face(face: GpuFace) -> wgpu::Face {
    match face {
        GpuFace::Front => wgpu::Face::Front,
        GpuFace::Back => wgpu::Face::Back,
    }
}

fn wgpu_color_target_state(target: GpuColorTargetState) -> Result<wgpu::ColorTargetState, String> {
    validate_color_target_state(target)?;
    Ok(wgpu::ColorTargetState {
        format: wgpu_texture_format(target.format),
        blend: target.blend.map(wgpu_blend_state),
        write_mask: wgpu_color_write_mask(target.write_mask)?,
    })
}

fn wgpu_blend_state(state: GpuBlendState) -> wgpu::BlendState {
    match state {
        GpuBlendState::Replace => wgpu::BlendState::REPLACE,
        GpuBlendState::AlphaBlending => wgpu::BlendState::ALPHA_BLENDING,
        GpuBlendState::PremultipliedAlphaBlending => wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
    }
}

fn wgpu_color_write_mask(mask: GpuColorWriteMask) -> Result<wgpu::ColorWrites, String> {
    validate_color_write_mask(mask)?;
    let mut out = wgpu::ColorWrites::empty();
    if mask.contains(GpuColorWriteMask::RED) {
        out |= wgpu::ColorWrites::RED;
    }
    if mask.contains(GpuColorWriteMask::GREEN) {
        out |= wgpu::ColorWrites::GREEN;
    }
    if mask.contains(GpuColorWriteMask::BLUE) {
        out |= wgpu::ColorWrites::BLUE;
    }
    if mask.contains(GpuColorWriteMask::ALPHA) {
        out |= wgpu::ColorWrites::ALPHA;
    }
    Ok(out)
}

fn wgpu_depth_stencil_state(state: GpuDepthStencilState) -> wgpu::DepthStencilState {
    wgpu::DepthStencilState {
        format: wgpu_texture_format(state.format),
        depth_write_enabled: Some(state.depth_write_enabled),
        depth_compare: Some(wgpu_compare_function(state.depth_compare)),
        stencil: wgpu::StencilState::default(),
        bias: wgpu::DepthBiasState::default(),
    }
}

fn wgpu_multisample_state(state: GpuMultisampleState) -> wgpu::MultisampleState {
    wgpu::MultisampleState {
        count: state.count,
        mask: state.mask,
        alpha_to_coverage_enabled: state.alpha_to_coverage_enabled,
    }
}

fn wgpu_color_operations(operations: GpuColorOperations) -> wgpu::Operations<wgpu::Color> {
    wgpu::Operations {
        load: wgpu_color_load_op(operations.load),
        store: wgpu_store_op(operations.store),
    }
}

fn wgpu_depth_operations(operations: GpuDepthOperations) -> wgpu::Operations<f32> {
    wgpu::Operations {
        load: wgpu_depth_load_op(operations.load),
        store: wgpu_store_op(operations.store),
    }
}

fn wgpu_color_load_op(load: GpuColorLoadOp) -> wgpu::LoadOp<wgpu::Color> {
    match load {
        GpuColorLoadOp::Load => wgpu::LoadOp::Load,
        GpuColorLoadOp::Clear(color) => wgpu::LoadOp::Clear(wgpu_color(color)),
    }
}

fn wgpu_depth_load_op(load: GpuDepthLoadOp) -> wgpu::LoadOp<f32> {
    match load {
        GpuDepthLoadOp::Load => wgpu::LoadOp::Load,
        GpuDepthLoadOp::Clear(value) => wgpu::LoadOp::Clear(value),
    }
}

fn wgpu_store_op(store: GpuStoreOp) -> wgpu::StoreOp {
    match store {
        GpuStoreOp::Store => wgpu::StoreOp::Store,
        GpuStoreOp::Discard => wgpu::StoreOp::Discard,
    }
}

fn wgpu_color(color: GpuColor) -> wgpu::Color {
    wgpu::Color {
        r: color.r,
        g: color.g,
        b: color.b,
        a: color.a,
    }
}

fn wgpu_shader_stages(stages: GpuShaderStages) -> Result<wgpu::ShaderStages, String> {
    let known = GpuShaderStages::VERTEX
        .union(GpuShaderStages::FRAGMENT)
        .union(GpuShaderStages::COMPUTE);
    if stages.bits & !known.bits != 0 {
        return Err(format!("unknown GPU shader stage bits: {}", stages.bits));
    }
    let mut out = wgpu::ShaderStages::empty();
    if stages.bits & GpuShaderStages::VERTEX.bits != 0 {
        out |= wgpu::ShaderStages::VERTEX;
    }
    if stages.bits & GpuShaderStages::FRAGMENT.bits != 0 {
        out |= wgpu::ShaderStages::FRAGMENT;
    }
    if stages.bits & GpuShaderStages::COMPUTE.bits != 0 {
        out |= wgpu::ShaderStages::COMPUTE;
    }
    Ok(out)
}

fn wgpu_bind_group_layout_entry(
    entry: patinae_scene::GpuBindGroupLayoutEntry,
) -> Result<wgpu::BindGroupLayoutEntry, String> {
    Ok(wgpu::BindGroupLayoutEntry {
        binding: entry.binding,
        visibility: wgpu_shader_stages(entry.visibility)?,
        ty: wgpu_binding_type(entry.ty)?,
        count: None,
    })
}

fn wgpu_binding_type(ty: GpuBindingType) -> Result<wgpu::BindingType, String> {
    match ty {
        GpuBindingType::Buffer {
            ty,
            has_dynamic_offset,
            min_binding_size,
        } => Ok(wgpu::BindingType::Buffer {
            ty: wgpu_buffer_binding_type(ty),
            has_dynamic_offset,
            min_binding_size: min_binding_size.and_then(wgpu::BufferSize::new),
        }),
        GpuBindingType::Sampler(ty) => {
            Ok(wgpu::BindingType::Sampler(wgpu_sampler_binding_type(ty)))
        }
        GpuBindingType::Texture {
            sample_type,
            view_dimension,
            multisampled,
        } => Ok(wgpu::BindingType::Texture {
            sample_type: wgpu_texture_sample_type(sample_type),
            view_dimension: wgpu_texture_view_dimension(view_dimension),
            multisampled,
        }),
        GpuBindingType::StorageTexture {
            access,
            format,
            view_dimension,
        } => {
            validate_storage_texture_format(format, access)?;
            Ok(wgpu::BindingType::StorageTexture {
                access: wgpu_storage_texture_access(access),
                format: wgpu_texture_format(format),
                view_dimension: wgpu_texture_view_dimension(view_dimension),
            })
        }
    }
}

fn wgpu_buffer_binding_type(ty: GpuBufferBindingType) -> wgpu::BufferBindingType {
    match ty {
        GpuBufferBindingType::Uniform => wgpu::BufferBindingType::Uniform,
        GpuBufferBindingType::StorageReadOnly => {
            wgpu::BufferBindingType::Storage { read_only: true }
        }
        GpuBufferBindingType::StorageReadWrite => {
            wgpu::BufferBindingType::Storage { read_only: false }
        }
    }
}

fn wgpu_sampler_binding_type(ty: GpuSamplerBindingType) -> wgpu::SamplerBindingType {
    match ty {
        GpuSamplerBindingType::Filtering => wgpu::SamplerBindingType::Filtering,
        GpuSamplerBindingType::NonFiltering => wgpu::SamplerBindingType::NonFiltering,
        GpuSamplerBindingType::Comparison => wgpu::SamplerBindingType::Comparison,
    }
}

fn wgpu_texture_sample_type(ty: GpuTextureSampleType) -> wgpu::TextureSampleType {
    match ty {
        GpuTextureSampleType::Float { filterable } => wgpu::TextureSampleType::Float { filterable },
        GpuTextureSampleType::Depth => wgpu::TextureSampleType::Depth,
        GpuTextureSampleType::Sint => wgpu::TextureSampleType::Sint,
        GpuTextureSampleType::Uint => wgpu::TextureSampleType::Uint,
    }
}

fn wgpu_storage_texture_access(access: GpuStorageTextureAccess) -> wgpu::StorageTextureAccess {
    match access {
        GpuStorageTextureAccess::WriteOnly => wgpu::StorageTextureAccess::WriteOnly,
        GpuStorageTextureAccess::ReadOnly => wgpu::StorageTextureAccess::ReadOnly,
        GpuStorageTextureAccess::ReadWrite => wgpu::StorageTextureAccess::ReadWrite,
        GpuStorageTextureAccess::Atomic => wgpu::StorageTextureAccess::Atomic,
    }
}

fn validate_storage_texture_format(
    format: GpuTextureFormat,
    access: GpuStorageTextureAccess,
) -> Result<(), String> {
    if matches!(access, GpuStorageTextureAccess::Atomic)
        && !matches!(
            format,
            GpuTextureFormat::R32Uint | GpuTextureFormat::R32Sint
        )
    {
        return Err("atomic storage textures require R32Uint or R32Sint format".to_string());
    }
    if !matches!(
        format,
        GpuTextureFormat::R32Uint
            | GpuTextureFormat::R32Sint
            | GpuTextureFormat::R32Float
            | GpuTextureFormat::Rg32Float
            | GpuTextureFormat::Rgba8Unorm
            | GpuTextureFormat::Rgba8Uint
            | GpuTextureFormat::Rgba8Sint
            | GpuTextureFormat::Rgba16Float
            | GpuTextureFormat::Rgba32Float
    ) {
        return Err(format!(
            "texture format {:?} is not supported for storage texture bindings",
            format
        ));
    }
    Ok(())
}

fn wgpu_texel_copy_buffer_layout(layout: GpuTexelCopyBufferLayout) -> wgpu::TexelCopyBufferLayout {
    wgpu::TexelCopyBufferLayout {
        offset: layout.offset,
        bytes_per_row: layout.bytes_per_row,
        rows_per_image: layout.rows_per_image,
    }
}

fn wgpu_texel_copy_texture_info<'a>(
    texture: &'a HostGpuTexture,
    info: GpuTexelCopyTextureInfo,
) -> wgpu::TexelCopyTextureInfo<'a> {
    wgpu::TexelCopyTextureInfo {
        texture: &texture.texture,
        mip_level: info.mip_level,
        origin: wgpu_origin3d(info.origin),
        aspect: wgpu_texture_aspect(info.aspect),
    }
}

fn wgpu_bind_group_entry<'a>(
    registry: &'a GpuHandleRegistry,
    entry: GpuBindGroupEntry,
    expected: GpuBindingType,
) -> Result<wgpu::BindGroupEntry<'a>, String> {
    match (entry.resource, expected) {
        (GpuBindingResource::Buffer(binding), expected) => {
            if !matches!(expected, GpuBindingType::Buffer { .. }) {
                return Err(format!(
                    "GPU bind group entry {} expected {:?}, got buffer",
                    entry.binding, expected
                ));
            }
            let buffer = registry.buffer(binding.buffer)?;
            let size = binding.size.and_then(wgpu::BufferSize::new);
            let len = binding
                .size
                .unwrap_or(buffer.size.saturating_sub(binding.offset));
            validate_buffer_range(buffer.size, binding.offset, len)?;
            Ok(wgpu::BindGroupEntry {
                binding: entry.binding,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buffer.buffer,
                    offset: binding.offset,
                    size,
                }),
            })
        }
        (GpuBindingResource::TextureView(handle), expected) => {
            let view = registry.texture_view(handle)?;
            match expected {
                GpuBindingType::Texture { .. } => {
                    validate_texture_view_usage(
                        view,
                        GpuTextureUsage::TEXTURE_BINDING,
                        entry.binding,
                    )?;
                }
                GpuBindingType::StorageTexture { format, .. } => {
                    validate_texture_view_usage(
                        view,
                        GpuTextureUsage::STORAGE_BINDING,
                        entry.binding,
                    )?;
                    if view.format != format {
                        return Err(format!(
                            "GPU bind group entry {} storage texture format {:?} does not match view format {:?}",
                            entry.binding, format, view.format
                        ));
                    }
                }
                _ => {
                    return Err(format!(
                        "GPU bind group entry {} expected {:?}, got texture view",
                        entry.binding, expected
                    ));
                }
            }
            Ok(wgpu::BindGroupEntry {
                binding: entry.binding,
                resource: wgpu::BindingResource::TextureView(&view.view),
            })
        }
        (GpuBindingResource::Sampler(handle), expected) => {
            if !matches!(expected, GpuBindingType::Sampler(_)) {
                return Err(format!(
                    "GPU bind group entry {} expected {:?}, got sampler",
                    entry.binding, expected
                ));
            }
            Ok(wgpu::BindGroupEntry {
                binding: entry.binding,
                resource: wgpu::BindingResource::Sampler(registry.sampler(handle)?),
            })
        }
    }
}

fn validate_texture_view_usage(
    view: &HostGpuTextureView,
    required: GpuTextureUsage,
    binding: u32,
) -> Result<(), String> {
    validate_texture_view_usage_bits(view, required, &format!("bind group entry {binding}"))
}

fn validate_texture_view_usage_bits(
    view: &HostGpuTextureView,
    required: GpuTextureUsage,
    context: &str,
) -> Result<(), String> {
    if !view.usage.contains(required) {
        return Err(format!(
            "GPU {context} requires texture view usage bits {}",
            required.bits
        ));
    }
    Ok(())
}

struct RuntimePayloadSpool {
    path: PathBuf,
}

impl Drop for RuntimePayloadSpool {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn command_runtime_input_from_context<V: ViewerLike + ?Sized>(
    ctx: &mut CommandContext<'_, '_, V>,
    args: &ParsedCommand,
    runtime_requirements: CommandRuntimeRequirements,
) -> Result<CommandRuntimeInput, String> {
    let mut spools = Vec::new();
    let input = build_command_input_from_context(
        ctx,
        args,
        runtime_requirements,
        DisplayedGeometryInputMode::Spool(&mut spools),
    )?;
    Ok(CommandRuntimeInput {
        input,
        _spools: spools,
    })
}

#[cfg(test)]
pub(crate) fn command_input_from_context<V: ViewerLike + ?Sized>(
    ctx: &mut CommandContext<'_, '_, V>,
    args: &ParsedCommand,
    runtime_requirements: CommandRuntimeRequirements,
) -> Result<WireCommandInput, String> {
    build_command_input_from_context(
        ctx,
        args,
        runtime_requirements,
        DisplayedGeometryInputMode::Inline,
    )
}

enum DisplayedGeometryInputMode<'a> {
    #[cfg(test)]
    Inline,
    Spool(&'a mut Vec<RuntimePayloadSpool>),
}

fn build_command_input_from_context<V: ViewerLike + ?Sized>(
    ctx: &mut CommandContext<'_, '_, V>,
    args: &ParsedCommand,
    runtime_requirements: CommandRuntimeRequirements,
    geometry_mode: DisplayedGeometryInputMode<'_>,
) -> Result<WireCommandInput, String> {
    let (viewport_width, viewport_height) = ctx.viewer.viewport_size();
    let (displayed_geometry, displayed_geometry_spool) =
        command_displayed_geometry_input(ctx, runtime_requirements, geometry_mode)?;
    Ok(WireCommandInput {
        wire_version: RUNTIME_WIRE_VERSION,
        session: command_session_bytes(ctx.viewer, runtime_requirements)?,
        viewport_image: ctx.viewer.viewport_image_ref().cloned(),
        parsed: args.clone(),
        quiet: ctx.quiet,
        viewport_width,
        viewport_height,
        dynamic_settings: dynamic_settings_to_wire(ctx.dynamic_settings()),
        displayed_geometry,
        displayed_geometry_spool,
    })
}

fn command_displayed_geometry_input<V: ViewerLike + ?Sized>(
    ctx: &mut CommandContext<'_, '_, V>,
    runtime_requirements: CommandRuntimeRequirements,
    geometry_mode: DisplayedGeometryInputMode<'_>,
) -> Result<
    (
        Option<wire::WireDisplayedGeometry>,
        Option<wire::WireDisplayedGeometrySpool>,
    ),
    String,
> {
    if !runtime_requirements.contains(CommandRuntimeRequirements::DISPLAYED_GEOMETRY) {
        return Ok((None, None));
    }

    let displayed = ctx
        .viewer
        .export_displayed_geometry(&patinae_render::GeometryExportOptions::default())?;
    match geometry_mode {
        #[cfg(test)]
        DisplayedGeometryInputMode::Inline => {
            Ok((Some(wire::displayed_geometry_to_wire(&displayed)), None))
        }
        DisplayedGeometryInputMode::Spool(spools) => {
            let spool = write_displayed_geometry_spool(&displayed)?;
            let descriptor = wire::WireDisplayedGeometrySpool {
                path: spool.path.to_string_lossy().into_owned(),
                chunk_count: spool.chunk_count,
            };
            spools.push(RuntimePayloadSpool { path: spool.path });
            Ok((None, Some(descriptor)))
        }
    }
}

struct DisplayedGeometrySpool {
    path: PathBuf,
    chunk_count: usize,
}

fn write_displayed_geometry_spool(
    displayed: &patinae_render::DisplayedGeometry,
) -> Result<DisplayedGeometrySpool, String> {
    let (path, mut file) = create_displayed_geometry_spool_file()?;
    let cleanup = RuntimePayloadSpool { path: path.clone() };
    file.write_all(wire::DISPLAYED_GEOMETRY_SPOOL_MAGIC)
        .map_err(|error| format!("write displayed geometry spool header: {error}"))?;

    let mut chunk_count = 0usize;
    for object in &displayed.objects {
        write_displayed_object_spool_chunks(object, &mut file, &mut chunk_count)?;
    }
    file.flush()
        .map_err(|error| format!("flush displayed geometry spool: {error}"))?;

    std::mem::forget(cleanup);
    Ok(DisplayedGeometrySpool { path, chunk_count })
}

fn create_displayed_geometry_spool_file() -> Result<(PathBuf, std::fs::File), String> {
    for _ in 0..32 {
        let path = next_displayed_geometry_spool_path();
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!("create displayed geometry spool: {error}"));
            }
        }
    }
    Err("create displayed geometry spool: exhausted unique path attempts".to_string())
}

fn next_displayed_geometry_spool_path() -> PathBuf {
    let id = DISPLAYED_GEOMETRY_SPOOL_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "patinae-displayed-geometry-{}-{id}.mpack",
        std::process::id()
    ))
}

fn write_displayed_object_spool_chunks(
    object: &patinae_render::DisplayedObjectGeometry,
    file: &mut std::fs::File,
    chunk_count: &mut usize,
) -> Result<(), String> {
    let object_id = object.object_id.0;
    let mut primitives = Vec::new();
    for primitive in &object.primitives {
        match primitive {
            patinae_render::DisplayedPrimitive::Mesh { rep, mesh } => {
                write_displayed_spool_primitive_chunk(
                    object_id,
                    &mut primitives,
                    file,
                    chunk_count,
                )?;
                for vertices in mesh.vertices.chunks(DISPLAYED_GEOMETRY_SPOOL_MESH_VERTICES) {
                    let chunk_primitive = patinae_render::DisplayedPrimitive::Mesh {
                        rep: *rep,
                        mesh: patinae_render::DisplayedMesh {
                            vertices: vertices.to_vec(),
                        },
                    };
                    primitives.push(wire::displayed_primitive_to_wire(&chunk_primitive));
                    write_displayed_spool_primitive_chunk(
                        object_id,
                        &mut primitives,
                        file,
                        chunk_count,
                    )?;
                }
            }
            _ => {
                primitives.push(wire::displayed_primitive_to_wire(primitive));
                if primitives.len() >= DISPLAYED_GEOMETRY_SPOOL_PRIMITIVES {
                    write_displayed_spool_primitive_chunk(
                        object_id,
                        &mut primitives,
                        file,
                        chunk_count,
                    )?;
                }
            }
        }
    }

    write_displayed_spool_primitive_chunk(object_id, &mut primitives, file, chunk_count)
}

fn write_displayed_spool_primitive_chunk(
    object_id: u32,
    primitives: &mut Vec<wire::WireDisplayedPrimitive>,
    file: &mut std::fs::File,
    chunk_count: &mut usize,
) -> Result<(), String> {
    if primitives.is_empty() {
        return Ok(());
    }

    let chunk = wire::WireDisplayedGeometryChunk {
        objects: vec![wire::WireDisplayedObjectGeometry {
            object_id,
            primitives: std::mem::take(primitives),
        }],
    };
    write_displayed_spool_chunk(file, &chunk)?;
    *chunk_count += 1;
    Ok(())
}

fn write_displayed_spool_chunk(
    file: &mut std::fs::File,
    chunk: &wire::WireDisplayedGeometryChunk,
) -> Result<(), String> {
    let bytes = wire::encode(chunk)?;
    ensure_runtime_payload_within_limit(
        "command execute",
        bytes.len(),
        "displayed geometry chunk",
    )?;
    file.write_all(&(bytes.len() as u64).to_le_bytes())
        .map_err(|error| format!("write displayed geometry chunk length: {error}"))?;
    file.write_all(&bytes)
        .map_err(|error| format!("write displayed geometry chunk: {error}"))
}

pub(crate) fn apply_command_output<V: ViewerLike + ?Sized>(
    ctx: &mut CommandContext<'_, '_, V>,
    output: WireCommandOutput,
    runtime_requirements: CommandRuntimeRequirements,
) -> CmdResult {
    validate_runtime_wire_version(output.wire_version).map_err(CmdError::execution)?;
    if runtime_requirements.contains(CommandRuntimeRequirements::FULL_SESSION) {
        let session = wire::decode_session(&output.session).map_err(CmdError::execution)?;
        ctx.viewer.replace_session(session);
    }
    if output.viewport_image_changed {
        ctx.viewer.set_viewport_image(output.viewport_image);
    }
    for message in output.output {
        apply_output_message(ctx, message);
    }
    for action in output.actions {
        apply_command_action(ctx, action);
    }
    output.result.map_err(CmdError::execution)
}

fn apply_output_message<V: ViewerLike + ?Sized>(
    ctx: &mut CommandContext<'_, '_, V>,
    message: patinae_cmd::OutputMessage,
) {
    match message.kind {
        MessageKind::Info => ctx.print(&message.text),
        MessageKind::Warning => ctx.print_warning(&message.text),
        MessageKind::Error => ctx.print_error(&message.text),
    }
}

fn apply_command_action<V: ViewerLike + ?Sized>(
    ctx: &mut CommandContext<'_, '_, V>,
    action: CommandAction,
) {
    match action {
        CommandAction::ShowPanel(panel) => ctx.show_panel(panel),
        CommandAction::HidePanel(panel) => ctx.hide_panel(panel),
        CommandAction::ClearOutput => ctx.clear_output(),
        CommandAction::Quit => ctx.quit(),
        CommandAction::RecordRecentFile { path, command } => ctx.record_recent_file(path, command),
    }
}

pub(crate) fn shared_input_from_context(
    ctx: &SharedContext<'_>,
    runtime_requirements: PanelRuntimeRequirements,
) -> Result<WireSharedInput, String> {
    let session = session_from_shared_context(ctx, runtime_requirements)?;
    Ok(WireSharedInput {
        wire_version: RUNTIME_WIRE_VERSION,
        session: wire::encode_session(&session)?,
        scene_generation: ctx.scene_generation,
        viewport_image: ctx.viewport_image.cloned(),
        command_names: ctx.command_names.to_vec(),
        setting_names: ctx
            .setting_names
            .iter()
            .map(|name| (*name).to_string())
            .collect(),
        dynamic_settings: dynamic_settings_to_wire(ctx.dynamic_settings),
    })
}

fn session_from_shared_context(
    ctx: &SharedContext<'_>,
    runtime_requirements: PanelRuntimeRequirements,
) -> Result<Session, String> {
    if !runtime_requirements.contains(PanelRuntimeRequirements::FULL_SESSION) {
        return lightweight_session_from_shared_context(ctx);
    }

    let mut session = Session::new();
    session.registry = patinae_scene::ObjectRegistry::from_snapshot(ctx.registry.to_snapshot());
    session.camera = ctx.camera.clone();
    session.selections = ctx.selections.clone();
    session.recent_atoms = ctx.recent_atoms.clone();
    session.named_palette = clone_wire(ctx.named_palette)?;
    session.movie = clone_wire(ctx.movie)?;
    session.settings = ctx.settings.clone();
    session.clear_color = ctx.clear_color;
    session.viewport_image = ctx.viewport_image.cloned();
    ensure_full_session_payload_within_limit("panel snapshot", &session)?;
    Ok(session)
}

fn command_session_bytes<V: ViewerLike + ?Sized>(
    viewer: &V,
    runtime_requirements: CommandRuntimeRequirements,
) -> Result<Vec<u8>, String> {
    if runtime_requirements.contains(CommandRuntimeRequirements::FULL_SESSION) {
        let bytes = wire::encode_session(viewer.session())?;
        ensure_runtime_payload_within_limit("command execute", bytes.len(), "full scene state")?;
        return Ok(bytes);
    }

    let session = lightweight_session_from_viewer(viewer)?;
    wire::encode_session(&session)
}

fn lightweight_session_from_viewer<V: ViewerLike + ?Sized>(viewer: &V) -> Result<Session, String> {
    let source = viewer.session();
    let mut session = Session::new();
    session.camera = viewer.camera().clone();
    session.settings = viewer.settings().clone();
    session.named_palette = clone_wire(&source.named_palette)?;
    session.movie = clone_wire(&source.movie)?;
    session.clear_color = source.clear_color;
    session.clear_color_set = source.clear_color_set;
    session.viewport_image = viewer.viewport_image_ref().cloned();
    Ok(session)
}

fn lightweight_session_from_shared_context(ctx: &SharedContext<'_>) -> Result<Session, String> {
    let mut session = Session::new();
    session.camera = ctx.camera.clone();
    session.settings = ctx.settings.clone();
    session.named_palette = clone_wire(ctx.named_palette)?;
    session.movie = clone_wire(ctx.movie)?;
    session.clear_color = ctx.clear_color;
    session.viewport_image = ctx.viewport_image.cloned();
    Ok(session)
}

fn ensure_full_session_payload_within_limit(phase: &str, session: &Session) -> Result<(), String> {
    let bytes = wire::encode_session(session)?;
    ensure_runtime_payload_within_limit(phase, bytes.len(), "full scene state")
}

pub(crate) fn ensure_runtime_payload_within_limit(
    phase: &str,
    len: usize,
    requirement: &str,
) -> Result<(), String> {
    if len <= wire::MAX_WIRE_PAYLOAD_LEN {
        return Ok(());
    }
    let limit_mib = wire::MAX_WIRE_PAYLOAD_LEN / (1024 * 1024);
    Err(format!(
        "Plugin {phase} input requires {requirement}, but serialized payload is {len} bytes and exceeds the {limit_mib} MiB plugin ABI limit"
    ))
}

fn clone_wire<T>(value: &T) -> Result<T, String>
where
    T: Serialize + DeserializeOwned,
{
    let bytes = wire::encode(value)?;
    wire::decode(&bytes)
}

fn dynamic_settings_to_wire(
    dynamic_settings: Option<&DynamicSettingRegistry>,
) -> Vec<wire::WireDynamicSetting> {
    let Some(dynamic_settings) = dynamic_settings else {
        return Vec::new();
    };
    dynamic_settings
        .names()
        .iter()
        .filter_map(|name| {
            let entry = dynamic_settings.lookup(name)?;
            let value = entry
                .store
                .read()
                .ok()
                .and_then(|store| store.get(name).cloned());
            Some(wire::WireDynamicSetting {
                descriptor: (&entry.descriptor).into(),
                value,
            })
        })
        .collect()
}

fn poll_input_from_context(ctx: &PollContext<'_>) -> Result<WirePollInput, String> {
    Ok(WirePollInput {
        shared: ctx.poll_shared.clone(),
        command_results: ctx
            .command_results
            .iter()
            .map(|result| wire::WireCommandResult {
                id: result.id,
                result: result.result.clone(),
            })
            .collect(),
        host_query_results: ctx.host_query_results.to_vec(),
        dynamic_invocations: ctx.dynamic_invocations.to_vec(),
        plugin_dirs: ctx
            .plugin_dirs
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect(),
    })
}

fn apply_poll_output(ctx: &mut PollContext<'_>, output: WirePollOutput) -> Result<(), String> {
    validate_runtime_wire_version(output.wire_version)?;
    send_bus_messages(output.messages, ctx.bus);
    for request in output.command_exec {
        ctx.execute_command(request.id, &request.command, request.silent);
    }
    for registration in output.dynamic_registrations {
        ctx.register_dynamic_command(
            registration.name,
            registration.description,
            registration.usage,
            registration.arguments,
        );
    }
    for name in output.dynamic_unregistrations {
        ctx.unregister_dynamic_command(&name);
    }
    for notification in output.notifications {
        ctx.set_notification(notification);
    }
    for registration in output.hotkey_registrations {
        ctx.register_hotkey(
            registration.key,
            plugin_key_action_from_wire(registration.action),
        );
    }
    for key in output.hotkey_unregistrations {
        ctx.unregister_hotkey(key);
    }
    for query in output.host_queries {
        ctx.query_host(query);
    }
    for action in output.viewer_actions {
        apply_viewer_action(ctx, action);
    }
    Ok(())
}

fn apply_viewer_action(ctx: &mut PollContext<'_>, action: WireViewerAction) {
    match action {
        WireViewerAction::ApplyAtomPropertyChanges(changes) => {
            ctx.queue_viewer_mutation(move |viewer| {
                apply_atom_property_change_batch(viewer, &changes);
            });
        }
        WireViewerAction::SetViewportImage(image) => {
            ctx.queue_viewer_mutation(move |viewer| {
                viewer.set_viewport_image(Some(image));
            });
        }
        WireViewerAction::ClearViewportImage => {
            ctx.queue_viewer_mutation(|viewer| {
                viewer.set_viewport_image(None);
            });
        }
        WireViewerAction::RequestRedraw => {
            ctx.queue_viewer_mutation(|viewer| viewer.request_redraw());
        }
        WireViewerAction::RequestPanelUpdate => ctx.request_panel_update(),
    }
}

fn send_bus_messages(messages: Vec<AppMessage>, bus: &mut MessageBus) {
    for message in messages {
        bus.send(message);
    }
}

fn plugin_key_action_from_wire(action: WireHotkeyAction) -> PluginKeyAction {
    match action {
        WireHotkeyAction::Command(command) => PluginKeyAction::Command(command),
        WireHotkeyAction::DynamicCommand { name, args } => {
            PluginKeyAction::DynamicCommand { name, args }
        }
        WireHotkeyAction::Custom { topic, payload } => PluginKeyAction::Custom { topic, payload },
    }
}

fn destroy_command_handle(handle: PluginCommandHandle, vtable: AbiCommandVTable) {
    if let Some(destroy) = vtable.destroy {
        destroy_runtime_handle("command destroy", || {
            // SAFETY: The handle and destroy callback were validated during
            // registration, and the proxy keeps the library alive during drop.
            unsafe { destroy(handle) }
        });
    }
}

fn destroy_panel_handle(handle: PluginPanelHandle, vtable: AbiPanelVTable) {
    if let Some(destroy) = vtable.destroy {
        destroy_runtime_handle("panel destroy", || {
            // SAFETY: The handle and destroy callback were validated during
            // registration, and the proxy keeps the library alive during drop.
            unsafe { destroy(handle) }
        });
    }
}

fn destroy_message_handler_handle(
    handle: PluginMessageHandlerHandle,
    vtable: AbiMessageHandlerVTable,
) {
    if let Some(destroy) = vtable.destroy {
        destroy_runtime_handle("message handler destroy", || {
            // SAFETY: The handle and destroy callback were validated during
            // registration, and the proxy keeps the library alive during drop.
            unsafe { destroy(handle) }
        });
    }
}

fn destroy_script_handler_handle(
    handle: PluginScriptHandlerHandle,
    vtable: AbiScriptHandlerVTable,
) {
    if let Some(destroy) = vtable.destroy {
        destroy_runtime_handle("script handler destroy", || {
            // SAFETY: The handle and destroy callback were validated during
            // registration, and the proxy keeps the library alive during drop.
            unsafe { destroy(handle) }
        });
    }
}

fn destroy_format_handler_handle(
    handle: PluginFormatHandlerHandle,
    vtable: AbiFormatHandlerVTable,
) {
    if let Some(destroy) = vtable.destroy {
        destroy_runtime_handle("format handler destroy", || {
            // SAFETY: The handle and destroy callback were validated during
            // registration, and the proxy keeps the library alive during drop.
            unsafe { destroy(handle) }
        });
    }
}

fn destroy_hotkey_handle(handle: PluginHotkeyHandle, vtable: AbiHotkeyVTable) {
    if let Some(destroy) = vtable.destroy {
        destroy_runtime_handle("hotkey destroy", || {
            // SAFETY: The handle and destroy callback were validated during
            // registration, and the proxy keeps the library alive during drop.
            unsafe { destroy(handle) }
        });
    }
}

fn destroy_runtime_handle(phase: &str, destroy: impl FnOnce() -> AbiStatus) {
    match catch_unwind(AssertUnwindSafe(destroy)) {
        Ok(status) if status.is_ok() => {}
        Ok(status) => {
            if let Err(error) = status_to_result(phase, status) {
                log::warn!("{error}");
            }
        }
        Err(panic_info) => log::error!(
            "Plugin {phase} panicked: {}",
            panic_payload_to_string(&panic_info)
        ),
    }
}

fn leak_aliases(values: Vec<String>) -> &'static [&'static str] {
    if values.is_empty() {
        return &[];
    }
    let aliases: Vec<&'static str> = values
        .into_iter()
        .map(|value| {
            let value: &'static mut str = Box::leak(value.into_boxed_str());
            &*value
        })
        .collect();
    Box::leak(aliases.into_boxed_slice())
}

fn leak_arg_hints(values: Vec<ArgHint>) -> &'static [ArgHint] {
    if values.is_empty() {
        return &[];
    }
    Box::leak(values.into_boxed_slice())
}

fn arg_hint_from_abi(value: u8) -> ArgHint {
    match value {
        ARG_HINT_PATH => ArgHint::Path,
        ARG_HINT_SELECTION => ArgHint::Selection,
        ARG_HINT_OBJECT => ArgHint::Object,
        ARG_HINT_REPRESENTATION => ArgHint::Representation,
        ARG_HINT_COLOR => ArgHint::Color,
        ARG_HINT_SETTING => ArgHint::Setting,
        ARG_HINT_SETTING_VALUE => ArgHint::SettingValue,
        ARG_HINT_NAMED_SELECTION => ArgHint::NamedSelection,
        ARG_HINT_LABEL_PROPERTY => ArgHint::LabelProperty,
        ARG_HINT_COMMAND => ArgHint::Command,
        _ => ArgHint::None,
    }
}

fn panel_descriptor_from_abi(descriptor: &AbiPanelDescriptor) -> Result<PanelDescriptor, String> {
    let placement = match descriptor.placement {
        PANEL_PLACEMENT_RIGHT => PanelPlacement::Right,
        PANEL_PLACEMENT_BOTTOM => PanelPlacement::Bottom,
        other => return Err(format!("unknown panel placement code {other}")),
    };
    Ok(PanelDescriptor {
        id: copy_str(descriptor.id, "panel.id")?,
        title: copy_str(descriptor.title, "panel.title")?,
        icon: copy_str(descriptor.icon, "panel.icon")?,
        placement,
        default_visible: descriptor.default_visible != 0,
    })
}

fn setting_from_abi(
    descriptor: &AbiSettingDescriptor,
) -> Result<(DynamicSettingDescriptor, SharedSettingStore), String> {
    let name = copy_str(descriptor.name, "setting.name")?;
    let default = setting_value_from_abi(descriptor.default_value, "setting.default")?;
    let side_effects = copy_u8_slice(descriptor.side_effects, "setting.side_effects")?
        .into_iter()
        .map(side_effect_from_abi)
        .collect::<Result<Vec<_>, _>>()?;
    let value_hints = copy_hint_slice(descriptor.value_hints, "setting.value_hints")?;
    let setting = DynamicSettingDescriptor {
        name: name.clone(),
        setting_type: setting_type_from_abi(descriptor.setting_type)?,
        default: default.clone(),
        min: (descriptor.has_min != 0).then_some(descriptor.min),
        max: (descriptor.has_max != 0).then_some(descriptor.max),
        value_hints,
        side_effects,
        object_overridable: descriptor.object_overridable != 0,
    };
    let mut store = DynamicSettingStore::new();
    store.set(&name, default);
    Ok((setting, Arc::new(RwLock::new(store))))
}

fn setting_hint_from_abi(
    hint: &AbiSettingValueHint,
    field: &str,
) -> Result<(String, SettingValue), String> {
    Ok((
        copy_str(hint.name, &format!("{field}.name"))?,
        setting_value_from_abi(hint.value, &format!("{field}.value"))?,
    ))
}

fn setting_type_from_abi(value: u8) -> Result<SettingType, String> {
    match value {
        SETTING_TYPE_BLANK => Ok(SettingType::Blank),
        SETTING_TYPE_BOOL => Ok(SettingType::Bool),
        SETTING_TYPE_INT => Ok(SettingType::Int),
        SETTING_TYPE_FLOAT => Ok(SettingType::Float),
        SETTING_TYPE_FLOAT3 => Ok(SettingType::Float3),
        SETTING_TYPE_COLOR => Ok(SettingType::Color),
        SETTING_TYPE_STRING => Ok(SettingType::String),
        other => Err(format!("unknown setting type code {other}")),
    }
}

fn setting_value_from_abi(value: AbiSettingValue, field: &str) -> Result<SettingValue, String> {
    match value.tag {
        SETTING_VALUE_NONE => Ok(SettingValue::String(String::new())),
        SETTING_VALUE_BOOL => Ok(SettingValue::Bool(value.bool_value != 0)),
        SETTING_VALUE_INT => Ok(SettingValue::Int(value.int_value)),
        SETTING_VALUE_FLOAT => Ok(SettingValue::Float(value.float_values[0])),
        SETTING_VALUE_FLOAT3 => Ok(SettingValue::Float3(value.float_values)),
        SETTING_VALUE_COLOR => Ok(SettingValue::Color(value.int_value)),
        SETTING_VALUE_STRING => Ok(SettingValue::String(copy_str(
            value.string_value,
            &format!("{field}.string"),
        )?)),
        other => Err(format!("{field} has unknown setting value code {other}")),
    }
}

fn side_effect_from_abi(value: u8) -> Result<SideEffectCategory, String> {
    match value {
        SIDE_EFFECT_SCENE_INVALIDATE => Ok(SideEffectCategory::SceneInvalidate),
        SIDE_EFFECT_SCENE_CHANGED => Ok(SideEffectCategory::SceneChanged),
        SIDE_EFFECT_SHADER_RELOAD => Ok(SideEffectCategory::ShaderReload),
        SIDE_EFFECT_SHADER_COMPUTE_LIGHTING => Ok(SideEffectCategory::ShaderComputeLighting),
        SIDE_EFFECT_ORTHO_DIRTY => Ok(SideEffectCategory::OrthoDirty),
        SIDE_EFFECT_SEQ_CHANGED => Ok(SideEffectCategory::SeqChanged),
        SIDE_EFFECT_STEREO_UPDATE => Ok(SideEffectCategory::StereoUpdate),
        SIDE_EFFECT_REPRESENTATION_REBUILD => Ok(SideEffectCategory::RepresentationRebuild),
        SIDE_EFFECT_COLOR_REBUILD => Ok(SideEffectCategory::ColorRebuild),
        SIDE_EFFECT_SURFACE_TRANSPARENCY => Ok(SideEffectCategory::SurfaceTransparency),
        SIDE_EFFECT_FULL_REBUILD => Ok(SideEffectCategory::FullRebuild),
        SIDE_EFFECT_VIEWPORT_UPDATE => Ok(SideEffectCategory::ViewportUpdate),
        other => Err(format!("unknown side-effect code {other}")),
    }
}

#[cfg(test)]
mod loader_tests {
    use super::*;
    use patinae_mol::AtomIndex;
    use patinae_scene::GpuFragmentState;

    struct TraceFixtureViewer {
        session: Session,
        chunks: Vec<patinae_render::TraceGeometryChunk>,
    }

    impl ViewerLike for TraceFixtureViewer {
        fn objects(&self) -> &patinae_scene::ObjectRegistry {
            &self.session.registry
        }

        fn objects_mut(&mut self) -> &mut patinae_scene::ObjectRegistry {
            &mut self.session.registry
        }

        fn camera(&self) -> &patinae_scene::Camera {
            &self.session.camera
        }

        fn camera_mut(&mut self) -> &mut patinae_scene::Camera {
            &mut self.session.camera
        }

        fn settings(&self) -> &patinae_settings::Settings {
            &self.session.settings
        }

        fn settings_mut(&mut self) -> &mut patinae_settings::Settings {
            &mut self.session.settings
        }

        fn request_redraw(&mut self) {}

        fn session(&self) -> &Session {
            &self.session
        }

        fn session_mut(&mut self) -> &mut Session {
            &mut self.session
        }

        fn replace_session(&mut self, session: Session) {
            self.session = session;
        }

        fn movie(&self) -> &patinae_scene::Movie {
            &self.session.movie
        }

        fn movie_mut(&mut self) -> &mut patinae_scene::Movie {
            &mut self.session.movie
        }

        fn scenes(&self) -> &patinae_scene::SceneManager {
            &self.session.scenes
        }

        fn scenes_mut(&mut self) -> &mut patinae_scene::SceneManager {
            &mut self.session.scenes
        }

        fn scene_store(&mut self, key: &str, storemask: u32) {
            let mask = patinae_scene::SceneStoreMask::from_bits_truncate(storemask);
            self.session
                .scenes
                .store(key, mask, &self.session.camera, &self.session.registry);
        }

        fn scene_recall(&mut self, key: &str, animate: bool, duration: f32) -> Result<(), String> {
            self.session
                .scenes
                .recall(
                    key,
                    &mut self.session.camera,
                    &mut self.session.registry,
                    animate,
                    duration,
                )
                .map_err(|error| error.to_string())
        }

        fn views(&self) -> &patinae_scene::ViewManager {
            &self.session.views
        }

        fn views_mut(&mut self) -> &mut patinae_scene::ViewManager {
            &mut self.session.views
        }

        fn view_recall(&mut self, key: &str, animate: f32) -> Result<(), String> {
            self.session
                .views
                .recall(key, &mut self.session.camera, animate)
                .map_err(|error| error.to_string())
        }

        fn selections(&self) -> &patinae_scene::SelectionManager {
            &self.session.selections
        }

        fn selections_mut(&mut self) -> &mut patinae_scene::SelectionManager {
            &mut self.session.selections
        }

        fn named_palette(&self) -> &patinae_scene::NamedPalette {
            &self.session.named_palette
        }

        fn named_palette_mut(&mut self) -> &mut patinae_scene::NamedPalette {
            &mut self.session.named_palette
        }

        fn clear_color(&self) -> [f32; 3] {
            self.session.clear_color
        }

        fn set_clear_color(&mut self, color: [f32; 3]) {
            self.session.clear_color = color;
        }

        fn viewport_image_ref(&self) -> Option<&patinae_scene::ViewportImage> {
            self.session.viewport_image.as_ref()
        }

        fn set_viewport_image_internal(&mut self, image: Option<patinae_scene::ViewportImage>) {
            self.session.viewport_image = image;
        }

        fn for_each_trace_geometry_chunk(
            &mut self,
            _options: &patinae_render::GeometryExportOptions,
            visitor: &mut dyn FnMut(patinae_render::TraceGeometryChunk) -> Result<(), String>,
        ) -> Result<(), String> {
            for chunk in self.chunks.clone() {
                visitor(chunk)?;
            }
            Ok(())
        }
    }

    #[test]
    fn isolated_plugin_identity_changes_reconcile_recent_atoms() {
        let mut molecule = ObjectMolecule::new("obj");
        molecule.add_atom(
            patinae_mol::AtomBuilder::new()
                .name("CA")
                .element_symbol("C")
                .resn("GLY")
                .resv(1)
                .chain("A")
                .build(),
        );
        let mut session = Session::new();
        session
            .registry
            .add(patinae_scene::MoleculeObject::with_name(molecule, "obj"));
        let path = patinae_scene::canonical_atom_path_for_hit(
            &patinae_scene::PickHit {
                object_name: "obj".to_string(),
                object_type: patinae_scene::ObjectType::Molecule,
                atom_index: Some(AtomIndex(0)),
                position: Default::default(),
                distance: 0.0,
            },
            session.registry.get_molecule("obj").unwrap().molecule(),
        )
        .unwrap();
        session
            .recent_atoms
            .insert(path, patinae_settings::groups::RecentPickLimit::Unlimited);
        let mut viewer = TraceFixtureViewer {
            session,
            chunks: Vec::new(),
        };

        apply_atom_property_change_batch(
            &mut viewer,
            &[wire::WireAtomPropertyChange {
                object: "obj".to_string(),
                atom_index: 0,
                changes: vec![(
                    "chain".to_string(),
                    wire::WireAtomPropertyValue::Str("B".to_string()),
                )],
            }],
        );

        assert!(viewer.session.recent_atoms.is_empty());
    }

    fn trace_material() -> patinae_render::TraceMaterial {
        patinae_render::TraceMaterial {
            rgba: [0.2, 0.4, 0.8, 1.0],
            transparency: 0.0,
        }
    }

    fn trace_chunk(
        sphere_count: usize,
        cylinder_count: usize,
        triangle_count: usize,
    ) -> patinae_render::TraceGeometryChunk {
        let material = trace_material();
        patinae_render::TraceGeometryChunk {
            spheres: (0..sphere_count)
                .map(|i| patinae_render::TraceSphere {
                    center: [i as f32, 0.0, 0.0],
                    radius: 1.0,
                    material,
                })
                .collect(),
            cylinders: (0..cylinder_count)
                .map(|i| patinae_render::TraceCylinder {
                    start: [i as f32, 0.0, 0.0],
                    end: [i as f32, 1.0, 0.0],
                    radius: 0.2,
                    material_start: material,
                    material_end: material,
                })
                .collect(),
            triangles: (0..triangle_count)
                .map(|i| patinae_render::TraceTriangle {
                    positions: [
                        [i as f32, 0.0, 0.0],
                        [i as f32, 1.0, 0.0],
                        [i as f32, 0.0, 1.0],
                    ],
                    normals: [[0.0, 0.0, 1.0]; 3],
                    material,
                })
                .collect(),
            line_segments: Vec::new(),
            point_samples: Vec::new(),
        }
    }

    fn gpu_handle(id: u64, kind: GpuHandleKind) -> GpuHandle {
        GpuHandle {
            id,
            kind,
            generation: 1,
        }
    }

    fn render_pipeline_descriptor() -> GpuRenderPipelineDescriptor {
        let shader = gpu_handle(1, GpuHandleKind::ShaderModule);
        GpuRenderPipelineDescriptor {
            label: Some("test.render.pipeline".to_string()),
            layout: gpu_handle(2, GpuHandleKind::PipelineLayout),
            vertex: GpuVertexState {
                module: shader,
                entry_point: "vs_main".to_string(),
                buffers: vec![GpuVertexBufferLayout {
                    array_stride: 16,
                    step_mode: GpuVertexStepMode::Vertex,
                    attributes: vec![GpuVertexAttribute {
                        format: GpuVertexFormat::Float32x4,
                        offset: 0,
                        shader_location: 0,
                    }],
                }],
            },
            primitive: GpuPrimitiveState {
                topology: GpuPrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: GpuFrontFace::Ccw,
                cull_mode: Some(GpuFace::Back),
            },
            depth_stencil: Some(GpuDepthStencilState {
                format: GpuTextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: GpuCompareFunction::LessEqual,
            }),
            multisample: GpuMultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            fragment: Some(GpuFragmentState {
                module: shader,
                entry_point: "fs_main".to_string(),
                targets: vec![Some(GpuColorTargetState {
                    format: GpuTextureFormat::Rgba8Unorm,
                    blend: None,
                    write_mask: GpuColorWriteMask::ALL,
                })],
            }),
        }
    }

    #[test]
    fn displayed_geometry_spool_splits_and_reads_mesh_chunks() {
        let material = patinae_render::DisplayedMaterial::from_rgba([0.2, 0.4, 0.8, 1.0]);
        let vertex_count = DISPLAYED_GEOMETRY_SPOOL_MESH_VERTICES + 3;
        let vertices = (0..vertex_count)
            .map(|i| patinae_render::DisplayedMeshVertex {
                position: [i as f32, 0.0, 0.0],
                normal: [0.0, 0.0, 1.0],
                owner_atom_id: i as u32,
                material,
                flags: 0,
            })
            .collect::<Vec<_>>();
        let displayed = patinae_render::DisplayedGeometry {
            objects: vec![patinae_render::DisplayedObjectGeometry {
                object_id: patinae_render::ObjectId(7),
                primitives: vec![patinae_render::DisplayedPrimitive::Mesh {
                    rep: patinae_render::RepKind::Cartoon,
                    mesh: patinae_render::DisplayedMesh {
                        vertices: vertices.clone(),
                    },
                }],
            }],
        };

        let spool = write_displayed_geometry_spool(&displayed).unwrap();
        let _cleanup = RuntimePayloadSpool {
            path: spool.path.clone(),
        };
        assert!(spool.chunk_count >= 2);

        let descriptor = wire::WireDisplayedGeometrySpool {
            path: spool.path.to_string_lossy().into_owned(),
            chunk_count: spool.chunk_count,
        };
        let mut chunk_vertex_counts = Vec::new();
        wire::for_each_displayed_geometry_spool_chunk(&descriptor, |chunk| {
            for object in chunk.objects {
                for primitive in object.primitives {
                    if let patinae_render::DisplayedPrimitive::Mesh { mesh, .. } = primitive {
                        chunk_vertex_counts.push(mesh.vertices.len());
                    }
                }
            }
            Ok(())
        })
        .unwrap();

        assert_eq!(chunk_vertex_counts.iter().sum::<usize>(), vertices.len());
        assert!(chunk_vertex_counts
            .iter()
            .all(|&count| count <= DISPLAYED_GEOMETRY_SPOOL_MESH_VERTICES));
    }

    #[test]
    fn trace_geometry_chunk_stays_below_abi_limit() {
        let chunk = trace_chunk(0, 0, 65_536);
        let wire_chunk = wire::trace_geometry_chunk_to_wire(chunk);
        let encoded = wire::encode(&wire_chunk).unwrap();

        ensure_runtime_payload_within_limit(
            "test trace geometry stream",
            encoded.len(),
            "trace geometry chunk",
        )
        .unwrap();
    }

    #[test]
    fn trace_geometry_stream_reads_multiple_chunks() {
        let mut viewer = TraceFixtureViewer {
            session: Session::new(),
            chunks: vec![trace_chunk(1, 2, 3), trace_chunk(4, 5, 6)],
        };
        let mut state = HostCommandRuntimeState::new(
            &mut viewer,
            CommandRuntimeRequirements::TRACE_GEOMETRY_STREAM,
        );

        let opened =
            state.handle_request(WireCommandRuntimeRequest::OpenTraceGeometryStream { id: 1 });
        let stream_id = match opened.result.unwrap() {
            WireCommandRuntimeValue::TraceGeometryOpened(opened) => opened.stream_id,
            _ => panic!("unexpected open response"),
        };

        let mut counts = (0usize, 0usize, 0usize);
        for id in [2, 3] {
            let response =
                state.handle_request(WireCommandRuntimeRequest::ReadTraceGeometryStream {
                    id,
                    stream_id,
                });
            let chunk = match response.result.unwrap() {
                WireCommandRuntimeValue::TraceGeometryChunk(Some(chunk)) => chunk,
                WireCommandRuntimeValue::TraceGeometryChunkBytes(Some(bytes)) => {
                    wire::decode(&bytes).unwrap()
                }
                _ => panic!("unexpected read response"),
            };
            counts.0 += chunk.spheres.len();
            counts.1 += chunk.cylinders.len();
            counts.2 += chunk.triangles.len();
        }

        let end = state.handle_request(WireCommandRuntimeRequest::ReadTraceGeometryStream {
            id: 4,
            stream_id,
        });
        assert!(matches!(
            end.result.unwrap(),
            WireCommandRuntimeValue::TraceGeometryChunkBytes(None)
        ));
        let closed = state.handle_request(WireCommandRuntimeRequest::CloseTraceGeometryStream {
            id: 5,
            stream_id,
        });
        assert!(matches!(
            closed.result.unwrap(),
            WireCommandRuntimeValue::TraceGeometryClosed
        ));
        assert_eq!(counts, (5, 7, 9));
    }

    #[test]
    fn gpu_runtime_request_requires_declared_requirement() {
        let mut viewer = TraceFixtureViewer {
            session: Session::new(),
            chunks: Vec::new(),
        };
        let mut state = HostCommandRuntimeState::new(&mut viewer, CommandRuntimeRequirements::NONE);

        let response = state.handle_request(WireCommandRuntimeRequest::GpuDeviceLimits { id: 9 });

        assert_eq!(response.id, 9);
        assert!(response.result.is_err());
    }

    #[test]
    fn gpu_batch_request_requires_declared_requirement() {
        let mut viewer = TraceFixtureViewer {
            session: Session::new(),
            chunks: Vec::new(),
        };
        let mut state = HostCommandRuntimeState::new(&mut viewer, CommandRuntimeRequirements::NONE);

        let response = state.handle_request(WireCommandRuntimeRequest::GpuSubmitBatch {
            id: 10,
            batch: GpuSubmitBatch {
                label: Some("test.batch".to_string()),
                wait_for_completion: false,
                commands: vec![GpuBatchCommand::ReadBuffer {
                    buffer: GpuHandle {
                        id: 1,
                        kind: GpuHandleKind::Buffer,
                        generation: 1,
                    },
                    offset: 0,
                    size: 4,
                }],
            },
        });

        assert_eq!(response.id, 10);
        assert!(response.result.is_err());
    }

    #[test]
    fn gpu_stage_timestamp_ticks_decode_native_u64_values() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&7_u64.to_ne_bytes());
        bytes.extend_from_slice(&11_u64.to_ne_bytes());

        assert_eq!(decode_timestamp_ticks(&bytes), vec![7, 11]);
        assert_eq!(decode_timestamp_ticks(&bytes[..15]), vec![7]);
    }

    #[test]
    fn gpu_stage_timestamps_aggregate_by_scope() {
        let ticks = [100_u64, 160, 200, 260, 300, 280];
        let timestamps = [
            GpuStageTimestamp {
                scope: "surface_compact".to_string(),
                begin_index: 0,
                end_index: 1,
            },
            GpuStageTimestamp {
                scope: "surface_compact".to_string(),
                begin_index: 2,
                end_index: 3,
            },
            GpuStageTimestamp {
                scope: "raytrace".to_string(),
                begin_index: 4,
                end_index: 5,
            },
            GpuStageTimestamp {
                scope: "missing".to_string(),
                begin_index: 6,
                end_index: 7,
            },
        ];

        let by_scope = aggregate_stage_timestamp_ms(&ticks, &timestamps, 1_000.0);

        let surface_compact = by_scope.get("surface_compact").copied().unwrap();
        assert!((surface_compact - 0.12).abs() < f64::EPSILON);
        assert_eq!(by_scope.get("raytrace").copied(), Some(0.0));
        assert!(!by_scope.contains_key("missing"));
    }

    #[test]
    fn gpu_cache_fingerprints_ignore_debug_labels() {
        let first = shader_module_fingerprint(&GpuShaderModuleDescriptor {
            label: Some("first".to_string()),
            wgsl: "@compute @workgroup_size(1) fn main() {}".to_string(),
        });
        let second = shader_module_fingerprint(&GpuShaderModuleDescriptor {
            label: Some("second".to_string()),
            wgsl: "@compute @workgroup_size(1) fn main() {}".to_string(),
        });
        let different_source = shader_module_fingerprint(&GpuShaderModuleDescriptor {
            label: Some("first".to_string()),
            wgsl: "@compute @workgroup_size(2) fn main() {}".to_string(),
        });

        assert_eq!(first, second);
        assert_ne!(first, different_source);
    }

    #[test]
    fn gpu_cache_keys_are_plugin_scoped_and_device_scoped() {
        let fingerprint = shader_module_fingerprint(&GpuShaderModuleDescriptor {
            label: Some("shader".to_string()),
            wgsl: "@compute @workgroup_size(1) fn main() {}".to_string(),
        });
        let mut plugin_a = GpuPluginCache::new("plugin-a@1".to_string());
        let mut plugin_b = GpuPluginCache::new("plugin-b@1".to_string());

        let first = plugin_a.resource_key_for_identity(10, fingerprint);
        let repeated = plugin_a.resource_key_for_identity(10, fingerprint);
        let other_plugin = plugin_b.resource_key_for_identity(10, fingerprint);
        let other_device = plugin_a.resource_key_for_identity(11, fingerprint);

        assert_eq!(first, repeated);
        assert_ne!(first, other_plugin);
        assert_ne!(first, other_device);
        assert_eq!(plugin_a.stats().entries, 0);

        plugin_a.record_hit();
        plugin_a.record_miss();
        let stats = plugin_a.stats();
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 1);

        plugin_a.record_profile_event(GpuCacheStatus::Hit, 3);
        plugin_a.record_profile_event(GpuCacheStatus::Miss, 5);
        let delta = plugin_a.take_profile_delta();
        assert_eq!(delta.hits, 1);
        assert_eq!(delta.misses, 1);
        assert_eq!(delta.lazy_create_ms, 8);
        assert_eq!(plugin_a.take_profile_delta().hits, 0);
    }

    #[test]
    fn gpu_cache_stats_and_drop_require_gpu_runtime() {
        let mut viewer = TraceFixtureViewer {
            session: Session::new(),
            chunks: Vec::new(),
        };
        let mut state = HostCommandRuntimeState::new(&mut viewer, CommandRuntimeRequirements::NONE);

        let stats = state.handle_request(WireCommandRuntimeRequest::GpuCacheStats { id: 11 });
        let drop = state.handle_request(WireCommandRuntimeRequest::GpuDropPluginCache { id: 12 });

        assert_eq!(stats.id, 11);
        assert_eq!(drop.id, 12);
        assert!(stats.result.is_err());
        assert!(drop.result.is_err());
    }

    #[test]
    fn gpu_cache_stats_and_drop_roundtrip_without_device() {
        let mut viewer = TraceFixtureViewer {
            session: Session::new(),
            chunks: Vec::new(),
        };
        let mut state =
            HostCommandRuntimeState::new(&mut viewer, CommandRuntimeRequirements::GPU_COMMANDS);

        let stats = state.handle_request(WireCommandRuntimeRequest::GpuCacheStats { id: 21 });
        match stats.result.unwrap() {
            WireCommandRuntimeValue::GpuCacheStats(stats) => {
                assert_eq!(stats.hits, 0);
                assert_eq!(stats.misses, 0);
                assert_eq!(stats.entries, 0);
            }
            _ => panic!("unexpected cache stats response"),
        }

        let dropped =
            state.handle_request(WireCommandRuntimeRequest::GpuDropPluginCache { id: 22 });
        assert!(matches!(
            dropped.result.unwrap(),
            WireCommandRuntimeValue::GpuOk
        ));
    }

    #[test]
    fn gpu_dispatch_workgroups_are_validated_before_wgpu() {
        let err = validate_dispatch_workgroups([65_536, 1, 1], 65_535).unwrap_err();

        assert!(err.contains("workgroups.x=65536"));
        assert!(validate_dispatch_workgroups([65_535, 2, 1], 65_535).is_ok());
    }

    #[test]
    fn gpu_batch_buffer_ranges_are_validated_before_wgpu() {
        let err = validate_buffer_range(16, 12, 8).unwrap_err();

        assert!(err.contains("exceeds buffer size"));
        assert!(validate_buffer_range(16, 12, 4).is_ok());
    }

    #[test]
    fn gpu_storage_texture_formats_are_validated_before_wgpu() {
        let err = validate_storage_texture_format(
            GpuTextureFormat::Bgra8UnormSrgb,
            GpuStorageTextureAccess::WriteOnly,
        )
        .unwrap_err();

        assert!(err.contains("not supported for storage texture"));
        assert!(validate_storage_texture_format(
            GpuTextureFormat::Rgba8Unorm,
            GpuStorageTextureAccess::WriteOnly,
        )
        .is_ok());
        assert!(validate_storage_texture_format(
            GpuTextureFormat::R32Uint,
            GpuStorageTextureAccess::Atomic,
        )
        .is_ok());
        assert!(validate_storage_texture_format(
            GpuTextureFormat::Rgba8Unorm,
            GpuStorageTextureAccess::Atomic,
        )
        .is_err());
    }

    #[test]
    fn gpu_texture_copy_row_alignment_is_validated_before_wgpu() {
        let size = GpuExtent3d {
            width: 4,
            height: 2,
            depth_or_array_layers: 1,
        };
        let unaligned = GpuTexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(16),
            rows_per_image: None,
        };
        let err =
            validate_texel_copy_buffer_layout(GpuTextureFormat::Rgba8Unorm, unaligned, size, 512)
                .unwrap_err();

        assert!(err.contains("aligned to 256"));

        let aligned = GpuTexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(256),
            rows_per_image: None,
        };
        assert!(validate_texel_copy_buffer_layout(
            GpuTextureFormat::Rgba8Unorm,
            aligned,
            size,
            512,
        )
        .is_ok());
    }

    #[test]
    fn gpu_texture_copy_single_row_allows_implicit_row_pitch() {
        let size = GpuExtent3d {
            width: 4,
            height: 1,
            depth_or_array_layers: 1,
        };
        let layout = GpuTexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: None,
            rows_per_image: None,
        };

        assert!(
            validate_texel_copy_buffer_layout(GpuTextureFormat::Rgba8Unorm, layout, size, 16,)
                .is_ok()
        );
    }

    #[test]
    fn gpu_render_pipeline_descriptor_is_validated_before_wgpu() {
        let descriptor = render_pipeline_descriptor();
        assert!(validate_render_pipeline_descriptor(&descriptor).is_ok());

        let mut multisampled = descriptor.clone();
        multisampled.multisample.count = 4;
        let err = validate_render_pipeline_descriptor(&multisampled).unwrap_err();
        assert!(err.contains("multisample count 1"));

        let mut bad_color = descriptor.clone();
        bad_color.fragment.as_mut().unwrap().targets[0]
            .as_mut()
            .unwrap()
            .format = GpuTextureFormat::Depth32Float;
        let err = validate_render_pipeline_descriptor(&bad_color).unwrap_err();
        assert!(err.contains("color targets cannot use depth"));

        let mut bad_stride = descriptor;
        bad_stride.vertex.buffers[0].array_stride = 14;
        let err = validate_render_pipeline_descriptor(&bad_stride).unwrap_err();
        assert!(err.contains("array_stride"));
    }

    #[test]
    fn gpu_render_pipeline_fingerprint_tracks_semantic_descriptor() {
        let descriptor = render_pipeline_descriptor();
        let layout = fingerprint_for(GpuHandleKind::PipelineLayout, "layout");
        let vertex_module = fingerprint_for(GpuHandleKind::ShaderModule, "shader");
        let fragment = descriptor.fragment.as_ref().map(|fragment| {
            (
                vertex_module,
                fragment.entry_point.as_str(),
                &fragment.targets,
            )
        });

        let first = render_pipeline_fingerprint(RenderPipelineFingerprintInput {
            layout,
            vertex_module,
            vertex_entry_point: &descriptor.vertex.entry_point,
            vertex_buffers: &descriptor.vertex.buffers,
            fragment,
            primitive: descriptor.primitive,
            depth_stencil: descriptor.depth_stencil,
            multisample: descriptor.multisample,
        });

        let mut changed = descriptor.clone();
        changed.fragment.as_mut().unwrap().targets[0]
            .as_mut()
            .unwrap()
            .format = GpuTextureFormat::Bgra8Unorm;
        let changed_fragment = changed.fragment.as_ref().map(|fragment| {
            (
                vertex_module,
                fragment.entry_point.as_str(),
                &fragment.targets,
            )
        });
        let second = render_pipeline_fingerprint(RenderPipelineFingerprintInput {
            layout,
            vertex_module,
            vertex_entry_point: &changed.vertex.entry_point,
            vertex_buffers: &changed.vertex.buffers,
            fragment: changed_fragment,
            primitive: changed.primitive,
            depth_stencil: changed.depth_stencil,
            multisample: changed.multisample,
        });

        assert_ne!(first, second);
        assert_eq!(first.kind, GpuHandleKind::RenderPipeline);
    }

    #[test]
    fn gpu_render_pipeline_attachment_formats_are_checked_before_wgpu() {
        let pass = RenderPassAttachmentFormats {
            color_targets: vec![Some(GpuTextureFormat::Rgba8Unorm)],
            depth_stencil: Some(GpuTextureFormat::Depth32Float),
        };

        assert!(validate_render_pipeline_formats(
            &[Some(GpuTextureFormat::Rgba8Unorm)],
            Some(GpuTextureFormat::Depth32Float),
            &pass,
        )
        .is_ok());

        let err = validate_render_pipeline_formats(
            &[Some(GpuTextureFormat::Bgra8Unorm)],
            Some(GpuTextureFormat::Depth32Float),
            &pass,
        )
        .unwrap_err();
        assert!(err.contains("color targets"));
    }

    #[test]
    fn gpu_render_buffer_usage_is_validated_before_wgpu() {
        let vertex_usage = GpuBufferUsage::VERTEX.union(GpuBufferUsage::COPY_DST);
        assert!(validate_buffer_usage_bits(
            vertex_usage,
            GpuBufferUsage::VERTEX,
            "render vertex buffer",
        )
        .is_ok());
        assert!(validate_buffer_usage_bits(
            vertex_usage,
            GpuBufferUsage::INDEX,
            "render index buffer",
        )
        .is_err());

        assert!(validate_buffer_slice(64, 16, Some(16)).is_ok());
        assert!(validate_buffer_slice(64, 16, Some(0)).is_err());

        assert!(validate_indirect_buffer_range(
            GpuBufferUsage::INDIRECT,
            64,
            4,
            16,
            "render indirect buffer",
        )
        .is_ok());
        let err = validate_indirect_buffer_range(
            GpuBufferUsage::INDIRECT,
            64,
            2,
            16,
            "render indirect buffer",
        )
        .unwrap_err();
        assert!(err.contains("aligned to 4"));
    }

    #[test]
    fn gpu_render_dynamic_offsets_are_rejected_for_v1() {
        let entries = vec![GpuBindGroupLayoutEntry {
            binding: 0,
            visibility: GpuShaderStages::VERTEX,
            ty: GpuBindingType::Buffer {
                ty: GpuBufferBindingType::Uniform,
                has_dynamic_offset: true,
                min_binding_size: Some(64),
            },
        }];

        assert!(bind_group_layout_has_dynamic_offsets(&entries));
    }

    #[test]
    fn gpu_render_draw_state_is_validated_before_wgpu() {
        assert!(validate_viewport(1.0, 1.0, 0.0, 1.0).is_ok());
        assert!(validate_viewport(1.0, 1.0, 0.8, 0.2).is_err());
        assert!(validate_scissor_rect(1, 1).is_ok());
        assert!(validate_scissor_rect(0, 1).is_err());
        assert!(validate_draw_counts(3, 1).is_ok());
        assert!(validate_draw_counts(0, 1).is_err());
        assert!(checked_draw_range("vertex", u32::MAX, 1).is_err());
    }
}
