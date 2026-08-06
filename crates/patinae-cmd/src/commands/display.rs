//! Display commands: show, hide, enable, disable, color, set_color, bg_color, label

use ahash::{AHashMap, AHashSet};
use patinae_mol::{three_to_one, Atom, RepMask};
use patinae_scene::{AtomAnchor, DirtyFlags, LabelEntity, LabelObject, ObjectType};
use patinae_select::AtomIndex;

use crate::args::ParsedCommand;
use crate::command::{ArgHint, Command, CommandContext, CommandRegistry, ViewerLike};
use crate::command_help;
use crate::commands::selecting::{evaluate_selection, select_with_context};
use crate::error::{CmdError, CmdResult};
use crate::helpers::{
    for_each_selected_molecule_mut, resolve_object_names, set_enabled_with_group_awareness,
    ResolvedNames,
};

/// Register display commands
pub fn register(registry: &mut CommandRegistry) {
    registry.register(ShowCommand);
    registry.register(HideCommand);
    registry.register(ShowAsCommand);
    registry.register(EnableCommand);
    registry.register(DisableCommand);
    registry.register(ToggleCommand);
    registry.register(ColorCommand);
    registry.register(SetColorCommand);
    registry.register(BgColorCommand);
    registry.register(LabelCommand);
}

/// Parse a representation name into a RepMask value
fn parse_rep(name: &str) -> Option<RepMask> {
    match name.to_lowercase().as_str() {
        "lines" | "line" => Some(RepMask::LINES),
        "sticks" | "stick" => Some(RepMask::STICKS),
        "spheres" | "sphere" => Some(RepMask::SPHERES),
        "surface" | "surf" => Some(RepMask::SURFACE),
        "mesh" => Some(RepMask::MESH),
        "dots" | "dot" => Some(RepMask::DOTS),
        "cartoon" | "cart" => Some(RepMask::CARTOON),
        "ribbon" | "ribb" => Some(RepMask::RIBBON),
        "labels" | "label" => Some(RepMask::LABELS),
        "nonbonded" | "nb_spheres" => Some(RepMask::NONBONDED),
        "cell" => Some(RepMask::CELL),
        "cgo" => Some(RepMask::CGO),
        "callback" => Some(RepMask::CALLBACK),
        "extent" => Some(RepMask::EXTENT),
        "slice" => Some(RepMask::SLICE),
        "everything" | "all" => Some(RepMask::ALL),
        _ => None,
    }
}

// ============================================================================
// show command
// ============================================================================

struct ShowCommand;

impl Command for ShowCommand {
    fn name(&self) -> &str {
        "show"
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Representation, ArgHint::Selection]
    }

    command_help! {
        CMD "show"
        DESCRIPTION [
            "makes representations visible.",
        ]
        REQUIRED []
        OPTIONAL [
            { "representation", "string", "representation type", "all" } => [
                "lines, sticks, spheres, surface, mesh, dots, cartoon, ribbon, labels, etc.",
            ],
            { "selection", "string", "atoms to show", "all" },
        ]
        EXAMPLES [
            "show",
            "show cartoon",
            "show sticks, organic",
            "show surface, chain A",
            "show labels",
            "show labels, distance",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let rep_name = args.str_arg(0, "representation");
        let selection = args.str_arg_or(1, "selection", "all");

        // If no representation specified, show all
        let rep = if let Some(name) = rep_name {
            parse_rep(name).ok_or_else(|| {
                CmdError::invalid_arg(
                    "representation",
                    format!("unknown representation: {}", name),
                )
            })?
        } else {
            RepMask::ALL
        };

        if rep == RepMask::LABELS {
            let affected =
                set_annotation_label_visibility(ctx.viewer, args.str_arg(1, "selection"), true)?;
            ctx.viewer.request_redraw();
            if !ctx.quiet {
                ctx.print(&format!(" Showing labels ({affected} affected)"));
            }
            return Ok(());
        }

        let total_affected = for_each_selected_molecule_mut(
            ctx.viewer,
            selection,
            DirtyFlags::empty(),
            |mol_obj, selected| {
                mol_obj.show_rep_for_selection(selected, rep);
            },
        )?;

        ctx.viewer.request_redraw();

        if !ctx.quiet {
            if total_affected == 0 {
                ctx.print_error(&format!(" Show: selection \"{}\" not found", selection));
            } else if let Some(name) = rep_name {
                ctx.print(&format!(" Showing {}", name));
            } else {
                ctx.print(" Showing all representations");
            }
        }

        Ok(())
    }
}

// ============================================================================
// hide command
// ============================================================================

struct HideCommand;

impl Command for HideCommand {
    fn name(&self) -> &str {
        "hide"
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Representation, ArgHint::Selection]
    }

    command_help! {
        CMD "hide"
        DESCRIPTION [
            "makes representations invisible.",
        ]
        REQUIRED []
        OPTIONAL [
            { "representation", "string", "representation type", "all" } => [
                "lines, sticks, spheres, surface, mesh, dots, cartoon, ribbon, labels, etc.",
            ],
            { "selection", "string", "atoms to hide", "all" },
        ]
        EXAMPLES [
            "hide",
            "hide lines",
            "hide sticks, all",
            "hide labels",
            "hide labels, all",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let rep_name = args.str_arg(0, "representation");
        let selection = args.str_arg_or(1, "selection", "all");

        // If no representation specified, hide all
        let rep = if let Some(name) = rep_name {
            parse_rep(name).ok_or_else(|| {
                CmdError::invalid_arg(
                    "representation",
                    format!("unknown representation: {}", name),
                )
            })?
        } else {
            RepMask::ALL
        };

        if rep == RepMask::LABELS {
            let affected =
                set_annotation_label_visibility(ctx.viewer, args.str_arg(1, "selection"), false)?;
            ctx.viewer.request_redraw();
            if !ctx.quiet {
                ctx.print(&format!(" Hiding labels ({affected} affected)"));
            }
            return Ok(());
        }

        let total_affected = for_each_selected_molecule_mut(
            ctx.viewer,
            selection,
            DirtyFlags::empty(),
            |mol_obj, selected| {
                mol_obj.hide_rep_for_selection(selected, rep);
            },
        )?;

        ctx.viewer.request_redraw();

        if !ctx.quiet {
            if total_affected == 0 {
                ctx.print_error(&format!(" Hide: selection \"{}\" not found", selection));
            } else if let Some(name) = rep_name {
                ctx.print(&format!(" Hiding {}", name));
            } else {
                ctx.print(" Hiding all representations");
            }
        }

        Ok(())
    }
}

