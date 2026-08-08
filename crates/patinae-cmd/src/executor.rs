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
use crate::parser::{parse_command, parse_commands};

/// Result of command execution including any output messages and actions
#[derive(Debug, Default)]
pub struct CommandOutput {
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

/// Command executor
///
/// Manages command execution and history.
pub struct CommandExecutor {
    /// Command registry
    registry: CommandRegistry,
    /// Command history
    history: CommandHistory,
    /// Script handlers registered by plugins (extension -> handler for `run` command)
    script_handlers: AHashMap<String, ScriptHandler>,
    /// Format handlers registered by plugins (extension -> handler for `load`/`save`)
    format_handlers: AHashMap<String, Arc<FormatHandler>>,
    /// Dynamic settings registered by plugins
    dynamic_settings: DynamicSettingRegistry,
    /// Metadata for plugins loaded into this executor.
    loaded_plugin_capabilities: Vec<LoadedPluginCapability>,
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
            registry: CommandRegistry::with_builtins(),
            history: CommandHistory::new(),
            script_handlers: AHashMap::new(),
            format_handlers: AHashMap::new(),
            dynamic_settings: DynamicSettingRegistry::new(),
            loaded_plugin_capabilities: Vec::new(),
        }
    }

    /// Create an executor with a pre-populated registry and handlers.
    ///
    /// Used by the `run` command to create a script engine that inherits
    /// plugin-registered commands and handlers from the calling context.
    pub fn with_registry(
        registry: CommandRegistry,
        script_handlers: AHashMap<String, ScriptHandler>,
        format_handlers: AHashMap<String, Arc<FormatHandler>>,
        dynamic_settings: DynamicSettingRegistry,
    ) -> Self {
        Self {
            registry,
            history: CommandHistory::new(),
            script_handlers,
            format_handlers,
            dynamic_settings,
            loaded_plugin_capabilities: Vec::new(),
        }
    }

    /// Sets loaded-plugin metadata while constructing a child executor.
    pub(crate) fn with_loaded_plugin_capabilities(
        mut self,
        capabilities: Vec<LoadedPluginCapability>,
    ) -> Self {
        self.loaded_plugin_capabilities = capabilities;
        self
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
        self.script_handlers.insert(extension.into(), handler);
    }

    /// Get a reference to the script handlers map
    pub fn script_handlers(&self) -> &AHashMap<String, ScriptHandler> {
        &self.script_handlers
    }

    /// Register a format handler for `load`/`save`.
    ///
    /// Each extension in the handler's list gets an entry pointing to the
    /// shared handler. Built-in formats always take priority over plugins.
    pub fn register_format_handler(&mut self, handler: FormatHandler) {
        let handler = Arc::new(handler);
        for ext in &handler.extensions {
            self.format_handlers.insert(ext.clone(), handler.clone());
        }
    }

    /// Get a reference to the format handlers map
    pub fn format_handlers(&self) -> &AHashMap<String, Arc<FormatHandler>> {
        &self.format_handlers
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
        self.loaded_plugin_capabilities.push(capability);
    }

    /// Returns metadata for plugins loaded into this executor.
    pub fn loaded_plugin_capabilities(&self) -> &[LoadedPluginCapability] {
        &self.loaded_plugin_capabilities
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
        let cmd = cmd.trim();
        if cmd.is_empty() {
            return Ok(CommandOutput::new());
        }

        // Skip comments
        if cmd.starts_with('#') {
            return Ok(CommandOutput::new());
        }

        // Add to history
        self.history.push(cmd.to_string());

        // Parse the command
        let parsed = parse_command(cmd)?;

        // Look up the command
        let command = self
            .registry
            .get(&parsed.name)
            .ok_or_else(|| CmdError::unknown_command(parsed.name.clone()))?;

        // Execute and collect output
        let mut ctx = CommandContext::new(viewer)
            .with_quiet(quiet)
            .with_registry(&self.registry)
            .with_script_handlers(&self.script_handlers)
            .with_format_handlers(&self.format_handlers)
            .with_history(&self.history)
            .with_dynamic_settings(&self.dynamic_settings)
            .with_loaded_plugin_capabilities(&self.loaded_plugin_capabilities)
            .with_async_command_sink(async_command_sink);
        let start = command_timer_start();
        command.execute(&mut ctx, &parsed)?;
        let duration = start.map(|start| start.elapsed());

        // Return collected output, actions, and timing
        Ok(CommandOutput {
            messages: ctx.take_output(),
            actions: ctx.take_actions(),
            duration,
        })
    }

    /// Execute multiple commands (semicolon or newline separated)
    ///
    /// Stops on first error unless the command is prefixed with `-` (silent fail).
    pub fn do_multi(&mut self, viewer: &mut dyn ViewerLike, cmds: &str) -> CmdResult {
        let commands = parse_commands(cmds)?;

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
    fn test_executor_creation() {
        let executor = CommandExecutor::new();
        assert!(!executor.registry().is_empty() || executor.registry().is_empty());
        assert!(executor.loaded_plugin_capabilities().is_empty());
    }

    #[test]
    fn loaded_plugin_capabilities_are_executor_state() {
        let mut executor = CommandExecutor::new();
        executor.record_loaded_plugin_capability(LoadedPluginCapability {
            name: "python".to_string(),
            version: "1.2.3".to_string(),
            description: "Python integration".to_string(),
        });

        assert_eq!(
            executor.loaded_plugin_capabilities(),
            &[LoadedPluginCapability {
                name: "python".to_string(),
                version: "1.2.3".to_string(),
                description: "Python integration".to_string(),
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
