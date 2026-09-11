//! Portable plugin ABI.
//!
//! The host loads this descriptor from a dynamic library and then communicates
//! with the plugin through primitive fields, opaque handles, and callback
//! tables. Rust-owned values may be used on either side of the boundary, but
//! they must not be passed across it.

use core::ffi::c_void;

/// ABI version bumped when any descriptor or callback layout changes.
pub const ABI_VERSION: u32 = 7;

/// Host callback table version expected by ABI v5 plugins.
pub const HOST_CALLBACKS_VERSION: u32 = 5;

/// Command runtime callback table version expected by ABI v4 plugins.
pub const HOST_COMMAND_RUNTIME_CALLBACKS_VERSION: u32 = 1;

/// SDK version checked between host and plugin at load time.
pub const SDK_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Largest string copied from plugin-owned memory.
pub const MAX_ABI_STRING_LEN: usize = 64 * 1024;

/// Largest slice copied from plugin-owned memory.
pub const MAX_ABI_SLICE_LEN: usize = 16 * 1024;

/// Plugin supports metadata registration.
pub const CAPABILITY_REGISTRATION: u64 = 1 << 0;
/// Plugin can describe commands through ABI-safe metadata.
pub const CAPABILITY_COMMANDS: u64 = 1 << 1;
/// Plugin can describe panels through ABI-safe metadata.
pub const CAPABILITY_PANELS: u64 = 1 << 2;
/// Plugin can describe dynamic settings through ABI-safe metadata.
pub const CAPABILITY_SETTINGS: u64 = 1 << 3;
/// Plugin can report explicitly staged unsupported features.
pub const CAPABILITY_DIAGNOSTICS: u64 = 1 << 4;
/// Plugin can execute commands through ABI-safe runtime callbacks.
pub const CAPABILITY_COMMAND_RUNTIME: u64 = 1 << 5;
/// Plugin can render panels through ABI-safe runtime callbacks.
pub const CAPABILITY_PANEL_RUNTIME: u64 = 1 << 6;
/// Plugin can run message handlers through ABI-safe runtime callbacks.
pub const CAPABILITY_MESSAGE_RUNTIME: u64 = 1 << 7;
/// Plugin can run script handlers through ABI-safe runtime callbacks.
pub const CAPABILITY_SCRIPT_HANDLERS: u64 = 1 << 8;
/// Plugin can run file format handlers through ABI-safe runtime callbacks.
pub const CAPABILITY_FORMAT_HANDLERS: u64 = 1 << 9;
/// Plugin can register portable hotkey actions.
pub const CAPABILITY_HOTKEYS: u64 = 1 << 10;

/// Command runtime input: host-resolved displayed geometry.
pub const COMMAND_RUNTIME_DISPLAYED_GEOMETRY: u64 = 1 << 0;
/// Command runtime input: full serialized scene session.
pub const COMMAND_RUNTIME_FULL_SESSION: u64 = 1 << 1;
/// Command runtime input: command-scoped compact trace geometry stream.
pub const COMMAND_RUNTIME_TRACE_GEOMETRY_STREAM: u64 = 1 << 2;
/// Command runtime input: host-executed GPU command callbacks.
pub const COMMAND_RUNTIME_GPU_COMMANDS: u64 = 1 << 3;
/// Command runtime input: renderer-owned GPU artifact snapshot.
pub const COMMAND_RUNTIME_RENDER_ARTIFACTS: u64 = 1 << 4;

/// Runtime input bits known by this ABI.
pub const KNOWN_COMMAND_RUNTIME_REQUIREMENTS: u64 = COMMAND_RUNTIME_DISPLAYED_GEOMETRY
    | COMMAND_RUNTIME_FULL_SESSION
    | COMMAND_RUNTIME_TRACE_GEOMETRY_STREAM
    | COMMAND_RUNTIME_GPU_COMMANDS
    | COMMAND_RUNTIME_RENDER_ARTIFACTS;

/// Panel runtime input: full serialized scene session.
pub const PANEL_RUNTIME_FULL_SESSION: u64 = 1 << 0;

/// Panel runtime input bits known by this ABI.
pub const KNOWN_PANEL_RUNTIME_REQUIREMENTS: u64 = PANEL_RUNTIME_FULL_SESSION;

