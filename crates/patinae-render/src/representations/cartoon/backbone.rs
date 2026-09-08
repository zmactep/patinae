//! CPU backbone extraction for the cartoon representation.
//!
//! Walks `chains() → polymer_subchains() → residues()`, keeps eligible
//! protein/nucleic residues with a backbone reference atom, and emits one
//! [`BackboneAtom`] per residue. Per-atom representation bits are intentionally
//! ignored here; the WGSL visibility gate decides whether retained geometry is
//! drawn. Chain breaks (resv jumps larger than `gap_cutoff` or transitions into
//! a new polymer subchain) are flagged on the *trailing* atom (`SEG_END`) and on
//! the *leading* atom of the next run (`SEG_START`). The compute chain reads
//! these flags to clamp the smoothing / sampling stencils so that disjoint
//! segments do not pull on each other.
//!
//! Both protein (Cα) and nucleic acid (P / C3' / C1') polymers are
//! extracted. Per-residue reference atom + orientation are picked by
//! kind:
//!   - Protein: Cα position + (Cα→O, fallback Cα→C) orientation;
//!     SS tag inherited from `atom.ss_type` (Helix/Sheet/Loop/…).
//!   - Nucleic: P (or C3'/C1' fallback) position + (ref→C1', fallback
//!     ref→C4') orientation; SS tag forced to `NucleicRibbon` so
//!     `cartoon_type_for` maps to `CartoonType::NucleicRect`.
//!
//! The downstream protein-specific refinement passes (helix-axis,
//! sheet-flatten, sheet-arrow tagging) are SS-gated by Helix/Sheet
//! numeric codes and skip NucleicRibbon residues. `parallel_transport`
//! still runs and gives a stable frame for the nucleic tube.

use bytemuck::{Pod, Zeroable};
use patinae_mol::{AtomIndex, CoordSet, ObjectMolecule, ResidueView, SecondaryStructure};

/// `BackboneAtom.flags` bit layout. Low 4 bits hold `SecondaryStructure`
/// (`SecondaryStructure as u8 & 0xF`), upper bits hold segment markers.
pub mod flags {
    pub const SS_MASK: u32 = 0x0000_000F;
    pub const SEG_START: u32 = 1 << 4;
    pub const SEG_END: u32 = 1 << 5;
    /// Last sheet residue of a strand run. The pair starting at this residue
    /// renders as a tapering arrow head instead of a uniform-width rectangle.
    pub const SHEET_ARROW: u32 = 1 << 6;
}

/// Per-residue backbone sample uploaded to the smoothing/sampling compute
/// chain. 32 B aligned for `vec4` storage layout in WGSL.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct BackboneAtom {
    /// CA position in object-space.
    pub position: [f32; 3],
    /// Atom index of the CA — flows through the compute chain into every
    /// extruded vertex's `group_id`, so picking + material LUT both keyed
    /// per-residue without any extra book-keeping.
    pub atom_id: u32,
    /// CA→O (or CA→C / default) direction. Used as the normal seed for the
    /// first frame in each segment; downstream samples parallel-transport
    /// from there.
    pub orientation: [f32; 3],
    /// `flags::SS_MASK` + `SEG_START` / `SEG_END` markers. See [`flags`].
    pub flags: u32,
}

impl BackboneAtom {
    pub const SIZE: u64 = std::mem::size_of::<Self>() as u64;
}

const _: () = assert!(std::mem::size_of::<BackboneAtom>() == 32);

/// Extract one [`BackboneAtom`] per eligible polymer residue, walking polymer
/// subchains and marking chain breaks.
///
/// A `(resv_next - resv_prev).abs() > gap_cutoff` jump within the same
/// polymer subchain promotes the boundary to a chain break.
pub fn extract_backbone(
    molecule: &ObjectMolecule,
    coord_set: &CoordSet,
    gap_cutoff: i32,
) -> Vec<BackboneAtom> {
    extract_retained_backbone(molecule, coord_set, gap_cutoff)
}

/// Extract retained backbone samples for cartoon and ribbon.
///
/// This builds the same retained backbone superset for both modes. Per-mode
/// visibility is handled by the scene atom LUT in WGSL.
pub fn extract_retained_backbone(
    molecule: &ObjectMolecule,
    coord_set: &CoordSet,
    gap_cutoff: i32,
) -> Vec<BackboneAtom> {
    extract_filtered_backbone(molecule, coord_set, gap_cutoff, &|_| true)
}

/// Filters source residues before chain smoothing and geometry generation.
pub(crate) fn extract_filtered_backbone(
    molecule: &ObjectMolecule,
    coord_set: &CoordSet,
    gap_cutoff: i32,
    includes: &dyn Fn(u32) -> bool,
) -> Vec<BackboneAtom> {
    let mut out: Vec<BackboneAtom> = Vec::new();

    for chain in molecule.chains() {
        for subchain in chain.polymer_subchains() {
            let mut prev_resv: Option<i32> = None;
            // Index in `out` of the most recent push for this subchain — used
            // to retroactively stamp `SEG_END` when we detect a gap.
            let mut last_idx_in_run: Option<usize> = None;
            let mut start_of_run = true;

            for residue in subchain.residues() {
                // PolymerSubchain iterator narrows to biopolymer residues;
                // sample_residue dispatches protein vs nucleic.
                let sample = match sample_residue(&residue, coord_set) {
                    Some(s) => s,
                    None => {
                        if residue
                            .iter_indexed()
                            .any(|(index, _)| !includes(index.as_u32()))
                        {
                            if let Some(index) = last_idx_in_run {
                                out[index].flags |= flags::SEG_END;
                            }
                            start_of_run = true;
                            prev_resv = None;
                        }
                        continue;
                    }
                };

                // Detect chain break against the previous sample in this run.
                if let (Some(prev), Some(idx)) = (prev_resv, last_idx_in_run) {
                    if (residue.resv() - prev).abs() > gap_cutoff {
                        out[idx].flags |= flags::SEG_END;
                        start_of_run = true;
                    }
                }

                let mut f = sample.ss;
                if start_of_run {
                    f |= flags::SEG_START;
                    start_of_run = false;
                }

                out.push(BackboneAtom {
                    position: [sample.position.x, sample.position.y, sample.position.z],
                    atom_id: sample.atom_id.as_u32(),
                    orientation: [
                        sample.orientation.x,
                        sample.orientation.y,
                        sample.orientation.z,
                    ],
                    flags: f,
                });
                last_idx_in_run = Some(out.len() - 1);
                prev_resv = Some(residue.resv());
            }

            // Close the trailing run.
            if let Some(idx) = last_idx_in_run {
                out[idx].flags |= flags::SEG_END;
            }
        }
    }

    // Four-pass orientation refinement:
    //
    // 1. Consistency: flip any residue whose CA→O points opposite its
    //    predecessor. CA→O can flip 180° at register breaks; without this
    //    the cross-section frame flips with it and the ribbon folds.
    //
    // 2. Round-helix axis tracking: for runs of 4+ helix CAs, override the
    //    raw CA→O with `cross(helix_axis, tangent)` so the orientation points
    //    radially outward from the helix axis. Adjacent residues then rotate
    //    ~100° per step around the axis (matching the helix turn), which is
    //    what makes the oval profile (1.2 × 0.25) read as a smooth helical
    //    twist rather than a kinked flat ribbon. Mirrors
    //    `compute_round_helix_orientations`.
    //
    // 3. Parallel-transport: walk each segment, projecting the running normal
    //    onto each residue's local tangent plane — but ONLY through residues
    //    that round_helix did not refine. The parallel-transport pass is what
    //    keeps loop / sheet sections from twisting between adjacent residues;
    //    on helix runs the round-helix orientations are correct as-is and
    //    parallel transport would rotate them off-axis.
    //
    // 4. SS-boundary smoothing: blend the loop residue immediately adjacent
    //    to a helix terminus toward the helix orientation. This prevents the
    //    cross-section frame from rotating abruptly at the helix exit.
    enforce_orientation_consistency(&mut out);
    let helix_locked = compute_round_helix_orientations(&mut out);
    // refine_normals (4 steps) runs BEFORE flatten_sheet_positions so sheet
    // ENDPOINTS get their orientations Gram-Schmidt'd and greedy-selected
    // against neighbours. Without this, endpoint orient may be near-tangent
    // (raw CA→O on a pleated strand), GPU finalize_frames falls back to an
    // arbitrary direction and the rectangle profile twists ~90° along the
    // strand. Also catches micro-flips on pleated CA→O via Step 3 greedy.
    refine_normals(&mut out, &helix_locked);
    flatten_sheet_positions(&mut out, FLAT_CYCLES);
    let sheet_locked = flatten_sheet_orientations(&out);
    let mut locked = helix_locked.clone();
    for (i, s) in sheet_locked.iter().enumerate() {
        locked[i] = locked[i] || *s;
    }
    parallel_transport_non_helix(&mut out, &locked);
    smooth_ss_boundary_orientations(&mut out);
    tag_sheet_arrows(&mut out);

    out
}

