//! Structural alignment command
//!
//! Implements `align mobile, target` with three methods:
//! - `kabsch` (default): direct Kabsch fit on corresponding atoms
//! - `sequence`: Needleman-Wunsch sequence alignment → Cα pairs → Kabsch fit
//! - `ce`: Combinatorial Extension structure-based alignment

use lin_alg::f32::{Mat4, Vec3};
use patinae_algos::{
    ce_align, global_align, rmsd, substitution_matrix, superpose, AlignedPair, AlignmentScoring,
    CeParams, SuperposeParams, SuperposeResult,
};
use patinae_mol::{residue_to_char, AtomIndex};
use patinae_scene::{DirtyFlags, Object};

use crate::args::ParsedCommand;
use crate::command::{ArgHint, Command, CommandContext, CommandRegistry, ViewerLike};
use crate::command_help;
use crate::commands::selecting::evaluate_atom_anchors;
use crate::error::{CmdError, CmdResult};

pub fn register(registry: &mut CommandRegistry) {
    registry.register(AlignCommand);
    registry.register(RmsdCommand);
}

struct AlignCommand;

impl Command for AlignCommand {
    fn name(&self) -> &str {
        "align"
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Selection, ArgHint::Selection]
    }

    command_help! {
        CMD "align"
        DESCRIPTION [
            "performs structural superposition of one selection onto another.",
            "Selected atoms define the fit; the complete mobile object moves, including all instances.",
        ]
        USAGE [
            "align mobile, target [, cycles [, cutoff [, method ]]]",
        ]
        REQUIRED [
            { "mobile", "string", "selection for the mobile object (will be moved)" },
            { "target", "string", "selection for the fixed object (stays in place)" },
        ]
        OPTIONAL [
            { "cycles", "int", "number of outlier rejection cycles", "5" },
            { "cutoff", "float", "outlier rejection cutoff (distance/RMSD ratio)", "2.0" },
            { "method", "string", "alignment method", "kabsch" } => [
                "kabsch   \u{2014} direct fit on matching atoms (selections must have same size)",
                "sequence \u{2014} sequence alignment to find C\u{03B1} correspondences",
                "ce       \u{2014} Combinatorial Extension structure-based alignment",
            ],
            { "matrix", "string", "substitution matrix for sequence alignment", "blosum62" } => [
                "Available: blosum62, blosum50, blosum80, pam250, identity",
            ],
            { "gap_open", "float", "gap opening penalty", "-10.0" },
            { "gap_extend", "float", "gap extension penalty", "-1.0" },
            { "win_size", "int", "CE fragment window size", "8" },
            { "gap_max", "int", "CE maximum gap between aligned fragments", "30" },
            { "d0", "float", "CE fragment similarity cutoff in Angstroms", "3.0" },
            { "d1", "float", "CE fragment compatibility cutoff in Angstroms", "4.0" },
        ]
        EXAMPLES [
            "align chain A, chain B",
            "align mobile, target, cycles=0",
            "align 1hpx, 1t46, method=sequence",
            "align capsid and instance 1 and chain A, reference and chain A, method=sequence",
            "align 1hpx, 1t46, method=sequence, matrix=blosum50",
            "align 1hpx, 1t46, method=sequence, gap_open=-12.0, gap_extend=-2.0",
            "align 1hpx, 1t46, method=ce",
            "align 1hpx, 1t46, method=ce, win_size=6",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let mobile_sel = args
            .get_str(0)
            .ok_or_else(|| CmdError::missing_argument("mobile selection"))?;
        let target_sel = args
            .get_str(1)
            .ok_or_else(|| CmdError::missing_argument("target selection"))?;

        let cycles = args.int_arg_or(2, "cycles", 5) as u32;

        let cutoff = args.float_arg(3, "cutoff").map(|f| f as f32).unwrap_or(2.0);

        let method = args.str_arg_or(4, "method", "kabsch");

        let params = SuperposeParams { cycles, cutoff };

        match method {
            "sequence" | "seq" => {
                let matrix_name = args.get_named_str("matrix").unwrap_or("blosum62");

                let matrix = substitution_matrix::get_matrix(matrix_name)
                    .ok_or_else(|| {
                        CmdError::invalid_arg(
                            "matrix",
                            format!(
                            "Unknown substitution matrix '{}'. Available: blosum62, blosum50, blosum80, pam250, identity",
                            matrix_name
                            ),
                        )
                    })?;

                let scoring = AlignmentScoring {
                    matrix,
                    gap_open: args.get_named_float("gap_open").unwrap_or(-10.0) as f32,
                    gap_extend: args.get_named_float("gap_extend").unwrap_or(-1.0) as f32,
                };

                align_by_sequence(ctx, mobile_sel, target_sel, &params, &scoring)
            }
            "ce" => align_by_ce(ctx, mobile_sel, target_sel, &params, args),
            _ => align_by_kabsch(ctx, mobile_sel, target_sel, &params),
        }
    }
}

