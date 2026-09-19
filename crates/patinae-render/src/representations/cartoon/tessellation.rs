//! CPU guide-point tessellation for cartoon/ribbon. Runs
//! once per `CartoonRep::build` to:
//!
//!   1. Compute differences / normals / tangents from guide-point positions.
//!   2. Round-helix orientations for helix runs.
//!   3. `refine_normals` (Gram-Schmidt + greedy alt-sign + kink soften).
//!   4. `flatten_sheets` (4-cycle window-1 Laplacian on positions + orient).
//!   5. Optional loop-only smoothing (`cartoon_smooth_loops`).
//!   6. Recompute tangents post-flatten/smooth.
//!   7. `cartoon_generate_sample` + `cartoon_generate_refine` per pair.
//!   8. Boundary-pair-as-loop run classification.
//!
//! Output: `Vec<ExtrudePoint>` + `Vec<RunDescriptor>`. The GPU
//! `cartoon_extrude.wgsl` kernel takes those and emits the final
//! `StdVertex` triangle list in a single dispatch.

use bytemuck::{Pod, Zeroable};
use lin_alg::f32::Vec3;
use patinae_mol::SecondaryStructure;
use patinae_settings::ResolvedSettings;

use super::backbone::{flags as bb_flags, BackboneAtom};
use super::utils::{is_helix, normalize_safe, smooth};
use crate::render_input::SceneLod;

/// Per-residue guide point.
#[derive(Debug, Clone, Copy)]
pub struct GuidePoint {
    pub position: Vec3,
    pub orientation: Vec3,
    pub ss_type: SecondaryStructure,
    pub atom_idx: u32,
}

/// Contiguous run of guide points for one segment: one polymer subchain
/// without chain breaks.
#[derive(Debug, Default, Clone)]
pub struct BackboneSegment {
    pub guide_points: Vec<GuidePoint>,
}

impl BackboneSegment {
    pub fn len(&self) -> usize {
        self.guide_points.len()
    }
    pub fn is_empty(&self) -> bool {
        self.guide_points.is_empty()
    }
}

/// Inverse of `SecondaryStructure as u8`. Mirrors the enum's `#[repr(u8)]`
/// layout (`patinae-mol::secondary`).
fn ss_from_u8(v: u8) -> SecondaryStructure {
    match v {
        1 => SecondaryStructure::Helix,
        2 => SecondaryStructure::Sheet,
        3 => SecondaryStructure::Helix310,
        4 => SecondaryStructure::HelixPi,
        5 => SecondaryStructure::Turn,
        6 => SecondaryStructure::Bend,
        7 => SecondaryStructure::NucleicRibbon,
        _ => SecondaryStructure::Loop,
    }
}

/// Convert `Vec<BackboneAtom>` extracted by `extract_retained_backbone` to
/// segments split at `SEG_END` markers. Each segment is a continuous
/// polymer run.
pub fn segments_from_backbone_atoms(bb: &[BackboneAtom]) -> Vec<BackboneSegment> {
    let mut segments = Vec::new();
    let mut current = BackboneSegment::default();
    for atom in bb {
        let ss_raw = (atom.flags & bb_flags::SS_MASK) as u8;
        let ss_type = ss_from_u8(ss_raw);
        current.guide_points.push(GuidePoint {
            position: Vec3::new(atom.position[0], atom.position[1], atom.position[2]),
            orientation: Vec3::new(
                atom.orientation[0],
                atom.orientation[1],
                atom.orientation[2],
            ),
            ss_type,
            atom_idx: atom.atom_id,
        });
        if (atom.flags & bb_flags::SEG_END) != 0 && !current.is_empty() {
            segments.push(std::mem::take(&mut current));
        }
    }
    // Flush trailing segment if it didn't end on a SEG_END marker (last atom
    // of the buffer). `extract_retained_backbone` always sets SEG_END on the
    // last atom of each subchain, so this is a defensive fallback.
    if !current.is_empty() {
        segments.push(current);
    }
    segments
}

// ============================================================================
// Pipeline + geometry settings.
// ============================================================================

#[derive(Debug, Clone)]
pub struct PipelineSettings {
    pub sampling: u32,        // cartoon_sampling = 7
    pub power_a: f32,         // cartoon_power = 2.0
    pub power_b: f32,         // cartoon_power_b = 0.52
    pub throw_factor: f32,    // cartoon_throw = 1.35
    pub flat_cycles: u32,     // cartoon_flat_cycles = 4
    pub smooth_first: u32,    // cartoon_smooth_first = 1
    pub smooth_last: u32,     // cartoon_smooth_last = 1
    pub smooth_cycles: u32,   // cartoon_smooth_cycles = 2
    pub smooth_loops: bool,   // cartoon_smooth_loops = false
    pub refine_normals: bool, // cartoon_refine_normals = true
    pub round_helices: bool,  // cartoon_round_helices = true
    pub refine: u32,          // cartoon_refine = 5 (forced even)
}

impl Default for PipelineSettings {
    fn default() -> Self {
        Self {
            sampling: 7,
            power_a: 2.0,
            power_b: 0.52,
            throw_factor: 1.35,
            flat_cycles: 4,
            smooth_first: 1,
            smooth_last: 1,
            smooth_cycles: 2,
            smooth_loops: false,
            refine_normals: true,
            round_helices: true,
            refine: 6, // even-rounded from default 5
        }
    }
}