/// Capability bits understood by this SDK.
pub const KNOWN_CAPABILITIES: u64 = CAPABILITY_REGISTRATION
    | CAPABILITY_COMMANDS
    | CAPABILITY_PANELS
    | CAPABILITY_SETTINGS
    | CAPABILITY_DIAGNOSTICS
    | CAPABILITY_COMMAND_RUNTIME
    | CAPABILITY_PANEL_RUNTIME
    | CAPABILITY_MESSAGE_RUNTIME
    | CAPABILITY_SCRIPT_HANDLERS
    | CAPABILITY_FORMAT_HANDLERS
    | CAPABILITY_HOTKEYS;

/// Setting value kind code for an empty value.
pub const SETTING_VALUE_NONE: u8 = 0;
/// Setting value kind code for booleans.
pub const SETTING_VALUE_BOOL: u8 = 1;
/// Setting value kind code for integers.
pub const SETTING_VALUE_INT: u8 = 2;
/// Setting value kind code for single floats.
pub const SETTING_VALUE_FLOAT: u8 = 3;
/// Setting value kind code for three-component floats.
pub const SETTING_VALUE_FLOAT3: u8 = 4;
/// Setting value kind code for color indices.
pub const SETTING_VALUE_COLOR: u8 = 5;
/// Setting value kind code for strings.
pub const SETTING_VALUE_STRING: u8 = 6;

/// Setting type code for blank settings.
pub const SETTING_TYPE_BLANK: u8 = 0;
/// Setting type code for booleans.
pub const SETTING_TYPE_BOOL: u8 = 1;
/// Setting type code for integers.
pub const SETTING_TYPE_INT: u8 = 2;
/// Setting type code for single floats.
pub const SETTING_TYPE_FLOAT: u8 = 3;
/// Setting type code for three-component floats.
pub const SETTING_TYPE_FLOAT3: u8 = 4;
/// Setting type code for color indices.
pub const SETTING_TYPE_COLOR: u8 = 5;
/// Setting type code for strings.
pub const SETTING_TYPE_STRING: u8 = 6;

/// Argument hint code for no specific hint.
pub const ARG_HINT_NONE: u8 = 0;
/// Argument hint code for paths.
pub const ARG_HINT_PATH: u8 = 1;
/// Argument hint code for selections.
pub const ARG_HINT_SELECTION: u8 = 2;
/// Argument hint code for object names.
pub const ARG_HINT_OBJECT: u8 = 3;
/// Argument hint code for representation names.
pub const ARG_HINT_REPRESENTATION: u8 = 4;
/// Argument hint code for colors.
pub const ARG_HINT_COLOR: u8 = 5;
/// Argument hint code for setting names.
pub const ARG_HINT_SETTING: u8 = 6;
/// Argument hint code for setting values.
pub const ARG_HINT_SETTING_VALUE: u8 = 7;
/// Argument hint code for named selections.
pub const ARG_HINT_NAMED_SELECTION: u8 = 8;
/// Argument hint code for label property expressions.
pub const ARG_HINT_LABEL_PROPERTY: u8 = 9;
/// Argument hint code for command names.
pub const ARG_HINT_COMMAND: u8 = 10;

/// Side-effect code for scene invalidation.
pub const SIDE_EFFECT_SCENE_INVALIDATE: u8 = 1;
/// Side-effect code for scene changes.
pub const SIDE_EFFECT_SCENE_CHANGED: u8 = 2;
/// Side-effect code for shader reloads.
pub const SIDE_EFFECT_SHADER_RELOAD: u8 = 3;
/// Side-effect code for shader lighting recomputation.
pub const SIDE_EFFECT_SHADER_COMPUTE_LIGHTING: u8 = 4;
/// Side-effect code for ortho/UI invalidation.
pub const SIDE_EFFECT_ORTHO_DIRTY: u8 = 5;
/// Side-effect code for sequence view changes.
pub const SIDE_EFFECT_SEQ_CHANGED: u8 = 6;
/// Side-effect code for stereo updates.
pub const SIDE_EFFECT_STEREO_UPDATE: u8 = 7;
/// Side-effect code for representation rebuilds.
pub const SIDE_EFFECT_REPRESENTATION_REBUILD: u8 = 8;
/// Side-effect code for representation color rebuilds.
pub const SIDE_EFFECT_COLOR_REBUILD: u8 = 9;
/// Side-effect code for surface transparency changes.
pub const SIDE_EFFECT_SURFACE_TRANSPARENCY: u8 = 10;
/// Side-effect code for full scene rebuilds.
pub const SIDE_EFFECT_FULL_REBUILD: u8 = 11;
/// Side-effect code for viewport updates.
pub const SIDE_EFFECT_VIEWPORT_UPDATE: u8 = 12;

