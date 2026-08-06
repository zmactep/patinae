//! Plugin Registrar
//!
//! Plugins use [`PluginRegistrar`] as the ergonomic SDK layer during
//! registration. Internally it adapts Rust plugin declarations into ABI-safe
//! host callbacks, so the dynamic library boundary only sees primitive views,
//! opaque handles, and callback tables.

use std::ffi::c_void;
use std::io::{Cursor, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub use patinae_cmd::DynamicCommandInvocation;
use patinae_cmd::{Command, CommandContext, CommandRegistry};
pub use patinae_cmd::{FormatHandler, PluginReaderFn, PluginWriterFn, ScriptHandler, ViewerLike};
use patinae_framework::component::SharedContext;
use patinae_framework::message::{AppMessage, MessageBus};
pub use patinae_framework::plugin_ui::{
    PanelAction, PanelControl, PanelDescriptor, PanelEvent, PanelOption, PanelPlacement,
    PanelSnapshot, PanelValue, PluginPanel,
};
pub use patinae_scene::{parse_key_string, KeyBinding, KeyCode};
pub use patinae_settings::{DynamicSettingDescriptor, DynamicSettingStore, SharedSettingStore};

use crate::ffi::{
    AbiBytesSinkFn, AbiCommandDescriptor, AbiCommandVTable, AbiFormatHandlerDescriptor,
    AbiFormatHandlerVTable, AbiHotkeyDescriptor, AbiHotkeyVTable, AbiMessageHandlerDescriptor,
    AbiMessageHandlerVTable, AbiPanelDescriptor, AbiPanelVTable, AbiScriptHandlerDescriptor,
    AbiScriptHandlerVTable, AbiSettingDescriptor, AbiSettingValue, AbiSettingValueHint,
    AbiSettingValueHintSlice, AbiStatus, AbiStr, AbiStrSlice, AbiU8Slice, HostCallbacks,
    HostCommandRuntimeCallbacks, HostCommandRuntimeHandle, HostRegistrarHandle,
    PluginCommandHandle, PluginFormatHandlerHandle, PluginHotkeyHandle, PluginMessageHandlerHandle,
    PluginPanelHandle, PluginScriptHandlerHandle, ARG_HINT_COLOR, ARG_HINT_COMMAND,
    ARG_HINT_LABEL_PROPERTY, ARG_HINT_NAMED_SELECTION, ARG_HINT_NONE, ARG_HINT_OBJECT,
    ARG_HINT_PATH, ARG_HINT_REPRESENTATION, ARG_HINT_SELECTION, ARG_HINT_SETTING,
    ARG_HINT_SETTING_VALUE, HOST_CALLBACKS_VERSION, HOST_COMMAND_RUNTIME_CALLBACKS_VERSION,
    PANEL_PLACEMENT_BOTTOM, PANEL_PLACEMENT_RIGHT, SETTING_TYPE_BLANK, SETTING_TYPE_BOOL,
    SETTING_TYPE_COLOR, SETTING_TYPE_FLOAT, SETTING_TYPE_FLOAT3, SETTING_TYPE_INT,
    SETTING_TYPE_STRING, SETTING_VALUE_BOOL, SETTING_VALUE_COLOR, SETTING_VALUE_FLOAT,
    SETTING_VALUE_FLOAT3, SETTING_VALUE_INT, SETTING_VALUE_STRING, SIDE_EFFECT_COLOR_REBUILD,
    SIDE_EFFECT_FULL_REBUILD, SIDE_EFFECT_ORTHO_DIRTY, SIDE_EFFECT_REPRESENTATION_REBUILD,
    SIDE_EFFECT_SCENE_CHANGED, SIDE_EFFECT_SCENE_INVALIDATE, SIDE_EFFECT_SEQ_CHANGED,
    SIDE_EFFECT_SHADER_COMPUTE_LIGHTING, SIDE_EFFECT_SHADER_RELOAD, SIDE_EFFECT_STEREO_UPDATE,
    SIDE_EFFECT_SURFACE_TRANSPARENCY, SIDE_EFFECT_VIEWPORT_UPDATE,
};
use crate::wire::{
    self, WireCommandInput, WireCommandOutput, WireCommandRuntimeRequest,
    WireCommandRuntimeResponse, WireCommandRuntimeValue, WireDynCmdRegistration,
    WireFormatReadInput, WireFormatReadOutput, WireFormatWriteInput, WireFormatWriteOutput,
    WireHostQuery, WireHostQueryResult, WireHotkeyAction, WireHotkeyRegistration, WireMessageInput,
    WireMessageOutput, WirePanelEventInput, WirePanelEventOutput, WirePanelSnapshotOutput,
    WirePollInput, WirePollOutput, WirePollSharedInput, WireScriptInput, WireScriptOutput,
    WireViewerAction, RUNTIME_WIRE_VERSION,
};

/// A boxed closure that mutates the viewer on the main thread.
///
/// Queued by plugins via [`PollContext::queue_viewer_mutation`] during `poll()`,
/// then executed by the host in phase 2 with full mutable viewer access.
pub type ViewerMutation = Box<dyn FnOnce(&mut dyn ViewerLike) + Send>;

// =============================================================================
// Polling types
// =============================================================================

/// Result of a command execution requested via [`PollContext::execute_command`].
///
/// Delivered to the plugin in the next `poll()` call.
pub struct CommandResult {
    /// Correlation ID (set by the plugin when requesting execution).
    pub id: u64,
    /// `Ok(())` on success, `Err(message)` on failure.
    pub result: Result<(), String>,
}

/// Queued command execution request (internal).
pub struct CommandExecRequest {
    pub id: u64,
    pub command: String,
    pub silent: bool,
}

/// Queued dynamic command registration (internal).
pub struct DynCmdRegistration {
    pub name: String,
    pub description: String,
    pub usage: String,
    pub arguments: String,
}

// =============================================================================
// Hotkey types
// =============================================================================

/// Callback for hotkey actions — runs during the plugin poll phase.
pub type HotkeyCallback = Box<dyn FnMut(&mut PollContext<'_>) + Send>;

type ScriptHandlerCallback = dyn Fn(&str) -> Result<(), String> + Send + Sync;
type BoxedScriptHandler = Box<ScriptHandlerCallback>;

/// Action to perform when a plugin hotkey is triggered.
pub enum PluginKeyAction {
    /// Execute a command string.
    Command(String),
    /// Invoke a dynamic command (delivered via `dynamic_invocations` in next poll).
    DynamicCommand { name: String, args: Vec<String> },
    /// Publish a custom message to the message bus.
    Custom { topic: String, payload: Vec<u8> },
    /// Run arbitrary code during the next poll phase with `PollContext` access.
    Callback(HotkeyCallback),
}

/// Context provided to plugins during periodic polling.
///
/// Plugins that return `needs_poll() == true` receive this each frame.
/// It provides read-only state access, message bus, deferred command
/// execution, and dynamic command registration.
pub struct PollContext<'a> {
    /// Read-only application state.
    pub shared: &'a SharedContext<'a>,
    /// Lightweight portable host state for dynamic poll paths.
    pub poll_shared: &'a WirePollSharedInput,
    /// Message bus for sending messages (print, quit, execute, etc.).
    pub bus: &'a mut MessageBus,
    /// Results from command executions requested in previous poll cycles.
    pub command_results: &'a [CommandResult],
    /// Results from host queries requested in previous poll cycles.
    pub host_query_results: &'a [WireHostQueryResult],
    /// Invocations of dynamic commands since last poll.
    pub dynamic_invocations: &'a [DynamicCommandInvocation],
    /// Hotkey bindings triggered since last poll (read-only).
    pub triggered_hotkeys: &'a [KeyBinding],
    /// Directories from which plugins were loaded (for resource discovery).
    pub plugin_dirs: &'a [PathBuf],
    // Internal queues — accessed via methods
    pub(crate) exec_queue: &'a mut Vec<CommandExecRequest>,
    pub(crate) reg_queue: &'a mut Vec<DynCmdRegistration>,
    pub(crate) unreg_queue: &'a mut Vec<String>,
    pub(crate) notification_queue: &'a mut Vec<String>,
    pub(crate) hotkey_reg_queue: &'a mut Vec<(String, PluginKeyAction)>,
    pub(crate) hotkey_unreg_queue: &'a mut Vec<String>,
    pub(crate) mutation_queue: &'a mut Vec<ViewerMutation>,
    pub(crate) host_query_queue: &'a mut Vec<WireHostQuery>,
    pub(crate) viewer_action_queue: &'a mut Vec<WireViewerAction>,
    pub(crate) panel_update_requested: &'a mut bool,
}

impl<'a> PollContext<'a> {
    /// Create a new poll context.
    ///
    /// This is used by the host to build the context before calling `poll()`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        shared: &'a SharedContext<'a>,
        poll_shared: &'a WirePollSharedInput,
        bus: &'a mut MessageBus,
        command_results: &'a [CommandResult],
        host_query_results: &'a [WireHostQueryResult],
        dynamic_invocations: &'a [DynamicCommandInvocation],
        triggered_hotkeys: &'a [KeyBinding],
        plugin_dirs: &'a [PathBuf],
        exec_queue: &'a mut Vec<CommandExecRequest>,
        reg_queue: &'a mut Vec<DynCmdRegistration>,
        unreg_queue: &'a mut Vec<String>,
        notification_queue: &'a mut Vec<String>,
        hotkey_reg_queue: &'a mut Vec<(String, PluginKeyAction)>,
        hotkey_unreg_queue: &'a mut Vec<String>,
        mutation_queue: &'a mut Vec<ViewerMutation>,
        host_query_queue: &'a mut Vec<WireHostQuery>,
        viewer_action_queue: &'a mut Vec<WireViewerAction>,
        panel_update_requested: &'a mut bool,
    ) -> Self {
        Self {
            shared,
            poll_shared,
            bus,
            command_results,
            host_query_results,
            dynamic_invocations,
            triggered_hotkeys,
            plugin_dirs,
            exec_queue,
            reg_queue,
            unreg_queue,
            notification_queue,
            hotkey_reg_queue,
            hotkey_unreg_queue,
            mutation_queue,
            host_query_queue,
            viewer_action_queue,
            panel_update_requested,
        }
    }

    /// Queue a command for execution.
    ///
    /// The command is executed by the host after `poll()` returns.
    /// The result (success or error message) is delivered in the next
    /// `poll()` call via [`PollContext::command_results`].
    pub fn execute_command(&mut self, id: u64, command: &str, silent: bool) {
        self.exec_queue.push(CommandExecRequest {
            id,
            command: command.to_string(),
            silent,
        });
    }

    /// Register a dynamic command.
    ///
    /// The command appears in autocomplete and help immediately (next frame).
    /// When a user invokes it, the invocation is delivered to the plugin
    /// via [`PollContext::dynamic_invocations`] in the next `poll()` call.
    pub fn register_dynamic_command(
        &mut self,
        name: String,
        description: String,
        usage: String,
        arguments: String,
    ) {
        self.reg_queue.push(DynCmdRegistration {
            name,
            description,
            usage,
            arguments,
        });
    }

    /// Unregister a dynamic command.
    pub fn unregister_dynamic_command(&mut self, name: &str) {
        self.unreg_queue.push(name.to_string());
    }

    /// Show a notification message in the overlay (spinner + text).
    ///
    /// Call this during `poll()` while a background operation is in progress.
    /// The notification is cleared automatically when `poll()` returns without
    /// calling this method.
    pub fn set_notification(&mut self, msg: impl Into<String>) {
        self.notification_queue.push(msg.into());
    }

    /// Register a hotkey binding by key string (e.g. `"ctrl+s"`).
    ///
    /// The string is parsed on the host side (which has the correct `KeyCode`
    /// enum from winit). Applied after `poll()` returns.
    pub fn register_hotkey(&mut self, key: impl Into<String>, action: PluginKeyAction) {
        self.hotkey_reg_queue.push((key.into(), action));
    }

    /// Unregister a hotkey binding by key string (e.g. `"ctrl+s"`).
    ///
    /// Applied after `poll()` returns.
    pub fn unregister_hotkey(&mut self, key: impl Into<String>) {
        self.hotkey_unreg_queue.push(key.into());
    }

    /// Queue a mutation to be applied to the viewer on the main thread.
    ///
    /// The closure is executed by the host after `poll()` returns, during
    /// phase 2, with full mutable access to the viewer. Use this for
    /// operations that require modifying viewer state (e.g., atom properties).
    pub fn queue_viewer_mutation(&mut self, f: impl FnOnce(&mut dyn ViewerLike) + Send + 'static) {
        self.mutation_queue.push(Box::new(f));
    }

    /// Queue a portable host query for dynamic plugin runtimes.
    pub fn query_host(&mut self, query: WireHostQuery) {
        self.host_query_queue.push(query);
    }

    /// Queue a portable viewer action for dynamic plugin runtimes.
    pub fn queue_viewer_action(&mut self, action: WireViewerAction) {
        self.viewer_action_queue.push(action);
    }

    /// Store a viewport overlay image.
    pub fn set_viewport_image(&mut self, image: patinae_scene::ViewportImage) {
        self.queue_viewer_action(WireViewerAction::SetViewportImage(image));
    }

    /// Clear the viewport overlay image.
    pub fn clear_viewport_image(&mut self) {
        self.queue_viewer_action(WireViewerAction::ClearViewportImage);
    }

    /// Request a host viewport redraw.
    pub fn request_redraw(&mut self) {
        self.queue_viewer_action(WireViewerAction::RequestRedraw);
    }

    /// Request a plugin panel refresh after poll-owned state changes.
    ///
    /// Use this when a message handler updates state that is rendered by a
    /// declarative panel but is not otherwise reflected in scene or command
    /// generation counters.
    pub fn request_panel_update(&mut self) {
        *self.panel_update_requested = true;
        self.queue_viewer_action(WireViewerAction::RequestPanelUpdate);
    }
}

