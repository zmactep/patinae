//! Object commands: delete, rename, create, copy, group, ungroup

use patinae_mol::dss::{assign_secondary_structure, assigner_for};
use patinae_mol::{AtomIndex, ObjectMolecule};
use patinae_scene::{MoleculeObject, Object};

use crate::args::{ArgValue, ParsedCommand};
use crate::command::{ArgHint, Command, CommandContext, CommandRegistry, ViewerLike};
use crate::command_help;
use crate::commands::selecting::evaluate_selection;
use crate::error::{CmdError, CmdResult};
use crate::helpers::state_index_from_user;

/// Parse a members string into individual name patterns
///
/// Members can be space-separated or comma-separated.
fn parse_members(members_str: &str) -> Vec<String> {
    if members_str.is_empty() {
        return Vec::new();
    }
    members_str
        .split([' ', ','])
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

/// Resolve member patterns against the object registry
///
/// Expands glob patterns (e.g., `lig*`) using `registry.matching()`.
fn resolve_members(registry: &patinae_scene::ObjectRegistry, patterns: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    for pattern in patterns {
        if pattern.contains('*') || pattern == "all" {
            result.extend(
                registry
                    .matching(pattern)
                    .into_iter()
                    .map(|s| s.to_string()),
            );
        } else if registry.contains(pattern) {
            result.push(pattern.clone());
        }
    }
    result
}

/// Recursively collect all descendants of a group (children, grandchildren, etc.)
///
/// Results are added depth-first so children are deleted before their parents.
fn collect_group_descendants(
    registry: &patinae_scene::ObjectRegistry,
    name: &str,
    result: &mut Vec<String>,
) {
    if let Some(group) = registry.get_group(name) {
        for child in group.children().to_vec() {
            collect_group_descendants(registry, &child, result);
            result.push(child);
        }
    }
}

/// Register object commands
pub fn register(registry: &mut CommandRegistry) {
    registry.register(DeleteCommand);
    registry.register(RenameCommand);
    registry.register(CreateCommand);
    registry.register(CopyCommand);
    registry.register(GroupCommand);
    registry.register(UngroupCommand);
    registry.register(StateCommand);
    registry.register(CountStatesCommand);
    registry.register(SplitStatesCommand);
    registry.register(DeleteStatesCommand);
    registry.register(ExtractCommand);
    registry.register(RemoveCommand);
    registry.register(DssCommand);
}

// ============================================================================
// delete command
// ============================================================================

struct DeleteCommand;

impl Command for DeleteCommand {
    fn name(&self) -> &str {
        "delete"
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Selection]
    }

    command_help! {
        CMD "delete"
        DESCRIPTION [
            "removes objects and/or named selections.",
        ]
        REQUIRED [
            { "name", "string", "object or selection name (supports glob patterns)" },
        ]
        OPTIONAL []
        EXAMPLES [
            "delete protein",
            "delete obj*",
            "delete sele",
            "delete all",
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

        let mut deleted_objects = 0usize;
        let mut deleted_selections = 0usize;

        // Delete matching objects
        let obj_matches: Vec<String> = ctx
            .viewer
            .objects()
            .matching(name)
            .iter()
            .map(|s| s.to_string())
            .collect();

        // Expand groups to include their descendants
        let mut to_delete = Vec::new();
        for obj_name in &obj_matches {
            collect_group_descendants(ctx.viewer.objects(), obj_name, &mut to_delete);
            to_delete.push(obj_name.clone());
        }
        // Deduplicate (a child might already be in obj_matches)
        to_delete.dedup();

        for obj_name in &to_delete {
            if ctx.viewer.remove_object(obj_name) {
                deleted_objects += 1;
            }
        }

        // Delete matching named selections
        let matching_sels: Vec<String> = ctx
            .viewer
            .selections()
            .matching(name)
            .iter()
            .map(|s| s.to_string())
            .collect();
        for sel_name in &matching_sels {
            ctx.viewer.remove_selection(sel_name);
            deleted_selections += 1;
        }

        let total = deleted_objects + deleted_selections;
        if total == 0 {
            return Err(CmdError::object_not_found(name.to_string()));
        }

        if deleted_objects > 0 {
            ctx.viewer.update_movie_state_count();
        }
        ctx.viewer.request_redraw();

        if !ctx.quiet {
            match (deleted_objects, deleted_selections) {
                (1, 0) => ctx.print(&format!(" Deleted \"{}\"", name)),
                (o, 0) => ctx.print(&format!(" Deleted {} objects matching \"{}\"", o, name)),
                (0, 1) => ctx.print(&format!(" Deleted selection \"{}\"", name)),
                (0, s) => ctx.print(&format!(" Deleted {} selections matching \"{}\"", s, name)),
                (o, s) => ctx.print(&format!(
                    " Deleted {} objects and {} selections matching \"{}\"",
                    o, s, name
                )),
            }
        }

        Ok(())
    }
}

// ============================================================================
// rename command (also: set_name)
// ============================================================================

struct RenameCommand;

impl Command for RenameCommand {
    fn name(&self) -> &str {
        "set_name"
    }

    fn aliases(&self) -> &[&str] {
        &["rename"]
    }

    command_help! {
        CMD "set_name"
        DESCRIPTION [
            "renames an object or selection.",
        ]
        REQUIRED [
            { "old_name", "string", "current object or selection name" },
            { "new_name", "string", "new name" },
        ]
        OPTIONAL []
        EXAMPLES [
            "set_name protein, myprotein",
            "rename ligand, drug",
        ]
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Object, ArgHint::None]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let old_name = args
            .str_arg(0, "old_name")
            .ok_or_else(|| CmdError::missing_argument("old_name".to_string()))?;
        let new_name = args
            .str_arg(1, "new_name")
            .ok_or_else(|| CmdError::missing_argument("new_name".to_string()))?;

        // Try renaming as an object first, then as a selection
        let renamed = match ctx.viewer.rename_object(old_name, new_name) {
            Ok(_) => true,
            Err(_) => ctx.viewer.rename_selection(old_name, new_name),
        };

        if !renamed {
            return Err(CmdError::object_not_found(old_name.to_string()));
        }

        if !ctx.quiet {
            ctx.print(&format!(" Renamed \"{}\" to \"{}\"", old_name, new_name));
        }

        Ok(())
    }
}