/// Routes label representation visibility to semantic annotation objects.
fn set_annotation_label_visibility(
    viewer: &mut dyn ViewerLike,
    selection: Option<&str>,
    visible: bool,
) -> CmdResult<usize> {
    let Some(selection) = selection else {
        return Ok(viewer.objects_mut().set_label_objects_enabled(visible));
    };

    let all = matches!(selection, "all" | "*");
    let selected_owners = match resolve_object_names(viewer.objects(), selection) {
        ResolvedNames::All => viewer.objects().names().map(str::to_string).collect(),
        ResolvedNames::Matched(names) => names,
        ResolvedNames::Unresolved => Vec::new(),
    }
    .into_iter()
    .collect::<AHashSet<_>>();
    let selected_atoms = if all {
        AHashMap::new()
    } else {
        selected_atom_indices(viewer, selection)?
    };

    let label_names = viewer
        .objects()
        .names()
        .filter(|name| viewer.objects().get_label(name).is_some())
        .map(str::to_string)
        .collect::<Vec<_>>();
    let mut affected = 0;
    for name in label_names {
        let entity_indices = {
            let label_object = viewer
                .objects()
                .get_label(&name)
                .expect("collected label object must remain present");
            label_object
                .entities()
                .iter()
                .enumerate()
                .filter_map(|(index, entity)| {
                    let anchor = entity.anchor();
                    let owner_selected = all || selected_owners.contains(&name);
                    let atom_selected = !anchor.is_orphaned()
                        && selected_atoms
                            .get(&anchor.object_name)
                            .is_some_and(|indices| indices.contains(&anchor.atom_index));
                    (owner_selected || atom_selected).then_some(index)
                })
                .collect::<Vec<_>>()
        };
        if let Some(label_object) = viewer.objects_mut().get_label_mut(&name) {
            affected += label_object.set_entities_visible(entity_indices, visible);
        }
    }

    let measurement_names = viewer
        .objects()
        .names()
        .filter(|name| viewer.objects().get_measurement(name).is_some())
        .filter(|name| all || selected_owners.contains(*name))
        .map(str::to_string)
        .collect::<Vec<_>>();
    for name in measurement_names {
        if let Some(measurement) = viewer.objects_mut().get_measurement_mut(&name) {
            affected += measurement.set_entity_labels_visible(0..measurement.len(), visible);
        }
    }

    Ok(affected)
}

fn selected_atom_indices(
    viewer: &dyn ViewerLike,
    selection: &str,
) -> CmdResult<AHashMap<String, AHashSet<AtomIndex>>> {
    let results = evaluate_selection(viewer, selection)?;
    let mut selected = AHashMap::new();
    for (object_name, atom_selection) in results {
        let indices = atom_selection.indices().collect::<AHashSet<_>>();
        if !indices.is_empty() {
            selected.insert(object_name, indices);
        }
    }
    Ok(selected)
}

// ============================================================================
// show_as command (alias: as)
// ============================================================================

struct ShowAsCommand;

impl Command for ShowAsCommand {
    fn name(&self) -> &str {
        "show_as"
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Representation, ArgHint::Selection]
    }

    fn aliases(&self) -> &[&str] {
        &["as"]
    }

    command_help! {
        CMD "as"
        DESCRIPTION [
            "(or \"as\") hides all representations and shows only the specified one.",
        ]
        REQUIRED [
            { "representation", "string", "representation type" } => [
                "lines, sticks, spheres, surface, mesh, cartoon, ribbon, labels, etc.",
            ],
        ]
        OPTIONAL [
            { "selection", "string", "atoms to show", "all" },
        ]
        EXAMPLES [
            "as cartoon",
            "as sticks, organic",
            "as surface, polymer",
            "as mesh, polymer",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let rep_name = args
            .str_arg(0, "representation")
            .ok_or_else(|| CmdError::missing_argument("representation".to_string()))?;
        let selection = args.str_arg_or(1, "selection", "all");

        let rep = parse_rep(rep_name).ok_or_else(|| {
            CmdError::invalid_arg(
                "representation",
                format!("unknown representation: {}", rep_name),
            )
        })?;
        if rep == RepMask::LABELS {
            return Err(CmdError::invalid_arg(
                "representation",
                "label primitives are semantic annotations; use 'show labels'",
            ));
        }

        let total_affected = for_each_selected_molecule_mut(
            ctx.viewer,
            selection,
            DirtyFlags::empty(),
            |mol_obj, selected| {
                mol_obj.show_as_rep_for_selection(selected, rep);
            },
        )?;

        ctx.viewer.request_redraw();

        if !ctx.quiet {
            if total_affected == 0 {
                ctx.print_error(&format!(" Show as: selection \"{}\" not found", selection));
            } else {
                ctx.print(&format!(" Showing as {}", rep_name));
            }
        }

        Ok(())
    }
}

// ============================================================================
// enable command
// ============================================================================

struct EnableCommand;

impl Command for EnableCommand {
    fn name(&self) -> &str {
        "enable"
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Selection]
    }

    command_help! {
        CMD "enable"
        DESCRIPTION [
            "makes objects visible.",
        ]
        REQUIRED []
        OPTIONAL [
            { "name", "string", "object name pattern", "all" },
        ]
        EXAMPLES [
            "enable",
            "enable protein",
            "enable obj*",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let name = args.str_arg_or(0, "name", "all");
        set_visibility(ctx, name, true)
    }
}

// ============================================================================
// disable command
// ============================================================================

struct DisableCommand;

impl Command for DisableCommand {
    fn name(&self) -> &str {
        "disable"
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Selection]
    }

    command_help! {
        CMD "disable"
        DESCRIPTION [
            "makes objects or selections invisible.",
        ]
        REQUIRED []
        OPTIONAL [
            { "name", "string", "object or selection name pattern", "all" },
        ]
        EXAMPLES [
            "disable",
            "disable protein",
            "disable sele",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let name = args.str_arg_or(0, "name", "all");
        set_visibility(ctx, name, false)
    }
}

/// Shared implementation for enable/disable commands.
fn set_visibility(
    ctx: &mut CommandContext<'_, '_, dyn ViewerLike + '_>,
    name: &str,
    enabled: bool,
) -> CmdResult {
    match resolve_object_names(ctx.viewer.objects(), name) {
        ResolvedNames::All => {
            let all_names: Vec<String> = ctx
                .viewer
                .objects()
                .names()
                .map(|s| s.to_string())
                .collect();
            for obj_name in &all_names {
                set_enabled_with_group_awareness(ctx.viewer.objects_mut(), obj_name, enabled);
            }
        }
        ResolvedNames::Matched(names) => {
            for obj_name in &names {
                set_enabled_with_group_awareness(ctx.viewer.objects_mut(), obj_name, enabled);
            }
        }
        ResolvedNames::Unresolved => {}
    }

    let matching_sels: Vec<String> = ctx
        .viewer
        .selections()
        .matching(name)
        .iter()
        .map(|s| s.to_string())
        .collect();
    for sel_name in &matching_sels {
        ctx.viewer.selections_mut().set_visible(sel_name, enabled);
    }

    ctx.viewer.request_redraw();

    if !ctx.quiet {
        let verb = if enabled { "Enabled" } else { "Disabled" };
        ctx.print(&format!(" {} \"{}\"", verb, name));
    }

    Ok(())
}

// ============================================================================
// toggle command
// ============================================================================

struct ToggleCommand;

