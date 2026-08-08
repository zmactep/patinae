//! Control commands: quit, reinitialize, refresh, rebuild, run

use patinae_scene::{DirtyFlags, Session};

use crate::args::ParsedCommand;
use crate::command::{
    ArgHint, BuiltinCommandCapability, Command, CommandContext, CommandRegistry, ViewerLike,
};
use crate::command_help;
use crate::error::{CmdError, CmdResult};
use crate::executor::CommandExecutor;
use crate::script::ScriptEngine;

use super::io::expand_path;

/// Register control commands
pub fn register(registry: &mut CommandRegistry) {
    registry.register(QuitCommand);
    registry.register(ClearCommand);
    registry.register(ReinitializeCommand);
    registry.register(RefreshCommand);
    registry.register(RebuildCommand);
    registry.register(HelpCommand);
    registry.register(RunCommand);
}

// ============================================================================
// quit command
// ============================================================================

struct QuitCommand;

impl Command for QuitCommand {
    fn name(&self) -> &str {
        "quit"
    }

    fn aliases(&self) -> &[&str] {
        &["exit"]
    }

    command_help! {
        CMD "quit"
        DESCRIPTION [
            "exits application.",
        ]
        REQUIRED []
        OPTIONAL [
            { "code", "integer", "exit code", "0" },
        ]
        EXAMPLES [
            "quit",
            "quit 1",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let _code = args.int_arg_or(0, "code", 0);

        ctx.quit();
        Ok(())
    }
}

// ============================================================================
// clear command
// ============================================================================

struct ClearCommand;

impl Command for ClearCommand {
    fn name(&self) -> &str {
        "clear"
    }

    command_help! {
        CMD "clear"
        DESCRIPTION [
            "clears the REPL output log.",
        ]
        REQUIRED []
        OPTIONAL []
        EXAMPLES [
            "clear",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let got = args.arg_count();
        if got != 0 {
            return Err(CmdError::too_many_arguments(0, got));
        }

        ctx.clear_output();
        Ok(())
    }
}

// ============================================================================
// reinitialize command
// ============================================================================

struct ReinitializeCommand;

impl Command for ReinitializeCommand {
    fn name(&self) -> &str {
        "reinitialize"
    }

    fn aliases(&self) -> &[&str] {
        &["reinit"]
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Keywords(&["everything", "settings", "objects"])]
    }

    command_help! {
        CMD "reinitialize"
        DESCRIPTION [
            "resets the application to its initial state.",
        ]
        REQUIRED []
        OPTIONAL [
            { "what", "string", "what to reinitialize: everything, settings, objects, purge_defaults", "everything" },
            { "object", "string", "specific object to reinitialize", "" },
        ]
        EXAMPLES [
            "reinitialize",
            "reinitialize settings",
            "reinitialize objects",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let what = args.str_arg_or(0, "what", "everything");

        match what.to_lowercase().as_str() {
            "everything" | "all" => {
                ctx.viewer.replace_session(Session::new());
            }
            "settings" => {
                *ctx.viewer.settings_mut() = patinae_settings::Settings::default();
                ctx.viewer.reconcile_recent_atoms();
            }
            "objects" => {
                ctx.viewer.clear_objects();
                ctx.viewer.selections_mut().clear();
                ctx.viewer.update_movie_state_count();
            }
            _ => {
                return Err(CmdError::invalid_arg(
                    "what",
                    format!("unknown reinitialize target: {}", what),
                ));
            }
        }

        ctx.viewer.request_redraw();

        if !ctx.quiet {
            ctx.print(&format!(" Reinitialized {}", what));
        }

        Ok(())
    }
}

// ============================================================================
// refresh command
// ============================================================================

struct RefreshCommand;

impl Command for RefreshCommand {
    fn name(&self) -> &str {
        "refresh"
    }

    command_help! {
        CMD "refresh"
        DESCRIPTION [
            "forces a redraw of the scene.",
        ]
        REQUIRED []
        OPTIONAL []
        EXAMPLES [
            "refresh",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        _args: &ParsedCommand,
    ) -> CmdResult {
        ctx.viewer.request_redraw();

        if !ctx.quiet {
            ctx.print(" Refreshed");
        }

        Ok(())
    }
}

// ============================================================================
// rebuild command
// ============================================================================

struct RebuildCommand;

impl Command for RebuildCommand {
    fn name(&self) -> &str {
        "rebuild"
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Selection]
    }

