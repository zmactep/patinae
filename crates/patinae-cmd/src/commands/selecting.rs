//! Selection commands: select, deselect, indicate

use crate::args::ParsedCommand;
use crate::command::{Command, CommandContext, CommandRegistry, ViewerLike};
use crate::command_help;
use crate::error::{CmdError, CmdResult};
use crate::ArgHint;

use patinae_select::{SelectionOptions, SelectionResult};

/// Expand self-references in a selection expression.
///
/// When a selection expression references its own name (e.g., `select sele, sele or resi 24`
/// where `sele` was previously `chain A`), replace occurrences of the name with the old
/// expression wrapped in parentheses: `(chain A) or resi 24`.
///
/// Only replaces whole-word matches to avoid replacing partial identifiers
/// (e.g., `sele` should not match inside `selected`).
fn expand_self_reference(expression: &str, name: &str, old_expression: &str) -> String {
    let mut result = String::with_capacity(expression.len() + old_expression.len());
    let replacement = format!("({})", old_expression);
    let mut remaining = expression;

    while let Some(pos) = remaining.find(name) {
        // Check word boundary before the match
        let before_ok = pos == 0
            || !remaining.as_bytes()[pos - 1].is_ascii_alphanumeric()
                && remaining.as_bytes()[pos - 1] != b'_';

        // Check word boundary after the match
        let end = pos + name.len();
        let after_ok = end >= remaining.len()
            || !remaining.as_bytes()[end].is_ascii_alphanumeric()
                && remaining.as_bytes()[end] != b'_';

        if before_ok && after_ok {
            result.push_str(&remaining[..pos]);
            result.push_str(&replacement);
        } else {
            result.push_str(&remaining[..end]);
        }
        remaining = &remaining[end..];
    }
    result.push_str(remaining);

    result
}

/// Register selection commands
pub fn register(registry: &mut CommandRegistry) {
    registry.register(SelectCommand);
    registry.register(DeselectCommand);
    registry.register(IndicateCommand);
}

// ============================================================================
// Selection evaluation helper
// ============================================================================

fn is_recent_atom_alias(name: &str) -> bool {
    name == "pk*"
        || name.strip_prefix("pk").is_some_and(|digits| {
            matches!(digits.as_bytes().first(), Some(b'1'..=b'9'))
                && digits.bytes().all(|digit| digit.is_ascii_digit())
        })
}

