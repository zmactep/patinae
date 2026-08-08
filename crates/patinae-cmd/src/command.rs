//! Command trait and registry
//!
//! Defines the interface for commands and the registry that maps names to implementations.

use std::io::{Read, Write};
use std::sync::Arc;

use ahash::AHashMap;
use patinae_mol::ObjectMolecule;
use patinae_settings::{DssAlgorithm, DynamicSettingDescriptor, SettingValue, SharedSettingStore};
use serde::{Deserialize, Serialize};

use crate::history::CommandHistory;
use crate::setting_access::ResolvedSetting;

/// A script handler registered by a plugin for a specific file extension.
///
/// Used by the `run` command to dispatch non-.pml files to the appropriate handler.
pub type ScriptHandler = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

/// Factory that reads molecules from a plugin-provided file format.
///
/// The host opens the file and passes a `Box<dyn Read>`. The plugin
/// parses the content and returns molecules.
pub type PluginReaderFn =
    Arc<dyn Fn(Box<dyn Read>) -> Result<Vec<ObjectMolecule>, String> + Send + Sync>;

/// Factory that writes molecules in a plugin-provided file format.
///
/// The host opens the file and passes a `Box<dyn Write>`. The plugin
/// serializes the molecules.
pub type PluginWriterFn =
    Arc<dyn Fn(Box<dyn Write>, &[ObjectMolecule]) -> Result<(), String> + Send + Sync>;

/// A plugin-provided file format handler for `load` and `save` commands.
pub struct FormatHandler {
    /// Human-readable format name (e.g., "MMTF", "DCD")
    pub name: String,
    /// File extensions this handler supports (lowercase, no dot, e.g., `["mmtf"]`)
    pub extensions: Vec<String>,
    /// Reader factory, or `None` if the format is write-only
    pub reader: Option<PluginReaderFn>,
    /// Writer factory, or `None` if the format is read-only
    pub writer: Option<PluginWriterFn>,
}

/// Describes one loaded plugin without depending on plugin-host types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedPluginCapability {
    /// Plugin display name.
    pub name: String,
    /// Plugin version reported during registration.
    pub version: String,
    /// Plugin description reported during registration.
    pub description: String,
}

/// Describes the file suffixes accepted by one built-in command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BuiltinCommandCapability {
    /// Canonical command name.
    pub command: &'static str,
    /// Registered aliases for the command.
    pub aliases: &'static [&'static str],
    /// Normalized suffixes accepted by the command, without a leading dot.
    pub suffixes: &'static [&'static str],
    /// Whether the command is available in this build.
    pub available: bool,
}

impl BuiltinCommandCapability {
    /// Returns whether this command accepts `suffix`.
    pub fn supports_suffix(&self, suffix: &str) -> bool {
        self.available && self.suffixes.contains(&suffix)
    }
}

// =============================================================================
// Async command requests
// =============================================================================

/// Fetch format carried across the command/host boundary.
///
/// This intentionally does not depend on `patinae_io::FetchFormat` so hosts can
/// compile the command infrastructure without enabling network fetch features.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchFormatCode {
    /// PDB text format.
    Pdb,
    /// mmCIF text format.
    Cif,
    /// BinaryCIF format.
    Bcif,
}

/// Request emitted by the `fetch` command when an async host is available.
#[derive(Debug, Clone)]
pub struct FetchRequest {
    /// PDB ID to fetch.
    pub code: String,
    /// Object name to create on completion.
    pub name: String,
    /// Format to download.
    pub format: FetchFormatCode,
    /// Bond tolerance captured at submission time.
    pub bond_tolerance: f32,
    /// Whether DSS should be applied after parsing.
    pub auto_dss: bool,
    /// DSS algorithm captured at submission time.
    pub dss_algorithm: DssAlgorithm,
}

/// Async work requested by a command.
#[derive(Debug, Clone)]
pub enum AsyncCommandRequest {
    /// Fetch a structure from RCSB PDB.
    Fetch(FetchRequest),
}

/// Host-provided callback for accepting async command work.
pub type AsyncCommandSink<'a> = &'a mut dyn FnMut(AsyncCommandRequest) -> bool;

// =============================================================================
// Dynamic setting registry
// =============================================================================

/// An entry in the dynamic setting registry: descriptor + shared store.
///
/// Cloning is cheap — the store is behind `Arc<RwLock>`.
#[derive(Clone)]
pub struct DynamicSettingEntry {
    /// Metadata (type, default, constraints, side effects).
    pub descriptor: DynamicSettingDescriptor,
    /// Shared store holding the actual values (owned by the plugin).
    pub store: SharedSettingStore,
}

/// Registry for dynamically registered settings (from plugins).
///
/// Lives on [`CommandExecutor`](crate::executor::CommandExecutor), mirroring
/// how `script_handlers` and `format_handlers` are stored.
/// The `set`/`get` commands fall through to this after checking the built-in
/// static registry.
/// Cloning is cheap — each entry's store is behind `Arc<RwLock>`.
#[derive(Clone)]
pub struct DynamicSettingRegistry {
    entries: AHashMap<String, DynamicSettingEntry>,
    /// Insertion-ordered names (for autocomplete).
    names: Vec<String>,
}

