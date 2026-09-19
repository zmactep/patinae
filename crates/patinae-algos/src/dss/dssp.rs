//! DSSP secondary structure assignment algorithm
//!
//! Implements the Kabsch & Sander DSSP algorithm for secondary structure
//! assignment based on hydrogen bond energy patterns.
//!
//! The algorithm computes backbone N-H···O=C hydrogen bonds using an
//! electrostatic energy model, then classifies residues into helices, strands,
//! turns, and bends based on H-bond patterns. Results are quantized to
//! helix/sheet/loop labels used by the rest of the workspace.
//!
//! Spatial hashing on carbonyl O atoms gives O(n) average-case H-bond
//! detection for typical protein packing densities.
//!
//! # References
//!
//! - Kabsch W, Sander C (1983). "Dictionary of protein secondary structure:
//!   pattern recognition of hydrogen-bonded and geometrical features."
//!   Biopolymers 22(12):2577-637.

use std::collections::HashMap;

use lin_alg::f32::Vec3;

use super::{BackboneResidue, SecondaryStructureAssigner, SsType};

// ============================================================================
// Constants
// ============================================================================

/// Electrostatic coupling constant: q1 × q2 × f = 0.20 × 0.42 × 332 kcal/mol
const COUPLING: f32 = 27.888;

/// Standard N-H bond length in Angstroms
const NH_BOND_LENGTH: f32 = 1.008;

/// Minimum interatomic distance to avoid division by near-zero
const MIN_DIST: f32 = 0.5;

/// Spatial hash grid cell size (Angstroms)
const SPATIAL_CELL_SIZE: f32 = 5.5;

/// Bend angle threshold in degrees
const BEND_ANGLE_CUTOFF: f32 = 70.0;

/// Number of dummy residues inserted at chain boundaries
const CHAIN_BREAK_PAD: usize = 5;

// ============================================================================
// DsspParams
// ============================================================================

/// Parameters for the DSSP algorithm
#[derive(Debug, Clone)]
pub struct DsspParams {
    /// Hydrogen bond energy cutoff in kcal/mol (Kabsch & Sander default: -0.5)
    pub hbond_energy_cutoff: f32,
}

impl Default for DsspParams {
    fn default() -> Self {
        Self {
            hbond_energy_cutoff: -0.5,
        }
    }
}

// ============================================================================
// Internal Enums
// ============================================================================

/// Fine-grained DSSP secondary structure state.
///
/// Variants are ordered by DSSP priority (lowest to highest). The derived
/// `Ord` gives the correct priority so `max()` preserves higher-priority
/// assignments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
enum DsspState {
    #[default]
    Empty, // ' ' (coil)
    Bend,     // S
    Turn,     // T
    Pi,       // I (5-helix)
    ThreeTen, // G (3₁₀-helix)
    Strand,   // E (extended strand in ladder)
    Bridge,   // B (isolated beta-bridge)
    Helix,    // H (4-helix, alpha)
}