#[derive(Debug, Clone)]
pub struct GeomSettings {
    pub helix_width: f32,     // cartoon_oval_width = 0.25
    pub helix_height: f32,    // cartoon_oval_length = 1.35
    pub sheet_width: f32,     // cartoon_rect_width = 0.4
    pub sheet_height: f32,    // cartoon_rect_length = 1.4
    pub loop_radius: f32,     // cartoon_loop_radius = 0.2
    pub quality: u32,         // 32
    pub fancy_sheets: bool,   // true
    pub arrow_tip_scale: f32, // 1.5
    pub arrow_residues: u32,  // 1
    pub uniform_tube: bool,   // false
}

impl Default for GeomSettings {
    fn default() -> Self {
        Self {
            helix_width: 0.25,
            helix_height: 1.35,
            sheet_width: 0.4,
            sheet_height: 1.4,
            loop_radius: 0.2,
            quality: 32,
            fancy_sheets: true,
            arrow_tip_scale: 1.5,
            arrow_residues: 1,
            uniform_tube: false,
        }
    }
}

/// LOD-aware defaults. The two knobs that govern total vertex count are
/// `PipelineSettings.sampling` (samples-per-residue along the backbone)
/// and `GeomSettings.quality` (number of profile segments around the
/// tube). On 3J3Q-class scenes the High defaults blow past the 4 GB
/// `max_storage_buffer_binding_size` cap for the cartoon vertex SSBO;
/// stepping these down drops vertex count by ~6×.
pub fn from_lod(lod: SceneLod) -> (PipelineSettings, GeomSettings) {
    use SceneLod::*;
    let mut p = PipelineSettings::default();
    let mut g = GeomSettings::default();
    match lod {
        Auto | High => {}
        Medium => {
            p.sampling = 5;
            g.quality = 16;
        }
        Low => {
            p.sampling = 4;
            g.quality = 10;
        }
        Minimum => {
            p.sampling = 3;
            g.quality = 6;
        }
    }
    (p, g)
}

pub fn from_resolved_settings(
    settings: &ResolvedSettings,
    lod: SceneLod,
) -> (PipelineSettings, GeomSettings) {
    let (mut p, mut g) = from_lod(lod);
    let c = &settings.cartoon;

    if c.sampling > 0 {
        p.sampling = (c.sampling as u32).max(1);
    }
    p.power_a = c.power.max(0.0);
    p.power_b = c.power_b.max(0.0);
    p.throw_factor = c.throw.max(0.0);
    p.flat_cycles = c.flat_cycles.max(0) as u32;
    p.smooth_first = c.smooth_first.max(0) as u32;
    p.smooth_last = c.smooth_last.max(0) as u32;
    p.smooth_cycles = c.smooth_cycles.max(0) as u32;
    p.smooth_loops = c.smooth_loops;
    p.refine_normals = c.refine_normals != 0;
    p.round_helices = c.round_helices;

    let raw_refine = c.refine.max(0) as u32;
    p.refine = raw_refine + (raw_refine & 1);

    // Cross-section overrides. `0.0` keeps the LOD-picked default; any
    // positive value replaces it. Mirrors the `ribbon_radius = 0.0 → auto`
    // convention used elsewhere in the cartoon path.
    if c.oval_width > 0.0 {
        g.helix_width = c.oval_width;
    }
    if c.oval_length > 0.0 {
        g.helix_height = c.oval_length;
    }
    if c.rect_width > 0.0 {
        g.sheet_width = c.rect_width;
    }
    if c.rect_length > 0.0 {
        g.sheet_height = c.rect_length;
    }
    if c.loop_radius > 0.0 {
        g.loop_radius = c.loop_radius;
    }
    if c.arrow_tip_scale > 0.0 {
        g.arrow_tip_scale = c.arrow_tip_scale;
    }

    (p, g)
}

/// Cartoon rendering type — selected per-residue from the secondary
/// structure + settings. The compute extruder branches on this in
/// `cartoon_extrude.wgsl::emit_vertex`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum CartoonType {
    Loop = 0,
    Oval = 1,
    Rect = 2,
    Arrow = 3,
    NucleicRect = 4,
}

/// Map SS type + settings → CartoonType.
pub fn cartoon_type_for(ss: SecondaryStructure, settings: &GeomSettings) -> CartoonType {
    if settings.uniform_tube {
        return CartoonType::Loop;
    }
    if is_helix(ss) {
        CartoonType::Oval
    } else if ss == SecondaryStructure::Sheet {
        if settings.fancy_sheets {
            CartoonType::Arrow
        } else {
            CartoonType::Rect
        }
    } else if ss == SecondaryStructure::NucleicRibbon {
        CartoonType::NucleicRect
    } else {
        CartoonType::Loop
    }
}

// ============================================================================
// Differences, normals, and tangents for sampled guide points.
// ============================================================================

/// Returns `(distances, normalized_directions)`, length n-1.
fn compute_differences_and_normals(gps: &[GuidePoint]) -> (Vec<f32>, Vec<Vec3>) {
    let n = gps.len();
    if n < 2 {
        return (Vec::new(), Vec::new());
    }
    let mut dl = Vec::with_capacity(n - 1);
    let mut nv = Vec::with_capacity(n - 1);
    for a in 0..n - 1 {
        let diff = gps[a + 1].position - gps[a].position;
        let d = diff.magnitude();
        dl.push(d);
        if d > 1e-4 {
            nv.push(diff / d);
        } else if a > 0 {
            nv.push(*nv.last().unwrap_or(&Vec3::new(0.0, 0.0, 0.0)));
        } else {
            nv.push(Vec3::new(0.0, 0.0, 0.0));
        }
    }
    (dl, nv)
}

