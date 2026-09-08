// Scene-wide LUTs and atom soup. Bind group 2.
//
// Mirrors `crates/patinae-render/src/scene_store/mod.rs`. Indices are
// **global**: `gid = obj.atom_offset + atom_local`. Per-rep alpha lives
// in group 3 (`material_params.alpha_mul`); per-atom per-rep alpha
// override is on `AtomGpu.<rep>_alpha` with `NaN` ⇒ "no override".

struct ObjectEntry {
    atom_offset: u32,
    atom_count:  u32,
    bond_offset: u32,
    bond_count:  u32,
    object_id:   u32,
    flags:       u32,
    _pad0_a:     u32,
    _pad0_b:     u32,
    model_matrix: mat4x4<f32>,
};

struct AtomGpu {
    vdw: f32,
    repr_flags: u32,
    alpha_pack_a: u32,
    alpha_pack_b: u32,
};

struct BondGpu {
    atom1: u32,
    atom2: u32,
    order: u32,
    flags: u32,
    valence_perp_pad: vec4<f32>,
};

struct ColorGpu {
    base:      vec4<f32>,
    sphere:    u32,
    stick:     u32,
    line:      u32,
    dot:       u32,
    cartoon:   u32,
    ribbon:    u32,
    surface:   u32,
    mesh:      u32,
    ellipsoid: u32,
    _pad0:     u32,
    _pad1:     u32,
    _pad2:     u32,
};

@group(2) @binding(0) var<uniform>      obj          : ObjectEntry;
@group(2) @binding(1) var<storage, read> scene_atoms : array<AtomGpu>;
@group(2) @binding(2) var<storage, read> scene_coords: array<vec4<f32>>;
@group(2) @binding(3) var<storage, read> scene_bonds : array<BondGpu>;
@group(2) @binding(4) var<storage, read> scene_color_lut : array<ColorGpu>;
@group(2) @binding(5) var<storage, read> scene_mask_lut  : array<u32>;
@group(2) @binding(6) var<storage, read> scene_marker_lut: array<u32>;
@group(2) @binding(7) var<storage, read> scene_csr_offsets: array<u32>;
@group(2) @binding(8) var<storage, read> scene_csr_indices: array<u32>;

const REP_COLOR_INHERIT: u32 = 0xFFFFFFFFu;

// `gid` here is the *global* index (already includes obj.atom_offset).
fn scene_color(gid: u32) -> vec4<f32> {
    return scene_color_lut[gid].base;
}

fn scene_unpack_rep_rgb8(packed: u32) -> vec4<f32> {
    let r = f32(packed & 0xFFu) / 255.0;
    let g = f32((packed >> 8u) & 0xFFu) / 255.0;
    let b = f32((packed >> 16u) & 0xFFu) / 255.0;
    return vec4<f32>(r, g, b, 1.0);
}

fn scene_rep_color_or_base(gid: u32, packed: u32) -> vec4<f32> {
    var base: vec4<f32>;
    if packed == REP_COLOR_INHERIT {
        base = scene_color(gid);
    } else {
        base = scene_unpack_rep_rgb8(packed);
    }
    return base;
}

fn scene_sphere_color(gid: u32) -> vec4<f32> {
    return scene_rep_color_or_base(gid, scene_color_lut[gid].sphere);
}

fn scene_stick_color(gid: u32) -> vec4<f32> {
    return scene_rep_color_or_base(gid, scene_color_lut[gid].stick);
}

fn scene_line_color(gid: u32) -> vec4<f32> {
    return scene_rep_color_or_base(gid, scene_color_lut[gid].line);
}

fn scene_dot_color(gid: u32) -> vec4<f32> {
    return scene_rep_color_or_base(gid, scene_color_lut[gid].dot);
}

fn scene_cartoon_color(gid: u32) -> vec4<f32> {
    return scene_rep_color_or_base(gid, scene_color_lut[gid].cartoon);
}

fn scene_ribbon_color(gid: u32) -> vec4<f32> {
    return scene_rep_color_or_base(gid, scene_color_lut[gid].ribbon);
}

fn scene_surface_color(gid: u32) -> vec4<f32> {
    return scene_rep_color_or_base(gid, scene_color_lut[gid].surface);
}

fn scene_mesh_color(gid: u32) -> vec4<f32> {
    return scene_rep_color_or_base(gid, scene_color_lut[gid].mesh);
}

fn scene_ellipsoid_color(gid: u32) -> vec4<f32> {
    return scene_rep_color_or_base(gid, scene_color_lut[gid].ellipsoid);
}