impl DsspState {
    fn to_ss_type(self) -> SsType {
        match self {
            DsspState::Helix | DsspState::ThreeTen | DsspState::Pi => SsType::Helix,
            DsspState::Strand | DsspState::Bridge => SsType::Sheet,
            _ => SsType::Loop,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BridgeType {
    Parallel,
    Antiparallel,
}

// ============================================================================
// H-Bond Storage
// ============================================================================

/// One slot in the best-2 H-bond list per residue role
#[derive(Debug, Clone, Copy)]
struct HBondSlot {
    partner: usize,
    energy: f32,
}

const NO_HBOND: HBondSlot = HBondSlot {
    partner: usize::MAX,
    energy: 0.0,
};

/// Insert an H-bond into a best-2 sorted pair (lowest energy first).
fn try_insert_hbond(slots: &mut [HBondSlot; 2], partner: usize, energy: f32) {
    if energy < slots[0].energy {
        slots[1] = slots[0];
        slots[0] = HBondSlot { partner, energy };
    } else if energy < slots[1].energy {
        slots[1] = HBondSlot { partner, energy };
    }
}

// ============================================================================
// Per-Residue Working Data
// ============================================================================

#[derive(Debug, Clone)]
struct DsspResidue {
    n: Option<Vec3>,
    ca: Option<Vec3>,
    c: Option<Vec3>,
    o: Option<Vec3>,
    /// Computed H position (N + nh_dir × 1.008)
    h: Option<Vec3>,

    /// Index into the original input slice (None for padding dummies)
    input_idx: Option<usize>,
    real: bool,

    /// Two best NH groups forming H-bonds with this residue's CO
    co_acceptor: [HBondSlot; 2],
    /// Two best CO groups forming H-bonds with this residue's NH
    nh_donor: [HBondSlot; 2],

    turn_3: bool,
    turn_4: bool,
    turn_5: bool,

    bridge_partner_1: Option<(usize, BridgeType)>,
    bridge_partner_2: Option<(usize, BridgeType)>,

    ss: DsspState,
}

impl Default for DsspResidue {
    fn default() -> Self {
        Self {
            n: None,
            ca: None,
            c: None,
            o: None,
            h: None,
            input_idx: None,
            real: false,
            co_acceptor: [NO_HBOND; 2],
            nh_donor: [NO_HBOND; 2],
            turn_3: false,
            turn_4: false,
            turn_5: false,
            bridge_partner_1: None,
            bridge_partner_2: None,
            ss: DsspState::Empty,
        }
    }
}

// ============================================================================
// Spatial Hash
// ============================================================================

struct SpatialHash {
    cells: HashMap<(i32, i32, i32), Vec<usize>>,
    cell_size: f32,
}

impl SpatialHash {
    fn new(cell_size: f32, capacity: usize) -> Self {
        Self {
            cells: HashMap::with_capacity(capacity),
            cell_size,
        }
    }

    fn cell_key(&self, pos: Vec3) -> (i32, i32, i32) {
        (
            (pos.x / self.cell_size).floor() as i32,
            (pos.y / self.cell_size).floor() as i32,
            (pos.z / self.cell_size).floor() as i32,
        )
    }

    fn insert(&mut self, pos: Vec3, idx: usize) {
        let key = self.cell_key(pos);
        self.cells.entry(key).or_default().push(idx);
    }

    fn query_neighbors(&self, pos: Vec3, out: &mut Vec<usize>) {
        out.clear();
        let (cx, cy, cz) = self.cell_key(pos);
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some(indices) = self.cells.get(&(cx + dx, cy + dy, cz + dz)) {
                        out.extend_from_slice(indices);
                    }
                }
            }
        }
    }
}

// ============================================================================
// Chain-Break Padding
// ============================================================================

fn collect_with_breaks(residues: &[BackboneResidue]) -> Vec<DsspResidue> {
    if residues.is_empty() {
        return Vec::new();
    }

    let mut result = Vec::with_capacity(residues.len() + 2 * CHAIN_BREAK_PAD + 10);

    // Initial padding
    for _ in 0..CHAIN_BREAK_PAD {
        result.push(DsspResidue::default());
    }

    for (i, r) in residues.iter().enumerate() {
        // Insert chain-break padding when not bonded to previous
        if i > 0 && !r.bonded_to_prev {
            for _ in 0..CHAIN_BREAK_PAD {
                result.push(DsspResidue::default());
            }
        }

        // Compute H position
        let h = if let Some(dir) = r.nh_direction {
            Some(r.n + dir * NH_BOND_LENGTH)
        } else if r.bonded_to_prev {
            // Find the previous real residue's C position
            let prev_c = result
                .iter()
                .rev()
                .find_map(|prev| if prev.real { prev.c } else { None });
            prev_c.map(|c_prev| {
                let diff = r.n - c_prev;
                let mag = diff.magnitude();
                if mag > 1e-6 {
                    r.n + diff * (NH_BOND_LENGTH / mag)
                } else {
                    r.n
                }
            })
        } else {
            None
        };

        result.push(DsspResidue {
            n: Some(r.n),
            ca: Some(r.ca),
            c: Some(r.c),
            o: Some(r.o),
            h,
            input_idx: Some(i),
            real: true,
            ..Default::default()
        });
    }

    // Final padding
    for _ in 0..CHAIN_BREAK_PAD {
        result.push(DsspResidue::default());
    }

    result
}