// ============================================================================
// RMSD command (no superposition)
// ============================================================================

struct RmsdCommand;

impl Command for RmsdCommand {
    fn name(&self) -> &str {
        "rmsd"
    }

    fn aliases(&self) -> &[&str] {
        &["rms_cur"]
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Selection, ArgHint::Selection]
    }

    command_help! {
        CMD "rmsd"
        DESCRIPTION [
            "computes the root-mean-square deviation between two selections",
            "without performing any superposition (atoms stay in place).",
        ]
        REQUIRED [
            { "sel1", "string", "first atom selection" },
            { "sel2", "string", "second atom selection (must have same number of atoms)" },
        ]
        OPTIONAL []
        EXAMPLES [
            "rmsd chain A, chain B",
            "rmsd mol1 and name CA, mol2 and name CA",
        ]
        SEE ALSO [
            "align",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let sel1 = args
            .get_str(0)
            .ok_or_else(|| CmdError::missing_argument("first selection"))?;
        let sel2 = args
            .get_str(1)
            .ok_or_else(|| CmdError::missing_argument("second selection"))?;

        let resolve = |selection: &str| -> CmdResult<Vec<Vec3>> {
            let anchors = crate::commands::selecting::evaluate_atom_anchors(ctx.viewer, selection)?;
            let mut object_name = None;
            anchors
                .into_iter()
                .map(|anchor| {
                    if object_name
                        .as_ref()
                        .is_some_and(|name| name != &anchor.object_name)
                    {
                        return Err(CmdError::selection(
                            "RMSD operands must each select one molecule",
                        ));
                    }
                    object_name = Some(anchor.object_name.clone());
                    ctx.viewer
                        .objects()
                        .get_molecule(&anchor.object_name)
                        .and_then(|object| {
                            object.instance_world_coord(anchor.atom_index, anchor.instance)
                        })
                        .ok_or_else(|| CmdError::execution("RMSD operand has missing coordinates"))
                })
                .collect()
        };
        let coords1 = resolve(sel1)?;
        let coords2 = resolve(sel2)?;
        if coords1.len() != coords2.len() {
            return Err(CmdError::execution(format!(
                "Selections have different atom counts: {} vs {}",
                coords1.len(),
                coords2.len()
            )));
        }
        let n = coords1.len();
        if n == 0 {
            return Err(CmdError::execution("Selections are empty"));
        }

        let value = rmsd(&coords1, &coords2);

        ctx.print(&format!(" Executive: RMSD = {:8.3}, {} atoms", value, n));

        Ok(())
    }
}

