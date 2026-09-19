//! Command executor
//!
//! Dispatches and executes commands against a ViewerLike implementation.

use std::sync::Arc;
use std::time::Instant;

use ahash::AHashMap;

use crate::command::{
    AsyncCommandSink, CommandAction, CommandContext, CommandRegistry, DynamicSettingRegistry,
    FormatHandler, LoadedPluginCapability, OutputMessage, ScriptHandler, ViewerLike,
};
use crate::error::{CmdError, CmdResult};
use crate::history::CommandHistory;

/// Result of command execution including any output messages and actions
#[derive(Debug, Default)]
pub struct CommandOutput {
    /// Work was queued; successful dispatch does not imply completion.
    /// Task identities accepted during execution, even if a later step failed.
    pub task_ids: Vec<crate::tasks::TaskId>,
    /// Output messages from the command (typed with info/warning/error)
    pub messages: Vec<OutputMessage>,
    /// Side-effect actions requested by the command
    pub actions: Vec<CommandAction>,
    /// Wall-clock duration of command execution
    pub duration: Option<std::time::Duration>,
}

impl CommandOutput {
    /// Create a new empty output
    pub fn new() -> Self {
        Self {
            task_ids: Vec::new(),
            messages: Vec::new(),
            actions: Vec::new(),
            duration: None,
        }
    }

    /// Check if there are any output messages
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }
}

/// Command status together with output collected before success or failure.
#[derive(Debug)]
pub struct CommandExecution {
    /// Original parsing or execution result.
    pub result: Result<(), CmdError>,
    /// Collected messages, actions, timing, and dispatch state.
    pub output: CommandOutput,
}

impl CommandExecution {
    /// Convert to the traditional success-output or error result.
    ///
    /// # Errors
    /// Returns the original command error, discarding partial output.
    pub fn into_result(self) -> Result<CommandOutput, CmdError> {
        self.result.map(|()| self.output)
    }
}

/// Portable command receipt; accepted tasks retain their own completion records.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CommandReply {
    pub result: Result<(), String>,
    pub messages: Vec<crate::OutputMessage>,
    pub task_ids: Vec<crate::tasks::TaskId>,
}
impl From<CommandExecution> for CommandReply {
    fn from(execution: CommandExecution) -> Self {
        Self {
            result: execution.result.map_err(|error| error.to_string()),
            messages: execution.output.messages,
            task_ids: execution.output.task_ids,
        }
    }
}

#[derive(Default)]
struct PluginRegistration {
    capability: Option<LoadedPluginCapability>,
    has_executor: bool,
    settings: Vec<String>,
}

/// Executes commands and owns their registrations and history.
pub struct CommandExecutor {
    next_plugin_owner: u64,
    plugins: Vec<(crate::PluginInstanceId, PluginRegistration)>,
    // Read-only projection for command contexts borrowing the executor.
    capabilities: Vec<LoadedPluginCapability>,
    /// Command registry
    registry: CommandRegistry,
    /// Command history
    history: CommandHistory,
    /// Script handlers registered by plugins (extension -> handler for `run` command)
    script_handlers: crate::registration::RegistrationLayers<ScriptHandler>,
    /// Format handlers registered by plugins (extension -> handler for `load`/`save`)
    format_handlers: crate::registration::RegistrationLayers<Arc<FormatHandler>>,
    /// Dynamic settings registered by plugins
    dynamic_settings: DynamicSettingRegistry,
    host_task_executors: std::collections::HashSet<String>,
}

impl Default for CommandExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandExecutor {
    /// Create a new executor with built-in commands
    pub fn new() -> Self {
        Self {
            next_plugin_owner: 0,
            plugins: Vec::new(),
            capabilities: Vec::new(),
            registry: CommandRegistry::with_builtins(),
            history: CommandHistory::new(),
            script_handlers: Default::default(),
            format_handlers: Default::default(),
            dynamic_settings: DynamicSettingRegistry::new(),
            host_task_executors: std::collections::HashSet::new(),
        }
    }

    /// Allocate a stable identity for one installation in this executor.
    pub fn allocate_plugin_owner(&mut self) -> crate::PluginInstanceId {
        self.next_plugin_owner = self
            .next_plugin_owner
            .checked_add(1)
            .expect("plugin identity exhausted");
        let id = crate::PluginInstanceId(self.next_plugin_owner);
        self.plugins.push((id, PluginRegistration::default()));
        id
    }

