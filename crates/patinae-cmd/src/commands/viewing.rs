//! Viewing commands: zoom, center, orient, reset, clip, move, turn, origin, view

use std::f32::consts::PI;

use lin_alg::f32::{Mat4, Vec3};
use patinae_algos::svd3;
use patinae_scene::Object; // For extent() method on MoleculeObject

use crate::args::ParsedCommand;
use crate::command::{ArgHint, Command, CommandContext, CommandRegistry, ViewerLike};
use crate::command_help;
use crate::commands::selecting::evaluate_selection;
use crate::error::{CmdError, CmdResult};
use crate::helpers::{
    collect_selection_coords, resolve_object_names, selection_extent, ResolvedNames,
};

/// Compute the center and max radius of atoms matching a selection expression.
///
/// Returns the bounding-box center and the maximum distance from that center
/// to any selected atom. Used for `complete=1` zoom mode.
fn selection_max_radius(
    viewer: &dyn ViewerLike,
    selection: &str,
) -> CmdResult<Option<(Vec3, f32)>> {
    let Some((min, max)) = selection_extent(viewer, selection)? else {
        return Ok(None);
    };

    let center = Vec3::new(
        (min.x + max.x) * 0.5,
        (min.y + max.y) * 0.5,
        (min.z + max.z) * 0.5,
    );

    let selection_results = evaluate_selection(viewer, selection)?;
    let mut max_dist_sq: f32 = 0.0;

    for (obj_name, selected) in &selection_results {
        if selected.count() > 0 {
            if let Some(mol_obj) = viewer.objects().get_molecule(obj_name) {
                for idx in selected.indices() {
                    if let Some(coord) = mol_obj.display_coord(idx) {
                        let dx = coord.x - center.x;
                        let dy = coord.y - center.y;
                        let dz = coord.z - center.z;
                        let dist_sq = dx * dx + dy * dy + dz * dz;
                        if dist_sq > max_dist_sq {
                            max_dist_sq = dist_sq;
                        }
                    }
                }
            }
        }
    }

    Ok(Some((center, max_dist_sq.sqrt())))
}

/// Apply an axis-angle rotation to the camera's rotation matrix.
fn apply_camera_rotation(
    camera: &mut patinae_scene::Camera,
    angle_deg: f32,
    ax: f32,
    ay: f32,
    az: f32,
) {
    let angle_rad = angle_deg * PI / 180.0;
    let len = (ax * ax + ay * ay + az * az).sqrt();
    if len < 1e-10 {
        return;
    }
    let axis = Vec3::new(ax / len, ay / len, az / len);
    camera.rotate(axis, angle_rad);
}

/// Register viewing commands
pub fn register(registry: &mut CommandRegistry) {
    registry.register(ZoomCommand);
    registry.register(CenterCommand);
    registry.register(OrientCommand);
    registry.register(ResetCommand);
    registry.register(ClipCommand);
    registry.register(GetViewCommand);
    registry.register(SetViewCommand);
    registry.register(MoveCommand);
    registry.register(TurnCommand);
    registry.register(OriginCommand);
    registry.register(ViewCommand);
    registry.register(ViewportCommand);
    registry.register(FullScreenCommand);
}

// ============================================================================
// zoom command
// ============================================================================

struct ZoomCommand;

impl Command for ZoomCommand {
    fn name(&self) -> &str {
        "zoom"
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Selection]
    }

    command_help! {
        CMD "zoom"
        DESCRIPTION [
            "scales and translates the camera to cover the indicated selection.",
        ]
        REQUIRED []
        OPTIONAL [
            { "selection", "string", "selection to zoom to", "all" },
            { "buffer", "float", "extra space around selection in Angstroms", "0" },
            { "state", "integer", "state to zoom to", "0 = current" },
            { "complete", "0/1", "if 1, always do a complete zoom", "0" },
            { "animate", "float", "animation time in seconds", "0" },
        ]
        EXAMPLES [
            "zoom",
            "zoom chain A",
            "zoom organic, buffer=5",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let selection = args.str_arg_or(0, "selection", "all");
        let buffer = args.float_arg_or(1, "buffer", 0.0) as f32;
        let _state = args.int_arg_or(2, "state", 0);
        let complete = args.int_arg_or(3, "complete", 0) != 0;
        let _animate = args.float_arg_or(4, "animate", 0.0);

        match resolve_object_names(ctx.viewer.objects(), selection) {
            ResolvedNames::All => {
                ctx.viewer.zoom_all(buffer);
                if !ctx.quiet {
                    ctx.print(" Zoomed to all objects");
                }
                return Ok(());
            }
            ResolvedNames::Matched(names) => {
                ctx.viewer.zoom_on(&names[0], buffer);
                if !ctx.quiet {
                    ctx.print(&format!(" Zoomed to \"{}\"", names[0]));
                }
                return Ok(());
            }
            ResolvedNames::Unresolved => {}
        }

        // Try as selection expression (resi 28, chain A, name CA, etc.)
        if complete {
            // complete=1: use true bounding sphere (max distance from center to any atom)
            if let Some((center, radius)) = selection_max_radius(ctx.viewer, selection)? {
                ctx.viewer
                    .camera_mut()
                    .zoom_to_sphere(center, radius, buffer);
                ctx.viewer.set_viewport_image(None);
                ctx.viewer.request_redraw();
                if !ctx.quiet {
                    ctx.print(&format!(" Zoomed to \"{}\"", selection));
                }
                return Ok(());
            }
        } else if let Some((min, max)) = selection_extent(ctx.viewer, selection)? {
            ctx.viewer.camera_mut().zoom_to(min, max, buffer);
            ctx.viewer.set_viewport_image(None);
            ctx.viewer.request_redraw();
            if !ctx.quiet {
                ctx.print(&format!(" Zoomed to \"{}\"", selection));
            }
            return Ok(());
        }

        Err(CmdError::selection(format!(
            "No atoms matching '{}'",
            selection
        )))
    }
}

