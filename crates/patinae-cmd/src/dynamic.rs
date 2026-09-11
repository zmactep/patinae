//! Dynamic Commands
//!
//! A proxy [`Command`] implementation for commands registered dynamically by
//! plugins at runtime. An invocation produces a task request; the host assigns
//! its identity and delivers the accepted work to the owning `PollContext`.

use crate::command::format_help;
use crate::{ArgHint, CmdResult, Command, CommandContext, ParsedCommand, ViewerLike};

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
    /// Host-bound executor identity.
    executor: String,
    owner_tag: Option<u64>,
}

impl DynamicCommand {
    /// Create a new dynamic command.
    ///
    /// The host binds the executor and optional external connection identity.
    pub fn new(
        name: String,
        description: String,
        usage: String,
        arguments: String,
        executor: String,
        owner_tag: Option<u64>,
    ) -> Self {
        let help_text = format_help(&description, &usage, &arguments);
        Self {
            name,
            description,
            usage,
            arguments,
            help_text,
            executor,
            owner_tag,
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

        let mut request = crate::PluginTaskRequest::new(
            "dynamic_command",
            serde_json::json!({"name": self.name, "args": arg_strings}),
        );
        request.executor = self.executor.clone();
        request.scene_scoped = false;
        request.owner_tag = self.owner_tag;
        ctx.request_task(crate::AsyncCommandRequest::Plugin(request))
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::None]
    }
}