    fn plugin_registration_mut(
        &mut self,
        owner: crate::PluginInstanceId,
    ) -> &mut PluginRegistration {
        &mut self
            .plugins
            .iter_mut()
            .find(|(id, _)| *id == owner)
            .expect("plugin identity belongs to this executor")
            .1
    }

    /// Install a plugin script handler with reversible extension overrides.
    pub fn register_owned_script(
        &mut self,
        owner: crate::PluginInstanceId,
        extension: String,
        handler: ScriptHandler,
    ) {
        self.script_handlers.insert(extension, owner, handler);
    }

    /// Install a plugin format handler with reversible extension overrides.
    pub fn register_owned_format(
        &mut self,
        owner: crate::PluginInstanceId,
        handler: FormatHandler,
    ) {
        let handler = Arc::new(handler);
        for extension in &handler.extensions {
            self.format_handlers
                .insert(extension.clone(), owner, handler.clone());
        }
    }

    /// Track a successfully registered setting for removal with its plugin.
    pub fn record_plugin_setting(&mut self, owner: crate::PluginInstanceId, name: String) {
        self.plugin_registration_mut(owner).settings.push(name);
    }

    /// Record a loaded plugin and its optional task executor.
    pub fn record_owned_plugin(
        &mut self,
        owner: crate::PluginInstanceId,
        capability: LoadedPluginCapability,
        has_executor: bool,
    ) {
        let registration = self.plugin_registration_mut(owner);
        registration.capability = Some(capability);
        registration.has_executor = has_executor;
        self.refresh_capabilities();
    }

    /// Remove all registrations owned by one plugin, preserving other owners.
    pub fn unregister_plugin(&mut self, owner: crate::PluginInstanceId) {
        self.registry.unregister_owner(owner);
        self.script_handlers.remove_owner(owner);
        self.format_handlers.remove_owner(owner);
        if let Some(index) = self.plugins.iter().position(|(id, _)| *id == owner) {
            let (_, registration) = self.plugins.remove(index);
            self.dynamic_settings
                .unregister_where(|name| registration.settings.iter().any(|entry| entry == name));
            self.refresh_capabilities();
        }
    }

    /// Register an executor provided directly by a host rather than a library.
    pub fn register_task_executor(&mut self, name: impl Into<String>) {
        self.host_task_executors.insert(name.into());
    }

    /// Whether a host or loaded library provides this executor.
    pub fn task_executor_available(&self, name: &str) -> bool {
        self.host_task_executors.contains(name)
            || self.plugins.iter().any(|(_, registration)| {
                registration.has_executor
                    && registration
                        .capability
                        .as_ref()
                        .is_some_and(|capability| capability.name == name)
            })
    }

    /// Returns the current executor names for host admission routing.
    pub fn task_executors(&self) -> std::collections::HashSet<String> {
        self.host_task_executors
            .iter()
            .cloned()
            .chain(self.plugins.iter().filter_map(|(_, registration)| {
                registration
                    .capability
                    .as_ref()
                    .filter(|_| registration.has_executor)
                    .map(|capability| capability.name.clone())
            }))
            .collect()
    }

    /// Get a reference to the command registry
    pub fn registry(&self) -> &CommandRegistry {
        &self.registry
    }

    /// Get a mutable reference to the command registry
    pub fn registry_mut(&mut self) -> &mut CommandRegistry {
        &mut self.registry
    }

    /// Get a reference to the command history
    pub fn history(&self) -> &CommandHistory {
        &self.history
    }

    /// Get a mutable reference to the command history
    pub fn history_mut(&mut self) -> &mut CommandHistory {
        &mut self.history
    }

    /// Register a script handler for a specific extension.
    ///
    /// Used by plugins to handle non-.pml files in the `run` command.
    pub fn register_script_handler(
        &mut self,
        extension: impl Into<String>,
        handler: ScriptHandler,
    ) {
        let extension = extension.into();
        self.script_handlers.replace(extension, handler);
    }

    /// Get a reference to the script handlers map
    pub fn script_handlers(&self) -> &AHashMap<String, ScriptHandler> {
        self.script_handlers.active()
    }

    /// Register a format handler for `load`/`save`.
    ///
    /// Each extension in the handler's list gets an entry pointing to the
    /// shared handler. Built-in formats always take priority over plugins.
    pub fn register_format_handler(&mut self, handler: FormatHandler) {
        let handler = Arc::new(handler);
        for ext in &handler.extensions {
            self.format_handlers.replace(ext.clone(), handler.clone());
        }
    }