// ============================================================================
// center command
// ============================================================================

struct CenterCommand;

impl Command for CenterCommand {
    fn name(&self) -> &str {
        "center"
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Selection]
    }

    command_help! {
        CMD "center"
        DESCRIPTION [
            "translates the camera origin to the center of the selection.",
        ]
        REQUIRED []
        OPTIONAL [
            { "selection", "string", "selection to center on", "all" },
            { "state", "integer", "state to center on", "0" },
            { "origin", "0/1", "also set origin", "1" },
            { "animate", "float", "animation time in seconds", "0" },
        ]
        EXAMPLES [
            "center",
            "center chain A",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let selection = args.str_arg_or(0, "selection", "all");

        match resolve_object_names(ctx.viewer.objects(), selection) {
            ResolvedNames::All => {
                ctx.viewer.center_all();
                if !ctx.quiet {
                    ctx.print(" Centered on all objects");
                }
                return Ok(());
            }
            ResolvedNames::Matched(names) => {
                ctx.viewer.center_on(&names[0]);
                if !ctx.quiet {
                    ctx.print(&format!(" Centered on \"{}\"", names[0]));
                }
                return Ok(());
            }
            ResolvedNames::Unresolved => {}
        }

        // Try as selection expression (resi 28, chain A, name CA, etc.)
        if let Some((min, max)) = selection_extent(ctx.viewer, selection)? {
            ctx.viewer.camera_mut().center_to(min, max);
            ctx.viewer.request_redraw();
            if !ctx.quiet {
                ctx.print(&format!(" Centered on \"{}\"", selection));
            }
            return Ok(());
        }

        Err(CmdError::selection(format!(
            "No atoms matching '{}'",
            selection
        )))
    }
}

// ============================================================================
// orient command
// ============================================================================

struct OrientCommand;

