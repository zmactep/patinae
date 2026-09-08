//! Surface / Mesh representation.
//!
//! Single `SurfaceRep` struct serves **two** rep kinds via a `mode` field
//! (mirroring CartoonRep's Cartoon/Ribbon pattern):
//!
//! - `SurfaceMode::Surface` → `RepKind::Surface`. Solid triangulated
//!   isosurface. Reads `settings.surface.*`. MC kernel runs with
//!   `emit_lines = 0`, output = TriangleList.
//! - `SurfaceMode::Mesh` → `RepKind::Mesh`. Wireframe of the same
//!   isosurface. Reads `settings.mesh.*` (own quality / solvent /
//!   transparency). MC kernel runs with `emit_lines = 1`, output =
//!   LineList (6 vertices per triangle, edge permutation
//!   `[0,1, 1,2, 2,0]`).
//!
//! Both modes share the entire compute machinery (density / SDF / morph
//! / MC pipelines on `RenderState`). They have **independent** voxel
//! grids — `mesh_quality` and `surface_quality` may differ, hence
//! independent textures + vertex buffers per rep instance.
//!
//! `build()` runs CPU prep (bbox, grid alloc/realloc, params upload) and
//! sets `needs_dispatch = true`. `dispatch_compute_build()` records the
//! producer chain (SAS density OR vdW SDF + SES morph) followed by the
//! marching-cubes pass into the shared encoder, then copies the atomic
//! vertex counter into byte 0 of the indirect args.
//!
//! Picking semantics: `atom_id = owner` — the atom whose
//! vdW sphere is nearest to the voxel that emitted the triangle/line.
//! `atom_id` is **object-local**; the FS prepends `obj.atom_offset` for
//! the scene-wide LUTs. `RepKind` distinguishes Surface vs Mesh clicks.

mod constants;
pub mod mc_tables;
#[cfg(test)]
mod oracle;

use bytemuck::{Pod, Zeroable};
use patinae_settings::{ResolvedSettings, SurfaceMode as SettingSurfaceMode};
use wgpu::util::DeviceExt;

use crate::compute::surface_density::{
    DensityParams, SurfaceAccelParams, SurfaceDensityCompute, SurfaceDensityInputs,
};
use crate::compute::surface_mc::{
    create_counter_buffer, create_vertex_buffer, McParams, SurfaceMcInputs,
};
use crate::compute::surface_ses_morph::{
    stencil_radius_voxels, MorphParams, SurfaceSesMorphCompute, SurfaceSesMorphInputs,
};
use crate::compute::surface_vdw_sdf::{SdfParams, SurfaceVdwSdfCompute, SurfaceVdwSdfInputs};
use crate::memory::{buffer_usage, GpuMemoryUsage};
use crate::picking::RepKind;
use crate::pipelines::mesh::MeshParamsLayout;
use crate::pipelines::surface::{SurfaceParams, SurfaceParamsLayout};
use crate::render_input::{RenderObjectInput, SceneLod};
use crate::representation_budget::{RepBuildDecision, RepMemoryEstimate, RepQualityLevel};
use crate::representations::surface::constants::{SAS_ALPHA, SAS_ISO};
use crate::representations::{BuildCtx, DrawPhase, Representation};
use patinae_mol::{Atom, AtomIndex, DirtyFlags, RepMask};

/// Mode flag for `SurfaceRep` — selects between solid triangulated
/// surface and wireframe mesh. Mirrors `CartoonMode` (Cartoon/Ribbon).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceMode {
    /// Solid isosurface — TriangleList rendering. Reads
    /// `settings.surface.*`.
    Surface,
    /// Wireframe isomesh — LineList rendering, 6 verts per MC triangle.
    /// Reads `settings.mesh.*` (own quality / transparency / solvent).
    Mesh,
}

impl SurfaceMode {
    fn rep_kind(self) -> RepKind {
        match self {
            SurfaceMode::Surface => RepKind::Surface,
            SurfaceMode::Mesh => RepKind::Mesh,
        }
    }
    fn emit_lines(self) -> u32 {
        match self {
            SurfaceMode::Surface => 0,
            SurfaceMode::Mesh => 1,
        }
    }

    fn draw_phase(self, is_opaque: bool) -> DrawPhase {
        match (self, is_opaque) {
            (SurfaceMode::Mesh, true) => DrawPhase::FastOverlay,
            (_, true) => DrawPhase::Opaque,
            (_, false) => DrawPhase::Wboit,
        }
    }

    fn casts_shadow(self, is_opaque: bool) -> bool {
        self == SurfaceMode::Surface && is_opaque
    }
}

/// 24-byte vertex emitted by the marching-cubes pass. Mirrors
/// `representations::mesh::StdVertex`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct SurfaceVertex {
    pub position: [f32; 3],
    pub normal_oct: u32,
    pub group_id: u32,
    pub flags: u32,
}

impl SurfaceVertex {
    pub const SIZE: u64 = std::mem::size_of::<Self>() as u64;
}

const MAX_DIM: u32 = 192;
const MAX_SURFACE_VOXELS: u64 = 1_500_000;
const MAX_ACCEL_CELLS: u64 = 4_000_000;
const SURFACE_ACCEL_SUPPORT_FACTOR: f32 = 1.5;
const MAX_VERTICES: u32 = 4_000_000;
const SURFACE_TILE_TRIGGER_VOXEL_SIZE: f32 = 3.0;
const SURFACE_TILE_CORE_SPAN: f32 = 180.0;

#[derive(Debug)]
struct PendingDispatch {
    mc_params: McParams,
    bbox_min: [f32; 3],
    voxel_size: f32,
    dims: [u32; 3],
    probe_radius: f32,
    is_ses: bool,
    accel: SurfaceAccelBuild,
}

pub struct SurfaceRep {
    mode: SurfaceMode,
    parts: Vec<SurfacePart>,
    last_build: Option<BuildSignature>,

    render_params_buffer: wgpu::Buffer,
    render_params_bind_group: Option<wgpu::BindGroup>,

    /// Set by `build()` when geometry must be rebuilt; consumed by
    /// `dispatch_compute_build`.
    needs_dispatch: bool,
    is_opaque_cache: bool,
    budget_coarsened: bool,
}

struct SurfacePart {
    chunks: Vec<SurfaceDrawChunk>,
}

struct SurfaceDrawChunk {
    draw: DrawResources,
    pending: Option<PendingDispatch>,
}

struct DrawResources {
    vertex_buf: wgpu::Buffer,
    counter_buf: wgpu::Buffer,
    indirect_buf: wgpu::Buffer,
    max_vertices: u32,
}

struct SurfaceComputeScratch {
    mc_inputs: SurfaceMcInputs,
    density_inputs: Option<SurfaceDensityInputs>,
    vdw_sdf_inputs: Option<SurfaceVdwSdfInputs>,
    ses_morph_inputs: Option<SurfaceSesMorphInputs>,

    _density_tex: wgpu::Texture,
    _owner_tex: wgpu::Texture,
    _sdf_tex: Option<wgpu::Texture>,
    _density_view: wgpu::TextureView,
    _owner_view: wgpu::TextureView,
    _sdf_view: Option<wgpu::TextureView>,
    _cell_offsets_buf: wgpu::Buffer,
    _cell_indices_buf: wgpu::Buffer,
}

pub(crate) struct SurfaceExportPart<'a> {
    pub(crate) vertex_buf: &'a wgpu::Buffer,
    pub(crate) indirect_buf: &'a wgpu::Buffer,
    pub(crate) max_vertices: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct BuildSignature {
    settings_hash: u64,
    parts_hash: u64,
}

impl SurfaceRep {
    pub fn new(device: &wgpu::Device) -> Self {
        Self::with_mode(device, SurfaceMode::Surface)
    }
    pub fn new_mesh(device: &wgpu::Device) -> Self {
        Self::with_mode(device, SurfaceMode::Mesh)
    }
    fn with_mode(device: &wgpu::Device, mode: SurfaceMode) -> Self {
        // Render params struct is identical between modes (alpha_mul + pad);
        // SIZE is the same. Layout binding type differs only in label.
        let render_params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(match mode {
                SurfaceMode::Surface => "patinae.surface.render_params",
                SurfaceMode::Mesh => "patinae.mesh.render_params",
            }),
            size: SurfaceParams::SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            mode,
            parts: Vec::new(),
            last_build: None,
            render_params_buffer,
            render_params_bind_group: None,
            needs_dispatch: false,
            is_opaque_cache: true,
            budget_coarsened: false,
        }
    }

    pub fn ensure_render_bind_group(
        &mut self,
        device: &wgpu::Device,
        surface_layout: &SurfaceParamsLayout,
        mesh_layout: &MeshParamsLayout,
    ) {
        if self.render_params_bind_group.is_some() {
            return;
        }
        let (label, bgl) = match self.mode {
            SurfaceMode::Surface => (
                "patinae.surface.render_bg",
                &surface_layout.bind_group_layout,
            ),
            SurfaceMode::Mesh => ("patinae.mesh.render_bg", &mesh_layout.bind_group_layout),
        };
        self.render_params_bind_group =
            Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: bgl,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.render_params_buffer.as_entire_binding(),
                }],
            }));
    }

    pub(crate) fn export_parts(&self) -> impl Iterator<Item = SurfaceExportPart<'_>> {
        self.parts.iter().flat_map(|part| {
            part.chunks.iter().map(|chunk| SurfaceExportPart {
                vertex_buf: &chunk.draw.vertex_buf,
                indirect_buf: &chunk.draw.indirect_buf,
                max_vertices: chunk.draw.max_vertices,
            })
        })
    }
}