// ============================================================================
// H-Bond Energy
// ============================================================================

/// Kabsch & Sander electrostatic H-bond energy.
///
/// `donor` is the residue donating N-H, `acceptor` provides C=O.
/// Returns energy in kcal/mol (negative = favorable).
fn hbond_energy(res: &[DsspResidue], donor: usize, acceptor: usize) -> f32 {
    let (n, h) = match (res[donor].n, res[donor].h) {
        (Some(n), Some(h)) => (n, h),
        _ => return 0.0,
    };
    let (c, o) = match (res[acceptor].c, res[acceptor].o) {
        (Some(c), Some(o)) => (c, o),
        _ => return 0.0,
    };

    let r_on = (o - n).magnitude();
    let r_ch = (c - h).magnitude();
    let r_oh = (o - h).magnitude();
    let r_cn = (c - n).magnitude();

    if r_on < MIN_DIST || r_ch < MIN_DIST || r_oh < MIN_DIST || r_cn < MIN_DIST {
        return 0.0;
    }

    COUPLING * (1.0 / r_on + 1.0 / r_ch - 1.0 / r_oh - 1.0 / r_cn)
}

// ============================================================================
// H-Bond Detection
// ============================================================================

fn detect_hbonds(res: &mut [DsspResidue], cutoff: f32) {
    let n_res = res.len();

    // Build spatial grid on O positions (acceptor atoms)
    let mut grid = SpatialHash::new(SPATIAL_CELL_SIZE, n_res);
    for (j, r) in res.iter().enumerate() {
        if let Some(o) = r.o {
            grid.insert(o, j);
        }
    }

    // Collect all valid H-bonds, then apply them
    // (avoids borrowing issues with mutable res)
    let mut hbonds: Vec<(usize, usize, f32)> = Vec::new(); // (donor, acceptor, energy)
    let mut neighbors = Vec::new();

    for i in 0..n_res {
        let n_pos = match res[i].n {
            Some(p) if res[i].h.is_some() => p,
            _ => continue,
        };

        grid.query_neighbors(n_pos, &mut neighbors);

        for &j in &neighbors {
            if i.abs_diff(j) < 2 {
                continue;
            }
            if !res[j].real {
                continue;
            }

            let e = hbond_energy(res, i, j);
            if e < cutoff {
                hbonds.push((i, j, e));
            }
        }
    }

    // Apply H-bonds to residue slots
    for &(donor, acceptor, energy) in &hbonds {
        try_insert_hbond(&mut res[acceptor].co_acceptor, donor, energy);
        try_insert_hbond(&mut res[donor].nh_donor, acceptor, energy);
    }
}

// ============================================================================
// H-Bond Query Helper
// ============================================================================

/// Check if an H-bond CO(co_res) → NH(nh_res) exists.
fn has_hbond(res: &[DsspResidue], co_res: usize, nh_res: usize) -> bool {
    let n = res.len();
    if co_res >= n || nh_res >= n {
        return false;
    }
    // Check acceptor side: co_res's CO accepted by nh_res
    for slot in &res[co_res].co_acceptor {
        if slot.partner == nh_res {
            return true;
        }
    }
    // Check donor side: nh_res's NH donated to co_res
    for slot in &res[nh_res].nh_donor {
        if slot.partner == co_res {
            return true;
        }
    }
    false
}

// ============================================================================
// Turn Detection
// ============================================================================

fn detect_turns(res: &mut [DsspResidue]) {
    let n = res.len();
    for i in 0..n {
        if !res[i].real {
            continue;
        }
        if i + 3 < n && res[i + 3].real && has_hbond(res, i, i + 3) {
            res[i].turn_3 = true;
        }
        if i + 4 < n && res[i + 4].real && has_hbond(res, i, i + 4) {
            res[i].turn_4 = true;
        }
        if i + 5 < n && res[i + 5].real && has_hbond(res, i, i + 5) {
            res[i].turn_5 = true;
        }
    }
}

