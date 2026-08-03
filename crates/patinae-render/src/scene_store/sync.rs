//! Per-object sync routine. Routes [`patinae_mol::DirtyFlags`] bits to the
//! cheapest possible buffer write inside [`SceneStore`].
//!
//! Bit-to-work routing (other bits ignored — handled elsewhere):
//!
//! | Dirty bit     | Work                                                |
//! |---------------|------------------------------------------------------|
//! | `TOPOLOGY`    | Rewrite atoms / bonds / csr slot. Visit every atom + bond once. |
//! | `COORDS`      | Rewrite coords slot + per-bond valence directions. |
//! | `COLOR`       | Rewrite fused color_lut slots.                      |
//! | `REPS`        | Rewrite atom representation flags + mask_lut bits. |
//! | `VISIBILITY`  | Rewrite atom representation flags + mask_lut bits. |
//! | `SELECTION`   | Apply sparse marker updates, or rewrite marker_lut. |
//! | `HOVER`       | Apply sparse marker updates when provided.          |
//! | `TRANSPARENCY`| Rewrite atoms slot's per-atom alpha overrides.      |
//! | `DRAW_MASK`   | No SceneStore writes; render order only.            |
//!
//! On a freshly-created slot the caller passes `DirtyFlags::ALL`; the
//! routine does the full populate.

use patinae_mol::{AtomIndex, BondOrder, CoordSet, DirtyFlags, ObjectMolecule, RepMask};

use super::{pack_alpha_override, AtomGpu, BondGpu, ObjectEntry, ObjectSlot, SceneStore};
use crate::render_input::{ColorLutEntry, RenderObjectInput, RepColorLutEntry};

const VALENCE_EPS: f32 = 1e-4;
const VALENCE_ALIGNMENT_DOT: f32 = 0.75;
const MAX_VALENCE_CANDIDATES: usize = 16;

impl SceneStore {
    /// Sync one object's data into the store. Returns the resolved
    /// [`ObjectSlot`] so callers can issue `set_bind_group(2, &scene.bg,
    /// &[slot.dynamic_offset()])`.
    ///
    /// `effective_dirty` must be `DirtyFlags::ALL` on the very first sync
    /// for a given `object_id`; subsequent syncs pass `input.dirty`.
    pub fn sync_object(
        &mut self,
        input: &RenderObjectInput<'_>,
        effective_dirty: DirtyFlags,
    ) -> ObjectSlot {
        let n_atoms = input.molecule.atom_count() as u32;
        let n_bonds = input.molecule.bonds().count() as u32;
        let slot_was_new = !self.has_slot(input.object_id);
        let slot = self.ensure_slot(input.object_id, n_atoms, n_bonds);
        let needs_table_write =
            slot_was_new || effective_dirty.intersects(DirtyFlags::COORDS | DirtyFlags::TOPOLOGY);

        if effective_dirty.intersects(DirtyFlags::TOPOLOGY) {
            write_atoms_for_object(self, &slot, input);
            write_bonds_for_object(self, &slot, input);
            write_csr_for_object(self, &slot, input);
        } else {
            if effective_dirty.intersects(DirtyFlags::COORDS) {
                write_bonds_for_object(self, &slot, input);
            }
            if effective_dirty
                .intersects(DirtyFlags::REPS | DirtyFlags::VISIBILITY | DirtyFlags::TRANSPARENCY)
            {
                // Cheaper than full TOPOLOGY: rewrite atoms slot only.
                // Per-rep visibility lives in AtomGpu::repr_flags, and
                // transparency overrides are folded into AtomGpu too, so
                // these dirties share this path.
                write_atoms_for_object(self, &slot, input);
            }
        }

        if effective_dirty.intersects(DirtyFlags::COORDS | DirtyFlags::TOPOLOGY) {
            write_coords_for_object(self, &slot, input);
        }

        if effective_dirty.intersects(DirtyFlags::COLOR | DirtyFlags::TOPOLOGY) {
            write_color_lut_for_object(self, &slot, input);
        }

        if effective_dirty
            .intersects(DirtyFlags::REPS | DirtyFlags::VISIBILITY | DirtyFlags::TOPOLOGY)
        {
            write_mask_lut_for_object(self, &slot, input);
        }

        if effective_dirty
            .intersects(DirtyFlags::SELECTION | DirtyFlags::HOVER | DirtyFlags::TOPOLOGY)
        {
            if input.marker_updates.is_empty() || effective_dirty.intersects(DirtyFlags::TOPOLOGY) {
                write_marker_lut_for_object(self, &slot, input);
            } else {
                write_marker_updates_for_object(self, &slot, input);
            }
        }

        if needs_table_write {
            self.write_obj_entry(
                slot.table_index,
                ObjectEntry {
                    atom_offset: slot.atom_offset,
                    atom_count: slot.atom_count,
                    bond_offset: slot.bond_offset,
                    bond_count: slot.bond_count,
                    object_id: input.object_id.0,
                    flags: 0,
                    _pad0: [0; 2],
                    model_matrix: input.transform,
                },
            );
        }

        slot
    }
}