impl Command for OrientCommand {
    fn name(&self) -> &str {
        "orient"
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Selection]
    }

    command_help! {
        CMD "orient"
        DESCRIPTION [
            "aligns the principal axes of the selection with the camera.",
        ]
        REQUIRED []
        OPTIONAL [
            { "selection", "string", "selection to orient", "all" },
            { "state", "integer", "state to orient", "0" },
            { "animate", "float", "animation time in seconds", "0" },
        ]
        EXAMPLES [
            "orient",
            "orient polymer",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let selection = args.str_arg_or(0, "selection", "all");

        // 1. Collect coordinates from the selection
        let coords = collect_selection_coords(ctx.viewer, selection)?;
        if coords.is_empty() {
            return Err(CmdError::selection(format!(
                "No atoms matching '{}'",
                selection
            )));
        }

        // 2. Compute centroid and bounds from the same resolved coordinates.
        let n = coords.len() as f32;
        let mut cx = 0.0f32;
        let mut cy = 0.0f32;
        let mut cz = 0.0f32;
        let mut min = Vec3::new(f32::MAX, f32::MAX, f32::MAX);
        let mut max = Vec3::new(f32::MIN, f32::MIN, f32::MIN);
        for c in &coords {
            cx += c.x;
            cy += c.y;
            cz += c.z;
            min.x = min.x.min(c.x);
            min.y = min.y.min(c.y);
            min.z = min.z.min(c.z);
            max.x = max.x.max(c.x);
            max.y = max.y.max(c.y);
            max.z = max.z.max(c.z);
        }
        cx /= n;
        cy /= n;
        cz /= n;

        // 3. Build moment of inertia tensor (3×3 symmetric matrix)
        //    Moment of inertia tensor:
        //    d[i][i] = Σ(|r|² - r_i²)
        //    d[i][j] = -Σ(r_i * r_j)   for i ≠ j
        let mut inertia = [[0.0f32; 3]; 3];
        for c in &coords {
            let dx = c.x - cx;
            let dy = c.y - cy;
            let dz = c.z - cz;
            let r2 = dx * dx + dy * dy + dz * dz;

            inertia[0][0] += r2 - dx * dx;
            inertia[0][1] += -dx * dy;
            inertia[0][2] += -dx * dz;
            inertia[1][0] += -dy * dx;
            inertia[1][1] += r2 - dy * dy;
            inertia[1][2] += -dy * dz;
            inertia[2][0] += -dz * dx;
            inertia[2][1] += -dz * dy;
            inertia[2][2] += r2 - dz * dz;
        }

        // 4. Eigendecompose via SVD (for symmetric PSD, SVD = eigendecomposition)
        //    svd3 takes column-major [[f32; 3]; 3] — symmetric matrix is its own transpose
        let svd = svd3(&inertia);

        // svd.u columns are eigenvectors, svd.s are eigenvalues (sorted descending)
        // svd.u[col][row] in column-major layout

        // 5. Build rotation matrix from eigenvectors
        let mut rot_cols = [
            [svd.u[0][0], svd.u[0][1], svd.u[0][2]], // column 0 (largest eigenvalue)
            [svd.u[1][0], svd.u[1][1], svd.u[1][2]], // column 1
            [svd.u[2][0], svd.u[2][1], svd.u[2][2]], // column 2 (smallest eigenvalue)
        ];

        // Normalize columns
        for col in &mut rot_cols {
            let len = (col[0] * col[0] + col[1] * col[1] + col[2] * col[2]).sqrt();
            if len > 1e-10 {
                col[0] /= len;
                col[1] /= len;
                col[2] /= len;
            }
        }

        // Ensure right-handedness: cross(col0, col1) should align with col2
        let cross = [
            rot_cols[0][1] * rot_cols[1][2] - rot_cols[0][2] * rot_cols[1][1],
            rot_cols[0][2] * rot_cols[1][0] - rot_cols[0][0] * rot_cols[1][2],
            rot_cols[0][0] * rot_cols[1][1] - rot_cols[0][1] * rot_cols[1][0],
        ];
        let dot = cross[0] * rot_cols[2][0] + cross[1] * rot_cols[2][1] + cross[2] * rot_cols[2][2];
        if dot < 0.0 {
            rot_cols[2][0] = -rot_cols[2][0];
            rot_cols[2][1] = -rot_cols[2][1];
            rot_cols[2][2] = -rot_cols[2][2];
        }

        // 6. Build Mat4 from column-major rotation
        //    Mat4 stores column-major: data[col*4 + row]
        let mut rotation = Mat4::new_identity();
        rotation.data[0] = rot_cols[0][0];
        rotation.data[1] = rot_cols[0][1];
        rotation.data[2] = rot_cols[0][2];
        rotation.data[4] = rot_cols[1][0];
        rotation.data[5] = rot_cols[1][1];
        rotation.data[6] = rot_cols[1][2];
        rotation.data[8] = rot_cols[2][0];
        rotation.data[9] = rot_cols[2][1];
        rotation.data[10] = rot_cols[2][2];

        // 7. Reorder axes: smallest eigenvalue → X, largest → Z
        //    SVD returns s[0] >= s[1] >= s[2], so column 0 has the largest
        //    eigenvalue and column 2 the smallest. We need to swap columns
        //    0 ↔ 2 so the most spread axis maps to Z.
        //    A 90° rotation around Y swaps X ↔ Z.
        let old_rotation = ctx.viewer.camera().view().rotation.clone();
        ctx.viewer.camera_mut().view_mut().rotation = rotation;
        apply_camera_rotation(ctx.viewer.camera_mut(), 90.0, 0.0, 1.0, 0.0);

        // 8. Choose orientation closest to previous view (minimize perturbation)
        let new_rotation = ctx.viewer.camera().view().rotation.clone();

        // Compare each axis direction with old rotation
        let x_dot = old_rotation.data[0] * new_rotation.data[0]
            + old_rotation.data[4] * new_rotation.data[4]
            + old_rotation.data[8] * new_rotation.data[8];
        let y_dot = old_rotation.data[1] * new_rotation.data[1]
            + old_rotation.data[5] * new_rotation.data[5]
            + old_rotation.data[9] * new_rotation.data[9];
        let z_dot = old_rotation.data[2] * new_rotation.data[2]
            + old_rotation.data[6] * new_rotation.data[6]
            + old_rotation.data[10] * new_rotation.data[10];

        // If two axes flip, do a 180° rotation around the third to correct
        if x_dot > 0.0 && y_dot < 0.0 && z_dot < 0.0 {
            apply_camera_rotation(ctx.viewer.camera_mut(), 180.0, 1.0, 0.0, 0.0);
        } else if x_dot < 0.0 && y_dot > 0.0 && z_dot < 0.0 {
            apply_camera_rotation(ctx.viewer.camera_mut(), 180.0, 0.0, 1.0, 0.0);
        } else if x_dot < 0.0 && y_dot < 0.0 && z_dot > 0.0 {
            apply_camera_rotation(ctx.viewer.camera_mut(), 180.0, 0.0, 0.0, 1.0);
        }

        // 9. Fit the bounds already accumulated before changing the camera.
        ctx.viewer.camera_mut().zoom_to(min, max, 0.0);

        ctx.viewer.set_viewport_image(None);
        ctx.viewer.request_redraw();

        if !ctx.quiet {
            ctx.print(&format!(" Oriented to \"{}\"", selection));
        }

        Ok(())
    }
}

// ============================================================================
// reset command
// ============================================================================

struct ResetCommand;

