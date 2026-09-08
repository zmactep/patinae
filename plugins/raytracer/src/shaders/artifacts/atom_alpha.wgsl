struct AtomGpu {
    vdw: f32,
    repr_flags: u32,
    alpha_pack_a: u32,
    alpha_pack_b: u32,
}

fn unpack_atom_alpha(pack: u32, byte_idx: u32, fallback: f32) -> f32 {
    let byte = (pack >> (byte_idx * 8u)) & 0xffu;
    if byte == 0xffu {
        return fallback;
    }
    return f32(byte) / 254.0;
}

fn atom_alpha_for_group(group_id: u32) -> f32 {
    let fallback = 1.0 - params.transparency;
    let atom = scene_atoms[params.atom_offset + group_id];
    if params.rep_slot == 4u || params.rep_slot == 5u {
        return unpack_atom_alpha(atom.alpha_pack_a, 3u, fallback);
    }
    if params.rep_slot == 6u {
        return unpack_atom_alpha(atom.alpha_pack_b, 0u, fallback);
    }
    return fallback;
}

fn material_for_group(group_id: u32) -> vec4<f32> {
    let entry = color_lut[params.atom_offset + group_id];
    let packed = rep_packed_color(entry.reps, params.rep_slot);
    var rgba = select(unpack_rep_rgb8(packed), entry.base, packed == REP_COLOR_INHERIT);
    rgba.a = clamp(rgba.a * atom_alpha_for_group(group_id), 0.0, 1.0);
    return rgba;
}