// ============================================================================
// create command
// ============================================================================

struct CreateCommand;

impl Command for CreateCommand {
    fn name(&self) -> &str {
        "create"
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::None, ArgHint::Selection]
    }

    command_help! {
        CMD "create"
        DESCRIPTION [
            "creates a new object from a selection.",
        ]
        REQUIRED [
            { "name", "string", "name for the new object" },
            { "selection", "string", "selection of atoms to include" },
        ]
        OPTIONAL [
            { "source_state", "integer", "state to copy from (0 = all)", "0" },
            { "target_state", "integer", "state to copy to", "0" },
            { "discrete", "0/1", "create discrete states", "0" },
        ]
        EXAMPLES [
            "create chainA, chain A",
            "create ligand, organic",
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
        let source_state = args.int_arg_or(2, "source_state", 0);

        // Evaluate selection
        let results = evaluate_selection(ctx.viewer, selection)?;

        // Find first object with selected atoms
        let (src_name, sel_result) = results
            .into_iter()
            .find(|(_, sel)| sel.count() > 0)
            .ok_or_else(|| CmdError::selection(format!("No atoms matching '{}'", selection)))?;

        let state_opt = state_index_from_user(source_state);

        // Get source molecule and its representations (read-only borrow)
        let (new_mol, source_reps) = {
            let mol_obj = ctx
                .viewer
                .objects()
                .get_molecule(&src_name)
                .ok_or_else(|| CmdError::object_not_found(src_name.clone()))?;
            let src_mol = mol_obj.molecule();

            (
                extract_molecule(src_mol, &sel_result, name, state_opt),
                mol_obj.visible_reps(),
            )
        };

        let atom_count = new_mol.atom_count();
        let mut mol_obj = MoleculeObject::from_raw_with_name(new_mol, name);
        mol_obj.set_visible_reps(source_reps);
        ctx.viewer.insert_object(Box::new(mol_obj));
        ctx.viewer.request_redraw();

        if !ctx.quiet {
            ctx.print(&format!(
                " Created \"{}\" from \"{}\" with {} atoms",
                name, selection, atom_count
            ));
        }

        Ok(())
    }
}

/// Extract a subset of atoms from a molecule based on a selection result.
///
/// If `source_state` is `None`, all states are copied. If `Some(i)` (0-indexed),
/// only that single state is copied into the new object.
pub(crate) fn extract_molecule(
    src: &ObjectMolecule,
    selection: &patinae_select::SelectionResult,
    name: &str,
    source_state: Option<usize>,
) -> ObjectMolecule {
    use std::collections::HashMap;

    let selected_indices: Vec<AtomIndex> = selection.indices().collect();
    let mut new_mol = ObjectMolecule::with_capacity(name, selected_indices.len(), 0);

    // Build old→new atom index mapping and copy atoms
    let mut index_map: HashMap<u32, AtomIndex> = HashMap::new();
    for &old_idx in &selected_indices {
        let atom = src.get_atom(old_idx).unwrap().clone();
        let new_idx = new_mol.add_atom(atom);
        index_map.insert(old_idx.0, new_idx);
    }

    // Copy bonds where both endpoints are selected
    for bond in src.bonds() {
        if let (Some(&new_a1), Some(&new_a2)) =
            (index_map.get(&bond.atom1.0), index_map.get(&bond.atom2.0))
        {
            let _ = new_mol.add_bond(new_a1, new_a2, bond.order);
        }
    }

    // Determine which states to copy
    let states: Vec<usize> = match source_state {
        Some(s) => vec![s],
        None => (0..src.state_count()).collect(),
    };

    // Copy coordinate sets (extract coords for selected atoms only)
    for state in states {
        if let Some(cs) = src.get_coord_set(state) {
            let mut coords = Vec::with_capacity(selected_indices.len() * 3);
            for &old_idx in &selected_indices {
                if let Some(v) = cs.get_atom_coord(old_idx) {
                    coords.push(v.x);
                    coords.push(v.y);
                    coords.push(v.z);
                } else {
                    coords.extend_from_slice(&[0.0, 0.0, 0.0]);
                }
            }
            let new_cs = patinae_mol::CoordSet::from_coords(coords);
            new_mol.add_coord_set(new_cs);
        }
    }

    // Classify atoms (protein, nucleic, etc.)
    new_mol.classify_atoms();

    new_mol
}

// ============================================================================
// copy command
// ============================================================================

struct CopyCommand;

impl Command for CopyCommand {
    fn name(&self) -> &str {
        "copy"
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::None, ArgHint::Selection]
    }

    command_help! {
        CMD "copy"
        DESCRIPTION [
            "creates a new object from an object or selection.",
        ]
        REQUIRED [
            { "target", "string", "name for the new object" },
            { "source", "string", "object name or selection expression" },
        ]
        OPTIONAL []
        EXAMPLES [
            "copy protein_copy, protein",
            "copy active_site, byres around 5 ligand",
            "copy chainA, chain A",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let target = args
            .str_arg(0, "target")
            .ok_or_else(|| CmdError::missing_argument("target".to_string()))?;
        let source = args
            .str_arg(1, "source")
            .ok_or_else(|| CmdError::missing_argument("source".to_string()))?;

        // Try as full object copy first (exact name match, no selection parsing)
        if let Some(mol_obj) = ctx.viewer.objects().get_molecule(source) {
            let cloned_mol = mol_obj.molecule().clone();
            let source_reps = mol_obj.visible_reps();
            let atom_count = cloned_mol.atom_count();

            let mut new_obj = MoleculeObject::from_raw_with_name(cloned_mol, target);
            new_obj.set_visible_reps(source_reps);
            ctx.viewer.insert_object(Box::new(new_obj));
            ctx.viewer.request_redraw();

            if !ctx.quiet {
                ctx.print(&format!(
                    " Created \"{}\" as copy of \"{}\" ({} atoms)",
                    target, source, atom_count
                ));
            }
            return Ok(());
        }

        // Otherwise treat as selection expression (like create)
        let results = evaluate_selection(ctx.viewer, source)?;

        let (src_name, sel_result) = results
            .into_iter()
            .find(|(_, sel)| sel.count() > 0)
            .ok_or_else(|| CmdError::selection(format!("No atoms matching '{}'", source)))?;

        let (new_mol, source_reps) = {
            let mol_obj = ctx
                .viewer
                .objects()
                .get_molecule(&src_name)
                .ok_or_else(|| CmdError::object_not_found(src_name.clone()))?;
            (
                extract_molecule(mol_obj.molecule(), &sel_result, target, None),
                mol_obj.visible_reps(),
            )
        };

        let atom_count = new_mol.atom_count();
        let mut new_obj = MoleculeObject::from_raw_with_name(new_mol, target);
        new_obj.set_visible_reps(source_reps);
        ctx.viewer.insert_object(Box::new(new_obj));
        ctx.viewer.request_redraw();

        if !ctx.quiet {
            ctx.print(&format!(
                " Created \"{}\" from \"{}\" ({} atoms)",
                target, source, atom_count
            ));
        }

        Ok(())
    }
}