// ============================================================================
// Helix Assignment
// ============================================================================

fn assign_helices(res: &mut [DsspResidue]) {
    let n = res.len();

    // Process helix types in priority order: α (4) > 3₁₀ (3) > π (5)
    for &(turn_n, state) in &[
        (4usize, DsspState::Helix),
        (3, DsspState::ThreeTen),
        (5, DsspState::Pi),
    ] {
        let get_turn = |r: &DsspResidue, tn: usize| match tn {
            3 => r.turn_3,
            4 => r.turn_4,
            5 => r.turn_5,
            _ => false,
        };

        // Mark helix from consecutive turns
        for i in 0..n.saturating_sub(1) {
            if !res[i].real {
                continue;
            }
            if get_turn(&res[i], turn_n) && i + 1 < n && get_turn(&res[i + 1], turn_n) {
                // Residues i+1 through i+turn_n are in this helix
                for r in &mut res[(i + 1)..=(i + turn_n).min(n - 1)] {
                    if r.real {
                        r.ss = r.ss.max(state);
                    }
                }
            }
        }
    }

    // Mark turns: residues inside a turn that aren't part of a helix
    for i in 0..n {
        if !res[i].real {
            continue;
        }
        for &(turn_n, is_turn) in &[
            (3usize, res[i].turn_3),
            (4, res[i].turn_4),
            (5, res[i].turn_5),
        ] {
            if !is_turn {
                continue;
            }
            let end = (i + turn_n).min(n);
            for r in &mut res[(i + 1)..end] {
                if r.real && r.ss < DsspState::Turn {
                    r.ss = DsspState::Turn;
                }
            }
        }
    }
}

// ============================================================================
// Bridge Detection
// ============================================================================

fn store_bridge(res: &mut DsspResidue, partner: usize, btype: BridgeType) {
    if res.bridge_partner_1.is_none() {
        res.bridge_partner_1 = Some((partner, btype));
    } else if res.bridge_partner_2.is_none() {
        res.bridge_partner_2 = Some((partner, btype));
    }
}

fn detect_bridges(res: &mut [DsspResidue]) {
    let n = res.len();

    // Collect bridges first to avoid borrow issues
    let mut bridges: Vec<(usize, usize, BridgeType)> = Vec::new();

    for i in CHAIN_BREAK_PAD..n.saturating_sub(CHAIN_BREAK_PAD) {
        if !res[i].real {
            continue;
        }

        // Gather candidate j values from i and its neighbors' H-bond partners.
        // Bridge conditions reference i-1, i, i+1, so we must scan H-bond lists
        // of all three, plus ±1 offsets on the partner side (for shifted conditions
        // like CO(i-1)→NH(j) where j is the partner of i-1, not i).
        let mut candidates = Vec::new();
        for &offset in &[i.wrapping_sub(1), i, i + 1] {
            if offset < n && res[offset].real {
                for slot in res[offset]
                    .co_acceptor
                    .iter()
                    .chain(res[offset].nh_donor.iter())
                {
                    if slot.partner != usize::MAX {
                        for &cand in &[slot.partner.wrapping_sub(1), slot.partner, slot.partner + 1]
                        {
                            if cand < n {
                                candidates.push(cand);
                            }
                        }
                    }
                }
            }
        }
        candidates.sort_unstable();
        candidates.dedup();

        for j in candidates {
            if j <= i + 2 || j >= n || !res[j].real {
                continue;
            }

            // Parallel bridge: [CO(i-1)→NH(j) && CO(j)→NH(i+1)]
            //                OR [CO(j-1)→NH(i) && CO(i)→NH(j+1)]
            if (has_hbond(res, i - 1, j) && i + 1 < n && has_hbond(res, j, i + 1))
                || (has_hbond(res, j - 1, i) && j + 1 < n && has_hbond(res, i, j + 1))
            {
                bridges.push((i, j, BridgeType::Parallel));
            }

            // Antiparallel bridge: [CO(i)→NH(j) && CO(j)→NH(i)]
            //                    OR [CO(i-1)→NH(j+1) && CO(j-1)→NH(i+1)]
            if (has_hbond(res, i, j) && has_hbond(res, j, i))
                || (j + 1 < n
                    && i + 1 < n
                    && has_hbond(res, i - 1, j + 1)
                    && has_hbond(res, j - 1, i + 1))
            {
                bridges.push((i, j, BridgeType::Antiparallel));
            }
        }
    }

    // Deduplicate: sort and dedup by (min(i,j), max(i,j), type)
    bridges.sort_by_key(|&(i, j, bt)| (i.min(j), i.max(j), bt as u8));
    bridges.dedup_by_key(|b| (b.0.min(b.1), b.0.max(b.1), b.2 as u8));

    // Store bridge partners on residues
    for &(i, j, bt) in &bridges {
        store_bridge(&mut res[i], j, bt);
        store_bridge(&mut res[j], i, bt);
    }
}