/// Number of Laplacian iterations
/// applied to β-sheet CA positions to flatten the natural pleated zig-zag into
/// a smooth strand plane.
const FLAT_CYCLES: u32 = 4;

/// Tag the SECOND-TO-LAST residue of each contiguous sheet run with
/// `SHEET_ARROW`. The pair starting at this residue (i.e. the LAST
/// intra-sheet pair `(sheet[N-2], sheet[N-1])`) is the arrow head — it
/// tapers width from `sheet_radius * arrow_tip_scale` at the base
/// (sheet[N-2]) to 0 at the tip (sheet[N-1]). The boundary pair
/// `(sheet[N-1], first_loop)` stays a normal SS-mismatch pair (pinned to
/// LOOP) so the arrow tip lands ON the last sheet residue's position
/// instead of extending into the loop region.
///
/// For runs of length < 3 the tag is omitted because a 2-residue run has only
/// one intra-sheet pair and cannot be split into body and arrow.
fn tag_sheet_arrows(bb: &mut [BackboneAtom]) {
    let n = bb.len();
    let mut i = 0usize;
    while i < n {
        if (bb[i].flags & flags::SS_MASK) != 2 {
            i += 1;
            continue;
        }
        // Walk to the end of this sheet run.
        let run_start = i;
        let mut run_end = i;
        loop {
            let last_in_run = run_end + 1 >= n
                || (bb[run_end].flags & flags::SEG_END) != 0
                || (bb[run_end + 1].flags & flags::SS_MASK) != 2;
            if last_in_run {
                break;
            }
            run_end += 1;
        }
        let run_len = run_end - run_start + 1;
        if run_len >= 3 {
            // Tag sheet[N-2] = run_end - 1 so pair (run_end - 1, run_end)
            // becomes the arrow.
            bb[run_end - 1].flags |= flags::SHEET_ARROW;
        }
        i = run_end + 1;
    }
}

/// Walk each `[SEG_START .. SEG_END]` run in `bb`, projecting the running
/// normal onto each residue's local tangent plane to keep the (normal,
/// binormal) frame rotating minimally between adjacent residues.
///
/// `helix_locked[i] = true` means residue `i` already received an orientation
/// from `compute_round_helix_orientations` and must not be overwritten —
/// instead it serves as an anchor that re-seeds the running normal so the
/// loop/turn residues that follow are continuous with the helix exit frame.
fn parallel_transport_non_helix(bb: &mut [BackboneAtom], helix_locked: &[bool]) {
    use lin_alg::f32::Vec3;
    let n = bb.len();
    if n == 0 {
        return;
    }
    let positions: Vec<Vec3> = bb
        .iter()
        .map(|a| Vec3::new(a.position[0], a.position[1], a.position[2]))
        .collect();
    let raw_orient: Vec<Vec3> = bb
        .iter()
        .map(|a| Vec3::new(a.orientation[0], a.orientation[1], a.orientation[2]))
        .collect();

    let tangent_at = |j: usize, run_start: usize, run_end: usize| -> Vec3 {
        if run_start == run_end {
            Vec3::new(0.0, 0.0, 1.0)
        } else if j == run_start {
            let t = positions[j + 1] - positions[j];
            let l = t.magnitude();
            if l > 1e-6 {
                t / l
            } else {
                Vec3::new(0.0, 0.0, 1.0)
            }
        } else if j == run_end {
            let t = positions[j] - positions[j - 1];
            let l = t.magnitude();
            if l > 1e-6 {
                t / l
            } else {
                Vec3::new(0.0, 0.0, 1.0)
            }
        } else {
            let t = positions[j + 1] - positions[j - 1];
            let l = t.magnitude();
            if l > 1e-6 {
                t / l
            } else {
                Vec3::new(0.0, 0.0, 1.0)
            }
        }
    };

    let mut i = 0;
    while i < n {
        let mut end = i;
        while end < n && (bb[end].flags & flags::SEG_END) == 0 {
            end += 1;
        }
        if end >= n {
            end = n - 1;
        }
        let t0 = tangent_at(i, i, end);
        let mut normal = ortho_unit(raw_orient[i], t0);
        if normal.magnitude_squared() < 1e-8 {
            let axis = if t0.y.abs() > 0.9 {
                Vec3::new(1.0, 0.0, 0.0)
            } else {
                Vec3::new(0.0, 1.0, 0.0)
            };
            normal = ortho_unit(axis, t0);
        }
        if !helix_locked[i] {
            bb[i].orientation = [normal.x, normal.y, normal.z];
        }
        for j in (i + 1)..=end {
            let tj = tangent_at(j, i, end);
            if helix_locked[j] {
                // Helix anchor: re-seed running normal from the locked
                // orientation so subsequent loop residues continue from the
                // helix exit frame.
                let locked = raw_orient[j];
                let projected = ortho_unit(locked, tj);
                if projected.magnitude_squared() > 1e-8 {
                    normal = projected;
                }
                continue;
            }
            let projected = ortho_unit(normal, tj);
            normal = if projected.magnitude_squared() > 1e-8 {
                projected
            } else {
                ortho_unit(raw_orient[j], tj)
            };
            if normal.magnitude_squared() < 1e-8 {
                let axis = if tj.y.abs() > 0.9 {
                    Vec3::new(1.0, 0.0, 0.0)
                } else {
                    Vec3::new(0.0, 1.0, 0.0)
                };
                normal = ortho_unit(axis, tj);
            }
            bb[j].orientation = [normal.x, normal.y, normal.z];
        }
        i = end + 1;
    }
}