fn voxel_size_for_quality(quality: i32, lod: SceneLod) -> f32 {
    let q = quality.clamp(0, 6) as f32;
    let base = if q < 1.0 { 1.0 } else { 1.0 / q };
    match lod {
        SceneLod::Low => base.max(0.75),
        SceneLod::Minimum => base.max(1.0),
        SceneLod::Auto | SceneLod::High | SceneLod::Medium => base,
    }
}

fn coarsened_voxel_size(requested_voxel_size: f32) -> f32 {
    (requested_voxel_size * 2.0).max(1.0)
}

fn hash_surface_atoms(atoms: &[SurfaceAccelAtom]) -> u64 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for atom in atoms {
        h.write_u32(atom.local_id);
        h.write_u32(atom.pos[0].to_bits());
        h.write_u32(atom.pos[1].to_bits());
        h.write_u32(atom.pos[2].to_bits());
    }
    h.finish()
}

#[derive(Debug, Clone)]
struct SurfaceBuildPrep {
    bbox_min: [f32; 3],
    dims: [u32; 3],
    voxel_size: f32,
    emit_core_min: [f32; 3],
    emit_core_max: [f32; 3],
    n_kept: usize,
    coord_hash: u64,
    max_radius: f32,
    atoms: Vec<SurfaceAccelAtom>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct SurfaceTileGeom {
    bbox_min: [f32; 3],
    dims: [u32; 3],
    voxel_size: f32,
    emit_core_min: [f32; 3],
    emit_core_max: [f32; 3],
}

#[derive(Debug, Clone, Copy)]
struct SurfaceAccelAtom {
    local_id: u32,
    pos: [f32; 3],
}

#[derive(Debug, Clone)]
struct SurfaceAtomSet {
    lo: [f32; 3],
    hi: [f32; 3],
    max_radius: f32,
    atoms: Vec<SurfaceAccelAtom>,
}

#[derive(Debug)]
struct SurfaceAccelBuild {
    params: SurfaceAccelParams,
    offsets: Vec<u32>,
    indices: Vec<u32>,
}

fn grid_domain_max(bbox_min: [f32; 3], dims: [u32; 3], voxel_size: f32) -> [f32; 3] {
    [
        bbox_min[0] + dims[0].saturating_sub(1) as f32 * voxel_size,
        bbox_min[1] + dims[1].saturating_sub(1) as f32 * voxel_size,
        bbox_min[2] + dims[2].saturating_sub(1) as f32 * voxel_size,
    ]
}

fn grid_layout_from_bounds(
    lo: [f32; 3],
    hi: [f32; 3],
    max_radius: f32,
    probe_radius: f32,
    requested_voxel_size: f32,
) -> ([f32; 3], [u32; 3], f32) {
    let base_span = [
        (hi[0] - lo[0]).max(0.0),
        (hi[1] - lo[1]).max(0.0),
        (hi[2] - lo[2]).max(0.0),
    ];
    let mut voxel_size = requested_voxel_size.max(1e-3);
    for _ in 0..6 {
        let margin = max_radius + probe_radius + 2.0 * voxel_size;
        let extent = [
            base_span[0] + 2.0 * margin,
            base_span[1] + 2.0 * margin,
            base_span[2] + 2.0 * margin,
        ];
        let axis_limit = extent
            .iter()
            .map(|span| span / ((MAX_DIM - 1) as f32))
            .fold(0.0_f32, f32::max);
        let volume_limit = (extent[0] * extent[1] * extent[2] / (MAX_SURFACE_VOXELS as f32)).cbrt();
        let required = axis_limit.max(volume_limit).max(requested_voxel_size);
        if required <= voxel_size * 1.001 {
            break;
        }
        voxel_size = required;
    }

    let margin = max_radius + probe_radius + 2.0 * voxel_size;
    let mut bbox_min = [0.0_f32; 3];
    let mut dims = [2_u32; 3];
    for k in 0..3 {
        bbox_min[k] = lo[k] - margin;
        let extent = base_span[k] + 2.0 * margin;
        dims[k] = ((extent / voxel_size).ceil() as u32 + 1).clamp(2, MAX_DIM);
    }
    (bbox_min, dims, voxel_size)
}

fn required_voxel_size_for_extent(extent: [f32; 3], requested_voxel_size: f32) -> f32 {
    let axis_limit = extent
        .iter()
        .map(|span| span / ((MAX_DIM - 1) as f32))
        .fold(0.0_f32, f32::max);
    let volume_limit = (extent[0] * extent[1] * extent[2] / (MAX_SURFACE_VOXELS as f32)).cbrt();
    axis_limit.max(volume_limit).max(requested_voxel_size)
}

fn tiled_surface_voxel_size(max_radius: f32, probe_radius: f32, requested_voxel_size: f32) -> f32 {
    let mut voxel_size = requested_voxel_size.max(1e-3);
    for _ in 0..8 {
        let margin = max_radius + probe_radius + 2.0 * voxel_size;
        // Snapping field bounds to the shared tile lattice can add up to one
        // voxel on each side, so budget it into the worst-case tile extent.
        let span = SURFACE_TILE_CORE_SPAN + 2.0 * margin + 2.0 * voxel_size;
        let required = required_voxel_size_for_extent([span, span, span], requested_voxel_size);
        if required <= voxel_size * 1.001 {
            break;
        }
        voxel_size = required;
    }
    voxel_size
}

fn surface_domain_from_bounds(
    lo: [f32; 3],
    hi: [f32; 3],
    max_radius: f32,
    probe_radius: f32,
    voxel_size: f32,
) -> ([f32; 3], [f32; 3]) {
    let margin = max_radius + probe_radius + 2.0 * voxel_size;
    (
        [lo[0] - margin, lo[1] - margin, lo[2] - margin],
        [hi[0] + margin, hi[1] + margin, hi[2] + margin],
    )
}

fn axis_tile_ranges(lo: f32, hi: f32) -> Vec<(f32, f32)> {
    let mut ranges = Vec::new();
    let mut start = lo;
    while start < hi {
        let end = (start + SURFACE_TILE_CORE_SPAN).min(hi);
        ranges.push((start, end));
        if end >= hi {
            break;
        }
        start = end;
    }
    if ranges.is_empty() {
        ranges.push((lo, hi));
    }
    ranges
}

fn tile_core_bounds_for_domain(
    domain_min: [f32; 3],
    domain_max: [f32; 3],
) -> Vec<([f32; 3], [f32; 3])> {
    let xs = axis_tile_ranges(domain_min[0], domain_max[0]);
    let ys = axis_tile_ranges(domain_min[1], domain_max[1]);
    let zs = axis_tile_ranges(domain_min[2], domain_max[2]);
    let mut bounds = Vec::with_capacity(xs.len() * ys.len() * zs.len());
    for &(x0, x1) in &xs {
        for &(y0, y1) in &ys {
            for &(z0, z1) in &zs {
                bounds.push(([x0, y0, z0], [x1, y1, z1]));
            }
        }
    }
    bounds
}

fn snap_floor_to_lattice(value: f32, origin: f32, voxel_size: f32) -> f32 {
    origin + ((value - origin) / voxel_size).floor() * voxel_size
}

fn snap_ceil_to_lattice(value: f32, origin: f32, voxel_size: f32) -> f32 {
    origin + ((value - origin) / voxel_size).ceil() * voxel_size
}

fn dims_for_snapped_domain(
    domain_min: [f32; 3],
    domain_max: [f32; 3],
    voxel_size: f32,
) -> [u32; 3] {
    let mut dims = [2_u32; 3];
    for k in 0..3 {
        let span = (domain_max[k] - domain_min[k]).max(voxel_size);
        dims[k] = ((span / voxel_size).ceil() as u32 + 1).clamp(2, MAX_DIM);
    }
    dims
}

fn bounds_contains_point_with_margin(lo: [f32; 3], hi: [f32; 3], margin: f32, p: [f32; 3]) -> bool {
    (0..3).all(|k| p[k] >= lo[k] - margin && p[k] <= hi[k] + margin)
}

fn make_surface_build_prep(
    bbox_min: [f32; 3],
    dims: [u32; 3],
    voxel_size: f32,
    emit_core_min: [f32; 3],
    emit_core_max: [f32; 3],
    max_radius: f32,
    atoms: Vec<SurfaceAccelAtom>,
) -> SurfaceBuildPrep {
    let coord_hash = hash_surface_atoms(&atoms);
    SurfaceBuildPrep {
        bbox_min,
        dims,
        voxel_size,
        emit_core_min,
        emit_core_max,
        n_kept: atoms.len(),
        coord_hash,
        max_radius,
        atoms,
    }
}

fn make_surface_tile_geom(
    bbox_min: [f32; 3],
    dims: [u32; 3],
    voxel_size: f32,
    emit_core_min: [f32; 3],
    emit_core_max: [f32; 3],
) -> SurfaceTileGeom {
    SurfaceTileGeom {
        bbox_min,
        dims,
        voxel_size,
        emit_core_min,
        emit_core_max,
    }
}

fn atom_matches_surface_mode(atom: &Atom, mode: SettingSurfaceMode, rep_mask: RepMask) -> bool {
    match mode {
        SettingSurfaceMode::Normal => atom.state.flags.is_biomolecule(),
        SettingSurfaceMode::All => true,
        SettingSurfaceMode::Heavy => atom.is_heavy(),
        SettingSurfaceMode::Visible => atom.repr.visible_reps.is_visible(rep_mask),
        SettingSurfaceMode::VisibleHeavy => {
            atom.repr.visible_reps.is_visible(rep_mask) && atom.is_heavy()
        }
    }
}

fn solvent_setting_to_is_ses(solvent_accessible: bool) -> bool {
    // PyMOL's *_solvent settings use `on` for solvent-accessible surfaces.
    // The renderer's compute branch is named for the opposite SES path.
    !solvent_accessible
}

fn surface_atom_partitions(
    input: &RenderObjectInput,
    mode: SettingSurfaceMode,
    individual_chains: bool,
    rep_mask: RepMask,
) -> Vec<Vec<u32>> {
    if individual_chains {
        let mut parts = Vec::new();
        for subchain in input.molecule.subchains() {
            let ids: Vec<u32> = subchain
                .iter_indexed()
                .filter_map(|(atom_idx, atom)| {
                    if input.includes_source_atom(atom_idx.as_u32())
                        && atom_matches_surface_mode(atom, mode, rep_mask)
                        && input.coord_set.get_atom_coord(atom_idx).is_some()
                    {
                        Some(atom_idx.as_u32())
                    } else {
                        None
                    }
                })
                .collect();
            if !ids.is_empty() {
                parts.push(ids);
            }
        }
        parts
    } else {
        let ids: Vec<u32> = input
            .coord_set
            .iter_with_atoms()
            .filter_map(|(atom_idx, _)| {
                let atom = input.molecule.get_atom(atom_idx)?;
                (input.includes_source_atom(atom_idx.as_u32())
                    && atom_matches_surface_mode(atom, mode, rep_mask))
                .then_some(atom_idx.as_u32())
            })
            .collect();
        if ids.is_empty() {
            Vec::new()
        } else {
            vec![ids]
        }
    }
}

/// Collect object-local atoms for one semantic surface partition. Zero-radius
/// atoms are excluded from bbox/hash/accel; the compute kernels also skip
/// them, so owner ids stay object-local and aligned.
fn collect_surface_atom_set(input: &RenderObjectInput, atom_ids: &[u32]) -> Option<SurfaceAtomSet> {
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    let mut r_max = 0.0_f32;
    let mut atoms: Vec<SurfaceAccelAtom> = Vec::with_capacity(atom_ids.len());
    for &atom_id in atom_ids {
        let atom_idx = AtomIndex(atom_id);
        let atom = match input.molecule.get_atom(atom_idx) {
            Some(a) => a,
            None => continue,
        };
        let coord = match input.coord_set.get_atom_coord(atom_idx) {
            Some(c) => c,
            None => continue,
        };
        let r = atom.effective_vdw();
        if r <= 0.0 {
            continue;
        }
        let p = [coord.x, coord.y, coord.z];
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
        r_max = r_max.max(r);
        atoms.push(SurfaceAccelAtom {
            local_id: atom_idx.as_u32(),
            pos: p,
        });
    }
    if atoms.is_empty() {
        return None;
    }
    Some(SurfaceAtomSet {
        lo,
        hi,
        max_radius: r_max,
        atoms,
    })
}

fn prepare_surface_builds_from_atom_set(
    atom_set: &SurfaceAtomSet,
    probe_radius: f32,
    requested_voxel_size: f32,
) -> Vec<SurfaceBuildPrep> {
    let (bbox_min, dims, voxel_size) = grid_layout_from_bounds(
        atom_set.lo,
        atom_set.hi,
        atom_set.max_radius,
        probe_radius,
        requested_voxel_size,
    );
    if voxel_size <= SURFACE_TILE_TRIGGER_VOXEL_SIZE {
        let emit_core_min = bbox_min;
        let emit_core_max = grid_domain_max(bbox_min, dims, voxel_size);
        return vec![make_surface_build_prep(
            bbox_min,
            dims,
            voxel_size,
            emit_core_min,
            emit_core_max,
            atom_set.max_radius,
            atom_set.atoms.clone(),
        )];
    }

    let tile_voxel_size =
        tiled_surface_voxel_size(atom_set.max_radius, probe_radius, requested_voxel_size);
    let (domain_min, domain_max) = surface_domain_from_bounds(
        atom_set.lo,
        atom_set.hi,
        atom_set.max_radius,
        probe_radius,
        tile_voxel_size,
    );
    let field_margin = atom_set.max_radius + probe_radius + 2.0 * tile_voxel_size;
    let support_radius = ((atom_set.max_radius + probe_radius) * SURFACE_ACCEL_SUPPORT_FACTOR)
        .max(tile_voxel_size * 2.0)
        .max(1e-3);

    let mut preps = Vec::new();
    for (core_min, core_max) in tile_core_bounds_for_domain(domain_min, domain_max) {
        let mut field_min = [0.0_f32; 3];
        let mut field_max = [0.0_f32; 3];
        for k in 0..3 {
            field_min[k] =
                snap_floor_to_lattice(core_min[k] - field_margin, domain_min[k], tile_voxel_size);
            field_max[k] =
                snap_ceil_to_lattice(core_max[k] + field_margin, domain_min[k], tile_voxel_size);
        }
        let atoms: Vec<SurfaceAccelAtom> = atom_set
            .atoms
            .iter()
            .copied()
            .filter(|atom| {
                bounds_contains_point_with_margin(field_min, field_max, support_radius, atom.pos)
            })
            .collect();
        if atoms.is_empty() {
            continue;
        }

        let dims = dims_for_snapped_domain(field_min, field_max, tile_voxel_size);
        preps.push(make_surface_build_prep(
            field_min,
            dims,
            tile_voxel_size,
            core_min,
            core_max,
            atom_set.max_radius,
            atoms,
        ));
    }
    preps
}

fn surface_tile_geoms(
    atom_set: &SurfaceAtomSet,
    probe_radius: f32,
    requested_voxel_size: f32,
) -> Vec<SurfaceTileGeom> {
    let (bbox_min, dims, voxel_size) = grid_layout_from_bounds(
        atom_set.lo,
        atom_set.hi,
        atom_set.max_radius,
        probe_radius,
        requested_voxel_size,
    );
    if voxel_size <= SURFACE_TILE_TRIGGER_VOXEL_SIZE {
        let emit_core_min = bbox_min;
        let emit_core_max = grid_domain_max(bbox_min, dims, voxel_size);
        return vec![make_surface_tile_geom(
            bbox_min,
            dims,
            voxel_size,
            emit_core_min,
            emit_core_max,
        )];
    }

    let tile_voxel_size =
        tiled_surface_voxel_size(atom_set.max_radius, probe_radius, requested_voxel_size);
    let (domain_min, domain_max) = surface_domain_from_bounds(
        atom_set.lo,
        atom_set.hi,
        atom_set.max_radius,
        probe_radius,
        tile_voxel_size,
    );
    let field_margin = atom_set.max_radius + probe_radius + 2.0 * tile_voxel_size;

    tile_core_bounds_for_domain(domain_min, domain_max)
        .into_iter()
        .map(|(core_min, core_max)| {
            let mut field_min = [0.0_f32; 3];
            let mut field_max = [0.0_f32; 3];
            for k in 0..3 {
                field_min[k] = snap_floor_to_lattice(
                    core_min[k] - field_margin,
                    domain_min[k],
                    tile_voxel_size,
                );
                field_max[k] = snap_ceil_to_lattice(
                    core_max[k] + field_margin,
                    domain_min[k],
                    tile_voxel_size,
                );
            }
            let dims = dims_for_snapped_domain(field_min, field_max, tile_voxel_size);
            make_surface_tile_geom(field_min, dims, tile_voxel_size, core_min, core_max)
        })
        .collect()
}

fn prepare_surface_builds_for_atoms(
    input: &RenderObjectInput,
    atom_ids: &[u32],
    probe_radius: f32,
    requested_voxel_size: f32,
) -> Vec<SurfaceBuildPrep> {
    collect_surface_atom_set(input, atom_ids)
        .map(|atom_set| {
            prepare_surface_builds_from_atom_set(&atom_set, probe_radius, requested_voxel_size)
        })
        .unwrap_or_default()
}

fn accel_dims_for_extent(extent: [f32; 3], mut cell_size: f32) -> ([u32; 3], f32) {
    loop {
        let dims = [
            ((extent[0] / cell_size).ceil() as u32).max(1),
            ((extent[1] / cell_size).ceil() as u32).max(1),
            ((extent[2] / cell_size).ceil() as u32).max(1),
        ];
        let cells = dims[0] as u64 * dims[1] as u64 * dims[2] as u64;
        if cells <= MAX_ACCEL_CELLS {
            return (dims, cell_size);
        }
        let grow = (cells as f32 / MAX_ACCEL_CELLS as f32).cbrt().max(1.01);
        cell_size *= grow;
    }
}

fn accel_cell_id(dims: [u32; 3], c: [u32; 3]) -> usize {
    (c[0] + c[1] * dims[0] + c[2] * dims[0] * dims[1]) as usize
}

fn build_surface_accel(prep: &SurfaceBuildPrep, probe_radius: f32) -> SurfaceAccelBuild {
    let extent = [
        (prep.dims[0].saturating_sub(1) as f32) * prep.voxel_size,
        (prep.dims[1].saturating_sub(1) as f32) * prep.voxel_size,
        (prep.dims[2].saturating_sub(1) as f32) * prep.voxel_size,
    ];
    let support_radius = ((prep.max_radius + probe_radius) * SURFACE_ACCEL_SUPPORT_FACTOR)
        .max(prep.voxel_size * 2.0)
        .max(1e-3);
    let (cell_dims, cell_size) = accel_dims_for_extent(extent, support_radius);
    let cell_count = (cell_dims[0] as usize) * (cell_dims[1] as usize) * (cell_dims[2] as usize);

    let mut counts = vec![0_u32; cell_count];
    for atom in &prep.atoms {
        let c = accel_cell_for_point(prep.bbox_min, cell_size, cell_dims, atom.pos);
        counts[accel_cell_id(cell_dims, c)] += 1;
    }

    let mut offsets = vec![0_u32; cell_count + 1];
    for i in 0..cell_count {
        offsets[i + 1] = offsets[i] + counts[i];
    }
    let mut heads = offsets[..cell_count].to_vec();
    let mut indices = vec![0_u32; prep.atoms.len()];
    for atom in &prep.atoms {
        let c = accel_cell_for_point(prep.bbox_min, cell_size, cell_dims, atom.pos);
        let id = accel_cell_id(cell_dims, c);
        let dst = heads[id] as usize;
        indices[dst] = atom.local_id;
        heads[id] += 1;
    }

    SurfaceAccelBuild {
        params: SurfaceAccelParams {
            origin: prep.bbox_min,
            cell_size,
            dims: cell_dims,
            _pad: 0,
        },
        offsets,
        indices,
    }
}

fn accel_cell_for_point(origin: [f32; 3], cell_size: f32, dims: [u32; 3], p: [f32; 3]) -> [u32; 3] {
    let mut c = [0_u32; 3];
    for k in 0..3 {
        let raw = ((p[k] - origin[k]) / cell_size).floor() as i32;
        c[k] = raw.clamp(0, dims[k] as i32 - 1) as u32;
    }
    c
}

fn make_storage_buffer<T: Pod>(device: &wgpu::Device, label: &str, data: &[T]) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: bytemuck::cast_slice(data),
        usage: wgpu::BufferUsages::STORAGE,
    })
}