/// Populate `atoms[atom_offset .. atom_offset+atom_count]`. Folds in
/// per-atom `sphere_scale` override (so `vdw` field is "use this radius;
/// per-rep `params.scale` multiplies on top") and packs per-atom per-rep
/// alpha overrides via `f32::NAN` sentinel.
fn write_atoms_for_object(
    store: &mut SceneStore,
    slot: &ObjectSlot,
    input: &RenderObjectInput<'_>,
) {
    let base = slot.atom_offset as usize;
    let mut idx = 0usize;
    let count = slot.atom_count as usize;
    for atom in input.molecule.atoms() {
        if idx >= count {
            break;
        }
        let scale_override = atom.repr.sphere_scale.unwrap_or(1.0);
        let vdw = atom.effective_vdw() * scale_override;
        let sphere_byte = pack_alpha_override(atom.repr.sphere_transparency);
        let stick_byte = pack_alpha_override(atom.repr.stick_transparency);
        let ellipsoid_byte = pack_alpha_override(atom.repr.ellipsoid_transparency);
        let cartoon_byte = pack_alpha_override(atom.repr.cartoon_transparency);
        let surface_byte = pack_alpha_override(atom.repr.surface_transparency);
        // Layout: pack_a = [sphere | stick | ellipsoid | cartoon] (LE bytes).
        let alpha_pack_a =
            u32::from_le_bytes([sphere_byte, stick_byte, ellipsoid_byte, cartoon_byte]);
        // pack_b = [surface | _ | _ | _]. Reserved bytes carry the
        // sentinel so future helpers reading them never spuriously
        // interpret an override.
        let alpha_pack_b = u32::from_le_bytes([
            surface_byte,
            super::ALPHA_NO_OVERRIDE,
            super::ALPHA_NO_OVERRIDE,
            super::ALPHA_NO_OVERRIDE,
        ]);
        let entry = AtomGpu {
            vdw,
            repr_flags: atom.repr.visible_reps.0,
            alpha_pack_a,
            alpha_pack_b,
            element_id: atom.element as u32,
            chain_id: 0,
            residue_id: 0,
            _pad: 0,
        };
        store.atoms.set(base + idx, entry);
        idx += 1;
    }
    // Trailing slots that weren't visited (atom_count > iter length) keep
    // their zeroed default — matches REPR=0 (invisible).
    debug_assert_eq!(
        idx, count,
        "scene_store: atom iter length {} != slot count {}",
        idx, count
    );
}