// ============================================================================
// group command
// ============================================================================

struct GroupCommand;

impl Command for GroupCommand {
    fn name(&self) -> &str {
        "group"
    }

    command_help! {
        CMD "group"
        DESCRIPTION [
            "creates or modifies a group of objects.",
        ]
        REQUIRED [
            { "name", "string", "group name" },
        ]
        OPTIONAL [
            { "members", "string", "object names to add (space or comma separated)", "" },
            { "action", "string", "group action", "add" } => [
                "add, remove, open, close, toggle, auto, ungroup, empty,",
                "purge, excise",
            ],
        ]
        EXAMPLES [
            "group proteins, obj1 obj2 obj3",
            "group ligands, lig*, action=add",
        ]
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[
            ArgHint::Object,
            ArgHint::None,
            ArgHint::Keywords(&[
                "add", "remove", "open", "close", "toggle", "auto", "ungroup", "empty", "purge",
                "excise",
            ]),
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
        let members_str = args.str_arg_or(1, "members", "");
        let action = args.str_arg_or(2, "action", "auto");

        let member_patterns = parse_members(members_str);

        match action {
            "add" => {
                let resolved = resolve_members(ctx.viewer.objects(), &member_patterns);
                // Create group if it doesn't exist and no members were specified
                if resolved.is_empty()
                    && member_patterns.is_empty()
                    && !ctx.viewer.objects().contains(name)
                {
                    use patinae_scene::GroupObject;
                    ctx.viewer.insert_object(Box::new(GroupObject::new(name)));
                }
                for member in &resolved {
                    ctx.viewer.objects_mut().add_to_group(name, member);
                }
                if !ctx.quiet {
                    if resolved.is_empty() && member_patterns.is_empty() {
                        ctx.print(&format!(" Created group \"{}\"", name));
                    } else {
                        ctx.print(&format!(
                            " Added {} members to group \"{}\"",
                            resolved.len(),
                            name
                        ));
                    }
                }
            }
            "remove" => {
                let resolved = resolve_members(ctx.viewer.objects(), &member_patterns);
                for member in &resolved {
                    if let Some(group) = ctx.viewer.objects_mut().get_group_mut(name) {
                        group.remove_child(member);
                    }
                }
                if !ctx.quiet {
                    ctx.print(&format!(
                        " Removed {} members from group \"{}\"",
                        resolved.len(),
                        name
                    ));
                }
            }
            "open" => {
                if let Some(group) = ctx.viewer.objects_mut().get_group_mut(name) {
                    group.set_open(true);
                } else {
                    return Err(CmdError::object_not_found(name.to_string()));
                }
                if !ctx.quiet {
                    ctx.print(&format!(" Opened group \"{}\"", name));
                }
            }
            "close" => {
                if let Some(group) = ctx.viewer.objects_mut().get_group_mut(name) {
                    group.set_open(false);
                } else {
                    return Err(CmdError::object_not_found(name.to_string()));
                }
                if !ctx.quiet {
                    ctx.print(&format!(" Closed group \"{}\"", name));
                }
            }
            "toggle" => {
                if let Some(group) = ctx.viewer.objects_mut().get_group_mut(name) {
                    group.toggle_open();
                } else {
                    return Err(CmdError::object_not_found(name.to_string()));
                }
                if !ctx.quiet {
                    ctx.print(&format!(" Toggled group \"{}\"", name));
                }
            }
            "auto" => {
                if !member_patterns.is_empty() {
                    // Has members → act like "add"
                    let resolved = resolve_members(ctx.viewer.objects(), &member_patterns);
                    for member in &resolved {
                        ctx.viewer.objects_mut().add_to_group(name, member);
                    }
                    if !ctx.quiet {
                        ctx.print(&format!(
                            " Added {} members to group \"{}\"",
                            resolved.len(),
                            name
                        ));
                    }
                } else if ctx.viewer.objects().get_group(name).is_some() {
                    // Group exists, no members → toggle open/close
                    if let Some(group) = ctx.viewer.objects_mut().get_group_mut(name) {
                        group.toggle_open();
                    }
                    if !ctx.quiet {
                        ctx.print(&format!(" Toggled group \"{}\"", name));
                    }
                } else {
                    // Create empty group
                    use patinae_scene::GroupObject;
                    ctx.viewer.insert_object(Box::new(GroupObject::new(name)));
                    if !ctx.quiet {
                        ctx.print(&format!(" Created group \"{}\"", name));
                    }
                }
            }
            "empty" => {
                if let Some(group) = ctx.viewer.objects_mut().get_group_mut(name) {
                    group.clear();
                } else {
                    return Err(CmdError::object_not_found(name.to_string()));
                }
                if !ctx.quiet {
                    ctx.print(&format!(" Emptied group \"{}\"", name));
                }
            }
            "purge" => {
                // Collect children, clear group, delete all child objects
                let children: Vec<String> = ctx
                    .viewer
                    .objects()
                    .get_group(name)
                    .map(|g| g.children().to_vec())
                    .unwrap_or_default();
                if let Some(group) = ctx.viewer.objects_mut().get_group_mut(name) {
                    group.clear();
                }
                for child in &children {
                    ctx.viewer.remove_object(child);
                }
                if !ctx.quiet {
                    ctx.print(&format!(
                        " Purged group \"{}\" ({} objects deleted)",
                        name,
                        children.len()
                    ));
                }
            }
            "excise" | "ungroup" => {
                // Clear children (they become ungrouped), then delete the group
                if let Some(group) = ctx.viewer.objects_mut().get_group_mut(name) {
                    group.clear();
                }
                ctx.viewer.remove_object(name);
                if !ctx.quiet {
                    let verb = if action == "excise" {
                        "Excised"
                    } else {
                        "Ungrouped"
                    };
                    ctx.print(&format!(" {} group \"{}\"", verb, name));
                }
            }
            _ => {
                return Err(CmdError::invalid_arg(
                    "action",
                    format!("Unknown group action: {}", action),
                ));
            }
        }

        ctx.viewer.request_redraw();
        Ok(())
    }
}

// ============================================================================
// ungroup command
// ============================================================================

struct UngroupCommand;

impl Command for UngroupCommand {
    fn name(&self) -> &str {
        "ungroup"
    }