fn make_storage_3d(
    device: &wgpu::Device,
    label: &str,
    format: wgpu::TextureFormat,
    dims: [u32; 3],
    extra_usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: dims[0],
            height: dims[1],
            depth_or_array_layers: dims[2],
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC | extra_usage,
        view_formats: &[],
    })
}

#[allow(clippy::too_many_arguments)]
fn build_producer_inputs(
    device: &wgpu::Device,
    is_ses: bool,
    bbox_min: [f32; 3],
    voxel_size: f32,
    dims: [u32; 3],
    probe_radius: f32,
    density_view: &wgpu::TextureView,
    owner_view: &wgpu::TextureView,
    sdf_view: Option<&wgpu::TextureView>,
    cell_offsets_buf: &wgpu::Buffer,
    cell_indices_buf: &wgpu::Buffer,
    accel_params: SurfaceAccelParams,
    density_compute: &SurfaceDensityCompute,
    vdw_sdf_compute: &SurfaceVdwSdfCompute,
    ses_morph_compute: &SurfaceSesMorphCompute,
) -> (
    Option<SurfaceDensityInputs>,
    Option<SurfaceVdwSdfInputs>,
    Option<SurfaceSesMorphInputs>,
) {
    if is_ses {
        let sdf_view = sdf_view.expect("SES path needs an allocated r32float SDF view");
        let sdf_params = SdfParams {
            bbox_min,
            voxel_size,
            dims,
            _pad_a: 0,
        };
        let vdw_sdf_inputs = vdw_sdf_compute.build_inputs(
            device,
            crate::compute::surface_vdw_sdf::SurfaceVdwSdfBuildArgs {
                params: sdf_params,
                sdf_view,
                owner_view,
                cell_offsets: cell_offsets_buf,
                cell_indices: cell_indices_buf,
                accel_params,
            },
        );
        let r_vox = stencil_radius_voxels(probe_radius, voxel_size);
        let morph_params = MorphParams {
            dims,
            voxel_size,
            probe_radius,
            stencil_radius_voxels: r_vox,
            _pad0: 0,
            _pad1: 0,
        };
        let ses_morph_inputs =
            ses_morph_compute.build_inputs(device, morph_params, sdf_view, density_view);
        (None, Some(vdw_sdf_inputs), Some(ses_morph_inputs))
    } else {
        let density_params = DensityParams {
            bbox_min,
            voxel_size,
            dims,
            _pad_a: 0,
            probe_radius,
            alpha: SAS_ALPHA,
            _pad0: 0.0,
            _pad1: 0.0,
        };
        let density_inputs = density_compute.build_inputs(
            device,
            crate::compute::surface_density::SurfaceDensityBuildArgs {
                params: density_params,
                density_view,
                owner_view,
                cell_offsets: cell_offsets_buf,
                cell_indices: cell_indices_buf,
                accel_params,
            },
        );
        (Some(density_inputs), None, None)
    }
}