/// Panel placement code for right-docked panels.
pub const PANEL_PLACEMENT_RIGHT: u8 = 1;
/// Panel placement code for bottom-docked panels.
pub const PANEL_PLACEMENT_BOTTOM: u8 = 2;

/// ABI call completed successfully.
pub const ABI_STATUS_OK: i32 = 0;
/// ABI call failed because plugin or host code panicked.
pub const ABI_STATUS_PANIC: i32 = 1;
/// ABI call received invalid input.
pub const ABI_STATUS_INVALID: i32 = 2;
/// ABI call reached an explicitly staged unsupported feature.
pub const ABI_STATUS_UNSUPPORTED: i32 = 3;
/// ABI call failed in host-side registration.
pub const ABI_STATUS_HOST_ERROR: i32 = 4;

/// Opaque handle to host registration state.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostRegistrarHandle(pub *mut c_void);

/// Opaque handle to a plugin-owned command.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PluginCommandHandle(pub *mut c_void);

/// Opaque handle to a plugin-owned panel.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PluginPanelHandle(pub *mut c_void);

/// Opaque handle to a plugin-owned message handler.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PluginMessageHandlerHandle(pub *mut c_void);

/// Opaque handle to a plugin-owned script handler.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PluginScriptHandlerHandle(pub *mut c_void);

/// Opaque handle to a plugin-owned format handler.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PluginFormatHandlerHandle(pub *mut c_void);

/// Opaque handle to a plugin-owned hotkey callback.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PluginHotkeyHandle(pub *mut c_void);

/// Opaque handle to host command runtime state.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostCommandRuntimeHandle(pub *mut c_void);

/// ABI status returned by plugin and host callbacks.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbiStatus {
    /// One of the `ABI_STATUS_*` constants.
    pub code: i32,
}

impl AbiStatus {
    /// Successful status.
    pub const OK: Self = Self {
        code: ABI_STATUS_OK,
    };
    /// Panic status.
    pub const PANIC: Self = Self {
        code: ABI_STATUS_PANIC,
    };
    /// Invalid input status.
    pub const INVALID: Self = Self {
        code: ABI_STATUS_INVALID,
    };
    /// Unsupported feature status.
    pub const UNSUPPORTED: Self = Self {
        code: ABI_STATUS_UNSUPPORTED,
    };
    /// Host registration error status.
    pub const HOST_ERROR: Self = Self {
        code: ABI_STATUS_HOST_ERROR,
    };

    /// Returns `true` when the status is successful.
    pub const fn is_ok(self) -> bool {
        self.code == ABI_STATUS_OK
    }
}

/// Host-provided sink that copies runtime output bytes.
pub type AbiBytesSinkFn = unsafe extern "C" fn(*mut c_void, AbiU8Slice) -> AbiStatus;

/// Host command-runtime request callback.
pub type HostCommandRuntimeRequestFn = unsafe extern "C" fn(
    HostCommandRuntimeHandle,
    AbiU8Slice,
    AbiBytesSinkFn,
    *mut c_void,
) -> AbiStatus;

/// Host command-runtime callback table.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct HostCommandRuntimeCallbacks {
    /// Must equal [`HOST_COMMAND_RUNTIME_CALLBACKS_VERSION`].
    pub table_version: u32,
    /// Execute a command-scoped runtime request.
    pub request: Option<HostCommandRuntimeRequestFn>,
}

/// Plugin command execution callback.
pub type PluginCommandExecuteFn = unsafe extern "C" fn(
    PluginCommandHandle,
    AbiU8Slice,
    *const HostCommandRuntimeCallbacks,
    HostCommandRuntimeHandle,
    AbiBytesSinkFn,
    *mut c_void,
) -> AbiStatus;
/// Plugin command destroy callback.
pub type PluginCommandDestroyFn = unsafe extern "C" fn(PluginCommandHandle) -> AbiStatus;