// =============================================================================
// MessageHandler trait
// =============================================================================

/// Plugin metadata — name, version, description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginMetadata {
    pub name: String,
    pub version: String,
    pub description: String,
}

impl PluginMetadata {
    /// Create plugin metadata.
    pub fn new(
        name: impl Into<String>,
        version: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            description: description.into(),
        }
    }
}

/// Trait for headless plugins that need to react to messages without a GUI.
///
/// Plugins that also need periodic polling should override [`Self::needs_poll`] and
/// [`Self::poll`] — the host will call `poll()` each frame with a [`PollContext`].
pub trait MessageHandler: Send {
    /// Called for every dispatched message after components have been notified.
    fn on_message(&mut self, msg: &AppMessage, bus: &mut MessageBus);

    /// Whether this handler needs periodic `poll()` calls (default: `false`).
    ///
    /// When `true`, the host will call [`Self::poll`] each frame and set a 50 ms
    /// wake-up timer so the event loop doesn't sleep indefinitely.
    fn needs_poll(&self) -> bool {
        false
    }

    /// Called each frame when [`Self::needs_poll`] returns `true`.
    ///
    /// Use this for non-blocking I/O (e.g., polling a socket), deferred
    /// command execution, and dynamic command management.
    fn poll(&mut self, _ctx: &mut PollContext<'_>) {}
}

// =============================================================================
// PluginRegistrar
// =============================================================================

/// ABI-backed plugin registrar used during plugin registration.
///
/// The plugin SDK keeps the ergonomic Rust API, but this type forwards
/// registration data to host callbacks using portable ABI descriptors.
pub struct PluginRegistrar<'a> {
    handle: HostRegistrarHandle,
    callbacks: &'a HostCallbacks,
    status: AbiStatus,
}

impl<'a> PluginRegistrar<'a> {
    /// Create a registrar from ABI inputs.
    ///
    /// # Safety
    ///
    /// `callbacks` must point to a valid [`HostCallbacks`] table that remains
    /// live for the duration of registration. `handle` must be the host-provided
    /// registration handle associated with that table.
    pub unsafe fn from_abi(
        handle: HostRegistrarHandle,
        callbacks: *const HostCallbacks,
    ) -> Result<Self, AbiStatus> {
        if callbacks.is_null() || handle.0.is_null() {
            return Err(AbiStatus::INVALID);
        }
        // SAFETY: The caller guarantees `callbacks` points to a live callback
        // table for this registration call. Null was checked above.
        let callbacks = unsafe { &*callbacks };
        if callbacks.table_version != HOST_CALLBACKS_VERSION {
            return Err(AbiStatus::INVALID);
        }
        Ok(Self {
            handle,
            callbacks,
            status: AbiStatus::OK,
        })
    }

    /// Set the plugin's metadata.
    pub fn set_metadata(&mut self, metadata: PluginMetadata) {
        let Some(register_metadata) = self.callbacks.register_metadata else {
            self.record_status(AbiStatus::INVALID);
            return;
        };
        // SAFETY: The callback table and registrar handle were validated when
        // this registrar was constructed. The ABI string views remain valid for
        // the duration of the callback, and the host copies them immediately.
        let status = unsafe {
            register_metadata(
                self.handle,
                AbiStr::from_borrowed(&metadata.name),
                AbiStr::from_borrowed(&metadata.version),
                AbiStr::from_borrowed(&metadata.description),
            )
        };
        self.record_status(status);
    }

    /// Register a command implementation.
    pub fn register_command(&mut self, cmd: impl Command + 'static) {
        let Some(register_command) = self.callbacks.register_command else {
            self.record_status(AbiStatus::INVALID);
            return;
        };
        let boxed: Box<Box<dyn Command>> = Box::new(Box::new(cmd));
        let handle = PluginCommandHandle(Box::into_raw(boxed).cast::<c_void>());

        // SAFETY: `handle` was just created from a live boxed command and is
        // only borrowed here to read static command metadata for registration.
        let cmd_ref = unsafe { &*(handle.0.cast::<Box<dyn Command>>()) };
        let aliases: Vec<AbiStr> = cmd_ref
            .aliases()
            .iter()
            .map(|alias| AbiStr::from_borrowed(alias))
            .collect();
        let arg_hints: Vec<u8> = cmd_ref.arg_hints().iter().map(arg_hint_to_abi).collect();
        let descriptor = AbiCommandDescriptor {
            handle,
            vtable: AbiCommandVTable {
                execute: Some(plugin_command_execute),
                destroy: Some(plugin_command_destroy),
            },
            name: AbiStr::from_borrowed(cmd_ref.name()),
            description: AbiStr::from_borrowed(cmd_ref.description()),
            usage: AbiStr::from_borrowed(cmd_ref.usage()),
            arguments: AbiStr::from_borrowed(cmd_ref.arguments()),
            help: AbiStr::from_borrowed(cmd_ref.help()),
            aliases: AbiStrSlice {
                ptr: aliases.as_ptr(),
                len: aliases.len(),
            },
            arg_hints: AbiU8Slice {
                ptr: arg_hints.as_ptr(),
                len: arg_hints.len(),
            },
            runtime_requirements: cmd_ref.runtime_requirements().bits(),
        };

        // SAFETY: `descriptor` and its borrowed slices live until the callback
        // returns. The host copies the descriptor before returning.
        let status = unsafe { register_command(self.handle, &descriptor) };
        if !status.is_ok() {
            // SAFETY: The host did not accept ownership of this handle.
            let _ = unsafe { plugin_command_destroy(handle) };
        }
        self.record_status(status);
    }

    /// Register a declarative UI panel.
    pub fn register_panel(&mut self, panel: impl PluginPanel + 'static) {
        let Some(register_panel) = self.callbacks.register_panel else {
            self.record_status(AbiStatus::INVALID);
            return;
        };
        let boxed: Box<Box<dyn PluginPanel>> = Box::new(Box::new(panel));
        let handle = PluginPanelHandle(Box::into_raw(boxed).cast::<c_void>());
        // SAFETY: `handle` was just created from a live boxed panel and is
        // only borrowed here to read static panel metadata for registration.
        let panel_ref = unsafe { &*(handle.0.cast::<Box<dyn PluginPanel>>()) };
        let descriptor = panel_ref.descriptor();
        let placement = match descriptor.placement {
            PanelPlacement::Right => PANEL_PLACEMENT_RIGHT,
            PanelPlacement::Bottom => PANEL_PLACEMENT_BOTTOM,
        };
        let abi_descriptor = AbiPanelDescriptor {
            handle,
            vtable: AbiPanelVTable {
                snapshot: Some(plugin_panel_snapshot),
                handle_event: Some(plugin_panel_handle_event),
                destroy: Some(plugin_panel_destroy),
            },
            id: AbiStr::from_borrowed(&descriptor.id),
            title: AbiStr::from_borrowed(&descriptor.title),
            icon: AbiStr::from_borrowed(&descriptor.icon),
            placement,
            default_visible: u8::from(descriptor.default_visible),
            runtime_requirements: panel_ref.runtime_requirements().bits(),
        };
        // SAFETY: `abi_descriptor` borrows from `descriptor`, which lives until
        // the callback returns. The host copies all strings before returning.
        let status = unsafe { register_panel(self.handle, &abi_descriptor) };
        if !status.is_ok() {
            // SAFETY: The host did not accept ownership of this handle.
            let _ = unsafe { plugin_panel_destroy(handle) };
        }
        self.record_status(status);
    }