impl DynamicSettingRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            entries: AHashMap::new(),
            names: Vec::new(),
        }
    }

    /// Register a dynamic setting with its shared store.
    ///
    /// Returns an error if the name collides with a built-in setting or an
    /// already-registered dynamic setting.
    pub fn register(
        &mut self,
        descriptor: DynamicSettingDescriptor,
        store: SharedSettingStore,
    ) -> Result<(), String> {
        let name = &descriptor.name;

        // Reject collision with built-in settings
        if patinae_settings::registry::lookup_by_name(name).is_some() {
            return Err(format!(
                "Cannot register dynamic setting '{}': collides with a built-in setting",
                name
            ));
        }

        // Reject duplicate dynamic settings
        if self.entries.contains_key(name) {
            return Err(format!("Dynamic setting '{}' is already registered", name));
        }

        self.names.push(name.clone());
        self.entries
            .insert(name.clone(), DynamicSettingEntry { descriptor, store });
        Ok(())
    }

    /// Remove all entries whose name matches a predicate.
    pub fn unregister_where(&mut self, predicate: impl Fn(&str) -> bool) {
        self.entries.retain(|name, _| !predicate(name));
        self.names.retain(|name| !predicate(name));
    }

    /// Look up a dynamic setting by name.
    pub fn lookup(&self, name: &str) -> Option<&DynamicSettingEntry> {
        self.entries.get(name)
    }

    /// All registered dynamic setting names (insertion order).
    pub fn names(&self) -> &[String] {
        &self.names
    }
}

impl Default for DynamicSettingRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// Re-export ViewerLike from patinae-scene
pub use patinae_scene::ViewerLike;

use crate::args::ParsedCommand;
use crate::error::CmdResult;

// ============================================================================
// Argument hints for completion
// ============================================================================

/// Hint about what type of argument a command expects at a given position.
/// Used by the completion system to provide context-aware suggestions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ArgHint {
    /// No specific hint (generic argument)
    #[default]
    None,
    /// File or directory path
    Path,
    /// Selection expression
    Selection,
    /// Object name
    Object,
    /// Representation type (cartoon, sticks, spheres, etc.)
    Representation,
    /// Color name or value
    Color,
    /// Setting name
    Setting,
    /// Setting value — resolve value hints from the setting name in the preceding arg.
    SettingValue,
    /// Named selection only (no objects)
    NamedSelection,
    /// Label property expression (name, resn, resi, chain, b, q, etc.)
    LabelProperty,
    /// Command name (for help, etc.)
    Command,
    /// Fixed list of keyword choices (e.g., &["show", "hide"])
    Keywords(&'static [&'static str]),
}

// ============================================================================
// Output message types
// ============================================================================

/// Kind of command output message
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MessageKind {
    /// Informational message (default)
    #[default]
    Info,
    /// Warning message
    Warning,
    /// Error message
    Error,
}

/// A typed output message from command execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputMessage {
    /// The message text
    pub text: String,
    /// The message kind (info, warning, error)
    pub kind: MessageKind,
}

impl OutputMessage {
    /// Create an info message
    pub fn info(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: MessageKind::Info,
        }
    }

    /// Create a warning message
    pub fn warning(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: MessageKind::Warning,
        }
    }

    /// Create an error message
    pub fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: MessageKind::Error,
        }
    }
}

/// Side-effect actions that a command can request.
///
/// These are dispatched by the application layer after command execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CommandAction {
    /// Request that a named panel be shown.
    ShowPanel(String),
    /// Request that a named panel be hidden.
    HidePanel(String),
    /// Request that the command output log be cleared.
    ClearOutput,
    /// Request application quit.
    Quit,
    /// Request that the host records a successfully opened local file.
    RecordRecentFile {
        /// Absolute or canonical path to the opened file.
        path: String,
        /// Command that opened the file.
        command: String,
    },
}

/// Runtime host inputs requested by a command.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CommandRuntimeRequirements {
    bits: u64,
}

impl CommandRuntimeRequirements {
    /// No extra host runtime inputs are required.
    pub const NONE: Self = Self { bits: 0 };

    /// The command needs host-resolved displayed geometry.
    pub const DISPLAYED_GEOMETRY: Self = Self { bits: 1 << 0 };

    /// The command needs a full serialized session.
    pub const FULL_SESSION: Self = Self { bits: 1 << 1 };

    /// The command needs a host-streamed compact trace geometry source.
    pub const TRACE_GEOMETRY_STREAM: Self = Self { bits: 1 << 2 };

    /// The command needs host-executed GPU runtime callbacks.
    pub const GPU_COMMANDS: Self = Self { bits: 1 << 3 };

    /// The command needs renderer-owned GPU artifact handles.
    pub const RENDER_ARTIFACTS: Self = Self { bits: 1 << 4 };