/// Tangent estimation by averaging adjacent normalized differences.
fn compute_tangents(nv: &[Vec3], n: usize) -> Vec<Vec3> {
    if n < 2 || nv.is_empty() {
        return vec![Vec3::new(0.0, 0.0, 1.0); n];
    }
    let mut tv = Vec::with_capacity(n);
    tv.push(nv[0]);
    for a in 1..n - 1 {
        if a < nv.len() && a >= 1 {
            let sum = nv[a] + nv[a - 1];
            tv.push(normalize_safe_or_z(sum));
        } else if a >= 1 && a - 1 < nv.len() {
            tv.push(nv[a - 1]);
        } else if a < nv.len() {
            tv.push(nv[a]);
        } else {
            tv.push(Vec3::new(0.0, 0.0, 1.0));
        }
    }
    tv.push(nv[nv.len() - 1]);
    tv
}

fn normalize_safe_or_z(v: Vec3) -> Vec3 {
    let n = normalize_safe(v);
    if n.magnitude() > 1e-6 {
        n
    } else {
        Vec3::new(0.0, 0.0, 1.0)
    }
}

// ============================================================================
// Round helix orientations.
// ============================================================================

#[allow(unused_assignments)]
fn compute_round_helix_orientations(gps: &mut [GuidePoint], tv: &[Vec3]) {
    let n = gps.len();
    if n < 2 {
        return;
    }
    let mut v1: Option<usize> = None;
    let mut v2: Option<usize> = None;
    let mut v3: Option<usize> = None;
    let mut v4: Option<usize> = None;
    let mut v5: Option<usize> = None;
    let mut last = 0i32;
    let mut prev_center = Vec3::new(0.0, 0.0, 0.0);

    for a in 0..n {
        v5 = v4;
        v4 = v3;
        v3 = v2;
        v2 = v1;

        if is_helix(gps[a].ss_type) {
            v1 = Some(a);
        } else {
            // Helix terminated. Short-helix fallback.
            if last < 2 {
                if let (Some(i2), Some(i3)) = (v2, v3) {
                    let mut t0 = gps[i2].position - gps[a].position;
                    t0 = normalize_safe_or_z(t0);
                    let mut t1 = gps[i3].position - gps[i2].position;
                    t1 = normalize_safe_or_z(t1);
                    t0 += t1;
                    if let Some(i4) = v4 {
                        t1 = gps[i4].position - gps[i3].position;
                        t1 = normalize_safe_or_z(t1);
                        t0 += t1;
                    }
                    if let Some(i5) = v5 {
                        if let Some(i4) = v4 {
                            t1 = gps[i5].position - gps[i4].position;
                            t1 = normalize_safe_or_z(t1);
                            t0 += t1;
                        }
                    }
                    t0 = normalize_safe_or_z(t0);
                    if a >= 1 {
                        gps[a - 1].orientation = normalize_safe_or_z(t0.cross(tv[a - 1]));
                    }
                    if a >= 2 {
                        gps[a - 2].orientation = normalize_safe_or_z(t0.cross(tv[a - 2]));
                    }
                    if v4.is_some() && a >= 3 {
                        gps[a - 3].orientation = normalize_safe_or_z(t0.cross(tv[a - 3]));
                    }
                    if v5.is_some() && a >= 4 {
                        gps[a - 4].orientation = normalize_safe_or_z(t0.cross(tv[a - 4]));
                    }
                    if v4.is_some()
                        && v5.is_some()
                        && a >= 4
                        && gps[a - 3].orientation.dot(gps[a - 4].orientation) < -0.8
                    {
                        gps[a - 4].orientation *= -1.0;
                    }
                }
            }
            v1 = None;
            v2 = None;
            v3 = None;
            v4 = None;
            v5 = None;
            last = 0;
        }

        if let (Some(i1), Some(i2), Some(i3), Some(i4)) = (v1, v2, v3, v4) {
            let t0_raw = gps[i1].position + gps[i4].position;
            let t1_raw = gps[i2].position + gps[i3].position;
            let center = t0_raw * 0.2130 + t1_raw * 0.2870;
            if last > 0 {
                let axis = normalize_safe_or_z(prev_center - center);
                gps[a].orientation = normalize_safe_or_z(axis.cross(tv[a]));
                if a >= 1 {
                    gps[a - 1].orientation = normalize_safe_or_z(axis.cross(tv[a - 1]));
                }
                if a >= 2 {
                    gps[a - 2].orientation = normalize_safe_or_z(axis.cross(tv[a - 2]));
                }
                if last == 1 {
                    if a >= 3 {
                        gps[a - 3].orientation = normalize_safe_or_z(axis.cross(tv[a - 3]));
                    }
                    if a >= 4 && i4 != usize::MAX {
                        // Only apply to position v4 if v4 is set.
                        gps[a - 4].orientation = normalize_safe_or_z(axis.cross(tv[a - 4]));
                    }
                }
            }
            prev_center = center;
            last += 1;
        }
    }
}

// ============================================================================
// Refine normals.
// ============================================================================