    /// Set a message handler for headless message processing.
    pub fn set_message_handler(&mut self, handler: impl MessageHandler + 'static) {
        let Some(register_message_handler) = self.callbacks.register_message_handler else {
            self.record_status(AbiStatus::INVALID);
            return;
        };
        let needs_poll = u8::from(handler.needs_poll());
        let boxed: Box<Box<dyn MessageHandler>> = Box::new(Box::new(handler));
        let handle = PluginMessageHandlerHandle(Box::into_raw(boxed).cast::<c_void>());
        let descriptor = AbiMessageHandlerDescriptor {
            handle,
            vtable: AbiMessageHandlerVTable {
                on_message: Some(plugin_message_on_message),
                needs_poll,
                poll: Some(plugin_message_poll),
                destroy: Some(plugin_message_destroy),
            },
        };
        // SAFETY: The descriptor is valid until the callback returns. The host
        // copies the handle and vtable before returning.
        let status = unsafe { register_message_handler(self.handle, &descriptor) };
        if !status.is_ok() {
            // SAFETY: The host did not accept ownership of this handle.
            let _ = unsafe { plugin_message_destroy(handle) };
        }
        self.record_status(status);
    }

    /// Register a script handler for a specific extension.
    ///
    /// Used by the builtin `run` command to dispatch non-.pml files
    /// to the appropriate plugin (e.g., `.py` → Python plugin).
    pub fn register_script_handler(
        &mut self,
        extension: &str,
        handler: impl Fn(&str) -> Result<(), String> + Send + Sync + 'static,
    ) {
        let Some(register_script_handler) = self.callbacks.register_script_handler else {
            self.record_status(AbiStatus::INVALID);
            return;
        };
        let boxed: Box<BoxedScriptHandler> = Box::new(Box::new(handler));
        let handle = PluginScriptHandlerHandle(Box::into_raw(boxed).cast::<c_void>());
        let descriptor = AbiScriptHandlerDescriptor {
            extension: AbiStr::from_borrowed(extension),
            handle,
            vtable: AbiScriptHandlerVTable {
                run: Some(plugin_script_run),
                destroy: Some(plugin_script_destroy),
            },
        };
        // SAFETY: The descriptor is valid until the callback returns. The host
        // copies strings, handle, and vtable before returning.
        let status = unsafe { register_script_handler(self.handle, &descriptor) };
        if !status.is_ok() {
            // SAFETY: The host did not accept ownership of this handle.
            let _ = unsafe { plugin_script_destroy(handle) };
        }
        self.record_status(status);
    }

    /// Register a file format handler for `load` and `save` commands.
    ///
    /// The handler specifies supported extensions and optional reader/writer
    /// factories. When a user runs `load file.ext` or `save file.ext` and
    /// the extension matches, the plugin's reader or writer is used.
    pub fn register_format_handler(&mut self, handler: FormatHandler) {
        let Some(register_format_handler) = self.callbacks.register_format_handler else {
            self.record_status(AbiStatus::INVALID);
            return;
        };
        let extensions: Vec<AbiStr> = handler
            .extensions
            .iter()
            .map(|extension| AbiStr::from_borrowed(extension))
            .collect();
        let name = handler.name.clone();
        let has_reader = handler.reader.is_some();
        let has_writer = handler.writer.is_some();
        let boxed = Box::new(handler);
        let handle = PluginFormatHandlerHandle(Box::into_raw(boxed).cast::<c_void>());
        let descriptor = AbiFormatHandlerDescriptor {
            name: AbiStr::from_borrowed(&name),
            extensions: AbiStrSlice {
                ptr: extensions.as_ptr(),
                len: extensions.len(),
            },
            handle,
            vtable: AbiFormatHandlerVTable {
                read: has_reader.then_some(plugin_format_read),
                write: has_writer.then_some(plugin_format_write),
                destroy: Some(plugin_format_destroy),
            },
        };
        // SAFETY: The descriptor and borrowed extension slice live until the
        // callback returns. The host copies everything it needs.
        let status = unsafe { register_format_handler(self.handle, &descriptor) };
        if !status.is_ok() {
            // SAFETY: The host did not accept ownership of this handle.
            let _ = unsafe { plugin_format_destroy(handle) };
        }
        self.record_status(status);
    }

    /// Register plugin settings with their shared store.
    ///
    /// The plugin creates a [`SharedSettingStore`], populates it with defaults,
    /// and passes the descriptors + store here. The host will dispatch
    /// `set`/`get`/`unset` commands to the store when a matching setting
    /// name is used.
    ///
    /// Use the [`define_plugin_settings!`](crate::define_plugin_settings) macro
    /// to generate descriptors and store initialization from a struct definition.
    pub fn register_settings(
        &mut self,
        descriptors: Vec<DynamicSettingDescriptor>,
        _store: SharedSettingStore,
    ) {
        let Some(register_setting) = self.callbacks.register_setting else {
            self.record_status(AbiStatus::INVALID);
            return;
        };

        for descriptor in descriptors {
            let default_value = setting_value_to_abi(&descriptor.default);
            let value_hints: Vec<AbiSettingValueHint> = descriptor
                .value_hints
                .iter()
                .map(|(name, value)| AbiSettingValueHint {
                    name: AbiStr::from_borrowed(name),
                    value: setting_value_to_abi(value),
                })
                .collect();
            let side_effects: Vec<u8> = descriptor
                .side_effects
                .iter()
                .copied()
                .map(side_effect_to_abi)
                .collect();
            let abi_descriptor = AbiSettingDescriptor {
                name: AbiStr::from_borrowed(&descriptor.name),
                setting_type: setting_type_to_abi(descriptor.setting_type),
                default_value,
                has_min: u8::from(descriptor.min.is_some()),
                min: descriptor.min.unwrap_or_default(),
                has_max: u8::from(descriptor.max.is_some()),
                max: descriptor.max.unwrap_or_default(),
                value_hints: AbiSettingValueHintSlice {
                    ptr: value_hints.as_ptr(),
                    len: value_hints.len(),
                },
                side_effects: AbiU8Slice {
                    ptr: side_effects.as_ptr(),
                    len: side_effects.len(),
                },
                object_overridable: u8::from(descriptor.object_overridable),
            };
            // SAFETY: `abi_descriptor` and its borrowed slices live until the
            // callback returns. The host copies all data before returning.
            let status = unsafe { register_setting(self.handle, &abi_descriptor) };
            self.record_status(status);
        }
    }

    /// Register a keyboard shortcut.
    ///
    /// Plugin hotkeys are checked after the main application bindings.
    /// For `Callback` actions, the closure runs during the next poll phase
    /// with access to [`PollContext`].
    pub fn register_hotkey(&mut self, key: impl Into<KeyBinding>, action: PluginKeyAction) {
        let Some(register_hotkey) = self.callbacks.register_hotkey else {
            self.record_status(AbiStatus::INVALID);
            return;
        };
        let key_text = key_binding_to_string(key.into());
        let Ok(action_bytes) = encode_hotkey_action(action) else {
            self.record_status(AbiStatus::UNSUPPORTED);
            return;
        };
        let descriptor = AbiHotkeyDescriptor {
            key: AbiStr::from_borrowed(&key_text),
            action: AbiU8Slice {
                ptr: action_bytes.as_ptr(),
                len: action_bytes.len(),
            },
            handle: PluginHotkeyHandle(std::ptr::null_mut()),
            vtable: AbiHotkeyVTable {
                invoke: None,
                destroy: None,
            },
        };
        // SAFETY: The descriptor and borrowed action bytes live until the
        // callback returns. The host copies them immediately.
        let status = unsafe { register_hotkey(self.handle, &descriptor) };
        self.record_status(status);
    }

    /// Final registration status.
    pub fn finish(&self) -> AbiStatus {
        self.status
    }

    fn record_status(&mut self, status: AbiStatus) {
        if self.status.is_ok() && !status.is_ok() {
            self.status = status;
        }
    }
}

unsafe extern "C" fn plugin_command_execute(
    handle: PluginCommandHandle,
    input: AbiU8Slice,
    runtime_callbacks: *const HostCommandRuntimeCallbacks,
    runtime_handle: HostCommandRuntimeHandle,
    sink: AbiBytesSinkFn,
    sink_user_data: *mut c_void,
) -> AbiStatus {
    runtime_status(|| {
        let input: WireCommandInput = decode_abi(input)?;
        validate_wire_version(input.wire_version)?;
        let command_runtime = CommandRuntimeClient::from_abi(runtime_callbacks, runtime_handle)?;
        let mut viewer = RuntimeViewer::from_session_bytes(
            &input.session,
            input.viewport_width,
            input.viewport_height,
            input.displayed_geometry,
            input.displayed_geometry_spool,
            command_runtime,
        )?;
        viewer.session.viewport_image = input.viewport_image;
        let dynamic_settings = wire::dynamic_registry_from_wire(&input.dynamic_settings)?;
        let (result, output, actions) = {
            let viewer_like: &mut dyn ViewerLike = &mut viewer;
            let mut ctx = CommandContext::new(viewer_like)
                .with_quiet(input.quiet)
                .with_dynamic_settings(&dynamic_settings);

            let result = with_command(handle, |command| {
                command
                    .execute(&mut ctx, &input.parsed)
                    .map_err(|error| error.to_string())
            })?;
            (result, ctx.take_output(), ctx.take_actions())
        };
        let output = WireCommandOutput {
            wire_version: RUNTIME_WIRE_VERSION,
            result,
            output,
            actions,
            session: wire::encode_session(viewer.session())?,
            viewport_image: viewer.viewport_image_ref().cloned(),
            viewport_image_changed: viewer.viewport_image_changed,
        };
        send_output(sink, sink_user_data, &output)
    })
}

unsafe extern "C" fn plugin_command_destroy(handle: PluginCommandHandle) -> AbiStatus {
    runtime_status(|| {
        if handle.0.is_null() {
            return Err("command handle was null".to_string());
        }
        // SAFETY: Command handles are created with `Box::into_raw` in
        // `register_command` and destroyed exactly once by the host.
        drop(unsafe { Box::from_raw(handle.0.cast::<Box<dyn Command>>()) });
        Ok(())
    })
}

unsafe extern "C" fn plugin_panel_snapshot(
    handle: PluginPanelHandle,
    input: AbiU8Slice,
    sink: AbiBytesSinkFn,
    sink_user_data: *mut c_void,
) -> AbiStatus {
    runtime_status(|| {
        let shared: crate::wire::WireSharedInput = decode_abi(input)?;
        let runtime = RuntimeShared::from_wire(shared)?;
        let snapshot =
            runtime.with_shared(|shared| with_panel(handle, |panel| panel.snapshot(&shared)))?;
        let output = WirePanelSnapshotOutput {
            wire_version: RUNTIME_WIRE_VERSION,
            snapshot,
        };
        send_output(sink, sink_user_data, &output)
    })
}