/// Evaluate a selection expression with support for named selections from the viewer.
///
/// This function:
/// 1. Gathers all molecules from the viewer's object registry
/// 2. Builds an EvalContext with all molecules
/// 3. Pre-evaluates stored named selections and adds them to the context
/// 4. Evaluates the target selection
/// 5. Returns the total count and a mapping of object name to SelectionResult
///
/// # Arguments
/// * `viewer` - The viewer providing objects and named selections
/// * `selection` - The selection expression to evaluate
///
/// # Returns
/// * Total atom count across all objects
/// * A vector of (object_name, SelectionResult) pairs
pub fn select_with_context(
    viewer: &dyn ViewerLike,
    selection: &str,
) -> CmdResult<(usize, Vec<(String, SelectionResult)>)> {
    // Gather all molecules from the registry
    let object_names: Vec<String> = viewer.objects().names().map(|s| s.to_string()).collect();

    // Parse the selection expression upfront (validates syntax even with no molecules)
    let parsed_expr = match patinae_select::parse(selection) {
        Ok(expr) => expr,
        Err(parse_err) => {
            // If parsing fails and the input contains wildcard characters,
            // try to interpret it as an object/selection name pattern
            if selection.contains('*') || selection.contains('?') {
                return evaluate_wildcard_pattern(viewer, selection, &object_names);
            }
            return Err(CmdError::invalid_arg(
                "selection",
                format!("parse error: {:?}", parse_err),
            ));
        }
    };

    let recent_aliases = parsed_expr
        .selection_references()
        .into_iter()
        .filter(|name| is_recent_atom_alias(name))
        .collect::<Vec<_>>();
    let recent_atoms = if recent_aliases.is_empty() {
        Vec::new()
    } else {
        viewer.session().resolved_recent_atoms()
    };
    for alias in &recent_aliases {
        if *alias == "pk*" {
            continue;
        }
        let position = alias
            .strip_prefix("pk")
            .and_then(|digits| digits.parse::<usize>().ok());
        if !matches!(position, Some(position) if position <= recent_atoms.len()) {
            return Err(CmdError::selection(format!(
                "recent atom alias '{alias}' is unavailable; Recent Atoms has {} entries",
                recent_atoms.len()
            )));
        }
    }

    // Validate all name references in the expression against known selections/objects
    let known_selections = viewer.selection_names();
    for name in parsed_expr.selection_references() {
        if is_recent_atom_alias(name) {
            continue;
        }
        let is_known =
            known_selections.iter().any(|s| s == name) || object_names.iter().any(|s| s == name);
        if !is_known {
            return Err(CmdError::invalid_arg(
                "selection",
                format!("'{}' is not a known selection or object name", name),
            ));
        }
    }

    if object_names.is_empty() {
        return Ok((0, Vec::new()));
    }

    let labels_by_source = if parsed_expr.uses_atom_labels() {
        let mut labels = ahash::AHashMap::<&str, Vec<(patinae_mol::AtomIndex, &str)>>::new();
        for label_name in viewer.objects().names() {
            let Some(label_object) = viewer.objects().get_label(label_name) else {
                continue;
            };
            for entity in label_object.entities() {
                let anchor = entity.anchor();
                if !anchor.is_orphaned() {
                    labels
                        .entry(&anchor.object_name)
                        .or_default()
                        .push((anchor.atom_index, entity.text()));
                }
            }
        }
        labels
    } else {
        ahash::AHashMap::new()
    };

    let mut total_count = 0;
    let mut results: Vec<(String, SelectionResult)> = Vec::new();

    let options = SelectionOptions {
        ignore_case: viewer.settings().behavior.ignore_case,
        ignore_case_chain: viewer.settings().behavior.ignore_case_chain,
    };

    for obj_name in &object_names {
        let Some(mol_obj) = viewer.objects().get_molecule(obj_name) else {
            continue;
        };
        let mol = mol_obj.molecule();

        // Build context with implicit object selections and named selections
        let mut ctx = viewer.selections().build_eval_context(
            mol,
            mol_obj.display_state(),
            obj_name,
            &object_names,
            options,
        );
        if !recent_aliases.is_empty() {
            let mut union = SelectionResult::none(mol.atom_count());
            for (index, (target_object, atom_index)) in recent_atoms.iter().enumerate() {
                let mut singleton = SelectionResult::none(mol.atom_count());
                if target_object == obj_name {
                    singleton.set(*atom_index);
                    union.set(*atom_index);
                }
                ctx.add_selection(format!("pk{}", index + 1), singleton);
            }
            ctx.add_selection("pk*", union);
        }
        if let Some(labels) = labels_by_source.get(obj_name.as_str()) {
            for &(atom_index, text) in labels {
                if mol.get_atom(atom_index).is_some() {
                    ctx.add_atom_label(atom_index.as_usize(), text);
                }
            }
        }

        match patinae_select::evaluate(&parsed_expr, &ctx) {
            Ok(result) => {
                total_count += result.count();
                results.push((obj_name.to_string(), result));
            }
            Err(e) => {
                log::debug!("Selection evaluation error for {}: {:?}", obj_name, e);
            }
        }
    }

    Ok((total_count, results))
}

/// Evaluate a wildcard pattern against object names and named selections.
///
/// When a selection expression fails to parse but contains `*` or `?`,
/// this function matches the pattern against object and named selection
/// names, returning all atoms from matching objects and the union of
/// cached results from matching named selections.
fn evaluate_wildcard_pattern(
    viewer: &dyn ViewerLike,
    pattern: &str,
    object_names: &[String],
) -> CmdResult<(usize, Vec<(String, SelectionResult)>)> {
    let matching_objects: Vec<&str> = viewer.objects().matching(pattern);
    let matching_selections: Vec<&str> = viewer.selections().matching(pattern);

    if matching_objects.is_empty() && matching_selections.is_empty() {
        return Err(CmdError::selection(format!(
            "No objects or selections matching pattern '{}'",
            pattern
        )));
    }

    let mut total_count = 0;
    let mut results: Vec<(String, SelectionResult)> = Vec::new();

    for obj_name in object_names {
        let Some(mol_obj) = viewer.objects().get_molecule(obj_name) else {
            continue;
        };
        let atom_count = mol_obj.molecule().atom_count();

        // Select all atoms if this object matches the pattern
        let mut combined = if matching_objects.iter().any(|&m| m == obj_name) {
            SelectionResult::all(atom_count)
        } else {
            SelectionResult::none(atom_count)
        };

        // Union with cached results from matching named selections
        for &sel_name in &matching_selections {
            if let Some(entry) = viewer.selections().get(sel_name) {
                if let Some(cached) = entry.cached_results.get(obj_name) {
                    if cached.atom_count() == atom_count {
                        combined.union_with(cached);
                    }
                }
            }
        }

        if combined.any() {
            total_count += combined.count();
            results.push((obj_name.clone(), combined));
        }
    }

    Ok((total_count, results))
}