fn refine_normals(gps: &mut [GuidePoint], tv: &[Vec3], nv: &[Vec3]) {
    let n = gps.len();
    if n < 3 {
        return;
    }
    // Step 1: orientation ⊥ tangent for interior.
    for a in 1..n - 1 {
        let orient = gps[a].orientation;
        let tangent = tv[a];
        let removed = orient - tangent * orient.dot(tangent);
        let mag = removed.magnitude();
        if mag > 1e-6 {
            gps[a].orientation = removed / mag;
        }
    }
    // Step 2: alternatives [orig, -orig]; helices keep only orig.
    let mut alternatives: Vec<[Vec3; 2]> = Vec::with_capacity(n);
    for gp in gps.iter().take(n) {
        let orient = gp.orientation;
        let alt = if is_helix(gp.ss_type) {
            orient
        } else {
            orient * -1.0
        };
        alternatives.push([orient, alt]);
    }
    // Step 3: greedy forward selection.
    for a in 1..n - 1 {
        let chain_dir = if a >= 1 && a - 1 < nv.len() {
            nv[a - 1]
        } else {
            tv[a]
        };
        let prev_orient = gps[a - 1].orientation;
        let o0 = prev_orient - chain_dir * prev_orient.dot(chain_dir);
        let o0_len = o0.magnitude();
        let o0 = if o0_len > 1e-6 {
            o0 / o0_len
        } else {
            prev_orient
        };

        let c0 = alternatives[a][0];
        let o1_0 = c0 - chain_dir * c0.dot(chain_dir);
        let o1_0_len = o1_0.magnitude();
        let o1_0 = if o1_0_len > 1e-6 { o1_0 / o1_0_len } else { c0 };

        let c1 = alternatives[a][1];
        let o1_1 = c1 - chain_dir * c1.dot(chain_dir);
        let o1_1_len = o1_1.magnitude();
        let o1_1 = if o1_1_len > 1e-6 { o1_1 / o1_1_len } else { c1 };

        let dot0 = o0.dot(o1_0);
        let dot1 = o0.dot(o1_1);
        if dot1 > dot0 {
            gps[a].orientation = c1;
        } else {
            gps[a].orientation = c0;
        }
    }
    // Step 4: kink softening.
    for a in 1..n - 1 {
        let curr = gps[a].orientation;
        let prev = gps[a - 1].orientation;
        let next = gps[a + 1].orientation;
        let dp = curr.dot(next) * curr.dot(prev);
        if dp < -0.10 {
            let tangent = tv[a];
            let mut t0 = next + prev;
            let t1 = curr * 0.001;
            t0 += t1;
            t0 = t0 - tangent * t0.dot(tangent);
            let t0_len = t0.magnitude();
            if t0_len > 1e-6 {
                let t0_norm = t0 / t0_len;
                let t2 = if curr.dot(t0_norm) < 0.0 {
                    curr - t0_norm
                } else {
                    curr + t0_norm
                };
                let t2 = normalize_safe_or_z(t2);
                let blend = (2.0 * (-0.10 - dp)).clamp(0.0, 1.0);
                let result = curr * (1.0 - blend) + t2 * blend;
                alternatives[a][0] = result;
            } else {
                alternatives[a][0] = curr;
            }
        } else {
            alternatives[a][0] = curr;
        }
    }
    for a in 1..n - 1 {
        gps[a].orientation = alternatives[a][0];
    }
}

// ============================================================================
// Flatten sheets.
// ============================================================================

fn flatten_sheets(gps: &mut [GuidePoint], flat_cycles: u32) {
    let n = gps.len();
    if n < 3 {
        return;
    }
    let runs = find_sheet_runs(gps);
    for (first, last) in runs {
        let f = 1usize;
        for _ in 0..flat_cycles {
            let mut tmp_pos = vec![Vec3::new(0.0, 0.0, 0.0); n];
            let mut tmp_orient = vec![Vec3::new(0.0, 0.0, 0.0); n];
            for (b, slot) in tmp_pos
                .iter_mut()
                .enumerate()
                .take(last.saturating_sub(f) + 1)
                .skip(first + f)
            {
                let mut sum = Vec3::new(0.0, 0.0, 0.0);
                for gp in &gps[(b - f)..=(b + f)] {
                    sum += gp.position;
                }
                *slot = sum / (2 * f + 1) as f32;
            }
            for b in (first + f)..=(last.saturating_sub(f)) {
                gps[b].position = tmp_pos[b];
            }
            for (b, slot) in tmp_orient
                .iter_mut()
                .enumerate()
                .take(last.saturating_sub(f) + 1)
                .skip(first + f)
            {
                let mut sum = Vec3::new(0.0, 0.0, 0.0);
                for gp in &gps[(b - f)..=(b + f)] {
                    sum += gp.orientation;
                }
                *slot = sum / (2 * f + 1) as f32;
            }
            for b in (first + f)..=(last.saturating_sub(f)) {
                gps[b].orientation = tmp_orient[b];
            }
            for b in (first + f)..=(last.saturating_sub(f)) {
                let prev = if b > 0 { b - 1 } else { 0 };
                let next = (b + 1).min(n - 1);
                let tangent = normalize_safe_or_z(gps[next].position - gps[prev].position);
                let orient = gps[b].orientation;
                let removed = orient - tangent * orient.dot(tangent);
                gps[b].orientation = normalize_safe_or_z(removed);
            }
        }
    }
}

fn find_sheet_runs(gps: &[GuidePoint]) -> Vec<(usize, usize)> {
    let mut runs = Vec::new();
    let mut first: Option<usize> = None;
    for (a, gp) in gps.iter().enumerate() {
        // Beta-sheet only: `flatten_sheets` is tuned to iron out a short
        // (3-10 residue) strand's CA zig-zag into a plane. Nucleic acid
        // guide points are also `is_flat_ribbon()`, but running the same
        // window-averaging over an entire 20-30+ nt strand erases the
        // backbone's natural helical twist -- the exact signal that makes
        // it read as a double helix instead of a flat/straight band.
        if gp.ss_type.is_sheet() {
            if first.is_none() {
                first = Some(a);
            }
        } else if let Some(f) = first {
            runs.push((f, a - 1));
            first = None;
        }
    }
    if let Some(f) = first {
        runs.push((f, gps.len() - 1));
    }
    runs
}

