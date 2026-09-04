//! CPU reference oracles for the surface compute pipeline.
//!
//! These are intentionally slow, allocation-friendly implementations whose
//! only job is to validate the GPU passes (`surface_density.wgsl`,
//! `surface_ses.wgsl`, `surface_mc_*.wgsl` — landing in 4.3..4.5) under
//! `cargo test` without a GPU adapter. Production rendering goes through the
//! compute pipeline; nothing in `SurfaceRep::build` calls these helpers.
//!
//! ## Definitions
//!
//! - **SAS density (`surface_solvent = true`)**:
//!   `ρ(x) = Σ_a exp(-α · |x - c_a|² / r_a_eff²)` with
//!   `r_a_eff = r_a + probe_radius` and `α = SAS_ALPHA`. Iso level is
//!   [`SAS_ISO`] — at this iso, an isolated atom's surface lies exactly at
//!   distance `r_a_eff` from its centre (since `exp(-α) = SAS_ISO`).
//! - **SES distance (default, `surface_solvent = false`)**:
//!   `f(x) = min_a(|x - c_a| - r_a)` — signed distance to the union of
//!   van-der-Waals spheres. The probe-rolled SES surface is approximated by
//!   the iso-set `f(x) = -probe_radius` (analytical morphological closing
//!   diverges in deep concave pockets, but for the smoke tests here this is
//!   sufficient — the GPU pass will do the real Connolly via SDF + morph
//!   dilate/erode in 4.4).
//!
//! ## Marching cubes convention
//!
//! Inside the surface ⇔ `value >= iso`. Bourke's `TRI_TABLE` was authored
//! against the opposite convention (`value < iso`), so `cpu_marching_cubes`
//! reverses the per-triangle edge order to keep winding right-handed (see
//! the docstring of [`mc_tables`](super::mc_tables) for the full discussion).

use super::constants::{SAS_ALPHA, SAS_ISO};
use super::mc_tables::{CORNER_OFFSETS, EDGE_TABLE, EDGE_VERTICES, TRI_COUNT, TRI_TABLE};

/// Iso level for SES distance — the surface is the offset of the vdW SDF by
/// the probe radius. Caller passes `-probe_radius` directly; this constant
/// just documents the sign.
pub const SES_ISO_OFFSET_SIGN: f32 = -1.0;

/// A single atom contribution. Mirrors what the GPU `SurfaceAtom` SSBO entry
/// will look like; staying parallel keeps the oracle a drop-in reference.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OracleAtom {
    pub pos: [f32; 3],
    pub radius: f32,
    pub index: u32,
}

/// Cubic voxel grid with one scalar per voxel. `values[i + j*nx + k*nx*ny]`
/// is the sample at world-space position
/// `bbox_min + voxel_size * (i, j, k)`.
#[derive(Debug, Clone)]
pub struct Grid3D {
    pub bbox_min: [f32; 3],
    pub voxel_size: f32,
    pub dims: [u32; 3],
    pub values: Vec<f32>,
}

impl Grid3D {
    pub fn voxel_count(&self) -> usize {
        (self.dims[0] * self.dims[1] * self.dims[2]) as usize
    }

    pub fn position(&self, i: u32, j: u32, k: u32) -> [f32; 3] {
        [
            self.bbox_min[0] + self.voxel_size * i as f32,
            self.bbox_min[1] + self.voxel_size * j as f32,
            self.bbox_min[2] + self.voxel_size * k as f32,
        ]
    }

    pub fn sample(&self, i: u32, j: u32, k: u32) -> f32 {
        let nx = self.dims[0] as usize;
        let ny = self.dims[1] as usize;
        self.values[i as usize + j as usize * nx + k as usize * nx * ny]
    }
}

/// Pad each axis by `r_max + probe + 2 voxels` so Gaussian tails of edge
/// atoms are fully captured.
pub fn build_grid_layout(
    atoms: &[OracleAtom],
    probe: f32,
    voxel_size: f32,
) -> Option<([f32; 3], [u32; 3])> {
    if atoms.is_empty() {
        return None;
    }
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    let mut r_max = 0.0_f32;
    for a in atoms {
        for k in 0..3 {
            min[k] = min[k].min(a.pos[k]);
            max[k] = max[k].max(a.pos[k]);
        }
        r_max = r_max.max(a.radius);
    }
    let pad = r_max + probe + 2.0 * voxel_size;
    let bbox_min = [min[0] - pad, min[1] - pad, min[2] - pad];
    let dims = [
        (((max[0] - min[0] + 2.0 * pad) / voxel_size).ceil() as u32).max(2),
        (((max[1] - min[1] + 2.0 * pad) / voxel_size).ceil() as u32).max(2),
        (((max[2] - min[2] + 2.0 * pad) / voxel_size).ceil() as u32).max(2),
    ];
    Some((bbox_min, dims))
}