    /// Builds requirements from raw ABI bits.
    pub const fn from_bits(bits: u64) -> Self {
        Self { bits }
    }

    /// Returns the raw ABI bitset.
    pub const fn bits(self) -> u64 {
        self.bits
    }

    /// Returns true when all requested bits are present.
    pub const fn contains(self, other: Self) -> bool {
        (self.bits & other.bits) == other.bits
    }

    /// Returns the union of two requirement sets.
    pub const fn union(self, other: Self) -> Self {
        Self {
            bits: self.bits | other.bits,
        }
    }

    /// Returns true when no extra runtime inputs are required.
    pub const fn is_empty(self) -> bool {
        self.bits == 0
    }
}

/// Command execution context
///
/// Provides access to the viewer state and execution options.
/// Generic over `V: ViewerLike` to support different viewer implementations.
///
/// The two lifetime parameters are:
/// - `'v` - lifetime of the viewer reference
/// - `'r` - lifetime of the registry reference (may differ from viewer)
pub struct CommandContext<'v, 'r, V: ViewerLike + ?Sized> {
    /// Reference to the viewer (scene, objects, camera, settings)
    pub viewer: &'v mut V,
    /// Whether to suppress output messages
    pub quiet: bool,
    /// Collected output messages (for GUI display)
    output_buffer: Vec<OutputMessage>,
    /// Collected side-effect actions
    action_buffer: Vec<CommandAction>,
    /// Optional reference to command registry (for help lookups)
    registry: Option<&'r CommandRegistry>,
    /// Script handlers registered by plugins (extension -> handler)
    script_handlers: Option<&'r AHashMap<String, ScriptHandler>>,
    /// Format handlers registered by plugins (extension -> handler)
    format_handlers: Option<&'r AHashMap<String, Arc<FormatHandler>>>,
    /// Command history (for saving as .pml script)
    history: Option<&'r CommandHistory>,
    /// Dynamic settings registered by plugins
    dynamic_settings: Option<&'r DynamicSettingRegistry>,
    /// Metadata for plugins loaded into the owning executor.
    loaded_plugin_capabilities: Option<&'r [LoadedPluginCapability]>,
    /// Host-provided async request sink.
    async_command_sink: Option<AsyncCommandSink<'r>>,
}

impl<'v, 'r, V: ViewerLike + ?Sized> CommandContext<'v, 'r, V> {
    /// Create a new command context
    pub fn new(viewer: &'v mut V) -> Self {
        Self {
            viewer,
            quiet: false,
            output_buffer: Vec::new(),
            action_buffer: Vec::new(),
            registry: None,
            script_handlers: None,
            format_handlers: None,
            history: None,
            dynamic_settings: None,
            loaded_plugin_capabilities: None,
            async_command_sink: None,
        }
    }

    /// Set the quiet flag
    pub fn with_quiet(mut self, quiet: bool) -> Self {
        self.quiet = quiet;
        self
    }

    /// Set the command registry reference
    pub fn with_registry(mut self, registry: &'r CommandRegistry) -> Self {
        self.registry = Some(registry);
        self
    }

    /// Set the script handlers reference
    pub fn with_script_handlers(mut self, handlers: &'r AHashMap<String, ScriptHandler>) -> Self {
        self.script_handlers = Some(handlers);
        self
    }

    /// Get a reference to the command registry (if available)
    pub fn registry(&self) -> Option<&CommandRegistry> {
        self.registry
    }

    /// Set the format handlers reference
    pub fn with_format_handlers(
        mut self,
        handlers: &'r AHashMap<String, Arc<FormatHandler>>,
    ) -> Self {
        self.format_handlers = Some(handlers);
        self
    }

    /// Get a script handler for the given extension
    pub fn script_handler(&self, ext: &str) -> Option<&ScriptHandler> {
        self.script_handlers.and_then(|h| h.get(ext))
    }

    /// Get a format handler for the given extension
    pub fn format_handler(&self, ext: &str) -> Option<&Arc<FormatHandler>> {
        self.format_handlers.and_then(|h| h.get(ext))
    }

    /// Get the full script handlers map (if available)
    pub fn script_handlers_map(&self) -> Option<&AHashMap<String, ScriptHandler>> {
        self.script_handlers
    }

    /// Get the full format handlers map (if available)
    pub fn format_handlers_map(&self) -> Option<&AHashMap<String, Arc<FormatHandler>>> {
        self.format_handlers
    }

    /// Set the command history reference
    pub fn with_history(mut self, history: &'r CommandHistory) -> Self {
        self.history = Some(history);
        self
    }

    /// Get the command history
    pub fn history(&self) -> Option<&CommandHistory> {
        self.history
    }

    /// Set the dynamic settings registry reference
    pub fn with_dynamic_settings(mut self, registry: &'r DynamicSettingRegistry) -> Self {
        self.dynamic_settings = Some(registry);
        self
    }

    /// Sets the loaded-plugin metadata visible to commands.
    pub fn with_loaded_plugin_capabilities(
        mut self,
        capabilities: &'r [LoadedPluginCapability],
    ) -> Self {
        self.loaded_plugin_capabilities = Some(capabilities);
        self
    }