    /// Get a reference to the format handlers map
    pub fn format_handlers(&self) -> &AHashMap<String, Arc<FormatHandler>> {
        self.format_handlers.active()
    }

    /// Get the dynamic settings registry for registration and host plumbing.
    ///
    /// Command implementations should read settings through
    /// [`CommandContext::setting_value`] or the typed `setting_*` helpers.
    pub fn dynamic_settings(&self) -> &DynamicSettingRegistry {
        &self.dynamic_settings
    }

    /// Get mutable dynamic settings for plugin registration.
    pub fn dynamic_settings_mut(&mut self) -> &mut DynamicSettingRegistry {
        &mut self.dynamic_settings
    }

    /// Records metadata for one successfully loaded plugin.
    pub fn record_loaded_plugin_capability(&mut self, capability: LoadedPluginCapability) {
        let owner = self.allocate_plugin_owner();
        self.record_owned_plugin(owner, capability, false);
    }

    /// Returns metadata for plugins loaded into this executor.
    pub fn loaded_plugin_capabilities(&self) -> &[LoadedPluginCapability] {
        &self.capabilities
    }

    fn refresh_capabilities(&mut self) {
        self.capabilities = self
            .plugins
            .iter()
            .filter_map(|(_, registration)| registration.capability.clone())
            .collect();
    }

    /// Execute a single command string
    ///
    /// # Arguments
    /// * `viewer` - The viewer to execute against (implements ViewerLike)
    /// * `cmd` - The command string to execute
    ///
    /// # Example
    /// ```ignore
    /// executor.do_(&mut viewer, "load protein.pdb")?;
    /// executor.do_(&mut viewer, "zoom")?;
    /// ```
    pub fn do_(&mut self, viewer: &mut dyn ViewerLike, cmd: &str) -> CmdResult {
        self.do_with_options(viewer, cmd, false).map(|_| ())
    }

    /// Execute a command with options, returning any output messages
    ///
    /// # Arguments
    /// * `viewer` - The viewer to execute against (implements ViewerLike)
    /// * `cmd` - The command string
    /// * `quiet` - Whether to suppress output
    ///
    /// # Returns
    /// On success, returns `CommandOutput` containing any messages from the command.
    pub fn do_with_options(
        &mut self,
        viewer: &mut dyn ViewerLike,
        cmd: &str,
        quiet: bool,
    ) -> Result<CommandOutput, CmdError> {
        self.do_with_async_sink(viewer, cmd, quiet, None)
    }

