//! Python Plugin Commands
//!
//! Implements commands for the Python plugin:
//! - `python` (alias `/`) — execute Python code inline
//! - `iterate` — execute a Python expression for each atom in a selection (read-only)
//! - `alter` — like iterate, but allows modifying atom properties
//!
//! All commands are non-blocking: code is submitted to the Python worker
//! thread, and output appears asynchronously via the poll cycle.

use patinae_plugin::prelude::*;
#[cfg(test)]
use patinae_scene::ObjectRegistry;

#[cfg(test)]
use crate::shared::SharedStateHandle;
use patinae_plugin::prelude::{AsyncCommandRequest, PluginTaskRequest};

// =============================================================================
// Shared state sync
// =============================================================================

/// Populate molecule snapshots for inline command fixtures.
#[cfg(test)]
pub(crate) fn sync_shared_molecules(
    shared: &SharedStateHandle,
    viewer: &dyn ViewerLike,
    force: bool,
) -> bool {
    sync_shared_molecules_from_registry(shared, viewer.objects(), force)
}

#[cfg(test)]
pub(crate) fn sync_shared_molecules_from_registry(
    shared: &SharedStateHandle,
    registry: &ObjectRegistry,
    force: bool,
) -> bool {
    let generation = registry.generation();
    let mut state = shared.lock().unwrap();
    if !force && state.molecule_generation == Some(generation) {
        return false;
    }
    state.names = registry.names().map(|s| s.to_string()).collect();
    state.molecules.clear();
    for name in registry.names() {
        if let Some(mol_obj) = registry.get_molecule(name) {
            state
                .molecules
                .push((name.to_string(), mol_obj.molecule().clone()));
        }
    }
    state.molecule_generation = Some(generation);
    true
}

// =============================================================================
// Helpers
// =============================================================================

/// Collect all arguments as strings, preserving order.
///
/// Named arguments are reconstructed as `key=value`. String values in
/// named args are re-quoted (the parser strips quotes, but we need them
/// for Python expressions like `chain="C"`).
fn collect_all_args(args: &ParsedCommand) -> Vec<String> {
    args.args
        .iter()
        .filter_map(|(name, val)| {
            let repr = val.to_string_repr()?;
            match name {
                Some(key) => {
                    // Re-quote string values: the parser strips quotes from
                    // `chain="C"` → String("C"), but Python needs `chain="C"`.
                    let rhs = if val.as_str().is_some() {
                        format!("\"{}\"", repr)
                    } else {
                        repr
                    };
                    Some(format!("{}={}", key, rhs))
                }
                None => Some(repr),
            }
        })
        .collect()
}

fn split_raw_atom_args(raw_args: &str) -> Option<(&str, &str)> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut string_char = '\0';
    let mut escaped = false;

    for (index, ch) in raw_args.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == string_char {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' | '\'' => {
                in_string = true;
                string_char = ch;
            }
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                let expression_start = index + ch.len_utf8();
                return Some((
                    raw_args[..index].trim(),
                    raw_args[expression_start..].trim(),
                ));
            }
            _ => {}
        }
    }

    None
}

/// Escape a string for inclusion in Python source code (single-quoted).
fn python_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

fn atom_expression(args: &ParsedCommand, reconstructed_args: &[String]) -> Option<String> {
    args.raw_args()
        .and_then(split_raw_atom_args)
        .map(|(_, expression)| expression)
        .filter(|expression| !expression.is_empty())
        .map(str::to_string)
        .or_else(|| Some(reconstructed_args[1..].join(", ")))
}

fn build_atom_command_code(args: &ParsedCommand, method: &str) -> Option<String> {
    let all_args = collect_all_args(args);
    if all_args.len() < 2 {
        return None;
    }

    let selection = &all_args[0];
    let expression = atom_expression(args, &all_args)?;

    Some(format!(
        "cmd.{}('{}', '{}')",
        method,
        python_escape(selection),
        python_escape(&expression),
    ))
}

/// Shared logic for `iterate` and `alter` commands.
///
/// Parses `<selection>, <expression>` from args, wraps into
/// `cmd.<method>('<selection>', '<expression>')`, and submits to the worker.
fn submit_atom_command(
    ctx: &mut CommandContext<'_, '_, dyn ViewerLike + '_>,
    args: &ParsedCommand,
    method: &str,
) -> CmdResult {
    let Some(code) = build_atom_command_code(args, method) else {
        ctx.print(&format!("Usage: {} <selection>, <expression>", method));
        return Ok(());
    };

    ctx.request_task(python_request(code, "command"))
}