    /// Returns metadata for plugins loaded into the current executor.
    pub fn loaded_plugin_capabilities(&self) -> &[LoadedPluginCapability] {
        self.loaded_plugin_capabilities.unwrap_or_default()
    }

    /// Clones loaded-plugin metadata for a child command executor.
    pub(crate) fn clone_loaded_plugin_capabilities_for_child_executor(
        &self,
    ) -> Vec<LoadedPluginCapability> {
        self.loaded_plugin_capabilities().to_vec()
    }

    /// Resolve a built-in or dynamic setting by name.
    ///
    /// Built-in settings take precedence over dynamic plugin settings.
    #[must_use]
    pub fn resolve_setting(&self, name: &str) -> Option<ResolvedSetting> {
        ResolvedSetting::lookup(name, self.dynamic_settings)
    }

    /// Read a setting value by name.
    ///
    /// This reads built-in settings first, then dynamic plugin settings.
    /// Dynamic settings fall back to their descriptor default after `unset`.
    #[must_use]
    pub fn setting_value(&self, name: &str) -> Option<SettingValue> {
        self.resolve_setting(name)
            .map(|setting| setting.global_value_lossy(self.viewer))
    }

    /// Read a setting as a boolean.
    ///
    /// Returns `default` when the setting is not registered, or when neither
    /// the current value nor the descriptor default can be converted to a
    /// boolean.
    #[must_use]
    pub fn setting_bool(&self, name: &str, default: bool) -> bool {
        self.setting_value(name)
            .and_then(|value| value.as_bool())
            .unwrap_or(default)
    }

    /// Read a setting as an integer.
    ///
    /// Returns `default` when the setting is not registered, or when neither
    /// the current value nor the descriptor default can be converted to an
    /// integer.
    #[must_use]
    pub fn setting_int(&self, name: &str, default: i32) -> i32 {
        self.setting_value(name)
            .and_then(|value| value.as_int())
            .unwrap_or(default)
    }

    /// Read a setting as a float.
    ///
    /// Returns `default` when the setting is not registered, or when neither
    /// the current value nor the descriptor default can be converted to a
    /// float.
    #[must_use]
    pub fn setting_float(&self, name: &str, default: f32) -> f32 {
        self.setting_value(name)
            .and_then(|value| value.as_float())
            .unwrap_or(default)
    }

    /// Return the dynamic settings registry for advanced runtime plumbing.
    ///
    /// Ordinary command implementations should use [`Self::resolve_setting`],
    /// [`Self::setting_value`], or the typed `setting_*` helpers. This accessor
    /// exists for host serialization and nested executors that must preserve
    /// plugin-registered setting descriptors and shared stores.
    pub fn dynamic_settings(&self) -> Option<&DynamicSettingRegistry> {
        self.dynamic_settings
    }

    /// Clone dynamic settings for a nested command executor.
    #[must_use]
    pub fn clone_dynamic_settings_for_child_executor(&self) -> DynamicSettingRegistry {
        self.dynamic_settings.cloned().unwrap_or_default()
    }

    /// Set the host async request sink.
    pub fn with_async_command_sink(mut self, sink: Option<AsyncCommandSink<'r>>) -> Self {
        self.async_command_sink = sink;
        self
    }

    /// Submit an async request to the host.
    ///
    /// Returns `true` when the host accepted the request. Commands should use
    /// their synchronous path when this returns `false`.
    pub fn submit_async_request(&mut self, request: AsyncCommandRequest) -> bool {
        self.async_command_sink
            .as_deref_mut()
            .map(|sink| sink(request))
            .unwrap_or(false)
    }

    /// Print an info message (unless quiet mode is enabled)
    ///
    /// The message is both logged and collected in the output buffer
    /// for retrieval by the GUI.
    pub fn print(&mut self, msg: &str) {
        if !self.quiet {
            log::info!("{}", msg);
            self.output_buffer.push(OutputMessage::info(msg));
        }
    }

    /// Print a warning message (unless quiet mode is enabled)
    pub fn print_warning(&mut self, msg: &str) {
        if !self.quiet {
            log::warn!("{}", msg);
            self.output_buffer.push(OutputMessage::warning(msg));
        }
    }

    /// Print an error message (even in quiet mode)
    ///
    /// Error messages are always shown regardless of quiet mode.
    pub fn print_error(&mut self, msg: &str) {
        log::error!("{}", msg);
        self.output_buffer.push(OutputMessage::error(msg));
    }

    /// Take the collected output messages, clearing the buffer
    pub fn take_output(&mut self) -> Vec<OutputMessage> {
        std::mem::take(&mut self.output_buffer)
    }

    /// Get a reference to the collected output messages
    pub fn output(&self) -> &[OutputMessage] {
        &self.output_buffer
    }

    /// Request that a named panel be shown
    pub fn show_panel(&mut self, name: impl Into<String>) {
        self.action_buffer
            .push(CommandAction::ShowPanel(name.into()));
    }