// ============================================================================
// Ladder / Sheet Assignment
// ============================================================================

fn assign_sheets(res: &mut [DsspResidue]) {
    // Collect all bridge pairs
    let mut bridges: Vec<(usize, usize, BridgeType)> = Vec::new();
    for (i, r) in res.iter().enumerate() {
        if let Some((j, bt)) = r.bridge_partner_1 {
            if i < j {
                bridges.push((i, j, bt));
            }
        }
        if let Some((j, bt)) = r.bridge_partner_2 {
            if i < j {
                bridges.push((i, j, bt));
            }
        }
    }
    bridges.sort_by_key(|&(i, j, _)| (i, j));
    bridges.dedup();

    // Group into ladders: consecutive bridges with same type and adjacent partners
    let mut in_ladder = vec![false; bridges.len()];

    for a in 0..bridges.len() {
        for b in (a + 1)..bridges.len() {
            let (i1, j1, bt1) = bridges[a];
            let (i2, j2, bt2) = bridges[b];
            if bt1 != bt2 {
                continue;
            }
            // Consecutive in first strand
            if i2 != i1 + 1 {
                // Since bridges are sorted by i, no further match for this a
                if i2 > i1 + 1 {
                    break;
                }
                continue;
            }
            // Adjacent in second strand (parallel: +1, antiparallel: -1)
            let adjacent = match bt1 {
                BridgeType::Parallel => j2 == j1 + 1,
                BridgeType::Antiparallel => j1 > 0 && j2 == j1 - 1,
            };
            if adjacent {
                in_ladder[a] = true;
                in_ladder[b] = true;
            }
        }
    }

    // Assign states
    for (idx, &(i, j, _)) in bridges.iter().enumerate() {
        let state = if in_ladder[idx] {
            DsspState::Strand
        } else {
            DsspState::Bridge
        };
        if res[i].ss < state {
            res[i].ss = state;
        }
        if res[j].ss < state {
            res[j].ss = state;
        }
    }
}

// ============================================================================
// Bend Detection
// ============================================================================

fn detect_bends(res: &mut [DsspResidue]) {
    let n = res.len();
    if n < 5 {
        return;
    }

    for i in 2..n - 2 {
        if !res[i].real
            || !res[i - 1].real
            || !res[i - 2].real
            || !res[i + 1].real
            || !res[i + 2].real
        {
            continue;
        }

        let (ca_i, ca_m2, ca_p2) = match (res[i].ca, res[i - 2].ca, res[i + 2].ca) {
            (Some(a), Some(b), Some(c)) => (a, b, c),
            _ => continue,
        };

        let v1 = ca_i - ca_m2;
        let v2 = ca_p2 - ca_i;
        let len1 = v1.magnitude();
        let len2 = v2.magnitude();

        if len1 < 1e-6 || len2 < 1e-6 {
            continue;
        }

        let cos_angle = (v1.dot(v2) / (len1 * len2)).clamp(-1.0, 1.0);
        let angle_deg = cos_angle.acos().to_degrees();

        if angle_deg > BEND_ANGLE_CUTOFF && res[i].ss < DsspState::Bend {
            res[i].ss = DsspState::Bend;
        }
    }
}