impl Command for ResetCommand {
    fn name(&self) -> &str {
        "reset"
    }

    command_help! {
        CMD "reset"
        DESCRIPTION [
            "resets the camera to the default view.",
        ]
        REQUIRED []
        OPTIONAL [
            { "object", "string", "object to reset", "all" },
        ]
        EXAMPLES [
            "reset",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        _args: &ParsedCommand,
    ) -> CmdResult {
        ctx.viewer.reset_view();

        if !ctx.quiet {
            ctx.print(" Reset view");
        }

        Ok(())
    }
}

// ============================================================================
// clip command
// ============================================================================

struct ClipCommand;

impl Command for ClipCommand {
    fn name(&self) -> &str {
        "clip"
    }

    command_help! {
        CMD "clip"
        DESCRIPTION [
            "alters the near and far clipping planes.",
        ]
        REQUIRED [
            { "mode", "string", "clipping mode" } => [
                "near: adjust near clipping plane",
                "far: adjust far clipping plane",
                "move: move both planes",
                "slab: set slab thickness",
                "atoms: clip to atoms",
            ],
            { "distance", "float", "clipping distance adjustment" },
        ]
        OPTIONAL [
            { "selection", "string", "selection for reference", "all" },
        ]
        EXAMPLES [
            "clip near, -5",
            "clip far, 10",
            "clip slab, 20",
        ]
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[
            ArgHint::Keywords(&["near", "far", "move", "slab"]),
            ArgHint::None,
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let mode = args
            .str_arg(0, "mode")
            .ok_or_else(|| CmdError::missing_argument("mode".to_string()))?;
        let distance = args
            .float_arg(1, "distance")
            .ok_or_else(|| CmdError::missing_argument("distance".to_string()))?
            as f32;

        let (clip_front, clip_back) = {
            let view = ctx.viewer.camera_mut().view_mut();

            match mode.to_lowercase().as_str() {
                "near" => {
                    view.clip_front = (view.clip_front + distance).max(0.01);
                }
                "far" => {
                    view.clip_back = (view.clip_back + distance).max(view.clip_front + 0.01);
                }
                "move" => {
                    view.clip_front = (view.clip_front + distance).max(0.01);
                    view.clip_back = (view.clip_back + distance).max(view.clip_front + 0.01);
                }
                "slab" => {
                    // Set slab thickness centered on current position
                    let center = (view.clip_front + view.clip_back) / 2.0;
                    view.clip_front = (center - distance / 2.0).max(0.01);
                    view.clip_back = center + distance / 2.0;
                }
                _ => {
                    return Err(CmdError::invalid_arg(
                        "mode",
                        format!("unknown clip mode: {}", mode),
                    ));
                }
            }
            (view.clip_front, view.clip_back)
        };

        ctx.viewer.request_redraw();

        if !ctx.quiet {
            ctx.print(&format!(
                " Clip: near={:.1}, far={:.1}",
                clip_front, clip_back
            ));
        }

        Ok(())
    }
}

// ============================================================================
// get_view command
// ============================================================================

struct GetViewCommand;

impl Command for GetViewCommand {
    fn name(&self) -> &str {
        "get_view"
    }

    command_help! {
        CMD "get_view"
        DESCRIPTION [
            "returns the current view matrix as a tuple of 18 values.",
        ]
        REQUIRED []
        OPTIONAL [
            { "output", "0/1/2", "output mode", "0" },
        ]
        EXAMPLES [
            "get_view",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        _args: &ParsedCommand,
    ) -> CmdResult {
        let view = ctx.viewer.camera().current_view();

        // Format view as set_view command output.
        let mut output = String::from("### cut below here and paste into the command line ###\n");
        output.push_str("set_view (\\\n");

        // Rotation matrix (4x4, but we output 3x3 part for cross-compatibility)
        let r = &view.rotation;
        output.push_str(&format!(
            "  {:12.6}, {:12.6}, {:12.6},\\\n",
            r.data[0], r.data[1], r.data[2]
        ));
        output.push_str(&format!(
            "  {:12.6}, {:12.6}, {:12.6},\\\n",
            r.data[4], r.data[5], r.data[6]
        ));
        output.push_str(&format!(
            "  {:12.6}, {:12.6}, {:12.6},\\\n",
            r.data[8], r.data[9], r.data[10]
        ));

        // Camera position
        output.push_str(&format!(
            "  {:12.6}, {:12.6}, {:12.6},\\\n",
            view.position.x, view.position.y, view.position.z
        ));

        // Origin
        output.push_str(&format!(
            "  {:12.6}, {:12.6}, {:12.6},\\\n",
            view.origin.x, view.origin.y, view.origin.z
        ));

        // Clip planes and FOV
        output.push_str(&format!(
            "  {:12.6}, {:12.6}, {:12.6} )\n",
            view.clip_front, view.clip_back, view.fov
        ));

        output.push_str("### cut above here and paste into the command line ###");

        ctx.print(&output);

        Ok(())
    }
}

// ============================================================================
// set_view command
// ============================================================================

struct SetViewCommand;

impl Command for SetViewCommand {
    fn name(&self) -> &str {
        "set_view"
    }