fn make_indirect_buf(device: &wgpu::Device) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("patinae.surface.indirect"),
        contents: bytemuck::cast_slice(&[0_u32, 1, 0, 0]),
        usage: wgpu::BufferUsages::INDIRECT
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
    })
}

fn surface_settings_hash(
    solvent: bool,
    probe_radius: f32,
    quality: i32,
    filter_mode: SettingSurfaceMode,
    individual_chains: bool,
    rep_mode: SurfaceMode,
    budget_coarsened: bool,
) -> u64 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    h.write_u8(solvent as u8);
    h.write_u32(probe_radius.to_bits());
    h.write_i32(quality);
    h.write_i32(filter_mode as i32);
    h.write_u8(individual_chains as u8);
    h.write_u32(rep_mode as u32);
    h.write_u8(budget_coarsened as u8);
    h.finish()
}

fn has_surface_draw_chunks(parts: &[SurfacePart]) -> bool {
    parts.iter().any(|part| !part.chunks.is_empty())
}

fn surface_geometry_invalidating_dirty() -> DirtyFlags {
    DirtyFlags::COORDS | DirtyFlags::TOPOLOGY | DirtyFlags::LOD
}

fn can_reuse_surface_geometry(
    dirty: DirtyFlags,
    has_geometry: bool,
    last_build: Option<BuildSignature>,
    settings_hash: u64,
) -> bool {
    has_geometry
        && last_build.is_some_and(|sig| sig.settings_hash == settings_hash)
        && !dirty.intersects(surface_geometry_invalidating_dirty())
}