// ============================================================================
// Trait Implementation
// ============================================================================

/// DSSP secondary structure assigner.
///
/// Implements the Kabsch & Sander algorithm which classifies residues based on
/// hydrogen bond energy patterns into helices (alpha, 3-10, pi), sheets, turns,
/// and bends — then collapses to the three canonical types (Helix, Sheet, Loop).
#[derive(Default)]
pub struct Dssp {
    pub params: DsspParams,
}

impl Dssp {
    pub fn new(params: DsspParams) -> Self {
        Self { params }
    }
}

impl SecondaryStructureAssigner for Dssp {
    fn assign(&self, residues: &[BackboneResidue]) -> Vec<SsType> {
        if residues.is_empty() {
            return Vec::new();
        }

        // Step 1: Build padded working array with H positions
        let mut res = collect_with_breaks(residues);

        if res.len() < 2 * CHAIN_BREAK_PAD + 1 {
            return vec![SsType::Loop; residues.len()];
        }

        // Step 2: Detect H-bonds (energy model + spatial hash)
        detect_hbonds(&mut res, self.params.hbond_energy_cutoff);

        // Step 3: Detect turns (n = 3, 4, 5)
        detect_turns(&mut res);

        // Step 4: Assign helices from consecutive turns
        assign_helices(&mut res);

        // Step 5: Detect bridge pairs
        detect_bridges(&mut res);

        // Step 6: Assign strands/sheets from ladders
        assign_sheets(&mut res);

        // Step 7: Detect bends
        detect_bends(&mut res);

        // Step 8: Map back to input and quantize
        let mut output = vec![SsType::Loop; residues.len()];
        for r in &res {
            if let Some(idx) = r.input_idx {
                output[idx] = r.ss.to_ss_type();
            }
        }
        output
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[expect(
        clippy::too_many_arguments,
        reason = "test fixture mirrors DSSP backbone fields for readable geometry setup"
    )]
    fn make_residue(
        n: [f32; 3],
        ca: [f32; 3],
        c: [f32; 3],
        o: [f32; 3],
        chain: &str,
        resv: i32,
        nh_direction: Option<[f32; 3]>,
        bonded_to_prev: bool,
    ) -> BackboneResidue {
        BackboneResidue {
            n: Vec3::new(n[0], n[1], n[2]),
            ca: Vec3::new(ca[0], ca[1], ca[2]),
            c: Vec3::new(c[0], c[1], c[2]),
            o: Vec3::new(o[0], o[1], o[2]),
            chain: chain.into(),
            resv,
            nh_direction: nh_direction.map(|d| Vec3::new(d[0], d[1], d[2])),
            bonded_to_prev,
        }
    }

    #[test]
    fn dssp_empty_input() {
        let dssp = Dssp::default();
        assert!(dssp.assign(&[]).is_empty());
    }

    #[test]
    fn dssp_single_residue_is_loop() {
        let dssp = Dssp::default();
        let residues = vec![make_residue(
            [-1.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            "A",
            1,
            None,
            false,
        )];
        let result = dssp.assign(&residues);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], SsType::Loop);
    }

    #[test]
    fn test_hbond_energy_known_geometry() {
        // Place donor and acceptor at known positions to verify energy formula.
        // Donor: N at origin, H at (1.008, 0, 0)
        // Acceptor: C at (4.0, 0, 0), O at (3.0, 0, 0)
        let mut res = vec![DsspResidue::default(); 2];
        res[0].n = Some(Vec3::new(0.0, 0.0, 0.0));
        res[0].h = Some(Vec3::new(1.008, 0.0, 0.0));
        res[0].real = true;
        res[1].c = Some(Vec3::new(4.0, 0.0, 0.0));
        res[1].o = Some(Vec3::new(3.0, 0.0, 0.0));
        res[1].real = true;

        let e = hbond_energy(&res, 0, 1);

        // r_ON = 3.0, r_CH = 2.992, r_OH = 1.992, r_CN = 4.0
        let expected = COUPLING * (1.0 / 3.0 + 1.0 / 2.992 - 1.0 / 1.992 - 1.0 / 4.0);
        assert!(
            (e - expected).abs() < 0.01,
            "energy = {e}, expected = {expected}"
        );
    }

