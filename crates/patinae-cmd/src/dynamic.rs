//! Dynamic Commands
//!
//! A proxy [`Command`] implementation for commands registered dynamically by
//! plugins at runtime.  When invoked by the user, the command captures the
//! invocation (name + args) into a shared list.  The host drains this list
//! each frame and delivers entries to the plugin via `PollContext`.

use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::command::format_help;
use crate::{ArgHint, CmdResult, Command, CommandContext, ParsedCommand, ViewerLike};

/// Record of a dynamic command invocation.
///
/// Produced by [`DynamicCommand::execute`] and delivered to plugins
/// via the polling context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DynamicCommandInvocation {
    /// Command name.
    pub name: String,
    /// Positional arguments.
    pub args: Vec<String>,
}

/// A command that captures invocations for asynchronous plugin processing.
///
/// Registered in `CommandRegistry` via the standard `register_boxed()` path,
/// so it appears in autocomplete, `help`, and `names()` automatically.
pub struct DynamicCommand {
    name: String,
    description: String,
    usage: String,
    arguments: String,
    help_text: String,
    /// Shared sink for captured invocations.
    invocations: Arc<Mutex<Vec<DynamicCommandInvocation>>>,
}

impl DynamicCommand {
    /// Create a new dynamic command.
    ///
    /// `invocations` is shared with the `PluginManager` which drains it
    /// each frame and delivers entries to plugins.
    pub fn new(
        name: String,
        description: String,
        usage: String,
        arguments: String,
        invocations: Arc<Mutex<Vec<DynamicCommandInvocation>>>,
    ) -> Self {
        let help_text = format_help(&description, &usage, &arguments);
        Self {
            name,
            description,
            usage,
            arguments,
            help_text,
            invocations,
        }
    }
}

impl Command for DynamicCommand {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn usage(&self) -> &str {
        &self.usage
    }

    fn arguments(&self) -> &str {
        &self.arguments
    }

    fn help(&self) -> &str {
        &self.help_text
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let arg_strings: Vec<String> = args.args.iter().map(|(_, v)| v.to_string()).collect();

        if let Ok(mut list) = self.invocations.lock() {
            list.push(DynamicCommandInvocation {
                name: self.name.clone(),
                args: arg_strings,
            });
            ctx.mark_deferred();
        }

        // Always succeeds -- actual handling is asynchronous in the plugin.
        Ok(())
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::None]
    }
}
