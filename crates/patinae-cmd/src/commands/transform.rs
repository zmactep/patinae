//! Structure transformation commands: translate, rotate, transform_selection
//!
//! These commands move atoms or whole instanced objects in world space.
//! Whole instanced objects retain source coordinates and copy matrices;
//! edits to subsets require materialization.
//!
//! Full selection expression support:
//! - Property selectors: `name CA`, `resn ALA`, `chain A`, `elem C`
//! - Numeric comparisons: `b > 50`, `resi 1-100`
//! - Logical operators: `name CA and chain A`, `organic or solvent`
//! - Special keywords: `backbone`, `sidechain`, `polymer`, `organic`

use lin_alg::f32::{Mat4, Vec3};
use patinae_mol::{
    rotation_ttt, translation_matrix, ttt_to_mat4, AtomIndex, InstanceGroup, InstanceTable,
    ObjectInstance,
};
use patinae_scene::{DirtyFlags, Object};
use patinae_select::SelectionResult;

use crate::args::ParsedCommand;
use crate::command::{ArgHint, Command, CommandContext, CommandRegistry, ViewerLike};
use crate::command_help;
use crate::commands::selecting::{evaluate_atom_anchors, evaluate_selection};
use crate::error::{CmdError, CmdResult};
use crate::helpers::{
    camera_to_model_vec, resolve_object_names, state_index_from_user, ResolvedNames,
};

/// Register transformation commands
pub fn register(registry: &mut CommandRegistry) {
    registry.register(TranslateCommand);
    registry.register(RotateCommand);
    registry.register(TransformSelectionCommand);
}

// ============================================================================
// translate command
// ============================================================================

struct TranslateCommand;

impl Command for TranslateCommand {
    fn name(&self) -> &str {
        "translate"
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::None, ArgHint::Selection]
    }

    command_help! {
        CMD "translate"
        DESCRIPTION [
            "translates the atomic coordinates of atoms in a selection.",
            "Supports full selection expressions.",
            "Whole instanced objects move through their object matrix; subsets require materialize.",
        ]
        REQUIRED [
            { "vector", "float vector", "translation vector [x, y, z]" },
        ]
        OPTIONAL [
            { "selection", "string", "atoms whose coordinates should be modified", "all" } => [
                "Supports full selection expressions:",
                "- Property selectors: name CA, resn ALA, chain A, elem C",
                "- Numeric ranges: resi 1-100, b > 50",
                "- Logical operators: name CA and chain A, organic or solvent",
                "- Keywords: backbone, sidechain, polymer, organic, solvent",
            ],
            { "state", "integer", "state to modify", "-1" } => [
                "state > 0: only the indicated state is modified",
                "state = 0: all states are modified",
                "state = -1: only the current state is modified",
            ],
            { "camera", "0/1", "is the vector in camera coordinates?", "1" },
            { "center", "0/1", "place one whole object's bounding-box center at vector (requires camera=0)", "0" },
        ]
        EXAMPLES [
            "translate [1, 0, 0], name CA",
            "translate [0, 5, 0], chain A",
            "translate [0, 0, 10], backbone and chain A",
            "translate [1, 1, 1], resi 50-100",
            "translate [0, 0, 5], organic",
            "translate [650, 0, 0], capsid, camera=0, center=1",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        // Parse vector argument (required)
        let vector = parse_vector(args, 0, "vector")?;

        let selection = args.str_arg_or(1, "selection", "all");
        let state = args.int_arg_or(2, "state", -1);
        let camera = args.int_arg_or(3, "camera", 1);

        // Convert vector from camera coordinates if needed
        let mut shift = if camera != 0 {
            // Transform vector from camera coordinates to model coordinates
            let rotation = &ctx.viewer.camera().current_view().rotation;
            camera_to_model_vec(rotation, vector)
        } else {
            vector
        };

        if args.int_arg_or(4, "center", 0) != 0 {
            if camera != 0 {
                return Err(CmdError::invalid_arg(
                    "camera",
                    "center=1 requires camera=0",
                ));
            }
            let object = ctx
                .viewer
                .objects()
                .get_molecule(selection)
                .ok_or_else(|| {
                    CmdError::invalid_arg("selection", "center=1 requires one whole object name")
                })?;
            let (min, max) = object
                .extent()
                .ok_or_else(|| CmdError::execution("Object has no extent"))?;
            shift = vector - (min + max) * 0.5;
        }

        let (atom_count, obj_count) =
            apply_selection_transform(ctx, selection, state, &translation_matrix(shift))?;

        ctx.viewer.request_redraw();

        if !ctx.quiet {
            ctx.print(&format!(
                " Translated {} atom(s) in {} object(s) by [{:.3}, {:.3}, {:.3}]",
                atom_count, obj_count, shift.x, shift.y, shift.z
            ));
        }

        Ok(())
    }
}