    #[test]
    fn test_try_insert_hbond_best_two() {
        let mut slots = [NO_HBOND; 2];

        try_insert_hbond(&mut slots, 10, -2.0);
        assert_eq!(slots[0].partner, 10);
        assert!((slots[0].energy - (-2.0)).abs() < 1e-6);

        try_insert_hbond(&mut slots, 20, -1.0);
        assert_eq!(slots[0].partner, 10); // -2.0 still best
        assert_eq!(slots[1].partner, 20);

        try_insert_hbond(&mut slots, 30, -3.0);
        assert_eq!(slots[0].partner, 30); // -3.0 is new best
        assert_eq!(slots[1].partner, 10); // -2.0 is second

        // -0.5 should be rejected (worse than both)
        try_insert_hbond(&mut slots, 40, -0.5);
        assert_eq!(slots[0].partner, 30);
        assert_eq!(slots[1].partner, 10);
    }

    #[test]
    fn test_dssp_state_priority() {
        assert!(DsspState::Helix > DsspState::Bridge);
        assert!(DsspState::Bridge > DsspState::Strand);
        assert!(DsspState::Strand > DsspState::ThreeTen);
        assert!(DsspState::ThreeTen > DsspState::Pi);
        assert!(DsspState::Pi > DsspState::Turn);
        assert!(DsspState::Turn > DsspState::Bend);
        assert!(DsspState::Bend > DsspState::Empty);
    }

    #[test]
    fn test_dssp_state_quantization() {
        assert_eq!(DsspState::Helix.to_ss_type(), SsType::Helix);
        assert_eq!(DsspState::ThreeTen.to_ss_type(), SsType::Helix);
        assert_eq!(DsspState::Pi.to_ss_type(), SsType::Helix);
        assert_eq!(DsspState::Strand.to_ss_type(), SsType::Sheet);
        assert_eq!(DsspState::Bridge.to_ss_type(), SsType::Sheet);
        assert_eq!(DsspState::Turn.to_ss_type(), SsType::Loop);
        assert_eq!(DsspState::Bend.to_ss_type(), SsType::Loop);
        assert_eq!(DsspState::Empty.to_ss_type(), SsType::Loop);
    }

    #[test]
    fn test_chain_break_isolation() {
        // Two residues on different chains should not form H-bonds across the break
        let dssp = Dssp::default();
        let residues = vec![
            make_residue(
                [-1.0, 0.0, 0.0],
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                "A",
                1,
                Some([0.0, -1.0, 0.0]),
                false,
            ),
            make_residue(
                [0.0, 3.0, 0.0],
                [1.0, 3.0, 0.0],
                [2.0, 3.0, 0.0],
                [2.0, 2.0, 0.0],
                "B",
                1,
                Some([0.0, 1.0, 0.0]),
                false, // not bonded — chain break
            ),
        ];
        let result = dssp.assign(&residues);
        assert_eq!(result, vec![SsType::Loop, SsType::Loop]);
    }

    #[test]
    fn bend_detection_obeys_angle_threshold_and_state_priority() {
        for (degrees, initial, expected) in [
            (69.0_f32, DsspState::Empty, DsspState::Empty),
            (71.0, DsspState::Empty, DsspState::Bend),
            (90.0, DsspState::Turn, DsspState::Turn),
        ] {
            let angle = degrees.to_radians();
            let mut residues = vec![DsspResidue::default(); 5];
            for residue in &mut residues {
                residue.real = true;
            }
            residues[0].ca = Some(Vec3::new(-2.0, 0.0, 0.0));
            residues[2].ca = Some(Vec3::new(0.0, 0.0, 0.0));
            residues[4].ca = Some(Vec3::new(2.0 * angle.cos(), 2.0 * angle.sin(), 0.0));
            residues[2].ss = initial;

            detect_bends(&mut residues);

            assert_eq!(
                residues[2].ss, expected,
                "angle={degrees}, initial={initial:?}"
            );
            assert!(residues[..2]
                .iter()
                .chain(&residues[3..])
                .all(|r| r.ss == DsspState::Empty));
        }
    }