unsafe extern "C" fn plugin_panel_handle_event(
    handle: PluginPanelHandle,
    input: AbiU8Slice,
    sink: AbiBytesSinkFn,
    sink_user_data: *mut c_void,
) -> AbiStatus {
    runtime_status(|| {
        let input: WirePanelEventInput = decode_abi(input)?;
        let runtime = RuntimeShared::from_wire(input.shared)?;
        let mut bus = MessageBus::new();
        let actions = runtime.with_shared(|shared| {
            with_panel(handle, |panel| {
                panel.handle_event(input.event, &shared, &mut bus)
            })
        })?;
        let output = WirePanelEventOutput {
            wire_version: RUNTIME_WIRE_VERSION,
            actions,
        };
        send_output(sink, sink_user_data, &output)
    })
}

unsafe extern "C" fn plugin_panel_destroy(handle: PluginPanelHandle) -> AbiStatus {
    runtime_status(|| {
        if handle.0.is_null() {
            return Err("panel handle was null".to_string());
        }
        // SAFETY: Panel handles are created with `Box::into_raw` in
        // `register_panel` and destroyed exactly once by the host.
        drop(unsafe { Box::from_raw(handle.0.cast::<Box<dyn PluginPanel>>()) });
        Ok(())
    })
}

unsafe extern "C" fn plugin_message_on_message(
    handle: PluginMessageHandlerHandle,
    input: AbiU8Slice,
    sink: AbiBytesSinkFn,
    sink_user_data: *mut c_void,
) -> AbiStatus {
    runtime_status(|| {
        let input: WireMessageInput = decode_abi(input)?;
        validate_wire_version(input.wire_version)?;
        let mut bus = MessageBus::new();
        with_message_handler(handle, |handler| {
            handler.on_message(&input.message, &mut bus);
        })?;
        let output = WireMessageOutput {
            wire_version: RUNTIME_WIRE_VERSION,
            messages: bus.drain_outbox(),
        };
        send_output(sink, sink_user_data, &output)
    })
}

unsafe extern "C" fn plugin_message_poll(
    handle: PluginMessageHandlerHandle,
    input: AbiU8Slice,
    sink: AbiBytesSinkFn,
    sink_user_data: *mut c_void,
) -> AbiStatus {
    runtime_status(|| {
        let input: WirePollInput = decode_abi(input)?;
        let WirePollInput {
            shared,
            command_results,
            host_query_results,
            dynamic_invocations,
            plugin_dirs,
        } = input;
        validate_wire_version(shared.wire_version)?;
        let poll_shared = shared.clone();
        let runtime = RuntimeShared::from_poll_wire(shared)?;
        let mut bus = MessageBus::new();
        let plugin_dirs: Vec<PathBuf> = plugin_dirs.into_iter().map(PathBuf::from).collect();
        let command_results: Vec<CommandResult> = command_results
            .into_iter()
            .map(|result| CommandResult {
                id: result.id,
                result: result.result,
            })
            .collect();
        let mut exec_queue = Vec::new();
        let mut reg_queue = Vec::new();
        let mut unreg_queue = Vec::new();
        let mut notification_queue = Vec::new();
        let mut hotkey_reg_queue = Vec::new();
        let mut hotkey_unreg_queue = Vec::new();
        let mut mutation_queue = Vec::new();
        let mut host_query_queue = Vec::new();
        let mut viewer_action_queue = Vec::new();
        let mut panel_update_requested = false;

        runtime.with_shared(|shared| {
            let mut ctx = PollContext::new(
                &shared,
                &poll_shared,
                &mut bus,
                &command_results,
                &host_query_results,
                &dynamic_invocations,
                &[],
                &plugin_dirs,
                &mut exec_queue,
                &mut reg_queue,
                &mut unreg_queue,
                &mut notification_queue,
                &mut hotkey_reg_queue,
                &mut hotkey_unreg_queue,
                &mut mutation_queue,
                &mut host_query_queue,
                &mut viewer_action_queue,
                &mut panel_update_requested,
            );
            with_message_handler(handle, |handler| handler.poll(&mut ctx))
        })?;

        if !mutation_queue.is_empty() {
            notification_queue.push(
                "dynamic plugin queued Rust-only viewer mutations; use portable viewer actions"
                    .to_string(),
            );
        }
        if panel_update_requested
            && !viewer_action_queue
                .iter()
                .any(|action| matches!(action, WireViewerAction::RequestPanelUpdate))
        {
            viewer_action_queue.push(WireViewerAction::RequestPanelUpdate);
        }

        let hotkey_registrations = hotkey_reg_queue
            .into_iter()
            .filter_map(|(key, action)| match wire_hotkey_action(action) {
                Ok(action) => Some(WireHotkeyRegistration { key, action }),
                Err(error) => {
                    notification_queue.push(error);
                    None
                }
            })
            .collect();

        let output = WirePollOutput {
            wire_version: RUNTIME_WIRE_VERSION,
            messages: bus.drain_outbox(),
            command_exec: exec_queue
                .into_iter()
                .map(|request| crate::wire::WireCommandExecRequest {
                    id: request.id,
                    command: request.command,
                    silent: request.silent,
                })
                .collect(),
            dynamic_registrations: reg_queue
                .into_iter()
                .map(|registration| WireDynCmdRegistration {
                    name: registration.name,
                    description: registration.description,
                    usage: registration.usage,
                    arguments: registration.arguments,
                })
                .collect(),
            dynamic_unregistrations: unreg_queue,
            notifications: notification_queue,
            hotkey_registrations,
            hotkey_unregistrations: hotkey_unreg_queue,
            host_queries: host_query_queue,
            viewer_actions: viewer_action_queue,
        };
        send_output(sink, sink_user_data, &output)
    })
}

unsafe extern "C" fn plugin_message_destroy(handle: PluginMessageHandlerHandle) -> AbiStatus {
    runtime_status(|| {
        if handle.0.is_null() {
            return Err("message handler handle was null".to_string());
        }
        // SAFETY: Message handler handles are created with `Box::into_raw` in
        // `set_message_handler` and destroyed exactly once by the host.
        drop(unsafe { Box::from_raw(handle.0.cast::<Box<dyn MessageHandler>>()) });
        Ok(())
    })
}

unsafe extern "C" fn plugin_script_run(
    handle: PluginScriptHandlerHandle,
    input: AbiU8Slice,
    sink: AbiBytesSinkFn,
    sink_user_data: *mut c_void,
) -> AbiStatus {
    runtime_status(|| {
        let input: WireScriptInput = decode_abi(input)?;
        validate_wire_version(input.wire_version)?;
        let result = with_script_handler(handle, |handler| handler(&input.path))?;
        let output = WireScriptOutput {
            wire_version: RUNTIME_WIRE_VERSION,
            result,
        };
        send_output(sink, sink_user_data, &output)
    })
}

unsafe extern "C" fn plugin_script_destroy(handle: PluginScriptHandlerHandle) -> AbiStatus {
    runtime_status(|| {
        if handle.0.is_null() {
            return Err("script handler handle was null".to_string());
        }
        // SAFETY: Script handler handles are created with `Box::into_raw` in
        // `register_script_handler` and destroyed exactly once by the host.
        drop(unsafe { Box::from_raw(handle.0.cast::<BoxedScriptHandler>()) });
        Ok(())
    })
}

unsafe extern "C" fn plugin_format_read(
    handle: PluginFormatHandlerHandle,
    input: AbiU8Slice,
    sink: AbiBytesSinkFn,
    sink_user_data: *mut c_void,
) -> AbiStatus {
    runtime_status(|| {
        let input: WireFormatReadInput = decode_abi(input)?;
        validate_wire_version(input.wire_version)?;
        let result = with_format_handler(handle, |handler| {
            let Some(reader) = &handler.reader else {
                return Err("format handler does not support reading".to_string());
            };
            reader(Box::new(Cursor::new(input.bytes)))
        })?;
        let output = WireFormatReadOutput {
            wire_version: RUNTIME_WIRE_VERSION,
            result,
        };
        send_output(sink, sink_user_data, &output)
    })
}

unsafe extern "C" fn plugin_format_write(
    handle: PluginFormatHandlerHandle,
    input: AbiU8Slice,
    sink: AbiBytesSinkFn,
    sink_user_data: *mut c_void,
) -> AbiStatus {
    runtime_status(|| {
        let input: WireFormatWriteInput = decode_abi(input)?;
        validate_wire_version(input.wire_version)?;
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let writer = SharedVecWriter {
            bytes: bytes.clone(),
        };
        let result = with_format_handler(handle, |handler| {
            let Some(writer_fn) = &handler.writer else {
                return Err("format handler does not support writing".to_string());
            };
            writer_fn(Box::new(writer), &input.molecules)
        })?;
        let result = result.and_then(|()| {
            bytes
                .lock()
                .map(|bytes| bytes.clone())
                .map_err(|_| "format writer output buffer was poisoned".to_string())
        });
        let output = WireFormatWriteOutput {
            wire_version: RUNTIME_WIRE_VERSION,
            result,
        };
        send_output(sink, sink_user_data, &output)
    })
}

unsafe extern "C" fn plugin_format_destroy(handle: PluginFormatHandlerHandle) -> AbiStatus {
    runtime_status(|| {
        if handle.0.is_null() {
            return Err("format handler handle was null".to_string());
        }
        // SAFETY: Format handler handles are created with `Box::into_raw` in
        // `register_format_handler` and destroyed exactly once by the host.
        drop(unsafe { Box::from_raw(handle.0.cast::<FormatHandler>()) });
        Ok(())
    })
}

fn with_command<T>(
    handle: PluginCommandHandle,
    f: impl FnOnce(&dyn Command) -> T,
) -> Result<T, String> {
    if handle.0.is_null() {
        return Err("command handle was null".to_string());
    }
    // SAFETY: The host only passes handles created by `register_command`.
    let command = unsafe { &*(handle.0.cast::<Box<dyn Command>>()) };
    Ok(f(command.as_ref()))
}

fn with_panel<T>(
    handle: PluginPanelHandle,
    f: impl FnOnce(&mut dyn PluginPanel) -> T,
) -> Result<T, String> {
    if handle.0.is_null() {
        return Err("panel handle was null".to_string());
    }
    // SAFETY: The host only passes handles created by `register_panel`.
    let panel = unsafe { &mut *(handle.0.cast::<Box<dyn PluginPanel>>()) };
    Ok(f(panel.as_mut()))
}

fn with_message_handler<T>(
    handle: PluginMessageHandlerHandle,
    f: impl FnOnce(&mut dyn MessageHandler) -> T,
) -> Result<T, String> {
    if handle.0.is_null() {
        return Err("message handler handle was null".to_string());
    }
    // SAFETY: The host only passes handles created by `set_message_handler`.
    let handler = unsafe { &mut *(handle.0.cast::<Box<dyn MessageHandler>>()) };
    Ok(f(handler.as_mut()))
}

fn with_script_handler<T>(
    handle: PluginScriptHandlerHandle,
    f: impl FnOnce(&(dyn Fn(&str) -> Result<(), String> + Send + Sync)) -> T,
) -> Result<T, String> {
    if handle.0.is_null() {
        return Err("script handler handle was null".to_string());
    }
    // SAFETY: The host only passes handles created by `register_script_handler`.
    let handler = unsafe { &*(handle.0.cast::<BoxedScriptHandler>()) };
    Ok(f(handler.as_ref()))
}