/// Plugin panel snapshot callback.
pub type PluginPanelSnapshotFn =
    unsafe extern "C" fn(PluginPanelHandle, AbiU8Slice, AbiBytesSinkFn, *mut c_void) -> AbiStatus;
/// Plugin panel event callback.
pub type PluginPanelEventFn =
    unsafe extern "C" fn(PluginPanelHandle, AbiU8Slice, AbiBytesSinkFn, *mut c_void) -> AbiStatus;
/// Plugin panel destroy callback.
pub type PluginPanelDestroyFn = unsafe extern "C" fn(PluginPanelHandle) -> AbiStatus;

/// Plugin message delivery callback.
pub type PluginMessageOnMessageFn = unsafe extern "C" fn(
    PluginMessageHandlerHandle,
    AbiU8Slice,
    AbiBytesSinkFn,
    *mut c_void,
) -> AbiStatus;
/// Plugin message poll callback.
pub type PluginMessagePollFn = unsafe extern "C" fn(
    PluginMessageHandlerHandle,
    AbiU8Slice,
    AbiBytesSinkFn,
    *mut c_void,
) -> AbiStatus;
/// Plugin message handler destroy callback.
pub type PluginMessageDestroyFn = unsafe extern "C" fn(PluginMessageHandlerHandle) -> AbiStatus;

/// Plugin script handler callback.
pub type PluginScriptRunFn = unsafe extern "C" fn(
    PluginScriptHandlerHandle,
    AbiU8Slice,
    AbiBytesSinkFn,
    *mut c_void,
) -> AbiStatus;
/// Plugin script handler destroy callback.
pub type PluginScriptDestroyFn = unsafe extern "C" fn(PluginScriptHandlerHandle) -> AbiStatus;

/// Plugin format reader callback.
pub type PluginFormatReadFn = unsafe extern "C" fn(
    PluginFormatHandlerHandle,
    AbiU8Slice,
    AbiBytesSinkFn,
    *mut c_void,
) -> AbiStatus;
/// Plugin format writer callback.
pub type PluginFormatWriteFn = unsafe extern "C" fn(
    PluginFormatHandlerHandle,
    AbiU8Slice,
    AbiBytesSinkFn,
    *mut c_void,
) -> AbiStatus;
/// Plugin format handler destroy callback.
pub type PluginFormatDestroyFn = unsafe extern "C" fn(PluginFormatHandlerHandle) -> AbiStatus;

/// Plugin hotkey callback.
pub type PluginHotkeyInvokeFn =
    unsafe extern "C" fn(PluginHotkeyHandle, AbiU8Slice, AbiBytesSinkFn, *mut c_void) -> AbiStatus;
/// Plugin hotkey destroy callback.
pub type PluginHotkeyDestroyFn = unsafe extern "C" fn(PluginHotkeyHandle) -> AbiStatus;

/// ABI-safe command runtime vtable.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AbiCommandVTable {
    /// Execute the command.
    pub execute: Option<PluginCommandExecuteFn>,
    /// Destroy the plugin-owned command handle.
    pub destroy: Option<PluginCommandDestroyFn>,
}

/// ABI-safe panel runtime vtable.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AbiPanelVTable {
    /// Render a snapshot.
    pub snapshot: Option<PluginPanelSnapshotFn>,
    /// Handle a frontend event.
    pub handle_event: Option<PluginPanelEventFn>,
    /// Destroy the plugin-owned panel handle.
    pub destroy: Option<PluginPanelDestroyFn>,
}

/// ABI-safe message handler runtime vtable.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AbiMessageHandlerVTable {
    /// Deliver a broadcast message.
    pub on_message: Option<PluginMessageOnMessageFn>,
    /// Return non-zero when this handler needs polling.
    pub needs_poll: u8,
    /// Poll the handler.
    pub poll: Option<PluginMessagePollFn>,
    /// Destroy the plugin-owned handler handle.
    pub destroy: Option<PluginMessageDestroyFn>,
}

/// ABI-safe script handler runtime vtable.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AbiScriptHandlerVTable {
    /// Run the script handler.
    pub run: Option<PluginScriptRunFn>,
    /// Destroy the plugin-owned script handler handle.
    pub destroy: Option<PluginScriptDestroyFn>,
}

