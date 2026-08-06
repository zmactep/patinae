//! Crystal symmetry commands: symexp
//!
//! Generates symmetry-related copies of a molecule using space group operations
//! and unit cell translations. Standard crystallographic technique — see
//! Rupp, "Biomolecular Crystallography", Garland Science, 2010, Ch. 6.

use std::sync::Arc;

use lin_alg::f32::{Mat4, Vec3, Vec4};

use patinae_algos::crystal::CrystalCell;
use patinae_mol::spatial::SpatialGrid;
use patinae_mol::{translation_matrix, RepMask};
use patinae_scene::{MoleculeObject, Object};

use crate::args::ParsedCommand;
use crate::command::{ArgHint, Command, CommandContext, CommandRegistry, ViewerLike};
use crate::command_help;
use crate::commands::selecting::evaluate_selection;
use crate::error::{CmdError, CmdResult};

pub fn register(registry: &mut CommandRegistry) {
    registry.register(SymExpCommand);
}

// ============================================================================
// symexp command
// ============================================================================

struct SymExpCommand;

impl Command for SymExpCommand {
    fn name(&self) -> &str {
        "symexp"
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[
            ArgHint::None,      // prefix
            ArgHint::Object,    // object
            ArgHint::Selection, // selection
            ArgHint::None,      // cutoff
        ]
    }