/// Direct Kabsch alignment — selections must have equal atom count.
///
/// Aligns each mobile object independently to the target.
fn align_by_kabsch(
    ctx: &mut CommandContext<'_, '_, dyn ViewerLike + '_>,
    mobile_sel: &str,
    target_sel: &str,
    params: &SuperposeParams,
) -> CmdResult {
    let mobile_objects = selected_fit_atoms(ctx.viewer, mobile_sel)?;
    let (target_obj, target_indices) = target_fit_atoms(ctx.viewer, target_sel)?;
    let target_coords = extract_coords(ctx.viewer, &target_obj, &target_indices)?;

    let mut aligned_count = 0usize;

    for (mobile_obj, mobile_indices) in &mobile_objects {
        if mobile_obj == &target_obj {
            continue;
        }

        if mobile_indices.len() != target_indices.len() {
            if mobile_objects.len() == 1 {
                return Err(CmdError::execution(format!(
                    "Selections have different atom counts: {} vs {} (use method=sequence for unequal sizes)",
                    mobile_indices.len(), target_indices.len()
                )));
            }
            ctx.print_warning(&format!(
                " Skipping \"{}\": atom count {} != target {}",
                mobile_obj,
                mobile_indices.len(),
                target_indices.len()
            ));
            continue;
        }

        let n = mobile_indices.len();
        if n < 3 {
            if mobile_objects.len() == 1 {
                return Err(CmdError::execution(format!(
                    "Need at least 3 atoms for alignment, got {}",
                    n
                )));
            }
            continue;
        }

        let mobile_coords = extract_coords(ctx.viewer, mobile_obj, mobile_indices)?;
        let pairs: Vec<(usize, usize)> = (0..n).map(|i| (i, i)).collect();

        let result = superpose(&mobile_coords, &target_coords, &pairs, params)
            .map_err(|e: patinae_algos::AlignError| CmdError::execution(e.to_string()))?;

        apply_superpose_transform(ctx, mobile_obj, &result)?;
        if mobile_objects.len() > 1 {
            ctx.print(&format!(
                " Executive: \"{}\" RMSD = {:8.3}, {} atoms",
                mobile_obj, result.final_rmsd, result.n_aligned
            ));
        } else {
            print_superpose_result(ctx, &result, params, &[]);
        }
        aligned_count += 1;
    }

    if aligned_count == 0 {
        return Err(CmdError::selection(format!(
            "No mobile objects to align (target \"{}\" excluded)",
            target_obj
        )));
    }

    Ok(())
}

/// Sequence-based alignment — aligns by matching residue sequences, then fits Cα atoms.
///
/// Aligns each mobile object independently to the target.
fn align_by_sequence(
    ctx: &mut CommandContext<'_, '_, dyn ViewerLike + '_>,
    mobile_sel: &str,
    target_sel: &str,
    params: &SuperposeParams,
    scoring: &AlignmentScoring,
) -> CmdResult {
    let mobile_objects = selected_fit_atoms(ctx.viewer, mobile_sel)?;
    let (target_obj, target_indices) = target_fit_atoms(ctx.viewer, target_sel)?;
    let (target_seq, target_ca) =
        extract_residue_sequence(ctx.viewer, &target_obj, &target_indices)?;

    if target_seq.is_empty() {
        return Err(CmdError::execution(
            "No protein/nucleic acid residues found in target selection",
        ));
    }

    let mut aligned_count = 0usize;

    for (mobile_obj, mobile_indices) in &mobile_objects {
        if mobile_obj == &target_obj {
            continue;
        }

        let (mobile_seq, mobile_ca) =
            extract_residue_sequence(ctx.viewer, mobile_obj, mobile_indices)?;

        if mobile_seq.is_empty() {
            if mobile_objects.len() == 1 {
                return Err(CmdError::execution(
                    "No protein/nucleic acid residues found in mobile selection",
                ));
            }
            continue;
        }

        let alignment = global_align(&mobile_seq, &target_seq, scoring);

        let mut mobile_ca_indices: Vec<FitAtom> = Vec::new();
        let mut target_ca_indices: Vec<FitAtom> = Vec::new();

        for pair in &alignment.pairs {
            if let AlignedPair::Match { source, target } = pair {
                if let (Some(&Some(src_ca_idx)), Some(&Some(tgt_ca_idx))) =
                    (mobile_ca.get(*source), target_ca.get(*target))
                {
                    mobile_ca_indices.push(src_ca_idx);
                    target_ca_indices.push(tgt_ca_idx);
                }
            }
        }

        let mobile_ca_coords = extract_coords(ctx.viewer, mobile_obj, &mobile_ca_indices)?;
        let target_ca_coords = extract_coords(ctx.viewer, &target_obj, &target_ca_indices)?;
        let ca_pairs: Vec<(usize, usize)> = (0..mobile_ca_coords.len()).map(|i| (i, i)).collect();

        if ca_pairs.len() < 3 {
            if mobile_objects.len() == 1 {
                return Err(CmdError::execution(format!(
                    "Too few Cα pairs for alignment (need ≥3, got {})",
                    ca_pairs.len()
                )));
            }
            continue;
        }

        let result = superpose(&mobile_ca_coords, &target_ca_coords, &ca_pairs, params)
            .map_err(|e: patinae_algos::AlignError| CmdError::execution(e.to_string()))?;

        apply_superpose_transform(ctx, mobile_obj, &result)?;
        if mobile_objects.len() > 1 {
            ctx.print(&format!(
                " Executive: \"{}\" RMSD = {:8.3}, {} Cα pairs, {:.1}% identity",
                mobile_obj,
                result.final_rmsd,
                ca_pairs.len(),
                alignment.identity * 100.0
            ));
        } else {
            print_superpose_result(
                ctx,
                &result,
                params,
                &[
                    format!(
                        "   Sequence identity: {:.1}% ({} of {} residues)",
                        alignment.identity * 100.0,
                        alignment.n_matched,
                        mobile_seq.len().max(target_seq.len())
                    ),
                    format!("   Matched Cα pairs:  {}", ca_pairs.len()),
                ],
            );
        }
        aligned_count += 1;
    }

    if aligned_count == 0 {
        return Err(CmdError::selection(format!(
            "No mobile objects to align (target \"{}\" excluded)",
            target_obj
        )));
    }

    Ok(())
}