fn surface_parts_hash(parts: &[Vec<SurfaceBuildPrep>]) -> u64 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    h.write_usize(parts.len());
    for tiles in parts {
        h.write_usize(tiles.len());
        for prep in tiles {
            h.write_usize(prep.n_kept);
            h.write_u64(prep.coord_hash);
            h.write_u32(prep.voxel_size.to_bits());
            h.write_u32(prep.dims[0]);
            h.write_u32(prep.dims[1]);
            h.write_u32(prep.dims[2]);
            for value in prep.emit_core_min {
                h.write_u32(value.to_bits());
            }
            for value in prep.emit_core_max {
                h.write_u32(value.to_bits());
            }
        }
    }
    h.finish()
}

struct DrawAllocArgs {
    max_vertices: u32,
}

fn allocate_draw_resources(device: &wgpu::Device, args: DrawAllocArgs) -> DrawResources {
    let max_vertices = args.max_vertices;
    let vertex_buf = create_vertex_buffer(device, max_vertices);
    let counter_buf = create_counter_buffer(device);
    let indirect_buf = make_indirect_buf(device);

    DrawResources {
        vertex_buf,
        counter_buf,
        indirect_buf,
        max_vertices,
    }
}

fn estimate_emit_cube_count_for_geom(
    dims: [u32; 3],
    voxel_size: f32,
    emit_core_min: [f32; 3],
    emit_core_max: [f32; 3],
) -> u64 {
    let mut cubes = 1_u64;
    for k in 0..3 {
        let core_span = (emit_core_max[k] - emit_core_min[k]).max(0.0);
        let core_cubes = ((core_span / voxel_size).ceil() as u64).max(1);
        let grid_cubes = dims[k].saturating_sub(1) as u64;
        cubes *= core_cubes.min(grid_cubes.max(1));
    }
    cubes
}

fn estimate_emit_cube_count(prep: &SurfaceBuildPrep) -> u64 {
    estimate_emit_cube_count_for_geom(
        prep.dims,
        prep.voxel_size,
        prep.emit_core_min,
        prep.emit_core_max,
    )
}

fn max_vertices_for_tile(mode: SurfaceMode, prep: &SurfaceBuildPrep) -> u32 {
    max_vertices_for_tile_geom(
        mode,
        prep.dims,
        prep.voxel_size,
        prep.emit_core_min,
        prep.emit_core_max,
    )
}

fn max_vertices_for_tile_geom(
    mode: SurfaceMode,
    dims: [u32; 3],
    voxel_size: f32,
    emit_core_min: [f32; 3],
    emit_core_max: [f32; 3],
) -> u32 {
    let per_cube = match mode {
        SurfaceMode::Surface => 6u64,
        SurfaceMode::Mesh => 12u64,
    };
    let cubes =
        estimate_emit_cube_count_for_geom(dims, voxel_size, emit_core_min, emit_core_max).max(1);
    ((cubes * per_cube).min(MAX_VERTICES as u64).max(64)) as u32
}