// ============================================================================
// rotate command
// ============================================================================

struct RotateCommand;

impl Command for RotateCommand {
    fn name(&self) -> &str {
        "rotate"
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[
            ArgHint::Keywords(&["x", "y", "z"]),
            ArgHint::None,
            ArgHint::Selection,
        ]
    }

    command_help! {
        CMD "rotate"
        DESCRIPTION [
            "rotates the atomic coordinates of atoms in a selection about",
            "an axis. Supports full selection expressions.",
            "Whole instanced objects rotate without materialization; subsets require materialize.",
        ]
        REQUIRED [
            { "axis", "x/y/z or float vector", "axis about which to rotate" },
            { "angle", "float", "degrees of rotation" },
        ]
        OPTIONAL [
            { "selection", "string", "atoms whose coordinates should be modified", "all" } => [
                "Supports full selection expressions:",
                "- Property selectors: name CA, resn ALA, chain A, elem C",
                "- Numeric ranges: resi 1-100, b > 50",
                "- Logical operators: name CA and chain A",
                "- Keywords: backbone, sidechain, polymer, organic",
            ],
            { "state", "integer", "state to modify", "-1" } => [
                "state > 0: only the indicated state is modified",
                "state = 0: all states are modified",
                "state = -1: only the current state is modified",
            ],
            { "camera", "0/1", "is the axis in camera coordinates?", "1" },
            { "origin", "float vector", "center of rotation", "view origin" },
        ]
        EXAMPLES [
            "rotate x, 45, all",
            "rotate y, 90, chain A",
            "rotate [1, 1, 1], 10, backbone",
            "rotate z, 180, resi 100-200, origin=[0, 0, 0]",
            "rotate x, 45, organic",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        // Parse axis argument (required)
        let axis = parse_axis(args, 0)?;

        let angle_deg = args
            .float_arg(1, "angle")
            .ok_or_else(|| CmdError::missing_argument("angle".to_string()))?;
        let angle = (angle_deg as f32) * std::f32::consts::PI / 180.0;

        let selection = args.str_arg_or(2, "selection", "all");
        let state = args.int_arg_or(3, "state", -1);
        let camera = args.int_arg_or(4, "camera", 1);

        // Parse origin (default: view origin)
        let origin = if let Some(origin_vec) = args.get_named("origin").and_then(|v| v.as_vec3()) {
            origin_vec
        } else {
            // Use view origin as default
            ctx.viewer.camera().current_view().origin
        };

        // Transform axis from camera coordinates if needed
        let rot_axis = if camera != 0 {
            let rotation = &ctx.viewer.camera().current_view().rotation;
            camera_to_model_vec(rotation, axis)
        } else {
            axis
        };

        // Build the TTT matrix for rotation about origin
        let ttt = rotation_ttt(rot_axis, angle, origin);

        // Apply rotation using selection expressions
        let (atom_count, obj_count) =
            apply_selection_transform(ctx, selection, state, &ttt_to_mat4(&ttt))?;

        ctx.viewer.request_redraw();

        if !ctx.quiet {
            ctx.print(&format!(
                " Rotated {} atom(s) in {} object(s) by {:.1} degrees",
                atom_count, obj_count, angle_deg
            ));
        }

        Ok(())
    }
}

// ============================================================================
// transform_selection command
// ============================================================================

struct TransformSelectionCommand;

impl Command for TransformSelectionCommand {
    fn name(&self) -> &str {
        "transform_selection"
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Selection]
    }