fn with_format_handler<T>(
    handle: PluginFormatHandlerHandle,
    f: impl FnOnce(&FormatHandler) -> T,
) -> Result<T, String> {
    if handle.0.is_null() {
        return Err("format handler handle was null".to_string());
    }
    // SAFETY: The host only passes handles created by `register_format_handler`.
    let handler = unsafe { &*(handle.0.cast::<FormatHandler>()) };
    Ok(f(handler))
}

fn runtime_status(f: impl FnOnce() -> Result<(), String>) -> AbiStatus {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(())) => AbiStatus::OK,
        Ok(Err(error)) => {
            log::warn!("Plugin runtime callback failed: {}", error);
            AbiStatus::HOST_ERROR
        }
        Err(_) => AbiStatus::PANIC,
    }
}

fn decode_abi<T: serde::de::DeserializeOwned>(value: AbiU8Slice) -> Result<T, String> {
    if value.len == 0 {
        return wire::decode(&[]);
    }
    if value.ptr.is_null() {
        return Err("runtime payload pointer was null".to_string());
    }
    if value.len > wire::MAX_WIRE_PAYLOAD_LEN {
        return Err("runtime payload exceeds ABI limit".to_string());
    }
    // SAFETY: The ABI contract requires non-null byte slices to point to
    // initialized memory for the duration of this callback.
    let bytes = unsafe { std::slice::from_raw_parts(value.ptr, value.len) };
    wire::decode(bytes)
}

fn send_output<T: serde::Serialize>(
    sink: AbiBytesSinkFn,
    sink_user_data: *mut c_void,
    output: &T,
) -> Result<(), String> {
    let bytes = wire::encode(output)?;
    let slice = AbiU8Slice {
        ptr: bytes.as_ptr(),
        len: bytes.len(),
    };
    // SAFETY: `slice` points to `bytes`, which lives until the sink returns.
    // The host sink copies the bytes before returning.
    let status = unsafe { sink(sink_user_data, slice) };
    if status.is_ok() {
        Ok(())
    } else {
        Err(format!("host byte sink returned status {}", status.code))
    }
}

fn validate_wire_version(version: u32) -> Result<(), String> {
    if version == RUNTIME_WIRE_VERSION {
        Ok(())
    } else {
        Err(format!(
            "runtime wire version mismatch: plugin got {}, expected {}",
            version, RUNTIME_WIRE_VERSION
        ))
    }
}

fn encode_hotkey_action(action: PluginKeyAction) -> Result<Vec<u8>, String> {
    wire::encode(&wire_hotkey_action(action)?)
}

fn wire_hotkey_action(action: PluginKeyAction) -> Result<WireHotkeyAction, String> {
    match action {
        PluginKeyAction::Command(command) => Ok(WireHotkeyAction::Command(command)),
        PluginKeyAction::DynamicCommand { name, args } => {
            Ok(WireHotkeyAction::DynamicCommand { name, args })
        }
        PluginKeyAction::Custom { topic, payload } => {
            Ok(WireHotkeyAction::Custom { topic, payload })
        }
        PluginKeyAction::Callback(_) => Err(
            "plugin hotkey callback actions require explicit portable callback handles".to_string(),
        ),
    }
}

fn key_binding_to_string(binding: KeyBinding) -> String {
    let mut parts = Vec::new();
    if binding.ctrl {
        parts.push("ctrl".to_string());
    }
    if binding.shift {
        parts.push("shift".to_string());
    }
    if binding.alt {
        parts.push("alt".to_string());
    }
    parts.push(format!("{:?}", binding.key).to_lowercase());
    parts.join("+")
}

struct SharedVecWriter {
    bytes: Arc<Mutex<Vec<u8>>>,
}

impl Write for SharedVecWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut bytes = self
            .bytes
            .lock()
            .map_err(|_| std::io::Error::other("format output buffer poisoned"))?;
        bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct CommandRuntimeClient {
    callbacks: *const HostCommandRuntimeCallbacks,
    handle: HostCommandRuntimeHandle,
}

impl CommandRuntimeClient {
    fn from_abi(
        callbacks: *const HostCommandRuntimeCallbacks,
        handle: HostCommandRuntimeHandle,
    ) -> Result<Option<Self>, String> {
        if callbacks.is_null() {
            return Ok(None);
        }

        // SAFETY: The host passes a callback table that must remain valid for
        // the duration of the command execution call.
        let table = unsafe { &*callbacks };
        if table.table_version != HOST_COMMAND_RUNTIME_CALLBACKS_VERSION {
            return Err(format!(
                "host command runtime callback version mismatch: got {}, expected {}",
                table.table_version, HOST_COMMAND_RUNTIME_CALLBACKS_VERSION
            ));
        }
        if table.request.is_none() {
            return Ok(None);
        }

        Ok(Some(Self { callbacks, handle }))
    }

    fn request(
        self,
        request: &WireCommandRuntimeRequest,
    ) -> Result<WireCommandRuntimeResponse, String> {
        // SAFETY: `CommandRuntimeClient` is only created after validating the
        // callback table pointer for the active command execution call.
        let table = unsafe { &*self.callbacks };
        let request_fn = table
            .request
            .ok_or_else(|| "host command runtime request callback was null".to_string())?;
        let input_bytes = wire::encode(request)?;
        let input = AbiU8Slice {
            ptr: input_bytes.as_ptr(),
            len: input_bytes.len(),
        };
        let mut output = RuntimeBytes::default();
        let output_ptr = (&mut output as *mut RuntimeBytes).cast::<c_void>();
        // SAFETY: The input bytes live until the callback returns, and the
        // output sink copies host-owned bytes into `output`.
        let status = unsafe { request_fn(self.handle, input, runtime_bytes_sink, output_ptr) };
        if !status.is_ok() {
            return Err(format!(
                "host command runtime request returned status {}",
                status.code
            ));
        }
        let response: WireCommandRuntimeResponse = wire::decode(&output.bytes)?;
        validate_wire_version(response.wire_version)?;
        Ok(response)
    }
}

impl CommandRuntimeClient {
    fn for_each_trace_geometry_chunk(
        self,
        visitor: &mut dyn FnMut(patinae_render::TraceGeometryChunk) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut next_id = 1u64;
        let open =
            self.request(&WireCommandRuntimeRequest::OpenTraceGeometryStream { id: next_id })?;
        validate_runtime_response_id(&open, next_id)?;
        let stream_id = match open.result? {
            WireCommandRuntimeValue::TraceGeometryOpened(opened) => opened.stream_id,
            _ => return Err("host returned unexpected trace stream open response".to_string()),
        };
        next_id += 1;

        let result = loop {
            let response = self.request(&WireCommandRuntimeRequest::ReadTraceGeometryStream {
                id: next_id,
                stream_id,
            })?;
            validate_runtime_response_id(&response, next_id)?;
            next_id += 1;
            match response.result? {
                WireCommandRuntimeValue::TraceGeometryChunk(Some(chunk)) => {
                    visitor(wire::trace_geometry_chunk_from_wire(chunk))?;
                }
                WireCommandRuntimeValue::TraceGeometryChunkBytes(Some(bytes)) => {
                    let chunk = wire::decode(&bytes)?;
                    visitor(wire::trace_geometry_chunk_from_wire(chunk))?;
                }
                WireCommandRuntimeValue::TraceGeometryChunk(None) => break Ok(()),
                WireCommandRuntimeValue::TraceGeometryChunkBytes(None) => break Ok(()),
                _ => break Err("host returned unexpected trace stream read response".to_string()),
            }
        };

        let close = self.request(&WireCommandRuntimeRequest::CloseTraceGeometryStream {
            id: next_id,
            stream_id,
        });
        match (result, close) {
            (Ok(()), Ok(response)) => {
                validate_runtime_response_id(&response, next_id)?;
                match response.result? {
                    WireCommandRuntimeValue::TraceGeometryClosed => Ok(()),
                    _ => Err("host returned unexpected trace stream close response".to_string()),
                }
            }
            (Err(error), Ok(_)) => Err(error),
            (Ok(()), Err(error)) => Err(error),
            (Err(error), Err(close_error)) => Err(format!("{error}; close failed: {close_error}")),
        }
    }
}

impl CommandRuntimeClient {
    fn open_render_artifact_snapshot(
        self,
    ) -> Result<patinae_scene::RenderArtifactSnapshotDescriptor, String> {
        let response =
            self.request(&WireCommandRuntimeRequest::OpenRenderArtifactSnapshot { id: 1 })?;
        validate_runtime_response_id(&response, 1)?;
        match response.result? {
            WireCommandRuntimeValue::RenderArtifactSnapshotOpened(snapshot) => Ok(snapshot),
            _ => Err("host returned unexpected render artifact snapshot response".to_string()),
        }
    }

    fn close_render_artifact_snapshot(self, snapshot_id: u64) -> Result<(), String> {
        let response = self.request(&WireCommandRuntimeRequest::CloseRenderArtifactSnapshot {
            id: 1,
            snapshot_id,
        })?;
        validate_runtime_response_id(&response, 1)?;
        match response.result? {
            WireCommandRuntimeValue::RenderArtifactSnapshotClosed => Ok(()),
            _ => Err("host returned unexpected render artifact close response".to_string()),
        }
    }
}

impl CommandRuntimeClient {
    fn gpu_device_limits(self) -> Result<patinae_scene::GpuDeviceLimits, String> {
        let response = self.request(&WireCommandRuntimeRequest::GpuDeviceLimits { id: 1 })?;
        validate_runtime_response_id(&response, 1)?;
        match response.result? {
            WireCommandRuntimeValue::GpuDeviceLimits(limits) => Ok(limits),
            _ => Err("host returned unexpected GPU limits response".to_string()),
        }
    }

    fn gpu_create_buffer(
        self,
        descriptor: patinae_scene::GpuBufferDescriptor,
        initial_data: Option<Vec<u8>>,
    ) -> Result<patinae_scene::GpuHandle, String> {
        self.gpu_handle_response(WireCommandRuntimeRequest::GpuCreateBuffer {
            id: 1,
            descriptor,
            initial_data,
        })
    }

    fn gpu_create_texture(
        self,
        descriptor: patinae_scene::GpuTextureDescriptor,
    ) -> Result<patinae_scene::GpuHandle, String> {
        self.gpu_handle_response(WireCommandRuntimeRequest::GpuCreateTexture { id: 1, descriptor })
    }

    fn gpu_create_texture_view(
        self,
        descriptor: patinae_scene::GpuTextureViewDescriptor,
    ) -> Result<patinae_scene::GpuHandle, String> {
        self.gpu_handle_response(WireCommandRuntimeRequest::GpuCreateTextureView {
            id: 1,
            descriptor,
        })
    }