/// Evaluate a selection expression and return results per object.
///
/// This is a convenience wrapper that evaluates a selection and returns
/// a mapping from object names to their selection results.
///
/// # Arguments
/// * `viewer` - The viewer providing objects and named selections
/// * `selection` - The selection expression to evaluate
///
/// # Returns
/// * A vector of (object_name, SelectionResult) pairs for objects with matches
pub fn evaluate_selection(
    viewer: &dyn ViewerLike,
    selection: &str,
) -> CmdResult<Vec<(String, SelectionResult)>> {
    let (_, results) = select_with_context(viewer, selection)?;
    Ok(results)
}

// ============================================================================
// select command
// ============================================================================

struct SelectCommand;

impl Command for SelectCommand {
    fn name(&self) -> &str {
        "select"
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::None, ArgHint::Selection]
    }

    command_help! {
        CMD "select"
        DESCRIPTION [
            "creates a named selection of atoms.",
        ]
        REQUIRED [
            { "name", "string", "selection name" },
            { "selection", "string", "selection expression" },
        ]
        OPTIONAL [
            { "enable", "0/1", "enable the selection", "1" },
            { "quiet", "0/1", "suppress feedback", "0" },
            { "merge", "0/1", "merge with existing selection", "0" },
            { "state", "integer", "state for selection (-1 = all)", "-1" },
        ]
        EXAMPLES [
            "select backbone, name C+CA+N+O",
            "select chain_A, chain A",
            "select active_site, byres (organic around 4)",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let name = args
            .str_arg(0, "name")
            .ok_or_else(|| CmdError::missing_argument("name".to_string()))?;

        let selection = args
            .str_arg(1, "selection")
            .ok_or_else(|| CmdError::missing_argument("selection".to_string()))?;

        // If the expression references its own name (e.g., `select sele, sele or resi 24`),
        // expand the self-reference by substituting the old expression. This prevents
        // circular references in the stored expression — selections are evaluated
        // selections are evaluated immediately and stored as results.
        let expanded = if let Some(old_expr) = ctx.viewer.get_selection(name) {
            expand_self_reference(selection, name, old_expr)
        } else {
            selection.to_string()
        };

        // Evaluate the selection to count atoms and cache results
        let (total_count, results) = select_with_context(ctx.viewer, &expanded)?;

        if total_count == 0 {
            // Remove empty selections instead of keeping them around
            ctx.viewer.remove_selection(name);
            if !ctx.quiet {
                ctx.print(&format!(
                    " Selector: selection \"{}\" defined with 0 atoms.",
                    name
                ));
            }
        } else {
            // Store the expanded expression with cached evaluation results
            ctx.viewer
                .define_selection_with_results(name, &expanded, results);

            // Ensure the selection is visible (show indicators)
            ctx.viewer.set_selection_visible(name, true);

            if !ctx.quiet {
                ctx.print(&format!(
                    " Selector: selection \"{}\" defined with {} atoms.",
                    name, total_count
                ));
            }
        }

        Ok(())
    }
}

// ============================================================================
// deselect command
// ============================================================================

struct DeselectCommand;

impl Command for DeselectCommand {
    fn name(&self) -> &str {
        "deselect"
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::NamedSelection]
    }

    command_help! {
        CMD "deselect"
        DESCRIPTION [
            "removes a named selection or clears all selections.",
        ]
        REQUIRED []
        OPTIONAL [
            { "selection", "string", "name of selection to remove", "all selections" },
        ]
        EXAMPLES [
            "deselect chainA",
            "deselect",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        // Get optional selection name
        let name = args.str_arg(0, "selection");

        if let Some(name) = name {
            // Remove specific selection
            let existed = ctx.viewer.remove_selection(name);
            if !ctx.quiet {
                if existed {
                    ctx.print(&format!(" Selection \"{}\" deleted.", name));
                } else {
                    ctx.print_error(&format!(" Selection \"{}\" not found.", name));
                }
            }
        } else {
            // Remove all selections
            let names = ctx.viewer.selection_names();
            let count = names.len();
            for name in names {
                ctx.viewer.remove_selection(&name);
            }
            if !ctx.quiet {
                if count > 0 {
                    ctx.print(&format!(" {} selection(s) cleared.", count));
                } else {
                    ctx.print(" No selections to clear.");
                }
            }
        }

        Ok(())
    }
}