    /// Build an idealized alpha-helix backbone.
    ///
    /// The geometry places backbone atoms on a helix with the carbonyl O
    /// extending along the helix axis (z) so that O(i) ends up close to
    /// NH(i+4), reproducing the characteristic i→i+4 H-bond pattern.
    fn build_ideal_helix(n_residues: usize) -> Vec<BackboneResidue> {
        let rise = 1.0_f32;
        let radius = 1.5_f32;
        let angle_step = 100.0_f32.to_radians();

        let mut residues = Vec::with_capacity(n_residues);

        for i in 0..n_residues {
            let theta = i as f32 * angle_step;
            let z = i as f32 * rise;

            let ca = Vec3::new(radius * theta.cos(), radius * theta.sin(), z);

            // N slightly inside the helix and below CA
            let n = Vec3::new(
                (radius - 0.2) * theta.cos(),
                (radius - 0.2) * theta.sin(),
                z - 0.4,
            );

            // C slightly outside and above CA
            let c = Vec3::new(
                (radius + 0.1) * theta.cos(),
                (radius + 0.1) * theta.sin(),
                z + 0.5,
            );

            // O extends from C primarily along the helix axis (z), so that
            // O(i) is close to N(i+4).  Rise of 4 residues = 4.0, N offset
            // = -0.4, C offset = +0.5, O extension = +1.2 ⇒ z-gap ≈ 1.7 Å.
            let o = Vec3::new(
                (radius + 0.1) * theta.cos(),
                (radius + 0.1) * theta.sin(),
                z + 0.5 + 1.2,
            );

            // NH direction: toward O(i-4) for i ≥ 4, else downward along z
            let nh_dir = if i >= 4 {
                let prev_theta = (i - 4) as f32 * angle_step;
                let prev_z = (i - 4) as f32 * rise;
                let target_o = Vec3::new(
                    (radius + 0.1) * prev_theta.cos(),
                    (radius + 0.1) * prev_theta.sin(),
                    prev_z + 0.5 + 1.2,
                );
                let dir = target_o - n;
                let mag = dir.magnitude();
                if mag > 1e-6 {
                    Some(dir * (1.0 / mag))
                } else {
                    None
                }
            } else if i > 0 {
                Some(Vec3::new(0.0, 0.0, -1.0))
            } else {
                None
            };

            residues.push(BackboneResidue {
                n,
                ca,
                c,
                o,
                chain: "A".into(),
                resv: i as i32 + 1,
                nh_direction: nh_dir,
                bonded_to_prev: i > 0,
            });
        }

        residues
    }

    #[test]
    fn test_helix_detection_ideal() {
        let residues = build_ideal_helix(10);
        let dssp = Dssp::default();
        let result = dssp.assign(&residues);

        // At least some middle residues should be helix
        let helix_count = result.iter().filter(|&&s| s == SsType::Helix).count();
        assert!(
            helix_count >= 3,
            "expected at least 3 helix residues in ideal 10-residue helix, got {helix_count}: {result:?}"
        );
    }

    #[test]
    fn test_has_hbond_helper() {
        let mut res = vec![DsspResidue::default(); 4];
        res[0].real = true;
        res[3].real = true;

        // No H-bonds yet
        assert!(!has_hbond(&res, 0, 3));

        // Add CO(0) → NH(3) via co_acceptor
        res[0].co_acceptor[0] = HBondSlot {
            partner: 3,
            energy: -1.0,
        };
        assert!(has_hbond(&res, 0, 3));
        assert!(!has_hbond(&res, 3, 0)); // reverse not set
    }
}
