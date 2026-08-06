//! Recent atom picking commands.

use patinae_scene::{canonical_atom_path_for_atom, display_atom_path};

use crate::args::ParsedCommand;
use crate::command::{Command, CommandContext, CommandRegistry, ViewerLike};
use crate::command_help;
use crate::commands::selecting::select_with_context;
use crate::error::{CmdError, CmdResult};
use crate::ArgHint;

/// Registers commands that mutate the session-owned recent atom list.
pub fn register(registry: &mut CommandRegistry) {
    registry.register(PickCommand);
    registry.register(UnpickCommand);
}

fn resolve_singleton_path(viewer: &dyn ViewerLike, selection: &str) -> CmdResult<String> {
    let (total_count, results) = select_with_context(viewer, selection)?;
    if total_count != 1 {
        return Err(CmdError::selection(format!(
            "selection '{selection}' does not resolve to exactly one atom"
        )));
    }

    for (object_name, selected) in results {
        let Some(atom_index) = selected.indices().next() else {
            continue;
        };
        let Some(molecule) = viewer.objects().get_molecule(&object_name) else {
            continue;
        };
        let path = canonical_atom_path_for_atom(&object_name, molecule.molecule(), atom_index)
            .map_err(|error| {
                CmdError::selection(format!(
                    "selection '{selection}' cannot be stored as a recent atom: {error}"
                ))
            })?;
        if !viewer.session().recent_atom_path_is_singleton(&path) {
            return Err(CmdError::selection(format!(
                "selection '{selection}' resolves to an atom whose slash path is not unique"
            )));
        }
        return Ok(path);
    }

    Err(CmdError::selection(format!(
        "selection '{selection}' does not resolve to exactly one atom"
    )))
}

struct PickCommand;

impl Command for PickCommand {
    fn name(&self) -> &str {
        "pick"
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Selection]
    }

    command_help! {
        CMD "pick"
        DESCRIPTION [
            "adds exactly one atom to the session's recent atom list.",
            "Repeating the same pick is a no-op.",
        ]
        REQUIRED [
            { "selection", "string", "selection resolving to exactly one atom" },
        ]
        OPTIONAL []
        EXAMPLES [
            "pick model 1fsd and chain A and resi 16 and name HZ2",
            "pick /1fsd//A/LYS`16/HZ2",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let selection = args
            .str_arg(0, "selection")
            .ok_or_else(|| CmdError::missing_argument("selection"))?;
        let path = resolve_singleton_path(ctx.viewer, selection)?;
        let limit = ctx.viewer.settings().behavior.recent_pick_limit();
        let already_picked = ctx.viewer.session().recent_atoms.row_id(&path).is_some();
        let changed = ctx
            .viewer
            .session_mut()
            .recent_atoms
            .insert(path.clone(), limit);

        if changed {
            ctx.viewer.request_redraw();
            ctx.print(&format!(
                " Recent atom picked: {}",
                display_atom_path(&path)
            ));
        } else if already_picked {
            ctx.print(&format!(
                " Recent atom already picked: {}",
                display_atom_path(&path)
            ));
        } else {
            ctx.print_warning(" Recent atom was not added because max_recent_picks is 0.");
        }
        Ok(())
    }
}

struct UnpickCommand;