/// CE structural alignment — structure-based alignment using Combinatorial Extension.
///
/// Aligns each mobile object independently to the target.
fn align_by_ce(
    ctx: &mut CommandContext<'_, '_, dyn ViewerLike + '_>,
    mobile_sel: &str,
    target_sel: &str,
    params: &SuperposeParams,
    args: &ParsedCommand,
) -> CmdResult {
    let mobile_objects = selected_fit_atoms(ctx.viewer, mobile_sel)?;
    let (target_obj, target_indices) = target_fit_atoms(ctx.viewer, target_sel)?;
    let (_, target_ca) = extract_residue_sequence(ctx.viewer, &target_obj, &target_indices)?;
    let target_ca_indices: Vec<FitAtom> = target_ca.iter().filter_map(|opt| *opt).collect();
    let target_ca_coords = extract_coords(ctx.viewer, &target_obj, &target_ca_indices)?;

    if target_ca_coords.is_empty() {
        return Err(CmdError::execution("No Cα atoms found in target selection"));
    }

    // Build CE params from named args
    let ce_params = CeParams {
        win_size: args.get_named_int("win_size").unwrap_or(8) as usize,
        gap_max: args.get_named_int("gap_max").unwrap_or(30) as usize,
        d0: args.get_named_float("d0").unwrap_or(3.0) as f32,
        d1: args.get_named_float("d1").unwrap_or(4.0) as f32,
        ..CeParams::default()
    };

    let mut aligned_count = 0usize;

    for (mobile_obj, mobile_indices) in &mobile_objects {
        if mobile_obj == &target_obj {
            continue;
        }

        let (_, mobile_ca) = extract_residue_sequence(ctx.viewer, mobile_obj, mobile_indices)?;
        let mobile_ca_indices: Vec<FitAtom> = mobile_ca.iter().filter_map(|opt| *opt).collect();
        let mobile_ca_coords = extract_coords(ctx.viewer, mobile_obj, &mobile_ca_indices)?;

        if mobile_ca_coords.is_empty() {
            if mobile_objects.len() == 1 {
                return Err(CmdError::execution("No Cα atoms found in mobile selection"));
            }
            continue;
        }

        let ce_result = ce_align(&mobile_ca_coords, &target_ca_coords, &ce_params)
            .map_err(|e: patinae_algos::AlignError| CmdError::execution(e.to_string()))?;

        if ce_result.pairs.len() < 3 {
            if mobile_objects.len() == 1 {
                return Err(CmdError::execution(format!(
                    "CE alignment found too few matching residues ({})",
                    ce_result.pairs.len()
                )));
            }
            continue;
        }

        // Extract aligned Cα coordinates for superposition
        let mut aligned_mobile: Vec<Vec3> = Vec::new();
        let mut aligned_target: Vec<Vec3> = Vec::new();
        let mut superpose_pairs: Vec<(usize, usize)> = Vec::new();
        for &(si, ti) in &ce_result.pairs {
            let idx = aligned_mobile.len();
            aligned_mobile.push(mobile_ca_coords[si]);
            aligned_target.push(target_ca_coords[ti]);
            superpose_pairs.push((idx, idx));
        }

        let result = superpose(&aligned_mobile, &aligned_target, &superpose_pairs, params)
            .map_err(|e: patinae_algos::AlignError| CmdError::execution(e.to_string()))?;

        apply_superpose_transform(ctx, mobile_obj, &result)?;
        if mobile_objects.len() > 1 {
            ctx.print(&format!(
                " Executive: \"{}\" RMSD = {:8.3}, {} CE pairs (Z-score: {:.1})",
                mobile_obj, result.final_rmsd, ce_result.n_aligned, ce_result.z_score
            ));
        } else {
            print_superpose_result(
                ctx,
                &result,
                params,
                &[format!(
                    "   CE alignment:    {} residue pairs (Z-score: {:.1})",
                    ce_result.n_aligned, ce_result.z_score
                )],
            );
        }
        aligned_count += 1;
    }

    if aligned_count == 0 {
        return Err(CmdError::selection(format!(
            "No mobile objects to align (target \"{}\" excluded)",
            target_obj
        )));
    }

    Ok(())
}