impl Command for ToggleCommand {
    fn name(&self) -> &str {
        "toggle"
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Selection]
    }

    command_help! {
        CMD "toggle"
        DESCRIPTION [
            "toggles visibility of objects or selections.",
        ]
        REQUIRED [
            { "name", "string", "object or selection name" },
        ]
        OPTIONAL []
        EXAMPLES [
            "toggle protein",
            "toggle sele",
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

        // Try object first
        if let Some(obj) = ctx.viewer.objects().get(name) {
            let currently_enabled = obj.is_enabled();
            set_enabled_with_group_awareness(ctx.viewer.objects_mut(), name, !currently_enabled);

            if !ctx.quiet {
                if currently_enabled {
                    ctx.print(&format!(" Disabled \"{}\"", name));
                } else {
                    ctx.print(&format!(" Enabled \"{}\"", name));
                }
            }
        } else if ctx.viewer.selections().names().contains(&name.to_string()) {
            // Try selection
            let currently_visible = ctx.viewer.selections().is_visible(name);
            ctx.viewer
                .selections_mut()
                .set_visible(name, !currently_visible);

            if !ctx.quiet {
                if currently_visible {
                    ctx.print(&format!(" Disabled selection \"{}\"", name));
                } else {
                    ctx.print(&format!(" Enabled selection \"{}\"", name));
                }
            }
        } else {
            return Err(CmdError::object_not_found(name.to_string()));
        }

        ctx.viewer.request_redraw();

        Ok(())
    }
}

// ============================================================================
// color command
// ============================================================================

struct ColorCommand;

impl Command for ColorCommand {
    fn name(&self) -> &str {
        "color"
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Color, ArgHint::Selection]
    }

    fn aliases(&self) -> &[&str] {
        &["colour"]
    }

    command_help! {
        CMD "color"
        DESCRIPTION [
            "sets the color of atoms or objects.",
        ]
        REQUIRED [
            { "color", "string", "color name or special scheme" } => [
                "Named colors: red, green, blue, yellow, cyan, magenta, orange, white, gray, etc.",
                "Special schemes:",
                "    atomic (cpk, element) - color by element type",
                "    chain (chainbow) - color by chain",
                "    ss (secondary_structure) - color by secondary structure",
                "    b (b_factor, bfactor) - color by B-factor",
                "    residue (residue_type, aa_type) - color by residue type",
                "    index (residue_index, rainbow) - color by residue index",
            ],
        ]
        OPTIONAL [
            { "selection", "string", "atoms to color", "all" },
        ]
        EXAMPLES [
            "color red",
            "color green, chain A",
            "color cyan, organic",
            "color atomic",
            "color chain",
            "color ss, polymer",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let color_name = args
            .str_arg(0, "color")
            .ok_or_else(|| CmdError::missing_argument("color".to_string()))?;
        let selection = args.str_arg_or(1, "selection", "all");

        let resolved_color =
            if let Some(color_index) = patinae_color::ColorIndex::from_scheme_name(color_name) {
                color_index
            } else if let Some(idx) = ctx.viewer.color_index(color_name) {
                patinae_color::ColorIndex::Named(idx)
            } else if let Some(color) = patinae_color::Color::from_hex(color_name) {
                let index = ctx.viewer.named_palette_mut().set(color_name, color);
                patinae_color::ColorIndex::Named(index)
            } else {
                return Err(CmdError::invalid_arg(
                    "color",
                    format!("unknown color: {}", color_name),
                ));
            };

        let annotation_type = ctx
            .viewer
            .objects()
            .get(selection)
            .map(|object| object.object_type())
            .filter(|object_type| {
                matches!(object_type, ObjectType::Measurement | ObjectType::Label)
            });
        if let Some(annotation_type) = annotation_type {
            let patinae_color::ColorIndex::Named(index) = resolved_color else {
                return Err(CmdError::invalid_arg(
                    "color",
                    "annotation objects require a named or hexadecimal color",
                ));
            };
            match annotation_type {
                ObjectType::Measurement => ctx
                    .viewer
                    .objects_mut()
                    .get_measurement_mut(selection)
                    .expect("measurement type checked above")
                    .set_color(patinae_color::ColorIndex::Named(index)),
                ObjectType::Label => ctx
                    .viewer
                    .objects_mut()
                    .get_label_mut(selection)
                    .expect("label type checked above")
                    .set_color(patinae_color::ColorIndex::Named(index)),
                _ => unreachable!("annotation type filter admits only measurement and label"),
            }
            ctx.viewer.request_redraw();
            if !ctx.quiet {
                ctx.print(&format!(
                    " Color: {} {} colored {}",
                    annotation_type, selection, color_name
                ));
            }
            return Ok(());
        }

        let color_index = i32::from(resolved_color);
        let total_colored = for_each_selected_molecule_mut(
            ctx.viewer,
            selection,
            DirtyFlags::COLOR,
            |mol_obj, selected| {
                let mol_mut = mol_obj.molecule_mut();
                for idx in selected.indices() {
                    if let Some(atom) = mol_mut.get_atom_mut(AtomIndex(idx.0)) {
                        atom.repr.colors.base = color_index;
                        atom.repr.colors.cartoon = color_index;
                        atom.repr.colors.ribbon = color_index;
                        atom.repr.colors.stick = color_index;
                        atom.repr.colors.line = color_index;
                        atom.repr.colors.sphere = color_index;
                        atom.repr.colors.surface = color_index;
                        atom.repr.colors.mesh = color_index;
                        atom.repr.colors.dot = color_index;
                        atom.repr.colors.ellipsoid = color_index;
                    }
                }
            },
        )?;

        ctx.viewer.request_redraw();

        if !ctx.quiet {
            if total_colored == 0 {
                ctx.print_error(&format!(
                    " Color: 0 atoms colored {} (selection not found)",
                    color_name
                ));
            } else {
                ctx.print(&format!(
                    " Color: {} atoms colored {}",
                    total_colored, color_name
                ));
            }
        }

        Ok(())
    }
}

// ============================================================================
// bg_color command
// ============================================================================

struct BgColorCommand;