    command_help! {
        CMD "ungroup"
        DESCRIPTION [
            "removes objects from a group.",
        ]
        REQUIRED [
            { "name", "string", "group name to ungroup" },
        ]
        OPTIONAL []
        EXAMPLES [
            "ungroup proteins",
        ]
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Object]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let name = args
            .str_arg(0, "name")
            .ok_or_else(|| CmdError::missing_argument("name".to_string()))?;

        if ctx.viewer.objects().get_group(name).is_none() {
            return Err(CmdError::object_not_found(name.to_string()));
        }

        // Clear children (they become ungrouped, not deleted)
        if let Some(group) = ctx.viewer.objects_mut().get_group_mut(name) {
            group.clear();
        }

        // Delete the group object itself
        ctx.viewer.remove_object(name);
        ctx.viewer.request_redraw();

        if !ctx.quiet {
            ctx.print(&format!(" Ungrouped \"{}\"", name));
        }

        Ok(())
    }
}

// ============================================================================
// state command
// ============================================================================

struct StateCommand;

impl Command for StateCommand {
    fn name(&self) -> &str {
        "state"
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::None, ArgHint::Object]
    }

    command_help! {
        CMD "state"
        DESCRIPTION [
            "sets the displayed state for objects.",
        ]
        REQUIRED [
            { "N", "integer", "state number (1-indexed)" },
        ]
        OPTIONAL [
            { "object", "string", "object name", "all objects" },
        ]
        EXAMPLES [
            "state 1",
            "state 5, 1nmr",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let state_num = args
            .int_arg(0, "N")
            .ok_or_else(|| CmdError::missing_argument("N".to_string()))?;

        if state_num < 1 {
            return Err(CmdError::invalid_arg("N", "State number must be >= 1"));
        }

        let state_idx = (state_num - 1) as usize;
        let object = args.str_arg(1, "object");

        if let Some(obj_name) = object {
            // Set state for a specific object
            let mol_obj = ctx
                .viewer
                .objects_mut()
                .get_molecule_mut(obj_name)
                .ok_or_else(|| CmdError::object_not_found(obj_name.to_string()))?;
            if !mol_obj.set_display_state(state_idx) {
                return Err(CmdError::execution(format!(
                    "State {} is out of range for \"{}\" ({} states)",
                    state_num,
                    obj_name,
                    mol_obj.molecule().state_count()
                )));
            }
        } else {
            // Set state for all objects
            let names: Vec<String> = ctx
                .viewer
                .objects()
                .names()
                .map(|s| s.to_string())
                .collect();
            for name in &names {
                if let Some(mol_obj) = ctx.viewer.objects_mut().get_molecule_mut(name) {
                    mol_obj.set_display_state(state_idx);
                }
            }
        }

        ctx.viewer.request_redraw();

        if !ctx.quiet {
            if let Some(obj_name) = object {
                ctx.print(&format!(" State {} for \"{}\"", state_num, obj_name));
            } else {
                ctx.print(&format!(" State {}", state_num));
            }
        }

        Ok(())
    }
}

// ============================================================================
// count_states command
// ============================================================================

struct CountStatesCommand;

impl Command for CountStatesCommand {
    fn name(&self) -> &str {
        "count_states"
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Selection]
    }

    command_help! {
        CMD "count_states"
        DESCRIPTION [
            "reports the number of states in an object.",
        ]
        REQUIRED []
        OPTIONAL [
            { "selection", "string", "object or selection", "all" },
        ]
        EXAMPLES [
            "count_states",
            "count_states 1nmr",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let selection = args.str_arg(0, "selection");
        let mut max_states = 0usize;

        if let Some(sel) = selection {
            // Try as object name first
            if let Some(mol_obj) = ctx.viewer.objects().get_molecule(sel) {
                let count = mol_obj.molecule().state_count();
                max_states = max_states.max(count);
                if !ctx.quiet {
                    ctx.print(&format!(" \"{}\" has {} states.", sel, count));
                }
            } else {
                // Try as selection — report for all matching objects
                let results = evaluate_selection(ctx.viewer, sel)?;
                for (obj_name, sel_result) in &results {
                    if sel_result.count() > 0 {
                        if let Some(mol_obj) = ctx.viewer.objects().get_molecule(obj_name) {
                            let count = mol_obj.molecule().state_count();
                            max_states = max_states.max(count);
                            if !ctx.quiet {
                                ctx.print(&format!(" \"{}\" has {} states.", obj_name, count));
                            }
                        }
                    }
                }
            }
        } else {
            // Report for all objects
            let names: Vec<String> = ctx
                .viewer
                .objects()
                .names()
                .map(|s| s.to_string())
                .collect();
            for name in &names {
                if let Some(mol_obj) = ctx.viewer.objects().get_molecule(name) {
                    let count = mol_obj.molecule().state_count();
                    max_states = max_states.max(count);
                    if !ctx.quiet {
                        ctx.print(&format!(" \"{}\" has {} states.", name, count));
                    }
                }
            }
        }

        if !ctx.quiet {
            ctx.print(&format!(" cmd.count_states: {} states.", max_states));
        }

        Ok(())
    }
}

// ============================================================================
// split_states command
// ============================================================================

struct SplitStatesCommand;