/// Populate `bonds[bond_offset .. bond_offset+bond_count]`. Atom indices
/// are stored object-local; the GPU adds `obj.atom_offset` at fetch time.
fn write_bonds_for_object(
    store: &mut SceneStore,
    slot: &ObjectSlot,
    input: &RenderObjectInput<'_>,
) {
    let base = slot.bond_offset as usize;
    for (i, bond) in input.molecule.bonds().enumerate() {
        if i >= slot.bond_count as usize {
            break;
        }
        let valence_perp_pad = compute_valence_perp(input.molecule, input.coord_set, bond);
        store.bonds.set(
            base + i,
            BondGpu {
                atom1: bond.atom1.as_u32(),
                atom2: bond.atom2.as_u32(),
                order: bond.order.as_raw() as u32,
                flags: 0,
                valence_perp_pad,
            },
        );
    }
}

fn compute_valence_perp(
    mol: &ObjectMolecule,
    coord_set: &CoordSet,
    bond: &patinae_mol::Bond,
) -> [f32; 4] {
    if !is_multi_bond_order(bond.order) {
        return [0.0; 4];
    }

    let Some(pa) = atom_pos(coord_set, bond.atom1) else {
        return [0.0; 4];
    };
    let Some(pb) = atom_pos(coord_set, bond.atom2) else {
        return [0.0; 4];
    };
    let Some(bond_dir) = normalize3(sub3(pb, pa)) else {
        return [0.0; 4];
    };
    let mid = mul3(add3(pa, pb), 0.5);

    let mut candidates = [[0.0_f32; 3]; MAX_VALENCE_CANDIDATES];
    let mut count = 0usize;
    for endpoint in [bond.atom1, bond.atom2] {
        for nb in mol.bonded_atoms_iter(endpoint) {
            if nb == bond.atom1 || nb == bond.atom2 {
                continue;
            }
            let Some(atom) = mol.get_atom(nb) else {
                continue;
            };
            if !atom.is_heavy() {
                continue;
            }
            let Some(pos) = atom_pos(coord_set, nb) else {
                continue;
            };
            let to_neighbor = sub3(pos, mid);
            let projected = sub3(to_neighbor, mul3(bond_dir, dot3(to_neighbor, bond_dir)));
            let Some(candidate) = normalize3(projected) else {
                continue;
            };
            candidates[count] = candidate;
            count += 1;
            if count == MAX_VALENCE_CANDIDATES {
                break;
            }
        }
        if count == MAX_VALENCE_CANDIDATES {
            break;
        }
    }

    let perp = best_supported_valence_perp(&candidates[..count], bond_dir)
        .unwrap_or_else(|| fallback_perp(bond_dir));
    [perp[0], perp[1], perp[2], 0.0]
}

fn is_multi_bond_order(order: BondOrder) -> bool {
    matches!(
        order,
        BondOrder::Double | BondOrder::Triple | BondOrder::Aromatic
    )
}

fn best_supported_valence_perp(candidates: &[[f32; 3]], bond_dir: [f32; 3]) -> Option<[f32; 3]> {
    if candidates.is_empty() {
        return None;
    }

    let mut normals = [[0.0_f32; 3]; MAX_VALENCE_CANDIDATES];
    let mut normal_count = 0usize;
    for &candidate in candidates {
        let Some(normal) = normalize3(cross3(bond_dir, candidate)) else {
            continue;
        };
        normals[normal_count] = normal;
        normal_count += 1;
    }
    if normal_count == 0 {
        return None;
    }

    let mut best_index = 0usize;
    let mut best_support = 0usize;
    let mut best_score = -1.0_f32;
    for i in 0..normal_count {
        let mut support = 0usize;
        let mut score = 0.0_f32;
        for j in 0..normal_count {
            let align = dot3(normals[i], normals[j]).abs();
            if align >= VALENCE_ALIGNMENT_DOT {
                support += 1;
                score += align;
            }
        }
        if support > best_support || (support == best_support && score > best_score) {
            best_index = i;
            best_support = support;
            best_score = score;
        }
    }

    let best_normal = normals[best_index];
    let mut normal_sum = [0.0_f32; 3];
    for &normal in &normals[..normal_count] {
        let align = dot3(normal, best_normal);
        if align.abs() < VALENCE_ALIGNMENT_DOT {
            continue;
        }
        let aligned = if align < 0.0 {
            mul3(normal, -1.0)
        } else {
            normal
        };
        normal_sum = add3(normal_sum, aligned);
    }
    let plane_normal = normalize3(normal_sum)?;
    let mut perp = normalize3(cross3(plane_normal, bond_dir))?;

    let mut candidate_sum = [0.0_f32; 3];
    for &candidate in candidates {
        candidate_sum = add3(candidate_sum, candidate);
    }
    let sign_ref = normalize3(candidate_sum).unwrap_or(candidates[0]);
    if dot3(perp, sign_ref) < 0.0 {
        perp = mul3(perp, -1.0);
    }
    Some(perp)
}