fn scene_visible(gid: u32) -> bool {
    let word = gid >> 5u;
    let bit  = gid & 31u;
    return scene_in_subset(gid) && (scene_mask_lut[word] & (1u << bit)) != 0u;
}

fn scene_marker(gid: u32) -> u32 {
    return scene_marker_lut[gid];
}

fn scene_atom(gid: u32) -> AtomGpu {
    return scene_atoms[gid];
}

fn scene_coord(gid: u32) -> vec3<f32> {
    return scene_coords[gid].xyz;
}

// RepMask bits — must match `patinae_mol::RepMask` constants.
const REP_BIT_STICKS:     u32 = 1u << 0u;
const REP_BIT_SPHERES:    u32 = 1u << 1u;
const REP_BIT_SURFACE:    u32 = 1u << 2u;
const REP_BIT_LABELS:     u32 = 1u << 3u;
const REP_BIT_NONBONDED:  u32 = 1u << 4u;
const REP_BIT_CARTOON:    u32 = 1u << 5u;
const REP_BIT_RIBBON:     u32 = 1u << 6u;
const REP_BIT_LINES:      u32 = 1u << 7u;
const REP_BIT_MESH:       u32 = 1u << 8u;
const REP_BIT_DOTS:       u32 = 1u << 9u;
const REP_BIT_ELLIPSOIDS: u32 = 1u << 16u;

fn scene_atom_in_rep(atom: AtomGpu, rep_bit: u32) -> bool {
    return (atom.repr_flags & rep_bit) != 0u;
}

// Per-atom per-rep alpha resolution. The atom soup packs each rep's
// alpha override as one byte inside `alpha_pack_a` / `alpha_pack_b` (see
// `scene_store/mod.rs`). Sentinel `0xFFu` ⇒ no override; caller should
// substitute the per-rep `alpha_mul` from group 3.
//
// `byte_idx` is the byte offset within the pack word (0 = LSB).
// `fallback` is the per-rep `alpha_mul` uniform.
fn scene_unpack_atom_alpha(pack: u32, byte_idx: u32, fallback: f32) -> f32 {
    let byte = (pack >> (byte_idx * 8u)) & 0xFFu;
    if byte == 0xFFu { return fallback; }
    return f32(byte) / 254.0;
}

// Per-rep alpha helpers — call site reads `let a = scene_atom_<rep>_alpha(atom, params.alpha_mul)`.
// Byte indices must match `scene_store::sync::write_atoms_for_object`:
//   pack_a [0]=sphere, [1]=stick, [2]=ellipsoid, [3]=cartoon
//   pack_b [0]=surface, [1..3]=reserved
fn scene_atom_sphere_alpha(atom: AtomGpu, fallback: f32) -> f32 {
    return scene_unpack_atom_alpha(atom.alpha_pack_a, 0u, fallback);
}
fn scene_atom_stick_alpha(atom: AtomGpu, fallback: f32) -> f32 {
    return scene_unpack_atom_alpha(atom.alpha_pack_a, 1u, fallback);
}
fn scene_atom_ellipsoid_alpha(atom: AtomGpu, fallback: f32) -> f32 {
    return scene_unpack_atom_alpha(atom.alpha_pack_a, 2u, fallback);
}
fn scene_atom_cartoon_alpha(atom: AtomGpu, fallback: f32) -> f32 {
    return scene_unpack_atom_alpha(atom.alpha_pack_a, 3u, fallback);
}
fn scene_atom_surface_alpha(atom: AtomGpu, fallback: f32) -> f32 {
    return scene_unpack_atom_alpha(atom.alpha_pack_b, 0u, fallback);
}

// Padding stores a shared source-subset mask, independent of copy count.
fn scene_in_subset(gid: u32) -> bool {
    if obj._pad0_b == 0u { return true; }
    let local = gid - obj.atom_offset;
    return local < obj.atom_count &&
        (scene_mask_lut[obj._pad0_a + (local >> 5u)] & (1u << (local & 31u))) != 0u;
}

fn scene_position(position: vec3<f32>) -> vec3<f32> {
    return (obj.model_matrix * vec4<f32>(position, 1.0)).xyz;
}

fn scene_direction(direction: vec3<f32>) -> vec3<f32> {
    return (obj.model_matrix * vec4<f32>(direction, 0.0)).xyz;
}

fn scene_normal(normal: vec3<f32>) -> vec3<f32> {
    let a = obj.model_matrix[0].xyz;
    let b = obj.model_matrix[1].xyz;
    let c = obj.model_matrix[2].xyz;
    let cofactor = mat3x3<f32>(cross(b, c), cross(c, a), cross(a, b));
    return normalize(cofactor * normal / dot(a, cross(b, c)));
}