impl Command for SplitStatesCommand {
    fn name(&self) -> &str {
        "split_states"
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Object]
    }

    command_help! {
        CMD "split_states"
        DESCRIPTION [
            "creates individual objects for each state of a",
            "multi-state object.",
        ]
        REQUIRED [
            { "object", "string", "source object name" },
        ]
        OPTIONAL [
            { "first", "integer", "first state (1-indexed)", "1" },
            { "last", "integer", "last state (1-indexed)", "last" },
            { "prefix", "string", "name prefix", "object_" },
        ]
        EXAMPLES [
            "split_states 1nmr",
            "split_states 1nmr, first=1, last=5",
            "split_states 1nmr, prefix=model_",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let object = args
            .str_arg(0, "object")
            .ok_or_else(|| CmdError::missing_argument("object".to_string()))?;

        let mol_obj = ctx
            .viewer
            .objects()
            .get_molecule(object)
            .ok_or_else(|| CmdError::object_not_found(object.to_string()))?;
        let n_states = mol_obj.molecule().state_count();

        if n_states == 0 {
            return Err(CmdError::execution(format!("\"{}\" has no states", object)));
        }

        let first = args
            .int_arg(1, "first")
            .map(|v| (v.max(1) - 1) as usize)
            .unwrap_or(0);
        let last = args
            .int_arg(2, "last")
            .map(|v| (v.max(1) as usize).min(n_states))
            .unwrap_or(n_states);
        let prefix = args
            .str_arg(3, "prefix")
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("{}_", object));

        // Collect new molecules and source reps (borrow src as read-only)
        let (new_mols, source_reps) = {
            let mol_obj = ctx.viewer.objects().get_molecule(object).unwrap();
            let src_mol = mol_obj.molecule();
            let reps = mol_obj.visible_reps();

            // Build an "all atoms" selection
            let all_sel = patinae_select::SelectionResult::all(src_mol.atom_count());

            let mols: Vec<(String, ObjectMolecule)> = (first..last)
                .map(|state_idx| {
                    let name = format!("{}{:04}", prefix, state_idx + 1);
                    let mol = extract_molecule(src_mol, &all_sel, &name, Some(state_idx));
                    (name, mol)
                })
                .collect();
            (mols, reps)
        };

        let count = new_mols.len();
        let mut objects: Vec<Box<dyn Object>> = Vec::with_capacity(count);
        for (name, mol) in new_mols {
            let mut mol_obj = MoleculeObject::from_raw_with_name(mol, &name);
            mol_obj.set_visible_reps(source_reps);
            objects.push(Box::new(mol_obj));
        }
        ctx.viewer.insert_objects(objects);

        ctx.viewer.request_redraw();

        if !ctx.quiet {
            ctx.print(&format!(
                " Split \"{}\" into {} objects ({}{:04} to {}{:04})",
                object,
                count,
                prefix,
                first + 1,
                prefix,
                last
            ));
        }

        Ok(())
    }
}

// ============================================================================
// delete_states command
// ============================================================================

/// Parse state spec tokens from args starting at `from` into sorted, deduplicated
/// 0-indexed state indices.
///
/// Accepts individual states (`1 2 3`) and ranges (`1-3 9-13`). All values are
/// 1-indexed on the user side and converted to 0-indexed internally.
fn parse_state_spec(args: &ParsedCommand, from: usize) -> Result<Vec<usize>, CmdError> {
    let mut tokens = Vec::new();
    for (_, val) in args.args.iter().skip(from) {
        match val {
            ArgValue::Int(i) => tokens.push(i.to_string()),
            ArgValue::Float(f) => tokens.push(f.to_string()),
            ArgValue::String(s) => tokens.push(s.clone()),
            _ => {}
        }
    }

    if tokens.is_empty() {
        return Err(CmdError::missing_argument("states".to_string()));
    }

    let mut indices = Vec::new();
    for token in &tokens {
        if let Some(dash_pos) = token[1..].find('-') {
            // Range like "1-3" — offset by 1 to skip possible leading digit
            let dash_pos = dash_pos + 1;
            let start: usize = token[..dash_pos].parse().map_err(|_| {
                CmdError::invalid_arg("states", format!("invalid range start in '{}'", token))
            })?;
            let end: usize = token[dash_pos + 1..].parse().map_err(|_| {
                CmdError::invalid_arg("states", format!("invalid range end in '{}'", token))
            })?;
            if start == 0 || end == 0 {
                return Err(CmdError::invalid_arg("states", "state indices are 1-based"));
            }
            let (lo, hi) = if start <= end {
                (start, end)
            } else {
                (end, start)
            };
            for s in lo..=hi {
                indices.push(s - 1);
            }
        } else {
            let state: usize = token.parse().map_err(|_| {
                CmdError::invalid_arg(
                    "states",
                    format!("expected integer or range, got '{}'", token),
                )
            })?;
            if state == 0 {
                return Err(CmdError::invalid_arg("states", "state indices are 1-based"));
            }
            indices.push(state - 1);
        }
    }

    indices.sort_unstable();
    indices.dedup();
    Ok(indices)
}

struct DeleteStatesCommand;

impl Command for DeleteStatesCommand {
    fn name(&self) -> &str {
        "delete_states"
    }

    fn aliases(&self) -> &[&str] {
        &["delete_state"]
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Object]
    }

    command_help! {
        CMD "delete_states"
        DESCRIPTION [
            "removes states from a multi-state object like a",
            "trajectory or NMR ensemble.",
        ]
        REQUIRED [
            { "name", "string", "object name (wildcard supported)" },
            { "states", "string", "space-separated list of state numbers or ranges (1-indexed)" },
        ]
        OPTIONAL []
        EXAMPLES [
            "delete_states 1nmr, 1-5",
            "delete_states *, 1-3 10-40",
            "delete_states traj, 1 2 3",
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

        let indices = parse_state_spec(args, 1)?;

        let obj_names: Vec<String> = ctx
            .viewer
            .objects()
            .matching(name)
            .iter()
            .map(|s| s.to_string())
            .collect();

        if obj_names.is_empty() {
            return Err(CmdError::object_not_found(name.to_string()));
        }

        let mut total_deleted = 0usize;
        let mut objects_modified = 0usize;

        for obj_name in &obj_names {
            let mol_obj = match ctx.viewer.objects_mut().get_molecule_mut(obj_name) {
                Some(m) => m,
                None => continue,
            };

            let n_states = mol_obj.molecule().state_count();
            if n_states == 0 {
                continue;
            }

            // Filter to valid indices for this object
            let valid: Vec<usize> = indices.iter().copied().filter(|&i| i < n_states).collect();
            if valid.is_empty() {
                continue;
            }

            let count = valid.len();
            mol_obj.molecule_mut().remove_states(&valid);
            total_deleted += count;
            objects_modified += 1;
        }

        if total_deleted == 0 {
            return Err(CmdError::execution(
                "no matching states found to delete".to_string(),
            ));
        }

        ctx.viewer.update_movie_state_count();
        ctx.viewer.request_redraw();

        if !ctx.quiet {
            if objects_modified == 1 {
                ctx.print(&format!(
                    " Deleted {} state(s) from \"{}\".",
                    total_deleted, obj_names[0]
                ));
            } else {
                ctx.print(&format!(
                    " Deleted {} state(s) from {} objects.",
                    total_deleted, objects_modified
                ));
            }
        }

        Ok(())
    }
}