// -----------------------------------------------------------------------------
// Density / distance evaluation
// -----------------------------------------------------------------------------

/// SAS density at `point`. `α` is typically [`SAS_ALPHA`] (4.0).
pub fn sas_density(point: [f32; 3], atoms: &[OracleAtom], probe: f32, alpha: f32) -> f32 {
    let mut sum = 0.0_f32;
    for a in atoms {
        let dx = point[0] - a.pos[0];
        let dy = point[1] - a.pos[1];
        let dz = point[2] - a.pos[2];
        let d2 = dx * dx + dy * dy + dz * dz;
        let r_eff = a.radius + probe;
        if r_eff <= 0.0 {
            continue;
        }
        sum += (-alpha * d2 / (r_eff * r_eff)).exp();
    }
    sum
}

/// Signed distance to the union of vdW spheres: `f(x) = min_a(|x - c_a| - r_a)`.
/// Negative inside, positive outside.
pub fn vdw_sdf(point: [f32; 3], atoms: &[OracleAtom]) -> f32 {
    let mut best = f32::INFINITY;
    for a in atoms {
        let dx = point[0] - a.pos[0];
        let dy = point[1] - a.pos[1];
        let dz = point[2] - a.pos[2];
        let d = (dx * dx + dy * dy + dz * dz).sqrt() - a.radius;
        if d < best {
            best = d;
        }
    }
    best
}

/// True Connolly SES scalar field at `point`, computed by brute-force
/// max-filter of `vdw_sdf` over a ball-shaped voxel stencil of radius
/// `stencil_radius_voxels` voxels (each axis step is `voxel_size` Å).
/// Returns `g_ses(x) = max_{y in ball(x, probe)} vdw_sdf(y) - probe`.
///
/// Convention: `g_ses(x) <= 0` means `x` lies in the molecular (excluded)
/// volume — the probe surface cannot reach `x`. The MC pass reads this with
/// `iso = 0`, `invert_inside = 1`. Mirrors `surface_ses_morph.wgsl` so the
/// GPU pass can be regression-checked voxel-by-voxel.
pub fn connolly_ses_field(
    point: [f32; 3],
    atoms: &[OracleAtom],
    probe: f32,
    stencil_radius_voxels: u32,
    voxel_size: f32,
) -> f32 {
    let r = stencil_radius_voxels as i32;
    let r2 = (r * r) as f32;
    let mut best = f32::NEG_INFINITY;
    for dk in -r..=r {
        for dj in -r..=r {
            for di in -r..=r {
                let d2 = (di * di + dj * dj + dk * dk) as f32;
                if d2 > r2 {
                    continue;
                }
                let y = [
                    point[0] + di as f32 * voxel_size,
                    point[1] + dj as f32 * voxel_size,
                    point[2] + dk as f32 * voxel_size,
                ];
                let v = vdw_sdf(y, atoms);
                if v > best {
                    best = v;
                }
            }
        }
    }
    best - probe
}

/// Index of the closest atom (Euclidean distance to centre, ignoring radius).
/// Used to assign per-voxel `owner_atom_index` in the density grid.
pub fn owner_atom(point: [f32; 3], atoms: &[OracleAtom]) -> Option<u32> {
    let mut best = f32::INFINITY;
    let mut owner = None;
    for a in atoms {
        let dx = point[0] - a.pos[0];
        let dy = point[1] - a.pos[1];
        let dz = point[2] - a.pos[2];
        let d2 = dx * dx + dy * dy + dz * dz;
        if d2 < best {
            best = d2;
            owner = Some(a.index);
        }
    }
    owner
}