// ============================================================================
// Loop smoothing (`cartoon_smooth_loops`)
// ============================================================================

fn smooth_loop_runs(gps: &mut [GuidePoint], pipeline: &PipelineSettings) {
    if !pipeline.smooth_loops || pipeline.smooth_cycles == 0 || gps.len() < 3 {
        return;
    }

    let mut run_start = 0usize;
    while run_start < gps.len() {
        while run_start < gps.len() && !gps[run_start].ss_type.is_loop() {
            run_start += 1;
        }
        if run_start == gps.len() {
            break;
        }

        let mut run_end = run_start;
        while run_end + 1 < gps.len() && gps[run_end + 1].ss_type.is_loop() {
            run_end += 1;
        }
        smooth_loop_run(gps, run_start, run_end, pipeline);
        run_start = run_end + 1;
    }
}

fn smooth_loop_run(
    gps: &mut [GuidePoint],
    run_start: usize,
    run_end: usize,
    pipeline: &PipelineSettings,
) {
    let len = run_end - run_start + 1;
    if len < 3 {
        return;
    }

    let first_pin = (pipeline.smooth_first as usize).min(len);
    let last_pin = (pipeline.smooth_last as usize).min(len.saturating_sub(first_pin));
    if first_pin + last_pin >= len {
        return;
    }
    let smooth_start = run_start + first_pin;
    let smooth_end = run_end - last_pin;

    for _ in 0..pipeline.smooth_cycles {
        let positions: Vec<Vec3> = gps[run_start..=run_end]
            .iter()
            .map(|gp| gp.position)
            .collect();
        let orientations: Vec<Vec3> = gps[run_start..=run_end]
            .iter()
            .map(|gp| gp.orientation)
            .collect();

        for (idx, gp) in gps
            .iter_mut()
            .enumerate()
            .take(smooth_end + 1)
            .skip(smooth_start)
        {
            let local = idx - run_start;
            let prev = local.saturating_sub(1);
            let next = (local + 1).min(len - 1);
            gp.position = (positions[prev] + positions[local] + positions[next]) / 3.0;
            gp.orientation =
                normalize_safe_or_z(orientations[prev] + orientations[local] + orientations[next]);
        }

        for idx in smooth_start..=smooth_end {
            let prev = if idx > run_start { idx - 1 } else { idx };
            let next = if idx < run_end { idx + 1 } else { idx };
            let tangent = normalize_safe_or_z(gps[next].position - gps[prev].position);
            let orient = gps[idx].orientation;
            let removed = orient - tangent * orient.dot(tangent);
            gps[idx].orientation = normalize_safe_or_z(removed);
        }
    }
}

// ============================================================================
// Sample interpolation.
// ============================================================================

#[derive(Debug, Clone, Copy)]
pub struct ExtrudePoint {
    pub position: Vec3,
    pub orientation: Vec3,
    pub atom_idx: u32,
}

#[allow(clippy::too_many_arguments)]
fn cartoon_generate_sample(
    buffer: &mut Vec<ExtrudePoint>,
    n_p: &mut usize,
    gp1: &GuidePoint,
    gp2: &GuidePoint,
    tv1: &Vec3,
    tv2: &Vec3,
    dev: f32,
    sampling: u32,
    power_a: f32,
    power_b: f32,
    is_run_start: bool,
) {
    let pos1 = gp1.position;
    let pos2 = gp2.position;
    let orient1 = gp1.orientation;
    let orient2 = gp2.orientation;

    for b in 0..sampling {
        if is_run_start && b == 0 {
            // First pair of a run — also emit the t=0 starting point so this
            // run's slice begins exactly at the residue boundary, coinciding
            // with the previous run's last sample (= run-N end == run-(N+1)
            // start at the same residue position).
            let f0_raw = b as f32 / sampling as f32;
            let f0 = smooth(f0_raw, power_a);
            let f1 = 1.0 - f0;
            let f2 = smooth(f0, power_b);
            let f3 = smooth(f1, power_b);
            let f4 = dev * f2 * f3;
            let pos = pos1 * f1 + pos2 * f0 + (*tv1 * f3 - *tv2 * f2) * f4;
            buffer.push(ExtrudePoint {
                position: pos,
                orientation: orient1, // first interpolated point inherits orient1
                atom_idx: gp1.atom_idx,
            });
            *n_p += 1;
        }
        let f0_raw = (b as f32 + 1.0) / sampling as f32;
        let f0 = smooth(f0_raw, power_a);
        let f1 = 1.0 - f0;
        let f2 = smooth(f0, power_b);
        let f3 = smooth(f1, power_b);
        let f4 = dev * f2 * f3;
        let pos = pos1 * f1 + pos2 * f0 + (*tv1 * f3 - *tv2 * f2) * f4;
        let orient = normalize_safe_or_z(orient1 * (f1 * f2) + orient2 * (f0 * f3));
        let atom_idx = if f0_raw <= 0.5 {
            gp1.atom_idx
        } else {
            gp2.atom_idx
        };
        buffer.push(ExtrudePoint {
            position: pos,
            orientation: orient,
            atom_idx,
        });
        // Last sample of the pair — pin orient to orient2.
        if b == sampling - 1 {
            buffer.last_mut().unwrap().orientation = orient2;
        }
        *n_p += 1;
    }
}

