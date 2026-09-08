//! Renderer-owned GPU artifact descriptions.
//!
//! These types describe buffers that already exist inside `RenderState`.
//! They intentionally do not contain ray-tracing concepts; plugin hosts can
//! turn borrowed buffer refs into opaque ABI handles for any trusted GPU
//! plugin that wants to run compute work next to the renderer.
//!
//! A snapshot is a command-scoped view of renderer state after `RenderState`
//! has synchronized its scene store and representation buffers. The snapshot
//! borrows renderer buffers; callers must not retain it across mutable renderer
//! work, device recreation, or plugin command boundaries.
//!
//! # Layout Version
//!
//! `RENDER_ARTIFACT_LAYOUT_VERSION` changes when a plugin-visible role,
//! stride, topology meaning, representation slot, or count-source rule changes.
//! Plugin code should reject unknown versions instead of guessing layouts.
//!
//! # Shared Buffer Layouts
//!
//! `SceneAtoms` uses the 16-byte `AtomGpu` layout: `vdw`, `repr_flags`,
//! `alpha_pack_a`, and `alpha_pack_b` at offsets 0, 4, 8, and 12. Version 3
//! removes the unused trailing 16 bytes from the version 2 atom record.
//!
//! `StdVertices` uses the 24-byte `StdVertex` layout shared by cartoon,
//! ribbon, surface, and mesh-wireframe output: `position` is three `f32` lanes
//! at byte offsets 0, 4, and 8; `normal_oct` is a packed octahedral normal at
//! byte offset 12; `group_id` is at byte offset 16; `flags` is at byte offset
//! 20. Consumers use `atom_offset + group_id` to address per-atom scene data.
//!
//! `SceneColorLut` uses `ColorLutEntry` with stride 64. The first 16 bytes are
//! base RGBA as four `f32` lanes. The next 48 bytes are twelve `u32` slots:
//! sphere, stick, line, dot, cartoon, ribbon, surface, mesh, ellipsoid, and
//! three padding slots. A slot value of `REP_COLOR_INHERIT` means the consumer
//! should use the base RGBA.
//!
//! # Representation Slots
//!
//! Representation color slots are stable in this order: sphere 0, stick 1,
//! line 2, dot 3, cartoon 4, ribbon 5, surface 6, mesh 7, ellipsoid 8.
//! `Other` is never assigned a color-LUT slot by this contract.
//!
//! # Count Sources
//!
//! `element_count` is a direct count only when no count or indirect buffer is
//! attached to a representation. `InstanceCount` buffers store one `u32` count.
//! `IndirectDraw` buffers store one WebGPU draw-indirect record of four `u32`
//! lanes. `max_element_count` remains the allocation capacity for validation
//! and bounds checks even when a GPU-side count source is authoritative.

use crate::picking::RepKind;

/// Layout version for renderer GPU artifacts exposed to plugins.
///
/// Bump this when a plugin-visible artifact role, stride, topology semantic,
/// representation color slot, or count-source rule changes.
pub const RENDER_ARTIFACT_LAYOUT_VERSION: u32 = 3;

/// Semantic role of a renderer-owned GPU buffer.
///
/// Roles describe the buffer layout expected by consumers. The buffer is still
/// owned by `RenderState` and must be surfaced to plugins only as a
/// command-scoped opaque handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RenderArtifactBufferRole {
    FrameUniforms,
    SceneAtoms,
    SceneCoords,
    SceneBonds,
    SceneColorLut,
    SceneMaskLut,
    SceneMarkerLut,
    SceneCsrOffsets,
    SceneCsrIndices,
    SceneObjectTable,
    SphereInstances,
    StickInstances,
    LineInstances,
    StdVertices,
    InstanceCount,
    IndirectDraw,
}

/// Primitive topology represented by a render artifact.
///
/// Instance topologies reference renderer instance formats. `TriangleList` and
/// `LineList` reference `StdVertices`, grouped by threes or twos respectively.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RenderArtifactPrimitiveTopology {
    SphereInstances,
    CylinderInstances,
    LineInstances,
    TriangleList,
    LineList,
}

/// Borrowed renderer GPU buffer plus neutral layout metadata.
///
/// `stride` is the byte distance between logical elements for this role.
/// `element_count` is the backing allocation count for role layouts that have
/// fixed-size elements; draw-time counts may still come from a representation's
/// count or indirect buffer.
#[derive(Debug, Clone, Copy)]
pub struct RenderArtifactBufferRef<'a> {
    pub role: RenderArtifactBufferRole,
    pub buffer: &'a wgpu::Buffer,
    pub size: u64,
    pub stride: u64,
    pub element_count: u64,
}

impl<'a> RenderArtifactBufferRef<'a> {
    pub(crate) fn new(
        role: RenderArtifactBufferRole,
        buffer: &'a wgpu::Buffer,
        stride: u64,
        element_count: u64,
    ) -> Self {
        Self {
            role,
            buffer,
            size: buffer.size(),
            stride,
            element_count,
        }
    }
}

/// One displayed representation backed by renderer GPU artifacts.
///
/// The geometry buffer index points into the same snapshot's `buffers` vector.
/// `atom_offset` and `atom_count` identify the object's range inside shared
/// scene-store buffers such as `SceneColorLut`.
#[derive(Debug, Clone, Copy)]
pub struct RenderArtifactRep {
    pub object_id: u32,
    pub rep_kind: RepKind,
    pub topology: RenderArtifactPrimitiveTopology,
    pub geometry_buffer_index: usize,
    pub count_buffer_index: Option<usize>,
    pub indirect_buffer_index: Option<usize>,
    pub element_count: u64,
    pub max_element_count: u64,
    pub atom_offset: u32,
    pub atom_count: u32,
    pub material_rgba: [f32; 4],
    pub transparency: f32,
}

/// Command-scoped snapshot of renderer GPU artifacts.
///
/// `cull_pass_initialized` indicates whether count and indirect buffers have
/// meaningful culling results for the current renderer state. When it is
/// false, consumers that depend on GPU-side counts should reject the snapshot
/// or use only direct-count representations.
#[derive(Debug)]
pub struct RenderArtifactSnapshot<'a> {
    pub layout_version: u32,
    pub scene_generation: u64,
    pub scene_bounds_min: [f32; 3],
    pub scene_bounds_max: [f32; 3],
    pub cull_pass_initialized: bool,
    pub buffers: Vec<RenderArtifactBufferRef<'a>>,
    pub reps: Vec<RenderArtifactRep>,
}
