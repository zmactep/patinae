//! Pull-based input to `RenderState::sync` — the single bridge between the
//! host (`patinae`, CLI, web, tests) and the renderer.
//!
//! `patinae-render` deliberately knows nothing about `patinae-scene::Viewer`.
//!
//! Color resolution lives on the **host** side: per-atom base colours and
//! representation-specific colour overrides are pre-computed (once per
//! rebuild) and passed in via [`atom_colors`] / [`atom_rep_colors`]. The
//! renderer fuses them into one GPU colour LUT and applies per-rep alpha
//! (sphere_transparency, etc.) on top. This keeps the renderer free of any
//! palette / color-resolution code, and preserves the dependency graph:
//! scene/color policy stays upstream of the renderer.
//!
//! [`atom_colors`]: RenderObjectInput::atom_colors
//! [`atom_rep_colors`]: RenderObjectInput::atom_rep_colors

use bytemuck::{Pod, Zeroable};
use patinae_algos::surface::Grid3D;
use patinae_mol::{CoordSet, DirtyFlags, ObjectMolecule, RepMask};
use patinae_settings::ResolvedSettings;

use crate::picking::ObjectId;

/// Sentinel in [`RepColorLutEntry`]: use the base `atom_colors` entry.
pub const REP_COLOR_INHERIT: u32 = u32::MAX;

/// Packed RGB overrides for one atom, one slot per representation.
///
/// Values are `0x00BBGGRR` in little-endian byte order. `REP_COLOR_INHERIT`
/// means "read the base colour from `color_lut`". Alpha is intentionally not
/// stored here; representation transparency still flows through the per-rep
/// params uniform and `AtomGpu` alpha override bytes.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Pod, Zeroable)]
pub struct RepColorLutEntry {
    pub sphere: u32,
    pub stick: u32,
    pub line: u32,
    pub dot: u32,
    pub cartoon: u32,
    pub ribbon: u32,
    pub surface: u32,
    pub mesh: u32,
    pub ellipsoid: u32,
    pub _pad0: u32,
    pub _pad1: u32,
    pub _pad2: u32,
}

impl RepColorLutEntry {
    pub const fn inherit_all() -> Self {
        Self {
            sphere: REP_COLOR_INHERIT,
            stick: REP_COLOR_INHERIT,
            line: REP_COLOR_INHERIT,
            dot: REP_COLOR_INHERIT,
            cartoon: REP_COLOR_INHERIT,
            ribbon: REP_COLOR_INHERIT,
            surface: REP_COLOR_INHERIT,
            mesh: REP_COLOR_INHERIT,
            ellipsoid: REP_COLOR_INHERIT,
            _pad0: REP_COLOR_INHERIT,
            _pad1: REP_COLOR_INHERIT,
            _pad2: REP_COLOR_INHERIT,
        }
    }
}

impl Default for RepColorLutEntry {
    fn default() -> Self {
        Self::inherit_all()
    }
}

/// SceneStore colour payload for one atom.
///
/// Base RGBA and representation-specific RGB overrides are deliberately
/// fused into one storage buffer. WebGPU's portable
/// `max_storage_buffers_per_shader_stage` limit is 8; keeping colour data in
/// one binding leaves group 2 inside that limit on Metal/WebGPU backends.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct ColorLutEntry {
    pub base: [f32; 4],
    pub reps: RepColorLutEntry,
}

impl ColorLutEntry {
    pub const fn new(base: [f32; 4], reps: RepColorLutEntry) -> Self {
        Self { base, reps }
    }
}

impl Default for ColorLutEntry {
    fn default() -> Self {
        Self::new([1.0, 1.0, 1.0, 1.0], RepColorLutEntry::inherit_all())
    }
}

/// Pack host-resolved linear RGB into the compact SceneStore rep-colour LUT.
pub fn pack_rep_rgb8(rgba: [f32; 4]) -> u32 {
    let to_byte = |v: f32| -> u32 { (v.clamp(0.0, 1.0) * 255.0).round() as u32 };
    to_byte(rgba[0]) | (to_byte(rgba[1]) << 8) | (to_byte(rgba[2]) << 16)
}

/// Per-atom marker change for incremental selection-overlay updates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkerUpdate {
    pub atom_index: u32,
    pub bits: u32,
}

