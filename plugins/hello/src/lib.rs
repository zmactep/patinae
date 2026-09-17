use patinae_plugin::prelude::*;
use patinae_plugin::{define_plugin_settings, patinae_plugin};

// Define typed settings for this plugin.
// `set hello_style, 1` / `get hello_style` work out of the box.
define_plugin_settings! {
    HelloSettings {
        style: i32 = 0, name = "hello_style";
    }
}

patinae_plugin! {
    name: "hello",
    description: "Example plugin: registers a 'hello' command with a configurable style setting",
    commands: [HelloCommand],
    settings: [HelloSettings],
    background_registration: true,
}

struct HelloCommand;

impl Command for HelloCommand {
    fn name(&self) -> &str {
        "hello"
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let name = args.get_str(0).unwrap_or("World");
        ctx.print(&format!("Greetings from plug-in system, {}!", name));
        Ok(())
    }

    command_help! {
        CMD "hello"
        DESCRIPTION ["greets the user from the plugin system."]
        REQUIRED []
        OPTIONAL [
            { "name", "string", "name to greet", "World" },
        ]
        EXAMPLES ["hello", "hello Alice"]
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::None]
    }
}