    fn gpu_create_sampler(
        self,
        descriptor: patinae_scene::GpuSamplerDescriptor,
    ) -> Result<patinae_scene::GpuHandle, String> {
        self.gpu_handle_response(WireCommandRuntimeRequest::GpuCreateSampler { id: 1, descriptor })
    }

    fn gpu_write_buffer(
        self,
        buffer: patinae_scene::GpuHandle,
        offset: u64,
        data: Vec<u8>,
    ) -> Result<(), String> {
        self.gpu_ok_response(WireCommandRuntimeRequest::GpuWriteBuffer {
            id: 1,
            buffer,
            offset,
            data,
        })
    }

    fn gpu_copy_buffer_to_buffer(
        self,
        source: patinae_scene::GpuHandle,
        source_offset: u64,
        destination: patinae_scene::GpuHandle,
        destination_offset: u64,
        size: u64,
    ) -> Result<(), String> {
        self.gpu_ok_response(WireCommandRuntimeRequest::GpuCopyBufferToBuffer {
            id: 1,
            source,
            source_offset,
            destination,
            destination_offset,
            size,
        })
    }

    fn gpu_read_buffer(
        self,
        buffer: patinae_scene::GpuHandle,
        offset: u64,
        size: u64,
    ) -> Result<Vec<u8>, String> {
        let response = self.request(&WireCommandRuntimeRequest::GpuReadBuffer {
            id: 1,
            buffer,
            offset,
            size,
        })?;
        validate_runtime_response_id(&response, 1)?;
        match response.result? {
            WireCommandRuntimeValue::GpuBytes(bytes) => Ok(bytes),
            _ => Err("host returned unexpected GPU read response".to_string()),
        }
    }

    fn gpu_create_shader_module(
        self,
        descriptor: patinae_scene::GpuShaderModuleDescriptor,
    ) -> Result<patinae_scene::GpuHandle, String> {
        self.gpu_handle_response(WireCommandRuntimeRequest::GpuCreateShaderModule {
            id: 1,
            descriptor,
        })
    }

    fn gpu_create_bind_group_layout(
        self,
        descriptor: patinae_scene::GpuBindGroupLayoutDescriptor,
    ) -> Result<patinae_scene::GpuHandle, String> {
        self.gpu_handle_response(WireCommandRuntimeRequest::GpuCreateBindGroupLayout {
            id: 1,
            descriptor,
        })
    }

    fn gpu_create_pipeline_layout(
        self,
        descriptor: patinae_scene::GpuPipelineLayoutDescriptor,
    ) -> Result<patinae_scene::GpuHandle, String> {
        self.gpu_handle_response(WireCommandRuntimeRequest::GpuCreatePipelineLayout {
            id: 1,
            descriptor,
        })
    }

    fn gpu_create_compute_pipeline(
        self,
        descriptor: patinae_scene::GpuComputePipelineDescriptor,
    ) -> Result<patinae_scene::GpuHandle, String> {
        self.gpu_handle_response(WireCommandRuntimeRequest::GpuCreateComputePipeline {
            id: 1,
            descriptor,
        })
    }

    fn gpu_create_render_pipeline(
        self,
        descriptor: patinae_scene::GpuRenderPipelineDescriptor,
    ) -> Result<patinae_scene::GpuHandle, String> {
        self.gpu_handle_response(WireCommandRuntimeRequest::GpuCreateRenderPipeline {
            id: 1,
            descriptor,
        })
    }
}

impl CommandRuntimeClient {
    fn gpu_create_cached_shader_module(
        self,
        descriptor: patinae_scene::GpuShaderModuleDescriptor,
    ) -> Result<patinae_scene::GpuCachedHandle, String> {
        self.gpu_cached_handle_response(WireCommandRuntimeRequest::GpuCreateCachedShaderModule {
            id: 1,
            descriptor,
        })
    }

    fn gpu_create_cached_bind_group_layout(
        self,
        descriptor: patinae_scene::GpuBindGroupLayoutDescriptor,
    ) -> Result<patinae_scene::GpuCachedHandle, String> {
        self.gpu_cached_handle_response(WireCommandRuntimeRequest::GpuCreateCachedBindGroupLayout {
            id: 1,
            descriptor,
        })
    }

    fn gpu_create_cached_pipeline_layout(
        self,
        descriptor: patinae_scene::GpuPipelineLayoutDescriptor,
    ) -> Result<patinae_scene::GpuCachedHandle, String> {
        self.gpu_cached_handle_response(WireCommandRuntimeRequest::GpuCreateCachedPipelineLayout {
            id: 1,
            descriptor,
        })
    }

    fn gpu_create_cached_compute_pipeline(
        self,
        descriptor: patinae_scene::GpuComputePipelineDescriptor,
    ) -> Result<patinae_scene::GpuCachedHandle, String> {
        self.gpu_cached_handle_response(WireCommandRuntimeRequest::GpuCreateCachedComputePipeline {
            id: 1,
            descriptor,
        })
    }

    fn gpu_create_cached_render_pipeline(
        self,
        descriptor: patinae_scene::GpuRenderPipelineDescriptor,
    ) -> Result<patinae_scene::GpuCachedHandle, String> {
        self.gpu_cached_handle_response(WireCommandRuntimeRequest::GpuCreateCachedRenderPipeline {
            id: 1,
            descriptor,
        })
    }

    fn gpu_cache_stats(self) -> Result<patinae_scene::GpuCacheStats, String> {
        let response = self.request(&WireCommandRuntimeRequest::GpuCacheStats { id: 1 })?;
        validate_runtime_response_id(&response, 1)?;
        match response.result? {
            WireCommandRuntimeValue::GpuCacheStats(stats) => Ok(stats),
            _ => Err("host returned unexpected GPU cache stats response".to_string()),
        }
    }

    fn gpu_drop_plugin_cache(self) -> Result<(), String> {
        self.gpu_ok_response(WireCommandRuntimeRequest::GpuDropPluginCache { id: 1 })
    }
}

impl CommandRuntimeClient {
    fn gpu_create_bind_group(
        self,
        descriptor: patinae_scene::GpuBindGroupDescriptor,
    ) -> Result<patinae_scene::GpuHandle, String> {
        self.gpu_handle_response(WireCommandRuntimeRequest::GpuCreateBindGroup {
            id: 1,
            descriptor,
        })
    }

    fn gpu_dispatch_compute(
        self,
        pipeline: patinae_scene::GpuHandle,
        bind_groups: Vec<patinae_scene::GpuHandle>,
        workgroups: [u32; 3],
    ) -> Result<(), String> {
        self.gpu_ok_response(WireCommandRuntimeRequest::GpuDispatchCompute {
            id: 1,
            pipeline,
            bind_groups,
            workgroups,
        })
    }

    fn gpu_submit_batch(
        self,
        batch: patinae_scene::GpuSubmitBatch,
    ) -> Result<patinae_scene::GpuBatchResult, String> {
        let response = self.request(&WireCommandRuntimeRequest::GpuSubmitBatch { id: 1, batch })?;
        validate_runtime_response_id(&response, 1)?;
        match response.result? {
            WireCommandRuntimeValue::GpuBatchResult(result) => Ok(result),
            _ => Err("host returned unexpected GPU batch response".to_string()),
        }
    }

    fn set_viewport_gpu_image_from_buffer(
        self,
        buffer: patinae_scene::GpuHandle,
        width: u32,
        height: u32,
    ) -> Result<(), String> {
        self.gpu_ok_response(WireCommandRuntimeRequest::SetViewportGpuImageFromBuffer {
            id: 1,
            buffer,
            width,
            height,
        })
    }

    fn gpu_drop_handles(self, handles: Vec<patinae_scene::GpuHandle>) -> Result<(), String> {
        self.gpu_ok_response(WireCommandRuntimeRequest::GpuDropHandles { id: 1, handles })
    }
}

impl CommandRuntimeClient {
    fn gpu_handle_response(
        self,
        request: WireCommandRuntimeRequest,
    ) -> Result<patinae_scene::GpuHandle, String> {
        let response = self.request(&request)?;
        validate_runtime_response_id(&response, 1)?;
        match response.result? {
            WireCommandRuntimeValue::GpuHandle(handle) => Ok(handle),
            _ => Err("host returned unexpected GPU handle response".to_string()),
        }
    }

    fn gpu_cached_handle_response(
        self,
        request: WireCommandRuntimeRequest,
    ) -> Result<patinae_scene::GpuCachedHandle, String> {
        let response = self.request(&request)?;
        validate_runtime_response_id(&response, 1)?;
        match response.result? {
            WireCommandRuntimeValue::GpuCachedHandle(handle) => Ok(handle),
            _ => Err("host returned unexpected GPU cached handle response".to_string()),
        }
    }

    fn gpu_ok_response(self, request: WireCommandRuntimeRequest) -> Result<(), String> {
        let response = self.request(&request)?;
        validate_runtime_response_id(&response, 1)?;
        match response.result? {
            WireCommandRuntimeValue::GpuOk => Ok(()),
            _ => Err("host returned unexpected GPU ok response".to_string()),
        }
    }
}

#[derive(Default)]
struct RuntimeBytes {
    bytes: Vec<u8>,
}

unsafe extern "C" fn runtime_bytes_sink(user_data: *mut c_void, bytes: AbiU8Slice) -> AbiStatus {
    if user_data.is_null() {
        return AbiStatus::INVALID;
    }
    let copied = match copy_abi_bytes(bytes) {
        Ok(bytes) => bytes,
        Err(error) => {
            log::warn!(
                "Plugin command runtime byte sink rejected output: {}",
                error
            );
            return AbiStatus::INVALID;
        }
    };
    // SAFETY: The caller provides a valid `RuntimeBytes` pointer for the
    // duration of this sink call. Null was checked above.
    unsafe { &mut *(user_data.cast::<RuntimeBytes>()) }.bytes = copied;
    AbiStatus::OK
}

fn copy_abi_bytes(bytes: AbiU8Slice) -> Result<Vec<u8>, String> {
    if bytes.len == 0 {
        return Ok(Vec::new());
    }
    if bytes.ptr.is_null() {
        return Err("runtime response pointer was null".to_string());
    }
    if bytes.len > wire::MAX_WIRE_PAYLOAD_LEN {
        return Err("runtime response exceeds ABI limit".to_string());
    }
    // SAFETY: The host callback must provide initialized bytes for the
    // duration of the sink call. Null and length were checked above.
    Ok(unsafe { std::slice::from_raw_parts(bytes.ptr, bytes.len) }.to_vec())
}

fn validate_runtime_response_id(
    response: &WireCommandRuntimeResponse,
    expected: u64,
) -> Result<(), String> {
    if response.id == expected {
        Ok(())
    } else {
        Err(format!(
            "host command runtime response id mismatch: got {}, expected {}",
            response.id, expected
        ))
    }
}