/// Sliding-window helix-axis orientation refinement. Returns a per-residue
/// boolean mask: `true` means the residue's orientation was overwritten with
/// `cross(helix_axis, local_tangent)` and downstream passes must not touch
/// it. Mirrors `compute_round_helix_orientations`.
///
/// Algorithm: maintain a sliding window of the last five helix CAs in this
/// segment. When four are seen, compute a weighted center
/// `0.2130*(v1+v4) + 0.2870*(v2+v3)` (weights derived from α-helix geometry).
/// The helix axis is the direction from the previous center to the current
/// one. Each helix residue's orientation is set to `axis × tangent` —
/// perpendicular to both the axis and the local tangent, i.e. radially
/// outward from the helix axis at that residue.
fn compute_round_helix_orientations(bb: &mut [BackboneAtom]) -> Vec<bool> {
    use lin_alg::f32::Vec3;
    let n = bb.len();
    let mut locked = vec![false; n];
    if n < 4 {
        return locked;
    }

    // Per-residue tangent estimate (averaged neighbour differences). Segment
    // boundaries clamp the stencil so we never average across a chain break.
    let positions: Vec<Vec3> = bb
        .iter()
        .map(|a| Vec3::new(a.position[0], a.position[1], a.position[2]))
        .collect();
    let mut tv: Vec<Vec3> = vec![Vec3::new(0.0, 0.0, 1.0); n];
    for i in 0..n {
        let prev_break = i == 0 || (bb[i - 1].flags & flags::SEG_END) != 0;
        let next_break = i + 1 >= n || (bb[i].flags & flags::SEG_END) != 0;
        let t = if prev_break && !next_break {
            positions[i + 1] - positions[i]
        } else if next_break && !prev_break {
            positions[i] - positions[i - 1]
        } else if !prev_break && !next_break {
            positions[i + 1] - positions[i - 1]
        } else {
            Vec3::new(0.0, 0.0, 1.0)
        };
        let l = t.magnitude();
        tv[i] = if l > 1e-6 {
            t / l
        } else {
            Vec3::new(0.0, 0.0, 1.0)
        };
    }

    let is_helix_ss = |ss: u32| matches!(ss, 1 | 3 | 4);

    // Walk segments independently — the sliding window must not cross a
    // chain break.
    let mut seg_start = 0usize;
    while seg_start < n {
        let mut seg_end = seg_start;
        while seg_end < n && (bb[seg_end].flags & flags::SEG_END) == 0 {
            seg_end += 1;
        }
        if seg_end >= n {
            seg_end = n - 1;
        }

        // Sliding window of the last five helix CA indices in this segment,
        // newest first.
        let mut win: [Option<usize>; 5] = [None; 5];
        let mut prev_center = Vec3::new(0.0, 0.0, 0.0);
        let mut centers_seen = 0i32;
        let mut helix_run_open = false;

        let mut a = seg_start;
        loop {
            // Shift window
            for k in (1..5).rev() {
                win[k] = win[k - 1];
            }
            let ss_a = if a <= seg_end {
                bb[a].flags & flags::SS_MASK
            } else {
                0
            };
            let in_helix = a <= seg_end && is_helix_ss(ss_a);

            if in_helix {
                win[0] = Some(a);
                helix_run_open = true;
            } else {
                // Helix run terminated (or never started).
                if helix_run_open && centers_seen < 2 {
                    // Short helix (<5 CAs): synthesise an axis from chain-
                    // direction vectors,
                    if let (Some(i2), Some(i3)) = (win[1], win[2]) {
                        let mut t0 = if a <= seg_end {
                            normalize_safe_vec(positions[i2] - positions[a])
                        } else {
                            Vec3::new(0.0, 0.0, 0.0)
                        };
                        let t1 = normalize_safe_vec(positions[i3] - positions[i2]);
                        t0 += t1;
                        if let Some(i4) = win[3] {
                            t0 += normalize_safe_vec(positions[i4] - positions[i3]);
                        }
                        if let (Some(i4), Some(i5)) = (win[3], win[4]) {
                            t0 += normalize_safe_vec(positions[i5] - positions[i4]);
                        }
                        let axis = normalize_safe_vec(t0);
                        if axis.magnitude_squared() > 1e-8 {
                            for &maybe in &win[1..5] {
                                if let Some(idx) = maybe {
                                    let o = normalize_safe_vec(axis.cross(tv[idx]));
                                    if o.magnitude_squared() > 1e-8 {
                                        bb[idx].orientation = [o.x, o.y, o.z];
                                        locked[idx] = true;
                                    }
                                }
                            }
                            // If the first helix residue points opposite the
                            // next refined residue, flip it for continuity.
                            if let (Some(i4), Some(i5)) = (win[3], win[4]) {
                                let o4 = Vec3::new(
                                    bb[i4].orientation[0],
                                    bb[i4].orientation[1],
                                    bb[i4].orientation[2],
                                );
                                let o5 = Vec3::new(
                                    bb[i5].orientation[0],
                                    bb[i5].orientation[1],
                                    bb[i5].orientation[2],
                                );
                                if o4.dot(o5) < -0.8 {
                                    bb[i5].orientation = [-o5.x, -o5.y, -o5.z];
                                }
                            }
                        }
                    }
                }
                win = [None; 5];
                helix_run_open = false;
                centers_seen = 0;
            }

            // When four helix CAs are in window, compute helix center + axis.
            if let (Some(i1), Some(i2), Some(i3), Some(i4)) = (win[0], win[1], win[2], win[3]) {
                let center = (positions[i1] + positions[i4]) * 0.2130
                    + (positions[i2] + positions[i3]) * 0.2870;
                if centers_seen > 0 {
                    let axis = normalize_safe_vec(prev_center - center);
                    if axis.magnitude_squared() > 1e-8 {
                        // Set orientations for the 3 newest residues (i1, i2, i3).
                        for &idx in &[i1, i2, i3] {
                            let o = normalize_safe_vec(axis.cross(tv[idx]));
                            if o.magnitude_squared() > 1e-8 {
                                bb[idx].orientation = [o.x, o.y, o.z];
                                locked[idx] = true;
                            }
                        }
                        // First time we have a 5th CA, also lock the start of
                        // the helix (i4 and beyond).
                        if centers_seen == 1 {
                            for &maybe in &[win[3], win[4]] {
                                if let Some(idx) = maybe {
                                    let o = normalize_safe_vec(axis.cross(tv[idx]));
                                    if o.magnitude_squared() > 1e-8 {
                                        bb[idx].orientation = [o.x, o.y, o.z];
                                        locked[idx] = true;
                                    }
                                }
                            }
                        }
                    }
                }
                centers_seen += 1;
                prev_center = center;
            }

            if a == seg_end {
                // Force one final iteration with `a` past seg_end so the
                // helix-run-terminated branch flushes any open window.
                if helix_run_open {
                    // Shift window again
                    for k in (1..5).rev() {
                        win[k] = win[k - 1];
                    }
                    win[0] = None;
                    if centers_seen < 2 {
                        if let (Some(i2), Some(i3)) = (win[1], win[2]) {
                            let t1 = normalize_safe_vec(positions[i3] - positions[i2]);
                            let mut t0 = t1;
                            if let Some(i4) = win[3] {
                                t0 += normalize_safe_vec(positions[i4] - positions[i3]);
                            }
                            if let (Some(i4), Some(i5)) = (win[3], win[4]) {
                                t0 += normalize_safe_vec(positions[i5] - positions[i4]);
                            }
                            let axis = normalize_safe_vec(t0);
                            if axis.magnitude_squared() > 1e-8 {
                                for &maybe in &win[1..5] {
                                    if let Some(idx) = maybe {
                                        let o = normalize_safe_vec(axis.cross(tv[idx]));
                                        if o.magnitude_squared() > 1e-8 {
                                            bb[idx].orientation = [o.x, o.y, o.z];
                                            locked[idx] = true;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                break;
            }
            a += 1;
        }

        seg_start = seg_end + 1;
    }

    locked
}

/// Walk each contiguous β-sheet run and mark every member residue as
/// "locked" so downstream `parallel_transport_non_helix` skips them. This
/// function is intentionally read-only on orientations — the actual
/// orientation flattening is performed inside `flatten_sheet_positions`,
/// which iterates 4 cycles of (3-window Laplacian) + (orthogonalize against
/// the locally-recomputed tangent). A previous version of this function
/// also overwrote orientations with the strand's GLOBAL mean projected onto
/// each residue's local tangent — that introduced a visible twist on curved
/// strands (the projected direction rotates with the tangent, giving a
/// small per-residue rotation around the chain that adds up across the
/// strand). The local orthogonalisation in `flatten_sheet_positions` does
/// not have that drift because every cycle re-uses the residue's own
/// tangent.
fn flatten_sheet_orientations(bb: &[BackboneAtom]) -> Vec<bool> {
    let n = bb.len();
    let mut locked = vec![false; n];
    for (i, atom) in bb.iter().enumerate() {
        if (atom.flags & flags::SS_MASK) == 2 {
            locked[i] = true;
        }
    }
    locked
}

/// Iterative window-1 Laplacian smoothing of CA positions in each contiguous
/// β-sheet run. Each cycle averages the three-residue stencil
/// `(p[b-1] + p[b] + p[b+1]) / 3` for every interior `b`, then re-orthogonalises
/// orientations against the tangent recomputed from the smoothed positions.
/// First and last residue of each run are preserved.
///
/// Without this the rectangle profile inherits the natural CA pleated zig-zag
/// and reads as a creased strand with multiple visible facets at the
/// arrow terminus.
fn flatten_sheet_positions(bb: &mut [BackboneAtom], cycles: u32) {
    use lin_alg::f32::Vec3;
    let n = bb.len();
    if n < 3 || cycles == 0 {
        return;
    }
    let mut i = 0usize;
    while i < n {
        let ss_i = bb[i].flags & flags::SS_MASK;
        if ss_i != 2 {
            i += 1;
            continue;
        }
        // Walk to the end of this contiguous sheet run.
        let mut j = i;
        loop {
            let last_in_run = j + 1 >= n
                || (bb[j].flags & flags::SEG_END) != 0
                || (bb[j + 1].flags & flags::SS_MASK) != 2;
            if last_in_run {
                break;
            }
            j += 1;
        }
        // Need at least 3 residues to have an interior to smooth.
        if j >= i + 2 {
            let run_len = j - i + 1;
            for _ in 0..cycles {
                let snap_pos: Vec<Vec3> = (i..=j)
                    .map(|k| Vec3::new(bb[k].position[0], bb[k].position[1], bb[k].position[2]))
                    .collect();
                let snap_orient: Vec<Vec3> = (i..=j)
                    .map(|k| {
                        Vec3::new(
                            bb[k].orientation[0],
                            bb[k].orientation[1],
                            bb[k].orientation[2],
                        )
                    })
                    .collect();
                // Average positions + orientations on interior with window=1.
                let mut new_pos = snap_pos.clone();
                let mut new_orient = snap_orient.clone();
                for b in 1..run_len - 1 {
                    new_pos[b] = (snap_pos[b - 1] + snap_pos[b] + snap_pos[b + 1]) / 3.0;
                    new_orient[b] =
                        (snap_orient[b - 1] + snap_orient[b] + snap_orient[b + 1]) / 3.0;
                }
                // Apply positions first so the tangent recomputation below sees
                // the smoothed positions.
                for b in 1..run_len - 1 {
                    bb[i + b].position = [new_pos[b].x, new_pos[b].y, new_pos[b].z];
                }
                // Re-orthogonalise orientations against the new tangents.
                for b in 1..run_len - 1 {
                    let tangent = normalize_safe_vec(new_pos[b + 1] - new_pos[b - 1]);
                    let removed = new_orient[b] - tangent * new_orient[b].dot(tangent);
                    let l = removed.magnitude();
                    if l > 1e-6 {
                        let no = removed / l;
                        bb[i + b].orientation = [no.x, no.y, no.z];
                    }
                }
            }
        }
        i = j + 1;
    }
}

/// Refines normals through four sequential steps applied to interior residues
/// `1..n-1`:
///
/// 1. **Gram-Schmidt** — make `orient ⊥ tangent`. Removes any tangent
///    component from the seed CA→O. CRITICAL for sheet endpoints whose raw
///    CA→O on a pleated strand may be near-parallel to the strand axis;
///    without this step the GPU `cartoon_finalize_frames` pass computes
///    `B = T × O ≈ 0` and falls back to an arbitrary direction, producing a
///    visible 90° twist of the rectangle profile along the strand.
/// 2. **Alternatives** — for each residue, candidates = `[orient, alt]`.
///    Helices (`helix_locked[i] == true`) keep `alt = orient` (no inversion;
///    round-helix orientations are by design CCW/CW pattern). Sheets and
///    loops get `alt = -orient`.
/// 3. **Greedy forward selection** — project both candidates perpendicular
///    to the chain direction `nv` (normalised forward difference of CA
///    positions), pick the one with maximum dot product to the previous
///    residue's projected orient. Catches micro-flips in raw CA→O on
///    pleated strands that `enforce_orientation_consistency` may miss.
/// 4. **Kink softening** — when `dot(prev,curr) * dot(curr,next) < -0.10`,
///    blend `curr` toward the neighbour-averaged direction with
///    `blend = clamp(2*(-0.10 - dp), 0, 1)`.
///
/// Skips across segment breaks (neighbours on the other side of `SEG_END`
/// are unrelated, their dot products would be noise).
fn refine_normals(bb: &mut [BackboneAtom], helix_locked: &[bool]) {
    use lin_alg::f32::Vec3;
    let n = bb.len();
    if n < 3 {
        return;
    }
    let positions: Vec<Vec3> = bb
        .iter()
        .map(|a| Vec3::new(a.position[0], a.position[1], a.position[2]))
        .collect();
    let seg_end: Vec<bool> = bb.iter().map(|a| (a.flags & flags::SEG_END) != 0).collect();

    // Tangent: averaged neighbour difference, segment-aware.
    let tangent_at = |i: usize| -> Vec3 {
        let prev_break = i == 0 || seg_end[i - 1];
        let next_break = i + 1 >= n || seg_end[i];
        let d = if prev_break && !next_break {
            positions[i + 1] - positions[i]
        } else if next_break && !prev_break {
            positions[i] - positions[i - 1]
        } else if !prev_break && !next_break {
            positions[i + 1] - positions[i - 1]
        } else {
            Vec3::new(0.0, 0.0, 1.0)
        };
        normalize_safe_vec(d)
    };

    // Forward chain direction: `nv[i] = normalize(positions[i+1] - positions[i])`.
    // Used only for projection in Step 3 — for residue `i`, reference uses
    // `nv[i-1]` (the chord arriving at `i`).
    let nv_back_at = |i: usize| -> Vec3 {
        if i == 0 || seg_end[i - 1] {
            // Fall back to forward chord at chain start.
            if i + 1 < n && !seg_end[i] {
                return normalize_safe_vec(positions[i + 1] - positions[i]);
            }
            return Vec3::new(0.0, 0.0, 1.0);
        }
        normalize_safe_vec(positions[i] - positions[i - 1])
    };

    // Step 1: orthogonalise interior orientations against tangent.
    let mut orient: Vec<Vec3> = bb
        .iter()
        .map(|a| Vec3::new(a.orientation[0], a.orientation[1], a.orientation[2]))
        .collect();
    for i in 1..n - 1 {
        if seg_end[i - 1] || seg_end[i] {
            continue;
        }
        let t = tangent_at(i);
        let removed = orient[i] - t * orient[i].dot(t);
        let l = removed.magnitude();
        if l > 1e-6 {
            orient[i] = removed / l;
        }
    }

    // Step 2: build alternatives. Helix-locked residues get `alt = orient`
    // (no inversion). Sheets and loops get the inverted candidate.
    let mut alternatives: Vec<[Vec3; 2]> = Vec::with_capacity(n);
    for i in 0..n {
        let alt = if helix_locked[i] {
            orient[i]
        } else {
            -orient[i]
        };
        alternatives.push([orient[i], alt]);
    }

    // Step 3: greedy forward selection. For each interior residue, project
    // both candidates and the previous orient perpendicular to the chain
    // direction (`nv_back`), pick whichever has the larger dot product to
    // the previous projected orient.
    for i in 1..n - 1 {
        if seg_end[i - 1] {
            // Chain-start at this residue — keep candidate 0 (the original
            // orient after Step 1).
            orient[i] = alternatives[i][0];
            continue;
        }
        let chain_dir = nv_back_at(i);

        let prev_o = orient[i - 1];
        let o_prev_proj = prev_o - chain_dir * prev_o.dot(chain_dir);
        let l_prev = o_prev_proj.magnitude();
        let o_prev_n = if l_prev > 1e-6 {
            o_prev_proj / l_prev
        } else {
            prev_o
        };

        let c0 = alternatives[i][0];
        let p0 = c0 - chain_dir * c0.dot(chain_dir);
        let l0 = p0.magnitude();
        let p0_n = if l0 > 1e-6 { p0 / l0 } else { c0 };

        let c1 = alternatives[i][1];
        let p1 = c1 - chain_dir * c1.dot(chain_dir);
        let l1 = p1.magnitude();
        let p1_n = if l1 > 1e-6 { p1 / l1 } else { c1 };

        let dot0 = o_prev_n.dot(p0_n);
        let dot1 = o_prev_n.dot(p1_n);
        orient[i] = if dot1 > dot0 { c1 } else { c0 };
    }

    // Step 4: detect and soften kinks. Snapshot Step 3's output so each
    // residue's decision uses the post-Step-3 neighbour values, not values
    // already softened by an earlier loop iteration.
    let snap = orient.clone();
    for i in 1..n - 1 {
        if seg_end[i - 1] || seg_end[i] {
            continue;
        }
        let curr = snap[i];
        let prev = snap[i - 1];
        let next = snap[i + 1];
        let dp = curr.dot(prev) * curr.dot(next);
        if dp >= -0.10 {
            continue;
        }
        let t = tangent_at(i);
        let mut t0 = next + prev + curr * 0.001;
        t0 = t0 - t * t0.dot(t);
        let t0_len = t0.magnitude();
        if t0_len <= 1e-6 {
            continue;
        }
        let t0_norm = t0 / t0_len;
        let candidate = if curr.dot(t0_norm) < 0.0 {
            curr - t0_norm
        } else {
            curr + t0_norm
        };
        let candidate = normalize_safe_vec(candidate);
        if candidate.magnitude_squared() < 1e-8 {
            continue;
        }
        let blend = (2.0 * (-0.10 - dp)).clamp(0.0, 1.0);
        let blended = curr * (1.0 - blend) + candidate * blend;
        let bn = normalize_safe_vec(blended);
        if bn.magnitude_squared() > 1e-8 {
            orient[i] = bn;
        }
    }

    // Write all interior orientations back to `bb`.
    for i in 1..n - 1 {
        bb[i].orientation = [orient[i].x, orient[i].y, orient[i].z];
    }
}

fn normalize_safe_vec(v: lin_alg::f32::Vec3) -> lin_alg::f32::Vec3 {
    let l = v.magnitude();
    if l > 1e-6 {
        v / l
    } else {
        lin_alg::f32::Vec3::new(0.0, 0.0, 0.0)
    }
}

fn ortho_unit(v: lin_alg::f32::Vec3, axis: lin_alg::f32::Vec3) -> lin_alg::f32::Vec3 {
    let projected = v - axis * v.dot(axis);
    let l = projected.magnitude();
    if l > 1e-6 {
        projected / l
    } else {
        lin_alg::f32::Vec3::new(0.0, 0.0, 0.0)
    }
}

/// Within each segment, flip the orientation of any residue whose seed points
/// opposite to its predecessor (`prev · curr < 0`). The CA→O direction can
/// flip 180° at register breaks, sheet boundaries, or where a planar peptide
/// happens to face "inward" — without this pass the cross-section frame
/// flips with it and the ribbon folds back on itself.
fn enforce_orientation_consistency(bb: &mut [BackboneAtom]) {
    use lin_alg::f32::Vec3;
    let n = bb.len();
    if n < 2 {
        return;
    }
    let mut i = 0;
    while i < n {
        let mut end = i;
        while end < n && (bb[end].flags & flags::SEG_END) == 0 {
            end += 1;
        }
        if end >= n {
            end = n - 1;
        }
        for j in (i + 1)..=end {
            let prev = Vec3::new(
                bb[j - 1].orientation[0],
                bb[j - 1].orientation[1],
                bb[j - 1].orientation[2],
            );
            let cur = Vec3::new(
                bb[j].orientation[0],
                bb[j].orientation[1],
                bb[j].orientation[2],
            );
            if prev.dot(cur) < 0.0 {
                bb[j].orientation = [-cur.x, -cur.y, -cur.z];
            }
        }
        i = end + 1;
    }
}

/// At every helix↔loop transition within a segment, blend the loop residues
/// immediately adjacent to the helix toward the helix's terminal orientation.
/// Mirrors `smooth_helices` (the C-terminal
/// blend uses 0.6/0.4 = helix end / loop residue). Without this the loop's
/// own CA→O often differs sharply from the helix's coiled orientation, and
/// the cross-section frame rotates rapidly in a single sample step,
/// producing the visible "fold" at the helix exit.
fn smooth_ss_boundary_orientations(bb: &mut [BackboneAtom]) {
    use lin_alg::f32::Vec3;
    let n = bb.len();
    if n < 2 {
        return;
    }
    let is_helix_ss = |ss: u32| matches!(ss, 1 | 3 | 4);
    // Snapshot orientations so the blends use original loop seeds, not
    // values that have already been overwritten by an earlier blend.
    let snap: Vec<Vec3> = bb
        .iter()
        .map(|a| Vec3::new(a.orientation[0], a.orientation[1], a.orientation[2]))
        .collect();
    for i in 0..n {
        let ss_cur = bb[i].flags & flags::SS_MASK;
        let in_helix = is_helix_ss(ss_cur);
        if !in_helix {
            continue;
        }

        // C-terminus: blend next residue (if loop in same segment) toward us.
        if (bb[i].flags & flags::SEG_END) == 0 && i + 1 < n {
            let ss_next = bb[i + 1].flags & flags::SS_MASK;
            if !is_helix_ss(ss_next) && ss_next != 2 {
                let helix = snap[i];
                let mut loop_o = snap[i + 1];
                if helix.dot(loop_o) < 0.0 {
                    loop_o = -loop_o;
                }
                let blended = helix * 0.6 + loop_o * 0.4;
                let l = blended.magnitude();
                if l > 1e-6 {
                    let n_o = blended / l;
                    bb[i + 1].orientation = [n_o.x, n_o.y, n_o.z];
                }
            }
        }
        // N-terminus: blend previous residue (if loop in same segment) toward us.
        if (bb[i].flags & flags::SEG_START) == 0 && i > 0 {
            let ss_prev = bb[i - 1].flags & flags::SS_MASK;
            if !is_helix_ss(ss_prev) && ss_prev != 2 {
                let helix = snap[i];
                let mut loop_o = snap[i - 1];
                if helix.dot(loop_o) < 0.0 {
                    loop_o = -loop_o;
                }
                let blended = helix * 0.6 + loop_o * 0.4;
                let l = blended.magnitude();
                if l > 1e-6 {
                    let n_o = blended / l;
                    bb[i - 1].orientation = [n_o.x, n_o.y, n_o.z];
                }
            }
        }
    }
}

/// Per-residue values picked by [`sample_residue`].
struct ResidueSample {
    atom_id: AtomIndex,
    position: lin_alg::f32::Vec3,
    orientation: lin_alg::f32::Vec3,
    /// `flags::SS_MASK`-encoded secondary structure code (4 bits).
    ss: u32,
}

/// Pick the reference atom + orientation seed for one biopolymer
/// residue. Returns `None` for residues lacking the required backbone atom.
fn sample_residue(residue: &ResidueView<'_>, coord_set: &CoordSet) -> Option<ResidueSample> {
    if residue.is_protein() {
        let (idx, atom) = residue.ca()?;
        let position = coord_set.get_atom_coord(idx)?;
        let orientation = orientation_for_protein(residue, coord_set, position);
        let ss = (atom.ss_type as u8) as u32 & flags::SS_MASK;
        return Some(ResidueSample {
            atom_id: idx,
            position,
            orientation,
            ss,
        });
    }
    if residue.is_nucleic() {
        // Reference atom for nucleic backbone: P, with fallbacks to C3'
        // and C1' so terminal residues and atypical PDBs still emit a sample.
        let (idx, _atom) = residue
            .find_by_name("P")
            .filter(|(index, _)| coord_set.has_atom(*index))
            .or_else(|| {
                residue
                    .find_by_name("C3'")
                    .filter(|(index, _)| coord_set.has_atom(*index))
            })
            .or_else(|| {
                residue
                    .find_by_name("C1'")
                    .filter(|(index, _)| coord_set.has_atom(*index))
            })?;
        let position = coord_set.get_atom_coord(idx)?;
        let orientation = orientation_for_nucleic(residue, coord_set, position);
        // Force NucleicRibbon SS — `cartoon_type_for` maps it to
        // `CartoonType::NucleicRect`, which `cartoon_extrude.wgsl`
        // renders as a flat paddle (case 2u, 4u in the run switch).
        let ss = (SecondaryStructure::NucleicRibbon as u8) as u32 & flags::SS_MASK;
        return Some(ResidueSample {
            atom_id: idx,
            position,
            orientation,
            ss,
        });
    }
    None
}

/// Pick the per-protein-residue orientation: prefer `CA→O`, then
/// `CA→C`, then a deterministic default.
fn orientation_for_protein(
    residue: &ResidueView<'_>,
    coord_set: &CoordSet,
    ca_pos: lin_alg::f32::Vec3,
) -> lin_alg::f32::Vec3 {
    use lin_alg::f32::Vec3;

    if let Some((o_idx, _)) = residue.find_by_name("O") {
        if let Some(o_pos) = coord_set.get_atom_coord(o_idx) {
            let dir = o_pos - ca_pos;
            let len_sq = dir.magnitude_squared();
            if len_sq > 1e-6 {
                return dir / len_sq.sqrt();
            }
        }
    }
    if let Some((c_idx, _)) = residue.find_by_name("C") {
        if let Some(c_pos) = coord_set.get_atom_coord(c_idx) {
            let dir = c_pos - ca_pos;
            let len_sq = dir.magnitude_squared();
            if len_sq > 1e-6 {
                return dir / len_sq.sqrt();
            }
        }
    }
    Vec3::new(0.0, 1.0, 0.0)
}

/// Pick the per-nucleic-residue orientation: direction from `ref_pos`
/// (typically P) toward the C1' sugar attachment — this points toward
/// the base, so the NucleicRect paddle's flat face ends up facing the
/// base plane. Fallbacks: C4' (still in the sugar), then default.
fn orientation_for_nucleic(
    residue: &ResidueView<'_>,
    coord_set: &CoordSet,
    ref_pos: lin_alg::f32::Vec3,
) -> lin_alg::f32::Vec3 {
    use lin_alg::f32::Vec3;

    for atom_name in ["C1'", "C4'"] {
        if let Some((idx, _)) = residue.find_by_name(atom_name) {
            if let Some(pos) = coord_set.get_atom_coord(idx) {
                let dir = pos - ref_pos;
                let len_sq = dir.magnitude_squared();
                if len_sq > 1e-6 {
                    return dir / len_sq.sqrt();
                }
            }
        }
    }
    Vec3::new(0.0, 1.0, 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lin_alg::f32::Vec3;
    use patinae_mol::{
        AtomBuilder, AtomFlags, AtomResidue, MoleculeBuilder, RepMask, SecondaryStructure,
    };
    use std::sync::Arc;

    /// Build a synthetic helix-like fragment with `n` residues whose CA atoms
    /// march along +z. Each residue has CA, C, O atoms so orientation falls
    /// back to CA→O.
    fn make_protein(
        name: &str,
        chain: &str,
        start_resv: i32,
        n: i32,
        cartoon_visible: bool,
    ) -> patinae_mol::ObjectMolecule {
        let mut b = MoleculeBuilder::new(name);
        for i in 0..n {
            let resv = start_resv + i;
            let z = i as f32 * 3.8; // ~CA-CA distance
            let res = Arc::new(AtomResidue::from_parts(chain, "ALA", resv, ' ', ""));

            let mut ca = AtomBuilder::new().name("CA").element_symbol("C").build();
            ca.residue = res.clone();
            ca.state.flags |= AtomFlags::PROTEIN;
            if cartoon_visible {
                ca.repr.visible_reps.set_visible(RepMask::CARTOON);
            }
            b = b.add_atom(ca, Vec3::new(0.0, 0.0, z));

            let mut c = AtomBuilder::new().name("C").element_symbol("C").build();
            c.residue = res.clone();
            c.state.flags |= AtomFlags::PROTEIN;
            b = b.add_atom(c, Vec3::new(0.7, 0.0, z + 0.5));

            let mut o = AtomBuilder::new().name("O").element_symbol("O").build();
            o.residue = res;
            o.state.flags |= AtomFlags::PROTEIN;
            b = b.add_atom(o, Vec3::new(0.0, 1.0, z));
        }
        b.build()
    }

    #[test]
    fn subset_backbone_breaks_before_smoothing_and_uses_only_retained_orientation_atoms() {
        let molecule = make_protein("subset", "A", 1, 4, true);
        let coords = molecule.current_coord_set().unwrap();
        let includes = |index: u32| !matches!(index, 2..=5);
        let mut filtered = CoordSet::new();
        for (index, position) in coords.iter_with_atoms() {
            if includes(index.as_u32()) {
                filtered.add_coord(index, position);
            }
        }
        let result = extract_filtered_backbone(&molecule, &filtered, 10, &includes);
        assert_eq!(
            result.iter().map(|atom| atom.atom_id).collect::<Vec<_>>(),
            [0, 6, 9]
        );
        assert_ne!(result[0].flags & flags::SEG_END, 0);
        assert_ne!(result[1].flags & flags::SEG_START, 0);
        // O was removed from the first residue, so its C atom supplies the
        // orientation. The excluded residue cannot smooth this isolated sample.
        assert!(result[0].orientation[0] > 0.5);
        assert!(result[0].orientation[1].abs() < 1.0e-6);
    }

    #[test]
    fn extracts_one_atom_per_eligible_ca() {
        let mol = make_protein("p", "A", 1, 5, true);
        let coord = mol.current_coord_set().expect("coord set").clone();
        let bb = extract_backbone(&mol, &coord, 10);
        assert_eq!(bb.len(), 5);
    }

    #[test]
    fn extracts_hidden_rep_residues_for_retained_geometry() {
        let mol = make_protein("p", "A", 1, 3, false);
        let coord = mol.current_coord_set().expect("coord set").clone();
        let bb = extract_backbone(&mol, &coord, 10);
        assert_eq!(bb.len(), 3);
    }

    #[test]
    fn first_atom_marked_seg_start_last_marked_seg_end() {
        let mol = make_protein("p", "A", 1, 4, true);
        let coord = mol.current_coord_set().expect("coord set").clone();
        let bb = extract_backbone(&mol, &coord, 10);
        assert_eq!(bb.len(), 4);
        assert!(bb[0].flags & flags::SEG_START != 0);
        assert!(bb[3].flags & flags::SEG_END != 0);
        // Middle atoms carry no segment marker.
        assert_eq!(bb[1].flags & (flags::SEG_START | flags::SEG_END), 0);
        assert_eq!(bb[2].flags & (flags::SEG_START | flags::SEG_END), 0);
    }

    #[test]
    fn resv_gap_promotes_to_chain_break() {
        // 1..=3, then 50..=52 — the jump 3→50 exceeds gap_cutoff and must
        // close the first run + open a new one.
        let mut b = MoleculeBuilder::new("p");
        for resv in [1, 2, 3, 50, 51, 52] {
            let res = Arc::new(AtomResidue::from_parts("A", "ALA", resv, ' ', ""));
            let mut ca = AtomBuilder::new().name("CA").element_symbol("C").build();
            ca.residue = res;
            ca.state.flags |= AtomFlags::PROTEIN;
            ca.repr.visible_reps.set_visible(RepMask::CARTOON);
            b = b.add_atom(ca, Vec3::new(0.0, 0.0, resv as f32 * 3.8));
        }
        let mol = b.build();
        let coord = mol.current_coord_set().expect("coord set").clone();
        let bb = extract_backbone(&mol, &coord, 10);
        assert_eq!(bb.len(), 6);

        // First run: 0..=2.
        assert!(bb[0].flags & flags::SEG_START != 0);
        assert!(bb[2].flags & flags::SEG_END != 0);
        // Second run: 3..=5.
        assert!(bb[3].flags & flags::SEG_START != 0);
        assert!(bb[5].flags & flags::SEG_END != 0);
        // No middle atom carries a marker.
        for i in [1, 4] {
            assert_eq!(bb[i].flags & (flags::SEG_START | flags::SEG_END), 0);
        }
    }

    #[test]
    fn sheet_arrow_tags_second_to_last_residue_of_run() {
        // Build a hand-crafted backbone:
        //   indices 0-3: 4-residue sheet run → arrow on index 2 (second-to-last)
        //   index 4: loop
        //   index 5: singleton sheet → no arrow (run length 1)
        //   index 6: loop
        //   indices 7-8: 2-residue sheet → no arrow (run length 2 < 3)
        let mut bb: Vec<BackboneAtom> = Vec::new();
        let make = |ss: u32, flags_extra: u32| BackboneAtom {
            position: [0.0, 0.0, 0.0],
            atom_id: 0,
            orientation: [0.0, 1.0, 0.0],
            flags: ss | flags_extra,
        };
        bb.push(make(2, flags::SEG_START)); // 0
        bb.push(make(2, 0)); // 1
        bb.push(make(2, 0)); // 2 ← ARROW (second-to-last of run 0..3)
        bb.push(make(2, 0)); // 3 (last of run)
        bb.push(make(0, 0)); // 4 loop
        bb.push(make(2, 0)); // 5 singleton sheet — no arrow
        bb.push(make(0, 0)); // 6 loop
        bb.push(make(2, 0)); // 7 sheet
        bb.push(make(2, flags::SEG_END)); // 8 sheet — 2-residue run, no arrow

        tag_sheet_arrows(&mut bb);

        assert_eq!(bb[0].flags & flags::SHEET_ARROW, 0);
        assert_eq!(bb[1].flags & flags::SHEET_ARROW, 0);
        assert_ne!(
            bb[2].flags & flags::SHEET_ARROW,
            0,
            "second-to-last of 4-run"
        );
        assert_eq!(bb[3].flags & flags::SHEET_ARROW, 0);
        assert_eq!(bb[4].flags & flags::SHEET_ARROW, 0);
        assert_eq!(bb[5].flags & flags::SHEET_ARROW, 0, "singleton: no arrow");
        assert_eq!(bb[6].flags & flags::SHEET_ARROW, 0);
        assert_eq!(bb[7].flags & flags::SHEET_ARROW, 0, "2-run: no arrow");
        assert_eq!(bb[8].flags & flags::SHEET_ARROW, 0);
    }

    /// Build a protein along +z with one residue per slot in `ss`. Each
    /// residue gets a CA / C / O atom triple so orientation falls back to
    /// CA→O. Helix CAs are placed on a real ~3.6-residues-per-turn helical
    /// curve so `compute_round_helix_orientations` can find a non-degenerate
    /// axis; loop CAs continue along the helix axis as a straight bridge.
    fn make_protein_with_ss(ss: &[SecondaryStructure]) -> patinae_mol::ObjectMolecule {
        let mut b = MoleculeBuilder::new("hch");
        let helix_radius = 2.3_f32;
        let helix_rise = 1.5_f32;
        let helix_angle = std::f32::consts::PI * (100.0 / 180.0);
        for (i, &ss_i) in ss.iter().enumerate() {
            let resv = (i + 1) as i32;
            let z = i as f32 * helix_rise;
            let (x, y) = if matches!(
                ss_i,
                SecondaryStructure::Helix
                    | SecondaryStructure::Helix310
                    | SecondaryStructure::HelixPi
            ) {
                let a = i as f32 * helix_angle;
                (helix_radius * a.cos(), helix_radius * a.sin())
            } else {
                (0.0, 0.0)
            };
            let res = Arc::new(AtomResidue::from_parts("A", "ALA", resv, ' ', ""));

            let mut ca = AtomBuilder::new()
                .name("CA")
                .element_symbol("C")
                .ss_type(ss_i)
                .build();
            ca.residue = res.clone();
            ca.state.flags |= AtomFlags::PROTEIN;
            ca.repr.visible_reps.set_visible(RepMask::CARTOON);
            b = b.add_atom(ca, Vec3::new(x, y, z));

            let mut c = AtomBuilder::new()
                .name("C")
                .element_symbol("C")
                .ss_type(ss_i)
                .build();
            c.residue = res.clone();
            c.state.flags |= AtomFlags::PROTEIN;
            b = b.add_atom(c, Vec3::new(x + 0.7, y, z + 0.5));

            let mut o = AtomBuilder::new()
                .name("O")
                .element_symbol("O")
                .ss_type(ss_i)
                .build();
            o.residue = res;
            o.state.flags |= AtomFlags::PROTEIN;
            b = b.add_atom(o, Vec3::new(x, y + 1.0, z));
        }
        b.build()
    }

    #[test]
    fn helix_coil_helix_preserves_ss_and_produces_finite_unit_orientations() {
        // Regression: a 10-helix / 5-loop / 10-helix run must
        // (a) keep its per-residue SS class through extraction;
        // (b) produce finite, unit-length orientations everywhere
        //     (no NaN at SS boundaries — the smooth_ss_boundary blend used
        //     to divide by zero for short windows);
        // (c) tag no residue as SHEET_ARROW (no sheet residues exist);
        // (d) emit SEG_START on the first residue and SEG_END on the last.
        let mut ss: Vec<SecondaryStructure> = Vec::with_capacity(25);
        ss.extend(std::iter::repeat_n(SecondaryStructure::Helix, 10));
        ss.extend(std::iter::repeat_n(SecondaryStructure::Loop, 5));
        ss.extend(std::iter::repeat_n(SecondaryStructure::Helix, 10));

        let mol = make_protein_with_ss(&ss);
        let coord = mol.current_coord_set().expect("coord set").clone();
        let bb = extract_backbone(&mol, &coord, 10);

        assert_eq!(bb.len(), 25, "one BackboneAtom per eligible residue");

        for (i, atom) in bb.iter().enumerate() {
            // (a) SS preserved
            let stored_ss = (atom.flags & flags::SS_MASK) as u8;
            let expected_ss = ss[i] as u8;
            assert_eq!(
                stored_ss, expected_ss,
                "SS at residue {i}: expected {expected_ss}, got {stored_ss}"
            );

            // (b) orientation finite + unit length
            let o = atom.orientation;
            assert!(
                o[0].is_finite() && o[1].is_finite() && o[2].is_finite(),
                "non-finite orientation at residue {i}: {o:?}"
            );
            let len = (o[0] * o[0] + o[1] * o[1] + o[2] * o[2]).sqrt();
            assert!(
                (len - 1.0).abs() < 1e-3,
                "orientation at residue {i} not unit length: |o|={len}, o={o:?}"
            );

            // (c) no sheet arrow
            assert_eq!(
                atom.flags & flags::SHEET_ARROW,
                0,
                "SHEET_ARROW unexpectedly set on residue {i}"
            );
        }

        // (d) segment markers on terminals only
        assert_ne!(bb[0].flags & flags::SEG_START, 0);
        assert_ne!(bb[bb.len() - 1].flags & flags::SEG_END, 0);
        for (i, atom) in bb.iter().enumerate().take(bb.len() - 1).skip(1) {
            assert_eq!(
                atom.flags & flags::SEG_END,
                0,
                "interior residue {i} carries SEG_END"
            );
        }
    }

    #[test]
    fn separate_chains_become_separate_runs() {
        let mut b = MoleculeBuilder::new("p");
        for &(chain, n) in &[("A", 3_i32), ("B", 2)] {
            for resv in 1..=n {
                let res = Arc::new(AtomResidue::from_parts(chain, "ALA", resv, ' ', ""));
                let mut ca = AtomBuilder::new().name("CA").element_symbol("C").build();
                ca.residue = res;
                ca.state.flags |= AtomFlags::PROTEIN;
                ca.repr.visible_reps.set_visible(RepMask::CARTOON);
                b = b.add_atom(ca, Vec3::new(0.0, 0.0, resv as f32 * 3.8));
            }
        }
        let mol = b.build();
        let coord = mol.current_coord_set().expect("coord set").clone();
        let bb = extract_backbone(&mol, &coord, 10);
        assert_eq!(bb.len(), 5);
        // Chain A starts/ends.
        assert!(bb[0].flags & flags::SEG_START != 0);
        assert!(bb[2].flags & flags::SEG_END != 0);
        // Chain B starts/ends.
        assert!(bb[3].flags & flags::SEG_START != 0);
        assert!(bb[4].flags & flags::SEG_END != 0);
    }
}