/// Build a SAS density grid by direct N×V evaluation. O(atoms × voxels) — only
/// for tests / oracles.
pub fn build_sas_grid(atoms: &[OracleAtom], probe: f32, voxel_size: f32) -> Option<Grid3D> {
    let (bbox_min, dims) = build_grid_layout(atoms, probe, voxel_size)?;
    let total = (dims[0] * dims[1] * dims[2]) as usize;
    let mut values = vec![0.0_f32; total];
    let nx = dims[0] as usize;
    let ny = dims[1] as usize;
    for k in 0..dims[2] {
        for j in 0..dims[1] {
            for i in 0..dims[0] {
                let p = [
                    bbox_min[0] + voxel_size * i as f32,
                    bbox_min[1] + voxel_size * j as f32,
                    bbox_min[2] + voxel_size * k as f32,
                ];
                values[i as usize + j as usize * nx + k as usize * nx * ny] =
                    sas_density(p, atoms, probe, SAS_ALPHA);
            }
        }
    }
    Some(Grid3D {
        bbox_min,
        voxel_size,
        dims,
        values,
    })
}

/// Build a vdW-SDF grid (negative inside spheres). For SES, threshold at
/// `iso = -probe`: voxels with `sdf < -probe` are inside the SES surface.
pub fn build_vdw_sdf_grid(atoms: &[OracleAtom], probe: f32, voxel_size: f32) -> Option<Grid3D> {
    let (bbox_min, dims) = build_grid_layout(atoms, probe, voxel_size)?;
    let total = (dims[0] * dims[1] * dims[2]) as usize;
    let mut values = vec![0.0_f32; total];
    let nx = dims[0] as usize;
    let ny = dims[1] as usize;
    for k in 0..dims[2] {
        for j in 0..dims[1] {
            for i in 0..dims[0] {
                let p = [
                    bbox_min[0] + voxel_size * i as f32,
                    bbox_min[1] + voxel_size * j as f32,
                    bbox_min[2] + voxel_size * k as f32,
                ];
                values[i as usize + j as usize * nx + k as usize * nx * ny] = vdw_sdf(p, atoms);
            }
        }
    }
    Some(Grid3D {
        bbox_min,
        voxel_size,
        dims,
        values,
    })
}

/// Discretised volume estimate: voxel_size³ × (count of voxels with value
/// satisfying `inside_predicate`). Used by the volume-conservation tests to
/// check density-iso conventions don't drift.
pub fn discretised_volume<F: Fn(f32) -> bool>(grid: &Grid3D, inside: F) -> f32 {
    let v = grid.voxel_size;
    let n = grid.values.iter().filter(|&&val| inside(val)).count();
    n as f32 * v * v * v
}

// -----------------------------------------------------------------------------
// Marching cubes
// -----------------------------------------------------------------------------