#[allow(clippy::too_many_arguments)]
fn build_compute_scratch(
    device: &wgpu::Device,
    pending: &PendingDispatch,
    draw: &DrawResources,
    density_compute: &SurfaceDensityCompute,
    vdw_sdf_compute: &SurfaceVdwSdfCompute,
    ses_morph_compute: &SurfaceSesMorphCompute,
    mc_compute: &crate::compute::surface_mc::SurfaceMcCompute,
) -> SurfaceComputeScratch {
    let density_tex = make_storage_3d(
        device,
        "patinae.surface.density",
        wgpu::TextureFormat::Rgba16Float,
        pending.dims,
        wgpu::TextureUsages::empty(),
    );
    let owner_tex = make_storage_3d(
        device,
        "patinae.surface.owner",
        wgpu::TextureFormat::R32Uint,
        pending.dims,
        wgpu::TextureUsages::empty(),
    );
    let density_view = density_tex.create_view(&wgpu::TextureViewDescriptor::default());
    let owner_view = owner_tex.create_view(&wgpu::TextureViewDescriptor::default());
    let (sdf_tex, sdf_view) = if pending.is_ses {
        let tex = make_storage_3d(
            device,
            "patinae.surface.vdw_sdf",
            wgpu::TextureFormat::R32Float,
            pending.dims,
            wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        (Some(tex), Some(view))
    } else {
        (None, None)
    };
    let cell_offsets_buf = make_storage_buffer(
        device,
        "patinae.surface.cell_offsets",
        &pending.accel.offsets,
    );
    let cell_indices_buf = make_storage_buffer(
        device,
        "patinae.surface.cell_indices",
        &pending.accel.indices,
    );

    let mc_inputs = mc_compute.build_inputs(
        device,
        pending.mc_params,
        &density_view,
        &owner_view,
        &draw.vertex_buf,
        &draw.counter_buf,
    );
    let (density_inputs, vdw_sdf_inputs, ses_morph_inputs) = build_producer_inputs(
        device,
        pending.is_ses,
        pending.bbox_min,
        pending.voxel_size,
        pending.dims,
        pending.probe_radius,
        &density_view,
        &owner_view,
        sdf_view.as_ref(),
        &cell_offsets_buf,
        &cell_indices_buf,
        pending.accel.params,
        density_compute,
        vdw_sdf_compute,
        ses_morph_compute,
    );

    SurfaceComputeScratch {
        mc_inputs,
        density_inputs,
        vdw_sdf_inputs,
        ses_morph_inputs,
        _density_tex: density_tex,
        _owner_tex: owner_tex,
        _sdf_tex: sdf_tex,
        _density_view: density_view,
        _owner_view: owner_view,
        _sdf_view: sdf_view,
        _cell_offsets_buf: cell_offsets_buf,
        _cell_indices_buf: cell_indices_buf,
    }
}

impl Representation for SurfaceRep {
    fn kind(&self) -> RepKind {
        self.mode.rep_kind()
    }

    fn build(
        &mut self,
        input: &RenderObjectInput,
        settings: &ResolvedSettings,
        dirty: DirtyFlags,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) {
        let s = input.object_settings.as_ref().unwrap_or(settings);
        // Mode dispatch — Surface reads s.surface.*, Mesh reads s.mesh.*.
        let (transparency, quality, solvent_accessible, solvent_radius) = match self.mode {
            SurfaceMode::Surface => (
                s.surface.transparency,
                s.surface.quality,
                s.surface.solvent,
                s.surface.solvent_radius,
            ),
            SurfaceMode::Mesh => (
                s.mesh.transparency,
                s.mesh.quality,
                s.mesh.solvent,
                s.mesh.solvent_radius,
            ),
        };
        let transparency = transparency.clamp(0.0, 1.0);
        let alpha_mul = 1.0 - transparency;
        let (color_smoothing, color_smoothing_threshold) = match self.mode {
            SurfaceMode::Surface => (
                s.surface.color_smoothing.max(0) as f32,
                s.surface.color_smoothing_threshold.max(0.0),
            ),
            SurfaceMode::Mesh => (0.0, 0.0),
        };

        // Render params struct is identical for Surface (SurfaceParams) and
        // Mesh (MeshParams) in size and alpha offset. Mesh ignores the
        // smoothing slots.
        queue.write_buffer(
            &self.render_params_buffer,
            0,
            bytemuck::bytes_of(&SurfaceParams {
                alpha_mul,
                color_smoothing,
                color_smoothing_threshold,
                _pad2: 0.0,
            }),
        );

        let probe_radius = solvent_radius.max(0.0);
        let requested_voxel_size = voxel_size_for_quality(quality, input.lod);
        let requested_voxel_size = if self.budget_coarsened {
            coarsened_voxel_size(requested_voxel_size)
        } else {
            requested_voxel_size
        };
        let is_ses = solvent_setting_to_is_ses(solvent_accessible);
        let filter_mode = s.surface.mode;
        let individual_chains = s.surface.individual_chains;
        let settings_hash = surface_settings_hash(
            is_ses,
            probe_radius,
            quality,
            filter_mode,
            individual_chains,
            self.mode,
            self.budget_coarsened,
        );
        let reuse_geometry = can_reuse_surface_geometry(
            dirty,
            has_surface_draw_chunks(&self.parts),
            self.last_build,
            settings_hash,
        );
        if reuse_geometry && !dirty.intersects(DirtyFlags::TRANSPARENCY) {
            return;
        }

        // Opacity heuristic. Per-rep transparency override scan only
        // applies in Surface mode (per-atom surface_transparency).
        // Mesh has no per-atom alpha override field (yet).
        let mut opaque = match self.mode {
            SurfaceMode::Surface => alpha_mul >= 0.999,
            SurfaceMode::Mesh => transparency <= 0.0,
        };
        if opaque && self.mode == SurfaceMode::Surface {
            for atom in input.molecule.atoms() {
                if let Some(t) = atom.repr.surface_transparency {
                    if (1.0 - t.clamp(0.0, 1.0)) < 0.999 {
                        opaque = false;
                        break;
                    }
                }
            }
        }
        self.is_opaque_cache = opaque;

        if reuse_geometry {
            return;
        }

        let rep_mask = match self.mode {
            SurfaceMode::Surface => RepMask::SURFACE,
            SurfaceMode::Mesh => RepMask::MESH,
        };
        let partitions = surface_atom_partitions(input, filter_mode, individual_chains, rep_mask);
        let part_preps: Vec<Vec<SurfaceBuildPrep>> = partitions
            .iter()
            .filter_map(|ids| {
                let tiles = prepare_surface_builds_for_atoms(
                    input,
                    ids,
                    probe_radius,
                    requested_voxel_size,
                );
                (!tiles.is_empty()).then_some(tiles)
            })
            .collect();
        if part_preps.is_empty() {
            self.parts.clear();
            self.last_build = None;
            self.needs_dispatch = false;
            return;
        }

        let sig = BuildSignature {
            settings_hash,
            parts_hash: surface_parts_hash(&part_preps),
        };
        if self.last_build == Some(sig) {
            return;
        }
        self.last_build = Some(sig);

        for (part_index, tiles) in part_preps.iter().enumerate() {
            if part_index >= self.parts.len() {
                self.parts.push(SurfacePart { chunks: Vec::new() });
            }

            let part = &mut self.parts[part_index];
            for (tile_index, prep) in tiles.iter().enumerate() {
                let max_vertices = max_vertices_for_tile(self.mode, prep);
                let need_realloc = part
                    .chunks
                    .get(tile_index)
                    .map(|chunk| chunk.draw.max_vertices < max_vertices)
                    .unwrap_or(true);

                let dims = prep.dims;
                let bbox_min = prep.bbox_min;
                let voxel_size = prep.voxel_size;
                let accel = build_surface_accel(prep, probe_radius);

                let (iso, invert_inside) = if is_ses {
                    (0.0_f32, 1_u32)
                } else {
                    (SAS_ISO, 0_u32)
                };
                let mc_params = McParams {
                    bbox_min,
                    voxel_size,
                    dims,
                    iso,
                    max_vertices,
                    invert_inside,
                    emit_lines: self.mode.emit_lines(),
                    _pad1: 0,
                    emit_core_min: prep.emit_core_min,
                    _pad2: 0.0,
                    emit_core_max: prep.emit_core_max,
                    _pad3: 0.0,
                };

                let pending = PendingDispatch {
                    mc_params,
                    bbox_min,
                    voxel_size,
                    dims,
                    probe_radius,
                    is_ses,
                    accel,
                };

                if tile_index >= part.chunks.len() {
                    let draw = allocate_draw_resources(device, DrawAllocArgs { max_vertices });
                    part.chunks.push(SurfaceDrawChunk {
                        draw,
                        pending: Some(pending),
                    });
                } else if need_realloc {
                    let draw = allocate_draw_resources(device, DrawAllocArgs { max_vertices });
                    part.chunks[tile_index] = SurfaceDrawChunk {
                        draw,
                        pending: Some(pending),
                    };
                } else {
                    part.chunks[tile_index].pending = Some(pending);
                }
            }
            part.chunks.truncate(tiles.len());
        }
        self.parts.truncate(part_preps.len());
        self.needs_dispatch = true;
    }

    fn is_opaque(&self) -> bool {
        self.is_opaque_cache
    }

    fn draw_phase(&self) -> DrawPhase {
        self.mode.draw_phase(self.is_opaque_cache)
    }

    fn casts_shadow(&self) -> bool {
        self.mode.casts_shadow(self.is_opaque_cache)
    }

    fn memory_usage(&self) -> GpuMemoryUsage {
        let mut usage = GpuMemoryUsage::default();
        usage.add(buffer_usage(&self.render_params_buffer));
        for part in &self.parts {
            for chunk in &part.chunks {
                usage.add(buffer_usage(&chunk.draw.vertex_buf));
                usage.add(buffer_usage(&chunk.draw.counter_buf));
                usage.add(buffer_usage(&chunk.draw.indirect_buf));
            }
        }
        usage
    }

    fn record_translucent<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        let bg = match self.render_params_bind_group.as_ref() {
            Some(b) => b,
            None => return,
        };
        pass.set_bind_group(3, bg, &[]);
        for part in &self.parts {
            for chunk in &part.chunks {
                let draw = &chunk.draw;
                pass.set_vertex_buffer(0, draw.vertex_buf.slice(..));
                pass.draw_indirect(&draw.indirect_buf, 0);
            }
        }
    }

    fn record_picking<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        for part in &self.parts {
            for chunk in &part.chunks {
                let draw = &chunk.draw;
                pass.set_vertex_buffer(0, draw.vertex_buf.slice(..));
                pass.draw_indirect(&draw.indirect_buf, 0);
            }
        }
    }

    /// Record the producer + MC dispatch chain into the shared encoder.
    /// Pipelines are sourced from `ctx.pipelines`.
    fn record_compute_build(&mut self, ctx: &mut BuildCtx<'_>) -> bool {
        self.ensure_render_bind_group(
            ctx.device,
            &ctx.pipelines.surface_params_layout,
            &ctx.pipelines.mesh_params_layout,
        );
        if !self.needs_dispatch {
            return false;
        }
        let density_compute = &ctx.pipelines.surface_density_compute;
        let vdw_sdf_compute = &ctx.pipelines.surface_vdw_sdf_compute;
        let ses_morph_compute = &ctx.pipelines.surface_ses_morph_compute;
        let mc_compute = &ctx.pipelines.surface_mc_compute;

        let mut dispatched_any = false;
        for part in &mut self.parts {
            for chunk in &mut part.chunks {
                let Some(pending) = chunk.pending.take() else {
                    continue;
                };
                let draw = &chunk.draw;

                mc_compute.reset_counter(ctx.queue, &draw.counter_buf);
                ctx.queue
                    .write_buffer(&draw.indirect_buf, 0, bytemuck::cast_slice(&[0_u32]));

                let scratch = build_compute_scratch(
                    ctx.device,
                    &pending,
                    draw,
                    density_compute,
                    vdw_sdf_compute,
                    ses_morph_compute,
                    mc_compute,
                );
                if pending.is_ses {
                    vdw_sdf_compute.dispatch(
                        ctx.encoder,
                        ctx.scene_bg,
                        ctx.obj_dynamic_offset,
                        scratch.vdw_sdf_inputs.as_ref().unwrap(),
                    );
                    ses_morph_compute
                        .dispatch(ctx.encoder, scratch.ses_morph_inputs.as_ref().unwrap());
                } else {
                    density_compute.dispatch(
                        ctx.encoder,
                        ctx.scene_bg,
                        ctx.obj_dynamic_offset,
                        scratch.density_inputs.as_ref().unwrap(),
                    );
                }
                mc_compute.dispatch(ctx.encoder, &scratch.mc_inputs);
                ctx.encoder
                    .copy_buffer_to_buffer(&draw.counter_buf, 0, &draw.indirect_buf, 0, 4);
                dispatched_any = true;
            }
        }

        self.needs_dispatch = false;
        dispatched_any
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn apply_budget_decision(
        &mut self,
        decision: RepBuildDecision,
        quality: RepQualityLevel,
    ) -> bool {
        let budget_coarsened = matches!(
            (decision, quality),
            (RepBuildDecision::Downgrade, RepQualityLevel::Coarsened)
        );
        let changed = self.budget_coarsened != budget_coarsened;
        self.budget_coarsened = budget_coarsened;
        changed
    }
}

