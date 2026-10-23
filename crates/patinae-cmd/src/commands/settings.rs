//! Settings commands: set, get, unset, dss

use crate::args::ParsedCommand;
use crate::command::{ArgHint, Command, CommandContext, CommandRegistry, ViewerLike};
use crate::command_help;
use crate::commands::selecting::evaluate_selection;
use crate::error::{CmdError, CmdResult};
use crate::ResolvedSetting;
use patinae_color::ColorValue;
use patinae_scene::{DirtyFlags, MoleculeObject, Object};
use patinae_settings::{SettingType, SettingValue, SideEffectCategory};

/// Register settings commands
pub fn register(registry: &mut CommandRegistry) {
    registry.register(SetCommand);
    registry.register(GetCommand);
    registry.register(UnsetCommand);
    registry.register(SetupCcdCommand);
}

/// Resolve deprecated setting names to their canonical forms, printing a
/// deprecation warning to `ctx` when an alias is hit. Returns the canonical
/// name; callers should use the returned string for descriptor lookup.
fn resolve_legacy_setting_name<'v, 'r, 'a>(
    ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
    name: &'a str,
) -> std::borrow::Cow<'a, str> {
    match name {
        "transparency" => {
            ctx.print_warning("'transparency' is deprecated, use 'surface_transparency' instead");
            std::borrow::Cow::Borrowed("surface_transparency")
        }
        _ => std::borrow::Cow::Borrowed(name),
    }
}

/// Format a setting source as an unknown-setting error.
fn unknown_setting(name: &str) -> CmdError {
    CmdError::invalid_arg("name", format!("Unknown setting: {}", name))
}

fn format_object_report(
    setting: &ResolvedSetting,
    value: &SettingValue,
    object_name: &str,
    palette: &patinae_color::NamedPalette,
) -> String {
    let raw = ResolvedSetting::format_value(value, palette);
    if let Some(hint_name) = setting.hint_name(value) {
        format!(
            "{} = {} ({}, object {})",
            setting.name(),
            raw,
            hint_name,
            object_name
        )
    } else {
        format!(
            "{} ({}) = {} (object {})",
            setting.name(),
            setting.setting_type(),
            raw,
            object_name
        )
    }
}

/// Apply side effects from a slice of categories.
///
/// Shared between built-in and dynamic settings.
fn apply_side_effects_from_slice<'v, 'r>(
    ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
    side_effects: &[SideEffectCategory],
    setting_name: &str,
    value: &SettingValue,
) {
    for category in side_effects {
        match category {
            SideEffectCategory::RepresentationRebuild => {
                if setting_name == "surface_quality" {
                    if let Some(quality) = value.as_int() {
                        let names: Vec<_> = ctx
                            .viewer
                            .objects()
                            .names()
                            .map(|s| s.to_string())
                            .collect();
                        for obj_name in names {
                            if let Some(mol) = ctx.viewer.objects_mut().get_molecule_mut(&obj_name)
                            {
                                mol.set_surface_quality(quality);
                            }
                        }
                    }
                }
                let names: Vec<_> = ctx
                    .viewer
                    .objects()
                    .names()
                    .map(|s| s.to_string())
                    .collect();
                for obj_name in names {
                    if let Some(mol) = ctx.viewer.objects_mut().get_molecule_mut(&obj_name) {
                        mol.invalidate_representations();
                    }
                }
            }
            SideEffectCategory::ColorRebuild => {
                let names: Vec<_> = ctx
                    .viewer
                    .objects()
                    .names()
                    .map(|s| s.to_string())
                    .collect();
                for obj_name in names {
                    if let Some(mol) = ctx.viewer.objects_mut().get_molecule_mut(&obj_name) {
                        mol.invalidate(DirtyFlags::COLOR);
                    }
                }
            }
            SideEffectCategory::SurfaceTransparency => {
                let names: Vec<_> = ctx
                    .viewer
                    .objects()
                    .names()
                    .map(|s| s.to_string())
                    .collect();
                for obj_name in names {
                    if let Some(mol) = ctx.viewer.objects_mut().get_molecule_mut(&obj_name) {
                        mol.invalidate(DirtyFlags::TRANSPARENCY);
                    }
                }
            }
            SideEffectCategory::SceneInvalidate
            | SideEffectCategory::SceneChanged
            | SideEffectCategory::FullRebuild
            | SideEffectCategory::ViewportUpdate => {}
            _ => {}
        }
    }
}

fn apply_object_side_effects(
    mol: &mut MoleculeObject,
    side_effects: &[SideEffectCategory],
    setting_name: &str,
    value: &SettingValue,
) {
    let mut dirty = DirtyFlags::empty();

    for category in side_effects {
        match category {
            SideEffectCategory::RepresentationRebuild => {
                if setting_name == "surface_quality" {
                    if let Some(quality) = value.as_int() {
                        mol.set_surface_quality(quality);
                    }
                }
                dirty |= DirtyFlags::REPS;
            }
            SideEffectCategory::ColorRebuild => {
                dirty |= DirtyFlags::COLOR;
            }
            SideEffectCategory::SurfaceTransparency => {
                dirty |= DirtyFlags::TRANSPARENCY;
            }
            SideEffectCategory::SceneInvalidate
            | SideEffectCategory::SceneChanged
            | SideEffectCategory::ViewportUpdate => {
                dirty |= DirtyFlags::REPS;
            }
            SideEffectCategory::FullRebuild => {
                dirty |= DirtyFlags::ALL;
            }
            _ => {}
        }
    }

    if dirty.is_empty() {
        dirty = DirtyFlags::REPS;
    }
    mol.invalidate(dirty);
}

// ============================================================================
// set command
// ============================================================================

struct SetCommand;

fn set_selection_color(
    viewer: &mut dyn ViewerLike,
    selection: &str,
    name: &str,
    color: ColorValue,
) -> CmdResult<usize> {
    let selections = evaluate_selection(viewer, selection)?;
    let mut changed = 0;
    for (object_name, selected) in selections {
        if !selected.any() {
            continue;
        }
        if let Some(object) = viewer.objects_mut().get_molecule_mut(&object_name) {
            changed += object
                .set_representation_color(&selected, name, color)
                .expect("representation setting");
        }
    }
    if changed != 0 {
        viewer.request_redraw();
    }
    Ok(changed)
}