// ============================================================================
// Helper functions
// ============================================================================

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct FitAtom {
    index: AtomIndex,
    instance: Option<u32>,
}

/// Keeps copy identity when choosing atoms for a whole-object fit.
fn selected_fit_atoms(
    viewer: &dyn ViewerLike,
    selection: &str,
) -> CmdResult<Vec<(String, Vec<FitAtom>)>> {
    let mut objects = std::collections::BTreeMap::<String, Vec<FitAtom>>::new();
    for anchor in evaluate_atom_anchors(viewer, selection)? {
        objects
            .entry(anchor.object_name)
            .or_default()
            .push(FitAtom {
                index: anchor.atom_index,
                instance: anchor.instance,
            });
    }
    if objects.is_empty() {
        return Err(CmdError::selection(format!(
            "No atoms matching '{selection}'"
        )));
    }
    Ok(objects.into_iter().collect())
}

fn target_fit_atoms(viewer: &dyn ViewerLike, selection: &str) -> CmdResult<(String, Vec<FitAtom>)> {
    let mut objects = selected_fit_atoms(viewer, selection)?;
    if objects.len() != 1 {
        return Err(CmdError::invalid_arg(
            "target",
            "target must select atoms from a single object",
        ));
    }
    Ok(objects.remove(0))
}

/// Extract coordinates for selected atoms from a molecule object.
fn extract_coords(
    viewer: &dyn ViewerLike,
    obj_name: &str,
    indices: &[FitAtom],
) -> CmdResult<Vec<Vec3>> {
    let mol_obj = viewer
        .objects()
        .get_molecule(obj_name)
        .ok_or_else(|| CmdError::execution(format!("Object '{}' not found", obj_name)))?;
    let mut coords = Vec::with_capacity(indices.len());
    for &idx in indices {
        let v = mol_obj
            .instance_world_coord(idx.index, idx.instance)
            .ok_or_else(|| {
                CmdError::execution(format!("Missing coord for atom {}", idx.index.0))
            })?;
        coords.push(v);
    }
    Ok(coords)
}

/// Extract residue sequence and Cα atom indices for selected atoms.
///
/// Returns (sequence_chars, ca_indices) where ca_indices[i] is Some(AtomIndex) if
/// residue i has a Cα in the selection, None otherwise.
fn extract_residue_sequence(
    viewer: &dyn ViewerLike,
    obj_name: &str,
    selected_indices: &[FitAtom],
) -> CmdResult<(Vec<char>, Vec<Option<FitAtom>>)> {
    let mol_obj = viewer
        .objects()
        .get_molecule(obj_name)
        .ok_or_else(|| CmdError::execution(format!("Object '{}' not found", obj_name)))?;
    let mol = mol_obj.molecule();

    // Build a set of selected atom indices for fast lookup
    let selected_set: std::collections::HashSet<FitAtom> =
        selected_indices.iter().copied().collect();
    let copies: std::collections::BTreeSet<Option<u32>> =
        selected_indices.iter().map(|atom| atom.instance).collect();

    let mut sequence = Vec::new();
    let mut ca_indices = Vec::new();

    for instance in copies {
        for residue in mol.residues() {
            if !residue.is_protein() && !residue.is_nucleic() {
                continue;
            }

            // Check if any atom of this residue is in the selection
            let has_selected = residue
                .iter_indexed()
                .any(|(index, _)| selected_set.contains(&FitAtom { index, instance }));

            if !has_selected {
                continue;
            }

            let ch = residue_to_char(residue.resn());
            sequence.push(ch);

            // Find Cα (or C3' for nucleic) in the selection
            let ca = residue.ca().and_then(|(idx, _)| {
                let atom = FitAtom {
                    index: idx,
                    instance,
                };
                if selected_set.contains(&atom) {
                    Some(atom)
                } else {
                    None
                }
            });
            ca_indices.push(ca);
        }
    }

    Ok((sequence, ca_indices))
}