// ============================================================================
// extract command
// ============================================================================

struct ExtractCommand;

impl Command for ExtractCommand {
    fn name(&self) -> &str {
        "extract"
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::None, ArgHint::Selection]
    }

    command_help! {
        CMD "extract"
        DESCRIPTION [
            "creates a new object from a selection and removes",
            "those atoms from the source object.",
        ]
        REQUIRED [
            { "name", "string", "name for the new object" },
            { "selection", "string", "selection of atoms to extract" },
        ]
        OPTIONAL [
            { "source_state", "integer", "state to copy from (0 = all)", "0" },
        ]
        EXAMPLES [
            "extract ligand, organic",
            "extract chainA, chain A",
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
        let source_state = args.int_arg_or(2, "source_state", 0);
        let state_opt = state_index_from_user(source_state);

        // Evaluate selection
        let results = evaluate_selection(ctx.viewer, selection)?;

        // Find first object with selected atoms
        let (src_name, sel_result) = results
            .into_iter()
            .find(|(_, sel)| sel.count() > 0)
            .ok_or_else(|| CmdError::selection(format!("No atoms matching '{}'", selection)))?;

        // Extract molecule from source (read-only borrow)
        let (new_mol, remove_indices) = {
            let mol_obj = ctx
                .viewer
                .objects()
                .get_molecule(&src_name)
                .ok_or_else(|| CmdError::object_not_found(src_name.clone()))?;
            let src_mol = mol_obj.molecule();

            let new_mol = extract_molecule(src_mol, &sel_result, name, state_opt);
            let remove_indices: Vec<AtomIndex> = sel_result.indices().collect();
            (new_mol, remove_indices)
        };

        let atom_count = new_mol.atom_count();

        // Copy visible representations from source object
        let source_reps = ctx
            .viewer
            .objects()
            .get_molecule(&src_name)
            .map(|obj| obj.visible_reps())
            .unwrap_or_default();

        // Add the new object preserving per-atom representations from source
        let mut mol_obj = MoleculeObject::from_raw_with_name(new_mol, name);
        mol_obj.set_visible_reps(source_reps);
        ctx.viewer.insert_object(Box::new(mol_obj));

        // Remove atoms from source
        ctx.viewer
            .remove_molecule_atoms(&src_name, &remove_indices)
            .map_err(|error| CmdError::scene(error.to_string()))?;

        ctx.viewer.request_redraw();

        if !ctx.quiet {
            ctx.print(&format!(
                " Extracted {} atoms from \"{}\" into \"{}\"",
                atom_count, src_name, name
            ));
        }

        Ok(())
    }
}

// ============================================================================
// remove command
// ============================================================================

struct RemoveCommand;

impl Command for RemoveCommand {
    fn name(&self) -> &str {
        "remove"
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Selection]
    }

    command_help! {
        CMD "remove"
        DESCRIPTION [
            "eliminates the atoms in a selection from their respective",
            "molecular objects. If all atoms of an object are removed, the object",
            "is deleted entirely.",
        ]
        REQUIRED [
            { "selection", "string", "atom selection expression" },
        ]
        OPTIONAL []
        EXAMPLES [
            "remove resn HOH",
            "remove elem H",
            "remove not polymer",
            "remove resi 124",
        ]
        SEE ALSO [
            "delete, extract",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let selection = args
            .str_arg(0, "selection")
            .ok_or_else(|| CmdError::missing_argument("selection".to_string()))?;

        // Evaluate selection across all objects
        let results = evaluate_selection(ctx.viewer, selection)?;

        // Collect object names and indices before mutating.
        let removals = results
            .into_iter()
            .filter_map(|(obj_name, sel_result)| {
                if sel_result.count() == 0 {
                    return None;
                }
                let indices: Vec<AtomIndex> = sel_result.indices().collect();
                Some((obj_name, indices))
            })
            .collect::<Vec<_>>();

        if removals.is_empty() {
            if !ctx.quiet {
                ctx.print(&format!(" No atoms selected by '{}'", selection));
            }
            return Ok(());
        }

        let mut total_removed = 0usize;
        // Apply removals
        for (obj_name, indices) in &removals {
            total_removed += indices.len();
            ctx.viewer
                .remove_molecule_atoms(obj_name, indices)
                .map_err(|error| CmdError::scene(error.to_string()))?;
        }

        ctx.viewer.request_redraw();

        if !ctx.quiet {
            ctx.print(&format!(" Removed {} atom(s).", total_removed));
        }

        Ok(())
    }
}

// ============================================================================
// dss command
// ============================================================================

/// Define secondary structure using backbone geometry
struct DssCommand;