fn atom_pos(coord_set: &CoordSet, atom: AtomIndex) -> Option<[f32; 3]> {
    let p = coord_set.get_atom_coord(atom)?;
    Some([p.x, p.y, p.z])
}

fn fallback_perp(bond_dir: [f32; 3]) -> [f32; 3] {
    let up = [0.0_f32, 1.0, 0.0];
    if let Some(perp) = normalize3(cross3(bond_dir, up)) {
        return perp;
    }
    let right = [1.0_f32, 0.0, 0.0];
    normalize3(cross3(bond_dir, right)).unwrap_or([0.0, 1.0, 0.0])
}

fn add3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn mul3(v: [f32; 3], s: f32) -> [f32; 3] {
    [v[0] * s, v[1] * s, v[2] * s]
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize3(v: [f32; 3]) -> Option<[f32; 3]> {
    let len2 = dot3(v, v);
    if len2 <= VALENCE_EPS {
        return None;
    }
    let inv = len2.sqrt().recip();
    Some(mul3(v, inv))
}

/// Populate `coords[atom_offset .. atom_offset+atom_count]`. xyz + 0 pad.
fn write_coords_for_object(
    store: &mut SceneStore,
    slot: &ObjectSlot,
    input: &RenderObjectInput<'_>,
) {
    let base = slot.atom_offset as usize;
    let count = slot.atom_count as usize;
    let mut written = 0usize;
    for (atom_idx, coord) in input.coord_set.iter_with_atoms() {
        let local = atom_idx.as_u32() as usize;
        if local >= count {
            continue;
        }
        store
            .coords
            .set(base + local, [coord.x, coord.y, coord.z, 0.0]);
        written += 1;
    }
    // CoordSet may be sparse for partially-loaded states; remaining slots
    // keep their previous value (or zero on first sync). That's
    // acceptable — invisible atoms (REPR=0) never get drawn.
    let _ = written;
}

/// Populate `color_lut[atom_offset .. atom_offset+atom_count]` from the
/// host-resolved base RGBA and representation-specific RGB overrides.
/// Per-rep alpha is **not** applied here — it's a per-rep multiplier
/// (`material_params.alpha_mul` group 3) plus per-atom override on
/// `AtomGpu.<rep>_alpha`.
fn write_color_lut_for_object(
    store: &mut SceneStore,
    slot: &ObjectSlot,
    input: &RenderObjectInput<'_>,
) {
    let base = slot.atom_offset as usize;
    let count = slot.atom_count as usize;
    debug_assert!(
        input.atom_colors.len() >= count,
        "scene_store: atom_colors {} < slot count {}",
        input.atom_colors.len(),
        count
    );
    for i in 0..count {
        let base_color = input.atom_colors[i];
        let rep_colors = input
            .atom_rep_colors
            .get(i)
            .copied()
            .unwrap_or_else(RepColorLutEntry::inherit_all);
        store
            .color_lut
            .set(base + i, ColorLutEntry::new(base_color, rep_colors));
    }
}

/// Populate `mask_lut[…]` bits for the slot's atom range. Bit set ⇒
/// atom has *some* representation visible; bit clear ⇒ atom hidden in
/// every rep. Coarse first cut — finer per-rep visibility lives in
/// `AtomGpu.repr_flags`.
fn write_mask_lut_for_object(
    store: &mut SceneStore,
    slot: &ObjectSlot,
    input: &RenderObjectInput<'_>,
) {
    let base = slot.atom_offset as usize;
    let count = slot.atom_count as usize;
    let any_mask = RepMask::SPHERES
        .union(RepMask::STICKS)
        .union(RepMask::LINES)
        .union(RepMask::DOTS)
        .union(RepMask::CARTOON)
        .union(RepMask::RIBBON)
        .union(RepMask::SURFACE)
        .union(RepMask::ELLIPSOIDS);

    for (idx, atom) in input.molecule.atoms().enumerate() {
        if idx >= count {
            break;
        }
        let visible = atom.repr.visible_reps.intersection(any_mask).0 != 0;
        let gid = (base + idx) as u32;
        let word = (gid / 32) as usize;
        let bit = gid & 31;
        if word >= store.mask_lut.cpu().len() {
            break;
        }
        let mut current = store.mask_lut.cpu()[word];
        if visible {
            current |= 1u32 << bit;
        } else {
            current &= !(1u32 << bit);
        }
        store.mask_lut.set(word, current);
    }
}

/// Populate `marker_lut[atom_offset .. atom_offset+atom_count]` from the
/// host-packed per-atom marker bits. Bit layout in
/// [`crate::scene_store::marker`]. Slices shorter than `atom_count` are
/// zero-padded — equivalent to "no atoms marked" for the trailing range.
fn write_marker_lut_for_object(
    store: &mut SceneStore,
    slot: &ObjectSlot,
    input: &RenderObjectInput<'_>,
) {
    let base = slot.atom_offset as usize;
    let count = slot.atom_count as usize;
    let provided = input.atom_markers.len().min(count);
    for (i, bits) in input.atom_markers.iter().take(provided).enumerate() {
        store.marker_lut.set(base + i, *bits);
    }
    for i in provided..count {
        store.marker_lut.set(base + i, 0);
    }
}

fn write_marker_updates_for_object(
    store: &mut SceneStore,
    slot: &ObjectSlot,
    input: &RenderObjectInput<'_>,
) {
    let base = slot.atom_offset as usize;
    let count = slot.atom_count as usize;
    for update in input.marker_updates {
        let local = update.atom_index as usize;
        if local < count {
            store.marker_lut.set(base + local, update.bits);
        }
    }
}

/// Build CSR atom→bond adjacency for the slot. Two passes: count
/// degrees, then scatter bond ids into per-atom slots.
fn write_csr_for_object(store: &mut SceneStore, slot: &ObjectSlot, input: &RenderObjectInput<'_>) {
    let atom_base = slot.atom_offset as usize;
    let bond_base = slot.bond_offset as usize;
    let n_atoms = slot.atom_count as usize;
    let n_bonds = slot.bond_count as usize;

    // Pass 1: per-atom degree.
    let mut degrees = vec![0u32; n_atoms];
    for bond in input.molecule.bonds() {
        let a = bond.atom1.as_u32() as usize;
        let b = bond.atom2.as_u32() as usize;
        if a < n_atoms {
            degrees[a] += 1;
        }
        if b < n_atoms {
            degrees[b] += 1;
        }
    }

    // Prefix-sum into a local Vec — write into the GrowableBuffer at the
    // end so we don't hold a partial mutable borrow across the scatter
    // pass. Object's CSR slice spans
    // `[bond_base*2 + running .. bond_base*2 + running + degree]`.
    let local_base = (bond_base * 2) as u32;
    let mut local_offsets = vec![0u32; n_atoms + 1];
    let mut running: u32 = 0;
    for i in 0..n_atoms {
        local_offsets[i] = running;
        running += degrees[i];
    }
    local_offsets[n_atoms] = running;
    for (i, &offset) in local_offsets.iter().enumerate().take(n_atoms + 1) {
        store.csr_offsets.set(atom_base + i, local_base + offset);
    }

    // Pass 2: scatter using `local_offsets` + per-atom write heads.
    let mut heads = vec![0u32; n_atoms];
    for (i, bond) in input.molecule.bonds().enumerate() {
        if i >= n_bonds {
            break;
        }
        let global_bond_id = (bond_base + i) as u32;
        for &a in &[bond.atom1.as_u32() as usize, bond.atom2.as_u32() as usize] {
            if a >= n_atoms {
                continue;
            }
            let lo = local_base as usize + local_offsets[a] as usize + heads[a] as usize;
            store.csr_indices.set(lo, global_bond_id);
            heads[a] += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::picking::ObjectId;
    use crate::render_input::{SceneLod, IDENTITY_TRANSFORM};
    use lin_alg::f32::Vec3;
    use patinae_mol::{Atom, CoordSet, Element, ObjectMolecule};

    fn add_atom(mol: &mut ObjectMolecule, name: &str) -> AtomIndex {
        mol.add_atom(Atom::new(name, Element::Carbon))
    }

    fn render_input<'a>(
        mol: &'a ObjectMolecule,
        coord: &'a CoordSet,
        colors: &'a [[f32; 4]],
    ) -> RenderObjectInput<'a> {
        RenderObjectInput {
            object_id: ObjectId(7),
            molecule: mol,
            coord_set: coord,
            transform: IDENTITY_TRANSFORM,
            visible_reps: RepMask::ALL,
            draw_reps: RepMask::ALL,
            object_settings: None,
            atom_colors: colors,
            atom_rep_colors: &[],
            atom_markers: &[],
            marker_updates: &[],
            has_markers: false,
            lod: SceneLod::Auto,
            dirty: DirtyFlags::empty(),
        }
    }

    fn first_perp(mol: &ObjectMolecule, coord: &CoordSet) -> [f32; 3] {
        let bond = mol.bonds().next().expect("bond");
        let p = compute_valence_perp(mol, coord, bond);
        [p[0], p[1], p[2]]
    }

    #[test]
    fn visibility_dirty_rewrites_repr_flags_and_mask_lut() {
        let mut store = SceneStore::new();
        let mut mol = ObjectMolecule::new("visibility");
        let mut atom = Atom::new("CA", Element::Carbon);
        atom.repr.visible_reps = RepMask::CARTOON;
        mol.add_atom(atom);
        let coord = CoordSet::from_vec3(&[Vec3::new(0.0, 0.0, 0.0)]);
        let colors = [[1.0, 1.0, 1.0, 1.0]];

        let slot = {
            let input = render_input(&mol, &coord, &colors);
            store.sync_object(&input, DirtyFlags::ALL)
        };
        let atom_offset = slot.atom_offset as usize;
        assert_eq!(
            store.atoms.cpu()[atom_offset].repr_flags,
            RepMask::CARTOON.0
        );
        assert_eq!(store.mask_lut.cpu()[0] & 1, 1);

        mol.get_atom_mut(AtomIndex(0)).unwrap().repr.visible_reps = RepMask::SURFACE;
        {
            let input = render_input(&mol, &coord, &colors);
            store.sync_object(&input, DirtyFlags::VISIBILITY);
        }
        assert_eq!(
            store.atoms.cpu()[atom_offset].repr_flags,
            RepMask::SURFACE.0
        );
        assert_eq!(store.mask_lut.cpu()[0] & 1, 1);

        mol.get_atom_mut(AtomIndex(0)).unwrap().repr.visible_reps = RepMask::NONE;
        {
            let input = render_input(&mol, &coord, &colors);
            store.sync_object(&input, DirtyFlags::VISIBILITY);
        }
        assert_eq!(store.atoms.cpu()[atom_offset].repr_flags, RepMask::NONE.0);
        assert_eq!(store.mask_lut.cpu()[0] & 1, 0);
    }

    #[test]
    fn draw_mask_dirty_does_not_rewrite_repr_flags_or_mask_lut() {
        let mut store = SceneStore::new();
        let mut mol = ObjectMolecule::new("draw-mask");
        let mut atom = Atom::new("CA", Element::Carbon);
        atom.repr.visible_reps = RepMask::CARTOON;
        mol.add_atom(atom);
        let coord = CoordSet::from_vec3(&[Vec3::new(0.0, 0.0, 0.0)]);
        let colors = [[1.0, 1.0, 1.0, 1.0]];

        let slot = {
            let input = render_input(&mol, &coord, &colors);
            store.sync_object(&input, DirtyFlags::ALL)
        };
        let atom_offset = slot.atom_offset as usize;

        mol.get_atom_mut(AtomIndex(0)).unwrap().repr.visible_reps = RepMask::NONE;
        {
            let input = render_input(&mol, &coord, &colors);
            store.sync_object(&input, DirtyFlags::DRAW_MASK);
        }

        assert_eq!(
            store.atoms.cpu()[atom_offset].repr_flags,
            RepMask::CARTOON.0
        );
        assert_eq!(store.mask_lut.cpu()[0] & 1, 1);
    }

    #[test]
    fn valence_perp_for_planar_ring_stays_in_ring_plane() {
        let mut mol = ObjectMolecule::new("ring");
        let atoms: Vec<_> = (0..6)
            .map(|i| add_atom(&mut mol, &format!("C{i}")))
            .collect();
        mol.add_bond(atoms[0], atoms[1], BondOrder::Double).unwrap();
        for i in 1..6 {
            mol.add_bond(atoms[i], atoms[(i + 1) % 6], BondOrder::Single)
                .unwrap();
        }
        let coord = CoordSet::from_vec3(&[
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.5, 0.866, 0.0),
            Vec3::new(-0.5, 0.866, 0.0),
            Vec3::new(-1.0, 0.0, 0.0),
            Vec3::new(-0.5, -0.866, 0.0),
            Vec3::new(0.5, -0.866, 0.0),
        ]);

        let perp = first_perp(&mol, &coord);
        let pa = atom_pos(&coord, atoms[0]).unwrap();
        let pb = atom_pos(&coord, atoms[1]).unwrap();
        let bond_dir = normalize3(sub3(pb, pa)).unwrap();
        assert!(dot3(perp, [0.0, 0.0, 1.0]).abs() < 1e-4);
        assert!(dot3(perp, bond_dir).abs() < 1e-4);
    }

    #[test]
    fn valence_perp_for_tree_double_bond_uses_heavy_neighbor_plane() {
        let mut mol = ObjectMolecule::new("tree");
        let a = add_atom(&mut mol, "C0");
        let b = add_atom(&mut mol, "C1");
        let c = add_atom(&mut mol, "C2");
        mol.add_bond(a, b, BondOrder::Double).unwrap();
        mol.add_bond(a, c, BondOrder::Single).unwrap();
        let coord = CoordSet::from_vec3(&[
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.3, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        ]);

        let perp = first_perp(&mol, &coord);
        assert!(dot3(perp, [0.0, 0.0, 1.0]).abs() < 1e-4);
        assert!(dot3(perp, [0.0, 1.0, 0.0]).abs() > 0.95);
    }

    #[test]
    fn valence_perp_for_trans_neighbors_does_not_fall_back_out_of_plane() {
        let mut mol = ObjectMolecule::new("trans");
        let a = add_atom(&mut mol, "C0");
        let b = add_atom(&mut mol, "C1");
        let c = add_atom(&mut mol, "C2");
        let d = add_atom(&mut mol, "C3");
        mol.add_bond(a, b, BondOrder::Double).unwrap();
        mol.add_bond(a, c, BondOrder::Single).unwrap();
        mol.add_bond(b, d, BondOrder::Single).unwrap();
        let coord = CoordSet::from_vec3(&[
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.3, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(1.3, -1.0, 0.0),
        ]);

        let perp = first_perp(&mol, &coord);
        assert!(dot3(perp, [0.0, 0.0, 1.0]).abs() < 1e-4);
        assert!(dot3(perp, [1.0, 0.0, 0.0]).abs() < 1e-4);
        assert!(dot3(perp, [0.0, 1.0, 0.0]).abs() > 0.95);
    }
}