/// Whole-frame input. The host packages its world state into this once per
/// `sync`; the renderer rebuilds dirty representations and uploads to GPU.
pub struct RenderInput<'a> {
    pub objects: &'a [RenderObjectInput<'a>],
    pub maps: &'a [RenderMapInput<'a>],
    pub strokes: &'a [RenderStrokeInput<'a>],
    pub settings: &'a ResolvedSettings,
    /// Scene-wide level-of-detail bucket, derived from the sum of atoms
    /// across all visible objects. Reps that produce O(N²)-ish vertex
    /// counts (cartoon, surface) downscale `n_samples_per` /
    /// `profile_segments` / grid spacing in the bigger buckets so a 2.4M-
    /// atom assembly stays inside GPU memory and bandwidth budgets. Hosts
    /// not threading this can pass `SceneLod::Auto`, in which case
    /// representations apply their own per-chain heuristics (or the
    /// settings-side `cartoon_quality` enum) without further downscaling.
    /// Sphere and stick use this only for the `Minimum` bucket's automatic
    /// sampling.
    pub lod: SceneLod,
}

/// Renderable map contour mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderMapMode {
    /// Wireframe contour lines.
    Isomesh,
    /// Solid contour triangles.
    Isosurface,
}

/// One drawable map contour object.
pub struct RenderMapInput<'a> {
    pub object_id: ObjectId,
    pub grid: &'a Grid3D,
    pub mode: RenderMapMode,
    pub level: f32,
    pub color: [f32; 4],
    pub transform: [[f32; 4]; 4],
    pub geometry_revision: u64,
    pub material_revision: u64,
    pub dirty: bool,
}

/// One styled world-space line segment rendered as a screen-space quad.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct StrokeSegment {
    pub start: [f32; 3],
    /// Width in physical viewport pixels.
    pub width_px: f32,
    pub end: [f32; 3],
    /// Non-zero when the fragment shader should render round caps.
    pub round_ends: u32,
    /// Per-segment RGBA, allowing heterogeneous styles in one owner.
    pub color: [f32; 4],
}

impl StrokeSegment {
    /// Creates one fully styled segment instance.
    pub const fn new(
        start: [f32; 3],
        end: [f32; 3],
        color: [f32; 4],
        width_px: f32,
        round_ends: bool,
    ) -> Self {
        Self {
            start,
            width_px,
            end,
            round_ends: round_ends as u32,
            color,
        }
    }

    pub(crate) fn vertex_layout() -> wgpu::VertexBufferLayout<'static> {
        const ATTRS: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
            0 => Float32x3,
            1 => Float32,
            2 => Float32x3,
            3 => Uint32,
            4 => Float32x4
        ];
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTRS,
        }
    }
}

/// Renderer payload for all strokes owned by one semantic scene object.
pub struct RenderStrokeInput<'a> {
    pub object_id: ObjectId,
    pub segments: &'a [StrokeSegment],
    /// World-space bounds shared with focus and zoom resolution.
    pub bounds: Option<([f32; 3], [f32; 3])>,
    pub geometry_revision: u64,
    pub material_revision: u64,
}

/// Scene-wide level-of-detail bucket. Larger structures auto-downgrade
/// quality knobs in representations that scale super-linearly with atom
/// count. The buckets correspond to the large-assembly LOD plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneLod {
    /// Default — host did not classify the scene; representations apply
    /// settings-driven defaults with no extra LOD pressure.
    Auto,
    /// Total scene atoms < 10k: full per-residue sampling, full profile.
    High,
    /// 10k–50k: halve a step (8 samples / 10 profile segments by default).
    Medium,
    /// 50k–500k: aggressive (6 samples / 8 profile).
    Low,
    /// > 500k (3J3Q-class): minimum interactive quality (4 samples / 6
    /// > profile). Cartoon vertex count drops ~10× vs `High` here.
    Minimum,
}

impl SceneLod {
    /// Classify a scene by its total atom count (sum across all enabled
    /// molecule objects). Buckets follow the large-assembly LOD thresholds.
    pub fn from_atom_count(total_atoms: usize) -> Self {
        match total_atoms {
            0..=9_999 => SceneLod::High,
            10_000..=49_999 => SceneLod::Medium,
            50_000..=499_999 => SceneLod::Low,
            _ => SceneLod::Minimum,
        }
    }
}