    command_help! {
        CMD "set_view"
        DESCRIPTION [
            "sets the camera view matrix from a tuple of 18 values.",
        ]
        REQUIRED [
            { "view", "tuple of 18 floats", "the view matrix" },
        ]
        OPTIONAL []
        EXAMPLES [
            "set_view (\\",
            "  1.0, 0.0, 0.0,\\",
            "  0.0, 1.0, 0.0,\\",
            "  0.0, 0.0, 1.0,\\",
            "  0.0, 0.0, -50.0,\\",
            "  0.0, 0.0, 0.0,\\",
            "  10.0, 100.0, 0.0)",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        // Parse the view values
        // Accept a tuple as the argument.
        let arg0 = args
            .get_arg(0)
            .ok_or_else(|| CmdError::missing_argument("view".to_string()))?;

        let values: Vec<f32> = match arg0 {
            crate::args::ArgValue::List(items) => items
                .iter()
                .filter_map(|v| v.as_float().map(|f| f as f32))
                .collect(),
            crate::args::ArgValue::String(s) => {
                // Try to parse as comma-separated values
                let s = s.trim_matches(|c| c == '(' || c == ')');
                s.split(',')
                    .filter_map(|v| v.trim().parse::<f32>().ok())
                    .collect()
            }
            _ => return Err(CmdError::invalid_arg("view", "expected tuple of 18 values")),
        };

        if values.len() < 18 {
            return Err(CmdError::invalid_arg(
                "view",
                format!("expected 18 values, got {}", values.len()),
            ));
        }

        let view = ctx.viewer.camera_mut().view_mut();

        // Set rotation matrix (3x3 -> 4x4)
        // PSE format uses row-major 3x3, we use column-major 4x4
        let mut rotation_data = [[0.0f32; 4]; 4];
        rotation_data[0] = [values[0], values[1], values[2], 0.0];
        rotation_data[1] = [values[3], values[4], values[5], 0.0];
        rotation_data[2] = [values[6], values[7], values[8], 0.0];
        rotation_data[3] = [0.0, 0.0, 0.0, 1.0];
        view.rotation = Mat4::from(rotation_data);

        // Set camera position
        view.position = Vec3::new(values[9], values[10], values[11]);

        // Set origin
        view.origin = Vec3::new(values[12], values[13], values[14]);

        // Set clip planes and FOV
        view.clip_front = values[15];
        view.clip_back = values[16];
        view.fov = values[17];

        ctx.viewer.request_redraw();

        if !ctx.quiet {
            ctx.print(" View set");
        }

        Ok(())
    }
}

// ============================================================================
// move command
// ============================================================================

struct MoveCommand;

impl Command for MoveCommand {
    fn name(&self) -> &str {
        "move"
    }

    command_help! {
        CMD "move"
        DESCRIPTION [
            "translates the camera about one of the three primary axes.",
        ]
        REQUIRED [
            { "axis", "x/y/z", "axis along which to translate" },
            { "distance", "float", "distance to move in Angstroms" },
        ]
        OPTIONAL []
        NOTES("NOTES") [
            "Positive x moves right, positive y moves up, positive z moves",
            "toward the viewer.",
        ]
        EXAMPLES [
            "move x, 3",
            "move y, -1",
            "move z, 10",
        ]
        SEE ALSO [
            "turn, rotate, translate, zoom, center, clip",
        ]
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Keywords(&["x", "y", "z"]), ArgHint::None]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let axis = args
            .str_arg(0, "axis")
            .ok_or_else(|| CmdError::missing_argument("axis".to_string()))?;
        let distance = args
            .float_arg(1, "distance")
            .ok_or_else(|| CmdError::missing_argument("distance".to_string()))?
            as f32;

        // Create translation vector based on axis
        let delta = match axis.to_lowercase().as_str() {
            "x" => Vec3::new(distance, 0.0, 0.0),
            "y" => Vec3::new(0.0, distance, 0.0),
            "z" => Vec3::new(0.0, 0.0, distance),
            _ => {
                return Err(CmdError::invalid_arg(
                    "axis",
                    format!("unknown axis '{}'. Use x, y, or z.", axis),
                ));
            }
        };

        // Apply translation to camera
        ctx.viewer.camera_mut().translate(delta);
        ctx.viewer.request_redraw();

        if !ctx.quiet {
            ctx.print(&format!(" Moved {} by {:.3}", axis, distance));
        }

        Ok(())
    }
}

// ============================================================================
// turn command
// ============================================================================

struct TurnCommand;

impl Command for TurnCommand {
    fn name(&self) -> &str {
        "turn"
    }

    command_help! {
        CMD "turn"
        DESCRIPTION [
            "rotates the camera about one of the three primary axes,",
            "centered at the origin.",
        ]
        REQUIRED [
            { "axis", "x/y/z", "axis about which to rotate" },
            { "angle", "float", "degrees of rotation" },
        ]
        OPTIONAL []
        NOTES("NOTES") [
            "Rotations follow the right-hand rule. For example, a positive",
            "rotation about the y-axis will rotate the view to the right.",
        ]
        EXAMPLES [
            "turn x, 90",
            "turn y, 45",
            "turn z, -30",
        ]
        SEE ALSO [
            "move, rotate, translate, zoom, center, clip",
        ]
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Keywords(&["x", "y", "z"]), ArgHint::None]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let axis = args
            .str_arg(0, "axis")
            .ok_or_else(|| CmdError::missing_argument("axis".to_string()))?;
        let angle_deg =
            args.float_arg(1, "angle")
                .ok_or_else(|| CmdError::missing_argument("angle".to_string()))? as f32;