    /// Request that a named panel be hidden
    pub fn hide_panel(&mut self, name: impl Into<String>) {
        self.action_buffer
            .push(CommandAction::HidePanel(name.into()));
    }

    /// Request that the command output log be cleared.
    pub fn clear_output(&mut self) {
        self.action_buffer.push(CommandAction::ClearOutput);
    }

    /// Request application quit
    pub fn quit(&mut self) {
        self.action_buffer.push(CommandAction::Quit);
    }

    /// Request that the host records a successfully opened local file.
    pub fn record_recent_file(&mut self, path: impl Into<String>, command: impl Into<String>) {
        self.action_buffer.push(CommandAction::RecordRecentFile {
            path: path.into(),
            command: command.into(),
        });
    }

    /// Take the collected actions, clearing the buffer
    pub fn take_actions(&mut self) -> Vec<CommandAction> {
        std::mem::take(&mut self.action_buffer)
    }
}

/// Trait for command implementations
///
/// Commands receive a context with access to the viewer state and
/// parsed arguments, and return a result indicating success or failure.
pub trait Command: Send + Sync {
    /// Get the command name
    fn name(&self) -> &str;

    /// Execute the command
    ///
    /// Uses lifetime parameters to allow the context to borrow from non-'static viewers.
    /// - `'v` - lifetime of the viewer reference
    /// - `'r` - lifetime of the registry reference
    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult;

    /// Get command description (plain text, no indentation)
    fn description(&self) -> &str {
        ""
    }

    /// Get usage string (plain text, no indentation)
    fn usage(&self) -> &str {
        ""
    }

    /// Get arguments description (plain text, no indentation, lines joined by `\n`)
    fn arguments(&self) -> &str {
        ""
    }

    /// Get help text for this command
    fn help(&self) -> &str {
        "No help available."
    }

    /// Get list of command aliases
    fn aliases(&self) -> &[&str] {
        &[]
    }

    /// Get argument hints for completion
    ///
    /// Returns a slice of hints indicating what type of argument is expected
    /// at each position. Used by the completion system to provide context-aware
    /// suggestions (e.g., file paths for "load", selections for "select").
    fn arg_hints(&self) -> &[ArgHint] {
        &[]
    }

    /// Runtime host inputs this command needs when invoked dynamically.
    fn runtime_requirements(&self) -> CommandRuntimeRequirements {
        CommandRuntimeRequirements::FULL_SESSION
    }
}

/// Build a formatted help string from structured sections at runtime.
///
/// Produces the same format as [`command_help!`]: each non-empty section gets a
/// header, a blank line, and 4-space-indented content. Empty sections are skipped.
pub fn format_help(description: &str, usage: &str, arguments: &str) -> String {
    let mut text = String::with_capacity(256);

    for (header, content) in [
        ("DESCRIPTION", description),
        ("USAGE", usage),
        ("ARGUMENTS", arguments),
    ] {
        if !content.is_empty() {
            text.push('\n');
            text.push_str(header);
            text.push_str("\n\n");
            for line in content.lines() {
                if line.is_empty() {
                    text.push('\n');
                } else {
                    text.push_str("    ");
                    text.push_str(line);
                    text.push('\n');
                }
            }
        }
    }

    if text.is_empty() {
        "No help available.".to_string()
    } else {
        text
    }
}