/// ABI-safe format handler runtime vtable.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AbiFormatHandlerVTable {
    /// Read file bytes.
    pub read: Option<PluginFormatReadFn>,
    /// Write file bytes.
    pub write: Option<PluginFormatWriteFn>,
    /// Destroy the plugin-owned format handler handle.
    pub destroy: Option<PluginFormatDestroyFn>,
}

/// ABI-safe hotkey callback vtable.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AbiHotkeyVTable {
    /// Invoke the hotkey callback.
    pub invoke: Option<PluginHotkeyInvokeFn>,
    /// Destroy the plugin-owned hotkey handle.
    pub destroy: Option<PluginHotkeyDestroyFn>,
}

/// Borrowed UTF-8 bytes owned by the caller.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbiStr {
    /// Pointer to UTF-8 bytes without a null terminator.
    pub ptr: *const u8,
    /// Byte length.
    pub len: usize,
}

impl AbiStr {
    /// Empty string view.
    pub const EMPTY: Self = Self {
        ptr: core::ptr::null(),
        len: 0,
    };

    /// Builds a string view from a static string.
    pub const fn from_static(value: &'static str) -> Self {
        Self {
            ptr: value.as_ptr(),
            len: value.len(),
        }
    }

    /// Builds a string view from a borrowed string.
    pub fn from_borrowed(value: &str) -> Self {
        Self {
            ptr: value.as_ptr(),
            len: value.len(),
        }
    }

    /// Read this view as bytes after null and length checks.
    ///
    /// # Safety
    ///
    /// Non-null pointers must reference initialized memory that remains valid
    /// for reads of `len` bytes for the returned lifetime.
    pub unsafe fn as_bytes_checked<'a>(self, max_len: usize) -> Result<&'a [u8], AbiReadError> {
        if self.len > max_len {
            return Err(AbiReadError::TooLong);
        }
        if self.len == 0 {
            return Ok(&[]);
        }
        if self.ptr.is_null() {
            return Err(AbiReadError::NullPointer);
        }
        // SAFETY: The caller guarantees that non-null pointers are valid for
        // reads of `len` initialized bytes. This method checks null and length.
        Ok(unsafe { core::slice::from_raw_parts(self.ptr, self.len) })
    }
}

/// Borrowed slice of [`AbiStr`] entries.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbiStrSlice {
    /// Pointer to the first entry.
    pub ptr: *const AbiStr,
    /// Number of entries.
    pub len: usize,
}

impl AbiStrSlice {
    /// Empty slice view.
    pub const EMPTY: Self = Self {
        ptr: core::ptr::null(),
        len: 0,
    };
}

/// Borrowed slice of bytes.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbiU8Slice {
    /// Pointer to the first byte.
    pub ptr: *const u8,
    /// Number of bytes.
    pub len: usize,
}

impl AbiU8Slice {
    /// Empty byte slice view.
    pub const EMPTY: Self = Self {
        ptr: core::ptr::null(),
        len: 0,
    };
}

/// Error reported while validating borrowed ABI memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbiReadError {
    /// Non-empty pointer field was null.
    NullPointer,
    /// Length exceeds the caller's configured maximum.
    TooLong,
    /// Bytes were not valid UTF-8.
    InvalidUtf8,
}

/// ABI-safe command descriptor copied by the host during registration.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AbiCommandDescriptor {
    /// Plugin-owned command handle.
    pub handle: PluginCommandHandle,
    /// Runtime callbacks for this command.
    pub vtable: AbiCommandVTable,
    /// Command name.
    pub name: AbiStr,
    /// Short description.
    pub description: AbiStr,
    /// Usage text.
    pub usage: AbiStr,
    /// Argument help text.
    pub arguments: AbiStr,
    /// Full help text.
    pub help: AbiStr,
    /// Alias string views.
    pub aliases: AbiStrSlice,
    /// Argument hint codes.
    pub arg_hints: AbiU8Slice,
    /// Argument grammar code from `patinae_cmd::ArgumentSyntax`.
    pub argument_syntax: u8,
    /// Extra host runtime inputs required by this command.
    pub runtime_requirements: u64,
}