        // Convert to radians
        let angle_rad = angle_deg * PI / 180.0;

        // Apply rotation based on axis
        match axis.to_lowercase().as_str() {
            "x" => ctx.viewer.camera_mut().rotate_x(angle_rad),
            "y" => ctx.viewer.camera_mut().rotate_y(angle_rad),
            "z" => ctx.viewer.camera_mut().rotate_z(angle_rad),
            _ => {
                return Err(CmdError::invalid_arg(
                    "axis",
                    format!("unknown axis '{}'. Use x, y, or z.", axis),
                ));
            }
        };

        ctx.viewer.request_redraw();

        if !ctx.quiet {
            ctx.print(&format!(" Turned {} by {:.1} degrees", axis, angle_deg));
        }

        Ok(())
    }
}

// ============================================================================
// origin command
// ============================================================================

struct OriginCommand;

impl Command for OriginCommand {
    fn name(&self) -> &str {
        "origin"
    }

    command_help! {
        CMD "origin"
        DESCRIPTION [
            "sets the center of rotation about a selection or position.",
            "If an object name is specified, it can be used to set the center of",
            "rotation for the object (for use in animation and editing).",
        ]
        REQUIRED []
        OPTIONAL [
            { "selection", "string", "selection-expression or name-list", "(all)" },
            { "object", "string", "object name", "none" },
            { "position", "[x, y, z]", "explicit position coordinates", "none" },
            { "state", "integer", "state to use for coordinates", "0" },
        ]
        EXAMPLES [
            "origin chain A",
            "origin position=[1.0, 2.0, 3.0]",
            "origin resi 100",
        ]
        SEE ALSO [
            "zoom, orient, center, reset",
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
        // Check for explicit position first
        if let Some(position_arg) = args.get_named("position") {
            let position = parse_position_from_arg(position_arg)?;
            ctx.viewer.camera_mut().view_mut().origin = position;
            ctx.viewer.request_redraw();

            if !ctx.quiet {
                ctx.print(&format!(
                    " Origin set to [{:.3}, {:.3}, {:.3}]",
                    position.x, position.y, position.z
                ));
            }
            return Ok(());
        }

        let selection = args.str_arg_or(0, "selection", "all");

        match resolve_object_names(ctx.viewer.objects(), selection) {
            ResolvedNames::All => {
                if let Some((min, max)) = ctx.viewer.objects().extent() {
                    let center = Vec3::new(
                        (min.x + max.x) * 0.5,
                        (min.y + max.y) * 0.5,
                        (min.z + max.z) * 0.5,
                    );
                    ctx.viewer.camera_mut().view_mut().origin = center;
                    ctx.viewer.request_redraw();
                    if !ctx.quiet {
                        ctx.print(&format!(
                            " Origin set to [{:.3}, {:.3}, {:.3}]",
                            center.x, center.y, center.z
                        ));
                    }
                } else if !ctx.quiet {
                    ctx.print(" No objects to set origin from");
                }
                return Ok(());
            }
            ResolvedNames::Matched(names) => {
                let name = &names[0];
                if let Some(mol) = ctx.viewer.objects().get_molecule(name) {
                    if let Some((min, max)) = mol.extent() {
                        let center = Vec3::new(
                            (min.x + max.x) * 0.5,
                            (min.y + max.y) * 0.5,
                            (min.z + max.z) * 0.5,
                        );
                        ctx.viewer.camera_mut().view_mut().origin = center;
                        ctx.viewer.request_redraw();
                        if !ctx.quiet {
                            ctx.print(&format!(
                                " Origin set to \"{}\" at [{:.3}, {:.3}, {:.3}]",
                                name, center.x, center.y, center.z
                            ));
                        }
                        return Ok(());
                    }
                }
            }
            ResolvedNames::Unresolved => {}
        }

        // Try as selection expression (resi 28, chain A, name CA, etc.)
        if let Some((min, max)) = selection_extent(ctx.viewer, selection)? {
            let center = Vec3::new(
                (min.x + max.x) * 0.5,
                (min.y + max.y) * 0.5,
                (min.z + max.z) * 0.5,
            );
            ctx.viewer.camera_mut().view_mut().origin = center;
            ctx.viewer.request_redraw();
            if !ctx.quiet {
                ctx.print(&format!(
                    " Origin set to \"{}\" at [{:.3}, {:.3}, {:.3}]",
                    selection, center.x, center.y, center.z
                ));
            }
            return Ok(());
        }

        Err(CmdError::selection(format!(
            "No atoms matching '{}'",
            selection
        )))
    }
}

/// Parse a position vector from an argument value
fn parse_position_from_arg(arg: &crate::args::ArgValue) -> Result<Vec3, CmdError> {
    use crate::args::ArgValue;

    match arg {
        ArgValue::List(items) if items.len() >= 3 => {
            let x = items
                .first()
                .and_then(|v| v.as_float())
                .ok_or_else(|| CmdError::invalid_arg("position", "invalid x coordinate"))?
                as f32;
            let y = items
                .get(1)
                .and_then(|v| v.as_float())
                .ok_or_else(|| CmdError::invalid_arg("position", "invalid y coordinate"))?
                as f32;
            let z = items
                .get(2)
                .and_then(|v| v.as_float())
                .ok_or_else(|| CmdError::invalid_arg("position", "invalid z coordinate"))?
                as f32;
            Ok(Vec3::new(x, y, z))
        }
        ArgValue::String(s) => {
            // Try to parse "[x, y, z]" format
            let s = s.trim().trim_matches(|c| c == '[' || c == ']');
            let parts: Vec<&str> = s.split(',').collect();
            if parts.len() >= 3 {
                let x = parts[0]
                    .trim()
                    .parse::<f32>()
                    .map_err(|_| CmdError::invalid_arg("position", "invalid x coordinate"))?;
                let y = parts[1]
                    .trim()
                    .parse::<f32>()
                    .map_err(|_| CmdError::invalid_arg("position", "invalid y coordinate"))?;
                let z = parts[2]
                    .trim()
                    .parse::<f32>()
                    .map_err(|_| CmdError::invalid_arg("position", "invalid z coordinate"))?;
                Ok(Vec3::new(x, y, z))
            } else {
                Err(CmdError::invalid_arg(
                    "position",
                    "expected [x, y, z] format",
                ))
            }
        }
        _ => Err(CmdError::invalid_arg(
            "position",
            "expected [x, y, z] format",
        )),
    }
}

// ============================================================================
// view command
// ============================================================================

struct ViewCommand;

impl Command for ViewCommand {
    fn name(&self) -> &str {
        "view"
    }