/// Generate `description()`, `usage()`, `arguments()`, and `help()` method
/// implementations from structured help sections at compile time.
///
/// Place this macro inside an `impl Command for ...` block. It replaces the
/// manual `fn help()` override with generated methods.
///
/// # Structured form (preferred)
///
/// Arguments are declared as structured data in `REQUIRED` and `OPTIONAL`
/// sections. Each argument is `{ name, type, description }` (required) or
/// `{ name, type, description, default }` (optional). The USAGE string is
/// auto-generated from the command name and argument names.
///
/// Per-argument notes can be added with `=> ["line1", "line2"]` after the
/// closing brace. These appear as indented continuation lines under the
/// argument in the help text.
///
/// ```ignore
/// command_help! {
///     CMD "zoom"
///     DESCRIPTION [
///         "scales and translates the camera to cover the indicated selection.",
///     ]
///     REQUIRED []
///     OPTIONAL [
///         { "selection", "string", "selection to zoom to", "all" },
///         { "buffer", "float", "extra space in Angstroms", "0" },
///     ]
///     EXAMPLES [
///         "zoom",
///         "zoom chain A",
///     ]
/// }
/// // Generated description: "zoom" scales and translates the camera...
/// // Generated usage:       zoom [ selection [, buffer ]]
/// // Generated args:        selection = string: selection to zoom to (default: all)
/// //                        buffer = float: extra space in Angstroms (default: 0)
/// ```
///
/// For commands with multiple USAGE variants (e.g. `set_color`), add an
/// explicit `USAGE [...]` section between DESCRIPTION and REQUIRED to
/// override auto-generation.
#[macro_export]
macro_rules! command_help {
    // ================================================================
    // Structured form with manual USAGE override
    // ================================================================
    (
        CMD $cmd:literal
        DESCRIPTION [$($desc_line:expr),* $(,)?]
        USAGE [$($usage_line:expr),* $(,)?]
        REQUIRED [$({ $rname:literal, $rtyp:literal, $rdesc:literal } $(=> [$($rnote:expr),* $(,)?])?),* $(,)?]
        OPTIONAL [$({ $oname:literal, $otyp:literal, $odesc:literal, $odef:literal } $(=> [$($onote:expr),* $(,)?])?),* $(,)?]
        $(NOTES($notes_title:literal) [$($notes_line:expr),* $(,)?])?
        $(EXAMPLES [$($ex_line:expr),* $(,)?])?
        $(SEE ALSO [$($sa_line:expr),* $(,)?])?
    ) => {
        fn description(&self) -> &str {
            concat!("\"", $cmd, "\" ", command_help!(@join $($desc_line),*))
        }

        fn usage(&self) -> &str {
            command_help!(@join $($usage_line),*)
        }

        fn arguments(&self) -> &str {
            concat!(
                $(
                    $rname, " = ", $rtyp, ": ", $rdesc, "\n",
                    $($( "    ", $rnote, "\n", )*)?
                )*
                $(
                    $oname, " = ", $otyp, ": ", $odesc, " (default: ", $odef, ")", "\n",
                    $($( "    ", $onote, "\n", )*)?
                )*
            )
        }

        fn help(&self) -> &str {
            concat!(
                "\nDESCRIPTION\n\n    \"", $cmd, "\" ",
                command_help!(@join $($desc_line),*),
                "\n",
                "\nUSAGE\n\n",
                $("    ", $usage_line, "\n",)*
                "\nARGUMENTS\n\n",
                $(
                    "    ", $rname, " = ", $rtyp, ": ", $rdesc, "\n",
                    $($( "        ", $rnote, "\n", )*)?
                )*
                $(
                    "    ", $oname, " = ", $otyp, ": ", $odesc, " (default: ", $odef, ")", "\n",
                    $($( "        ", $onote, "\n", )*)?
                )*
                $(
                    "\n", $notes_title, "\n\n",
                    $("    ", $notes_line, "\n",)*
                )?
                $(
                    "\nEXAMPLES\n\n",
                    $("    ", $ex_line, "\n",)*
                )?
                $(
                    "\nSEE ALSO\n\n",
                    $("    ", $sa_line, "\n",)*
                )?
            )
        }
    };

    // ================================================================
    // Structured form with auto-generated USAGE
    // ================================================================
    (
        CMD $cmd:literal
        DESCRIPTION [$($desc_line:expr),* $(,)?]
        REQUIRED [$({ $rname:literal, $rtyp:literal, $rdesc:literal } $(=> [$($rnote:expr),* $(,)?])?),* $(,)?]
        OPTIONAL [$({ $oname:literal, $otyp:literal, $odesc:literal, $odef:literal } $(=> [$($onote:expr),* $(,)?])?),* $(,)?]
        $(NOTES($notes_title:literal) [$($notes_line:expr),* $(,)?])?
        $(EXAMPLES [$($ex_line:expr),* $(,)?])?
        $(SEE ALSO [$($sa_line:expr),* $(,)?])?
    ) => {
        fn description(&self) -> &str {
            concat!("\"", $cmd, "\" ", command_help!(@join $($desc_line),*))
        }

        fn usage(&self) -> &str {
            command_help!(@gen_usage $cmd [$($rname),*] [$($oname),*])
        }

        fn arguments(&self) -> &str {
            concat!(
                $(
                    $rname, " = ", $rtyp, ": ", $rdesc, "\n",
                    $($( "    ", $rnote, "\n", )*)?
                )*
                $(
                    $oname, " = ", $otyp, ": ", $odesc, " (default: ", $odef, ")", "\n",
                    $($( "    ", $onote, "\n", )*)?
                )*
            )
        }

        fn help(&self) -> &str {
            concat!(
                "\nDESCRIPTION\n\n    \"", $cmd, "\" ",
                command_help!(@join $($desc_line),*),
                "\n",
                "\nUSAGE\n\n",
                "    ",
                command_help!(@gen_usage $cmd [$($rname),*] [$($oname),*]),
                "\n",
                "\nARGUMENTS\n\n",
                $(
                    "    ", $rname, " = ", $rtyp, ": ", $rdesc, "\n",
                    $($( "        ", $rnote, "\n", )*)?
                )*
                $(
                    "    ", $oname, " = ", $otyp, ": ", $odesc, " (default: ", $odef, ")", "\n",
                    $($( "        ", $onote, "\n", )*)?
                )*
                $(
                    "\n", $notes_title, "\n\n",
                    $("    ", $notes_line, "\n",)*
                )?
                $(
                    "\nEXAMPLES\n\n",
                    $("    ", $ex_line, "\n",)*
                )?
                $(
                    "\nSEE ALSO\n\n",
                    $("    ", $sa_line, "\n",)*
                )?
            )
        }
    };

    // ================================================================
    // Internal helpers
    // ================================================================

    // Join expressions with newlines
    (@join) => { "" };
    (@join $single:expr) => { $single };
    (@join $first:expr, $($rest:expr),+) => {
        concat!($first, $("\n", $rest),+)
    };

    // Generate full USAGE string: dispatches on whether REQUIRED is empty
    (@gen_usage $cmd:literal [] [$($onames:literal),*]) => {
        concat!($cmd, command_help!(@usage_opts_first $($onames),*))
    };
    (@gen_usage $cmd:literal [$($rnames:literal),+] [$($onames:literal),*]) => {
        concat!($cmd, " ", command_help!(@usage_req $($rnames),+), command_help!(@usage_opts $($onames),*))
    };

    // Join required arg names with ", "
    (@usage_req $single:literal) => { $single };
    (@usage_req $first:literal, $($rest:literal),+) => {
        concat!($first, ", ", command_help!(@usage_req $($rest),+))
    };

    // Nested optional brackets with comma prefix: " [, opt1 [, opt2 ]]"
    (@usage_opts) => { "" };
    (@usage_opts $name:literal) => {
        concat!(" [, ", $name, " ]")
    };
    (@usage_opts $name:literal, $($rest:literal),+) => {
        concat!(" [, ", $name, command_help!(@usage_opts $($rest),+), " ]")
    };

    // First optional bracket (no comma prefix): " [ opt1 [, opt2 ]]"
    (@usage_opts_first) => { "" };
    (@usage_opts_first $name:literal) => {
        concat!(" [ ", $name, " ]")
    };
    (@usage_opts_first $name:literal, $($rest:literal),+) => {
        concat!(" [ ", $name, command_help!(@usage_opts $($rest),+), " ]")
    };
}