impl Command for BgColorCommand {
    fn name(&self) -> &str {
        "bg_color"
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Color]
    }

    fn aliases(&self) -> &[&str] {
        &["bg_colour", "background"]
    }

    command_help! {
        CMD "bg_color"
        DESCRIPTION [
            "sets the background color.",
        ]
        REQUIRED []
        OPTIONAL [
            { "color", "string", "color name (white, black, gray, etc.)", "theme default" },
        ]
        EXAMPLES [
            "bg_color",
            "bg_color white",
            "bg_color black",
            "bg_color gray",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        if args.arg_count() == 0 {
            ctx.viewer.reset_background_color();
            if !ctx.quiet {
                ctx.print(" Background color reset to theme default");
            }
            return Ok(());
        }

        // Try [r, g, b] vector first (from ArgValue::List)
        if let Some(crate::args::ArgValue::List(items)) = args.get_arg(0) {
            if items.len() == 3 {
                if let (Some(r), Some(g), Some(b)) = (
                    items[0].as_float().map(|v| v as f32),
                    items[1].as_float().map(|v| v as f32),
                    items[2].as_float().map(|v| v as f32),
                ) {
                    ctx.viewer.set_background_color(
                        r.clamp(0.0, 1.0),
                        g.clamp(0.0, 1.0),
                        b.clamp(0.0, 1.0),
                    );
                    if !ctx.quiet {
                        ctx.print(&format!(
                            " Background color set to [{:.2}, {:.2}, {:.2}]",
                            r, g, b
                        ));
                    }
                    return Ok(());
                }
            }
        }

        let color_name = args
            .str_arg(0, "color")
            .ok_or_else(|| CmdError::missing_argument("color".to_string()))?;

        // Resolve color: named colors registry, then hex
        let color = if let Some((_, color)) = ctx.viewer.named_palette().get_by_name(color_name) {
            color
        } else if let Some(color) = patinae_color::Color::from_hex(color_name) {
            color
        } else {
            return Err(CmdError::invalid_arg(
                "color",
                format!("unknown color: {}", color_name),
            ));
        };

        let [r, g, b] = color.to_array();
        ctx.viewer.set_background_color(r, g, b);

        if !ctx.quiet {
            ctx.print(&format!(" Background color set to {}", color_name));
        }

        Ok(())
    }
}

// ============================================================================
// set_color command
// ============================================================================

struct SetColorCommand;

impl Command for SetColorCommand {
    fn name(&self) -> &str {
        "set_color"
    }

    fn aliases(&self) -> &[&str] {
        &["set_colour"]
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Color]
    }

    command_help! {
        CMD "set_color"
        DESCRIPTION [
            "defines a new named color or removes an existing one.",
        ]
        USAGE [
            "set_color name, [ r, g, b ]",
            "set_color name",
        ]
        REQUIRED [
            { "name", "string", "the color name to define or remove" },
        ]
        OPTIONAL [
            { "[r, g, b]", "list of integers (0-255)", "the RGB color value", "none" },
        ]
        NOTES("NOTES") [
            "If only a name is provided, the named color is removed.",
            "If a name and RGB list are provided, the named color is created or updated.",
        ]
        EXAMPLES [
            "set_color mywhite, [255, 255, 255]",
            "set_color darkred, [128, 0, 0]",
            "set_color mywhite",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let color_name = args
            .str_arg(0, "name")
            .ok_or_else(|| CmdError::missing_argument("name".to_string()))?;

        if let Some(crate::args::ArgValue::List(items)) = args.get_arg(1) {
            if items.len() != 3 {
                return Err(CmdError::invalid_arg(
                    "rgb",
                    format!("expected [r, g, b] (3 values), got {} values", items.len()),
                ));
            }

            let r = items[0]
                .as_int()
                .ok_or_else(|| CmdError::invalid_arg("r", "expected an integer"))?
                as u8;
            let g = items[1]
                .as_int()
                .ok_or_else(|| CmdError::invalid_arg("g", "expected an integer"))?
                as u8;
            let b = items[2]
                .as_int()
                .ok_or_else(|| CmdError::invalid_arg("b", "expected an integer"))?
                as u8;

            let color = patinae_color::Color::from_rgb8(r, g, b);
            let idx = ctx.viewer.named_palette_mut().set(color_name, color);

            ctx.viewer.request_redraw();

            if !ctx.quiet {
                ctx.print(&format!(
                    " Color: \"{}\" defined as [{}, {}, {}] (index {})",
                    color_name, r, g, b, idx
                ));
            }
        } else {
            let removed = ctx.viewer.named_palette_mut().unregister(color_name);

            if !ctx.quiet {
                if removed {
                    ctx.print(&format!(" Color: \"{}\" removed", color_name));
                } else {
                    ctx.print_warning(&format!(
                        " Color: \"{}\" not found (nothing removed)",
                        color_name
                    ));
                }
            }
        }

        Ok(())
    }
}

// ============================================================================
// label command
// ============================================================================

/// Selects which atom value a typed label displays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LabelExpression {
    /// Atom name.
    Name,
    /// Residue name.
    Resn,
    /// Residue number plus insertion code.
    Resi,
    /// Chain identifier.
    Chain,
    /// Occupancy (PyMOL convention: q = occupancy)
    Q,
    /// B-factor
    B,
    /// Segment identifier.
    Segi,
    /// "ATOM" or "HETATM"
    Type,
    /// Formal charge.
    FormalCharge,
    /// Partial charge.
    PartialCharge,
    /// Element symbol (e.g., "C", "N", "O")
    Elem,
    /// Van der Waals radius
    Vdw,
    /// One-letter amino acid code
    Oneletter,
    /// Literal text preserved exactly as supplied by the native UI.
    Literal(String),
}

impl LabelExpression {
    /// Parses a built-in label expression key.
    pub fn from_builtin_key(key: &str) -> Option<Self> {
        Some(match key.to_ascii_lowercase().as_str() {
            "name" => Self::Name,
            "resn" => Self::Resn,
            "resi" => Self::Resi,
            "chain" => Self::Chain,
            "q" => Self::Q,
            "b" => Self::B,
            "segi" => Self::Segi,
            "type" => Self::Type,
            "formal_charge" => Self::FormalCharge,
            "partial_charge" => Self::PartialCharge,
            "elem" | "element" => Self::Elem,
            "vdw" => Self::Vdw,
            "oneletter" | "one_letter" => Self::Oneletter,
            _ => return None,
        })
    }
}

/// Selects whether a typed label request creates or appends an object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LabelTarget {
    /// Create a new object with a registry-allocated `labelNN` name.
    New,
    /// Append to an existing label object.
    Existing(String),
}

/// Requests ordered labels for one through four singleton atom paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelRequest {
    /// Ordered atom selection paths shown in the native operand queue.
    pub operands: Vec<String>,
    /// Expression evaluated independently for every atom.
    pub expression: LabelExpression,
    /// Destination for the resulting label entities.
    pub target: LabelTarget,
}

impl LabelRequest {
    /// Creates a typed label request.
    pub fn new<I, S>(operands: I, expression: LabelExpression, target: LabelTarget) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            operands: operands.into_iter().map(Into::into).collect(),
            expression,
            target,
        }
    }
}

/// Describes one successfully applied typed label request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelOutcome {
    /// Object created or appended by the request.
    pub object_name: String,
    /// Number of appended entities.
    pub entity_count: usize,
}

/// Parse a label expression string into a LabelExpression
fn parse_label_expr(s: &str) -> Result<LabelExpression, CmdError> {
    let trimmed = s.trim();

    if trimmed.is_empty() {
        return Err(CmdError::invalid_arg(
            "expression",
            "label expression must not be empty",
        ));
    }

    // Check for quoted string literal
    if (trimmed.starts_with('"') && trimmed.ends_with('"'))
        || (trimmed.starts_with('\'') && trimmed.ends_with('\''))
    {
        let inner = &trimmed[1..trimmed.len() - 1];
        if inner.trim().is_empty() {
            return Err(CmdError::invalid_arg(
                "expression",
                "label expression must not be empty",
            ));
        }
        return Ok(LabelExpression::Literal(inner.to_string()));
    }

    // Anything else is a string literal (quotes are stripped by the command parser).
    Ok(LabelExpression::from_builtin_key(trimmed)
        .unwrap_or_else(|| LabelExpression::Literal(trimmed.to_string())))
}