    command_help! {
        CMD "view"
        DESCRIPTION [
            "saves and restores named camera views.",
            "",
            "Unlike \"scene\", \"view\" only stores camera state (rotation, position,",
            "origin, clipping planes, FOV) without colors, representations, or",
            "frame state.",
        ]
        REQUIRED [
            { "key", "string", "view name, or * for all views" },
        ]
        OPTIONAL [
            { "action", "string", "store, recall, or clear", "recall" },
            { "animate", "float", "animation duration in seconds", "0" },
        ]
        NOTES("NOTES") [
            "Views F1 through F12 are automatically bound to function keys",
            "provided that \"set_key\" has not been used to redefine the",
            "behavior of the respective key, and that a \"scene\" has not been",
            "defined for that key.",
        ]
        EXAMPLES [
            "view F1, store          # Store current view as \"F1\"",
            "view F1                 # Recall view \"F1\"",
            "view F1, recall, 1.0    # Recall with 1 second animation",
            "view F1, clear          # Delete view \"F1\"",
            "view *, clear           # Delete all views",
        ]
        SEE ALSO [
            "scene, get_view, set_view",
        ]
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[
            ArgHint::None,
            ArgHint::Keywords(&["store", "recall", "clear"]),
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let key = args.str_arg_or(0, "key", "");
        let action = args.str_arg_or(1, "action", "recall");
        let animate = args.float_arg_or(2, "animate", 0.0) as f32;

        // Handle wildcard key for listing/clearing
        if key == "*" {
            match action.to_lowercase().as_str() {
                "clear" => {
                    ctx.viewer.view_clear();
                    if !ctx.quiet {
                        ctx.print(" All views cleared.");
                    }
                }
                _ => {
                    // List all views
                    let views = ctx.viewer.view_list();
                    if views.is_empty() {
                        ctx.print(" No views stored.");
                    } else {
                        ctx.print(&format!(" Stored views: {}", views.join(", ")));
                    }
                }
            }
            return Ok(());
        }

        match action.to_lowercase().as_str() {
            "store" => {
                if key.is_empty() {
                    return Err(CmdError::missing_argument("key".to_string()));
                }
                ctx.viewer.view_store(key);
                if !ctx.quiet {
                    ctx.print(&format!(" View \"{}\" stored.", key));
                }
            }

            "recall" | "" => {
                if key.is_empty() {
                    return Err(CmdError::missing_argument("key".to_string()));
                }
                ctx.viewer
                    .view_recall(key, animate)
                    .map_err(|e| CmdError::invalid_arg("key", e))?;
                if !ctx.quiet {
                    ctx.print(&format!(" View \"{}\" recalled.", key));
                }
            }

            "clear" | "delete" => {
                if key.is_empty() {
                    ctx.viewer.view_clear();
                    if !ctx.quiet {
                        ctx.print(" All views cleared.");
                    }
                } else if ctx.viewer.view_delete(key) {
                    if !ctx.quiet {
                        ctx.print(&format!(" View \"{}\" deleted.", key));
                    }
                } else {
                    return Err(CmdError::invalid_arg(
                        "key",
                        format!("View '{}' not found.", key),
                    ));
                }
            }

            _ => {
                return Err(CmdError::invalid_arg(
                    "action",
                    format!("unknown action '{}'. Use store, recall, or clear.", action),
                ));
            }
        }

        Ok(())
    }
}

// ============================================================================
// viewport command
// ============================================================================

struct ViewportCommand;

impl Command for ViewportCommand {
    fn name(&self) -> &str {
        "viewport"
    }