impl Command for DssCommand {
    fn name(&self) -> &str {
        "dss"
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Selection]
    }

    command_help! {
        CMD "dss"
        DESCRIPTION [
            "calculates secondary structure assignments using backbone",
            "phi/psi dihedral angles, using the DSS algorithm.",
            "",
            "This overwrites any existing secondary structure assignments from",
            "the PDB/CIF file with computed values based on backbone geometry.",
        ]
        REQUIRED []
        OPTIONAL [
            { "selection", "string", "atoms to process", "all" },
            { "state", "integer", "coordinate state to use", "1" },
            { "quiet", "0/1", "suppress feedback", "0" },
        ]
        NOTES("NOTES") [
            "The algorithm classifies residues based on phi/psi angles:",
            "- Alpha helix: phi ~-57\u{00b0}, psi ~-48\u{00b0}",
            "- Beta strand: phi ~-124\u{00b0}, psi ~124\u{00b0}",
            "",
            "This is automatically applied when loading structures if the",
            "auto_dss setting is enabled (default: on).",
        ]
        EXAMPLES [
            "dss",
            "dss protein",
            "dss all, 1, quiet=1",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let _selection = args.str_arg_or(0, "selection", "all");
        let state_idx = state_index_from_user(args.int_arg_or(1, "state", 1)).unwrap_or(0);
        let quiet = args.bool_arg_or(2, "quiet", false);

        // Get molecule names first to avoid borrowing conflicts
        let mol_names: Vec<String> = ctx
            .viewer
            .objects()
            .names()
            .map(|s| s.to_string())
            .collect();

        let algorithm = ctx.viewer.settings().behavior.dss_algorithm;
        let assigner = assigner_for(algorithm);
        let mut total_updated = 0usize;

        // Apply DSS to all molecule objects
        // TODO: Support selection filtering when selection system is more developed
        for name in mol_names {
            if let Some(mol_obj) = ctx.viewer.objects_mut().get_molecule_mut(&name) {
                let mol = mol_obj.molecule_mut();
                let updated = assign_secondary_structure(mol, state_idx, assigner.as_ref());
                total_updated += updated;
            }
        }

        ctx.viewer.request_redraw();

        if !quiet {
            ctx.print(&format!(
                " Assigned secondary structure to {} atoms",
                total_updated
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::CommandContext;
    use crate::parser::parse_command;
    use crate::CommandExecutor;
    use patinae_mol::{AtomBuilder, AtomIndex, BondOrder, CoordSet, Element};
    use patinae_scene::{
        AtomAnchor, LabelEntity, LabelObject, MeasurementEntity, MeasurementKind,
        MeasurementObject, MoleculeObject, Session, SessionAdapter,
    };
    use patinae_settings::groups::RecentPickLimit;

    /// Create a molecule with two residues: ALA (3 atoms) and HOH (1 atom)
    fn create_test_molecule(name: &str) -> patinae_mol::ObjectMolecule {
        let mut mol = patinae_mol::ObjectMolecule::new(name);

        // ALA residue: 3 atoms (CA, CB, C)
        let ca = AtomBuilder::new()
            .name("CA")
            .element(Element::Carbon)
            .resn("ALA")
            .resv(1)
            .chain("A")
            .build();
        let cb = AtomBuilder::new()
            .name("CB")
            .element(Element::Carbon)
            .resn("ALA")
            .resv(1)
            .chain("A")
            .build();
        let c = AtomBuilder::new()
            .name("C")
            .element(Element::Carbon)
            .resn("ALA")
            .resv(1)
            .chain("A")
            .build();

        // HOH residue: 1 atom (O)
        let o = AtomBuilder::new()
            .name("O")
            .element(Element::Oxygen)
            .resn("HOH")
            .resv(2)
            .chain("A")
            .build();

        let i_ca = mol.add_atom(ca);
        let i_cb = mol.add_atom(cb);
        let i_c = mol.add_atom(c);
        let _i_o = mol.add_atom(o);

        let _ = mol.add_bond(i_ca, i_cb, BondOrder::Single);
        let _ = mol.add_bond(i_ca, i_c, BondOrder::Single);

        // Add a coordinate set
        let cs = patinae_mol::CoordSet::from_vec3(&[
            lin_alg::f32::Vec3::new(0.0, 0.0, 0.0),
            lin_alg::f32::Vec3::new(1.5, 0.0, 0.0),
            lin_alg::f32::Vec3::new(0.0, 1.5, 0.0),
            lin_alg::f32::Vec3::new(3.0, 0.0, 0.0),
        ]);
        mol.add_coord_set(cs);
        mol
    }

    fn create_multistate_molecule(name: &str) -> patinae_mol::ObjectMolecule {
        let mut mol = patinae_mol::ObjectMolecule::new(name);
        mol.add_atom(
            AtomBuilder::new()
                .name("CA")
                .element(Element::Carbon)
                .resn("ALA")
                .resv(1)
                .chain("A")
                .build(),
        );
        mol.add_coord_set(CoordSet::from_vec3(&[lin_alg::f32::Vec3::new(
            0.0, 0.0, 0.0,
        )]));
        mol.add_coord_set(CoordSet::from_vec3(&[lin_alg::f32::Vec3::new(
            9.0, 0.0, 0.0,
        )]));
        mol
    }

    fn setup_session_with(mol: patinae_mol::ObjectMolecule) -> Session {
        let mut session = Session::new();
        session.registry.add(MoleculeObject::new(mol));
        session
    }

    fn run_remove(session: &mut Session, cmd_str: &str) -> CmdResult {
        let mut needs_redraw = false;
        let mut adapter = SessionAdapter {
            session,
            render_context: None,
            default_size: (800, 600),
            needs_redraw: &mut needs_redraw,
            async_fetch_fn: None,
        };
        let parsed = parse_command(cmd_str).map_err(CmdError::parse)?;
        let viewer: &mut dyn ViewerLike = &mut adapter;
        let mut ctx = CommandContext::new(viewer).with_quiet(true);
        RemoveCommand.execute(&mut ctx, &parsed)
    }

    fn run_state(session: &mut Session, cmd_str: &str) -> CmdResult {
        let mut needs_redraw = false;
        let mut adapter = SessionAdapter {
            session,
            render_context: None,
            default_size: (800, 600),
            needs_redraw: &mut needs_redraw,
            async_fetch_fn: None,
        };
        let parsed = parse_command(cmd_str).map_err(CmdError::parse)?;
        let viewer: &mut dyn ViewerLike = &mut adapter;
        let mut ctx = CommandContext::new(viewer).with_quiet(true);
        StateCommand.execute(&mut ctx, &parsed)
    }

    fn run_extract(session: &mut Session, cmd_str: &str) -> CmdResult {
        let mut needs_redraw = false;
        let mut adapter = SessionAdapter {
            session,
            render_context: None,
            default_size: (800, 600),
            needs_redraw: &mut needs_redraw,
            async_fetch_fn: None,
        };
        let parsed = parse_command(cmd_str).map_err(CmdError::parse)?;
        let viewer: &mut dyn ViewerLike = &mut adapter;
        let mut ctx = CommandContext::new(viewer).with_quiet(true);
        ExtractCommand.execute(&mut ctx, &parsed)
    }

    fn run_registered(session: &mut Session, cmd_str: &str) -> CmdResult {
        let mut needs_redraw = false;
        let mut adapter = SessionAdapter {
            session,
            render_context: None,
            default_size: (800, 600),
            needs_redraw: &mut needs_redraw,
            async_fetch_fn: None,
        };
        CommandExecutor::new().do_(&mut adapter, cmd_str)
    }

    fn recent_path(session: &Session, object_name: &str, atom_index: u32) -> String {
        let molecule = session.registry.get_molecule(object_name).unwrap();
        patinae_scene::canonical_atom_path_for_hit(
            &patinae_scene::PickHit {
                object_name: object_name.to_string(),
                object_type: patinae_scene::ObjectType::Molecule,
                atom_index: Some(AtomIndex(atom_index)),
                position: lin_alg::f32::Vec3::new(0.0, 0.0, 0.0),
                distance: 0.0,
            },
            molecule.molecule(),
        )
        .unwrap()
    }

    fn add_atom_annotations(session: &mut Session, source_name: &str, atom_index: AtomIndex) {
        let anchor = AtomAnchor::new(source_name, atom_index);
        session.registry.add(LabelObject::with_entities(
            "labels",
            vec![LabelEntity::new(anchor.clone(), "selected")],
        ));
        let mut measurement = MeasurementObject::new("distance", MeasurementKind::Distance);
        measurement
            .add_entry(MeasurementEntity::new(vec![
                anchor,
                AtomAnchor::new(source_name, AtomIndex(0)),
            ]))
            .unwrap();
        session.registry.add(measurement);
    }

    #[test]
    fn test_remove_by_resn() {
        let mol = create_test_molecule("test");
        let mut session = setup_session_with(mol);

        assert_eq!(
            session
                .registry
                .get_molecule("test")
                .unwrap()
                .molecule()
                .atom_count(),
            4
        );

        run_remove(&mut session, "remove resn HOH").unwrap();

        let obj = session.registry.get_molecule("test").unwrap();
        assert_eq!(obj.molecule().atom_count(), 3);
    }

    #[test]
    fn test_remove_all_deletes_object() {
        let mol = create_test_molecule("test");
        let mut session = setup_session_with(mol);

        run_remove(&mut session, "remove all").unwrap();

        assert!(session.registry.get_molecule("test").is_none());
    }

    #[test]
    fn test_remove_empty_selection() {
        let mol = create_test_molecule("test");
        let mut session = setup_session_with(mol);

        // resn GLY doesn't exist in our molecule
        run_remove(&mut session, "remove resn GLY").unwrap();

        let obj = session.registry.get_molecule("test").unwrap();
        assert_eq!(obj.molecule().atom_count(), 4);
    }

    #[test]
    fn test_remove_by_element() {
        let mol = create_test_molecule("test");
        let mut session = setup_session_with(mol);

        // Remove all oxygen atoms (the HOH water)
        run_remove(&mut session, "remove elem O").unwrap();

        let obj = session.registry.get_molecule("test").unwrap();
        assert_eq!(obj.molecule().atom_count(), 3);
    }

    #[test]
    fn rename_remove_and_delete_commands_reconcile_recent_atom_paths() {
        let mut session = setup_session_with(create_test_molecule("test"));
        let ca = recent_path(&session, "test", 0);
        let cb = recent_path(&session, "test", 1);
        let water = recent_path(&session, "test", 3);
        session
            .recent_atoms
            .insert(ca.clone(), RecentPickLimit::Unlimited);
        let ca_id = session.recent_atoms.row_id(&ca).unwrap();
        session.recent_atoms.insert(cb, RecentPickLimit::Unlimited);
        session
            .recent_atoms
            .insert(water, RecentPickLimit::Unlimited);

        run_registered(&mut session, "rename test, renamed").unwrap();
        assert_eq!(session.recent_atoms.rows()[0].id(), ca_id);
        assert!(session
            .recent_atoms
            .paths()
            .all(|path| path.starts_with("/renamed/")));

        run_registered(&mut session, "remove name CB").unwrap();
        assert_eq!(session.recent_atoms.len(), 2);
        assert!(session
            .recent_atoms
            .paths()
            .all(|path| !path.contains("/CB`")));

        run_registered(&mut session, "delete renamed").unwrap();
        assert!(session.recent_atoms.is_empty());
    }

    #[test]
    fn remove_orphans_annotation_anchors_before_atom_deletion() {
        let mol = create_test_molecule("test");
        let mut session = setup_session_with(mol);
        add_atom_annotations(&mut session, "test", AtomIndex(3));

        run_remove(&mut session, "remove resn HOH").unwrap();

        assert!(session.registry.get_label("labels").unwrap().entities()[0]
            .anchor()
            .is_orphaned());
        assert!(session
            .registry
            .get_measurement("distance")
            .unwrap()
            .entries()[0]
            .anchors[0]
            .is_orphaned());
        assert!(!session
            .registry
            .get_measurement("distance")
            .unwrap()
            .entries()[0]
            .anchors[1]
            .is_orphaned());
    }

    #[test]
    fn extract_orphans_source_annotation_anchors_before_atom_deletion() {
        let mol = create_test_molecule("test");
        let mut session = setup_session_with(mol);
        add_atom_annotations(&mut session, "test", AtomIndex(3));

        run_extract(&mut session, "extract water, resn HOH").unwrap();

        assert!(session.registry.get_label("labels").unwrap().entities()[0]
            .anchor()
            .is_orphaned());
        assert!(session
            .registry
            .get_measurement("distance")
            .unwrap()
            .entries()[0]
            .anchors[0]
            .is_orphaned());
        assert!(session.registry.get_molecule("water").is_some());
    }

    #[test]
    fn test_state_command_switches_displayed_coord_set() {
        let mol = create_multistate_molecule("test");
        let mut session = setup_session_with(mol);

        run_state(&mut session, "state 2").unwrap();

        let obj = session.registry.get_molecule("test").unwrap();
        assert_eq!(obj.display_state(), 1);
        assert_eq!(obj.molecule().current_state, 0);
        assert_eq!(
            obj.display_coord(AtomIndex(0)).map(|coord| coord.x),
            Some(9.0)
        );
    }
}
