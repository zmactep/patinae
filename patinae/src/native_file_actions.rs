use std::path::Path;

use patinae_cmd::{CommandExecutor, FormatHandler};
use patinae_framework::kernel::AppKernel;
use patinae_io::FileFormat;

/// Origin of a native file action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NativeFileSource {
    /// File path supplied on the process command line.
    StartupArgv,
    /// Future native menu Open action.
    #[cfg(target_os = "macos")]
    MenuOpen,
    /// Future native menu Run Script action.
    #[cfg(target_os = "macos")]
    MenuRunScript,
    /// File dropped onto the native window.
    DragDrop,
}

impl NativeFileSource {
    fn label(self) -> &'static str {
        match self {
            Self::StartupArgv => "startup file",
            #[cfg(target_os = "macos")]
            Self::MenuOpen => "menu Open file",
            #[cfg(target_os = "macos")]
            Self::MenuRunScript => "menu Run Script file",
            Self::DragDrop => "dropped file",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum NativeFileAction {
    Command(String),
    Warning(String),
}

/// Queue a native file action on the app command bus.
pub(crate) fn enqueue_file_action(kernel: &mut AppKernel, path: &Path, source: NativeFileSource) {
    match resolve_file_action(&kernel.executor, path, source) {
        NativeFileAction::Command(command) => kernel.bus.execute_command(command),
        NativeFileAction::Warning(message) => kernel.bus.print_warning(message),
    }
    kernel.bus.request_redraw();
}

fn resolve_file_action(
    executor: &CommandExecutor,
    path: &Path,
    source: NativeFileSource,
) -> NativeFileAction {
    let Some(path_text) = path.to_str() else {
        return NativeFileAction::Warning(format!(
            "Cannot open {}: path is not valid UTF-8: {}",
            source.label(),
            path.display()
        ));
    };

    let ext = path_extension_lower(path);
    let builtin_format = FileFormat::from_path(path);

    if ext.as_deref() == Some("pml") || registered_script_extension(executor, ext.as_deref()) {
        return NativeFileAction::Command(format!("run {}", quote_command_arg(path_text)));
    }

    if builtin_format.is_trajectory_only() {
        return NativeFileAction::Command(format!("load_traj {}", quote_command_arg(path_text)));
    }

    if is_builtin_load_format(builtin_format) || is_session_extension(ext.as_deref()) {
        return NativeFileAction::Command(format!("load {}", quote_command_arg(path_text)));
    }

    if let Some(ext) = ext.as_deref() {
        if let Some(handler) = executor.format_handlers().get(ext) {
            return plugin_format_action(handler, ext, path_text, source);
        }
    }

    NativeFileAction::Warning(format!(
        "Cannot open {} \"{}\": unsupported file extension {}. Use load, load_traj, or run explicitly.",
        source.label(),
        path.display(),
        display_extension(path)
    ))
}

fn registered_script_extension(executor: &CommandExecutor, ext: Option<&str>) -> bool {
    ext.is_some_and(|ext| executor.script_handlers().contains_key(ext))
}

fn plugin_format_action(
    handler: &FormatHandler,
    ext: &str,
    path_text: &str,
    source: NativeFileSource,
) -> NativeFileAction {
    if handler.reader.is_some() {
        NativeFileAction::Command(format!("load {}", quote_command_arg(path_text)))
    } else {
        NativeFileAction::Warning(format!(
            "Cannot open {} \"{}\": plugin format \"{}\" for .{} is write-only.",
            source.label(),
            path_text,
            handler.name,
            ext
        ))
    }
}

fn is_builtin_load_format(format: FileFormat) -> bool {
    format != FileFormat::Unknown && !format.is_trajectory_only()
}

fn is_session_extension(ext: Option<&str>) -> bool {
    matches!(ext, Some("prs" | "pse" | "pze"))
}

fn path_extension_lower(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
}

fn display_extension(path: &Path) -> String {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return "unknown".to_string();
    };
    let lower = file_name.to_ascii_lowercase();
    if let Some(stem) = lower.strip_suffix(".gz") {
        if let Some(inner) = Path::new(stem).extension().and_then(|ext| ext.to_str()) {
            return format!(".{inner}.gz");
        }
    }
    path_extension_lower(path)
        .map(|ext| format!(".{ext}"))
        .unwrap_or_else(|| "none".to_string())
}

/// Quote one command argument for Patinae's command parser.
pub(crate) fn quote_command_arg(s: &str) -> String {
    let needs_quote = s
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || matches!(c, ',' | ';' | '"' | '\'' | '\\'));
    if !needs_quote {
        return s.to_string();
    }