pub(crate) fn budget_estimates(
    input: &RenderObjectInput<'_>,
    settings: &ResolvedSettings,
    mode: SurfaceMode,
) -> Vec<RepMemoryEstimate> {
    let s = input.object_settings.as_ref().unwrap_or(settings);
    let quality = match mode {
        SurfaceMode::Surface => s.surface.quality,
        SurfaceMode::Mesh => s.mesh.quality,
    };
    let requested = voxel_size_for_quality(quality, input.lod);
    let filter_mode = s.surface.mode;
    let individual_chains = s.surface.individual_chains;
    let rep_mask = match mode {
        SurfaceMode::Surface => RepMask::SURFACE,
        SurfaceMode::Mesh => RepMask::MESH,
    };
    let partitions = surface_atom_partitions(input, filter_mode, individual_chains, rep_mask);
    let atom_sets: Vec<_> = partitions
        .iter()
        .filter_map(|ids| collect_surface_atom_set(input, ids))
        .collect();
    let mut estimates = Vec::with_capacity(2);
    estimates.push(surface_estimate(
        &atom_sets,
        s,
        mode,
        requested,
        RepQualityLevel::Full,
    ));
    let coarsened = coarsened_voxel_size(requested);
    if coarsened > requested * 1.001 {
        estimates.push(surface_estimate(
            &atom_sets,
            s,
            mode,
            coarsened,
            RepQualityLevel::Coarsened,
        ));
    }
    estimates
}

fn surface_estimate(
    atom_sets: &[SurfaceAtomSet],
    settings: &ResolvedSettings,
    mode: SurfaceMode,
    requested_voxel_size: f32,
    quality: RepQualityLevel,
) -> RepMemoryEstimate {
    let probe_radius = match mode {
        SurfaceMode::Surface => settings.surface.solvent_radius,
        SurfaceMode::Mesh => settings.mesh.solvent_radius,
    }
    .max(0.0);
    let mut capacity_bytes = SurfaceParams::SIZE;
    let mut scratch_bytes = 0_u64;
    let mut required_bytes = SurfaceParams::SIZE;

    for atom_set in atom_sets {
        for tile in surface_tile_geoms(atom_set, probe_radius, requested_voxel_size) {
            let vertex_bytes = u64::from(max_vertices_for_tile_geom(
                mode,
                tile.dims,
                tile.voxel_size,
                tile.emit_core_min,
                tile.emit_core_max,
            ))
            .saturating_mul(SurfaceVertex::SIZE);
            let chunk_bytes = vertex_bytes.saturating_add(4).saturating_add(16);
            capacity_bytes = capacity_bytes.saturating_add(chunk_bytes);
            scratch_bytes = scratch_bytes.saturating_add(surface_scratch_estimate(tile.dims));
            required_bytes = required_bytes.max(vertex_bytes);
        }
    }

    RepMemoryEstimate {
        required_bytes,
        scratch_bytes,
        capacity_bytes,
        quality,
        can_chunk: false,
        can_skip: true,
    }
}