impl SetCommand {
    /// Parse a value string with per-setting overrides.
    fn parse_value<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        setting: &ResolvedSetting,
        value_str: &str,
    ) -> CmdResult<SettingValue> {
        // Built-in color settings resolve names and color schemes through the viewer.
        if setting.setting_type() == SettingType::Color {
            let color = ctx
                .viewer
                .named_palette()
                .parse_color(value_str)
                .map_err(|error| CmdError::invalid_arg("value", format!("{error}: {value_str}")))?;
            return Ok(SettingValue::Color(color));
        }

        setting.parse_value(value_str)
    }

    /// Execute a `set` for a resolved setting.
    fn execute_setting<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
        setting: ResolvedSetting,
        value_str: Option<String>,
        selection: Option<&str>,
    ) -> CmdResult {
        let name = setting.name().to_string();

        // Handle missing value (boolean toggle)
        let value_str = match value_str {
            Some(v) => v,
            None => {
                if setting.setting_type() == SettingType::Bool {
                    let current = setting
                        .global_value(ctx.viewer)
                        .map_err(CmdError::execution)?
                        .as_bool()
                        .unwrap_or(false);
                    let new_value = !current;
                    let value = SettingValue::Bool(new_value);
                    setting
                        .set_global(ctx.viewer, value.clone())
                        .map_err(CmdError::execution)?;
                    ctx.viewer.reconcile_recent_atom_setting(&name);
                    apply_side_effects_from_slice(ctx, setting.side_effects(), &name, &value);
                    ctx.viewer.request_redraw();
                    if !ctx.quiet {
                        ctx.print(&format!(
                            " {} = {}",
                            name,
                            if new_value { "on" } else { "off" }
                        ));
                    }
                    return Ok(());
                } else {
                    return Err(CmdError::missing_argument("value".to_string()));
                }
            }
        };

        // For Float3 settings or color settings: collect 3 comma-separated floats
        let (value_str, selection) = if (setting.setting_type() == SettingType::Float3
            || setting.setting_type() == SettingType::Color)
            && !value_str.starts_with('[')
            && value_str.parse::<f32>().is_ok()
        {
            if let (Some(y), Some(z)) = (
                args.get_arg(2).and_then(|v| v.to_string_repr()),
                args.get_arg(3).and_then(|v| v.to_string_repr()),
            ) {
                if y.parse::<f32>().is_ok() && z.parse::<f32>().is_ok() {
                    let combined = format!("[{}, {}, {}]", value_str, y, z);
                    let shifted_selection = args.str_arg(4, "selection");
                    (combined, shifted_selection)
                } else {
                    (value_str, selection)
                }
            } else {
                (value_str, selection)
            }
        } else {
            (value_str, selection)
        };

        let value = self.parse_value(ctx, &setting, &value_str)?;
        setting
            .validate_range(&value)
            .map_err(|msg| CmdError::invalid_arg("value", msg))?;

        if selection.is_some() && name == "max_recent_picks" {
            return Err(CmdError::invalid_arg(
                "selection",
                format!("Setting '{name}' does not support object overrides"),
            ));
        }

        if let Some(sel) = selection {
            if patinae_mol::AtomColors::representation_field(&name).is_some() {
                let color = value
                    .as_color()
                    .ok_or_else(|| CmdError::invalid_arg("value", "Expected a color value"))?;
                let changed = set_selection_color(ctx.viewer, sel, &name, color)?;
                if !ctx.quiet {
                    ctx.print(&format!(
                        " {} = {} (selection {}, {} atoms changed)",
                        name,
                        setting.format_display(&value, ctx.viewer.named_palette()),
                        sel,
                        changed
                    ));
                }
                return Ok(());
            }
        }

        if let Some(desc) = setting.built_in_descriptor() {
            // Built-in object override
            if let Some(sel) = selection {
                if desc.is_object_overridable() {
                    if let Some(mol) = ctx.viewer.objects_mut().get_molecule_mut(sel) {
                        desc.set_override(mol.get_or_create_overrides(), value.clone())
                            .map_err(|e| CmdError::execution(e.to_string()))?;
                        apply_object_side_effects(mol, desc.side_effects, desc.name, &value);
                        ctx.viewer.request_redraw();
                        if !ctx.quiet {
                            let display =
                                setting.format_display(&value, ctx.viewer.named_palette());
                            ctx.print(&format!(" {} = {} (object {})", name, display, sel));
                        }
                        return Ok(());
                    }
                    return Err(CmdError::object_not_found(sel.to_string()));
                }
            }
        }

        // Dynamic object override
        if let Some(sel) = selection {
            if setting.is_object_overridable()
                && setting
                    .set_object_value(sel, value.clone())
                    .map_err(CmdError::execution)?
            {
                apply_side_effects_from_slice(ctx, setting.side_effects(), &name, &value);
                ctx.viewer.request_redraw();
                if !ctx.quiet {
                    let display = setting.format_display(&value, ctx.viewer.named_palette());
                    ctx.print(&format!(" {} = {} (object {})", name, display, sel));
                }
                return Ok(());
            }
        }

        // Global set
        setting
            .set_global(ctx.viewer, value.clone())
            .map_err(CmdError::execution)?;
        ctx.viewer.reconcile_recent_atom_setting(&name);
        apply_side_effects_from_slice(ctx, setting.side_effects(), &name, &value);
        ctx.viewer.request_redraw();

        if !ctx.quiet {
            let display = setting.format_display(&value, ctx.viewer.named_palette());
            ctx.print(&format!(" {} = {}", name, display));
        }

        Ok(())
    }
}

impl Command for SetCommand {
    fn name(&self) -> &str {
        "set"
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Setting, ArgHint::SettingValue, ArgHint::Selection]
    }

    command_help! {
        CMD "set"
        DESCRIPTION [
            "changes a setting value.",
            "Representation colors are assigned to selected atoms; an object name selects all its atoms.",
        ]
        USAGE [
            "set name [, value [, selection [, state ]]]",
        ]
        REQUIRED [
            { "name", "string", "setting name" },
        ]
        OPTIONAL [
            { "value", "string", "new value (depends on setting type)", "" },
            { "selection", "string", "object name; representation colors also accept atom selections", "global" },
            { "state", "integer", "state for state-specific settings", "0" },
        ]
        EXAMPLES [
            "set sphere_scale, 0.5",
            "set cartoon_color, red, obj",
            "set stick_color, atomic, sele",
            "set cartoon_color, red",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        // === 1. Parse arguments ===
        let raw_name = args
            .str_arg(0, "name")
            .ok_or_else(|| CmdError::missing_argument("name".to_string()))?;
        let resolved = resolve_legacy_setting_name(ctx, raw_name);
        let name = resolved.as_ref();

        let value_str = args
            .get_arg(1)
            .and_then(|v| v.to_string_repr())
            .or_else(|| args.get_named("value").and_then(|v| v.to_string_repr()));

        let selection = args.str_arg(2, "selection");

        let setting = ctx
            .resolve_setting(name)
            .ok_or_else(|| unknown_setting(name))?;
        self.execute_setting(ctx, args, setting, value_str, selection)
    }
}

// ============================================================================
// get command
// ============================================================================

struct GetCommand;