    command_help! {
        CMD "rebuild"
        DESCRIPTION [
            "forces rebuilding of all representations.",
        ]
        REQUIRED []
        OPTIONAL [
            { "selection", "string", "selection to rebuild", "all" },
        ]
        EXAMPLES [
            "rebuild",
            "rebuild protein",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let selection = args.str_arg_or(0, "selection", "all");

        // Get objects to rebuild
        let object_names: Vec<String> = if selection == "all" || selection == "*" {
            ctx.viewer
                .objects()
                .names()
                .map(|s| s.to_string())
                .collect()
        } else {
            ctx.viewer
                .objects()
                .matching(selection)
                .iter()
                .map(|s| s.to_string())
                .collect()
        };

        // Mark all molecule objects as dirty to force rebuild
        for name in &object_names {
            if let Some(mol) = ctx.viewer.objects_mut().get_molecule_mut(name) {
                mol.invalidate(DirtyFlags::ALL);
            }
        }

        ctx.viewer.request_redraw();

        if !ctx.quiet {
            ctx.print(&format!(" Rebuilt \"{}\"", selection));
        }

        Ok(())
    }
}

// ============================================================================
// help command
// ============================================================================

struct HelpCommand;

impl Command for HelpCommand {
    fn name(&self) -> &str {
        "help"
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Command]
    }

    command_help! {
        CMD "help"
        DESCRIPTION [
            "displays help for commands.",
        ]
        REQUIRED []
        OPTIONAL [
            { "command", "string", "command to get help for", "list commands" },
        ]
        EXAMPLES [
            "help",
            "help load",
            "help zoom",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let command = args.str_arg(0, "command");

        if let Some(cmd_name) = command {
            // Show help for specific command
            let cmd = ctx.registry().and_then(|r| r.get(cmd_name));
            match cmd {
                Some(cmd) => ctx.print(cmd.help()),
                None => ctx.print(&format!(" Unknown command: '{}'", cmd_name)),
            }
        } else {
            // List available commands from registry
            let lines: Vec<String> = ctx
                .registry()
                .map(|registry| {
                    let mut names: Vec<&str> = registry.names().collect();
                    names.sort();
                    names
                        .chunks(5)
                        .map(|chunk| format!("   {}", chunk.join(", ")))
                        .collect()
                })
                .unwrap_or_default();
            if !lines.is_empty() {
                ctx.print(" Available commands:");
                for line in &lines {
                    ctx.print(line);
                }
            }
            ctx.print("");
            ctx.print(" Type 'help <command>' for detailed help");
        }

        Ok(())
    }
}

// ============================================================================
// run command
// ============================================================================

pub(crate) const RUN_CAPABILITY: BuiltinCommandCapability = BuiltinCommandCapability {
    command: "run",
    aliases: &["@"],
    suffixes: &["pml"],
    available: true,
};

struct RunCommand;

impl Command for RunCommand {
    fn name(&self) -> &str {
        RUN_CAPABILITY.command
    }

    fn aliases(&self) -> &[&str] {
        RUN_CAPABILITY.aliases
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Path]
    }