/// ABI-safe panel descriptor copied by the host during registration.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AbiPanelDescriptor {
    /// Plugin-owned panel handle.
    pub handle: PluginPanelHandle,
    /// Runtime callbacks for this panel.
    pub vtable: AbiPanelVTable,
    /// Stable panel ID.
    pub id: AbiStr,
    /// Human-readable title.
    pub title: AbiStr,
    /// Short icon text.
    pub icon: AbiStr,
    /// One of the `PANEL_PLACEMENT_*` constants.
    pub placement: u8,
    /// Non-zero when the panel should start visible.
    pub default_visible: u8,
    /// Extra host runtime inputs required by this panel.
    pub runtime_requirements: u64,
}

/// ABI-safe message handler descriptor.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AbiMessageHandlerDescriptor {
    /// Plugin-owned message handler handle.
    pub handle: PluginMessageHandlerHandle,
    /// Runtime callbacks for this message handler.
    pub vtable: AbiMessageHandlerVTable,
}

/// ABI-safe script handler descriptor.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AbiScriptHandlerDescriptor {
    /// File extension without a dot.
    pub extension: AbiStr,
    /// Plugin-owned script handler handle.
    pub handle: PluginScriptHandlerHandle,
    /// Runtime callbacks for this script handler.
    pub vtable: AbiScriptHandlerVTable,
}

/// ABI-safe format handler descriptor.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AbiFormatHandlerDescriptor {
    /// Human-readable format name.
    pub name: AbiStr,
    /// Supported file extensions without dots.
    pub extensions: AbiStrSlice,
    /// Plugin-owned format handler handle.
    pub handle: PluginFormatHandlerHandle,
    /// Runtime callbacks for this format handler.
    pub vtable: AbiFormatHandlerVTable,
}

/// ABI-safe hotkey descriptor.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AbiHotkeyDescriptor {
    /// Host-parsed key string.
    pub key: AbiStr,
    /// Serialized host-owned action bytes.
    pub action: AbiU8Slice,
    /// Optional plugin-owned callback handle.
    pub handle: PluginHotkeyHandle,
    /// Optional runtime callbacks for plugin-owned callback actions.
    pub vtable: AbiHotkeyVTable,
}

/// ABI-safe setting value.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AbiSettingValue {
    /// One of the `SETTING_VALUE_*` constants.
    pub tag: u8,
    /// Boolean payload for [`SETTING_VALUE_BOOL`].
    pub bool_value: u8,
    /// Integer payload for integer and color values.
    pub int_value: i32,
    /// Float payload for single and vector values.
    pub float_values: [f32; 3],
    /// String payload for [`SETTING_VALUE_STRING`].
    pub string_value: AbiStr,
}

impl AbiSettingValue {
    /// Empty setting value.
    pub const NONE: Self = Self {
        tag: SETTING_VALUE_NONE,
        bool_value: 0,
        int_value: 0,
        float_values: [0.0; 3],
        string_value: AbiStr::EMPTY,
    };
}

/// ABI-safe setting value hint.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AbiSettingValueHint {
    /// Hint name.
    pub name: AbiStr,
    /// Hint value.
    pub value: AbiSettingValue,
}

/// Borrowed slice of setting value hints.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AbiSettingValueHintSlice {
    /// Pointer to the first hint.
    pub ptr: *const AbiSettingValueHint,
    /// Number of hints.
    pub len: usize,
}

impl AbiSettingValueHintSlice {
    /// Empty hint slice view.
    pub const EMPTY: Self = Self {
        ptr: core::ptr::null(),
        len: 0,
    };
}

/// ABI-safe dynamic setting descriptor.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AbiSettingDescriptor {
    /// Setting name.
    pub name: AbiStr,
    /// One of the `SETTING_TYPE_*` constants.
    pub setting_type: u8,
    /// Default value.
    pub default_value: AbiSettingValue,
    /// Non-zero when `min` is present.
    pub has_min: u8,
    /// Minimum numeric value.
    pub min: f32,
    /// Non-zero when `max` is present.
    pub has_max: u8,
    /// Maximum numeric value.
    pub max: f32,
    /// Value hint descriptors.
    pub value_hints: AbiSettingValueHintSlice,
    /// Side-effect code slice.
    pub side_effects: AbiU8Slice,
    /// Non-zero when object overrides are supported.
    pub object_overridable: u8,
}

