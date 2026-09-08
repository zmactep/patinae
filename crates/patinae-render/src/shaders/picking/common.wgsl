// Picking — single source of truth for the Rg32Uint bit layout. Mirrors
// `PackedId::pack` in src/picking/mod.rs.
//
//   .r = (rep_kind:4) | (object_id:12) | (atom_id_low:16)
//   .g = (atom_id_high:16) | (copy_plus_one:16)

struct PickingParams {
    /// (rep_kind << 28) | (object_id << 16). Combined with atom_id at draw
    /// time so the shader does no shifting beyond the atom_id split.
    rep_object: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

fn pack_id(rep_object: u32, atom_id: u32) -> vec2<u32> {
    let lo = rep_object | (atom_id & 0xFFFFu);
    let hi = ((atom_id >> 16u) << 16u) | (obj.flags & 0xFFFFu);
    return vec2<u32>(lo, hi);
}