    let mut quoted = String::with_capacity(s.len() + 2);
    quoted.push('"');
    for c in s.chars() {
        match c {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            c => quoted.push(c),
        }
    }
    quoted.push('"');
    quoted
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use patinae_cmd::{CommandExecutor, FormatHandler};

    use super::*;

    fn command_for(path: &str, executor: &CommandExecutor) -> String {
        match resolve_file_action(executor, Path::new(path), NativeFileSource::DragDrop) {
            NativeFileAction::Command(command) => command,
            NativeFileAction::Warning(message) => {
                panic!("expected command, got warning: {message}")
            }
        }
    }

    fn warning_for(path: &str, executor: &CommandExecutor) -> String {
        match resolve_file_action(executor, Path::new(path), NativeFileSource::DragDrop) {
            NativeFileAction::Warning(message) => message,
            NativeFileAction::Command(command) => {
                panic!("expected warning, got command: {command}")
            }
        }
    }

    #[test]
    fn built_in_structure_session_and_map_formats_route_to_load() {
        let executor = CommandExecutor::new();

        assert_eq!(command_for("1fsd.pdb", &executor), "load 1fsd.pdb");
        assert_eq!(command_for("1fsd.cif.gz", &executor), "load 1fsd.cif.gz");
        assert_eq!(command_for("density.ccp4", &executor), "load density.ccp4");
        assert_eq!(command_for("session.prs", &executor), "load session.prs");
        assert_eq!(command_for("legacy.pse", &executor), "load legacy.pse");
        assert_eq!(command_for("legacy.pze", &executor), "load legacy.pze");
    }

    #[test]
    fn scripts_route_to_run() {
        let mut executor = CommandExecutor::new();
        executor.register_script_handler(
            "py",
            Arc::new(|path| {
                Ok(patinae_cmd::AsyncCommandRequest::Plugin(
                    patinae_cmd::PluginTaskRequest::new(
                        "python",
                        serde_json::json!({"path": path}),
                    ),
                ))
            }),
        );

        assert_eq!(command_for("setup.pml", &executor), "run setup.pml");
        assert_eq!(command_for("analysis.py", &executor), "run analysis.py");
    }

    #[test]
    fn trajectories_route_to_load_traj() {
        let executor = CommandExecutor::new();

        assert_eq!(
            command_for("trajectory.xtc", &executor),
            "load_traj trajectory.xtc"
        );
        assert_eq!(
            command_for("trajectory.trr", &executor),
            "load_traj trajectory.trr"
        );
    }

    #[test]
    fn readable_plugin_formats_route_to_load() {
        let mut executor = CommandExecutor::new();
        executor.register_format_handler(FormatHandler {
            name: "MMTF".into(),
            extensions: vec!["mmtf".into()],
            reader: Some(Arc::new(|_| Ok(Vec::new()))),
            writer: None,
        });

        assert_eq!(
            command_for("structure.mmtf", &executor),
            "load structure.mmtf"
        );
    }

    #[test]
    fn write_only_plugin_formats_warn() {
        let mut executor = CommandExecutor::new();
        executor.register_format_handler(FormatHandler {
            name: "ExportOnly".into(),
            extensions: vec!["xout".into()],
            reader: None,
            writer: Some(Arc::new(|_, _| Ok(()))),
        });

        let warning = warning_for("structure.xout", &executor);

        assert!(warning.contains("ExportOnly"));
        assert!(warning.contains("write-only"));
    }

    #[test]
    fn unknown_extensions_warn() {
        let executor = CommandExecutor::new();

        let warning = warning_for("doom.wad", &executor);

        assert!(warning.contains("unsupported file extension .wad"));
    }

    #[test]
    fn command_arguments_are_quoted_safely() {
        assert_eq!(quote_command_arg("simple.pdb"), "simple.pdb");
        assert_eq!(
            quote_command_arg("/tmp/patinae startup/input;1\".pdb"),
            "\"/tmp/patinae startup/input;1\\\".pdb\""
        );
        assert_eq!(
            quote_command_arg("C:\\data\\input.pdb"),
            "\"C:\\\\data\\\\input.pdb\""
        );
    }
}