// =============================================================================
// PythonCommand
// =============================================================================

/// The `python` command — executes Python code inline.
///
/// Usage:
///   python print("hello")
///   /import math; print(math.pi)
pub struct PythonCommand;

pub(crate) fn python_request(code: String, origin: &str) -> AsyncCommandRequest {
    AsyncCommandRequest::Plugin(python_task(
        serde_json::json!({"code": code, "origin": origin}),
    ))
}

pub(crate) fn python_task(payload: serde_json::Value) -> PluginTaskRequest {
    let mut request = PluginTaskRequest::new("python", payload);
    // A script may intentionally replace its own scene; each child captures
    // the scene at admission and each mutation is acknowledged by the host.
    request.scene_scoped = false;
    request
}

impl Command for PythonCommand {
    fn argument_syntax(&self) -> patinae_plugin::prelude::ArgumentSyntax {
        patinae_plugin::prelude::ArgumentSyntax::Verbatim
    }

    fn name(&self) -> &str {
        "python"
    }

    fn aliases(&self) -> &[&str] {
        &["/"]
    }

    fn runtime_requirements(&self) -> CommandRuntimeRequirements {
        CommandRuntimeRequirements::NONE
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let code = args.raw_args().unwrap_or("").trim();
        if code.is_empty() {
            ctx.print("Usage: python <code>  or  /<code>");
            return Ok(());
        }

        ctx.request_task(python_request(code.to_string(), "command"))
    }

    fn help(&self) -> &str {
        "python <code>\n/code\n\n\
         Execute Python code.\n\n\
         Examples:\n\
             python print(\"hello\")\n\
             /import math; print(math.pi)\n\
             python from patinae import cmd; cmd.color(\"red\", \"all\")"
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::None]
    }
}

// =============================================================================
// IterateCommand
// =============================================================================

/// The `iterate` command — executes a Python expression for each atom
/// in a selection (read-only).
///
/// Usage:
///   iterate all, print(name, resn, b)
///   iterate chain A, mylist.append(b)
pub struct IterateCommand;

impl Command for IterateCommand {
    fn name(&self) -> &str {
        "iterate"
    }

    fn runtime_requirements(&self) -> CommandRuntimeRequirements {
        CommandRuntimeRequirements::NONE
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        submit_atom_command(ctx, args, "iterate")
    }

    fn help(&self) -> &str {
        "iterate <selection>, <expression>\n\n\
         Execute a Python expression for each atom in a selection (read-only).\n\n\
         Available atom properties:\n\
             name, resn, resv, resi, chain, segi, alt, elem,\n\
             b, q, vdw, partial_charge, formal_charge,\n\
             ss, color, type, hetatm, index, ID, rank, model,\n\
             x, y, z\n\n\
         Examples:\n\
             iterate all, print(name, resn, b)\n\
             iterate chain A, mylist.append(b)"
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Selection, ArgHint::None]
    }
}

// =============================================================================
// AlterCommand
// =============================================================================

/// The `alter` command — executes a Python expression for each atom
/// in a selection, allowing modification of atom properties.
///
/// Usage:
///   alter all, b=0.0
///   alter chain A, chain='B'
pub struct AlterCommand;

impl Command for AlterCommand {
    fn name(&self) -> &str {
        "alter"
    }

    fn runtime_requirements(&self) -> CommandRuntimeRequirements {
        CommandRuntimeRequirements::NONE
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        submit_atom_command(ctx, args, "alter")
    }

    fn help(&self) -> &str {
        "alter <selection>, <expression>\n\n\
         Execute a Python expression for each atom in a selection,\n\
         allowing modification of atom properties.\n\n\
         Mutable properties:\n\
             name, resn, resv, chain, segi, alt, elem,\n\
             b, q, vdw, partial_charge, formal_charge,\n\
             ss, color, type\n\n\
         Read-only: x, y, z, index, ID, rank, model, hetatm\n\n\
         Examples:\n\
             alter all, b=0.0\n\
             alter chain A, chain='B'\n\
             alter name CA, vdw=2.0"
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Selection, ArgHint::None]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{atomic::AtomicBool, Arc, Mutex};