fn cartoon_generate_refine(
    buffer: &mut [ExtrudePoint],
    n_p: usize,
    sampling: u32,
    refine_cycles: u32,
    orient_a: &Vec3,
    orient_b: &Vec3,
) {
    let sampling = sampling as usize;
    if sampling <= 1 || refine_cycles == 0 || n_p < sampling {
        return;
    }
    let t0 = orient_a.cross(*orient_b);
    if t0.magnitude() < 1e-4 {
        return;
    }
    let t0 = t0 / t0.magnitude();
    let start = n_p - sampling;
    if start == 0 {
        return;
    }
    let mut tmp = vec![Vec3::new(0.0, 0.0, 0.0); sampling];
    for _ in 0..refine_cycles {
        for (b, tmp_val) in tmp.iter_mut().enumerate().take(sampling - 1) {
            let idx0 = start - 1 + b;
            let idx1 = start + b;
            let idx2 = start + 1 + b;
            let f0 = t0.dot(buffer[idx0].position);
            let f1 = t0.dot(buffer[idx1].position);
            let f2 = t0.dot(buffer[idx2].position);
            let f3 = (f2 + f0) / 2.0;
            *tmp_val = buffer[idx1].position + t0 * (f3 - f1);
        }
        for (b, tmp_val) in tmp.iter().enumerate().take(sampling - 1) {
            buffer[start + b].position = *tmp_val;
        }
    }
}

// ============================================================================
// Run classification (mirror generate_per_run_mesh:1227-1332)
// ============================================================================

/// One run of consecutive same-CartoonType pairs. The GPU compute kernel
/// reads this descriptor, applies type-specific extrusion to the
/// `[sample_start..sample_end]` slice of `ExtrudePoints`, and writes
/// `vertex_count` vertices into the global vertex buffer at
/// `vertex_offset`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct RunDescriptor {
    pub car_type: u32, // CartoonType as u32
    pub sample_start: u32,
    pub sample_end: u32, // INCLUSIVE
    pub vertex_offset: u32,
    pub vertex_count: u32,
    pub body_end: u32, // for Arrow runs: last body sample index (within run); 0 if no arrow
    pub flags: u32,    // bit 0: has_arrow
    pub _pad: u32,
}

pub const RUN_HAS_ARROW: u32 = 1 << 0;

/// CPU-side output of the cartoon pipeline. Uploaded to GPU as two storage
/// buffers (extrude_points + run_descriptors). The GPU
/// `cartoon_extrude.wgsl` kernel reads both and produces the final
/// vertex stream.
pub struct PipelineOutput {
    pub extrude_points: Vec<ExtrudePoint>,
    pub runs: Vec<RunDescriptor>,
    /// Total vertices emitted by all runs. Used by host to allocate the
    /// vertex buffer + write `draw_indirect` args.
    pub total_vertices: u32,
}

/// Sample one backbone segment and classify its samples into per-CartoonType runs.
pub fn process_segment(
    segment: &mut BackboneSegment,
    pipeline: &PipelineSettings,
    geom: &GeomSettings,
    output: &mut PipelineOutput,
) {
    let n = segment.guide_points.len();
    if n < 2 {
        return;
    }

    // Differences, normals, tangents.
    let (_dl0, nv0) = compute_differences_and_normals(&segment.guide_points);
    let tv0 = compute_tangents(&nv0, n);

    // Round helix orientations.
    if pipeline.round_helices {
        compute_round_helix_orientations(&mut segment.guide_points, &tv0);
    }
    // Refine normals.
    if pipeline.refine_normals {
        refine_normals(&mut segment.guide_points, &tv0, &nv0);
    }
    // Flatten sheets.
    flatten_sheets(&mut segment.guide_points, pipeline.flat_cycles);
    // Optional PyMOL-style smoothing for loop-only guide runs.
    smooth_loop_runs(&mut segment.guide_points, pipeline);

    // Recompute differences/tangents after position changes.
    let (dl, _nv) = compute_differences_and_normals(&segment.guide_points);
    // Tangents (computed from final positions) used by `cartoon_generate_sample`
    // — same `compute_tangents` formula as the initial tangent estimate.
    let tv = compute_tangents(&_nv, n);

    // Walk pairs, classify by `cartoon_type_for`, flush runs.
    let gps = &segment.guide_points;
    let mut buffer: Vec<ExtrudePoint> = Vec::new();
    let mut n_p: usize = 0;
    let mut cur_car: Option<CartoonType> = None;
    let mut run_buffer_start: usize = 0;

    let sampling = pipeline.sampling;
    let power_a = pipeline.power_a;
    let power_b = pipeline.power_b;

    let mut a = 0usize;
    while a < n {
        let pair_car = if a < n - 1 {
            let car_a = cartoon_type_for(gps[a].ss_type, geom);
            let car_b = cartoon_type_for(gps[a + 1].ss_type, geom);
            if car_a == car_b {
                car_a
            } else {
                CartoonType::Loop
            }
        } else {
            cartoon_type_for(gps[a].ss_type, geom)
        };

        let type_changed = cur_car.is_some_and(|c| c != pair_car);
        if type_changed && n_p > 0 {
            // Flush this run as a RunDescriptor.
            flush_run(
                &buffer[run_buffer_start..n_p],
                run_buffer_start,
                cur_car.unwrap(),
                geom,
                output,
            );
            run_buffer_start = n_p;
        }
        cur_car = Some(pair_car);

        if a < n - 1 {
            let dev = pipeline.throw_factor * dl[a];
            let is_run_start = n_p == run_buffer_start;
            cartoon_generate_sample(
                &mut buffer,
                &mut n_p,
                &gps[a],
                &gps[a + 1],
                &tv[a],
                &tv[a + 1],
                dev,
                sampling,
                power_a,
                power_b,
                is_run_start,
            );
            // Refine — only for same-type pairs (boundary pairs collapse to Loop).
            let car_a = cartoon_type_for(gps[a].ss_type, geom);
            let car_b = cartoon_type_for(gps[a + 1].ss_type, geom);
            if pipeline.refine > 0 && car_a == car_b {
                cartoon_generate_refine(
                    &mut buffer,
                    n_p,
                    sampling,
                    pipeline.refine,
                    &gps[a].orientation,
                    &gps[a + 1].orientation,
                );
            }
        }
        a += 1;
    }

    // Final run.
    if n_p > 0 {
        if let Some(car) = cur_car {
            flush_run(
                &buffer[run_buffer_start..n_p],
                run_buffer_start,
                car,
                geom,
                output,
            );
        }
    }

    output.extrude_points.extend_from_slice(&buffer);
}