struct RuntimeViewer {
    session: patinae_scene::Session,
    viewport_size: (u32, u32),
    displayed_geometry: Option<crate::wire::WireDisplayedGeometry>,
    displayed_geometry_spool: Option<crate::wire::WireDisplayedGeometrySpool>,
    command_runtime: Option<CommandRuntimeClient>,
    redraw_requested: bool,
    viewport_image_changed: bool,
}

impl RuntimeViewer {
    fn from_session_bytes(
        bytes: &[u8],
        width: u32,
        height: u32,
        displayed_geometry: Option<crate::wire::WireDisplayedGeometry>,
        displayed_geometry_spool: Option<crate::wire::WireDisplayedGeometrySpool>,
        command_runtime: Option<CommandRuntimeClient>,
    ) -> Result<Self, String> {
        if displayed_geometry.is_some() && displayed_geometry_spool.is_some() {
            return Err("displayed geometry was supplied twice".to_string());
        }
        Ok(Self {
            session: wire::decode_session(bytes)?,
            viewport_size: (width, height),
            displayed_geometry,
            displayed_geometry_spool,
            command_runtime,
            redraw_requested: false,
            viewport_image_changed: false,
        })
    }
}

impl ViewerLike for RuntimeViewer {
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

    fn request_redraw(&mut self) {
        self.redraw_requested = true;
    }

    fn session(&self) -> &patinae_scene::Session {
        &self.session
    }

    fn session_mut(&mut self) -> &mut patinae_scene::Session {
        &mut self.session
    }

    fn replace_session(&mut self, session: patinae_scene::Session) {
        self.session = session;
        self.redraw_requested = true;
        self.viewport_image_changed = true;
    }

    fn scene_store(&mut self, key: &str, storemask: u32) {
        let mask = patinae_scene::SceneStoreMask::from_bits_truncate(storemask);
        self.session
            .scenes
            .store(key, mask, &self.session.camera, &self.session.registry);
        self.redraw_requested = true;
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
            .map_err(|error| error.to_string())?;
        self.redraw_requested = true;
        Ok(())
    }

    fn view_recall(&mut self, key: &str, animate: f32) -> Result<(), String> {
        self.session
            .views
            .recall(key, &mut self.session.camera, animate)
            .map_err(|error| error.to_string())?;
        self.redraw_requested = true;
        Ok(())
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

    fn views(&self) -> &patinae_scene::ViewManager {
        &self.session.views
    }

    fn views_mut(&mut self) -> &mut patinae_scene::ViewManager {
        &mut self.session.views
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
        self.session.clear_color_set = true;
        self.redraw_requested = true;
    }

    fn viewport_image_ref(&self) -> Option<&patinae_scene::ViewportImage> {
        self.session.viewport_image.as_ref()
    }

    fn set_viewport_image_internal(&mut self, image: Option<patinae_scene::ViewportImage>) {
        self.session.viewport_image = image;
        self.redraw_requested = true;
        self.viewport_image_changed = true;
    }

    fn viewport_size(&self) -> (u32, u32) {
        self.viewport_size
    }

    fn export_displayed_geometry(
        &mut self,
        _options: &patinae_render::GeometryExportOptions,
    ) -> Result<patinae_render::DisplayedGeometry, String> {
        if let Some(displayed_geometry) = self.displayed_geometry.clone() {
            return Ok(wire::displayed_geometry_from_wire(displayed_geometry));
        }
        let Some(spool) = &self.displayed_geometry_spool else {
            return Err("displayed geometry was not supplied by the host".to_string());
        };

        let mut displayed = patinae_render::DisplayedGeometry::default();
        wire::for_each_displayed_geometry_spool_chunk(spool, |chunk| {
            displayed.objects.extend(chunk.objects);
            Ok(())
        })?;
        Ok(displayed)
    }

    fn for_each_displayed_geometry_chunk(
        &mut self,
        _options: &patinae_render::GeometryExportOptions,
        visitor: &mut dyn FnMut(patinae_render::DisplayedGeometry) -> Result<(), String>,
    ) -> Result<(), String> {
        if let Some(displayed_geometry) = self.displayed_geometry.clone() {
            return visitor(wire::displayed_geometry_from_wire(displayed_geometry));
        }
        let Some(spool) = &self.displayed_geometry_spool else {
            return Err("displayed geometry was not supplied by the host".to_string());
        };
        wire::for_each_displayed_geometry_spool_chunk(spool, visitor)
    }

    fn for_each_trace_geometry_chunk(
        &mut self,
        options: &patinae_render::GeometryExportOptions,
        visitor: &mut dyn FnMut(patinae_render::TraceGeometryChunk) -> Result<(), String>,
    ) -> Result<(), String> {
        if let Some(command_runtime) = self.command_runtime {
            return command_runtime.for_each_trace_geometry_chunk(visitor);
        }
        self.for_each_displayed_geometry_chunk(options, &mut |displayed| {
            visitor(patinae_render::TraceGeometryChunk::from_displayed(
                &displayed,
            ))
        })
    }

    fn open_render_artifact_snapshot(
        &mut self,
    ) -> Result<patinae_scene::RenderArtifactSnapshotDescriptor, String> {
        self.command_runtime
            .ok_or_else(|| "host command runtime is not available".to_string())?
            .open_render_artifact_snapshot()
    }

    fn close_render_artifact_snapshot(&mut self, snapshot_id: u64) -> Result<(), String> {
        self.command_runtime
            .ok_or_else(|| "host command runtime is not available".to_string())?
            .close_render_artifact_snapshot(snapshot_id)
    }

    fn gpu_device_limits_for_plugins(&mut self) -> Result<patinae_scene::GpuDeviceLimits, String> {
        self.command_runtime
            .ok_or_else(|| "host command runtime is not available".to_string())?
            .gpu_device_limits()
    }

    fn gpu_create_buffer(
        &mut self,
        descriptor: patinae_scene::GpuBufferDescriptor,
        initial_data: Option<Vec<u8>>,
    ) -> Result<patinae_scene::GpuHandle, String> {
        self.command_runtime
            .ok_or_else(|| "host command runtime is not available".to_string())?
            .gpu_create_buffer(descriptor, initial_data)
    }

    fn gpu_create_texture(
        &mut self,
        descriptor: patinae_scene::GpuTextureDescriptor,
    ) -> Result<patinae_scene::GpuHandle, String> {
        self.command_runtime
            .ok_or_else(|| "host command runtime is not available".to_string())?
            .gpu_create_texture(descriptor)
    }

    fn gpu_create_texture_view(
        &mut self,
        descriptor: patinae_scene::GpuTextureViewDescriptor,
    ) -> Result<patinae_scene::GpuHandle, String> {
        self.command_runtime
            .ok_or_else(|| "host command runtime is not available".to_string())?
            .gpu_create_texture_view(descriptor)
    }

    fn gpu_create_sampler(
        &mut self,
        descriptor: patinae_scene::GpuSamplerDescriptor,
    ) -> Result<patinae_scene::GpuHandle, String> {
        self.command_runtime
            .ok_or_else(|| "host command runtime is not available".to_string())?
            .gpu_create_sampler(descriptor)
    }

    fn gpu_write_buffer(
        &mut self,
        buffer: patinae_scene::GpuHandle,
        offset: u64,
        data: Vec<u8>,
    ) -> Result<(), String> {
        self.command_runtime
            .ok_or_else(|| "host command runtime is not available".to_string())?
            .gpu_write_buffer(buffer, offset, data)
    }

    fn gpu_copy_buffer_to_buffer(
        &mut self,
        source: patinae_scene::GpuHandle,
        source_offset: u64,
        destination: patinae_scene::GpuHandle,
        destination_offset: u64,
        size: u64,
    ) -> Result<(), String> {
        self.command_runtime
            .ok_or_else(|| "host command runtime is not available".to_string())?
            .gpu_copy_buffer_to_buffer(source, source_offset, destination, destination_offset, size)
    }

    fn gpu_read_buffer(
        &mut self,
        buffer: patinae_scene::GpuHandle,
        offset: u64,
        size: u64,
    ) -> Result<Vec<u8>, String> {
        self.command_runtime
            .ok_or_else(|| "host command runtime is not available".to_string())?
            .gpu_read_buffer(buffer, offset, size)
    }

    fn gpu_create_shader_module(
        &mut self,
        descriptor: patinae_scene::GpuShaderModuleDescriptor,
    ) -> Result<patinae_scene::GpuHandle, String> {
        self.command_runtime
            .ok_or_else(|| "host command runtime is not available".to_string())?
            .gpu_create_shader_module(descriptor)
    }

    fn gpu_create_bind_group_layout(
        &mut self,
        descriptor: patinae_scene::GpuBindGroupLayoutDescriptor,
    ) -> Result<patinae_scene::GpuHandle, String> {
        self.command_runtime
            .ok_or_else(|| "host command runtime is not available".to_string())?
            .gpu_create_bind_group_layout(descriptor)
    }

    fn gpu_create_pipeline_layout(
        &mut self,
        descriptor: patinae_scene::GpuPipelineLayoutDescriptor,
    ) -> Result<patinae_scene::GpuHandle, String> {
        self.command_runtime
            .ok_or_else(|| "host command runtime is not available".to_string())?
            .gpu_create_pipeline_layout(descriptor)
    }

    fn gpu_create_compute_pipeline(
        &mut self,
        descriptor: patinae_scene::GpuComputePipelineDescriptor,
    ) -> Result<patinae_scene::GpuHandle, String> {
        self.command_runtime
            .ok_or_else(|| "host command runtime is not available".to_string())?
            .gpu_create_compute_pipeline(descriptor)
    }

    fn gpu_create_render_pipeline(
        &mut self,
        descriptor: patinae_scene::GpuRenderPipelineDescriptor,
    ) -> Result<patinae_scene::GpuHandle, String> {
        self.command_runtime
            .ok_or_else(|| "host command runtime is not available".to_string())?
            .gpu_create_render_pipeline(descriptor)
    }

    fn gpu_create_cached_shader_module(
        &mut self,
        descriptor: patinae_scene::GpuShaderModuleDescriptor,
    ) -> Result<patinae_scene::GpuCachedHandle, String> {
        self.command_runtime
            .ok_or_else(|| "host command runtime is not available".to_string())?
            .gpu_create_cached_shader_module(descriptor)
    }

    fn gpu_create_cached_bind_group_layout(
        &mut self,
        descriptor: patinae_scene::GpuBindGroupLayoutDescriptor,
    ) -> Result<patinae_scene::GpuCachedHandle, String> {
        self.command_runtime
            .ok_or_else(|| "host command runtime is not available".to_string())?
            .gpu_create_cached_bind_group_layout(descriptor)
    }

    fn gpu_create_cached_pipeline_layout(
        &mut self,
        descriptor: patinae_scene::GpuPipelineLayoutDescriptor,
    ) -> Result<patinae_scene::GpuCachedHandle, String> {
        self.command_runtime
            .ok_or_else(|| "host command runtime is not available".to_string())?
            .gpu_create_cached_pipeline_layout(descriptor)
    }

    fn gpu_create_cached_compute_pipeline(
        &mut self,
        descriptor: patinae_scene::GpuComputePipelineDescriptor,
    ) -> Result<patinae_scene::GpuCachedHandle, String> {
        self.command_runtime
            .ok_or_else(|| "host command runtime is not available".to_string())?
            .gpu_create_cached_compute_pipeline(descriptor)
    }

    fn gpu_create_cached_render_pipeline(
        &mut self,
        descriptor: patinae_scene::GpuRenderPipelineDescriptor,
    ) -> Result<patinae_scene::GpuCachedHandle, String> {
        self.command_runtime
            .ok_or_else(|| "host command runtime is not available".to_string())?
            .gpu_create_cached_render_pipeline(descriptor)
    }

    fn gpu_cache_stats(&mut self) -> Result<patinae_scene::GpuCacheStats, String> {
        self.command_runtime
            .ok_or_else(|| "host command runtime is not available".to_string())?
            .gpu_cache_stats()
    }

    fn gpu_drop_plugin_cache(&mut self) -> Result<(), String> {
        self.command_runtime
            .ok_or_else(|| "host command runtime is not available".to_string())?
            .gpu_drop_plugin_cache()
    }

    fn gpu_create_bind_group(
        &mut self,
        descriptor: patinae_scene::GpuBindGroupDescriptor,
    ) -> Result<patinae_scene::GpuHandle, String> {
        self.command_runtime
            .ok_or_else(|| "host command runtime is not available".to_string())?
            .gpu_create_bind_group(descriptor)
    }

    fn gpu_dispatch_compute(
        &mut self,
        pipeline: patinae_scene::GpuHandle,
        bind_groups: Vec<patinae_scene::GpuHandle>,
        workgroups: [u32; 3],
    ) -> Result<(), String> {
        self.command_runtime
            .ok_or_else(|| "host command runtime is not available".to_string())?
            .gpu_dispatch_compute(pipeline, bind_groups, workgroups)
    }

    fn gpu_submit_batch(
        &mut self,
        batch: patinae_scene::GpuSubmitBatch,
    ) -> Result<patinae_scene::GpuBatchResult, String> {
        self.command_runtime
            .ok_or_else(|| "host command runtime is not available".to_string())?
            .gpu_submit_batch(batch)
    }

    fn set_viewport_gpu_image_from_buffer(
        &mut self,
        buffer: patinae_scene::GpuHandle,
        width: u32,
        height: u32,
    ) -> Result<(), String> {
        self.command_runtime
            .ok_or_else(|| "host command runtime is not available".to_string())?
            .set_viewport_gpu_image_from_buffer(buffer, width, height)?;
        self.session.viewport_image = None;
        self.redraw_requested = true;
        Ok(())
    }

    fn gpu_drop_handles(&mut self, handles: Vec<patinae_scene::GpuHandle>) -> Result<(), String> {
        self.command_runtime
            .ok_or_else(|| "host command runtime is not available".to_string())?
            .gpu_drop_handles(handles)
    }
}

struct RuntimeShared {
    viewer: RuntimeViewer,
    command_registry: CommandRegistry,
    command_names: Vec<String>,
    setting_names: Vec<String>,
    dynamic_settings: patinae_cmd::DynamicSettingRegistry,
    scene_generation: u64,
}

impl RuntimeShared {
    fn from_wire(input: crate::wire::WireSharedInput) -> Result<Self, String> {
        validate_wire_version(input.wire_version)?;
        let mut viewer = RuntimeViewer::from_session_bytes(&input.session, 0, 0, None, None, None)?;
        viewer.session.viewport_image = input.viewport_image;
        Ok(Self {
            viewer,
            command_registry: CommandRegistry::new(),
            command_names: input.command_names,
            setting_names: input.setting_names,
            dynamic_settings: wire::dynamic_registry_from_wire(&input.dynamic_settings)?,
            scene_generation: input.scene_generation,
        })
    }