/// Standard marching cubes against a scalar grid. Returns `(positions, indices)`
/// — vertices are *not* deduplicated across adjacent voxels (the GPU pass uses
/// an atomic counter and emits triangles directly; the oracle mirrors that).
///
/// **Convention.** A corner is *inside* when `value >= iso`. This matches the
/// WGSL classify pass; `TRI_TABLE` is read with reversed per-triangle slot
/// order to compensate for Bourke's opposite convention (see `mc_tables`).
pub fn cpu_marching_cubes(grid: &Grid3D, iso: f32) -> (Vec<[f32; 3]>, Vec<u32>) {
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    if grid.dims[0] < 2 || grid.dims[1] < 2 || grid.dims[2] < 2 {
        return (positions, indices);
    }

    for cz in 0..grid.dims[2] - 1 {
        for cy in 0..grid.dims[1] - 1 {
            for cx in 0..grid.dims[0] - 1 {
                // Read 8 corner samples + positions.
                let mut corner_val = [0.0_f32; 8];
                let mut corner_pos = [[0.0_f32; 3]; 8];
                let mut case_idx = 0_usize;
                for (c, off) in CORNER_OFFSETS.iter().enumerate() {
                    let i = cx + off[0];
                    let j = cy + off[1];
                    let k = cz + off[2];
                    let v = grid.sample(i, j, k);
                    corner_val[c] = v;
                    corner_pos[c] = grid.position(i, j, k);
                    if v >= iso {
                        case_idx |= 1 << c;
                    }
                }

                let edge_mask = EDGE_TABLE[case_idx];
                if edge_mask == 0 {
                    continue;
                }

                // Interpolate vertex on each crossed edge.
                let mut edge_vert = [[0.0_f32; 3]; 12];
                for e in 0..12 {
                    if edge_mask & (1 << e) == 0 {
                        continue;
                    }
                    let [a, b] = EDGE_VERTICES[e];
                    let va = corner_val[a as usize];
                    let vb = corner_val[b as usize];
                    let denom = vb - va;
                    let t = if denom.abs() > 1e-6 {
                        ((iso - va) / denom).clamp(0.0, 1.0)
                    } else {
                        0.5
                    };
                    let pa = corner_pos[a as usize];
                    let pb = corner_pos[b as usize];
                    edge_vert[e] = [
                        pa[0] + t * (pb[0] - pa[0]),
                        pa[1] + t * (pb[1] - pa[1]),
                        pa[2] + t * (pb[2] - pa[2]),
                    ];
                }

                let n_tri = TRI_COUNT[case_idx] as usize;
                let row = &TRI_TABLE[case_idx];
                for t in 0..n_tri {
                    // Reversed slot order to compensate for Bourke's opposite
                    // inside/outside convention — emits CCW with our `>=` rule.
                    for slot in 0..3 {
                        let edge_id = row[3 * t + (2 - slot)] as usize;
                        positions.push(edge_vert[edge_id]);
                        indices.push((indices.len()) as u32);
                    }
                }
            }
        }
    }

    (positions, indices)
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn atom(pos: [f32; 3], radius: f32, index: u32) -> OracleAtom {
        OracleAtom { pos, radius, index }
    }

    #[test]
    fn isolated_atom_density_at_centre_is_one() {
        let a = [atom([0.0, 0.0, 0.0], 1.5, 0)];
        let d = sas_density([0.0, 0.0, 0.0], &a, 1.4, SAS_ALPHA);
        assert!((d - 1.0).abs() < 1e-6, "ρ(0) = {d}, expected 1.0");
    }

    #[test]
    fn isolated_atom_density_at_r_eff_is_iso() {
        // At distance r_eff from the centre, density = exp(-α) = SAS_ISO.
        let r = 1.5;
        let probe = 1.4;
        let r_eff = r + probe;
        let a = [atom([0.0, 0.0, 0.0], r, 0)];
        let d = sas_density([r_eff, 0.0, 0.0], &a, probe, SAS_ALPHA);
        assert!(
            (d - SAS_ISO).abs() < 1e-6,
            "ρ(r_eff) = {d}, expected SAS_ISO = {SAS_ISO}"
        );
    }

    #[test]
    fn isolated_atom_volume_within_5pct() {
        // Build a SAS grid for a single atom and integrate by counting voxels
        // with density >= SAS_ISO. Should match (4/3)π(r+probe)³ within ±5 %
        // (discretisation error dominates).
        let r: f32 = 1.5;
        let probe: f32 = 1.4;
        let r_eff = r + probe;
        let analytical = (4.0 / 3.0) * PI * r_eff.powi(3);

        let voxel = 0.20;
        let atoms = vec![atom([0.0, 0.0, 0.0], r, 0)];
        let grid = build_sas_grid(&atoms, probe, voxel).unwrap();
        let measured = discretised_volume(&grid, |v| v >= SAS_ISO);
        let err = (measured - analytical).abs() / analytical;
        assert!(
            err < 0.05,
            "single-atom SAS volume {measured} vs analytical {analytical} (err = {err:.4})"
        );
    }

    #[test]
    fn well_separated_atoms_volume_is_additive() {
        // Two atoms 12 Å apart — far enough that Gaussian tails don't lift the
        // midpoint above iso. Combined volume should be ~2× single, ±10 %.
        let r: f32 = 1.5;
        let probe: f32 = 1.4;
        let r_eff = r + probe;
        let single_vol = (4.0 / 3.0) * PI * r_eff.powi(3);

        let voxel = 0.25;
        let atoms = vec![atom([-6.0, 0.0, 0.0], r, 0), atom([6.0, 0.0, 0.0], r, 1)];
        let grid = build_sas_grid(&atoms, probe, voxel).unwrap();
        let measured = discretised_volume(&grid, |v| v >= SAS_ISO);
        let expected = 2.0 * single_vol;
        let err = (measured - expected).abs() / expected;
        assert!(
            err < 0.10,
            "two-atom SAS volume {measured} vs 2×single {expected} (err = {err:.4})"
        );

        // And the midpoint really is below iso.
        let mid = sas_density([0.0, 0.0, 0.0], &atoms, probe, SAS_ALPHA);
        assert!(mid < SAS_ISO, "midpoint density {mid} ≥ iso {SAS_ISO}");
    }

    #[test]
    fn connolly_ses_inside_at_atom_centre() {
        // The atom centre is unambiguously inside the molecular volume — no
        // probe can fit anywhere within `probe` of it.
        let atoms = vec![atom([0.0, 0.0, 0.0], 1.7, 0)];
        let probe = 1.4;
        let voxel = 0.25_f32;
        let r_vox = (probe / voxel).ceil() as u32;
        let g = connolly_ses_field([0.0, 0.0, 0.0], &atoms, probe, r_vox, voxel);
        assert!(g <= 0.0, "atom centre should be inside SES, g_ses = {g}");
    }

    #[test]
    fn connolly_ses_outside_far_from_molecule() {
        let atoms = vec![atom([0.0, 0.0, 0.0], 1.7, 0)];
        let probe = 1.4;
        let voxel = 0.25_f32;
        let r_vox = (probe / voxel).ceil() as u32;
        // 10 Å away — well clear of the molecule + probe shell.
        let g = connolly_ses_field([10.0, 0.0, 0.0], &atoms, probe, r_vox, voxel);
        assert!(g > 0.0, "deep solvent should be outside SES, g_ses = {g}");
    }

    #[test]
    fn connolly_ses_neck_present_when_atoms_overlap() {
        // Atoms close enough that the midpoint is deep inside both vdW shells:
        // the rolling probe cannot squeeze through ⇒ neck is part of SES.
        let r = 2.5_f32;
        let probe = 1.0_f32;
        let half = (r - probe) - 0.1;
        let atoms = vec![atom([-half, 0.0, 0.0], r, 0), atom([half, 0.0, 0.0], r, 1)];
        let voxel = 0.25_f32;
        let r_vox = (probe / voxel).ceil() as u32;
        let g_mid = connolly_ses_field([0.0, 0.0, 0.0], &atoms, probe, r_vox, voxel);
        assert!(
            g_mid <= 0.0,
            "midpoint should be inside SES neck, g_ses = {g_mid}"
        );
    }

    #[test]
    fn ses_no_neck_when_gap_exceeds_two_probes() {
        // Two atoms separated by more than 2·probe of free space between their
        // vdW surfaces ⇒ probe rolls through, no SES neck. The midpoint must
        // be classified as outside (vdw_sdf > -probe).
        let r = 1.5;
        let probe = 1.4;
        // Centre-to-centre distance > 2(r) + 2(probe).
        let d_centre = 2.0 * r + 2.0 * probe + 0.5; // 0.5 Å of slack
        let atoms = vec![
            atom([-d_centre / 2.0, 0.0, 0.0], r, 0),
            atom([d_centre / 2.0, 0.0, 0.0], r, 1),
        ];
        let f_mid = vdw_sdf([0.0, 0.0, 0.0], &atoms);
        // Outside SES iff vdw_sdf > -probe.
        assert!(
            f_mid > -probe,
            "expected probe-pass midpoint outside SES; vdw_sdf = {f_mid}, probe = {probe}"
        );
    }

    #[test]
    fn ses_has_neck_when_atoms_overlap_deeply() {
        // Atoms overlapping enough that the midpoint is at least `probe` deep
        // inside their vdW spheres ⇒ no probe can fit through ⇒ SES neck.
        // Centre-to-centre `d` ⇒ midpoint depth = r - d/2; inside SES iff
        // depth >= probe, i.e. d <= 2(r - probe).
        let r = 2.5;
        let probe = 1.0;
        let half = (r - probe) - 0.1; // 0.1 Å of slack so depth comfortably > probe
        let atoms = vec![atom([-half, 0.0, 0.0], r, 0), atom([half, 0.0, 0.0], r, 1)];
        let f_mid = vdw_sdf([0.0, 0.0, 0.0], &atoms);
        assert!(
            f_mid <= -probe,
            "expected midpoint inside SES; vdw_sdf = {f_mid}, probe = {probe}"
        );
    }

    #[test]
    fn owner_picks_nearest_atom() {
        let atoms = vec![
            atom([-3.0, 0.0, 0.0], 1.5, 7),
            atom([3.0, 0.0, 0.0], 1.5, 11),
        ];
        assert_eq!(owner_atom([-2.5, 0.0, 0.0], &atoms), Some(7));
        assert_eq!(owner_atom([2.5, 0.0, 0.0], &atoms), Some(11));
        assert_eq!(owner_atom([0.1, 0.0, 0.0], &atoms), Some(11));
    }

    #[test]
    fn empty_grid_layout_returns_none() {
        assert!(build_sas_grid(&[], 1.4, 0.5).is_none());
        assert!(build_vdw_sdf_grid(&[], 1.4, 0.5).is_none());
    }

    #[test]
    fn marching_cubes_empty_when_grid_below_iso() {
        // All-zero grid, iso = 0.5 ⇒ no triangles.
        let grid = Grid3D {
            bbox_min: [0.0; 3],
            voxel_size: 1.0,
            dims: [4, 4, 4],
            values: vec![0.0; 64],
        };
        let (verts, idx) = cpu_marching_cubes(&grid, 0.5);
        assert!(verts.is_empty());
        assert!(idx.is_empty());
    }

    #[test]
    fn marching_cubes_empty_when_grid_above_iso() {
        // All-one grid, iso = 0.5 ⇒ all corners inside, no surface.
        let grid = Grid3D {
            bbox_min: [0.0; 3],
            voxel_size: 1.0,
            dims: [4, 4, 4],
            values: vec![1.0; 64],
        };
        let (verts, idx) = cpu_marching_cubes(&grid, 0.5);
        assert!(verts.is_empty());
        assert!(idx.is_empty());
    }

    #[test]
    fn marching_cubes_single_atom_produces_closed_mesh() {
        // SAS surface around an isolated atom: should be a closed triangle
        // mesh enclosing the atom centre. Sanity: every vertex lies near
        // distance r_eff from the centre; triangle count is positive and
        // a multiple of 1 (trivially).
        let r = 1.5;
        let probe = 1.4;
        let r_eff = r + probe;
        let atoms = vec![atom([0.0, 0.0, 0.0], r, 0)];
        let grid = build_sas_grid(&atoms, probe, 0.30).unwrap();
        let (verts, idx) = cpu_marching_cubes(&grid, SAS_ISO);

        assert!(!verts.is_empty(), "expected non-empty mesh for single atom");
        assert_eq!(idx.len() % 3, 0, "indices not a multiple of 3");
        assert_eq!(
            verts.len(),
            idx.len(),
            "oracle does not dedupe — counts must match"
        );

        // Every vertex sits on the SAS iso-shell, so |v| ≈ r_eff (±voxel).
        let tol = 0.30; // one voxel of MC linear-interp slack
        for v in &verts {
            let d = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            assert!(
                (d - r_eff).abs() < tol,
                "vertex {:?} at distance {d}, expected ~{r_eff}",
                v
            );
        }
    }

    #[test]
    fn marching_cubes_winding_is_outward() {
        // For a single atom, every triangle's geometric normal should point
        // away from the centre (signed dot with centroid > 0).
        let r = 1.5;
        let probe = 1.4;
        let atoms = vec![atom([0.0, 0.0, 0.0], r, 0)];
        let grid = build_sas_grid(&atoms, probe, 0.30).unwrap();
        let (verts, idx) = cpu_marching_cubes(&grid, SAS_ISO);
        assert!(!idx.is_empty());

        let mut outward = 0usize;
        let mut total = 0usize;
        for tri in idx.as_chunks::<3>().0 {
            let a = verts[tri[0] as usize];
            let b = verts[tri[1] as usize];
            let c = verts[tri[2] as usize];
            let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let n = [
                ab[1] * ac[2] - ab[2] * ac[1],
                ab[2] * ac[0] - ab[0] * ac[2],
                ab[0] * ac[1] - ab[1] * ac[0],
            ];
            let centroid = [
                (a[0] + b[0] + c[0]) / 3.0,
                (a[1] + b[1] + c[1]) / 3.0,
                (a[2] + b[2] + c[2]) / 3.0,
            ];
            // Outward ⇔ normal aligned with centroid - centre (centre = 0).
            let dot = n[0] * centroid[0] + n[1] * centroid[1] + n[2] * centroid[2];
            if dot > 0.0 {
                outward += 1;
            }
            total += 1;
        }
        let frac = outward as f32 / total as f32;
        assert!(
            frac > 0.95,
            "only {outward}/{total} triangles point outward (frac = {frac:.3}) — winding broken"
        );
    }
}