impl Command for UnpickCommand {
    fn name(&self) -> &str {
        "unpick"
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Selection]
    }

    command_help! {
        CMD "unpick"
        DESCRIPTION [
            "removes one atom from the recent atom list, or clears the list",
            "when called without a selection.",
        ]
        REQUIRED []
        OPTIONAL [
            { "selection", "string", "selection resolving to exactly one atom", "all recent atoms" },
        ]
        EXAMPLES [
            "unpick model 1fsd and chain A and resi 16 and name HZ2",
            "unpick",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let changed = if let Some(selection) = args.str_arg(0, "selection") {
            let path = resolve_singleton_path(ctx.viewer, selection)?;
            let changed = ctx.viewer.session_mut().recent_atoms.remove_path(&path);
            if changed {
                ctx.print(&format!(
                    " Recent atom removed: {}",
                    display_atom_path(&path)
                ));
            } else {
                ctx.print(&format!(
                    " Recent atom was not picked: {}",
                    display_atom_path(&path)
                ));
            }
            changed
        } else {
            let count = ctx.viewer.session().recent_atoms.len();
            let changed = ctx.viewer.session_mut().recent_atoms.clear();
            if count == 0 {
                ctx.print(" No recent atoms to clear.");
            } else {
                ctx.print(&format!(" {count} recent atom(s) cleared."));
            }
            changed
        };

        if changed {
            ctx.viewer.request_redraw();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use lin_alg::f32::Vec3;
    use patinae_mol::{Atom, AtomIndex, CoordSet, Element, ObjectMolecule};
    use patinae_scene::{
        canonical_atom_path_for_hit, display_atom_path, MoleculeObject, ObjectType, PickHit,
        Session, SessionAdapter,
    };

    use crate::{CmdError, CmdResult, CommandExecutor, CommandOutput};

    fn picking_session() -> Session {
        let mut molecule = ObjectMolecule::new("source");
        for name in ["A", "B", "C"] {
            molecule.add_atom(Atom::new(name, Element::Carbon));
        }
        molecule.add_coord_set(CoordSet::from_vec3(&[
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(2.0, 0.0, 0.0),
        ]));
        let mut session = Session::new();
        session
            .registry
            .add(MoleculeObject::with_name(molecule, "source"));
        session
    }

    fn execute(session: &mut Session, command: &str) -> CmdResult {
        execute_with_output(session, command).map(|_| ())
    }

    fn execute_with_output(
        session: &mut Session,
        command: &str,
    ) -> Result<CommandOutput, CmdError> {
        let mut needs_redraw = false;
        let mut adapter = SessionAdapter {
            session,
            render_context: None,
            default_size: (64, 64),
            needs_redraw: &mut needs_redraw,
            async_fetch_fn: None,
        };
        CommandExecutor::new().do_with_options(&mut adapter, command, false)
    }

    fn canonical_path(session: &Session, atom_index: usize) -> String {
        let molecule = session.registry.get_molecule("source").unwrap();
        canonical_atom_path_for_hit(
            &PickHit {
                object_name: "source".to_string(),
                object_type: ObjectType::Molecule,
                atom_index: Some(AtomIndex::from(atom_index)),
                position: Vec3::new(0.0, 0.0, 0.0),
                distance: 0.0,
            },
            molecule.molecule(),
        )
        .unwrap()
    }

    #[test]
    fn pick_adds_an_idempotent_canonical_singleton_and_enforces_the_limit() {
        let mut session = picking_session();
        let first = canonical_path(&session, 0);
        let second = canonical_path(&session, 1);
        let third = canonical_path(&session, 2);

        execute(&mut session, "pick model source and name A").unwrap();
        execute(&mut session, "pick model source and name A").unwrap();
        assert_eq!(session.recent_atoms.paths().collect::<Vec<_>>(), [first]);

        session.settings.behavior.max_recent_picks = 2;
        execute(&mut session, "pick model source and name B").unwrap();
        execute(&mut session, "pick model source and name C").unwrap();
        assert_eq!(
            session.recent_atoms.paths().collect::<Vec<_>>(),
            [second, third]
        );
    }

    #[test]
    fn pick_obeys_a_zero_recent_pick_limit() {
        let mut session = picking_session();
        session.settings.behavior.max_recent_picks = 0;

        execute(&mut session, "pick model source and name A").unwrap();

        assert!(session.recent_atoms.is_empty());
    }

    #[test]
    fn unpick_removes_one_singleton_or_clears_everything_without_an_argument() {
        let mut session = picking_session();
        let second = canonical_path(&session, 1);
        execute(&mut session, "pick model source and name A").unwrap();
        execute(&mut session, "pick model source and name B").unwrap();

        execute(&mut session, "unpick model source and name A").unwrap();
        assert_eq!(session.recent_atoms.paths().collect::<Vec<_>>(), [second]);

        execute(&mut session, "unpick").unwrap();
        assert!(session.recent_atoms.is_empty());
    }

    #[test]
    fn canonical_slash_paths_round_trip_through_pick_and_unpick_commands() {
        let mut session = picking_session();
        let path = canonical_path(&session, 0);
        let command_path = format!("\"{}\"", path.replace('"', "\\\""));

        execute(&mut session, &format!("pick {command_path}")).unwrap();
        assert_eq!(session.recent_atoms.paths().collect::<Vec<_>>(), [path]);

        execute(&mut session, &format!("unpick {command_path}")).unwrap();
        assert!(session.recent_atoms.is_empty());
    }

    #[test]
    fn pick_and_unpick_messages_use_the_display_atom_path() {
        let mut session = picking_session();
        let path = canonical_path(&session, 0);
        let display_path = display_atom_path(&path);

        let picked = execute_with_output(&mut session, "pick model source and name A").unwrap();
        assert_eq!(
            picked.messages[0].text,
            format!(" Recent atom picked: {display_path}")
        );
        assert!(!picked.messages[0].text.contains('"'));

        let removed = execute_with_output(&mut session, "unpick model source and name A").unwrap();
        assert_eq!(
            removed.messages[0].text,
            format!(" Recent atom removed: {display_path}")
        );
        assert!(!removed.messages[0].text.contains('"'));
    }

    #[test]
    fn pick_and_targeted_unpick_require_exactly_one_atom() {
        let mut session = picking_session();

        let empty = execute(&mut session, "pick none").unwrap_err();
        assert!(empty.is_selection());
        let multiple = execute(&mut session, "pick model source").unwrap_err();
        assert!(multiple.is_selection());
        let unpick_multiple = execute(&mut session, "unpick model source").unwrap_err();
        assert!(unpick_multiple.is_selection());
        assert!(session.recent_atoms.is_empty());
    }

    #[test]
    fn pick_rejects_an_atom_whose_persisted_slash_path_is_ambiguous() {
        let mut molecule = ObjectMolecule::new("duplicates");
        for _ in 0..2 {
            molecule.add_atom(Atom::new("CA", Element::Carbon));
        }
        molecule.add_coord_set(CoordSet::from_vec3(&[
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
        ]));
        let mut session = Session::new();
        session
            .registry
            .add(MoleculeObject::with_name(molecule, "duplicates"));

        let error = execute(&mut session, "pick model duplicates and index 0").unwrap_err();

        assert!(error.is_selection());
        assert!(error.to_string().contains("slash path is not unique"));
        assert!(session.recent_atoms.is_empty());
    }
}