    fn from_poll_wire(input: crate::wire::WirePollSharedInput) -> Result<Self, String> {
        validate_wire_version(input.wire_version)?;
        let mut session = patinae_scene::Session::new();
        session.camera = input.camera;
        session.settings = input.settings;
        session.clear_color = input.clear_color;
        session.movie.set_frame_count(input.movie.frame_count);
        if input.movie.frame_count > 0 {
            session.movie.goto_frame(input.movie.current_frame);
        }
        if input.movie.is_playing {
            session.movie.play();
        }
        if input.movie.rock_enabled {
            session.movie.set_rock(true);
        }
        Ok(Self {
            viewer: RuntimeViewer {
                session,
                viewport_size: (0, 0),
                displayed_geometry: None,
                displayed_geometry_spool: None,
                command_runtime: None,
                redraw_requested: false,
                viewport_image_changed: false,
            },
            command_registry: CommandRegistry::new(),
            command_names: input.command_names,
            setting_names: input.setting_names,
            dynamic_settings: wire::dynamic_registry_from_wire(&input.dynamic_settings)?,
            scene_generation: input.scene_generation,
        })
    }

    fn with_shared<T>(&self, f: impl FnOnce(SharedContext<'_>) -> T) -> T {
        let setting_names: Vec<&str> = self.setting_names.iter().map(String::as_str).collect();
        let shared = SharedContext {
            registry: &self.viewer.session.registry,
            camera: &self.viewer.session.camera,
            selections: &self.viewer.session.selections,
            recent_atoms: &self.viewer.session.recent_atoms,
            named_palette: &self.viewer.session.named_palette,
            movie: &self.viewer.session.movie,
            settings: &self.viewer.session.settings,
            clear_color: self.viewer.session.clear_color,
            gpu_device: None,
            gpu_queue: None,
            scene_generation: self.scene_generation,
            viewport_image: self.viewer.session.viewport_image.as_ref(),
            command_names: &self.command_names,
            command_registry: &self.command_registry,
            setting_names: &setting_names,
            dynamic_settings: Some(&self.dynamic_settings),
        };
        f(shared)
    }
}

fn arg_hint_to_abi(hint: &patinae_cmd::ArgHint) -> u8 {
    match hint {
        patinae_cmd::ArgHint::None | patinae_cmd::ArgHint::Keywords(_) => ARG_HINT_NONE,
        patinae_cmd::ArgHint::Path => ARG_HINT_PATH,
        patinae_cmd::ArgHint::Selection => ARG_HINT_SELECTION,
        patinae_cmd::ArgHint::Object => ARG_HINT_OBJECT,
        patinae_cmd::ArgHint::Representation => ARG_HINT_REPRESENTATION,
        patinae_cmd::ArgHint::Color => ARG_HINT_COLOR,
        patinae_cmd::ArgHint::Setting => ARG_HINT_SETTING,
        patinae_cmd::ArgHint::SettingValue => ARG_HINT_SETTING_VALUE,
        patinae_cmd::ArgHint::NamedSelection => ARG_HINT_NAMED_SELECTION,
        patinae_cmd::ArgHint::LabelProperty => ARG_HINT_LABEL_PROPERTY,
        patinae_cmd::ArgHint::Command => ARG_HINT_COMMAND,
    }
}

fn setting_type_to_abi(setting_type: patinae_settings::SettingType) -> u8 {
    match setting_type {
        patinae_settings::SettingType::Blank => SETTING_TYPE_BLANK,
        patinae_settings::SettingType::Bool => SETTING_TYPE_BOOL,
        patinae_settings::SettingType::Int => SETTING_TYPE_INT,
        patinae_settings::SettingType::Float => SETTING_TYPE_FLOAT,
        patinae_settings::SettingType::Float3 => SETTING_TYPE_FLOAT3,
        patinae_settings::SettingType::Color => SETTING_TYPE_COLOR,
        patinae_settings::SettingType::String => SETTING_TYPE_STRING,
    }
}

fn setting_value_to_abi(value: &patinae_settings::SettingValue) -> AbiSettingValue {
    match value {
        patinae_settings::SettingValue::Bool(value) => AbiSettingValue {
            tag: SETTING_VALUE_BOOL,
            bool_value: u8::from(*value),
            ..AbiSettingValue::NONE
        },
        patinae_settings::SettingValue::Int(value) => AbiSettingValue {
            tag: SETTING_VALUE_INT,
            int_value: *value,
            ..AbiSettingValue::NONE
        },
        patinae_settings::SettingValue::Float(value) => AbiSettingValue {
            tag: SETTING_VALUE_FLOAT,
            float_values: [*value, 0.0, 0.0],
            ..AbiSettingValue::NONE
        },
        patinae_settings::SettingValue::Float3(value) => AbiSettingValue {
            tag: SETTING_VALUE_FLOAT3,
            float_values: *value,
            ..AbiSettingValue::NONE
        },
        patinae_settings::SettingValue::Color(value) => AbiSettingValue {
            tag: SETTING_VALUE_COLOR,
            int_value: *value,
            ..AbiSettingValue::NONE
        },
        patinae_settings::SettingValue::String(value) => AbiSettingValue {
            tag: SETTING_VALUE_STRING,
            string_value: AbiStr::from_borrowed(value),
            ..AbiSettingValue::NONE
        },
    }
}

fn side_effect_to_abi(side_effect: patinae_settings::SideEffectCategory) -> u8 {
    match side_effect {
        patinae_settings::SideEffectCategory::SceneInvalidate => SIDE_EFFECT_SCENE_INVALIDATE,
        patinae_settings::SideEffectCategory::SceneChanged => SIDE_EFFECT_SCENE_CHANGED,
        patinae_settings::SideEffectCategory::ShaderReload => SIDE_EFFECT_SHADER_RELOAD,
        patinae_settings::SideEffectCategory::ShaderComputeLighting => {
            SIDE_EFFECT_SHADER_COMPUTE_LIGHTING
        }
        patinae_settings::SideEffectCategory::OrthoDirty => SIDE_EFFECT_ORTHO_DIRTY,
        patinae_settings::SideEffectCategory::SeqChanged => SIDE_EFFECT_SEQ_CHANGED,
        patinae_settings::SideEffectCategory::StereoUpdate => SIDE_EFFECT_STEREO_UPDATE,
        patinae_settings::SideEffectCategory::RepresentationRebuild => {
            SIDE_EFFECT_REPRESENTATION_REBUILD
        }
        patinae_settings::SideEffectCategory::ColorRebuild => SIDE_EFFECT_COLOR_REBUILD,
        patinae_settings::SideEffectCategory::SurfaceTransparency => {
            SIDE_EFFECT_SURFACE_TRANSPARENCY
        }
        patinae_settings::SideEffectCategory::FullRebuild => SIDE_EFFECT_FULL_REBUILD,
        patinae_settings::SideEffectCategory::ViewportUpdate => SIDE_EFFECT_VIEWPORT_UPDATE,
    }
}