/// Apply the superposition transform to the mobile object and request a redraw.
fn apply_superpose_transform(
    ctx: &mut CommandContext<'_, '_, dyn ViewerLike + '_>,
    obj_name: &str,
    result: &SuperposeResult,
) -> CmdResult {
    let transform = build_transform_mat4(&result.transform.rotation, &result.transform.translation);
    apply_transform_to_object(ctx, obj_name, &transform)?;
    ctx.viewer.request_redraw();
    Ok(())
}

/// Print superposition results. `extra_lines` are method-specific lines
/// printed between the summary and the initial/final RMSD.
fn print_superpose_result(
    ctx: &mut CommandContext<'_, '_, dyn ViewerLike + '_>,
    result: &SuperposeResult,
    params: &SuperposeParams,
    extra_lines: &[String],
) {
    if ctx.quiet {
        return;
    }
    ctx.print(&format!(
        " Executive: RMSD = {:8.3}, {} to {} atoms, {} cycles",
        result.final_rmsd, result.n_aligned, result.n_aligned, result.cycles_performed
    ));
    for line in extra_lines {
        ctx.print(line);
    }
    if params.cycles > 0 {
        ctx.print(&format!("   Initial RMSD:    {:.3}", result.initial_rmsd));
        ctx.print(&format!(
            "   Final RMSD:      {:.3} ({} atoms after rejection of {})",
            result.final_rmsd, result.n_aligned, result.n_rejected
        ));
    }
}

/// Build a combined 4×4 transform matrix from rotation + translation.
///
/// The Mat4 from Kabsch has rotation in the 3×3 upper-left (column-major).
/// We embed the translation in column 3 (indices 12, 13, 14).
fn build_transform_mat4(rotation: &Mat4, translation: &Vec3) -> Mat4 {
    let r = &rotation.data;
    // Column-major: data[col*4 + row]
    Mat4::new([
        r[0],
        r[1],
        r[2],
        0.0, // col 0
        r[4],
        r[5],
        r[6],
        0.0, // col 1
        r[8],
        r[9],
        r[10],
        0.0, // col 2
        translation.x,
        translation.y,
        translation.z,
        1.0, // col 3
    ])
}

/// Apply a rigid-body transform to all atoms in an object (all states).
fn apply_transform_to_object(
    ctx: &mut CommandContext<'_, '_, dyn ViewerLike + '_>,
    obj_name: &str,
    transform: &Mat4,
) -> CmdResult {
    let mol_obj = ctx
        .viewer
        .objects_mut()
        .get_molecule_mut(obj_name)
        .ok_or_else(|| CmdError::execution(format!("Object '{}' not found", obj_name)))?;
    let current = mol_obj.state().transform.clone();
    if mol_obj.state().instances.is_some() || !mol_obj.molecule().assembly.definitions.is_empty() {
        // Keep the source and its assembly operators in the same local frame.
        // biounit inherits this object-to-world transform with the source snapshot.
        mol_obj
            .state_mut()
            .set_transform(transform.clone() * current);
        mol_obj.invalidate(DirtyFlags::COORDS);
    } else {
        let inverse = super::transform::inverse_object_transform(&current)
            .ok_or_else(|| CmdError::execution("Object transform is singular"))?;
        mol_obj
            .molecule_mut()
            .transform_all_states(&(inverse * transform.clone() * current));
    }
    Ok(())
}