/// Host metadata registration callback.
pub type HostRegisterMetadataFn =
    unsafe extern "C" fn(HostRegistrarHandle, AbiStr, AbiStr, AbiStr) -> AbiStatus;
/// Host command registration callback.
pub type HostRegisterCommandFn =
    unsafe extern "C" fn(HostRegistrarHandle, *const AbiCommandDescriptor) -> AbiStatus;
/// Host panel registration callback.
pub type HostRegisterPanelFn =
    unsafe extern "C" fn(HostRegistrarHandle, *const AbiPanelDescriptor) -> AbiStatus;
/// Host setting registration callback.
pub type HostRegisterSettingFn =
    unsafe extern "C" fn(HostRegistrarHandle, *const AbiSettingDescriptor) -> AbiStatus;
/// Host message handler registration callback.
pub type HostRegisterMessageHandlerFn =
    unsafe extern "C" fn(HostRegistrarHandle, *const AbiMessageHandlerDescriptor) -> AbiStatus;
/// Host script handler registration callback.
pub type HostRegisterScriptHandlerFn =
    unsafe extern "C" fn(HostRegistrarHandle, *const AbiScriptHandlerDescriptor) -> AbiStatus;
/// Host format handler registration callback.
pub type HostRegisterFormatHandlerFn =
    unsafe extern "C" fn(HostRegistrarHandle, *const AbiFormatHandlerDescriptor) -> AbiStatus;
/// Host hotkey registration callback.
pub type HostRegisterHotkeyFn =
    unsafe extern "C" fn(HostRegistrarHandle, *const AbiHotkeyDescriptor) -> AbiStatus;
/// Host unsupported-feature diagnostic callback.
pub type HostReportUnsupportedFn =
    unsafe extern "C" fn(HostRegistrarHandle, AbiStr, AbiStr) -> AbiStatus;

/// Host callback table passed to plugin entry points.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct HostCallbacks {
    /// Must equal [`HOST_CALLBACKS_VERSION`].
    pub table_version: u32,
    /// Register plugin metadata.
    pub register_metadata: Option<HostRegisterMetadataFn>,
    /// Register a command descriptor.
    pub register_command: Option<HostRegisterCommandFn>,
    /// Register a panel descriptor.
    pub register_panel: Option<HostRegisterPanelFn>,
    /// Register a dynamic setting descriptor.
    pub register_setting: Option<HostRegisterSettingFn>,
    /// Register a message handler descriptor.
    pub register_message_handler: Option<HostRegisterMessageHandlerFn>,
    /// Register a script handler descriptor.
    pub register_script_handler: Option<HostRegisterScriptHandlerFn>,
    /// Register a format handler descriptor.
    pub register_format_handler: Option<HostRegisterFormatHandlerFn>,
    /// Register a hotkey descriptor.
    pub register_hotkey: Option<HostRegisterHotkeyFn>,
    /// Report an explicitly staged feature.
    pub report_unsupported: Option<HostReportUnsupportedFn>,
}

/// Plugin initialization callback.
pub type PluginInitFn = unsafe extern "C" fn(*const HostCallbacks) -> AbiStatus;
/// Plugin registration callback.
pub type PluginRegisterFn =
    unsafe extern "C" fn(HostRegistrarHandle, *const HostCallbacks) -> AbiStatus;

/// C-compatible plugin declaration exported by every dynamic plugin.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct PluginDeclaration {
    /// Must equal [`ABI_VERSION`].
    pub abi_version: u32,
    /// SDK version string.
    pub sdk_version: AbiStr,
    /// Capability bits advertised by the plugin.
    pub capabilities: u64,
    /// Optional initialization function.
    pub init: Option<PluginInitFn>,
    /// Required registration function.
    pub register: Option<PluginRegisterFn>,
}

// SAFETY: The declaration is an immutable descriptor containing only scalar
// fields, raw borrowed static bytes, and function pointers. Calling function
// pointers still requires the documented ABI contract.
unsafe impl Send for PluginDeclaration {}
// SAFETY: Shared references only observe immutable fields. The function pointer
// targets are not invoked without explicit unsafe ABI validation by the host.
unsafe impl Sync for PluginDeclaration {}