    command_help! {
        CMD "run"
        DESCRIPTION [
            "executes a script file. Supports .pml (PyMOL script) and any",
            "file type registered by a plugin (e.g., .py via the Python plugin).",
        ]
        REQUIRED [
            { "filename", "string", "path to script file" },
        ]
        OPTIONAL []
        EXAMPLES [
            "run setup.pml",
            "run ~/scripts/analysis.pml",
            "run script.py",
            "@ script.pml",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let filename = args
            .str_arg(0, "filename")
            .ok_or_else(|| CmdError::missing_argument("filename".to_string()))?;

        let path = expand_path(filename);
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("pml");

        if RUN_CAPABILITY.supports_suffix(ext) {
            let executor = if let Some(registry) = ctx.registry() {
                CommandExecutor::with_registry(
                    registry.clone(),
                    ctx.script_handlers_map().cloned().unwrap_or_default(),
                    ctx.format_handlers_map().cloned().unwrap_or_default(),
                    ctx.clone_dynamic_settings_for_child_executor(),
                )
                .with_loaded_plugin_capabilities(
                    ctx.clone_loaded_plugin_capabilities_for_child_executor(),
                )
            } else {
                CommandExecutor::new()
            };
            let mut engine = ScriptEngine::with_executor(executor);
            engine.run_pml(ctx.viewer, &path)
        } else if let Some(handler) = ctx.script_handler(ext) {
            let path_str = path
                .to_str()
                .ok_or_else(|| CmdError::execution("invalid path encoding".to_string()))?;
            handler(path_str).map_err(CmdError::execution)
        } else {
            Err(CmdError::execution(format!(
                "no handler for .{} files. Install a plugin that handles this format.",
                ext
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        CmdError, CmdResult, Command, CommandAction, CommandContext, CommandExecutor,
        DynamicCommand, LoadedPluginCapability, ParsedCommand, ViewerLike,
    };
    use patinae_mol::{AtomBuilder, AtomIndex, ObjectMolecule};
    use patinae_scene::{MoleculeObject, Session, SessionAdapter};
    use patinae_settings::groups::RecentPickLimit;
    use std::sync::{Arc, Mutex};

    struct CaptureLoadedPlugins {
        observed: Arc<Mutex<Vec<LoadedPluginCapability>>>,
    }

    impl Command for CaptureLoadedPlugins {
        fn name(&self) -> &str {
            "capture_loaded_plugins"
        }

        fn execute<'v, 'r>(
            &self,
            ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
            _args: &ParsedCommand,
        ) -> CmdResult {
            *self
                .observed
                .lock()
                .expect("captured plugin metadata should be available") =
                ctx.loaded_plugin_capabilities().to_vec();
            Ok(())
        }
    }

    #[test]
    fn run_capability_shares_the_registered_alias_and_builtin_pml_suffix() {
        let executor = CommandExecutor::new();

        assert_eq!(super::RUN_CAPABILITY.command, "run");
        assert_eq!(super::RUN_CAPABILITY.aliases, &["@"]);
        assert_eq!(super::RUN_CAPABILITY.suffixes, &["pml"]);
        assert!(executor.registry().contains("@"));
    }

    fn execute_control_command(command: &str) -> Result<crate::CommandOutput, CmdError> {
        let mut session = Session::new();
        let mut needs_redraw = false;
        let mut adapter = SessionAdapter {
            session: &mut session,
            render_context: None,
            default_size: (800, 600),
            needs_redraw: &mut needs_redraw,
            async_fetch_fn: None,
        };
        let mut executor = CommandExecutor::new();

        executor.do_with_options(&mut adapter, command, false)
    }

    fn execute_in_session(session: &mut Session, command: &str) -> Result<(), CmdError> {
        let mut needs_redraw = false;
        let mut adapter = SessionAdapter {
            session,
            render_context: None,
            default_size: (800, 600),
            needs_redraw: &mut needs_redraw,
            async_fetch_fn: None,
        };
        CommandExecutor::new().do_(&mut adapter, command)
    }

    fn add_case_colliding_molecule(session: &mut Session) -> String {
        let mut molecule = ObjectMolecule::new("obj");
        for name in ["CA", "ca"] {
            molecule.add_atom(
                AtomBuilder::new()
                    .name(name)
                    .element_symbol("C")
                    .resn("GLY")
                    .resv(1)
                    .chain("A")
                    .build(),
            );
        }
        session
            .registry
            .add(MoleculeObject::with_name(molecule, "obj"));
        let molecule = session.registry.get_molecule("obj").unwrap();
        patinae_scene::canonical_atom_path_for_hit(
            &patinae_scene::PickHit {
                object_name: "obj".to_string(),
                object_type: patinae_scene::ObjectType::Molecule,
                atom_index: Some(AtomIndex(0)),
                position: lin_alg::f32::Vec3::new(0.0, 0.0, 0.0),
                distance: 0.0,
            },
            molecule.molecule(),
        )
        .unwrap()
    }

    fn temp_script_path(name: &str) -> std::path::PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("patinae-{name}-{nonce}.pml"))
    }

    #[test]
    fn clear_command_is_registered_and_documented() {
        let output = execute_control_command("help clear").unwrap();

        assert!(output
            .messages
            .iter()
            .any(|msg| msg.text.contains("\"clear\" clears the REPL output log.")));
    }

    #[test]
    fn clear_command_requests_output_clear() {
        let output = execute_control_command("clear").unwrap();

        assert!(output.messages.is_empty());
        assert!(matches!(
            output.actions.as_slice(),
            [CommandAction::ClearOutput]
        ));
    }

    #[test]
    fn clear_command_rejects_arguments() {
        let err = execute_control_command("clear extra").unwrap_err();

        assert!(err.is_too_many_arguments());
    }

    #[test]
    fn reinitialize_objects_clears_recent_atoms_and_settings_revalidates_them() {
        let mut objects_session = Session::new();
        objects_session.settings.behavior.ignore_case = false;
        let path = add_case_colliding_molecule(&mut objects_session);
        objects_session
            .recent_atoms
            .insert(path, RecentPickLimit::Unlimited);

        execute_in_session(&mut objects_session, "reinitialize objects").unwrap();
        assert!(objects_session.registry.is_empty());
        assert!(objects_session.recent_atoms.is_empty());

        let mut settings_session = Session::new();
        settings_session.settings.behavior.ignore_case = false;
        let path = add_case_colliding_molecule(&mut settings_session);
        settings_session
            .recent_atoms
            .insert(path, RecentPickLimit::Unlimited);

        execute_in_session(&mut settings_session, "reinitialize settings").unwrap();
        assert!(settings_session.settings.behavior.ignore_case);
        assert!(settings_session.recent_atoms.is_empty());
    }

    #[test]
    fn run_command_inherits_plugin_registered_commands() {
        let invocations = Arc::new(Mutex::new(Vec::new()));
        let mut session = Session::new();
        let mut needs_redraw = false;
        let mut adapter = SessionAdapter {
            session: &mut session,
            render_context: None,
            default_size: (800, 600),
            needs_redraw: &mut needs_redraw,
            async_fetch_fn: None,
        };
        let mut executor = CommandExecutor::new();
        executor
            .registry_mut()
            .register_boxed(Box::new(DynamicCommand::new(
                "plugin_marker".into(),
                "test plugin command".into(),
                "plugin_marker value".into(),
                "value: marker payload".into(),
                invocations.clone(),
            )));
        let script_path = temp_script_path("run-plugin-command");
        std::fs::write(&script_path, "plugin_marker alpha\n")
            .expect("test script should be writable");
        let command = format!("run \"{}\"", script_path.display());

        let result = executor.do_with_options(&mut adapter, &command, false);
        let _ = std::fs::remove_file(&script_path);

        result.expect("run should execute plugin-registered command");
        let invocations = invocations
            .lock()
            .expect("dynamic invocation list should be available");
        assert_eq!(invocations.len(), 1);
        assert_eq!(invocations[0].name, "plugin_marker");
        assert_eq!(invocations[0].args, vec!["alpha"]);
    }

    #[test]
    fn run_command_child_inherits_loaded_plugin_metadata() {
        let observed = Arc::new(Mutex::new(Vec::new()));
        let mut session = Session::new();
        let mut needs_redraw = false;
        let mut adapter = SessionAdapter {
            session: &mut session,
            render_context: None,
            default_size: (800, 600),
            needs_redraw: &mut needs_redraw,
            async_fetch_fn: None,
        };
        let mut executor = CommandExecutor::new();
        executor
            .registry_mut()
            .register_boxed(Box::new(CaptureLoadedPlugins {
                observed: observed.clone(),
            }));
        let expected = LoadedPluginCapability {
            name: "python".to_string(),
            version: "1.2.3".to_string(),
            description: "Python integration".to_string(),
        };
        executor.record_loaded_plugin_capability(expected.clone());
        let script_path = temp_script_path("run-plugin-metadata");
        std::fs::write(&script_path, "capture_loaded_plugins\n")
            .expect("test script should be writable");
        let command = format!("run \"{}\"", script_path.display());

        let result = executor.do_with_options(&mut adapter, &command, false);
        let _ = std::fs::remove_file(&script_path);

        result.expect("run should preserve loaded-plugin metadata");
        assert_eq!(
            observed
                .lock()
                .expect("captured plugin metadata should be available")
                .as_slice(),
            &[expected]
        );
    }
}