impl Command for GetCommand {
    fn name(&self) -> &str {
        "get"
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Setting, ArgHint::Selection]
    }

    command_help! {
        CMD "get"
        DESCRIPTION [
            "displays the current value of a setting.",
        ]
        USAGE [
            "get name [, object [, state ]]",
        ]
        REQUIRED [
            { "name", "string", "setting name" },
        ]
        OPTIONAL [
            { "object", "string", "get object override when present", "global" },
            { "state", "integer", "state for state-specific settings", "0" },
        ]
        EXAMPLES [
            "get sphere_scale",
            "get antialias",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let raw_name = args
            .str_arg(0, "name")
            .ok_or_else(|| CmdError::missing_argument("name".to_string()))?;
        let resolved = resolve_legacy_setting_name(ctx, raw_name);
        let name = resolved.as_ref();

        let setting = ctx
            .resolve_setting(name)
            .ok_or_else(|| unknown_setting(name))?;
        let selection = args.str_arg(1, "selection");

        if let Some(sel) = selection {
            if let Some(field) = patinae_mol::AtomColors::representation_field(name) {
                let selected = evaluate_selection(ctx.viewer, sel)?;
                let mut uniform = None;
                let mut mixed = false;
                'objects: for (object, atoms) in selected {
                    if let Some(mol) = ctx.viewer.objects().get_molecule(&object) {
                        let mut agrees = |atom: &patinae_mol::Atom| {
                            if name == "cartoon_nucleic_acid_color"
                                && !atom.state.flags.contains(patinae_mol::AtomFlags::NUCLEIC)
                            {
                                return true;
                            }
                            let value = *field(&mut atom.repr.colors.clone());
                            if uniform.is_some_and(|previous| previous != value) {
                                return false;
                            }
                            uniform = Some(value);
                            true
                        };
                        let molecule = mol.molecule();
                        let same = if atoms.atom_count() == molecule.atom_count() && atoms.is_all()
                        {
                            molecule.atoms().all(&mut agrees)
                        } else {
                            atoms
                                .indices()
                                .filter_map(|index| molecule.get_atom(index))
                                .all(agrees)
                        };
                        if !same {
                            mixed = true;
                            break 'objects;
                        }
                    }
                }
                let value = if mixed {
                    "mixed".to_string()
                } else if let Some(value) = uniform {
                    ctx.viewer.named_palette().format_color(value)
                } else {
                    "empty selection".to_string()
                };
                ctx.print(&format!(" {name} = {value} (selection {sel})"));
                return Ok(());
            }
        }

        if let Some(desc) = setting.built_in_descriptor() {
            if let Some(sel) = selection {
                if desc.is_object_overridable() {
                    if let Some(mol) = ctx.viewer.objects().get_molecule(sel) {
                        if let Some(value) = setting.built_in_object_value(mol) {
                            ctx.print(&format!(
                                " {}",
                                format_object_report(
                                    &setting,
                                    &value,
                                    sel,
                                    ctx.viewer.named_palette()
                                )
                            ));
                            return Ok(());
                        }
                    } else {
                        return Err(CmdError::object_not_found(sel.to_string()));
                    }
                }
            }
        }

        if let Some(sel) = selection {
            if setting.is_object_overridable() {
                if let Some(value) = setting.object_value(sel).map_err(CmdError::execution)? {
                    ctx.print(&format!(
                        " {}",
                        format_object_report(&setting, &value, sel, ctx.viewer.named_palette())
                    ));
                    return Ok(());
                }
            }
        }

        let value = setting
            .global_value(ctx.viewer)
            .map_err(CmdError::execution)?;
        ctx.print(&format!(
            " {}",
            setting.format_report(&value, ctx.viewer.named_palette())
        ));
        Ok(())
    }
}

// ============================================================================
// unset command
// ============================================================================

struct UnsetCommand;

impl Command for UnsetCommand {
    fn name(&self) -> &str {
        "unset"
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Setting, ArgHint::Selection]
    }

    command_help! {
        CMD "unset"
        DESCRIPTION [
            "restores a setting to its default value.",
        ]
        USAGE [
            "unset name [, selection [, state ]]",
        ]
        REQUIRED [
            { "name", "string", "setting name" },
        ]
        OPTIONAL [
            { "selection", "string", "object name; representation colors also accept atom selections", "global" },
            { "state", "integer", "state for state-specific settings", "0" },
        ]
        EXAMPLES [
            "unset sphere_scale",
            "unset ray_trace_mode",
            "unset stick_color, sele",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let raw_name = args
            .str_arg(0, "name")
            .ok_or_else(|| CmdError::missing_argument("name".to_string()))?;
        let resolved = resolve_legacy_setting_name(ctx, raw_name);
        let name = resolved.as_ref();

        let setting = ctx
            .resolve_setting(name)
            .ok_or_else(|| unknown_setting(name))?;
        let selection = args.str_arg(1, "selection");

        if selection.is_some() && name == "max_recent_picks" {
            return Err(CmdError::invalid_arg(
                "selection",
                format!("Setting '{name}' does not support object overrides"),
            ));
        }

        if let Some(sel) = selection {
            if patinae_mol::AtomColors::representation_field(name).is_some() {
                let changed =
                    set_selection_color(ctx.viewer, sel, name, patinae_color::ColorValue::INHERIT)?;
                if !ctx.quiet {
                    ctx.print(&format!(
                        " {} reset to inherited color (selection {}, {} atoms changed)",
                        name, sel, changed
                    ));
                }
                return Ok(());
            }
        }

        if let Some(desc) = setting.built_in_descriptor() {
            if let Some(sel) = selection {
                if desc.is_object_overridable() {
                    let inherited_value = desc.get(ctx.viewer.settings());
                    if let Some(mol) = ctx.viewer.objects_mut().get_molecule_mut(sel) {
                        if let Some(overrides) = mol.overrides_mut() {
                            desc.unset_override(overrides);
                        }
                        apply_object_side_effects(
                            mol,
                            desc.side_effects,
                            desc.name,
                            &inherited_value,
                        );
                        ctx.viewer.request_redraw();
                        if !ctx.quiet {
                            ctx.print(&format!(
                                " {} reset to global default (object {})",
                                name, sel
                            ));
                        }
                        return Ok(());
                    }
                    return Err(CmdError::object_not_found(sel.to_string()));
                }
            }
        }

        if let Some(sel) = selection {
            if setting.is_object_overridable()
                && setting
                    .unset_object_value(sel)
                    .map_err(CmdError::execution)?
            {
                ctx.viewer.request_redraw();
                if !ctx.quiet {
                    ctx.print(&format!(
                        " {} reset to global default (object {})",
                        name, sel
                    ));
                }
                return Ok(());
            }
        }

        let default_value = setting
            .unset_global(ctx.viewer)
            .map_err(CmdError::execution)?;
        ctx.viewer.reconcile_recent_atom_setting(name);
        apply_side_effects_from_slice(ctx, setting.side_effects(), name, &default_value);

        ctx.viewer.request_redraw();

        if !ctx.quiet {
            ctx.print(&format!(
                " {} reset to default: {}",
                name,
                setting.format_display(&default_value, ctx.viewer.named_palette())
            ));
        }

        Ok(())
    }
}

// ============================================================================
// setup_ccd command
// ============================================================================

struct SetupCcdCommand;

impl Command for SetupCcdCommand {
    fn name(&self) -> &str {
        "setup_ccd"
    }