/// One drawable object — atomic data + per-object overrides.
///
/// `object_id` is opaque to the renderer: the host picks a stable render id
/// for the object's lifetime in the scene registry. The renderer round-trips
/// it through the picking texture and the host resolves it back to its own
/// object representation. **Never use `ObjectId(0)`** — that value is reserved
/// as the "no hit" sentinel.
pub struct RenderObjectInput<'a> {
    pub object_id: ObjectId,
    pub molecule: &'a ObjectMolecule,
    pub coord_set: &'a CoordSet,
    /// Object-to-world transform, stored as column-major matrix columns.
    pub transform: [[f32; 4]; 4],
    /// Bitmask of representations materialized on at least one atom.
    ///
    /// Atoms still gate per-rep visibility individually via
    /// `atom.repr.visible_reps`.
    pub visible_reps: RepMask,
    /// Bitmask of representations drawn this frame at object level.
    pub draw_reps: RepMask,
    /// Per-object settings resolved from object overrides. `None` means "use
    /// the global block as-is".
    pub object_settings: Option<ResolvedSettings>,
    /// Pre-resolved base RGBA per atom, indexed by `AtomIndex`. Length must
    /// equal `molecule.atoms().len()`. Element / chain / SS / b-factor mapping
    /// happens on the host before this call. Per-rep alpha (e.g.
    /// `sphere_transparency`) is applied by the renderer on top.
    pub atom_colors: &'a [[f32; 4]],
    /// Packed per-representation colour overrides, indexed by `AtomIndex`.
    /// Length should equal `molecule.atoms().len()`. When empty or shorter
    /// than the atom count, missing entries inherit `atom_colors`.
    pub atom_rep_colors: &'a [RepColorLutEntry],
    /// Pre-packed per-atom marker bits, indexed by local `AtomIndex`. Length
    /// must equal `molecule.atoms().len()`. Bit layout in
    /// `crate::scene_store::marker` (bit 0 = selected, bit 1 = hover; higher
    /// bits reserved). The renderer uploads this directly into the
    /// scene-wide `marker_lut` slice owned by this object — no aliasing
    /// across multi-object scenes. Hosts that don't have selection state
    /// can pass `&[]`, in which case the renderer treats every atom as
    /// unmarked (slice shorter than `atom_count` ⇒ tail interpreted as 0).
    pub atom_markers: &'a [u32],
    /// Sparse marker changes since the previous frame. Hosts use this for
    /// hover-only updates so large assemblies do not upload an object's
    /// whole marker LUT when only one atom changed.
    pub marker_updates: &'a [MarkerUpdate],
    /// True when this object currently has at least one non-zero marker bit.
    pub has_markers: bool,
    /// Scene-wide LOD bucket — copied from [`RenderInput::lod`] by the
    /// caller. Reps that produce O(N²)-ish vertex buffers (cartoon,
    /// surface) downscale quality knobs based on this. Sphere and stick
    /// apply automatic sampling in `Minimum`.
    pub lod: SceneLod,
    /// What changed for this object since the last `sync`. Empty means the
    /// renderer reuses last frame's geometry / materials / instance buffers
    /// without any `write_buffer` or atom iteration — the dominant cost on
    /// assemblies with hundreds of chains.
    ///
    /// When non-empty, individual representations decide which work to redo
    /// based on the bit pattern: a `COLOR`-only flip flushes the colour LUT
    /// without rebuilding instance buffers; a `COORDS` change rebuilds
    /// instances; a `TOPOLOGY` change rebuilds everything. See
    /// `patinae_mol::DirtyFlags` and `DirtyFlags::is_lut_only`.
    ///
    /// Hosts wire this from `MoleculeObject::dirty_flags()` and clear the
    /// per-object flags *after* `sync()`.
    pub dirty: DirtyFlags,
}

/// Column-major identity transform for hosts that keep molecule coordinates
/// directly in world space.
pub const IDENTITY_TRANSFORM: [[f32; 4]; 4] = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

#[cfg(test)]
mod tests {
    use super::StrokeSegment;

    #[test]
    fn stroke_segment_keeps_heterogeneous_instance_material() {
        let segment = StrokeSegment::new(
            [1.0, 2.0, 3.0],
            [4.0, 5.0, 6.0],
            [0.1, 0.2, 0.3, 0.4],
            3.5,
            true,
        );

        assert_eq!(std::mem::size_of::<StrokeSegment>(), 48);
        assert_eq!(segment.color, [0.1, 0.2, 0.3, 0.4]);
        assert_eq!(segment.width_px, 3.5);
        assert_eq!(segment.round_ends, 1);
    }
}