/// Whether a command was registered by the core application or by a plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CommandSource {
    #[default]
    BuiltIn,
    Plugin,
}

/// Registry mapping command names to implementations
pub struct CommandRegistry {
    /// Commands indexed by name
    commands: AHashMap<String, Arc<dyn Command>>,
    /// Aliases mapping alias -> command name
    aliases: AHashMap<String, String>,
    /// Source tracking for each command (built-in vs plugin)
    sources: AHashMap<String, CommandSource>,
}

impl Clone for CommandRegistry {
    fn clone(&self) -> Self {
        Self {
            commands: self.commands.clone(),
            aliases: self.aliases.clone(),
            sources: self.sources.clone(),
        }
    }
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            commands: AHashMap::new(),
            aliases: AHashMap::new(),
            sources: AHashMap::new(),
        }
    }

    /// Create a registry with all built-in commands registered
    pub fn with_builtins() -> Self {
        let mut registry = Self::new();
        crate::commands::register_all(&mut registry);
        registry
    }

    /// Register a command
    ///
    /// Also registers any aliases defined by the command.
    pub fn register<C: Command + 'static>(&mut self, cmd: C) {
        let name = cmd.name().to_string();
        let aliases: Vec<String> = cmd.aliases().iter().map(|s| s.to_string()).collect();
        let cmd = Arc::new(cmd);

        // Register aliases
        for alias in aliases {
            self.aliases.insert(alias, name.clone());
        }

        self.sources.insert(name.clone(), CommandSource::BuiltIn);
        self.commands.insert(name, cmd);
    }

    /// Register a boxed command (for plugin system).
    ///
    /// Like [`register`](Self::register) but accepts a `Box<dyn Command>`.
    pub fn register_boxed(&mut self, cmd: Box<dyn Command>) {
        let name = cmd.name().to_string();
        let aliases: Vec<String> = cmd.aliases().iter().map(|s| s.to_string()).collect();
        let cmd: Arc<dyn Command> = cmd.into();

        for alias in aliases {
            self.aliases.insert(alias, name.clone());
        }

        self.sources.insert(name.clone(), CommandSource::Plugin);
        self.commands.insert(name, cmd);
    }

    /// Unregister a command by name.
    ///
    /// Removes the command and any aliases that point to it.
    /// Returns `true` if the command was found and removed.
    pub fn unregister(&mut self, name: &str) -> bool {
        if self.commands.remove(name).is_some() {
            self.aliases.retain(|_, target| target != name);
            self.sources.remove(name);
            true
        } else {
            false
        }
    }

    /// Add an alias for an existing command
    pub fn add_alias(&mut self, alias: impl Into<String>, command: impl Into<String>) {
        self.aliases.insert(alias.into(), command.into());
    }

    /// Look up a command by name or alias
    pub fn get(&self, name: &str) -> Option<Arc<dyn Command>> {
        // Try direct lookup first
        if let Some(cmd) = self.commands.get(name) {
            return Some(cmd.clone());
        }

        // Try alias lookup
        if let Some(real_name) = self.aliases.get(name) {
            return self.commands.get(real_name).cloned();
        }

        None
    }

    /// Check if a command exists
    pub fn contains(&self, name: &str) -> bool {
        self.commands.contains_key(name) || self.aliases.contains_key(name)
    }

    /// Get all command names (not including aliases)
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.commands.keys().map(|s| s.as_str())
    }

    /// Get all command names and aliases
    pub fn all_names(&self) -> impl Iterator<Item = &str> {
        self.commands
            .keys()
            .map(|s| s.as_str())
            .chain(self.aliases.keys().map(|s| s.as_str()))
    }

    /// Get the number of registered commands
    pub fn len(&self) -> usize {
        self.commands.len()
    }

    /// Check if the registry is empty
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Get the source (built-in vs plugin) of a command, resolving aliases.
    pub fn source(&self, name: &str) -> CommandSource {
        let canonical = self.aliases.get(name).map(|s| s.as_str()).unwrap_or(name);
        self.sources.get(canonical).copied().unwrap_or_default()
    }

    /// Get all command names (including aliases) that expect a specific argument hint
    /// at the given position (0-indexed).
    ///
    /// This is used by the completion system to determine which commands should
    /// trigger specific completion types (e.g., file path completion).
    pub fn commands_with_hint(&self, hint: ArgHint, position: usize) -> Vec<&str> {
        let mut result = Vec::new();

        for (name, cmd) in &self.commands {
            let hints = cmd.arg_hints();
            if hints.get(position) == Some(&hint) {
                result.push(name.as_str());
                // Also include aliases for this command
                for (alias, target) in &self.aliases {
                    if target == name {
                        result.push(alias.as_str());
                    }
                }
            }
        }

        result
    }

    /// Check if a command expects a specific argument hint at the given position
    pub fn command_has_hint(&self, name: &str, hint: ArgHint, position: usize) -> bool {
        self.get(name)
            .map(|cmd| cmd.arg_hints().get(position) == Some(&hint))
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, RwLock};

    use patinae_scene::{Session, SessionAdapter};
    use patinae_settings::{registry, DynamicSettingStore, SettingType};

    struct TestCommand {
        name: String,
    }

    impl Command for TestCommand {
        fn name(&self) -> &str {
            &self.name
        }

        fn execute<'v, 'r>(
            &self,
            _ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
            _args: &ParsedCommand,
        ) -> CmdResult {
            Ok(())
        }

        fn help(&self) -> &str {
            "Test command"
        }

        fn aliases(&self) -> &[&str] {
            &["test_alias"]
        }
    }

    #[test]
    fn test_registry() {
        let mut registry = CommandRegistry::new();

        registry.register(TestCommand {
            name: "test".to_string(),
        });

        assert!(registry.contains("test"));
        assert!(registry.contains("test_alias"));
        assert!(!registry.contains("unknown"));

        let cmd = registry.get("test").unwrap();
        assert_eq!(cmd.name(), "test");

        let cmd = registry.get("test_alias").unwrap();
        assert_eq!(cmd.name(), "test");
    }

    #[test]
    fn typed_setting_getters_read_builtin_dynamic_and_defaults() {
        let mut registry = DynamicSettingRegistry::new();

        register_test_setting(
            &mut registry,
            "plugin_enabled",
            SettingType::Bool,
            SettingValue::Bool(true),
            Some(SettingValue::Bool(false)),
        );
        register_test_setting(
            &mut registry,
            "plugin_style",
            SettingType::Int,
            SettingValue::Int(7),
            None,
        );
        register_test_setting(
            &mut registry,
            "plugin_scale",
            SettingType::Float,
            SettingValue::Float(1.5),
            Some(SettingValue::Float(2.5)),
        );

        let mut session = Session::new();
        registry::lookup_by_name("ambient")
            .unwrap()
            .set(&mut session.settings, SettingValue::Float(0.33))
            .unwrap();

        let mut needs_redraw = false;
        let mut viewer = SessionAdapter {
            session: &mut session,
            render_context: None,
            default_size: (1, 1),
            needs_redraw: &mut needs_redraw,
            async_fetch_fn: None,
        };
        let ctx = CommandContext::new(&mut viewer).with_dynamic_settings(&registry);

        assert_eq!(ctx.setting_float("ambient", 0.0), 0.33);
        assert!(!ctx.setting_bool("plugin_enabled", true));
        assert_eq!(ctx.setting_int("plugin_style", 0), 7);
        assert_eq!(ctx.setting_float("plugin_scale", 0.0), 2.5);
        assert_eq!(ctx.setting_int("missing_setting", 42), 42);
    }

    fn register_test_setting(
        registry: &mut DynamicSettingRegistry,
        name: &str,
        setting_type: SettingType,
        default: SettingValue,
        current: Option<SettingValue>,
    ) {
        let descriptor = DynamicSettingDescriptor {
            name: name.to_string(),
            setting_type,
            default,
            min: None,
            max: None,
            value_hints: Vec::new(),
            side_effects: Vec::new(),
            object_overridable: false,
        };
        let mut store = DynamicSettingStore::new();
        if let Some(value) = current {
            store.set(name, value);
        }
        registry
            .register(descriptor, Arc::new(RwLock::new(store)))
            .unwrap();
    }
}