/// Convert a flushed run buffer to a `RunDescriptor`, computing vertex
/// count from CartoonType. `run_buffer_start` is the run's offset within
/// the current segment's buffer; combined with the global pool's current
/// length it gives the global `sample_start` index.
fn flush_run(
    run_samples: &[ExtrudePoint],
    run_buffer_start: usize,
    car_type: CartoonType,
    geom: &GeomSettings,
    output: &mut PipelineOutput,
) {
    let len = run_samples.len();
    if len < 2 {
        return;
    }
    let sample_count = len as u32;
    let sample_start = output.extrude_points.len() as u32 + run_buffer_start as u32;
    // The GPU emits vertices from run descriptors; CPU-side counts reserve
    // exactly enough output space for each cartoon segment.
    let (vertex_count, body_end_in_run, flags) = compute_run_vertex_layout(car_type, len, geom);

    let descriptor = RunDescriptor {
        car_type: car_type as u32,
        sample_start,
        sample_end: sample_start + sample_count - 1,
        vertex_offset: output.total_vertices,
        vertex_count,
        body_end: body_end_in_run,
        flags,
        _pad: 0,
    };
    output.runs.push(descriptor);
    output.total_vertices += vertex_count;
}

/// Compute vertex count and arrow split for a run. Vertex counts are tight
/// so the GPU emits exactly `vertex_count` triangles per run.
fn compute_run_vertex_layout(
    car_type: CartoonType,
    sample_count: usize,
    geom: &GeomSettings,
) -> (u32, u32, u32) {
    let n = sample_count;
    if n < 2 {
        return (0, 0, 0);
    }
    let n_minus_1 = n - 1; // pair count
    match car_type {
        CartoonType::Loop | CartoonType::Oval => {
            let q = geom.quality as usize;
            // Tube: per-pair perimeter quad strip = q quads × 6 verts/quad.
            // Plus 2 caps (start + end) = 2 × q × 3 verts (triangle fan from center).
            let body = n_minus_1 * q * 6;
            let caps = 2 * q * 3;
            (body as u32 + caps as u32, 0, 0)
        }
        CartoonType::Rect => {
            // Sheet body uses four fixed face strips per pair
            // (top/bottom/left/right, each one quad). Add terminal caps.
            let body = n_minus_1 * 4 * 6;
            let caps = 2 * 6;
            (body as u32 + caps as u32, 0, 0)
        }
        CartoonType::Arrow => {
            // Sheet body up to body_end, then arrow head, back cap, and
            // shoulder caps. Estimate arrow length from residue count and
            // default sampling density.
            let arrow_len = (geom.arrow_residues as usize) * 8; // sampling+1=8 default
            let arrow_len = arrow_len.min(n / 2);
            let arrow_len = arrow_len.max(2); // at least 2
            let body_pairs = n - 1 - arrow_len + 1; // pairs in body region
            let arrow_pairs = arrow_len - 1; // arrow segments
            let body_verts = body_pairs * 4 * 6;
            let arrow_verts = arrow_pairs * 4 * 6;
            let n_cap = 6; // back cap (quad)
            let shoulders = 2 * 6; // 2 shoulder quads
            let body_end_in_run = (body_pairs) as u32; // sample idx where arrow starts
            (
                (body_verts + arrow_verts + n_cap + shoulders) as u32,
                body_end_in_run,
                RUN_HAS_ARROW,
            )
        }
        CartoonType::NucleicRect => {
            // Same as Rect (no arrow).
            let body = n_minus_1 * 4 * 6;
            let caps = 2 * 6;
            (body as u32 + caps as u32, 0, 0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cartoon_type_for_helix_oval() {
        let geom = GeomSettings::default();
        assert_eq!(
            cartoon_type_for(SecondaryStructure::Helix, &geom),
            CartoonType::Oval
        );
    }

    #[test]
    fn cartoon_type_for_sheet_arrow_default() {
        let geom = GeomSettings::default();
        assert_eq!(
            cartoon_type_for(SecondaryStructure::Sheet, &geom),
            CartoonType::Arrow
        );
    }

    #[test]
    fn cartoon_type_for_loop() {
        let geom = GeomSettings::default();
        assert_eq!(
            cartoon_type_for(SecondaryStructure::Loop, &geom),
            CartoonType::Loop
        );
    }

    #[test]
    fn ss_from_u8_roundtrips_all_variants() {
        let variants = [
            SecondaryStructure::Loop,
            SecondaryStructure::Helix,
            SecondaryStructure::Sheet,
            SecondaryStructure::Helix310,
            SecondaryStructure::HelixPi,
            SecondaryStructure::Turn,
            SecondaryStructure::Bend,
            SecondaryStructure::NucleicRibbon,
        ];
        for ss in variants {
            assert_eq!(ss_from_u8(ss as u8), ss, "round-trip failed for {ss:?}");
        }
    }

    #[test]
    fn find_sheet_runs_excludes_nucleic_ribbon() {
        // A run of NucleicRibbon guide points must not be treated as a
        // sheet run -- flatten_sheets' window-averaging is tuned for a
        // short beta-strand zig-zag and would iron out a nucleic strand's
        // helical twist if applied to it.
        let gps: Vec<GuidePoint> = (0..20)
            .map(|i| GuidePoint {
                position: Vec3::new(i as f32, 0.0, 0.0),
                orientation: Vec3::new(0.0, 0.0, 1.0),
                ss_type: SecondaryStructure::NucleicRibbon,
                atom_idx: i,
            })
            .collect();
        assert!(find_sheet_runs(&gps).is_empty());
    }

    #[test]
    fn find_sheet_runs_still_finds_sheet() {
        let gps: Vec<GuidePoint> = (0..5)
            .map(|i| GuidePoint {
                position: Vec3::new(i as f32, 0.0, 0.0),
                orientation: Vec3::new(0.0, 0.0, 1.0),
                ss_type: SecondaryStructure::Sheet,
                atom_idx: i,
            })
            .collect();
        assert_eq!(find_sheet_runs(&gps), vec![(0, 4)]);
    }

    fn loop_gp(x: f32, y: f32, atom_idx: u32) -> GuidePoint {
        GuidePoint {
            position: Vec3::new(x, y, 0.0),
            orientation: Vec3::new(0.0, 0.0, 1.0),
            ss_type: SecondaryStructure::Loop,
            atom_idx,
        }
    }

    fn assert_vec3_close(actual: Vec3, expected: Vec3) {
        let delta = actual - expected;
        assert!(
            delta.magnitude() < 1e-6,
            "expected {:?}, got {:?}",
            expected,
            actual
        );
    }

    #[test]
    fn smooth_loop_runs_off_leaves_loop_positions_unchanged() {
        let mut guide_points = vec![
            loop_gp(0.0, 0.0, 0),
            loop_gp(1.0, 3.0, 1),
            loop_gp(2.0, 0.0, 2),
            loop_gp(3.0, 3.0, 3),
            loop_gp(4.0, 0.0, 4),
        ];
        let before: Vec<Vec3> = guide_points.iter().map(|gp| gp.position).collect();

        smooth_loop_runs(
            &mut guide_points,
            &PipelineSettings {
                smooth_loops: false,
                smooth_cycles: 1,
                smooth_first: 1,
                smooth_last: 1,
                ..PipelineSettings::default()
            },
        );

        for (gp, expected) in guide_points.iter().zip(before) {
            assert_vec3_close(gp.position, expected);
        }
    }

    #[test]
    fn smooth_loop_runs_on_moves_interior_toward_average() {
        let mut guide_points = vec![
            loop_gp(0.0, 0.0, 0),
            loop_gp(1.0, 3.0, 1),
            loop_gp(2.0, 0.0, 2),
            loop_gp(3.0, 3.0, 3),
            loop_gp(4.0, 0.0, 4),
        ];

        smooth_loop_runs(
            &mut guide_points,
            &PipelineSettings {
                smooth_loops: true,
                smooth_cycles: 1,
                smooth_first: 1,
                smooth_last: 1,
                ..PipelineSettings::default()
            },
        );

        assert_vec3_close(guide_points[0].position, Vec3::new(0.0, 0.0, 0.0));
        assert_vec3_close(guide_points[4].position, Vec3::new(4.0, 0.0, 0.0));
        assert!((guide_points[1].position.y - 1.0).abs() < 1e-6);
        assert!((guide_points[2].position.y - 2.0).abs() < 1e-6);
        assert!((guide_points[3].position.y - 1.0).abs() < 1e-6);
    }

    #[test]
    fn smooth_loop_runs_respects_first_last_pin_counts() {
        let mut guide_points = vec![
            loop_gp(0.0, 0.0, 0),
            loop_gp(1.0, 4.0, 1),
            loop_gp(2.0, 0.0, 2),
            loop_gp(3.0, 4.0, 3),
            loop_gp(4.0, 0.0, 4),
            loop_gp(5.0, 4.0, 5),
        ];
        let before: Vec<Vec3> = guide_points.iter().map(|gp| gp.position).collect();

        smooth_loop_runs(
            &mut guide_points,
            &PipelineSettings {
                smooth_loops: true,
                smooth_cycles: 1,
                smooth_first: 2,
                smooth_last: 2,
                ..PipelineSettings::default()
            },
        );

        for idx in [0usize, 1, 4, 5] {
            assert_vec3_close(guide_points[idx].position, before[idx]);
        }
        assert!((guide_points[2].position.y - before[2].y).abs() > 1e-6);
        assert!((guide_points[3].position.y - before[3].y).abs() > 1e-6);
    }

    #[test]
    fn smooth_loop_runs_all_pinned_is_noop() {
        let mut guide_points = vec![
            loop_gp(0.0, 0.0, 0),
            loop_gp(1.0, 4.0, 1),
            loop_gp(2.0, 0.0, 2),
        ];
        let before: Vec<Vec3> = guide_points.iter().map(|gp| gp.position).collect();

        smooth_loop_runs(
            &mut guide_points,
            &PipelineSettings {
                smooth_loops: true,
                smooth_cycles: 1,
                smooth_first: 3,
                smooth_last: 3,
                ..PipelineSettings::default()
            },
        );

        for (gp, expected) in guide_points.iter().zip(before) {
            assert_vec3_close(gp.position, expected);
        }
    }
}