// ============================================================================
// indicate command
// ============================================================================

struct IndicateCommand;

impl Command for IndicateCommand {
    fn name(&self) -> &str {
        "indicate"
    }

    command_help! {
        CMD "indicate"
        DESCRIPTION [
            "shows a visual indicator (pink/magenta dots) for the specified",
            "selection in the 3D viewport. This helps visualize which atoms are",
            "currently selected.",
        ]
        REQUIRED []
        OPTIONAL [
            { "selection", "string", "selection to indicate", "sele" } => [
                "Use \"none\" or empty to clear the indication.",
            ],
        ]
        EXAMPLES [
            "indicate chain A",
            "indicate organic",
            "indicate sele",
            "indicate none",
        ]
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Selection]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let selection = args.str_arg(0, "selection");

        match selection {
            Some("none") | Some("") => {
                // Clear indication when "none" or empty is specified
                ctx.viewer.clear_indication();
                if !ctx.quiet {
                    ctx.print(" Indication cleared.");
                }
            }
            None => {
                // No arguments - default to "sele"
                let sel = "sele";

                // Check if "sele" is a named selection that exists
                if ctx.viewer.get_selection(sel).is_some() {
                    // Make the existing selection visible
                    ctx.viewer.set_selection_visible(sel, true);
                    let (count, _) = select_with_context(ctx.viewer, sel)?;
                    if !ctx.quiet {
                        ctx.print(&format!(" Indicating \"{}\" ({} atoms)", sel, count));
                    }
                } else {
                    // "sele" doesn't exist - create it with "all" as default
                    let (count, results) = select_with_context(ctx.viewer, "all")?;
                    ctx.viewer
                        .define_selection_with_results(sel, "all", results);
                    ctx.viewer.set_selection_visible(sel, true);
                    if !ctx.quiet {
                        ctx.print(&format!(" Indicating \"{}\" ({} atoms)", sel, count));
                    }
                }
            }
            Some(sel) => {
                // Check if this is an existing named selection
                if ctx.viewer.get_selection(sel).is_some() {
                    // Make the existing selection visible
                    ctx.viewer.set_selection_visible(sel, true);
                    let (count, _) = select_with_context(ctx.viewer, sel)?;
                    if !ctx.quiet {
                        ctx.print(&format!(" Indicating \"{}\" ({} atoms)", sel, count));
                    }
                } else {
                    // It's a selection expression - create/update "indicate" selection
                    let (count, results) = select_with_context(ctx.viewer, sel)?;
                    ctx.viewer
                        .define_selection_with_results("indicate", sel, results);
                    ctx.viewer.set_selection_visible("indicate", true);
                    if !ctx.quiet {
                        ctx.print(&format!(" Indicating \"{}\" ({} atoms)", sel, count));
                    }
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lin_alg::f32::Vec3;
    use patinae_mol::{Atom, AtomIndex, CoordSet, Element, ObjectMolecule};
    use patinae_scene::{canonical_atom_path_for_atom, MoleculeObject, Session, SessionAdapter};
    use patinae_settings::groups::RecentPickLimit;

    fn recent_atom_session() -> Session {
        let mut molecule = ObjectMolecule::new("source");
        for name in ["A", "B", "C", "D", "E"] {
            molecule.add_atom(Atom::new(name, Element::Carbon));
        }
        molecule.add_coord_set(CoordSet::from_vec3(&[
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::new(3.0, 0.0, 0.0),
            Vec3::new(4.0, 0.0, 0.0),
        ]));

        let mut session = Session::new();
        session
            .registry
            .add(MoleculeObject::with_name(molecule, "source"));
        for index in 0usize..5 {
            let molecule = session.registry.get_molecule("source").unwrap();
            let path =
                canonical_atom_path_for_atom("source", molecule.molecule(), AtomIndex::from(index))
                    .unwrap();
            session
                .recent_atoms
                .insert(path, RecentPickLimit::Unlimited);
        }
        session
    }

    fn evaluate_recent(
        session: &mut Session,
        selection: &str,
    ) -> CmdResult<(usize, Vec<(String, SelectionResult)>)> {
        let mut needs_redraw = false;
        let adapter = SessionAdapter {
            session,
            render_context: None,
            default_size: (64, 64),
            needs_redraw: &mut needs_redraw,
            async_fetch_fn: None,
        };
        select_with_context(&adapter, selection)
    }

    #[test]
    fn test_expand_self_reference_basic() {
        let result = expand_self_reference("sele or resi 24", "sele", "chain A");
        assert_eq!(result, "(chain A) or resi 24");
    }

    #[test]
    fn test_expand_self_reference_multiple() {
        let result = expand_self_reference("sele and not sele", "sele", "chain A");
        assert_eq!(result, "(chain A) and not (chain A)");
    }

    #[test]
    fn test_expand_self_reference_no_match() {
        let result = expand_self_reference("chain A or resi 24", "sele", "chain B");
        assert_eq!(result, "chain A or resi 24");
    }

    #[test]
    fn test_expand_self_reference_word_boundary() {
        // "sele" should not match inside "selected"
        let result = expand_self_reference("selected or resi 24", "sele", "chain A");
        assert_eq!(result, "selected or resi 24");
    }

    #[test]
    fn test_expand_self_reference_at_end() {
        let result = expand_self_reference("resi 24 or sele", "sele", "chain A");
        assert_eq!(result, "resi 24 or (chain A)");
    }

    #[test]
    fn test_coordinate_selection_uses_display_state() {
        use patinae_mol::AtomBuilder;

        let mut mol = ObjectMolecule::new("obj");
        mol.add_atom(AtomBuilder::new().name("CA").element_symbol("C").build());
        mol.add_coord_set(CoordSet::from_vec3(&[Vec3::new(0.0, 0.0, 0.0)]));
        mol.add_coord_set(CoordSet::from_vec3(&[Vec3::new(8.0, 0.0, 0.0)]));

        let mut obj = MoleculeObject::with_name(mol, "obj");
        assert!(obj.set_display_state(1));

        let mut session = Session::new();
        session.registry.add(obj);
        let mut needs_redraw = false;
        let adapter = SessionAdapter {
            session: &mut session,
            render_context: None,
            default_size: (800, 600),
            needs_redraw: &mut needs_redraw,
            async_fetch_fn: None,
        };

        let (count, results) = select_with_context(&adapter, "x > 4").unwrap();

        assert_eq!(count, 1);
        assert_eq!(results[0].1.count(), 1);
        assert_eq!(
            adapter
                .session
                .registry
                .get_molecule("obj")
                .unwrap()
                .molecule()
                .current_state,
            0
        );
    }

    #[test]
    fn recent_atom_aliases_follow_list_order_and_algebra() {
        let mut session = recent_atom_session();

        let (count, results) = evaluate_recent(&mut session, "pk2").unwrap();
        assert_eq!(count, 1);
        assert!(results[0].1.contains(AtomIndex::from(1usize)));

        let (count, results) = evaluate_recent(&mut session, "pk* and name A+E").unwrap();
        assert_eq!(count, 2);
        assert!(results[0].1.contains(AtomIndex::from(0usize)));
        assert!(results[0].1.contains(AtomIndex::from(4usize)));

        let (count, results) = evaluate_recent(&mut session, "pk5").unwrap();
        assert_eq!(count, 1);
        assert!(results[0].1.contains(AtomIndex::from(4usize)));

        let (_, stored_results) = evaluate_recent(&mut session, "name E").unwrap();
        session
            .selections
            .define_with_results("pk2", "name E", stored_results);
        let (_, results) = evaluate_recent(&mut session, "pk2").unwrap();
        assert!(results[0].1.contains(AtomIndex::from(1usize)));
        assert!(!results[0].1.contains(AtomIndex::from(4usize)));
    }

    #[test]
    fn recent_atom_aliases_compact_and_report_missing_rows() {
        let mut session = recent_atom_session();
        let first = session.recent_atoms.paths().next().unwrap().to_string();
        assert!(session.recent_atoms.remove_path(&first));

        let (count, results) = evaluate_recent(&mut session, "pk1").unwrap();
        assert_eq!(count, 1);
        assert!(results[0].1.contains(AtomIndex::from(1usize)));

        let error = evaluate_recent(&mut session, "pk5").unwrap_err();
        assert!(error.to_string().contains("pk5"));
        assert!(error.to_string().contains("4 entries"));

        assert!(session.recent_atoms.clear());
        let (count, results) = evaluate_recent(&mut session, "pk*").unwrap();
        assert_eq!(count, 0);
        assert_eq!(results[0].1.count(), 0);
    }
}