fn surface_scratch_estimate(dims: [u32; 3]) -> u64 {
    let voxels = dims
        .iter()
        .fold(1_u64, |acc, &dim| acc.saturating_mul(u64::from(dim.max(1))));
    voxels.saturating_mul(12)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voxel_size_quality_curve() {
        assert!((voxel_size_for_quality(0, SceneLod::Auto) - 1.0).abs() < 1e-6);
        assert!((voxel_size_for_quality(1, SceneLod::Auto) - 1.0).abs() < 1e-6);
        assert!((voxel_size_for_quality(2, SceneLod::Auto) - 0.5).abs() < 1e-6);
        assert!((voxel_size_for_quality(3, SceneLod::Auto) - (1.0 / 3.0)).abs() < 1e-6);
        assert!((voxel_size_for_quality(99, SceneLod::Auto) - (1.0 / 6.0)).abs() < 1e-6);
        assert!((voxel_size_for_quality(6, SceneLod::Minimum) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn solvent_setting_follows_pymol_semantics() {
        // PyMOL: surface_solvent=on means solvent-accessible surface (SAS).
        assert!(!solvent_setting_to_is_ses(true));
        // PyMOL default/off means solvent-excluded molecular surface (SES).
        assert!(solvent_setting_to_is_ses(false));
    }

    #[test]
    fn large_grid_increases_voxel_instead_of_clamping_extent() {
        let lo = [0.0, 0.0, 0.0];
        let hi = [420.0, 260.0, 180.0];
        let max_radius = 2.0;
        let probe = 1.4;
        let requested = 0.5;
        let (bbox_min, dims, voxel) = grid_layout_from_bounds(lo, hi, max_radius, probe, requested);

        assert!(dims.iter().all(|&d| d <= MAX_DIM));
        assert!(voxel > requested);
        for k in 0..3 {
            let covered_max = bbox_min[k] + (dims[k] - 1) as f32 * voxel;
            assert!(bbox_min[k] <= lo[k] - max_radius - probe);
            assert!(
                covered_max >= hi[k] + max_radius + probe,
                "axis {k}: covered {covered_max}, expected at least {}",
                hi[k] + max_radius + probe
            );
        }
    }

    #[test]
    fn three_j3q_sized_domain_tiles_to_improve_voxel_quality() {
        let atom_set = SurfaceAtomSet {
            lo: [80.000, 28.094, 100.111],
            hi: [846.690, 1105.052, 897.788],
            max_radius: 2.0,
            atoms: vec![
                SurfaceAccelAtom {
                    local_id: 0,
                    pos: [80.000, 28.094, 100.111],
                },
                SurfaceAccelAtom {
                    local_id: 1,
                    pos: [846.690, 1105.052, 897.788],
                },
            ],
        };
        let probe = 1.4;
        let requested = 1.0;
        let (_, _, monolithic_voxel) = grid_layout_from_bounds(
            atom_set.lo,
            atom_set.hi,
            atom_set.max_radius,
            probe,
            requested,
        );

        let tiles = prepare_surface_builds_from_atom_set(&atom_set, probe, requested);

        assert!(monolithic_voxel > SURFACE_TILE_TRIGGER_VOXEL_SIZE);
        assert!(tiles.len() > 1);
        assert!(tiles.iter().all(|tile| tile.voxel_size <= 1.8));
    }

    #[test]
    fn three_j3q_tiled_surface_uses_per_tile_draw_budget() {
        let atom_set = SurfaceAtomSet {
            lo: [80.000, 28.094, 100.111],
            hi: [846.690, 1105.052, 897.788],
            max_radius: 2.0,
            atoms: vec![
                SurfaceAccelAtom {
                    local_id: 0,
                    pos: [80.000, 28.094, 100.111],
                },
                SurfaceAccelAtom {
                    local_id: 1,
                    pos: [846.690, 1105.052, 897.788],
                },
            ],
        };
        let tiles = prepare_surface_builds_from_atom_set(&atom_set, 1.4, 1.0);
        let capacities: Vec<u32> = tiles
            .iter()
            .map(|tile| max_vertices_for_tile(SurfaceMode::Surface, tile))
            .collect();

        assert!(capacities.len() > 1);
        assert!(capacities.iter().all(|&capacity| capacity <= MAX_VERTICES));
        assert!(
            capacities
                .iter()
                .map(|&capacity| capacity as u64)
                .sum::<u64>()
                > MAX_VERTICES as u64
        );
    }

    #[test]
    fn surface_tile_geoms_match_build_prep_dimensions() {
        let base_atom_set = SurfaceAtomSet {
            lo: [80.000, 28.094, 100.111],
            hi: [846.690, 1105.052, 897.788],
            max_radius: 2.0,
            atoms: vec![
                SurfaceAccelAtom {
                    local_id: 0,
                    pos: [80.000, 28.094, 100.111],
                },
                SurfaceAccelAtom {
                    local_id: 1,
                    pos: [846.690, 1105.052, 897.788],
                },
            ],
        };
        let probe = 1.4;
        let requested = 1.0;
        let base_tiles = surface_tile_geoms(&base_atom_set, probe, requested);

        let mut atoms = base_atom_set.atoms.clone();
        atoms.extend(base_tiles.iter().enumerate().map(|(i, tile)| {
            let pos = [
                (tile.emit_core_min[0] + tile.emit_core_max[0]) * 0.5,
                (tile.emit_core_min[1] + tile.emit_core_max[1]) * 0.5,
                (tile.emit_core_min[2] + tile.emit_core_max[2]) * 0.5,
            ];
            SurfaceAccelAtom {
                local_id: (i + 2) as u32,
                pos,
            }
        }));
        let atom_set = SurfaceAtomSet {
            atoms,
            ..base_atom_set
        };

        let geoms = surface_tile_geoms(&atom_set, probe, requested);
        let preps = prepare_surface_builds_from_atom_set(&atom_set, probe, requested);

        assert_eq!(geoms.len(), preps.len());
        for (geom, prep) in geoms.iter().zip(preps.iter()) {
            assert_eq!(geom.bbox_min, prep.bbox_min);
            assert_eq!(geom.dims, prep.dims);
            assert_eq!(geom.voxel_size, prep.voxel_size);
            assert_eq!(geom.emit_core_min, prep.emit_core_min);
            assert_eq!(geom.emit_core_max, prep.emit_core_max);
        }
    }

    #[test]
    fn seven_kp3_sized_domain_stays_single_grid() {
        let atom_set = SurfaceAtomSet {
            lo: [-137.129, -137.129, -137.129],
            hi: [137.129, 137.129, 137.129],
            max_radius: 2.0,
            atoms: vec![
                SurfaceAccelAtom {
                    local_id: 0,
                    pos: [-137.129, -137.129, -137.129],
                },
                SurfaceAccelAtom {
                    local_id: 1,
                    pos: [137.129, 137.129, 137.129],
                },
            ],
        };
        let tiles = prepare_surface_builds_from_atom_set(&atom_set, 1.4, 1.0);

        assert_eq!(tiles.len(), 1);
        let tile = &tiles[0];
        assert_eq!(tile.emit_core_min, tile.bbox_min);
        assert_eq!(
            tile.emit_core_max,
            grid_domain_max(tile.bbox_min, tile.dims, tile.voxel_size)
        );
    }

    #[test]
    fn seven_kp3_untiled_surface_uses_one_draw_budget() {
        let atom_set = SurfaceAtomSet {
            lo: [-137.129, -137.129, -137.129],
            hi: [137.129, 137.129, 137.129],
            max_radius: 2.0,
            atoms: vec![
                SurfaceAccelAtom {
                    local_id: 0,
                    pos: [-137.129, -137.129, -137.129],
                },
                SurfaceAccelAtom {
                    local_id: 1,
                    pos: [137.129, 137.129, 137.129],
                },
            ],
        };
        let tiles = prepare_surface_builds_from_atom_set(&atom_set, 1.4, 1.0);

        assert_eq!(tiles.len(), 1);
        assert!(max_vertices_for_tile(SurfaceMode::Surface, &tiles[0]) <= MAX_VERTICES);
    }

    #[test]
    fn tile_core_bounds_cover_padded_domain_without_gaps() {
        let lo = [80.000, 28.094, 100.111];
        let hi = [846.690, 1105.052, 897.788];
        let max_radius = 2.0;
        let probe = 1.4;
        let tile_voxel = tiled_surface_voxel_size(max_radius, probe, 1.0);
        let (domain_min, domain_max) =
            surface_domain_from_bounds(lo, hi, max_radius, probe, tile_voxel);

        for axis in 0..3 {
            let ranges = axis_tile_ranges(domain_min[axis], domain_max[axis]);
            assert!((ranges[0].0 - domain_min[axis]).abs() < 1e-5);
            assert!((ranges.last().unwrap().1 - domain_max[axis]).abs() < 1e-5);
            for pair in ranges.windows(2) {
                assert!((pair[0].1 - pair[1].0).abs() < 1e-5);
            }
            assert!(ranges
                .iter()
                .all(|(start, end)| end - start <= SURFACE_TILE_CORE_SPAN + 1e-3));
        }
    }

    #[test]
    fn surface_geometry_fast_path_reuses_on_non_surface_rep_dirty() {
        let sig = BuildSignature {
            settings_hash: 7,
            parts_hash: 11,
        };

        assert!(can_reuse_surface_geometry(
            DirtyFlags::REPS,
            true,
            Some(sig),
            sig.settings_hash
        ));
        assert!(can_reuse_surface_geometry(
            DirtyFlags::REPS | DirtyFlags::VISIBILITY,
            true,
            Some(sig),
            sig.settings_hash
        ));
    }

    #[test]
    fn surface_geometry_fast_path_rebuilds_for_geometry_or_settings_dirty() {
        let sig = BuildSignature {
            settings_hash: 7,
            parts_hash: 11,
        };

        for dirty in [DirtyFlags::COORDS, DirtyFlags::TOPOLOGY, DirtyFlags::LOD] {
            assert!(!can_reuse_surface_geometry(
                dirty,
                true,
                Some(sig),
                sig.settings_hash
            ));
        }
        assert!(!can_reuse_surface_geometry(
            DirtyFlags::REPS,
            true,
            Some(sig),
            sig.settings_hash + 1
        ));
        assert!(!can_reuse_surface_geometry(
            DirtyFlags::REPS,
            false,
            Some(sig),
            sig.settings_hash
        ));
        assert!(!can_reuse_surface_geometry(
            DirtyFlags::REPS,
            true,
            None,
            sig.settings_hash
        ));
    }

    #[test]
    fn surface_accel_bins_preserve_object_local_atom_ids() {
        let prep = SurfaceBuildPrep {
            bbox_min: [0.0, 0.0, 0.0],
            dims: [16, 16, 16],
            voxel_size: 1.0,
            emit_core_min: [0.0, 0.0, 0.0],
            emit_core_max: [15.0, 15.0, 15.0],
            n_kept: 2,
            coord_hash: 0,
            max_radius: 1.5,
            atoms: vec![
                SurfaceAccelAtom {
                    local_id: 7,
                    pos: [1.0, 1.0, 1.0],
                },
                SurfaceAccelAtom {
                    local_id: 11,
                    pos: [12.0, 1.0, 1.0],
                },
            ],
        };
        let accel = build_surface_accel(&prep, 1.4);

        assert_eq!(accel.indices.len(), 2);
        assert!(accel.indices.contains(&7));
        assert!(accel.indices.contains(&11));
        assert_eq!(accel.offsets.first().copied(), Some(0));
        assert_eq!(accel.offsets.last().copied(), Some(2));
    }

    #[test]
    fn surface_mode_filters_atom_classes_and_visibility() {
        let mut protein = Atom::new("CA", patinae_mol::Element::Carbon);
        protein.state.flags = patinae_mol::AtomFlags::PROTEIN | patinae_mol::AtomFlags::POLYMER;
        protein.repr.visible_reps = RepMask::SURFACE;
        let mut hydrogen = Atom::new("H", patinae_mol::Element::Hydrogen);
        hydrogen.repr.visible_reps = RepMask::SURFACE;
        let mut hidden_carbon = Atom::new("C", patinae_mol::Element::Carbon);
        hidden_carbon.repr.visible_reps = RepMask::NONE;

        assert!(atom_matches_surface_mode(
            &protein,
            SettingSurfaceMode::Normal,
            RepMask::SURFACE
        ));
        assert!(!atom_matches_surface_mode(
            &hydrogen,
            SettingSurfaceMode::Normal,
            RepMask::SURFACE
        ));
        assert!(!atom_matches_surface_mode(
            &hydrogen,
            SettingSurfaceMode::Heavy,
            RepMask::SURFACE
        ));
        assert!(atom_matches_surface_mode(
            &protein,
            SettingSurfaceMode::VisibleHeavy,
            RepMask::SURFACE
        ));
        assert!(!atom_matches_surface_mode(
            &hidden_carbon,
            SettingSurfaceMode::Visible,
            RepMask::SURFACE
        ));
    }

    #[test]
    fn surface_part_hash_distinguishes_individual_chain_builds() {
        let one_tile = SurfaceBuildPrep {
            bbox_min: [0.0, 0.0, 0.0],
            dims: [8, 8, 8],
            voxel_size: 1.0,
            emit_core_min: [0.0, 0.0, 0.0],
            emit_core_max: [7.0, 7.0, 7.0],
            n_kept: 2,
            coord_hash: 11,
            max_radius: 1.5,
            atoms: Vec::new(),
        };
        let one_part = vec![vec![one_tile.clone()]];
        let two_parts = vec![
            vec![SurfaceBuildPrep {
                coord_hash: 7,
                n_kept: 1,
                ..one_tile.clone()
            }],
            vec![SurfaceBuildPrep {
                coord_hash: 13,
                n_kept: 1,
                ..one_tile
            }],
        ];

        assert_ne!(
            surface_parts_hash(&one_part),
            surface_parts_hash(&two_parts)
        );
    }

    #[test]
    fn mesh_draw_policy_uses_overlay_without_shadow() {
        assert_eq!(SurfaceMode::Mesh.draw_phase(true), DrawPhase::FastOverlay);
        assert_eq!(SurfaceMode::Mesh.draw_phase(false), DrawPhase::Wboit);
        assert!(!SurfaceMode::Mesh.casts_shadow(true));
        assert!(!SurfaceMode::Mesh.casts_shadow(false));

        assert_eq!(SurfaceMode::Surface.draw_phase(true), DrawPhase::Opaque);
        assert_eq!(SurfaceMode::Surface.draw_phase(false), DrawPhase::Wboit);
        assert!(SurfaceMode::Surface.casts_shadow(true));
        assert!(!SurfaceMode::Surface.casts_shadow(false));
    }
}