    use patinae_mol::ObjectMolecule;
    use patinae_scene::{MoleculeObject, Session, SessionAdapter};

    fn shared_state() -> SharedStateHandle {
        Arc::new(Mutex::new(crate::shared::SharedState::new(Arc::new(
            AtomicBool::new(false),
        ))))
    }

    #[test]
    fn python_commands_declare_scene_requirements_explicitly() {
        assert!(PythonCommand.runtime_requirements().is_empty());
        assert!(IterateCommand.runtime_requirements().is_empty());
        assert!(AlterCommand.runtime_requirements().is_empty());
    }

    #[test]
    fn atom_command_code_preserves_raw_python_expression() {
        let args = ParsedCommand::new("alter")
            .with_arg("name CA")
            .with_named_arg("b", "random.random()")
            .with_raw_args("name CA, b=random.random()");

        let code = build_atom_command_code(&args, "alter").unwrap();

        assert_eq!(code, "cmd.alter('name CA', 'b=random.random()')");
    }

    #[test]
    fn atom_command_code_preserves_raw_string_assignment() {
        let args = ParsedCommand::new("alter")
            .with_arg("chain A")
            .with_named_arg("chain", "B")
            .with_raw_args("chain A, chain='B'");

        let code = build_atom_command_code(&args, "alter").unwrap();

        assert_eq!(code, r#"cmd.alter('chain A', 'chain=\'B\'')"#);
    }

    #[test]
    fn atom_command_code_preserves_raw_stored_expression() {
        let args = ParsedCommand::new("alter")
            .with_arg("name CA")
            .with_named_arg("b", "stored.b.pop()")
            .with_raw_args("name CA, b=stored.b.pop()");

        let code = build_atom_command_code(&args, "alter").unwrap();

        assert_eq!(code, "cmd.alter('name CA', 'b=stored.b.pop()')");
    }

    #[test]
    fn atom_command_code_preserves_expression_commas() {
        let args = ParsedCommand::new("iterate")
            .with_arg("all")
            .with_arg("print(name")
            .with_arg("resn")
            .with_arg("b)")
            .with_raw_args("all, print(name, resn, b)");

        let code = build_atom_command_code(&args, "iterate").unwrap();

        assert_eq!(code, "cmd.iterate('all', 'print(name, resn, b)')");
    }

    #[test]
    fn molecule_snapshot_sync_is_generation_gated() {
        let shared = shared_state();
        let mut session = Session::new();
        session
            .registry
            .add(MoleculeObject::new(ObjectMolecule::new("obj")));
        let mut needs_redraw = false;
        let adapter = SessionAdapter {
            session: &mut session,
            render_context: None,
            default_size: (800, 600),
            needs_redraw: &mut needs_redraw,
        };

        assert!(sync_shared_molecules(&shared, &adapter, false));
        {
            let state = shared.lock().unwrap();
            assert_eq!(state.names, vec!["obj".to_string()]);
            assert_eq!(state.molecules.len(), 1);
            assert!(state.molecule_generation.is_some());
        }

        assert!(!sync_shared_molecules(&shared, &adapter, false));
        assert!(sync_shared_molecules(&shared, &adapter, true));
    }

    #[test]
    fn molecule_snapshot_refreshes_when_registry_generation_changes() {
        let shared = shared_state();
        let mut session = Session::new();
        session
            .registry
            .add(MoleculeObject::new(ObjectMolecule::new("first")));
        let mut needs_redraw = false;
        {
            let adapter = SessionAdapter {
                session: &mut session,
                render_context: None,
                default_size: (800, 600),
                needs_redraw: &mut needs_redraw,
            };
            assert!(sync_shared_molecules(&shared, &adapter, false));
        }

        session
            .registry
            .add(MoleculeObject::new(ObjectMolecule::new("second")));
        let adapter = SessionAdapter {
            session: &mut session,
            render_context: None,
            default_size: (800, 600),
            needs_redraw: &mut needs_redraw,
        };

        assert!(sync_shared_molecules(&shared, &adapter, false));
        let state = shared.lock().unwrap();
        assert_eq!(state.molecules.len(), 2);
    }
}