    command_help! {
        CMD "transform_selection"
        DESCRIPTION [
            "applies a transformation matrix to the atomic",
            "coordinates of a selection. Supports full selection expressions.",
            "Whole instanced objects support rigid matrices without materialization.",
        ]
        USAGE [
            "transform_selection selection, matrix [, state [, homogenous ]]",
        ]
        REQUIRED [
            { "selection", "string", "atoms to transform" } => [
                "Supports full selection expressions:",
                "- Property selectors: name CA, resn ALA, chain A",
                "- Logical operators: name CA and chain A",
                "- Keywords: backbone, sidechain, polymer, organic",
            ],
            { "matrix", "list of 16 floats", "transformation matrix" },
        ]
        OPTIONAL [
            { "state", "integer", "state to modify", "-1" } => [
                "state > 0: only the indicated state is modified",
                "state = 0: all states are modified",
                "state = -1: only the current state is modified",
            ],
            { "homogenous", "0/1", "matrix format", "0" } => [
                "0 = TTT format (pre-translate, rotate, post-translate)",
                "1 = Standard homogenous 4x4 matrix",
            ],
        ]
        NOTES("NOTES") [
            "When homogenous=0, the matrix is in TTT format:",
            "- [0-2, 4-6, 8-10]: 3x3 rotation matrix",
            "- [3, 7, 11]: post-rotation translation",
            "- [12, 13, 14]: pre-rotation translation",
            "- [15]: 1.0",
        ]
        EXAMPLES [
            "# Translate all atoms by [1, 2, 3]",
            "transform_selection all, [1,0,0,1, 0,1,0,2, 0,0,1,3, 0,0,0,1], homogenous=1",
            "",
            "# Transform only chain A",
            "transform_selection chain A, [1,0,0,5, 0,1,0,0, 0,0,1,0, 0,0,0,1], homogenous=1",
            "",
            "# Transform backbone atoms",
            "transform_selection backbone, [1,0,0,0, 0,1,0,0, 0,0,1,10, 0,0,0,1], homogenous=1",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let selection = args.str_arg_or(0, "selection", "all");
        let matrix = parse_matrix(args, 1)?;
        let state = args.int_arg_or(2, "state", -1);
        let homogenous = args.int_arg_or(3, "homogenous", 0);

        // Apply transformation using selection expressions
        let mat4 = if homogenous != 0 {
            // The command accepts a row-major homogeneous matrix.
            Mat4::new(std::array::from_fn(|i| matrix[(i % 4) * 4 + i / 4]))
        } else {
            ttt_to_mat4(&matrix)
        };
        let (atom_count, obj_count) = apply_selection_transform(ctx, selection, state, &mat4)?;

        ctx.viewer.request_redraw();

        if !ctx.quiet {
            ctx.print(&format!(
                " Transformed {} atom(s) in {} object(s)",
                atom_count, obj_count
            ));
        }

        Ok(())
    }
}

// ============================================================================
// Helper functions - Argument parsing
// ============================================================================

/// Parse a vector argument (either [x,y,z] list or x,y,z positional)
fn parse_vector(args: &ParsedCommand, pos: usize, name: &str) -> Result<Vec3, CmdError> {
    args.vec3_arg(pos, name)
        .ok_or_else(|| CmdError::missing_argument(name.to_string()))
}

/// Parse axis argument (x, y, z, or [ax, ay, az])
fn parse_axis(args: &ParsedCommand, pos: usize) -> Result<Vec3, CmdError> {
    // Check for named axes first
    if let Some(s) = args.get_str(pos) {
        match s.to_lowercase().as_str() {
            "x" => return Ok(Vec3::new(1.0, 0.0, 0.0)),
            "y" => return Ok(Vec3::new(0.0, 1.0, 0.0)),
            "z" => return Ok(Vec3::new(0.0, 0.0, 1.0)),
            _ => {}
        }
    }

    // Try as vector
    parse_vector(args, pos, "axis")
}

/// Parse a 16-element matrix
fn parse_matrix(args: &ParsedCommand, pos: usize) -> Result<[f32; 16], CmdError> {
    let mut matrix = [0.0f32; 16];

    if let Some(arg) = args.get_arg(pos) {
        match arg {
            crate::args::ArgValue::List(items) if items.len() >= 16 => {
                for (i, item) in items.iter().take(16).enumerate() {
                    matrix[i] = item.as_float().unwrap_or(0.0) as f32;
                }
                return Ok(matrix);
            }
            crate::args::ArgValue::String(s) => {
                let s = s.trim().trim_matches(|c| c == '[' || c == ']' || c == '(');
                let parts: Vec<f32> = s
                    .split(',')
                    .filter_map(|p| p.trim().parse::<f32>().ok())
                    .collect();
                if parts.len() >= 16 {
                    for (i, &v) in parts.iter().take(16).enumerate() {
                        matrix[i] = v;
                    }
                    return Ok(matrix);
                }
            }
            _ => {}
        }
    }

    // Try named argument
    if let Some(crate::args::ArgValue::List(items)) = args.get_named("matrix") {
        if items.len() >= 16 {
            for (i, item) in items.iter().take(16).enumerate() {
                matrix[i] = item.as_float().unwrap_or(0.0) as f32;
            }
            return Ok(matrix);
        }
    }

    Err(CmdError::missing_argument("matrix (16 floats)".to_string()))
}

// ============================================================================
// Helper functions - Selection-based transformations
// ============================================================================

/// Apply a transformation to atoms matching a selection expression.
///
/// Uses `evaluate_selection()` which supports full selection expressions,
/// named selections, object names, and wildcard patterns (e.g. `1fsd_*`).
///
/// Returns (total_atoms_transformed, num_objects_affected)
fn apply_selection_transform(
    ctx: &mut CommandContext<'_, '_, dyn ViewerLike + '_>,
    selection: &str,
    state: i64,
    transform: &Mat4,
) -> CmdResult<(usize, usize)> {
    // Whole-object movie edits must be available to mview interpolation.
    let movie_object = !ctx.viewer.movie().is_empty() && ctx.viewer.objects().contains(selection);
    let whole_names = resolve_object_names(ctx.viewer.objects(), selection);
    let selection_results = match &whole_names {
        ResolvedNames::All => ctx
            .viewer
            .objects()
            .names()
            .filter_map(|name| {
                ctx.viewer.objects().get_molecule(name).map(|object| {
                    (
                        name.to_string(),
                        SelectionResult::all(object.molecule().atom_count()),
                    )
                })
            })
            .collect(),
        ResolvedNames::Matched(names)
            if names
                .iter()
                .all(|name| ctx.viewer.objects().get_molecule(name).is_some()) =>
        {
            names
                .iter()
                .map(|name| {
                    let object = ctx
                        .viewer
                        .objects()
                        .get_molecule(name)
                        .expect("matched molecule");
                    (
                        name.clone(),
                        SelectionResult::all(object.molecule().atom_count()),
                    )
                })
                .collect()
        }
        _ => evaluate_selection(ctx.viewer, selection)?,
    };
    let has_instances = selection_results.iter().any(|(name, mask)| {
        mask.count() > 0
            && ctx
                .viewer
                .objects()
                .get_molecule(name)
                .is_some_and(|object| object.state().instances.is_some())
    });
    // A source mask alone cannot distinguish all copies from just one copy.
    // Object names and globs avoid allocating one anchor per displayed atom.
    let anchors = if has_instances && matches!(whole_names, ResolvedNames::Unresolved) {
        evaluate_atom_anchors(ctx.viewer, selection)?
    } else {
        Vec::new()
    };
    if has_instances {
        InstanceTable {
            groups: vec![InstanceGroup {
                indices: Vec::new(),
            }],
            copies: vec![ObjectInstance {
                group: 0,
                transform: std::array::from_fn(|c| {
                    std::array::from_fn(|r| transform.data[c * 4 + r])
                }),
            }],
        }
        .validate(1)
        .map_err(|error| {
            CmdError::execution(format!(
                "Instanced objects require a rigid transform: {error}"
            ))
        })?;
    }
    // Validate the entire batch before changing either explicit or instanced objects.
    for (name, mask) in &selection_results {
        if mask.count() == 0 {
            continue;
        }
        let Some(object) = ctx.viewer.objects().get_molecule(name) else {
            continue;
        };
        if state > object.molecule().state_count() as i64 {
            return Err(CmdError::invalid_arg(
                "state",
                format!("state {state} does not exist in '{name}'"),
            ));
        }
        if inverse_object_transform(&object.state().transform).is_none() {
            return Err(CmdError::execution(format!(
                "Object '{name}' has a singular transform"
            )));
        }
        if object.state().instances.is_some() {
            let whole = match &whole_names {
                ResolvedNames::All => true,
                ResolvedNames::Matched(names) => names.contains(name),
                ResolvedNames::Unresolved => {
                    anchors.iter().filter(|a| &a.object_name == name).count()
                        == object.displayed_atom_count()
                }
            };
            if !whole {
                return Err(CmdError::execution(format!("Move the whole instanced object '{name}', or run materialize before editing a subset")));
            }
        }
    }

    let mut total_atoms = 0usize;
    let mut affected_objects = 0usize;

    for (obj_name, selected) in selection_results {
        let atoms: Vec<AtomIndex> = selected.indices().collect();
        if atoms.is_empty() {
            continue;
        }

        if let Some(mol_obj) = ctx.viewer.objects_mut().get_molecule_mut(&obj_name) {
            // Resolve state=-1 (current state) to the object's display state
            let resolved_state = if state == -1 {
                mol_obj.display_state() as i64 + 1 // convert 0-indexed to 1-indexed
            } else {
                state
            };
            let current = mol_obj.state().transform.clone();
            let whole = atoms.len() == mol_obj.molecule().atom_count();
            let has_assembly = !mol_obj.molecule().assembly.definitions.is_empty();
            let all_states = state == 0 || mol_obj.molecule().state_count() == 1;
            if movie_object
                || mol_obj.state().instances.is_some()
                || (whole && has_assembly && all_states)
            {
                mol_obj
                    .state_mut()
                    .set_transform(transform.clone() * current);
                mol_obj.invalidate(DirtyFlags::COORDS);
                total_atoms += mol_obj.displayed_atom_count();
            } else {
                // Commands operate in world coordinates, including after materialization.
                let local = inverse_object_transform(&current).expect("preflight checked inverse")
                    * transform.clone()
                    * current;
                apply_matrix_to_atoms(mol_obj.molecule_mut(), resolved_state, &atoms, &local);
                total_atoms += atoms.len();
            }
            affected_objects += 1;
        }
    }

    if affected_objects == 0 {
        return Err(CmdError::selection(format!(
            "No atoms matching '{}'",
            selection
        )));
    }

    Ok((total_atoms, affected_objects))
}

/// Inverts an affine object matrix using its three-dimensional basis.
pub(crate) fn inverse_object_transform(matrix: &Mat4) -> Option<Mat4> {
    let d = &matrix.data;
    if d.iter().any(|v| !v.is_finite())
        || d[3] != 0.0
        || d[7] != 0.0
        || d[11] != 0.0
        || d[15] != 1.0
    {
        return None;
    }
    // Use the 3x3 determinant: lin_alg 1.3's Mat4 determinant omits terms.
    let a = Vec3::new(d[0], d[1], d[2]);
    let b = Vec3::new(d[4], d[5], d[6]);
    let c = Vec3::new(d[8], d[9], d[10]);
    let det = a.dot(b.cross(c));
    if det == 0.0 || !det.is_finite() {
        return None;
    }
    let x = b.cross(c) * (1.0 / det);
    let y = c.cross(a) * (1.0 / det);
    let z = a.cross(b) * (1.0 / det);
    let t = Vec3::new(d[12], d[13], d[14]);
    let inverse = Mat4::new([
        x.x,
        y.x,
        z.x,
        0.0,
        x.y,
        y.y,
        z.y,
        0.0,
        x.z,
        y.z,
        z.z,
        0.0,
        -x.dot(t),
        -y.dot(t),
        -z.dot(t),
        1.0,
    ]);
    inverse
        .data
        .iter()
        .all(|v| v.is_finite())
        .then_some(inverse)
}

// ============================================================================
// Helper functions - Atom-level transformations
// ============================================================================

/// Collect which 0-indexed states to apply a transformation to.
///
/// - `0` (or negative) → all states
/// - Positive N → just state N-1 (if valid)
fn resolve_state_indices(state: i64, num_states: usize) -> Vec<usize> {
    match state_index_from_user(state) {
        None => (0..num_states).collect(),
        Some(idx) if idx < num_states => vec![idx],
        Some(_) => Vec::new(),
    }
}

/// Apply standard 4x4 matrix transform to specific atoms based on state parameter
fn apply_matrix_to_atoms(
    mol: &mut patinae_mol::ObjectMolecule,
    state: i64,
    atoms: &[AtomIndex],
    matrix: &lin_alg::f32::Mat4,
) {
    for i in resolve_state_indices(state, mol.state_count()) {
        mol.transform_atoms(i, atoms, matrix);
    }
}