/// Evaluate a label expression for a given atom
fn eval_label_expr(expr: &LabelExpression, atom: &Atom) -> String {
    match expr {
        LabelExpression::Name => atom.name.to_string(),
        LabelExpression::Resn => atom.residue.resn.clone(),
        LabelExpression::Resi => {
            if atom.residue.inscode != ' ' {
                format!("{}{}", atom.residue.resv, atom.residue.inscode)
            } else {
                atom.residue.resv.to_string()
            }
        }
        LabelExpression::Chain => atom.residue.chain.clone(),
        LabelExpression::Q => format!("{:.2}", atom.occupancy),
        LabelExpression::B => format!("{:.2}", atom.b_factor),
        LabelExpression::Segi => atom.residue.segi.clone(),
        LabelExpression::Type => {
            if atom.state.hetatm {
                "HETATM".to_string()
            } else {
                "ATOM".to_string()
            }
        }
        LabelExpression::FormalCharge => atom.formal_charge.to_string(),
        LabelExpression::PartialCharge => format!("{:.4}", atom.partial_charge),
        LabelExpression::Elem => atom.element.symbol().to_string(),
        LabelExpression::Vdw => format!("{:.2}", atom.effective_vdw()),
        LabelExpression::Oneletter => three_to_one(&atom.residue.resn)
            .map(|c| c.to_string())
            .unwrap_or_else(|| atom.residue.resn.clone()),
        LabelExpression::Literal(s) => s.clone(),
    }
}

fn typed_label_entity(
    viewer: &dyn ViewerLike,
    operand: &str,
    expression: &LabelExpression,
) -> CmdResult<LabelEntity> {
    let (total_count, results) = select_with_context(viewer, operand)?;
    if total_count != 1 {
        return Err(CmdError::selection(format!(
            "operand '{operand}' does not resolve to exactly one atom"
        )));
    }
    for (object_name, selected) in results {
        let Some(molecule) = viewer.objects().get_molecule(&object_name) else {
            continue;
        };
        if let Some(atom_index) = selected.indices().next() {
            let atom = molecule.molecule().get_atom(atom_index).ok_or_else(|| {
                CmdError::selection(format!("operand '{operand}' refers to a stale atom"))
            })?;
            return Ok(LabelEntity::new(
                AtomAnchor::new(object_name.clone(), atom_index),
                eval_label_expr(expression, atom),
            ));
        }
    }
    Err(CmdError::selection(format!(
        "operand '{operand}' does not resolve to exactly one atom"
    )))
}

fn validate_label_target(viewer: &dyn ViewerLike, name: &str, require_existing: bool) -> CmdResult {
    match viewer.objects().get(name) {
        Some(existing) if viewer.objects().get_label(name).is_none() => Err(CmdError::invalid_arg(
            "object",
            format!("object '{name}' is {}, not a label", existing.object_type()),
        )),
        Some(_) => Ok(()),
        None if require_existing => Err(CmdError::object_not_found(name)),
        None => Ok(()),
    }
}

fn add_labels_to_scene(
    viewer: &mut dyn ViewerLike,
    object_name: &str,
    entities: Vec<LabelEntity>,
) -> CmdResult {
    validate_label_target(viewer, object_name, false)?;
    if let Some(label_object) = viewer.objects_mut().get_label_mut(object_name) {
        label_object.extend_entities(entities);
    } else {
        viewer.objects_mut().add(LabelObject::with_entities(
            object_name.to_string(),
            entities,
        ));
    }
    viewer.request_redraw();
    Ok(())
}

/// Validates and applies one typed label request transactionally.
///
/// Every operand and the target object are validated before any label is
/// appended. Automatic names are allocated only after validation succeeds.
///
/// # Errors
///
/// Returns an error for invalid cardinality, empty literal text, stale or
/// non-singleton operands, and missing or incompatible existing targets.
pub fn execute_label_request(
    viewer: &mut dyn ViewerLike,
    request: &LabelRequest,
) -> CmdResult<LabelOutcome> {
    if !(1..=4).contains(&request.operands.len()) {
        return Err(CmdError::invalid_arg(
            "operands",
            "labels require between 1 and 4 atoms",
        ));
    }
    if matches!(&request.expression, LabelExpression::Literal(text) if text.trim().is_empty()) {
        return Err(CmdError::invalid_arg(
            "expression",
            "literal label text must not be empty",
        ));
    }
    if let LabelTarget::Existing(name) = &request.target {
        validate_label_target(viewer, name, true)?;
    }

    let entities = request
        .operands
        .iter()
        .map(|operand| typed_label_entity(viewer, operand, &request.expression))
        .collect::<CmdResult<Vec<_>>>()?;
    let entity_count = entities.len();
    let object_name = match &request.target {
        LabelTarget::New => viewer.objects().first_free_label_name(),
        LabelTarget::Existing(name) => name.clone(),
    };
    add_labels_to_scene(viewer, &object_name, entities)?;
    Ok(LabelOutcome {
        object_name,
        entity_count,
    })
}

struct LabelCommand;

fn collect_label_entities(
    viewer: &dyn ViewerLike,
    selection: &str,
    expression: &LabelExpression,
) -> CmdResult<Vec<LabelEntity>> {
    let results = evaluate_selection(viewer, selection)?;
    let mut entities = Vec::new();
    for (object_name, selected) in results {
        let Some(molecule) = viewer.objects().get_molecule(&object_name) else {
            continue;
        };
        for index in selected.indices() {
            let Some(atom) = molecule.molecule().get_atom(index) else {
                continue;
            };
            entities.push(LabelEntity::new(
                AtomAnchor::new(object_name.clone(), index),
                eval_label_expr(expression, atom),
            ));
        }
    }
    if entities.is_empty() {
        return Err(CmdError::selection(format!(
            "no atoms found in selection '{selection}'"
        )));
    }
    Ok(entities)
}