    /// Execute a command with an optional host async request sink.
    ///
    /// GUI hosts pass a sink to accept non-blocking command work such as
    /// `fetch`; headless callers usually pass `None` and use sync fallbacks.
    pub fn do_with_async_sink<'a>(
        &'a mut self,
        viewer: &mut dyn ViewerLike,
        cmd: &str,
        quiet: bool,
        async_command_sink: Option<AsyncCommandSink<'a>>,
    ) -> Result<CommandOutput, CmdError> {
        self.execute_captured(viewer, cmd, quiet, async_command_sink)
            .into_result()
    }

    /// Execute a command and retain output even when execution fails.
    pub fn execute_captured<'a>(
        &'a mut self,
        viewer: &mut dyn ViewerLike,
        cmd: &str,
        quiet: bool,
        async_command_sink: Option<AsyncCommandSink<'a>>,
    ) -> CommandExecution {
        let mut output = CommandOutput::new();
        let result = self.capture_into(viewer, cmd, quiet, async_command_sink, &mut output);
        CommandExecution { result, output }
    }

    fn capture_into<'a>(
        &'a mut self,
        viewer: &mut dyn ViewerLike,
        cmd: &str,
        quiet: bool,
        async_command_sink: Option<AsyncCommandSink<'a>>,
        output: &mut CommandOutput,
    ) -> Result<(), CmdError> {
        let cmd = cmd.trim();
        if cmd.is_empty() {
            return Ok(());
        }

        // Skip comments
        if cmd.starts_with('#') {
            return Ok(());
        }

        // Add to history
        self.history.push(cmd.to_string());

        // Parse the command
        let parsed = self.registry.parse_command(cmd)?;

        // Look up the command
        let command = self
            .registry
            .get(&parsed.name)
            .ok_or_else(|| CmdError::unknown_command(parsed.name.clone()))?;

        // Execute and collect output
        let mut ctx = CommandContext::new(viewer)
            .with_quiet(quiet)
            .with_registry(&self.registry)
            .with_script_handlers(self.script_handlers.active())
            .with_format_handlers(self.format_handlers.active())
            .with_history(&self.history)
            .with_dynamic_settings(&self.dynamic_settings)
            .with_loaded_plugin_capabilities(&self.capabilities)
            .with_async_command_sink(async_command_sink);
        let start = command_timer_start();
        let mut result = command.execute(&mut ctx, &parsed);
        if !ctx.take_task_requests().is_empty() && result.is_ok() {
            result = Err(CmdError::execution("task executor unavailable"));
        }
        let duration = start.map(|start| start.elapsed());

        // Return collected output, actions, and timing
        *output = CommandOutput {
            task_ids: ctx.take_task_ids(),
            messages: ctx.take_output(),
            actions: ctx.take_actions(),
            duration,
        };
        result
    }

    /// Execute multiple commands (semicolon or newline separated)
    ///
    /// Stops on first error unless the command is prefixed with `-` (silent fail).
    pub fn do_multi(&mut self, viewer: &mut dyn ViewerLike, cmds: &str) -> CmdResult {
        let commands = self.registry.parse_commands(cmds)?;

        for cmd in commands {
            // Reconstruct command string for logging
            let cmd_str = format_command(&cmd);

            if let Err(e) = self.do_(viewer, &cmd_str) {
                // Check if this is a "silent fail" command (starts with -)
                if cmd.name.starts_with('-') {
                    log::debug!("Silently ignoring error in '{}': {}", cmd.name, e);
                    continue;
                }
                return Err(e);
            }
        }

        Ok(())
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn command_timer_start() -> Option<Instant> {
    None
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
fn command_timer_start() -> Option<Instant> {
    Some(Instant::now())
}

/// Format a parsed command back to a string (for logging)
fn format_command(cmd: &crate::args::ParsedCommand) -> String {
    if let Some(raw_args) = cmd.raw_args() {
        return format!("{} {}", cmd.name, raw_args);
    }

    let mut s = cmd.name.clone();

    for (i, (name, value)) in cmd.args.iter().enumerate() {
        if i == 0 {
            s.push(' ');
        } else {
            s.push_str(", ");
        }

        if let Some(name) = name {
            s.push_str(name);
            s.push('=');
        }

        s.push_str(&value.to_string());
    }

    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LoadedPluginCapability;

    #[test]
    fn unloading_script_overrides_preserves_other_plugins_and_restores_base_handler() {
        let mut executor = CommandExecutor::new();
        let base: ScriptHandler = Arc::new(|_| Err("base".into()));
        let first: ScriptHandler = Arc::new(|_| Err("first".into()));
        let second: ScriptHandler = Arc::new(|_| Err("second".into()));
        executor.register_script_handler("script", base.clone());
        executor.register_owned_script(crate::PluginInstanceId(1), "script".into(), first);
        executor.register_owned_script(crate::PluginInstanceId(2), "script".into(), second.clone());
        executor.unregister_plugin(crate::PluginInstanceId(1));
        assert!(Arc::ptr_eq(&executor.script_handlers()["script"], &second));
        executor.unregister_plugin(crate::PluginInstanceId(2));
        assert!(Arc::ptr_eq(&executor.script_handlers()["script"], &base));
    }

    #[test]
    fn loaded_plugin_capabilities_are_executor_state() {
        let mut executor = CommandExecutor::new();
        assert!(executor.loaded_plugin_capabilities().is_empty());
        executor.record_loaded_plugin_capability(LoadedPluginCapability {
            name: "fixture".to_string(),
            version: "1.2.3".to_string(),
            description: "Fixture integration".to_string(),
        });

        assert_eq!(
            executor.loaded_plugin_capabilities(),
            &[LoadedPluginCapability {
                name: "fixture".to_string(),
                version: "1.2.3".to_string(),
                description: "Fixture integration".to_string(),
            }]
        );
    }

    #[test]
    fn test_format_command() {
        use crate::args::ParsedCommand;

        let cmd = ParsedCommand::new("load")
            .with_arg("file.pdb")
            .with_named_arg("object", "mol");

        let formatted = format_command(&cmd);
        assert_eq!(formatted, "load file.pdb, object=mol");
    }

    #[test]
    fn test_format_command_preserves_raw_args() {
        let cmd = crate::parser::parse_command("alter chain A, chain='B'").unwrap();
        assert_eq!(format_command(&cmd), "alter chain A, chain='B'");

        let cmd = crate::parser::parse_command("alter name CA, b=random.random()").unwrap();
        assert_eq!(format_command(&cmd), "alter name CA, b=random.random()");
    }
}