    command_help! {
        CMD "symexp"
        DESCRIPTION [
            "generates symmetry-related copies of an object within a",
            "specified distance cutoff. The new objects are named using the",
            "given prefix followed by symmetry operation and translation codes.",
        ]
        USAGE [
            "symexp prefix, object, selection, cutoff [, segi [, quiet]]",
        ]
        REQUIRED [
            { "prefix", "string", "prefix for new object names" },
            { "object", "string", "source object with crystallographic symmetry" },
            { "selection", "string", "atom selection defining the region of interest" },
        ]
        OPTIONAL [
            { "cutoff", "float", "distance cutoff in Angstroms", "10.0" },
            { "segi", "0/1", "store symmetry info in segment identifier", "0" },
            { "quiet", "0/1", "suppress output messages", "1" },
        ]
        EXAMPLES [
            "symexp sym, 1oky, chain A, 20.0",
            "symexp s, myprotein, all, 15.0, segi=1",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let prefix = args
            .str_arg(0, "prefix")
            .ok_or_else(|| CmdError::missing_argument("prefix".to_string()))?;
        let object = args
            .str_arg(1, "object")
            .ok_or_else(|| CmdError::missing_argument("object".to_string()))?;
        let selection = args
            .str_arg(2, "selection")
            .ok_or_else(|| CmdError::missing_argument("selection".to_string()))?;
        let cutoff = args.float_arg_or(3, "cutoff", 10.0) as f32;
        let segi = args.int_arg_or(4, "segi", 0) != 0;
        let quiet = args
            .int_arg(5, "quiet")
            .map(|v| v != 0)
            .unwrap_or(ctx.quiet);

        // Clone source data to release borrow on ctx.viewer
        let source = extract_source_data(ctx, object)?;

        let results = evaluate_selection(ctx.viewer, selection)?;
        let (sel_center, sel_coords) = collect_selection_coords(ctx.viewer, &results);

        if sel_coords.is_empty() {
            return Err(CmdError::execution("No atoms in selection!"));
        }

        let sel_frac = source.cell.to_fractional(sel_center);
        let grid = build_spatial_grid(&sel_coords, cutoff);
        let cutoff_sq = cutoff * cutoff;

        if !quiet {
            ctx.print(" SymExp: Generating symmetry mates...");
        }

        let mates = generate_symmetry_mates(
            &source,
            sel_frac,
            &grid,
            &sel_coords,
            cutoff_sq,
            prefix,
            segi,
        );

        let count = mates.len();
        let mut objects: Vec<Box<dyn Object>> = Vec::with_capacity(count);
        for mate in mates {
            let mut obj = MoleculeObject::from_raw_with_name(mate.molecule, &mate.name);
            obj.set_visible_reps(source.visible_reps);
            objects.push(Box::new(obj));
        }
        ctx.viewer.insert_objects(objects);

        ctx.viewer.request_redraw();

        if !quiet {
            ctx.print(&format!(
                " SymExp: Created {} symmetry mate objects.",
                count
            ));
        }

        Ok(())
    }
}

// ============================================================================
// Data extraction
// ============================================================================

/// All data needed from the source molecule, cloned to avoid borrow conflicts.
struct SourceData {
    molecule: patinae_mol::ObjectMolecule,
    visible_reps: RepMask,
    symops: Vec<Mat4>,
    cell: CrystalCell,
    state_centers: Vec<Vec3>,
}

fn extract_source_data(
    ctx: &CommandContext<'_, '_, dyn ViewerLike + '_>,
    object: &str,
) -> Result<SourceData, CmdError> {
    let mol_obj = ctx
        .viewer
        .objects()
        .get_molecule(object)
        .ok_or_else(|| CmdError::object_not_found(object.to_string()))?;

    let mol = mol_obj.molecule();

    let symmetry = mol
        .symmetry
        .as_ref()
        .or_else(|| {
            (0..mol.state_count())
                .find_map(|s| mol.get_coord_set(s).and_then(|cs| cs.symmetry.as_ref()))
        })
        .ok_or_else(|| CmdError::execution("No symmetry loaded!"))?;

    let symops =
        patinae_algos::space_groups::get_symops(&symmetry.space_group).ok_or_else(|| {
            CmdError::execution(format!("Unknown space group: '{}'", symmetry.space_group))
        })?;

    let cell = CrystalCell::new(
        Vec3::new(
            symmetry.cell_lengths[0],
            symmetry.cell_lengths[1],
            symmetry.cell_lengths[2],
        ),
        Vec3::new(
            symmetry.cell_angles[0],
            symmetry.cell_angles[1],
            symmetry.cell_angles[2],
        ),
    );

    let state_centers: Vec<Vec3> = (0..mol.state_count())
        .map(|s| {
            mol.get_coord_set(s)
                .map(|cs| cs.center())
                .unwrap_or(Vec3::new(0.0, 0.0, 0.0))
        })
        .collect();

    Ok(SourceData {
        molecule: mol.clone(),
        visible_reps: mol_obj.visible_reps(),
        symops,
        cell,
        state_centers,
    })
}

// ============================================================================
// Selection helpers
// ============================================================================

/// Collect coordinates and centroid from selection results.
fn collect_selection_coords(
    viewer: &dyn ViewerLike,
    results: &[(String, patinae_select::SelectionResult)],
) -> (Vec3, Vec<Vec3>) {
    let mut coords = Vec::new();
    let mut sum = Vec3::new(0.0, 0.0, 0.0);

    for (obj_name, selection) in results {
        if selection.count() == 0 {
            continue;
        }
        let Some(mol_obj) = viewer.objects().get_molecule(obj_name) else {
            continue;
        };
        let mol = mol_obj.molecule();

        for state in 0..mol.state_count() {
            let Some(cs) = mol.get_coord_set(state) else {
                continue;
            };
            for idx in selection.indices() {
                if let Some(coord) = cs.get_atom_coord(idx) {
                    sum += coord;
                    coords.push(coord);
                }
            }
        }
    }

    let center = if coords.is_empty() {
        Vec3::new(0.0, 0.0, 0.0)
    } else {
        sum * (1.0 / coords.len() as f32)
    };

    (center, coords)
}

fn build_spatial_grid(coords: &[Vec3], cutoff: f32) -> SpatialGrid {
    let mut grid = SpatialGrid::with_capacity(cutoff, coords.len());
    for (i, coord) in coords.iter().enumerate() {
        grid.insert(*coord, i);
    }
    grid
}

// ============================================================================
// Symmetry mate generation
// ============================================================================

/// A single symmetry-expanded molecule ready to be added to the scene.
struct SymmetryMate {
    name: String,
    molecule: patinae_mol::ObjectMolecule,
}

/// Unit cell translation offsets to search: -1, 0, +1 in each axis.
const CELL_OFFSETS: std::ops::RangeInclusive<i32> = -1..=1;

fn generate_symmetry_mates(
    source: &SourceData,
    sel_frac: Vec3,
    grid: &SpatialGrid,
    sel_coords: &[Vec3],
    cutoff_sq: f32,
    prefix: &str,
    segi: bool,
) -> Vec<SymmetryMate> {
    let r2f = source.cell.real_to_frac_4x4();
    let f2r = source.cell.frac_to_real_4x4();
    let mut mates = Vec::new();

    for x in CELL_OFFSETS {
        for y in CELL_OFFSETS {
            for z in CELL_OFFSETS {
                let cell_shift = Vec3::new(x as f32, y as f32, z as f32);
                for (op_idx, symop) in source.symops.iter().enumerate() {
                    let matrices = compute_state_transforms(
                        &r2f,
                        &f2r,
                        symop,
                        &source.state_centers,
                        sel_frac,
                        cell_shift,
                    );

                    if matrices.iter().all(is_approx_identity) {
                        continue;
                    }

                    if !any_atom_within_cutoff(
                        &source.molecule,
                        &matrices,
                        grid,
                        sel_coords,
                        cutoff_sq,
                    ) {
                        continue;
                    }

                    let molecule =
                        build_transformed_mol(&source.molecule, &matrices, segi, op_idx, x, y, z);
                    let name = format!(
                        "{}{:02}{:02}{:02}{:02}",
                        prefix,
                        op_idx,
                        x + 1,
                        y + 1,
                        z + 1
                    );

                    mates.push(SymmetryMate { name, molecule });
                }
            }
        }
    }

    mates
}

/// Compute the Cartesian transformation matrix for each state.
///
/// For each state center, the pipeline is:
///   real→frac → apply symop → shift into selection's unit cell → frac→real
fn compute_state_transforms(
    r2f: &Mat4,
    f2r: &Mat4,
    symop: &Mat4,
    state_centers: &[Vec3],
    sel_frac: Vec3,
    cell_shift: Vec3,
) -> Vec<Mat4> {
    state_centers
        .iter()
        .map(|center| {
            // Combine symop with real-to-fractional
            let frac_transform = symop.clone() * r2f.clone();

            // Find where this state's center lands in fractional space
            let center_frac = transform_point(&frac_transform, *center);

            // Round into the same unit cell as the selection, plus explicit cell offset
            let rounding_shift = Vec3::new(
                (sel_frac.x - center_frac.x).round() + cell_shift.x,
                (sel_frac.y - center_frac.y).round() + cell_shift.y,
                (sel_frac.z - center_frac.z).round() + cell_shift.z,
            );

            // Full pipeline: frac→real * shift * symop * real→frac
            f2r.clone() * translation_matrix(rounding_shift) * frac_transform
        })
        .collect()
}

// ============================================================================
// Distance check
// ============================================================================

/// Returns true if any transformed source atom is within `cutoff_sq` of a selection atom.
fn any_atom_within_cutoff(
    mol: &patinae_mol::ObjectMolecule,
    matrices: &[Mat4],
    grid: &SpatialGrid,
    sel_coords: &[Vec3],
    cutoff_sq: f32,
) -> bool {
    let mut neighbors = Vec::new();

    for (state_idx, mat) in matrices.iter().enumerate() {
        let Some(cs) = mol.get_coord_set(state_idx) else {
            continue;
        };

        for coord in cs.iter() {
            let transformed = transform_point(mat, coord);

            grid.query_neighbors(transformed, &mut neighbors);
            for &idx in &neighbors {
                if (transformed - sel_coords[idx]).magnitude_squared() <= cutoff_sq {
                    return true;
                }
            }
        }
    }

    false
}

// ============================================================================
// Molecule construction
// ============================================================================

fn build_transformed_mol(
    src: &patinae_mol::ObjectMolecule,
    matrices: &[Mat4],
    segi: bool,
    op_idx: usize,
    x: i32,
    y: i32,
    z: i32,
) -> patinae_mol::ObjectMolecule {
    let mut mol = src.clone();

    for (state_idx, mat) in matrices.iter().enumerate() {
        if !is_approx_identity(mat) {
            mol.transform(state_idx, mat);
        }
    }

    if segi {
        let label = segi_label(op_idx, x, y, z);
        for atom in mol.atoms_mut() {
            Arc::make_mut(&mut atom.residue).segi = label.clone();
        }
    }

    mol
}

// ============================================================================
// Utilities
// ============================================================================

/// Transform a point by a 4×4 matrix (homogeneous, w=1).
fn transform_point(m: &Mat4, v: Vec3) -> Vec3 {
    let r = m.clone() * Vec4::new(v.x, v.y, v.z, 1.0);
    Vec3::new(r.x, r.y, r.z)
}

fn is_approx_identity(m: &Mat4) -> bool {
    const EPS: f32 = 1e-4;
    m.data
        .iter()
        .zip(Mat4::new_identity().data.iter())
        .all(|(a, b)| (a - b).abs() < EPS)
}

/// Encode a symmetry operation + cell offset as a 4-character segment identifier.
///
/// Format: `<op><x+1><y+1><z+1>` where op is `A`..`Z`, `0`..`9`, `a`..`z`.
fn segi_label(symop_idx: usize, x: i32, y: i32, z: i32) -> String {
    let op_char = match symop_idx {
        0..26 => (b'A' + symop_idx as u8) as char,
        26..36 => (b'0' + (symop_idx - 26) as u8) as char,
        _ => (b'a' + (symop_idx - 36) as u8) as char,
    };
    format!("{}{}{}{}", op_char, x + 1, y + 1, z + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_segi_label() {
        assert_eq!(segi_label(0, 0, 0, 0), "A111");
        assert_eq!(segi_label(3, -1, 0, 1), "D012");
        assert_eq!(segi_label(25, 1, 1, 1), "Z222");
    }
}
