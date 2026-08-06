//! Settings commands: set, get, unset, dss

use crate::args::ParsedCommand;
use crate::command::{ArgHint, Command, CommandContext, CommandRegistry, ViewerLike};
use crate::command_help;
use crate::error::{CmdError, CmdResult};
use crate::ResolvedSetting;
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
) -> String {
    let raw = ResolvedSetting::format_value(value);
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

impl SetCommand {
    /// Parse a value string with per-setting overrides.
    fn parse_value<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        setting: &ResolvedSetting,
        value_str: &str,
    ) -> CmdResult<SettingValue> {
        // Built-in color settings resolve names and color schemes through the viewer.
        if setting.built_in_descriptor().is_some() && setting.setting_type() == SettingType::Color {
            // Accept [r, g, b] float vector and convert it to 0x00RRGGBB.
            if value_str.starts_with('[') && value_str.ends_with(']') {
                let inner = value_str[1..value_str.len() - 1].trim();
                let parts: Vec<&str> = inner.split(',').map(|p| p.trim()).collect();
                if parts.len() == 3 {
                    if let (Ok(r), Ok(g), Ok(b)) = (
                        parts[0].parse::<f32>(),
                        parts[1].parse::<f32>(),
                        parts[2].parse::<f32>(),
                    ) {
                        let packed = patinae_color::Color::new(
                            r.clamp(0.0, 1.0),
                            g.clamp(0.0, 1.0),
                            b.clamp(0.0, 1.0),
                        )
                        .to_packed_rgb();
                        return Ok(SettingValue::Int(packed));
                    }
                }
            }

            if let Ok(v) = value_str.parse::<i32>() {
                return Ok(SettingValue::Int(v));
            }
            let color_index =
                if let Some(ci) = patinae_color::ColorIndex::from_scheme_name(value_str) {
                    i32::from(ci)
                } else if let Some(idx) = ctx.viewer.color_index(value_str) {
                    idx as i32
                } else if let Some(color) = patinae_color::Color::from_hex(value_str) {
                    ctx.viewer.named_palette_mut().set(value_str, color) as i32
                } else {
                    return Err(CmdError::invalid_arg(
                        "value",
                        format!("Unknown color: {}", value_str),
                    ));
                };
            return Ok(SettingValue::Int(color_index));
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
                            let display = setting.format_display(&value);
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
                    let display = setting.format_display(&value);
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
            let display = setting.format_display(&value);
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
        ]
        USAGE [
            "set name [, value [, object [, state ]]]",
        ]
        REQUIRED [
            { "name", "string", "setting name" },
        ]
        OPTIONAL [
            { "value", "string", "new value (depends on setting type)", "" },
            { "object", "string", "apply object-overridable settings to an object", "global" },
            { "state", "integer", "state for state-specific settings", "0" },
        ]
        EXAMPLES [
            "set sphere_scale, 0.5",
            "set cartoon_color, red, obj",
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

        if let Some(desc) = setting.built_in_descriptor() {
            if let Some(sel) = selection {
                if desc.is_object_overridable() {
                    if let Some(mol) = ctx.viewer.objects().get_molecule(sel) {
                        if let Some(value) = setting.built_in_object_value(mol) {
                            ctx.print(&format!(" {}", format_object_report(&setting, &value, sel)));
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
                    ctx.print(&format!(" {}", format_object_report(&setting, &value, sel)));
                    return Ok(());
                }
            }
        }

        let value = setting
            .global_value(ctx.viewer)
            .map_err(CmdError::execution)?;
        ctx.print(&format!(" {}", setting.format_report(&value)));
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
            "unset name [, object [, state ]]",
        ]
        REQUIRED [
            { "name", "string", "setting name" },
        ]
        OPTIONAL [
            { "object", "string", "reset object override when present", "global" },
            { "state", "integer", "state for state-specific settings", "0" },
        ]
        EXAMPLES [
            "unset sphere_scale",
            "unset ray_trace_mode",
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
                setting.format_display(&default_value)
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
    use patinae_mol::{AtomBuilder, AtomIndex, ObjectMolecule, COLOR_UNSET};
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
            async_fetch_fn: None,
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
    fn global_color_setting_does_not_write_atom_color_override() {
        let mut session = session_with_single_atom_object("obj");
        let mut executor = CommandExecutor::new();

        execute(&mut session, &mut executor, "set sphere_color, red").unwrap();

        let obj = session.registry.get_molecule("obj").unwrap();
        assert_ne!(session.settings.sphere.color.0, COLOR_UNSET);
        assert_eq!(
            obj.molecule().atoms_slice()[0].repr.colors.sphere,
            COLOR_UNSET
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
            .sphere = 7;
        let mut executor = CommandExecutor::new();

        execute(&mut session, &mut executor, "set sphere_color, red").unwrap();
        execute(&mut session, &mut executor, "unset sphere_color").unwrap();

        let obj = session.registry.get_molecule("obj").unwrap();
        assert_eq!(
            session.settings.sphere.color.0,
            patinae_settings::Color::UNSET.0
        );
        assert_eq!(obj.molecule().atoms_slice()[0].repr.colors.sphere, 7);
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