    command_help! {
        CMD "setup_ccd"
        DESCRIPTION [
            "loads Chemical Component Dictionary (CCD) bond templates and",
            "re-bonds all objects using template-based connectivity for",
            "known HETATM residues (HEM, ATP, NAD, etc.).",
        ]
        REQUIRED []
        OPTIONAL [
            { "mode", "string", "'scene' to load only templates for residues in the scene", "all" },
        ]
        EXAMPLES [
            "setup_ccd",
            "setup_ccd scene",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let mode = args.str_arg(0, "mode");

        for name in ctx.viewer.objects().names() {
            if let Some(object) = ctx.viewer.objects().get_molecule(name) {
                object.require_explicit().map_err(CmdError::execution)?;
            }
        }
        // Load CCD cache
        let count = if mode == Some("scene") {
            // Collect unique HETATM residue names from all objects
            let mut resnames = std::collections::HashSet::new();
            let names: Vec<String> = ctx
                .viewer
                .objects()
                .names()
                .map(|s| s.to_string())
                .collect();
            for obj_name in &names {
                if let Some(mol_obj) = ctx.viewer.objects().get_molecule(obj_name) {
                    for atom in mol_obj.molecule().atoms_slice() {
                        if atom.state.hetatm {
                            resnames.insert(atom.residue.resn.clone());
                        }
                    }
                }
            }
            patinae_mol::ccd::load_cache_filtered(&resnames).map_err(CmdError::execution)?
        } else {
            patinae_mol::ccd::load_cache().map_err(CmdError::execution)?
        };
        let bond_tolerance = ctx.viewer.settings().behavior.bonding_vdw_cutoff;

        // Re-bond all objects
        let names: Vec<String> = ctx
            .viewer
            .objects()
            .names()
            .map(|s| s.to_string())
            .collect();
        let mut rebonded = 0;
        for obj_name in &names {
            if let Some(mol_obj) = ctx.viewer.objects_mut().get_molecule_mut(obj_name) {
                mol_obj.molecule_mut().rebond(bond_tolerance);
                mol_obj.invalidate(DirtyFlags::ALL);
                rebonded += 1;
            }
        }

        ctx.viewer.request_redraw();
        if !ctx.quiet {
            ctx.print(&format!(
                " CCD: loaded {} templates, re-bonded {} object(s)",
                count, rebonded
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CommandExecutor;
    use patinae_mol::{AtomBuilder, AtomIndex, ObjectMolecule};
    use patinae_scene::{Session, SessionAdapter};
    use patinae_settings::groups::RecentPickLimit;
    use patinae_settings::registry;

    fn execute(session: &mut Session, executor: &mut CommandExecutor, cmd: &str) -> CmdResult {
        let mut needs_redraw = false;
        let mut adapter = SessionAdapter {
            session,
            render_context: None,
            default_size: (800, 600),
            needs_redraw: &mut needs_redraw,
        };
        executor.do_(&mut adapter, cmd)
    }

    fn session_with_single_atom_object(name: &str) -> Session {
        let mut mol = ObjectMolecule::new(name);
        mol.add_atom(AtomBuilder::new().name("CA").element_symbol("C").build());

        let mut session = Session::new();
        session.registry.add(MoleculeObject::with_name(mol, name));
        session
    }

    #[test]
    fn theme_commands_support_system_and_reset_to_system() {
        use patinae_settings::{SettingEnum as _, ThemeMode};
        let mut session = Session::new();
        let mut executor = CommandExecutor::new();
        assert_eq!(session.settings.ui.theme, ThemeMode::System);
        for mode in [ThemeMode::Dark, ThemeMode::Light, ThemeMode::System] {
            execute(
                &mut session,
                &mut executor,
                &format!("set theme, {}", mode.name()),
            )
            .unwrap();
            assert_eq!(session.settings.ui.theme, mode);
            let mut needs_redraw = false;
            let mut adapter = SessionAdapter {
                session: &mut session,
                render_context: None,
                default_size: (800, 600),
                needs_redraw: &mut needs_redraw,
            };
            let output = executor
                .do_with_options(&mut adapter, "get theme", false)
                .unwrap();
            assert!(output.messages.iter().any(|m| m.text.contains(mode.name())));
        }
        execute(&mut session, &mut executor, "set theme, dark").unwrap();
        execute(&mut session, &mut executor, "unset theme").unwrap();
        assert_eq!(session.settings.ui.theme, ThemeMode::System);
        assert!(execute(&mut session, &mut executor, "set theme, unknown").is_err());
    }

    #[test]
    fn nucleic_cartoon_assignment_and_reset_do_not_touch_protein() {
        let mut session = session_with_case_collisions(("A", "B"), ("CA", "P"));
        let mol = session
            .registry
            .get_molecule_mut("obj")
            .unwrap()
            .molecule_mut();
        mol.get_atom_mut(AtomIndex(0)).unwrap().state.flags = patinae_mol::AtomFlags::PROTEIN;
        mol.get_atom_mut(AtomIndex(1)).unwrap().state.flags = patinae_mol::AtomFlags::NUCLEIC;
        let mut executor = CommandExecutor::new();
        execute(&mut session, &mut executor, "set cartoon_color, blue, obj").unwrap();
        execute(
            &mut session,
            &mut executor,
            "set cartoon_nucleic_acid_color, red, obj",
        )
        .unwrap();
        let blue = session.named_palette.parse_color("blue").unwrap();
        let red = session.named_palette.parse_color("red").unwrap();
        let colors = |session: &Session| {
            session
                .registry
                .get_molecule("obj")
                .unwrap()
                .molecule()
                .atoms()
                .map(|a| a.repr.colors.cartoon)
                .collect::<Vec<_>>()
        };
        assert_eq!(colors(&session), [blue, red]);
        execute(
            &mut session,
            &mut executor,
            "unset cartoon_nucleic_acid_color, obj",
        )
        .unwrap();
        assert_eq!(colors(&session), [blue, ColorValue::INHERIT]);
    }

    #[test]
    fn color_names_and_tagged_literals_select_the_same_atoms() {
        let mut session = session_with_single_atom_object("obj");
        let mut executor = CommandExecutor::new();
        let custom = session
            .named_palette
            .set("custom", patinae_color::Color::RED);
        for text in [
            "red".to_string(),
            "custom".to_string(),
            "#12abef".to_string(),
            "ss".to_string(),
        ] {
            execute(&mut session, &mut executor, &format!("color {text}, obj")).unwrap();
            let value = session.named_palette.parse_color(&text).unwrap();
            for expression in [format!("color {text}"), format!("color {value}")] {
                let mut redraw = false;
                let viewer = SessionAdapter {
                    session: &mut session,
                    render_context: None,
                    default_size: (800, 600),
                    needs_redraw: &mut redraw,
                };
                let selected = evaluate_selection(&viewer, &expression).unwrap();
                assert_eq!(
                    selected
                        .iter()
                        .map(|(_, result)| result.count())
                        .sum::<usize>(),
                    1,
                    "{expression}"
                );
            }
        }
        execute(
            &mut session,
            &mut executor,
            &format!("bg_color palette:{custom}"),
        )
        .unwrap();
    }

    #[test]
    fn color_commands_share_user_facing_color_names() {
        let mut session = session_with_single_atom_object("obj");
        let mut executor = CommandExecutor::new();
        let index = session.named_palette.get_by_name("burntorange").unwrap().0;
        let custom = session
            .named_palette
            .set("custom", patinae_color::Color::RED);
        for (input, expected) in [
            ("BURNTORANGE".to_string(), "burntorange"),
            (format!("palette:{index}"), "burntorange"),
            (format!("palette:{custom}"), "custom"),
            ("atomic".to_string(), "element"),
            ("by_ss".to_string(), "ss"),
            ("0x12ABEF".to_string(), "#12abef"),
            ("inherit".to_string(), "inherit"),
            ("palette:16777215".to_string(), "palette:16777215"),
            ("scheme:16777215".to_string(), "scheme:16777215"),
        ] {
            for (command, message) in [
                (
                    format!("color {input}, obj"),
                    format!("Color: 1 atoms colored {expected}"),
                ),
                (
                    format!("set surface_color, {input}, obj"),
                    format!("surface_color = {expected} (selection obj,"),
                ),
                (
                    "get surface_color, obj".to_string(),
                    format!("surface_color = {expected} (selection obj)"),
                ),
                (
                    format!("set surface_color, {input}"),
                    format!("surface_color = {expected}"),
                ),
                (
                    "get surface_color".to_string(),
                    format!("surface_color (color) = {expected}"),
                ),
            ] {
                let mut needs_redraw = false;
                let mut adapter = SessionAdapter {
                    session: &mut session,
                    render_context: None,
                    default_size: (800, 600),
                    needs_redraw: &mut needs_redraw,
                };
                let output = executor
                    .do_with_options(&mut adapter, &command, false)
                    .unwrap();
                assert!(
                    output
                        .messages
                        .iter()
                        .any(|m| m.text.trim().starts_with(&message)),
                    "{command}: {:?}",
                    output.messages
                );
            }
        }
    }

    #[test]
    fn object_and_equivalent_selection_assign_identical_colors() {
        for name in [
            "cartoon_color",
            "ribbon_color",
            "stick_color",
            "sphere_color",
            "line_color",
            "surface_color",
            "mesh_color",
            "dot_color",
            "ellipsoid_color",
        ] {
            let mut by_object = session_with_case_collisions(("A", "B"), ("CA", "CA"));
            let mut by_selection = session_with_case_collisions(("A", "B"), ("CA", "CA"));
            for (session, target) in [(&mut by_object, "obj"), (&mut by_selection, "model obj")] {
                let mut executor = CommandExecutor::new();
                execute(
                    session,
                    &mut executor,
                    &format!("set {name}, atomic, chain A"),
                )
                .unwrap();
                execute(
                    session,
                    &mut executor,
                    &format!("set {name}, green, {target}"),
                )
                .unwrap();
                execute(session, &mut executor, "color blue, all").unwrap();
                let field = patinae_mol::AtomColors::representation_field(name).unwrap();
                let green =
                    ColorValue::palette(session.named_palette.get_by_name("green").unwrap().0);
                for atom in session
                    .registry
                    .get_molecule("obj")
                    .unwrap()
                    .molecule()
                    .atoms()
                {
                    assert_eq!(*field(&mut atom.repr.colors.clone()), green);
                }
                execute(session, &mut executor, &format!("unset {name}, {target}")).unwrap();
            }
            let colors = |session: &Session| {
                session
                    .registry
                    .get_molecule("obj")
                    .unwrap()
                    .molecule()
                    .atoms()
                    .map(|a| a.repr.colors.clone())
                    .collect::<Vec<_>>()
            };
            assert_eq!(colors(&by_object), colors(&by_selection));
        }
    }

    #[test]
    fn canonical_color_text_roundtrips_through_set() {
        let mut session = session_with_single_atom_object("obj");
        let mut executor = CommandExecutor::new();
        for value in [
            ColorValue::rgb(0, 0, 2),
            ColorValue::palette(2),
            ColorValue::SECONDARY_STRUCTURE,
            ColorValue::INHERIT,
            ColorValue::scheme(0xffffff),
            ColorValue::from_bits(0xffffffff),
        ] {
            execute(
                &mut session,
                &mut executor,
                &format!("set stick_color, {value}"),
            )
            .unwrap();
            assert_eq!(session.settings.stick.color, value);
        }
        assert!(execute(&mut session, &mut executor, "set stick_color, -9").is_err());
        assert!(execute(
            &mut session,
            &mut executor,
            "set stick_color, palette:16777216"
        )
        .is_err());
    }

    fn recent_path(session: &Session, object_name: &str, atom_index: u32) -> String {
        let molecule = session.registry.get_molecule(object_name).unwrap();
        patinae_scene::canonical_atom_path_for_hit(
            &patinae_scene::PickHit {
                object_name: object_name.to_string(),
                object_type: patinae_scene::ObjectType::Molecule,
                atom_index: Some(AtomIndex(atom_index)),

                instance: None,
                position: lin_alg::f32::Vec3::new(0.0, 0.0, 0.0),
                distance: 0.0,
            },
            molecule.molecule(),
        )
        .unwrap()
    }

    fn session_with_case_collisions(chains: (&str, &str), names: (&str, &str)) -> Session {
        let mut molecule = ObjectMolecule::new("obj");
        for (chain, name) in [(chains.0, names.0), (chains.1, names.1)] {
            molecule.add_atom(
                AtomBuilder::new()
                    .name(name)
                    .element_symbol("C")
                    .resn("GLY")
                    .resv(1)
                    .chain(chain)
                    .build(),
            );
        }
        let mut session = Session::new();
        session
            .registry
            .add(MoleculeObject::with_name(molecule, "obj"));
        session
    }

    #[test]
    fn test_cartoon_smooth_loops_set_on_value_path() {
        let desc = registry::lookup_by_name("cartoon_smooth_loops").unwrap();
        let setting = ResolvedSetting::BuiltIn(desc);
        let value = setting.parse_value("on").unwrap();
        let mut settings = patinae_settings::Settings::default();

        desc.set(&mut settings, value).unwrap();

        assert!(settings.cartoon.smooth_loops);
    }

    #[test]
    fn representation_transparency_set_and_unset_preserve_geometry() {
        let mut session = session_with_single_atom_object("obj");
        let mut executor = CommandExecutor::new();

        for (setting, expected) in [
            ("sphere_transparency", DirtyFlags::COLOR),
            ("stick_transparency", DirtyFlags::COLOR),
            ("ellipsoid_transparency", DirtyFlags::COLOR),
            ("cartoon_transparency", DirtyFlags::TRANSPARENCY),
            ("surface_transparency", DirtyFlags::TRANSPARENCY),
            ("mesh_transparency", DirtyFlags::TRANSPARENCY),
        ] {
            for command in [
                format!("set {setting}, 0.5"),
                format!("unset {setting}"),
                format!("set {setting}, 0.5, obj"),
                format!("unset {setting}, obj"),
            ] {
                session.registry.clear_all_dirty_objects();
                execute(&mut session, &mut executor, &command).unwrap();
                let dirty = session.registry.get_molecule("obj").unwrap().dirty_flags();
                assert!(dirty.contains(expected), "{command}: {dirty:?}");
                assert!(dirty.is_lut_only(), "{command}: {dirty:?}");
            }
        }
    }

    #[test]
    fn test_object_side_effects_use_descriptor_dirty_contract() {
        let mol = patinae_mol::MoleculeBuilder::new("obj").build();
        let mut obj = MoleculeObject::with_name(mol, "obj");
        obj.clear_dirty();

        let desc = registry::lookup_by_name("cartoon_smooth_loops").unwrap();
        apply_object_side_effects(
            &mut obj,
            desc.side_effects,
            desc.name,
            &SettingValue::Bool(true),
        );
        assert!(obj.dirty_flags().contains(DirtyFlags::REPS));
        assert!(!obj.dirty_flags().contains(DirtyFlags::COLOR));

        obj.clear_dirty();
        let desc = registry::lookup_by_name("mesh_transparency").unwrap();
        apply_object_side_effects(
            &mut obj,
            desc.side_effects,
            desc.name,
            &SettingValue::Float(0.5),
        );
        assert!(obj.dirty_flags().contains(DirtyFlags::TRANSPARENCY));
    }

    #[test]
    fn set_object_setting_uses_object_override_without_atom_write() {
        let mut session = session_with_single_atom_object("obj");
        let mut executor = CommandExecutor::new();

        execute(&mut session, &mut executor, "set sphere_scale, 0.5, obj").unwrap();

        let obj = session.registry.get_molecule("obj").unwrap();
        assert_eq!(session.settings.sphere.scale, 1.0);
        assert_eq!(obj.overrides().unwrap().sphere.scale, Some(0.5));
        assert_eq!(obj.molecule().atoms_slice()[0].repr.sphere_scale, None);
    }

    #[test]
    fn representation_colors_accept_named_selections_and_expressions() {
        for (setting, read) in [
            (
                "cartoon_color",
                (|c: &patinae_mol::AtomColors| c.cartoon)
                    as fn(&patinae_mol::AtomColors) -> ColorValue,
            ),
            ("ribbon_color", |c| c.ribbon),
            ("stick_color", |c| c.stick),
            ("line_color", |c| c.line),
            ("surface_color", |c| c.surface),
            ("mesh_color", |c| c.mesh),
            ("sphere_color", |c| c.sphere),
            ("dot_color", |c| c.dot),
            ("ellipsoid_color", |c| c.ellipsoid),
        ] {
            let mut session = session_with_case_collisions(("A", "B"), ("CA", "CA"));
            let mut executor = CommandExecutor::new();
            execute(&mut session, &mut executor, "color red, all").unwrap();
            execute(&mut session, &mut executor, "select sele, chain A").unwrap();
            execute(
                &mut session,
                &mut executor,
                &format!("set {setting}, blue, obj"),
            )
            .unwrap();
            session.registry.clear_all_dirty_objects();

            execute(
                &mut session,
                &mut executor,
                &format!("set {setting}, atomic, sele"),
            )
            .unwrap();
            let obj = session.registry.get_molecule("obj").unwrap();
            let atoms = obj.molecule().atoms_slice();
            assert_eq!(
                read(&atoms[0].repr.colors),
                patinae_color::ColorValue::ELEMENT
            );
            assert_eq!(
                read(&atoms[1].repr.colors),
                ColorValue::palette(session.named_palette.get_by_name("blue").unwrap().0)
            );
            assert_ne!(atoms[0].repr.colors.base, ColorValue::ELEMENT);
            assert!(obj.dirty_flags().is_lut_only());

            execute(
                &mut session,
                &mut executor,
                &format!("set {setting}, chain, sele"),
            )
            .unwrap();
            execute(&mut session, &mut executor, "color green, all").unwrap();
            let colors = &session
                .registry
                .get_molecule("obj")
                .unwrap()
                .molecule()
                .atoms_slice()[0]
                .repr
                .colors;
            assert_ne!(read(colors), ColorValue::INHERIT);
            assert_eq!(read(colors), patinae_color::ColorValue::CHAIN);

            execute(
                &mut session,
                &mut executor,
                &format!("unset {setting}, chain A"),
            )
            .unwrap();
            let obj = session.registry.get_molecule("obj").unwrap();
            assert_eq!(
                read(&obj.molecule().atoms_slice()[0].repr.colors),
                ColorValue::INHERIT
            );
            let desc = registry::lookup_by_name(setting).unwrap();
            assert_eq!(
                desc.get(&session.settings),
                SettingValue::Color(ColorValue::INHERIT)
            );

            // Recoloring the whole object replaces earlier selection colors,
            // even after the named selection has been deleted.
            let other_setting = if setting == "sphere_color" {
                "stick_color"
            } else {
                "sphere_color"
            };
            execute(
                &mut session,
                &mut executor,
                &format!("set {other_setting}, red, sele"),
            )
            .unwrap();
            execute(
                &mut session,
                &mut executor,
                &format!("set {setting}, atomic, sele"),
            )
            .unwrap();
            let before = session
                .registry
                .get_molecule("obj")
                .unwrap()
                .molecule()
                .atoms_slice()[0]
                .repr
                .colors
                .clone();
            execute(&mut session, &mut executor, "deselect sele").unwrap();
            session.registry.clear_all_dirty_objects();
            execute(
                &mut session,
                &mut executor,
                &format!("set {setting}, green, obj"),
            )
            .unwrap();
            let obj = session.registry.get_molecule("obj").unwrap();
            assert!(obj.dirty_flags().is_lut_only());
            let mut after = obj.molecule().atoms_slice()[0].repr.colors.clone();
            assert_eq!(
                read(&after),
                ColorValue::palette(session.named_palette.get_by_name("green").unwrap().0),
                "{setting}"
            );
            // Only the requested representation changes; base and other reps survive.
            *patinae_mol::AtomColors::representation_field(setting).unwrap()(&mut after) =
                read(&before);
            assert_eq!(after, before);

            // A later selection color still overrides the new object color.
            execute(
                &mut session,
                &mut executor,
                &format!("set {setting}, atomic, chain B"),
            )
            .unwrap();
            let obj = session.registry.get_molecule("obj").unwrap();
            assert_eq!(
                read(&obj.molecule().atoms_slice()[0].repr.colors),
                ColorValue::palette(session.named_palette.get_by_name("green").unwrap().0)
            );
            assert_eq!(
                read(&obj.molecule().atoms_slice()[1].repr.colors),
                patinae_color::ColorValue::ELEMENT
            );
        }
    }

    #[test]
    fn selection_color_scope_and_inheritance_do_not_leak() {
        let mut session = session_with_case_collisions(("A", "B"), ("CA", "CA"));
        let other = session
            .registry
            .get_molecule("obj")
            .unwrap()
            .molecule()
            .clone();
        session
            .registry
            .add(MoleculeObject::with_name(other, "other"));
        let mut executor = CommandExecutor::new();
        execute(&mut session, &mut executor, "select sele, chain A").unwrap();
        execute(&mut session, &mut executor, "set stick_color, blue").unwrap();
        execute(&mut session, &mut executor, "set stick_color, green, obj").unwrap();
        execute(&mut session, &mut executor, "set stick_color, red, sele").unwrap();
        let red = ColorValue::palette(session.named_palette.get_by_name("red").unwrap().0);
        for name in ["obj", "other"] {
            let atoms = session
                .registry
                .get_molecule(name)
                .unwrap()
                .molecule()
                .atoms_slice();
            assert_eq!(atoms[0].repr.colors.stick, red);
            assert_eq!(
                atoms[1].repr.colors.stick,
                if name == "obj" {
                    ColorValue::palette(session.named_palette.get_by_name("green").unwrap().0)
                } else {
                    ColorValue::INHERIT
                }
            );
            assert_eq!(atoms[0].repr.colors.sphere, ColorValue::INHERIT);
        }
        execute(
            &mut session,
            &mut executor,
            "set stick_color, inherit, model obj and sele",
        )
        .unwrap();
        assert_eq!(
            session
                .registry
                .get_molecule("obj")
                .unwrap()
                .molecule()
                .atoms_slice()[0]
                .repr
                .colors
                .stick,
            ColorValue::INHERIT
        );
        assert_eq!(
            session
                .registry
                .get_molecule("other")
                .unwrap()
                .molecule()
                .atoms_slice()[0]
                .repr
                .colors
                .stick,
            red
        );
        session.registry.clear_all_dirty_objects();
        execute(&mut session, &mut executor, "set stick_color, red, chain Z").unwrap();
        assert!(execute(
            &mut session,
            &mut executor,
            "set stick_color, red, (chain A"
        )
        .is_err());
        for name in ["obj", "other"] {
            assert!(session
                .registry
                .get_molecule(name)
                .unwrap()
                .dirty_flags()
                .is_empty());
        }
        assert_eq!(
            session.settings.stick.color,
            ColorValue::palette(session.named_palette.get_by_name("blue").unwrap().0)
        );
        let obj = session.registry.get_molecule("obj").unwrap();
        assert_eq!(
            obj.molecule().atoms_slice()[1].repr.colors.stick,
            ColorValue::palette(session.named_palette.get_by_name("green").unwrap().0)
        );
    }

    #[test]
    fn atomic_representation_setting_is_distinct_from_inheritance() {
        let mut session = session_with_single_atom_object("obj");
        let mut executor = CommandExecutor::new();
        execute(&mut session, &mut executor, "color red, obj").unwrap();
        for command in ["set stick_color, atomic", "set stick_color, atomic, obj"] {
            execute(&mut session, &mut executor, command).unwrap();
        }
        let obj = session.registry.get_molecule("obj").unwrap();
        let global = session.settings.stick.color;
        let object = obj.molecule().atoms_slice()[0].repr.colors.stick;
        assert_ne!(global, ColorValue::INHERIT);
        assert_eq!(global, object);
        assert_eq!(global, patinae_color::ColorValue::ELEMENT);
        assert_eq!(
            obj.molecule().atoms_slice()[0].repr.colors.stick,
            ColorValue::ELEMENT
        );
    }

    #[test]
    fn rgb_literals_do_not_alias_palette_indices_or_register_colors() {
        let mut session = session_with_single_atom_object("obj");
        let mut executor = CommandExecutor::new();
        let palette_before = serde_json::to_value(&session.named_palette).unwrap();
        execute(&mut session, &mut executor, "color 0x000002, obj").unwrap();
        execute(
            &mut session,
            &mut executor,
            "set stick_color, 0x000002, obj",
        )
        .unwrap();
        let obj = session.registry.get_molecule("obj").unwrap();
        assert_eq!(
            obj.molecule().atoms_slice()[0].repr.colors.base,
            ColorValue::rgb(0, 0, 2)
        );
        assert_eq!(
            obj.molecule().atoms_slice()[0].repr.colors.stick,
            ColorValue::rgb(0, 0, 2)
        );
        assert_eq!(
            serde_json::to_value(&session.named_palette).unwrap(),
            palette_before
        );
        execute(&mut session, &mut executor, "set stick_color, 2, obj").unwrap();
        assert_eq!(
            session
                .registry
                .get_molecule("obj")
                .unwrap()
                .molecule()
                .atoms_slice()[0]
                .repr
                .colors
                .stick,
            ColorValue::palette(2)
        );
    }

    #[test]
    fn global_color_setting_does_not_write_atom_color_override() {
        let mut session = session_with_single_atom_object("obj");
        let mut executor = CommandExecutor::new();

        execute(&mut session, &mut executor, "set sphere_color, red").unwrap();

        let obj = session.registry.get_molecule("obj").unwrap();
        assert_ne!(session.settings.sphere.color, ColorValue::INHERIT);
        assert_eq!(
            obj.molecule().atoms_slice()[0].repr.colors.sphere,
            ColorValue::INHERIT
        );
    }

    #[test]
    fn object_setting_rejects_non_object_selection_without_global_fallback() {
        let mut session = session_with_single_atom_object("obj");
        let mut executor = CommandExecutor::new();

        let err = execute(
            &mut session,
            &mut executor,
            "set sphere_scale, 0.5, chain A",
        )
        .unwrap_err();

        assert!(err.is_object_not_found());
        assert_eq!(session.settings.sphere.scale, 1.0);
    }

    #[test]
    fn unset_global_color_setting_preserves_atom_color_override() {
        let mut session = session_with_single_atom_object("obj");
        session
            .registry
            .get_molecule_mut("obj")
            .unwrap()
            .molecule_mut()
            .atoms_slice_mut()[0]
            .repr
            .colors
            .sphere = ColorValue::palette(7);
        let mut executor = CommandExecutor::new();

        execute(&mut session, &mut executor, "set sphere_color, red").unwrap();
        execute(&mut session, &mut executor, "unset sphere_color").unwrap();

        let obj = session.registry.get_molecule("obj").unwrap();
        assert_eq!(
            session.settings.sphere.color,
            patinae_color::ColorValue::INHERIT
        );
        assert_eq!(
            obj.molecule().atoms_slice()[0].repr.colors.sphere,
            ColorValue::palette(7)
        );
    }

    #[test]
    fn unset_object_setting_clears_object_override_without_atom_write() {
        let mut session = session_with_single_atom_object("obj");
        let mut executor = CommandExecutor::new();

        execute(&mut session, &mut executor, "set sphere_scale, 0.5, obj").unwrap();
        execute(&mut session, &mut executor, "unset sphere_scale, obj").unwrap();

        let obj = session.registry.get_molecule("obj").unwrap();
        assert_eq!(obj.overrides().unwrap().sphere.scale, None);
        assert_eq!(obj.molecule().atoms_slice()[0].repr.sphere_scale, None);
    }

    #[test]
    fn state_uses_regular_setting_path() {
        let mut session = Session::new();
        let mut executor = CommandExecutor::new();

        execute(&mut session, &mut executor, "set state, 2").unwrap();

        let desc = registry::lookup_by_name("state").unwrap();
        assert_eq!(desc.get(&session.settings), SettingValue::Int(2));
    }

    #[test]
    fn render_memory_profile_uses_regular_setting_path() {
        let mut session = Session::new();
        let mut executor = CommandExecutor::new();

        execute(
            &mut session,
            &mut executor,
            "set render_memory_profile, performance",
        )
        .unwrap();
        assert_eq!(
            session.settings.renderer.memory_profile,
            patinae_settings::RenderMemoryProfileSetting::Performance
        );

        execute(
            &mut session,
            &mut executor,
            "set render_memory_profile, lite",
        )
        .unwrap();
        assert_eq!(
            session.settings.renderer.memory_profile,
            patinae_settings::RenderMemoryProfileSetting::Lite
        );
    }

    #[test]
    fn render_memory_profile_rejects_legacy_profile_name() {
        let mut session = Session::new();
        let mut executor = CommandExecutor::new();
        let legacy = ["lo", "w"].concat();
        let command = format!("set render_memory_profile, {legacy}");

        let err = execute(&mut session, &mut executor, &command).unwrap_err();

        assert!(err
            .to_string()
            .contains(&format!("Unknown value '{legacy}'")));

        let legacy = ["bud", "geted"].concat();
        let command = format!("set render_memory_profile, {legacy}");
        let err = execute(&mut session, &mut executor, &command).unwrap_err();

        assert!(err
            .to_string()
            .contains(&format!("Unknown value '{legacy}'")));
    }

    #[test]
    fn render_memory_manual_profile_uses_regular_setting_path() {
        let mut session = Session::new();
        let mut executor = CommandExecutor::new();

        execute(
            &mut session,
            &mut executor,
            "set render_memory_profile, manual",
        )
        .unwrap();
        execute(
            &mut session,
            &mut executor,
            "set render_memory_budget, 1024",
        )
        .unwrap();

        assert_eq!(
            session.settings.renderer.memory_profile,
            patinae_settings::RenderMemoryProfileSetting::Manual
        );
        assert_eq!(session.settings.renderer.memory_budget_mib, 1024);
    }

    #[test]
    fn render_memory_budget_uses_regular_setting_path() {
        let mut session = Session::new();
        let mut executor = CommandExecutor::new();

        execute(
            &mut session,
            &mut executor,
            "set render_memory_budget, 1024",
        )
        .unwrap();

        assert_eq!(session.settings.renderer.memory_budget_mib, 1024);
    }

    #[test]
    fn render_memory_budget_rejects_negative_values() {
        let mut session = Session::new();
        let mut executor = CommandExecutor::new();

        let err = execute(&mut session, &mut executor, "set render_memory_budget, -1").unwrap_err();

        assert!(err.to_string().contains("out of range"));
    }

    #[test]
    fn render_memory_unset_is_independent_regular_setting_reset() {
        let mut session = Session::new();
        let mut executor = CommandExecutor::new();

        execute(
            &mut session,
            &mut executor,
            "set render_memory_profile, balanced",
        )
        .unwrap();
        execute(
            &mut session,
            &mut executor,
            "set render_memory_budget, 1024",
        )
        .unwrap();
        execute(&mut session, &mut executor, "unset render_memory_profile").unwrap();

        assert_eq!(
            session.settings.renderer.memory_profile,
            patinae_settings::RenderMemoryProfileSetting::Auto
        );
        assert_eq!(session.settings.renderer.memory_budget_mib, 1024);

        execute(&mut session, &mut executor, "unset render_memory_budget").unwrap();

        assert_eq!(
            session.settings.renderer.memory_profile,
            patinae_settings::RenderMemoryProfileSetting::Auto
        );
        assert_eq!(session.settings.renderer.memory_budget_mib, 0);
    }

    #[test]
    fn max_recent_picks_changes_apply_immediately_and_reject_object_override() {
        let mut session = Session::new();
        for name in ["one", "two", "three"] {
            let object = session_with_single_atom_object(name)
                .registry
                .remove(name)
                .unwrap();
            session.registry.insert_boxed(name, object);
        }
        let paths = ["one", "two", "three"].map(|name| recent_path(&session, name, 0));
        for path in &paths {
            session
                .recent_atoms
                .insert(path.clone(), RecentPickLimit::Unlimited);
        }
        let mut executor = CommandExecutor::new();

        execute(&mut session, &mut executor, "set max_recent_picks, 2").unwrap();
        assert_eq!(
            session.recent_atoms.paths().collect::<Vec<_>>(),
            [&paths[1], &paths[2]]
        );
        execute(&mut session, &mut executor, "set max_recent_picks, 0").unwrap();
        assert!(session.recent_atoms.is_empty());
        execute(&mut session, &mut executor, "set max_recent_picks, 5").unwrap();
        assert!(session.recent_atoms.is_empty());

        session
            .recent_atoms
            .insert(paths[0].clone(), RecentPickLimit::Unlimited);
        execute(&mut session, &mut executor, "unset max_recent_picks").unwrap();
        assert_eq!(
            session.recent_atoms.paths().collect::<Vec<_>>(),
            [&paths[0]]
        );
        assert_eq!(session.settings.behavior.max_recent_picks, -1);

        let error =
            execute(&mut session, &mut executor, "set max_recent_picks, 1, one").unwrap_err();
        assert!(error
            .to_string()
            .contains("does not support object overrides"));
        assert_eq!(session.settings.behavior.max_recent_picks, -1);

        let error =
            execute(&mut session, &mut executor, "unset max_recent_picks, one").unwrap_err();
        assert!(error
            .to_string()
            .contains("does not support object overrides"));
        assert_eq!(session.settings.behavior.max_recent_picks, -1);
    }

    #[test]
    fn selection_case_settings_reconcile_paths_that_become_ambiguous() {
        let mut executor = CommandExecutor::new();
        let mut name_session = session_with_case_collisions(("A", "A"), ("CA", "ca"));
        execute(&mut name_session, &mut executor, "set ignore_case, off").unwrap();
        let name_path = recent_path(&name_session, "obj", 0);
        name_session
            .recent_atoms
            .insert(name_path, RecentPickLimit::Unlimited);

        execute(&mut name_session, &mut executor, "set ignore_case, on").unwrap();
        assert!(name_session.recent_atoms.is_empty());

        let mut chain_session = session_with_case_collisions(("A", "a"), ("CA", "CA"));
        let chain_path = recent_path(&chain_session, "obj", 0);
        chain_session
            .recent_atoms
            .insert(chain_path, RecentPickLimit::Unlimited);

        execute(
            &mut chain_session,
            &mut executor,
            "set ignore_case_chain, on",
        )
        .unwrap();
        assert!(chain_session.recent_atoms.is_empty());
    }
}