impl Command for LabelCommand {
    fn name(&self) -> &str {
        "label"
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Selection, ArgHint::LabelProperty, ArgHint::Object]
    }

    command_help! {
        CMD "label"
        DESCRIPTION [
            "creates atom-anchored semantic label collections.",
        ]
        REQUIRED [
            { "selection", "string", "atoms to label" },
            { "expression", "string", "property to display" } => [
                "name           - atom name",
                "resn           - residue name",
                "resi           - residue number/identifier",
                "chain          - chain identifier",
                "q              - occupancy",
                "b              - B-factor",
                "segi           - segment identifier",
                "type           - ATOM or HETATM",
                "formal_charge  - formal charge",
                "partial_charge - partial charge",
                "\"string\"       - literal string",
            ],
        ]
        OPTIONAL [
            { "object", "string", "label object to create or append", "automatic labelNN" },
        ]
        EXAMPLES [
            "label all, name",
            "label chain A, resn",
            "label organic, resi",
            "label sele, \"hello\"",
            "label name CA, resn, object=ca_labels",
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
        let expression = args
            .str_arg(1, "expression")
            .ok_or_else(|| CmdError::missing_argument("expression"))?;
        let expression = parse_label_expr(expression)?;

        let requested_name = match args.get_named("object") {
            None => None,
            Some(value) => Some(value.as_str().ok_or_else(|| {
                CmdError::invalid_arg("object", "label object name must be a string")
            })?),
        };
        if let Some(name) = requested_name {
            let trimmed_name = name.trim();
            if trimmed_name.is_empty() || matches!(trimmed_name, "\"\"" | "''") {
                return Err(CmdError::invalid_arg(
                    "object",
                    "label object name must not be empty",
                ));
            }
            validate_label_target(ctx.viewer, name, false)?;
        }

        let entities = collect_label_entities(ctx.viewer, selection, &expression)?;
        let count = entities.len();
        let object_name = requested_name
            .map(str::to_string)
            .unwrap_or_else(|| ctx.viewer.objects().first_free_label_name());

        add_labels_to_scene(ctx.viewer, &object_name, entities)?;

        if !ctx.quiet {
            ctx.print(&format!(
                " Label: {count} atoms labeled in \"{object_name}\""
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        execute_label_request, parse_rep, LabelExpression, LabelOutcome, LabelRequest, LabelTarget,
    };
    use crate::commands::selecting::evaluate_selection;
    use crate::error::CmdResult;
    use crate::CommandExecutor;
    use lin_alg::f32::Vec3;
    use patinae_algos::surface::Grid3D;
    use patinae_color::ThemedPalette;
    use patinae_mol::{Atom, Element, ObjectMolecule, RepMask};
    use patinae_scene::{
        AtomAnchor, GroupObject, LabelEntity, LabelObject, MapObject, MeasurementEntity,
        MeasurementKind, MeasurementObject, MoleculeObject, Object, Session, SessionAdapter,
    };
    use patinae_select::{AtomIndex, SelectionResult};

    fn execute_display_command(session: &mut Session, command: &str) -> CmdResult<bool> {
        let mut needs_redraw = false;
        {
            let mut adapter = SessionAdapter {
                session,
                render_context: None,
                default_size: (64, 64),
                needs_redraw: &mut needs_redraw,
                async_fetch_fn: None,
            };
            CommandExecutor::new().do_(&mut adapter, command)?;
        }
        Ok(needs_redraw)
    }

    fn run_display_command(session: &mut Session, command: &str) -> bool {
        execute_display_command(session, command).unwrap()
    }

    fn execute_typed_label(
        session: &mut Session,
        request: &LabelRequest,
    ) -> CmdResult<LabelOutcome> {
        let mut needs_redraw = false;
        let mut adapter = SessionAdapter {
            session,
            render_context: None,
            default_size: (64, 64),
            needs_redraw: &mut needs_redraw,
            async_fetch_fn: None,
        };
        execute_label_request(&mut adapter, request)
    }

    fn cartoon_object_named(name: &str) -> MoleculeObject {
        let mut mol = ObjectMolecule::new(name);
        mol.add_atom(Atom::new("CA", Element::Carbon));
        mol.add_atom(Atom::new("CB", Element::Carbon));
        mol.add_coord_set(patinae_mol::CoordSet::from_vec3(&[
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
        ]));
        MoleculeObject::new(mol)
    }

    #[test]
    fn typed_labels_cover_expressions_and_preserve_escaped_literal() {
        let mut session = Session::new();
        session.registry.add(cartoon_object_named("source"));
        let expressions = [
            LabelExpression::Name,
            LabelExpression::Resn,
            LabelExpression::Resi,
            LabelExpression::Chain,
            LabelExpression::Q,
            LabelExpression::B,
            LabelExpression::Segi,
            LabelExpression::Type,
            LabelExpression::FormalCharge,
            LabelExpression::PartialCharge,
            LabelExpression::Elem,
            LabelExpression::Vdw,
            LabelExpression::Oneletter,
            LabelExpression::Literal("quoted \"text\" \\ path".to_string()),
        ];

        for expression in expressions {
            let request = LabelRequest::new(["name CA"], expression, LabelTarget::New);
            execute_typed_label(&mut session, &request).unwrap();
        }

        assert_eq!(session.registry.get_label("label01").unwrap().len(), 1);
        assert_eq!(
            session.registry.get_label("label14").unwrap().entities()[0].text(),
            "quoted \"text\" \\ path"
        );
    }

    #[test]
    fn typed_labels_allocate_at_execution_and_append_ordered_operands() {
        let mut session = Session::new();
        session.registry.add(cartoon_object_named("source"));
        let requests = [
            LabelRequest::new(["name CA"], LabelExpression::Name, LabelTarget::New),
            LabelRequest::new(["name CB"], LabelExpression::Name, LabelTarget::New),
        ];

        let first = execute_typed_label(&mut session, &requests[0]).unwrap();
        let second = execute_typed_label(&mut session, &requests[1]).unwrap();
        assert_eq!(first.object_name, "label01");
        assert_eq!(second.object_name, "label02");

        let append = LabelRequest::new(
            ["name CA", "name CB", "name CA", "name CB"],
            LabelExpression::Name,
            LabelTarget::Existing("label01".to_string()),
        );
        execute_typed_label(&mut session, &append).unwrap();
        let labels = session.registry.get_label("label01").unwrap();
        assert_eq!(
            labels
                .entities()
                .iter()
                .map(LabelEntity::text)
                .collect::<Vec<_>>(),
            ["CA", "CA", "CB", "CA", "CB"]
        );
    }

    #[test]
    fn typed_label_rejects_invalid_operand_and_target_without_partial_append() {
        let mut session = Session::new();
        session.registry.add(cartoon_object_named("source"));
        session.registry.add(LabelObject::new("labels"));
        session.registry.add(MeasurementObject::new(
            "distance",
            MeasurementKind::Distance,
        ));

        let invalid_operand = LabelRequest::new(
            ["name CA", "all"],
            LabelExpression::Name,
            LabelTarget::Existing("labels".to_string()),
        );
        assert!(execute_typed_label(&mut session, &invalid_operand).is_err());
        assert!(session.registry.get_label("labels").unwrap().is_empty());

        let wrong_target = LabelRequest::new(
            ["name CA"],
            LabelExpression::Name,
            LabelTarget::Existing("distance".to_string()),
        );
        assert!(execute_typed_label(&mut session, &wrong_target).is_err());
        assert!(session.registry.get_label("labels").unwrap().is_empty());

        let stale_target = LabelRequest::new(
            ["name CA"],
            LabelExpression::Name,
            LabelTarget::Existing("missing".to_string()),
        );
        assert!(execute_typed_label(&mut session, &stale_target).is_err());
        assert!(session.registry.get_label("missing").is_none());

        let empty_literal = LabelRequest::new(
            ["name CA"],
            LabelExpression::Literal("   ".to_string()),
            LabelTarget::Existing("labels".to_string()),
        );
        assert!(execute_typed_label(&mut session, &empty_literal).is_err());
        assert!(session.registry.get_label("labels").unwrap().is_empty());
    }

    #[test]
    fn typed_labels_accept_each_supported_operand_count() {
        let mut session = Session::new();
        session.registry.add(cartoon_object_named("source"));
        session.registry.add(LabelObject::new("labels"));

        for count in 1..=4 {
            let request = LabelRequest::new(
                std::iter::repeat_n("name CA", count),
                LabelExpression::Name,
                LabelTarget::Existing("labels".to_string()),
            );
            execute_typed_label(&mut session, &request).unwrap();
        }

        assert_eq!(session.registry.get_label("labels").unwrap().len(), 10);
    }

    fn prepare_partial_cartoon_then_full_hide(obj: &mut MoleculeObject) {
        let all_atoms = SelectionResult::all(obj.molecule().atom_count());
        obj.hide_rep_for_selection(&all_atoms, RepMask::CARTOON);
        let one_atom =
            SelectionResult::from_indices(obj.molecule().atom_count(), [AtomIndex(0)].into_iter());
        obj.show_rep_for_selection(&one_atom, RepMask::CARTOON);
        obj.hide_rep_for_selection(&all_atoms, RepMask::CARTOON);
        obj.clear_dirty();
    }

    #[test]
    fn display_parse_rep_accepts_mesh() {
        assert_eq!(parse_rep("mesh"), Some(RepMask::MESH));
    }

    #[test]
    fn bg_color_name_sets_explicit_background() {
        let mut session = Session::new();

        let needs_redraw = run_display_command(&mut session, "bg_color white");

        assert!(needs_redraw);
        assert_eq!(session.clear_color, [1.0, 1.0, 1.0]);
        assert!(session.clear_color_set);
    }

    #[test]
    fn bg_color_without_args_resets_to_theme_background() {
        let mut session = Session::new();
        session.palette = ThemedPalette::light();
        let theme_bg = session.palette.viewport_bg.to_array();
        run_display_command(&mut session, "bg_color white");
        assert!(session.clear_color_set);

        let needs_redraw = run_display_command(&mut session, "bg_color");

        assert!(needs_redraw);
        assert_eq!(session.clear_color, theme_bg);
        assert!(!session.clear_color_set);
    }

    #[test]
    fn color_named_updates_exact_measurement_material() {
        let mut session = Session::new();
        session.registry.add(MeasurementObject::new(
            "distance",
            MeasurementKind::Distance,
        ));
        let before = session
            .registry
            .get_measurement("distance")
            .unwrap()
            .revisions()
            .material;

        assert!(run_display_command(&mut session, "color red, distance"));

        let red = session.named_palette.get_by_name("red").unwrap().0;
        let measurement = session.registry.get_measurement("distance").unwrap();
        assert_eq!(
            measurement.state().color,
            patinae_color::ColorIndex::Named(red)
        );
        assert!(measurement.has_explicit_color());
        assert!(measurement.revisions().material > before);
    }

    #[test]
    fn color_scheme_rejects_exact_measurement_without_mutation() {
        let mut session = Session::new();
        session.registry.add(MeasurementObject::new(
            "distance",
            MeasurementKind::Distance,
        ));
        let before = session
            .registry
            .get_measurement("distance")
            .unwrap()
            .revisions();
        let mut needs_redraw = false;
        let error = {
            let mut adapter = SessionAdapter {
                session: &mut session,
                render_context: None,
                default_size: (64, 64),
                needs_redraw: &mut needs_redraw,
                async_fetch_fn: None,
            };
            CommandExecutor::new()
                .do_(&mut adapter, "color chain, distance")
                .unwrap_err()
        };

        assert!(error.to_string().contains("named or hexadecimal"));
        let measurement = session.registry.get_measurement("distance").unwrap();
        assert_eq!(measurement.revisions(), before);
        assert_eq!(
            measurement.state().color,
            patinae_color::ColorIndex::default()
        );
        assert!(!needs_redraw);
    }

    #[test]
    fn color_named_updates_exact_label_material() {
        let mut session = Session::new();
        session.registry.add(LabelObject::new("labels"));
        let before = session.registry.get_label("labels").unwrap().revisions();

        assert!(run_display_command(&mut session, "color red, labels"));

        let red = session.named_palette.get_by_name("red").unwrap().0;
        let labels = session.registry.get_label("labels").unwrap();
        assert_eq!(
            labels.presentation().color(),
            Some(patinae_color::ColorIndex::Named(red))
        );
        assert!(labels.revisions().material > before.material);
        assert!(labels.revisions().labels > before.labels);
    }

    #[test]
    fn bg_color_background_alias_without_args_resets_to_theme_background() {
        let mut session = Session::new();
        let theme_bg = session.palette.viewport_bg.to_array();
        run_display_command(&mut session, "bg_color white");
        assert!(session.clear_color_set);

        let needs_redraw = run_display_command(&mut session, "background");

        assert!(needs_redraw);
        assert_eq!(session.clear_color, theme_bg);
        assert!(!session.clear_color_set);
    }

    #[test]
    fn show_cartoon_command_matches_helper_after_invalid_draw_mask_restore() {
        let mut helper_obj = cartoon_object_named("obj");
        prepare_partial_cartoon_then_full_hide(&mut helper_obj);
        let all_atoms = SelectionResult::all(helper_obj.molecule().atom_count());
        helper_obj.show_rep_for_selection(&all_atoms, RepMask::CARTOON);

        let mut command_obj = cartoon_object_named("obj");
        prepare_partial_cartoon_then_full_hide(&mut command_obj);
        let mut session = Session::new();
        session.registry.add(command_obj);

        assert!(run_display_command(&mut session, "show cartoon, obj"));
        let command_obj = session.registry.get_molecule("obj").unwrap();

        assert_eq!(command_obj.dirty_flags(), helper_obj.dirty_flags());
        assert_eq!(command_obj.visible_reps(), helper_obj.visible_reps());
        assert_eq!(command_obj.draw_reps(), helper_obj.draw_reps());
        assert_eq!(
            command_obj.draw_mask_restorable_reps(),
            helper_obj.draw_mask_restorable_reps()
        );
        let helper_atom_reps: Vec<_> = helper_obj
            .molecule()
            .atoms()
            .map(|atom| atom.repr.visible_reps)
            .collect();
        let command_atom_reps: Vec<_> = command_obj
            .molecule()
            .atoms()
            .map(|atom| atom.repr.visible_reps)
            .collect();
        assert_eq!(command_atom_reps, helper_atom_reps);
    }

    #[test]
    fn label_command_creates_one_ordered_collection_per_invocation() {
        let mut session = Session::new();
        session.registry.add(cartoon_object_named("source"));

        run_display_command(&mut session, "label source, name");
        run_display_command(&mut session, "label source, resi");

        let first = session.registry.get_label("label01").unwrap();
        assert_eq!(first.len(), 2);
        assert_eq!(
            first
                .entities()
                .iter()
                .map(LabelEntity::text)
                .collect::<Vec<_>>(),
            vec!["CA", "CB"]
        );
        assert_eq!(first.entities()[0].anchor().object_name, "source");
        assert_eq!(first.entities()[1].anchor().object_name, "source");

        let second = session.registry.get_label("label02").unwrap();
        assert_eq!(second.len(), 2);
        assert_eq!(
            second
                .entities()
                .iter()
                .map(LabelEntity::text)
                .collect::<Vec<_>>(),
            vec!["0", "0"]
        );
        assert!(session
            .registry
            .get_molecule("source")
            .unwrap()
            .molecule()
            .atoms()
            .all(|atom| atom.repr.label.is_empty()));
    }

    #[test]
    fn unnamed_label_uses_first_free_name_in_shared_namespace() {
        let mut session = Session::new();
        session.registry.add(cartoon_object_named("label01"));
        session.registry.add(cartoon_object_named("source"));

        run_display_command(&mut session, "label source, name");

        assert!(session.registry.get_label("label02").is_some());
        assert!(session.registry.get_molecule("label01").is_some());
    }

    #[test]
    fn named_label_appends_duplicate_anchors_without_upsert() {
        let mut session = Session::new();
        session.registry.add(cartoon_object_named("source"));

        run_display_command(
            &mut session,
            "label name CA, name, object=active_site_labels",
        );
        run_display_command(
            &mut session,
            "label name CA, resi, object=active_site_labels",
        );

        let labels = session.registry.get_label("active_site_labels").unwrap();
        assert_eq!(labels.len(), 2);
        assert_eq!(
            labels.entities()[0].anchor().atom_index,
            labels.entities()[1].anchor().atom_index
        );
        assert_eq!(labels.entities()[0].text(), "CA");
        assert_eq!(labels.entities()[1].text(), "0");
    }

    #[test]
    fn label_failures_do_not_mutate_registry_or_consume_name() {
        let mut session = Session::new();
        session.registry.add(cartoon_object_named("source"));
        session
            .registry
            .add(MeasurementObject::new("taken", MeasurementKind::Distance));
        session.registry.add(GroupObject::new("group"));
        session.registry.add(MapObject::new(
            "map",
            Grid3D::from_dims([0.0; 3], [1.0; 3], [1, 1, 1], vec![0.0; 8]),
        ));
        let before_len = session.registry.len();
        let before_measurement = session
            .registry
            .get_measurement("taken")
            .unwrap()
            .revisions();

        let empty_error = execute_display_command(&mut session, "label name ZZ, name").unwrap_err();
        assert!(empty_error.to_string().contains("no atoms"));
        let expression_error =
            execute_display_command(&mut session, "label source, \"\"").unwrap_err();
        assert!(expression_error.to_string().contains("expression"));
        let conflict_error =
            execute_display_command(&mut session, "label source, name, object=taken").unwrap_err();
        assert!(conflict_error.to_string().contains("not a label"));
        for target in ["source", "group", "map"] {
            let error = execute_display_command(
                &mut session,
                &format!("label name CA, name, object={target}"),
            )
            .unwrap_err();
            assert!(error.to_string().contains("not a label"));
        }
        let name_error =
            execute_display_command(&mut session, "label source, name, object=\"\"").unwrap_err();
        assert!(name_error.to_string().contains("must not be empty"));

        assert_eq!(session.registry.len(), before_len);
        assert_eq!(
            session
                .registry
                .get_measurement("taken")
                .unwrap()
                .revisions(),
            before_measurement
        );
        assert!(session.registry.get_label("label01").is_none());

        run_display_command(&mut session, "label source, name");
        assert!(session.registry.get_label("label01").is_some());
    }

    #[test]
    fn expressionless_label_is_an_error_and_preserves_collections() {
        let mut session = Session::new();
        session.registry.add(cartoon_object_named("source"));
        run_display_command(&mut session, "label source, name");
        let before = session.registry.get_label("label01").unwrap().revisions();

        let error = execute_display_command(&mut session, "label source").unwrap_err();

        assert!(error.to_string().contains("expression"));
        let labels = session.registry.get_label("label01").unwrap();
        assert_eq!(labels.len(), 2);
        assert_eq!(labels.revisions(), before);
    }

    #[test]
    fn label_selection_predicate_reads_semantic_label_text() {
        let mut session = Session::new();
        session.registry.add(cartoon_object_named("source"));
        run_display_command(&mut session, "label name CA, semantic_label");

        let mut needs_redraw = false;
        let adapter = SessionAdapter {
            session: &mut session,
            render_context: None,
            default_size: (64, 64),
            needs_redraw: &mut needs_redraw,
            async_fetch_fn: None,
        };
        let selected = evaluate_selection(&adapter, "label semantic_label").unwrap();

        assert_eq!(
            selected
                .iter()
                .map(|(_, result)| result.count())
                .sum::<usize>(),
            1
        );
        assert_eq!(
            selected[0].1.indices().collect::<Vec<_>>(),
            vec![AtomIndex(0)]
        );
    }

    #[test]
    fn show_hide_labels_routes_object_entity_and_measurement_visibility() {
        let mut session = Session::new();
        session.registry.add(cartoon_object_named("source"));
        run_display_command(&mut session, "label source, name");

        let anchors = [AtomIndex(0), AtomIndex(1)]
            .into_iter()
            .map(|index| AtomAnchor::new("source", index))
            .collect::<Vec<_>>();
        let mut measurement = MeasurementObject::new("distance", MeasurementKind::Distance);
        measurement
            .add_entry(MeasurementEntity::new(anchors))
            .unwrap();
        measurement
            .entity_presentation_mut(0)
            .unwrap()
            .set_label_visible(true);
        session.registry.add(measurement);

        run_display_command(&mut session, "hide labels");
        assert!(!session.registry.get_label("label01").unwrap().is_enabled());
        assert!(session
            .registry
            .get_measurement("distance")
            .unwrap()
            .is_enabled());

        run_display_command(&mut session, "show labels");
        assert!(session.registry.get_label("label01").unwrap().is_enabled());

        run_display_command(&mut session, "hide labels, name CA");
        let labels = session.registry.get_label("label01").unwrap();
        assert_eq!(labels.entities()[0].presentation().visible(), Some(false));
        assert_eq!(labels.entities()[1].presentation().visible(), None);

        run_display_command(&mut session, "hide labels, distance");
        let measurement = session.registry.get_measurement("distance").unwrap();
        assert_eq!(
            measurement.entries()[0].presentation().label_visible(),
            Some(false)
        );
        assert_eq!(measurement.presentation().label_visible(), None);
        assert!(measurement.is_enabled());

        run_display_command(&mut session, "show labels, all");
        let labels = session.registry.get_label("label01").unwrap();
        assert!(labels
            .entities()
            .iter()
            .all(|entity| entity.presentation().visible() == Some(true)));
        let measurement = session.registry.get_measurement("distance").unwrap();
        assert_eq!(
            measurement.entries()[0].presentation().label_visible(),
            Some(true)
        );
        assert_eq!(measurement.presentation().label_visible(), None);
        assert!(measurement.is_enabled());
    }
}