    command_help! {
        CMD "viewport"
        DESCRIPTION [
            "changes the size of the graphics display area.",
        ]
        REQUIRED []
        OPTIONAL [
            { "width", "integer", "width in pixels", "current" },
            { "height", "integer", "height in pixels", "current" },
        ]
        NOTES("NOTES") [
            "If only width is specified, height will be scaled to maintain",
            "the current aspect ratio.",
            "",
            "If no arguments are given, the current viewport size is displayed.",
        ]
        EXAMPLES [
            "viewport                # Show current size",
            "viewport 800, 600       # Set to 800x600",
            "viewport 1920, 1080     # Set to 1920x1080",
        ]
        SEE ALSO [
            "full_screen, get_view, set_view",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let width = args.int_arg(0, "width");
        let height = args.int_arg(1, "height");

        match (width, height) {
            (None, None) => {
                // Display current viewport size
                let (w, h) = ctx.viewer.viewport_size();
                if w > 0 && h > 0 {
                    ctx.print(&format!(" Viewport: {} x {}", w, h));
                } else {
                    ctx.print(" Viewport size not available.");
                }
            }
            (Some(w), None) => {
                // Width only - maintain aspect ratio
                let (current_w, current_h) = ctx.viewer.viewport_size();
                let aspect = if current_w > 0 {
                    current_h as f64 / current_w as f64
                } else {
                    0.75 // Default 4:3 aspect
                };
                let h = (w as f64 * aspect).round() as u32;
                ctx.viewer.viewport_set_size(w as u32, h);
                if !ctx.quiet {
                    ctx.print(&format!(" Viewport set to {} x {}", w, h));
                }
            }
            (Some(w), Some(h)) => {
                // Both specified
                ctx.viewer.viewport_set_size(w as u32, h as u32);
                if !ctx.quiet {
                    ctx.print(&format!(" Viewport set to {} x {}", w, h));
                }
            }
            (None, Some(h)) => {
                // Height only - maintain aspect ratio
                let (current_w, current_h) = ctx.viewer.viewport_size();
                let aspect = if current_h > 0 {
                    current_w as f64 / current_h as f64
                } else {
                    4.0 / 3.0 // Default 4:3 aspect
                };
                let w = (h as f64 * aspect).round() as u32;
                ctx.viewer.viewport_set_size(w, h as u32);
                if !ctx.quiet {
                    ctx.print(&format!(" Viewport set to {} x {}", w, h));
                }
            }
        }

        Ok(())
    }
}

// ============================================================================
// full_screen command
// ============================================================================

struct FullScreenCommand;

impl Command for FullScreenCommand {
    fn name(&self) -> &str {
        "full_screen"
    }

    fn aliases(&self) -> &[&str] {
        &["fullscreen"]
    }

    command_help! {
        CMD "full_screen"
        DESCRIPTION [
            "enables or disables full screen mode.",
        ]
        REQUIRED []
        OPTIONAL [
            { "toggle", "string", "on, off, or toggle", "toggle" } => [
                "\"on\" or \"1\" enables fullscreen",
                "\"off\" or \"0\" disables fullscreen",
                "\"toggle\" or \"-1\" toggles the current state",
            ],
        ]
        NOTES("NOTES") [
            "This does not work correctly on all platforms. If you encounter",
            "trouble, try using the maximize button on the viewer window",
            "instead.",
        ]
        EXAMPLES [
            "full_screen             # Toggle fullscreen",
            "full_screen on          # Enable fullscreen",
            "full_screen off         # Disable fullscreen",
        ]
        SEE ALSO [
            "viewport",
        ]
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Keywords(&["on", "off", "toggle"])]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let toggle = args.str_arg(0, "toggle");

        match toggle {
            None | Some("toggle") | Some("-1") => {
                // Toggle
                ctx.viewer.toggle_fullscreen();
                let state = if ctx.viewer.is_fullscreen() {
                    "on"
                } else {
                    "off"
                };
                if !ctx.quiet {
                    ctx.print(&format!(" Fullscreen {}.", state));
                }
            }
            Some("on") | Some("1") | Some("true") | Some("yes") => {
                ctx.viewer.set_fullscreen(true);
                if !ctx.quiet {
                    ctx.print(" Fullscreen on.");
                }
            }
            Some("off") | Some("0") | Some("false") | Some("no") => {
                ctx.viewer.set_fullscreen(false);
                if !ctx.quiet {
                    ctx.print(" Fullscreen off.");
                }
            }
            Some(other) => {
                return Err(CmdError::invalid_arg(
                    "toggle",
                    format!("unknown value '{}'. Use on, off, or toggle.", other),
                ));
            }
        }

        Ok(())
    }
}
