//! Runtime capability reporting.

use std::collections::BTreeSet;

use patinae_io::FileFormat;

use crate::args::ParsedCommand;
use crate::command::{
    ArgHint, BuiltinCommandCapability, Command, CommandContext, CommandRegistry,
    CommandRuntimeRequirements, FormatHandler, ViewerLike,
};
use crate::command_help;
use crate::error::{CmdError, CmdResult};

use super::control::RUN_CAPABILITY;
use super::io::{LOAD_CAPABILITY, LOAD_TRAJ_CAPABILITY, SAVE_CAPABILITY};

const TOPIC_HINTS: &[&str] = &["plugins", "formats"];
const FORMAT_HINTS: &[&str] = &["run", "load", "load_traj", "save"];
const EMPTY_METADATA_FIELD: &str = "\"\"";

/// Registers runtime capability introspection.
pub fn register(registry: &mut CommandRegistry) {
    registry.register(CapabilitiesCommand);
}

struct CapabilitiesCommand;

impl Command for CapabilitiesCommand {
    fn name(&self) -> &str {
        "capabilities"
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[
            ArgHint::Keywords(TOPIC_HINTS),
            ArgHint::Keywords(FORMAT_HINTS),
        ]
    }

    command_help! {
        CMD "capabilities"
        DESCRIPTION [
            "reports effective plugins and file formats in the current runtime.",
        ]
        REQUIRED []
        OPTIONAL [
            { "topic", "string", "plugins or formats", "short topic index" },
            { "leaf", "string", "run, load, load_traj, or save", "format index" },
        ]
        EXAMPLES [
            "capabilities",
            "capabilities plugins",
            "capabilities formats",
            "capabilities formats run",
            "capabilities formats load",
            "capabilities formats load_traj",
            "capabilities formats save",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let path = capability_path(args);
        let got = path.len();
        if got == 0 {
            ctx.print("Capabilities:\n  plugins\n  formats");
            return Ok(());
        }
        let topic = &path[0];
        match topic.as_str() {
            "plugins" => {
                if got > 1 {
                    return Err(CmdError::too_many_arguments(1, got));
                }
                ctx.print(&render_plugins(ctx));
            }
            "formats" => {
                if got == 1 {
                    ctx.print("Format capabilities:\n  run\n  load\n  load_traj\n  save");
                } else {
                    if got > 2 {
                        return Err(CmdError::too_many_arguments(2, got));
                    }
                    let leaf = path[1].trim();
                    if leaf.is_empty() || matches!(leaf, "\"\"" | "''") {
                        return Err(CmdError::missing_argument(
                            "leaf after 'capabilities formats'",
                        ));
                    }
                    ctx.print(&render_formats(ctx, leaf)?);
                }
            }
            _ => {
                return Err(CmdError::invalid_arg(
                    "topic",
                    format!(
                        "unknown capabilities topic '{topic}'; expected 'plugins' or 'formats'"
                    ),
                ));
            }
        }

        Ok(())
    }

    fn runtime_requirements(&self) -> CommandRuntimeRequirements {
        CommandRuntimeRequirements::NONE
    }
}

fn capability_path(args: &ParsedCommand) -> Vec<String> {
    let mut path = Vec::new();
    for (name, value) in &args.args {
        if let Some(name) = name {
            path.push(format!("{name}={value}"));
            continue;
        }

        let value = value.to_string();
        if value.is_empty() {
            path.push(value);
        } else {
            path.extend(value.split_whitespace().map(str::to_string));
        }
    }
    path
}

fn render_plugins(ctx: &CommandContext<'_, '_, dyn ViewerLike + '_>) -> String {
    let mut rows = ctx
        .loaded_plugin_capabilities()
        .iter()
        .map(|plugin| {
            (
                normalize_metadata(&plugin.name),
                normalize_metadata(&plugin.version),
                normalize_metadata(&plugin.description),
            )
        })
        .collect::<Vec<_>>();
    rows.sort();

    let rows = if rows.is_empty() {
        "(none)".to_string()
    } else {
        rows.into_iter()
            .map(|(name, version, description)| format!("{name}\t{version}\t{description}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!("Loaded plugins:\n{rows}")
}

fn normalize_metadata(value: &str) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        EMPTY_METADATA_FIELD.to_string()
    } else {
        normalized
    }
}

fn render_formats(
    ctx: &CommandContext<'_, '_, dyn ViewerLike + '_>,
    leaf: &str,
) -> CmdResult<String> {
    let (capability, plugin_extensions) = match leaf {
        "run" => (RUN_CAPABILITY, script_extensions(ctx, RUN_CAPABILITY)),
        "load" => (
            LOAD_CAPABILITY,
            format_extensions(ctx, LOAD_CAPABILITY, FormatAccess::Read),
        ),
        "load_traj" => (LOAD_TRAJ_CAPABILITY, BTreeSet::new()),
        "save" => (
            SAVE_CAPABILITY,
            format_extensions(ctx, SAVE_CAPABILITY, FormatAccess::Write),
        ),
        _ => {
            return Err(CmdError::invalid_arg(
                "leaf",
                format!(
                    "unknown capabilities format '{leaf}'; expected 'run', 'load', 'load_traj', or 'save'"
                ),
            ));
        }
    };

    if !capability.available {
        return Ok(format!("Formats for {leaf}:\n(unavailable)"));
    }

    let mut rows = capability
        .suffixes
        .iter()
        .map(|extension| format!(".{extension}"))
        .collect::<BTreeSet<_>>();
    rows.extend(plugin_extensions);

    let body = if rows.is_empty() {
        "(none)".to_string()
    } else {
        rows.into_iter().collect::<Vec<_>>().join("\n")
    };
    Ok(format!("Formats for {leaf}:\n{body}"))
}

fn script_extensions(
    ctx: &CommandContext<'_, '_, dyn ViewerLike + '_>,
    capability: BuiltinCommandCapability,
) -> BTreeSet<String> {
    ctx.script_handlers_map()
        .into_iter()
        .flat_map(|handlers| handlers.keys())
        .filter_map(|extension| canonical_extension(extension))
        .filter(|extension| !capability.supports_suffix(extension))
        .map(|extension| format!(".{extension}"))
        .collect()
}

#[derive(Clone, Copy)]
enum FormatAccess {
    Read,
    Write,
}

fn format_extensions(
    ctx: &CommandContext<'_, '_, dyn ViewerLike + '_>,
    capability: BuiltinCommandCapability,
    access: FormatAccess,
) -> BTreeSet<String> {
    ctx.format_handlers_map()
        .into_iter()
        .flat_map(|handlers| handlers.iter())
        .filter(|(_, handler)| supports_access(handler, access))
        .filter_map(|(extension, _)| canonical_extension(extension))
        .filter(|extension| !builtin_claims_format(extension, capability, access))
        .map(|extension| format!(".{extension}"))
        .collect()
}

fn supports_access(handler: &FormatHandler, access: FormatAccess) -> bool {
    match access {
        FormatAccess::Read => handler.reader.is_some(),
        FormatAccess::Write => handler.writer.is_some(),
    }
}

fn builtin_claims_format(
    extension: &str,
    capability: BuiltinCommandCapability,
    access: FormatAccess,
) -> bool {
    if capability.supports_suffix(extension) {
        return true;
    }

    let recognized = FileFormat::from_extension(extension);
    match access {
        FormatAccess::Read => recognized.is_trajectory_only(),
        FormatAccess::Write => recognized != FileFormat::Unknown || extension == "pse",
    }
}

fn canonical_extension(extension: &str) -> Option<&str> {
    let normalized = normalized_extension(extension)?;
    (extension == normalized
        && !extension.contains('.')
        && !extension.chars().any(char::is_whitespace))
    .then_some(extension)
}

fn normalized_extension(extension: &str) -> Option<String> {
    let extension = extension
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase();
    (!extension.is_empty()).then_some(extension)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use patinae_scene::{Session, SessionAdapter};

    use crate::{
        CmdError, CommandExecutor, CommandOutput, FormatHandler, LoadedPluginCapability,
        MessageKind,
    };

    fn execute(executor: &mut CommandExecutor, command: &str) -> Result<CommandOutput, CmdError> {
        let mut session = Session::new();
        let mut needs_redraw = false;
        let mut adapter = SessionAdapter {
            session: &mut session,
            render_context: None,
            default_size: (64, 64),
            needs_redraw: &mut needs_redraw,
            async_fetch_fn: None,
        };

        executor.do_with_options(&mut adapter, command, false)
    }

    fn execute_text(executor: &mut CommandExecutor, command: &str) -> String {
        let output = execute(executor, command).expect("capabilities command should succeed");
        assert_eq!(output.messages.len(), 1);
        assert_eq!(output.messages[0].kind, MessageKind::Info);
        output.messages[0].text.clone()
    }

    fn empty_reader() -> crate::PluginReaderFn {
        Arc::new(|_| Ok(Vec::new()))
    }

    fn empty_writer() -> crate::PluginWriterFn {
        Arc::new(|_, _| Ok(()))
    }

    fn register_format(
        executor: &mut CommandExecutor,
        name: &str,
        extensions: &[&str],
        readable: bool,
        writable: bool,
    ) {
        executor.register_format_handler(FormatHandler {
            name: name.to_string(),
            extensions: extensions
                .iter()
                .map(|extension| (*extension).to_string())
                .collect(),
            reader: readable.then(empty_reader),
            writer: writable.then(empty_writer),
        });
    }

    #[test]
    fn all_seven_valid_forms_have_exact_stable_output() {
        let mut executor = CommandExecutor::new();

        assert_eq!(
            execute_text(&mut executor, "capabilities"),
            "Capabilities:\n  plugins\n  formats"
        );
        assert_eq!(
            execute_text(&mut executor, "capabilities plugins"),
            "Loaded plugins:\n(none)"
        );
        assert_eq!(
            execute_text(&mut executor, "capabilities formats"),
            "Format capabilities:\n  run\n  load\n  load_traj\n  save"
        );
        assert_eq!(
            execute_text(&mut executor, "capabilities formats run"),
            "Formats for run:\n.pml"
        );
        assert_eq!(
            execute_text(&mut executor, "capabilities formats load"),
            concat!(
                "Formats for load:\n",
                ".bcif\n",
                ".bcif.gz\n",
                ".ccp4\n",
                ".ccp4.gz\n",
                ".cif\n",
                ".cif.gz\n",
                ".ent\n",
                ".ent.gz\n",
                ".gro\n",
                ".gro.gz\n",
                ".map\n",
                ".map.gz\n",
                ".ml2\n",
                ".ml2.gz\n",
                ".mmcif\n",
                ".mmcif.gz\n",
                ".mol\n",
                ".mol.gz\n",
                ".mol2\n",
                ".mol2.gz\n",
                ".mrc\n",
                ".mrc.gz\n",
                ".pdb\n",
                ".pdb.gz\n",
                ".prs\n",
                ".pse\n",
                ".pze\n",
                ".sd\n",
                ".sd.gz\n",
                ".sdf\n",
                ".sdf.gz\n",
                ".xyz\n",
                ".xyz.gz"
            )
        );
        assert_eq!(
            execute_text(&mut executor, "capabilities formats load_traj"),
            if cfg!(feature = "traj") {
                "Formats for load_traj:\n.trr\n.trr.gz\n.xtc\n.xtc.gz"
            } else {
                "Formats for load_traj:\n(unavailable)"
            }
        );
        assert_eq!(
            execute_text(&mut executor, "capabilities formats save"),
            concat!(
                "Formats for save:\n",
                ".cif\n",
                ".ent\n",
                ".gro\n",
                ".ml2\n",
                ".mmcif\n",
                ".mol\n",
                ".mol2\n",
                ".pdb\n",
                ".pml\n",
                ".prs\n",
                ".sd\n",
                ".sdf\n",
                ".xyz"
            )
        );
    }

    #[test]
    fn plugin_rows_are_normalized_sorted_and_not_deduplicated() {
        let mut executor = CommandExecutor::new();
        for capability in [
            LoadedPluginCapability {
                name: "".to_string(),
                version: " \t ".to_string(),
                description: "".to_string(),
            },
            LoadedPluginCapability {
                name: " \n".to_string(),
                version: "".to_string(),
                description: "\t".to_string(),
            },
            LoadedPluginCapability {
                name: "zeta\tplugin".to_string(),
                version: "2.0".to_string(),
                description: "last\nrow".to_string(),
            },
            LoadedPluginCapability {
                name: " alpha\nplugin ".to_string(),
                version: " 2.0 ".to_string(),
                description: "second\tdescription".to_string(),
            },
            LoadedPluginCapability {
                name: "alpha\tplugin".to_string(),
                version: "1.0\r\nrc1".to_string(),
                description: " first  description ".to_string(),
            },
            LoadedPluginCapability {
                name: "alpha plugin".to_string(),
                version: "1.0 rc1".to_string(),
                description: "first description".to_string(),
            },
        ] {
            executor.record_loaded_plugin_capability(capability);
        }

        assert_eq!(
            execute_text(&mut executor, "capabilities plugins"),
            concat!(
                "Loaded plugins:\n",
                "\"\"\t\"\"\t\"\"\n",
                "\"\"\t\"\"\t\"\"\n",
                "alpha plugin\t1.0 rc1\tfirst description\n",
                "alpha plugin\t1.0 rc1\tfirst description\n",
                "alpha plugin\t2.0\tsecond description\n",
                "zeta plugin\t2.0\tlast row"
            )
        );
    }

    #[test]
    fn format_rows_use_winning_maps_access_modes_and_builtin_precedence() {
        let mut executor = CommandExecutor::new();
        register_format(&mut executor, "reader", &["reader_only"], true, false);
        register_format(&mut executor, "writer", &["writer_only"], false, true);
        register_format(&mut executor, "both", &["Both", "both"], true, true);
        register_format(
            &mut executor,
            "shadowed",
            &["pdb", "xtc", "bcif"],
            true,
            true,
        );
        register_format(&mut executor, "loser", &["winner"], true, false);
        register_format(&mut executor, "winner", &["winner"], false, true);
        register_format(
            &mut executor,
            "noncanonical",
            &[".dotted", "UPPER", " spaced ", "foo.gz"],
            true,
            true,
        );
        executor.register_script_handler("pml", Arc::new(|_| Ok(())));
        executor.register_script_handler("zed", Arc::new(|_| Ok(())));
        executor.register_script_handler(".dotted", Arc::new(|_| Ok(())));
        executor.register_script_handler("UPPER", Arc::new(|_| Ok(())));
        executor.register_script_handler("foo.gz", Arc::new(|_| Ok(())));

        let load = execute_text(&mut executor, "capabilities formats load");
        assert!(load.contains("\n.both\n"));
        assert_eq!(load.matches("\n.both").count(), 1);
        assert!(load.contains("\n.reader_only\n"));
        assert!(!load.contains("writer_only"));
        assert!(!load.contains("winner"));
        assert_eq!(load.matches("\n.pdb").count(), 2);
        assert_eq!(load.matches("\n.bcif").count(), 2);
        assert!(!load.contains("\n.xtc"));
        assert!(!load.contains("dotted"));
        assert!(!load.contains("upper"));
        assert!(!load.contains("spaced"));
        assert!(!load.contains("foo.gz"));

        let save = execute_text(&mut executor, "capabilities formats save");
        assert!(save.contains("\n.both\n"));
        assert_eq!(save.matches("\n.both").count(), 1);
        assert!(save.contains("\n.writer_only\n"));
        assert!(save.contains("\n.winner\n"));
        assert!(!save.contains("reader_only"));
        assert_eq!(save.matches("\n.pdb").count(), 1);
        assert!(!save.contains("\n.bcif"));
        assert!(!save.contains("\n.xtc"));
        assert!(!save.contains("dotted"));
        assert!(!save.contains("upper"));
        assert!(!save.contains("spaced"));
        assert!(!save.contains("foo.gz"));

        let run = execute_text(&mut executor, "capabilities formats run");
        assert_eq!(run, "Formats for run:\n.pml\n.zed");
        assert!(!run.contains("foo.gz"));
    }

    #[test]
    fn every_invalid_path_class_has_exact_error_text() {
        let cases = [
            (
                "capabilities unknown",
                "invalid argument 'topic': unknown capabilities topic 'unknown'; expected 'plugins' or 'formats'",
            ),
            (
                "capabilities formats read",
                "invalid argument 'leaf': unknown capabilities format 'read'; expected 'run', 'load', 'load_traj', or 'save'",
            ),
            (
                "capabilities formats, \"\"",
                "missing required argument: leaf after 'capabilities formats'",
            ),
            (
                "capabilities plugins extra",
                "too many arguments: expected at most 1, got 2",
            ),
            (
                "capabilities formats load extra",
                "too many arguments: expected at most 2, got 3",
            ),
        ];

        for (command, expected) in cases {
            let error = execute(&mut CommandExecutor::new(), command)
                .expect_err("invalid capabilities path should fail");
            assert_eq!(error.to_string(), expected, "command: {command}");
        }
    }

    #[test]
    fn capabilities_help_documents_the_exact_surface() {
        let text = execute_text(&mut CommandExecutor::new(), "help capabilities");

        for invocation in [
            "capabilities",
            "capabilities plugins",
            "capabilities formats",
            "capabilities formats run",
            "capabilities formats load",
            "capabilities formats load_traj",
            "capabilities formats save",
        ] {
            assert!(text.contains(invocation), "missing help form: {invocation}");
        }
    }

    #[test]
    fn rendered_text_has_no_trailing_whitespace_or_terminal_newline() {
        let mut executor = CommandExecutor::new();
        executor.record_loaded_plugin_capability(LoadedPluginCapability {
            name: " plugin\t".to_string(),
            version: " version\n".to_string(),
            description: "\r\n".to_string(),
        });

        for command in [
            "capabilities",
            "capabilities plugins",
            "capabilities formats",
            "capabilities formats run",
            "capabilities formats load",
            "capabilities formats load_traj",
            "capabilities formats save",
        ] {
            let text = execute_text(&mut executor, command);
            assert!(!text.ends_with('\n'), "terminal newline for {command}");
            assert!(
                text.lines().all(|line| line.trim_end() == line),
                "trailing whitespace for {command}: {text:?}"
            );
        }
    }
}
