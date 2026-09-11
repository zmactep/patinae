//! `SceneStore` — group 2, scene-wide. The single source of truth for
//! per-atom data shared across every representation.
//!
//! One `SceneStore` per `RenderState`. Holds **every** object's atom soup,
//! bond graph, coordinates, and per-atom LUTs in linearly-packed storage
//! buffers; one [`ObjectEntry`] per object indexes those slabs via
//! `atom_offset / bond_offset`. Drawing rebinds via `set_bind_group(2,
//! &scene.bg, &[obj.dynamic_offset])` once per object — N×M
//! bind-group churn collapses to N.
//!
//! Layout invariants:
//!
//! - `atoms`, `coords`, `color_lut`, `mask_lut`, `marker_lut` are all
//!   indexed by **global** atom id
//!   `gid = obj.atom_offset + atom_index`.
//! - `bonds` and `atom_bond_csr` are indexed by global ids the same way
//!   (`obj.bond_offset + bond_index`).
//! - `obj_table` is a dynamic-uniform array; the host advances
//!   `dynamic_offset` per object inside a draw loop. Each entry is padded
//!   to `min_uniform_buffer_offset_alignment` (256 B on every WebGPU
//!   adapter we target).
//! - All per-rep alpha (`sphere_transparency`, etc.) flows through the
//!   per-rep `material_params.alpha_mul` uniform on group 3 — never baked
//!   into `color_lut`. Per-atom per-rep alpha overrides live on
//!   [`AtomGpu`] (`sphere_alpha`, `stick_alpha`) with `f32::NAN` as the
//!   "no override" sentinel.

mod instances;
pub mod layout;
pub mod marker;
pub mod sync;

use std::collections::{HashMap, HashSet};

use bytemuck::{Pod, Zeroable};

use crate::lut_buffer::{growable_capacity_entries, BufferFlushStats, GrowableStorageBuffer};
use crate::memory::GpuMemoryUsage;
use crate::memory_policy::{SceneStoreCompactionPolicy, SceneStoreGrowthPolicy};
use crate::picking::ObjectId;
use crate::render_input::ColorLutEntry;

pub use layout::SceneStoreLayout;

/// Required dynamic-uniform-buffer offset alignment. WebGPU spec guarantees
/// adapters expose at least 256; we hard-code that and assert at runtime if
/// the adapter reports a larger value.
pub const OBJECT_ENTRY_STRIDE: u64 = 256;
const SCENE_STORE_BUFFER_COUNT: usize = 9;

/// Per-atom GPU soup. **16 B**. Mirror of `AtomGpu` in
/// `shaders/common/scene.wgsl`.
///
/// Host-side baking semantics (populated by `sync::write_atoms_for_object`):
///
/// - `vdw` = `atom.effective_vdw() * atom.repr.sphere_scale.unwrap_or(1.0)`.
///   Per-atom `sphere_scale` override is folded in here so changing
///   `settings.sphere.scale` slider only rewrites the per-rep params
///   uniform — not the entire atom buffer. The compute shader applies
///   `params.scale` on top.
/// - `repr_flags` = `atom.repr.visible_reps.0`. These bits are rewritten on
///   both `REPS` and `VISIBILITY`, because surface/mesh/cartoon shaders use
///   them for per-representation visibility without rebuilding geometry.
///   Reserved high bits will carry override-present flags as more reps gain
///   per-atom overrides.
/// - `alpha_pack_a` / `alpha_pack_b` — packed per-atom per-rep alpha
///   overrides, **one byte per rep kind**. Sentinel `0xFF` means
///   "no override → use the per-rep `<rep>_params.alpha_mul` from
///   group 3". Real overrides span `0..=254` mapping to alpha
///   `0.0..=1.0` (precision ≈ 0.4%, perceptually indistinguishable).
///   Layout (LE byte order, byte 0 = LSB):
///   `alpha_pack_a` bytes [sphere | stick | ellipsoid | cartoon]
///   `alpha_pack_b` bytes [surface |   _   |   _    |   _   ]
///   3 spare slots in `alpha_pack_b` for future per-rep alphas without
///   widening the AtomGpu struct.
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct AtomGpu {
    pub vdw: f32,
    pub repr_flags: u32,
    pub alpha_pack_a: u32,
    pub alpha_pack_b: u32,
}

/// Sentinel byte in `AtomGpu::alpha_pack_*`. Means "no override for this
/// rep kind — use the per-rep `alpha_mul` uniform".
pub const ALPHA_NO_OVERRIDE: u8 = 0xFF;

/// Pack a per-atom transparency override (`atom.repr.<rep>_transparency`)
/// into a byte slot. `None` ⇒ `ALPHA_NO_OVERRIDE`. `Some(t)` is converted
/// to alpha `1.0 - t`, scaled to `0..=254` (rounded). Value `255` is
/// reserved for the sentinel; actual alpha `1.0` clamps to `254`
/// (≈0.996, perceptually identical).
#[inline]
pub fn pack_alpha_override(override_transparency: Option<f32>) -> u8 {
    match override_transparency {
        None => ALPHA_NO_OVERRIDE,
        Some(t) => {
            let alpha = (1.0 - t.clamp(0.0, 1.0)).clamp(0.0, 1.0);
            (alpha * 254.0).round().clamp(0.0, 254.0) as u8
        }
    }
}

const _: () = assert!(std::mem::size_of::<AtomGpu>() == 16);

/// Per-bond GPU entry. **32 B**. Mirror of `BondGpu` in
/// `shaders/common/scene.wgsl`.
///
/// `atom1` / `atom2` are **object-local** atom indices. The compute build
/// shader resolves them to global ids via `obj.atom_offset` so per-object
/// bond data stays relocatable. `valence_perp_pad.xyz` is the precomputed
/// local-plane offset direction used by sticks for multiple bonds.
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct BondGpu {
    pub atom1: u32,
    pub atom2: u32,
    pub order: u32,
    pub flags: u32,
    pub valence_perp_pad: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<BondGpu>() == 32);

/// Per-object table entry. Bound as a `<uniform>` with `dynamic_offset`,
/// so each entry is padded to `OBJECT_ENTRY_STRIDE` (256 B). The Pod
/// payload is 96 B; the trailing 160 B are pad.
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct ObjectEntry {
    pub atom_offset: u32,
    pub atom_count: u32,
    pub bond_offset: u32,
    pub bond_count: u32,
    pub object_id: u32,
    /// Copy index plus one; zero identifies an ordinary explicit draw.
    pub flags: u32,
    /// Shared subset mask word offset and a nonzero subset-present flag.
    pub _pad0: [u32; 2],
    pub model_matrix: [[f32; 4]; 4],
}

const _: () = assert!(std::mem::size_of::<ObjectEntry>() == 96);

impl ObjectEntry {
    /// Padded byte size used as `dynamic_offset` step. 256 B per WebGPU.
    pub const STRIDE: u64 = OBJECT_ENTRY_STRIDE;
}

/// Per-object slot bookkeeping. The store keeps one slot per `ObjectId`,
/// allocated linearly the first time the object is seen. Slot reuse on
/// object removal is not implemented yet; reps churning objects
/// would leak buffer space until a future compaction pass.
#[derive(Debug, Clone, Copy)]
pub struct ObjectSlot {
    pub atom_offset: u32,
    pub atom_count: u32,
    pub bond_offset: u32,
    pub bond_count: u32,
    /// Index into the dynamic-uniform table. `dynamic_offset = idx *
    /// ObjectEntry::STRIDE`.
    pub table_index: u32,
}

impl ObjectSlot {
    pub fn dynamic_offset(&self) -> u32 {
        self.table_index * ObjectEntry::STRIDE as u32
    }
}

/// SceneStore buffer category.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum SceneStoreBufferKind {
    /// Packed [`AtomGpu`] entries.
    #[default]
    Atoms,
    /// Per-atom homogeneous coordinates.
    Coords,
    /// Packed [`BondGpu`] entries.
    Bonds,
    /// Per-atom color lookup table.
    ColorLut,
    /// Packed per-atom visibility mask.
    MaskLut,
    /// Per-atom selection and hover marker bits.
    MarkerLut,
    /// CSR atom-to-bond offset table.
    CsrOffsets,
    /// CSR atom-to-bond index table.
    CsrIndices,
    /// Dynamic-uniform object table.
    ObjectTable,
}

impl SceneStoreBufferKind {
    /// Buffer kinds in deterministic diagnostic order.
    pub const ALL: [Self; SCENE_STORE_BUFFER_COUNT] = [
        Self::Atoms,
        Self::Coords,
        Self::Bonds,
        Self::ColorLut,
        Self::MaskLut,
        Self::MarkerLut,
        Self::CsrOffsets,
        Self::CsrIndices,
        Self::ObjectTable,
    ];

    /// Short label used in diagnostics.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Atoms => "atoms",
            Self::Coords => "coords",
            Self::Bonds => "bonds",
            Self::ColorLut => "color_lut",
            Self::MaskLut => "mask_lut",
            Self::MarkerLut => "marker_lut",
            Self::CsrOffsets => "csr_offsets",
            Self::CsrIndices => "csr_indices",
            Self::ObjectTable => "object_table",
        }
    }
}

/// Byte counters for one SceneStore buffer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SceneStoreBufferStats {
    /// Buffer kind.
    pub kind: SceneStoreBufferKind,
    /// Bytes occupied by live object payloads after ideal compaction.
    pub live_bytes: u64,
    /// Logical bytes covered by the current linear watermarks.
    pub allocated_bytes: u64,
    /// Logical bytes reclaimable by compaction.
    pub orphaned_bytes: u64,
    /// GPU capacity bytes currently allocated.
    pub capacity_bytes: u64,
    /// Capacity bytes beyond logical allocation.
    pub capacity_slack_bytes: u64,
}

/// Byte-level SceneStore memory summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneStoreMemoryStats {
    /// Per-buffer statistics.
    pub buffers: [SceneStoreBufferStats; SCENE_STORE_BUFFER_COUNT],
    /// Total bytes occupied by live object payloads.
    pub live_bytes: u64,
    /// Total logical bytes covered by current watermarks.
    pub allocated_bytes: u64,
    /// Total logical bytes reclaimable by compaction.
    pub orphaned_bytes: u64,
    /// Total GPU capacity bytes currently allocated.
    pub capacity_bytes: u64,
    /// Total capacity bytes beyond logical allocation.
    pub capacity_slack_bytes: u64,
    /// Buffer with the largest orphaned byte count.
    pub largest_orphaned_buffer: Option<SceneStoreBufferKind>,
}

impl SceneStoreMemoryStats {
    fn new(buffers: [SceneStoreBufferStats; SCENE_STORE_BUFFER_COUNT]) -> Self {
        let mut stats = Self {
            buffers,
            live_bytes: 0,
            allocated_bytes: 0,
            orphaned_bytes: 0,
            capacity_bytes: 0,
            capacity_slack_bytes: 0,
            largest_orphaned_buffer: None,
        };
        let mut largest_orphaned_bytes = 0;
        for buffer in stats.buffers {
            stats.live_bytes = stats.live_bytes.saturating_add(buffer.live_bytes);
            stats.allocated_bytes = stats.allocated_bytes.saturating_add(buffer.allocated_bytes);
            stats.orphaned_bytes = stats.orphaned_bytes.saturating_add(buffer.orphaned_bytes);
            stats.capacity_bytes = stats.capacity_bytes.saturating_add(buffer.capacity_bytes);
            stats.capacity_slack_bytes = stats
                .capacity_slack_bytes
                .saturating_add(buffer.capacity_slack_bytes);
            if buffer.orphaned_bytes > largest_orphaned_bytes {
                largest_orphaned_bytes = buffer.orphaned_bytes;
                stats.largest_orphaned_buffer = Some(buffer.kind);
            }
        }
        stats
    }

    fn allocation_count(self) -> u64 {
        self.buffers
            .iter()
            .filter(|buffer| buffer.capacity_bytes > 0)
            .count() as u64
    }
}

impl Default for SceneStoreMemoryStats {
    fn default() -> Self {
        Self::new(SceneStoreBufferKind::ALL.map(|kind| SceneStoreBufferStats {
            kind,
            ..Default::default()
        }))
    }
}

/// SceneStore compaction result.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SceneStoreCompactionStats {
    /// Whether compaction ran.
    pub ran: bool,
    /// Number of live object slots moved.
    pub moved_objects: u64,
    /// GPU capacity bytes reclaimed after the following flush.
    pub reclaimed_bytes: u64,
    /// GPU capacity bytes before compaction.
    pub capacity_before_bytes: u64,
    /// GPU capacity bytes after the following flush.
    pub capacity_after_bytes: u64,
    /// Logical orphaned bytes before compaction.
    pub orphaned_before_bytes: u64,
    /// Buffer with the largest orphaned byte count before compaction.
    pub largest_orphaned_buffer: Option<SceneStoreBufferKind>,
}

impl SceneStoreCompactionStats {
    pub(crate) fn finish_after_flush(&mut self, capacity_after_bytes: u64) {
        if !self.ran {
            return;
        }
        self.capacity_after_bytes = capacity_after_bytes;
        self.reclaimed_bytes = self
            .capacity_before_bytes
            .saturating_sub(self.capacity_after_bytes);
    }
}

fn buffer_stats<T>(
    kind: SceneStoreBufferKind,
    live_entries: u64,
    allocated_entries: u64,
    orphaned_entries: u64,
    capacity_entries: usize,
) -> SceneStoreBufferStats {
    let item_size = std::mem::size_of::<T>() as u64;
    let allocated_bytes = allocated_entries.saturating_mul(item_size);
    let capacity_bytes = (capacity_entries as u64).saturating_mul(item_size);
    SceneStoreBufferStats {
        kind,
        live_bytes: live_entries.saturating_mul(item_size),
        allocated_bytes,
        orphaned_bytes: orphaned_entries.saturating_mul(item_size),
        capacity_bytes,
        capacity_slack_bytes: capacity_bytes.saturating_sub(allocated_bytes),
    }
}

fn mask_words_for_atoms(atom_count: u64) -> u64 {
    atom_count.div_ceil(32)
}

fn identity4() -> [[f32; 4]; 4] {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

#[expect(
    clippy::too_many_arguments,
    reason = "SceneStore compaction copies every parallel buffer in one dense pass"
)]
fn copy_compacted_slot(
    old_slot: ObjectSlot,
    new_slot: ObjectSlot,
    old_atoms: &GrowableStorageBuffer<AtomGpu>,
    old_coords: &GrowableStorageBuffer<[f32; 4]>,
    old_color_lut: &GrowableStorageBuffer<ColorLutEntry>,
    old_mask_lut: &GrowableStorageBuffer<u32>,
    old_marker_lut: &GrowableStorageBuffer<u32>,
    old_bonds: &GrowableStorageBuffer<BondGpu>,
    old_csr_offsets: &GrowableStorageBuffer<u32>,
    old_csr_indices: &GrowableStorageBuffer<u32>,
    atoms: &mut [AtomGpu],
    coords: &mut [[f32; 4]],
    color_lut: &mut [ColorLutEntry],
    mask_lut: &mut [u32],
    marker_lut: &mut [u32],
    bonds: &mut [BondGpu],
    csr_offsets: &mut [u32],
    csr_indices: &mut [u32],
) {
    let old_atom = old_slot.atom_offset as usize;
    let new_atom = new_slot.atom_offset as usize;
    let atom_count = old_slot.atom_count as usize;
    let old_bond = old_slot.bond_offset as usize;
    let new_bond = new_slot.bond_offset as usize;
    let bond_count = old_slot.bond_count as usize;

    copy_range(
        old_atoms.cpu(),
        old_atom,
        &mut atoms[new_atom..new_atom + atom_count],
    );
    copy_range(
        old_coords.cpu(),
        old_atom,
        &mut coords[new_atom..new_atom + atom_count],
    );
    copy_range(
        old_color_lut.cpu(),
        old_atom,
        &mut color_lut[new_atom..new_atom + atom_count],
    );
    copy_range(
        old_marker_lut.cpu(),
        old_atom,
        &mut marker_lut[new_atom..new_atom + atom_count],
    );
    copy_range(
        old_bonds.cpu(),
        old_bond,
        &mut bonds[new_bond..new_bond + bond_count],
    );
    copy_mask_bits(
        old_mask_lut.cpu(),
        old_slot.atom_offset,
        mask_lut,
        new_slot.atom_offset,
        old_slot.atom_count,
    );
    copy_csr_for_slot(
        old_csr_offsets.cpu(),
        old_csr_indices.cpu(),
        old_slot,
        csr_offsets,
        csr_indices,
        new_slot,
    );
}

fn copy_range<T: Copy>(source: &[T], source_start: usize, target: &mut [T]) {
    let source_end = source_start.saturating_add(target.len());
    if source_end <= source.len() {
        target.copy_from_slice(&source[source_start..source_end]);
    }
}

fn copy_mask_bits(
    old_mask_lut: &[u32],
    old_atom_offset: u32,
    new_mask_lut: &mut [u32],
    new_atom_offset: u32,
    atom_count: u32,
) {
    for local in 0..atom_count {
        let old_gid = old_atom_offset + local;
        let new_gid = new_atom_offset + local;
        if mask_bit(old_mask_lut, old_gid) {
            set_mask_bit(new_mask_lut, new_gid);
        }
    }
}

fn mask_bit(mask_lut: &[u32], gid: u32) -> bool {
    let word = (gid / 32) as usize;
    let bit = gid & 31;
    mask_lut
        .get(word)
        .is_some_and(|word| (word & (1_u32 << bit)) != 0)
}

fn set_mask_bit(mask_lut: &mut [u32], gid: u32) {
    let word = (gid / 32) as usize;
    let bit = gid & 31;
    if let Some(word) = mask_lut.get_mut(word) {
        *word |= 1_u32 << bit;
    }
}

fn copy_csr_for_slot(
    old_offsets: &[u32],
    old_indices: &[u32],
    old_slot: ObjectSlot,
    new_offsets: &mut [u32],
    new_indices: &mut [u32],
    new_slot: ObjectSlot,
) {
    let mut running = 0_u32;
    let old_atom_base = old_slot.atom_offset as usize;
    let new_atom_base = new_slot.atom_offset as usize;
    let old_bond_base = old_slot.bond_offset;
    let new_bond_base = new_slot.bond_offset;
    for local in 0..old_slot.atom_count as usize {
        let new_offset_index = new_atom_base + local;
        if let Some(offset) = new_offsets.get_mut(new_offset_index) {
            *offset = new_bond_base.saturating_mul(2).saturating_add(running);
        }
        let old_lo = old_offsets.get(old_atom_base + local).copied().unwrap_or(0) as usize;
        let old_hi = old_offsets
            .get(old_atom_base + local + 1)
            .copied()
            .unwrap_or(old_lo as u32) as usize;
        if old_hi < old_lo || old_hi > old_indices.len() {
            continue;
        }
        for old_bond_id in &old_indices[old_lo..old_hi] {
            let local_bond = old_bond_id.saturating_sub(old_bond_base);
            let new_bond_id = new_bond_base.saturating_add(local_bond);
            let write_index = new_bond_base.saturating_mul(2).saturating_add(running) as usize;
            if let Some(target) = new_indices.get_mut(write_index) {
                *target = new_bond_id;
            }
            running = running.saturating_add(1);
        }
    }
    if let Some(offset) = new_offsets.get_mut(new_atom_base + old_slot.atom_count as usize) {
        *offset = new_bond_base.saturating_mul(2).saturating_add(running);
    }
}

fn write_object_entry_bytes(table: &mut [u8], table_index: u32, entry: ObjectEntry) {
    let offset = table_index as usize * ObjectEntry::STRIDE as usize;
    let payload = bytemuck::bytes_of(&entry);
    let end = offset + payload.len();
    if end <= table.len() {
        table[offset..end].copy_from_slice(payload);
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SceneStoreFragmentationStats {
    pub live_atoms: u64,
    pub allocated_atoms: u64,
    pub orphaned_atoms: u64,
    pub live_bonds: u64,
    pub allocated_bonds: u64,
    pub orphaned_bonds: u64,
    pub live_table_slots: u64,
    pub allocated_table_slots: u64,
    pub orphaned_table_slots: u64,
    pub live_bytes: u64,
    pub allocated_bytes: u64,
    pub orphaned_bytes: u64,
    pub capacity_bytes: u64,
    pub capacity_slack_bytes: u64,
    pub largest_orphaned_buffer: Option<SceneStoreBufferKind>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SceneStoreFlushStats {
    pub marker_lut_upload_ranges: u32,
    pub marker_lut_upload_bytes: u64,
    pub marker_lut_reallocated: bool,
    pub fragmentation: SceneStoreFragmentationStats,
    pub compaction: SceneStoreCompactionStats,
}

/// Group-2 backing store. One per `RenderState`.
pub struct SceneStore {
    pub atoms: GrowableStorageBuffer<AtomGpu>,
    pub coords: GrowableStorageBuffer<[f32; 4]>,
    pub bonds: GrowableStorageBuffer<BondGpu>,
    pub color_lut: GrowableStorageBuffer<ColorLutEntry>,
    pub mask_lut: GrowableStorageBuffer<u32>,
    pub marker_lut: GrowableStorageBuffer<u32>,
    /// CSR-style atom→bond adjacency. Two parallel buffers:
    /// - `csr_offsets[gid] .. csr_offsets[gid+1]` — slice into
    ///   `csr_indices` of bond global ids touching the atom.
    /// - `csr_indices[k]` = global bond id.
    pub csr_offsets: GrowableStorageBuffer<u32>,
    pub csr_indices: GrowableStorageBuffer<u32>,
    /// Dynamic-uniform `ObjectEntry` table. Each entry occupies
    /// [`ObjectEntry::STRIDE`] bytes; the payload sits at the start of
    /// each slot, the rest is pad.
    obj_table_cpu: Vec<u8>,
    obj_table_gpu: Option<wgpu::Buffer>,
    obj_table_capacity_bytes: u64,
    obj_table_dirty: Option<(usize, usize)>,

    slots: HashMap<u32, ObjectSlot>,
    geometry_aliases: HashMap<u32, ObjectSlot>,
    instance_tables: HashMap<u32, patinae_mol::instancing::InstanceTable>,
    draw_offsets: HashMap<u32, Vec<u32>>,
    /// Linear allocation watermarks. Slot reclamation is left for a future
    /// compaction pass; object adds/removes are infrequent on the current UI path.
    next_atom_offset: u32,
    next_bond_offset: u32,
    next_table_index: u32,
    /// Ranges made unreachable by replacing an existing `ObjectId` with a
    /// differently-sized object. A future compaction pass may reclaim these
    /// ranges, but draw/pick paths already iterate only entries in `slots`.
    orphaned_atom_count: u64,
    orphaned_bond_count: u64,
    orphaned_table_slots: u64,

    bind_group: Option<wgpu::BindGroup>,
    object_coords_bind_group: Option<wgpu::BindGroup>,
    /// `(atoms_cap, coords_cap, bonds_cap, color_cap, mask_cap, marker_cap,
    ///   csr_off_cap, csr_idx_cap, obj_table_cap)` snapshot from the last
    /// bind-group rebuild. Any mismatch triggers a rebuild.
    bind_capacities: [usize; 9],
    object_coords_bind_capacities: [usize; 2],
}

impl SceneStore {
    pub fn new() -> Self {
        Self {
            atoms: GrowableStorageBuffer::new("patinae.scene_store.atoms"),
            coords: GrowableStorageBuffer::new("patinae.scene_store.coords"),
            bonds: GrowableStorageBuffer::new("patinae.scene_store.bonds"),
            color_lut: GrowableStorageBuffer::new("patinae.scene_store.color_lut"),
            mask_lut: GrowableStorageBuffer::new("patinae.scene_store.mask_lut"),
            marker_lut: GrowableStorageBuffer::new("patinae.scene_store.marker_lut"),
            csr_offsets: GrowableStorageBuffer::new("patinae.scene_store.csr_offsets"),
            csr_indices: GrowableStorageBuffer::new("patinae.scene_store.csr_indices"),
            obj_table_cpu: Vec::new(),
            obj_table_gpu: None,
            obj_table_capacity_bytes: 0,
            obj_table_dirty: None,
            slots: HashMap::new(),
            geometry_aliases: HashMap::new(),
            instance_tables: HashMap::new(),
            draw_offsets: HashMap::new(),
            next_atom_offset: 0,
            next_bond_offset: 0,
            next_table_index: 0,
            orphaned_atom_count: 0,
            orphaned_bond_count: 0,
            orphaned_table_slots: 0,
            bind_group: None,
            object_coords_bind_group: None,
            bind_capacities: [0; 9],
            object_coords_bind_capacities: [0; 2],
        }
    }

    pub fn slot(&self, object_id: ObjectId) -> Option<&ObjectSlot> {
        self.slots
            .get(&object_id.0)
            .or_else(|| self.geometry_aliases.get(&object_id.0))
    }

    pub fn has_slot(&self, object_id: ObjectId) -> bool {
        self.slots.contains_key(&object_id.0)
    }

    pub fn bind_group(&self) -> Option<&wgpu::BindGroup> {
        self.bind_group.as_ref()
    }

    pub fn object_coords_bind_group(&self) -> Option<&wgpu::BindGroup> {
        self.object_coords_bind_group.as_ref()
    }

    pub fn obj_table_buffer(&self) -> Option<&wgpu::Buffer> {
        self.obj_table_gpu.as_ref()
    }

    pub fn marker_lut_buffer(&self) -> Option<&wgpu::Buffer> {
        self.marker_lut.buffer()
    }

    pub fn iter_atom_offsets(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        self.slots
            .iter()
            .map(|(&object_id, slot)| (object_id, slot.atom_offset))
    }

    /// Returns byte-level SceneStore memory statistics.
    pub fn memory_stats(&self) -> SceneStoreMemoryStats {
        let counts = self.fragmentation_counts();
        let live_atoms = counts.live_atoms;
        let allocated_atoms = counts.allocated_atoms;
        let orphaned_atoms = counts.orphaned_atoms;
        let live_bonds = counts.live_bonds;
        let allocated_bonds = counts.allocated_bonds;
        let orphaned_bonds = counts.orphaned_bonds;
        let live_table_slots = counts.live_table_slots;
        let allocated_table_slots = counts.allocated_table_slots;
        let orphaned_table_slots = counts.orphaned_table_slots;

        let source_mask_words = mask_words_for_atoms(allocated_atoms);
        let subset_mask_words =
            (self.mask_lut.cpu().len() as u64).saturating_sub(source_mask_words);
        let mask_live_words = mask_words_for_atoms(live_atoms) + subset_mask_words;
        let mask_allocated_words = source_mask_words + subset_mask_words;
        let csr_live_offsets = if live_atoms == 0 { 0 } else { live_atoms + 1 };
        let csr_allocated_offsets = if allocated_atoms == 0 {
            0
        } else {
            allocated_atoms + 1
        };
        let buffers = [
            buffer_stats::<AtomGpu>(
                SceneStoreBufferKind::Atoms,
                live_atoms,
                allocated_atoms,
                orphaned_atoms,
                self.atoms.capacity_entries(),
            ),
            buffer_stats::<[f32; 4]>(
                SceneStoreBufferKind::Coords,
                live_atoms,
                allocated_atoms,
                orphaned_atoms,
                self.coords.capacity_entries(),
            ),
            buffer_stats::<BondGpu>(
                SceneStoreBufferKind::Bonds,
                live_bonds,
                allocated_bonds,
                orphaned_bonds,
                self.bonds.capacity_entries(),
            ),
            buffer_stats::<ColorLutEntry>(
                SceneStoreBufferKind::ColorLut,
                live_atoms,
                allocated_atoms,
                orphaned_atoms,
                self.color_lut.capacity_entries(),
            ),
            buffer_stats::<u32>(
                SceneStoreBufferKind::MaskLut,
                mask_live_words,
                mask_allocated_words,
                mask_allocated_words.saturating_sub(mask_live_words),
                self.mask_lut.capacity_entries(),
            ),
            buffer_stats::<u32>(
                SceneStoreBufferKind::MarkerLut,
                live_atoms,
                allocated_atoms,
                orphaned_atoms,
                self.marker_lut.capacity_entries(),
            ),
            buffer_stats::<u32>(
                SceneStoreBufferKind::CsrOffsets,
                csr_live_offsets,
                csr_allocated_offsets,
                csr_allocated_offsets.saturating_sub(csr_live_offsets),
                self.csr_offsets.capacity_entries(),
            ),
            buffer_stats::<u32>(
                SceneStoreBufferKind::CsrIndices,
                live_bonds.saturating_mul(2),
                allocated_bonds.saturating_mul(2),
                orphaned_bonds.saturating_mul(2),
                self.csr_indices.capacity_entries(),
            ),
            SceneStoreBufferStats {
                kind: SceneStoreBufferKind::ObjectTable,
                live_bytes: live_table_slots.saturating_mul(ObjectEntry::STRIDE),
                allocated_bytes: allocated_table_slots.saturating_mul(ObjectEntry::STRIDE),
                orphaned_bytes: orphaned_table_slots.saturating_mul(ObjectEntry::STRIDE),
                capacity_bytes: self.obj_table_capacity_bytes,
                capacity_slack_bytes: self
                    .obj_table_capacity_bytes
                    .saturating_sub(allocated_table_slots.saturating_mul(ObjectEntry::STRIDE)),
            },
        ];
        SceneStoreMemoryStats::new(buffers)
    }

    fn fragmentation_counts(&self) -> SceneStoreFragmentationStats {
        let live_atoms = self
            .slots
            .values()
            .map(|slot| u64::from(slot.atom_count))
            .sum();
        let live_bonds = self
            .slots
            .values()
            .map(|slot| u64::from(slot.bond_count))
            .sum();
        SceneStoreFragmentationStats {
            live_atoms,
            allocated_atoms: u64::from(self.next_atom_offset),
            orphaned_atoms: self.orphaned_atom_count,
            live_bonds,
            allocated_bonds: u64::from(self.next_bond_offset),
            orphaned_bonds: self.orphaned_bond_count,
            live_table_slots: self.slots.len() as u64
                + self
                    .instance_tables
                    .values()
                    .map(|table| table.copies.len() as u64)
                    .sum::<u64>(),
            allocated_table_slots: self.obj_table_cpu.len() as u64 / ObjectEntry::STRIDE,
            orphaned_table_slots: self.orphaned_table_slots,
            ..Default::default()
        }
    }

    pub(crate) fn fragmentation_stats(&self) -> SceneStoreFragmentationStats {
        let mut stats = self.fragmentation_counts();
        let memory = self.memory_stats();
        stats.live_bytes = memory.live_bytes;
        stats.allocated_bytes = memory.allocated_bytes;
        stats.orphaned_bytes = memory.orphaned_bytes;
        stats.capacity_bytes = memory.capacity_bytes;
        stats.capacity_slack_bytes = memory.capacity_slack_bytes;
        stats.largest_orphaned_buffer = memory.largest_orphaned_buffer;
        stats
    }

    pub(crate) fn should_compact(&self, policy: SceneStoreCompactionPolicy) -> bool {
        let stats = self.fragmentation_stats();
        policy.should_compact(stats.orphaned_bytes, stats.allocated_bytes)
    }

    pub(crate) fn retain_objects<I>(&mut self, live_object_ids: I) -> bool
    where
        I: IntoIterator<Item = ObjectId>,
    {
        let live: HashSet<ObjectId> = live_object_ids.into_iter().collect();
        let removed: Vec<u32> = self
            .slots
            .keys()
            .copied()
            .filter(|object_id| !live.contains(&ObjectId(*object_id)))
            .collect();
        if removed.is_empty() {
            return false;
        }

        for object_id in removed {
            self.instance_tables.remove(&object_id);
            self.draw_offsets.remove(&object_id);
            if let Some(slot) = self.slots.remove(&object_id) {
                self.record_orphaned_slot(slot);
            }
        }
        true
    }

    /// Estimated GPU bytes allocated by scene-wide storage buffers.
    pub fn memory_usage(&self) -> GpuMemoryUsage {
        let stats = self.memory_stats();
        GpuMemoryUsage::new(
            stats.live_bytes,
            stats.capacity_bytes,
            stats.capacity_bytes,
            stats.allocation_count(),
        )
    }

    /// Allocate (or look up) a slot for `object_id` of the given size.
    ///
    /// If a slot already exists with the same size, return it as-is. If the
    /// size changed (host reloaded the molecule, swapped objects under the
    /// same id, etc.) the old slot is **orphaned** — its `atom_offset` /
    /// `bond_offset` range stays as dead space in the linear buffers — and
    /// a fresh slot is appended at the current tail. The buffer-level
    /// orphan space is acceptable for now: a future compaction pass will
    /// reclaim it (`slots.iter` then sees only live ranges so picking and
    /// draw loops are unaffected).
    pub(crate) fn ensure_slot(
        &mut self,
        object_id: ObjectId,
        atom_count: u32,
        bond_count: u32,
    ) -> ObjectSlot {
        if let Some(existing) = self.slots.get(&object_id.0).copied() {
            if existing.atom_count == atom_count && existing.bond_count == bond_count {
                return existing;
            }
            // Size mismatch: drop the stale slot, fall through to a fresh
            // allocation at the buffer tail. The previous range
            // (`existing.atom_offset .. +existing.atom_count`) is now
            // orphaned. `table_index` is also re-assigned; the old
            // dynamic-uniform slot becomes unused (kept at its last value,
            // harmless since no rep references it anymore).
            self.record_orphaned_slot(existing);
            self.slots.remove(&object_id.0);
        }
        let slot = ObjectSlot {
            atom_offset: self.next_atom_offset,
            atom_count,
            bond_offset: self.next_bond_offset,
            bond_count,
            table_index: self.next_table_index,
        };
        self.next_atom_offset = self
            .next_atom_offset
            .checked_add(atom_count)
            .expect("scene_store: atom offset overflow");
        self.next_bond_offset = self
            .next_bond_offset
            .checked_add(bond_count)
            .expect("scene_store: bond offset overflow");
        self.next_table_index += 1;

        // Grow the per-buffer CPU staging vectors so subsequent writes can
        // index into them without bounds checks.
        let atom_total = self.next_atom_offset as usize;
        let bond_total = self.next_bond_offset as usize;
        self.atoms.resize(atom_total, AtomGpu::zeroed());
        self.coords.resize(atom_total, [0.0; 4]);
        self.color_lut.resize(atom_total, ColorLutEntry::default());
        let n_mask_words = atom_total.div_ceil(32);
        self.mask_lut.resize(n_mask_words, u32::MAX);
        self.marker_lut.resize(atom_total, 0);
        self.bonds.resize(bond_total, BondGpu::zeroed());
        // CSR offsets has one extra entry (sentinel) so `csr[gid+1] -
        // csr[gid]` always works.
        self.csr_offsets.resize(atom_total + 1, 0);
        // CSR indices length is twice the bond count (each bond touches
        // two atoms). Capacity grows; actual size set at write time.
        self.csr_indices.resize(2 * bond_total, 0);

        self.slots.insert(object_id.0, slot);
        self.grow_obj_table(self.next_table_index as usize);
        slot
    }

    fn record_orphaned_slot(&mut self, slot: ObjectSlot) {
        self.orphaned_atom_count = self
            .orphaned_atom_count
            .checked_add(u64::from(slot.atom_count))
            .expect("scene_store: orphaned atom counter overflow");
        self.orphaned_bond_count = self
            .orphaned_bond_count
            .checked_add(u64::from(slot.bond_count))
            .expect("scene_store: orphaned bond counter overflow");
        self.orphaned_table_slots = self
            .orphaned_table_slots
            .checked_add(1)
            .expect("scene_store: orphaned table counter overflow");
    }

    fn grow_obj_table(&mut self, count: usize) {
        let needed = count * ObjectEntry::STRIDE as usize;
        if needed > self.obj_table_cpu.len() {
            let prev = self.obj_table_cpu.len();
            self.obj_table_cpu.resize(needed, 0);
            self.mark_obj_table_dirty(prev, needed);
        }
    }

    pub(crate) fn write_obj_entry(&mut self, table_index: u32, entry: ObjectEntry) {
        let offset = table_index as usize * ObjectEntry::STRIDE as usize;
        let payload = bytemuck::bytes_of(&entry);
        self.obj_table_cpu[offset..offset + payload.len()].copy_from_slice(payload);
        self.mark_obj_table_dirty(offset, offset + ObjectEntry::STRIDE as usize);
    }

    fn mark_obj_table_dirty(&mut self, lo: usize, hi: usize) {
        self.obj_table_dirty = Some(match self.obj_table_dirty {
            Some((a, b)) => (a.min(lo), b.max(hi)),
            None => (lo, hi),
        });
    }

    pub(crate) fn compact(&mut self) -> SceneStoreCompactionStats {
        let before = self.memory_stats();
        if self.slots.is_empty() {
            self.clear_compacted_storage();
            return SceneStoreCompactionStats {
                ran: true,
                capacity_before_bytes: before.capacity_bytes,
                orphaned_before_bytes: before.orphaned_bytes,
                largest_orphaned_buffer: before.largest_orphaned_buffer,
                ..Default::default()
            };
        }

        let mut live_slots: Vec<(u32, ObjectSlot)> = self
            .slots
            .iter()
            .map(|(&object_id, &slot)| (object_id, slot))
            .collect();
        live_slots.sort_by_key(|(_, slot)| slot.table_index);

        let live_atom_count: usize = live_slots
            .iter()
            .map(|(_, slot)| slot.atom_count as usize)
            .sum();
        let live_bond_count: usize = live_slots
            .iter()
            .map(|(_, slot)| slot.bond_count as usize)
            .sum();
        let mut atoms = vec![AtomGpu::zeroed(); live_atom_count];
        let mut coords = vec![[0.0; 4]; live_atom_count];
        let mut color_lut = vec![ColorLutEntry::default(); live_atom_count];
        let mut mask_lut = vec![0_u32; mask_words_for_atoms(live_atom_count as u64) as usize];
        let mut marker_lut = vec![0_u32; live_atom_count];
        let mut bonds = vec![BondGpu::zeroed(); live_bond_count];
        let mut csr_offsets = if live_atom_count == 0 {
            Vec::new()
        } else {
            vec![0_u32; live_atom_count + 1]
        };
        let mut csr_indices = vec![0_u32; live_bond_count.saturating_mul(2)];
        let mut obj_table_cpu = vec![0_u8; live_slots.len() * ObjectEntry::STRIDE as usize];
        let mut new_slots = HashMap::with_capacity(live_slots.len());

        let mut next_atom_offset = 0_u32;
        let mut next_bond_offset = 0_u32;
        let mut moved_objects = 0_u64;
        for (table_index, (object_id, old_slot)) in live_slots.into_iter().enumerate() {
            let new_slot = ObjectSlot {
                atom_offset: next_atom_offset,
                atom_count: old_slot.atom_count,
                bond_offset: next_bond_offset,
                bond_count: old_slot.bond_count,
                table_index: table_index as u32,
            };
            if old_slot.atom_offset != new_slot.atom_offset
                || old_slot.bond_offset != new_slot.bond_offset
                || old_slot.table_index != new_slot.table_index
            {
                moved_objects = moved_objects.saturating_add(1);
            }
            copy_compacted_slot(
                old_slot,
                new_slot,
                &self.atoms,
                &self.coords,
                &self.color_lut,
                &self.mask_lut,
                &self.marker_lut,
                &self.bonds,
                &self.csr_offsets,
                &self.csr_indices,
                &mut atoms,
                &mut coords,
                &mut color_lut,
                &mut mask_lut,
                &mut marker_lut,
                &mut bonds,
                &mut csr_offsets,
                &mut csr_indices,
            );
            let mut entry = self.object_entry(old_slot);
            entry.atom_offset = new_slot.atom_offset;
            entry.atom_count = new_slot.atom_count;
            entry.bond_offset = new_slot.bond_offset;
            entry.bond_count = new_slot.bond_count;
            entry.object_id = object_id;
            write_object_entry_bytes(&mut obj_table_cpu, new_slot.table_index, entry);

            new_slots.insert(object_id, new_slot);
            next_atom_offset = next_atom_offset
                .checked_add(new_slot.atom_count)
                .expect("scene_store: compacted atom offset overflow");
            next_bond_offset = next_bond_offset
                .checked_add(new_slot.bond_count)
                .expect("scene_store: compacted bond offset overflow");
        }

        self.atoms.replace_all_and_discard_gpu(atoms);
        self.coords.replace_all_and_discard_gpu(coords);
        self.color_lut.replace_all_and_discard_gpu(color_lut);
        self.mask_lut.replace_all_and_discard_gpu(mask_lut);
        self.marker_lut.replace_all_and_discard_gpu(marker_lut);
        self.bonds.replace_all_and_discard_gpu(bonds);
        self.csr_offsets.replace_all_and_discard_gpu(csr_offsets);
        self.csr_indices.replace_all_and_discard_gpu(csr_indices);
        self.obj_table_cpu = obj_table_cpu;
        self.obj_table_gpu = None;
        self.obj_table_capacity_bytes = 0;
        self.obj_table_dirty =
            (!self.obj_table_cpu.is_empty()).then_some((0, self.obj_table_cpu.len()));
        self.slots = new_slots;
        self.next_atom_offset = next_atom_offset;
        self.next_bond_offset = next_bond_offset;
        self.next_table_index = self.slots.len() as u32;
        self.orphaned_atom_count = 0;
        self.orphaned_bond_count = 0;
        self.orphaned_table_slots = 0;
        self.discard_bind_groups();

        SceneStoreCompactionStats {
            ran: true,
            moved_objects,
            capacity_before_bytes: before.capacity_bytes,
            orphaned_before_bytes: before.orphaned_bytes,
            largest_orphaned_buffer: before.largest_orphaned_buffer,
            ..Default::default()
        }
    }

    fn clear_compacted_storage(&mut self) {
        self.atoms.replace_all_and_discard_gpu(Vec::new());
        self.coords.replace_all_and_discard_gpu(Vec::new());
        self.color_lut.replace_all_and_discard_gpu(Vec::new());
        self.mask_lut.replace_all_and_discard_gpu(Vec::new());
        self.marker_lut.replace_all_and_discard_gpu(Vec::new());
        self.bonds.replace_all_and_discard_gpu(Vec::new());
        self.csr_offsets.replace_all_and_discard_gpu(Vec::new());
        self.csr_indices.replace_all_and_discard_gpu(Vec::new());
        self.obj_table_cpu.clear();
        self.obj_table_gpu = None;
        self.obj_table_capacity_bytes = 0;
        self.obj_table_dirty = None;
        self.slots.clear();
        self.next_atom_offset = 0;
        self.next_bond_offset = 0;
        self.next_table_index = 0;
        self.orphaned_atom_count = 0;
        self.orphaned_bond_count = 0;
        self.orphaned_table_slots = 0;
        self.discard_bind_groups();
    }

    fn object_entry(&self, slot: ObjectSlot) -> ObjectEntry {
        let offset = slot.table_index as usize * ObjectEntry::STRIDE as usize;
        let end = offset + std::mem::size_of::<ObjectEntry>();
        if end <= self.obj_table_cpu.len() {
            // A byte buffer does not guarantee ObjEntry alignment on WASM.
            return bytemuck::pod_read_unaligned(&self.obj_table_cpu[offset..end]);
        }
        ObjectEntry {
            atom_offset: slot.atom_offset,
            atom_count: slot.atom_count,
            bond_offset: slot.bond_offset,
            bond_count: slot.bond_count,
            object_id: 0,
            flags: 0,
            _pad0: [0; 2],
            model_matrix: identity4(),
        }
    }

    fn discard_bind_groups(&mut self) {
        self.bind_group = None;
        self.object_coords_bind_group = None;
        self.bind_capacities = [0; 9];
        self.object_coords_bind_capacities = [0; 2];
    }

    /// Upload all pending writes and (re)build the bind group. Idempotent
    /// when nothing changed since the last call.
    pub fn flush(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &SceneStoreLayout,
        growth: SceneStoreGrowthPolicy,
    ) -> SceneStoreFlushStats {
        self.rebuild_instance_entries();
        let atoms_grew = self.atoms.ensure_capacity(device, growth);
        let coords_grew = self.coords.ensure_capacity(device, growth);
        let bonds_grew = self.bonds.ensure_capacity(device, growth);
        let color_grew = self.color_lut.ensure_capacity(device, growth);
        let mask_grew = self.mask_lut.ensure_capacity(device, growth);
        let marker_grew = self.marker_lut.ensure_capacity(device, growth);
        let csr_off_grew = self.csr_offsets.ensure_capacity(device, growth);
        let csr_idx_grew = self.csr_indices.ensure_capacity(device, growth);
        let obj_table_grew = self.ensure_obj_table_capacity(device, growth);

        self.atoms.flush(queue);
        self.coords.flush(queue);
        self.bonds.flush(queue);
        self.color_lut.flush(queue);
        self.mask_lut.flush(queue);
        let marker_flush = self.marker_lut.flush(queue);
        self.csr_offsets.flush(queue);
        self.csr_indices.flush(queue);
        if let Some((lo, hi)) = self.obj_table_dirty.take() {
            if let Some(buf) = self.obj_table_gpu.as_ref() {
                queue.write_buffer(buf, lo as u64, &self.obj_table_cpu[lo..hi]);
            }
        }

        let caps = [
            self.atoms.capacity_entries(),
            self.coords.capacity_entries(),
            self.bonds.capacity_entries(),
            self.color_lut.capacity_entries(),
            self.mask_lut.capacity_entries(),
            self.marker_lut.capacity_entries(),
            self.csr_offsets.capacity_entries(),
            self.csr_indices.capacity_entries(),
            self.obj_table_capacity_bytes as usize,
        ];
        let object_coords_caps = [
            self.coords.capacity_entries(),
            self.obj_table_capacity_bytes as usize,
        ];
        let any_grew = atoms_grew
            || coords_grew
            || bonds_grew
            || color_grew
            || mask_grew
            || marker_grew
            || csr_off_grew
            || csr_idx_grew
            || obj_table_grew;
        if self.bind_group.is_none() || any_grew || caps != self.bind_capacities {
            self.rebuild_bind_group(device, layout);
            self.bind_capacities = caps;
        }
        if self.object_coords_bind_group.is_none()
            || coords_grew
            || obj_table_grew
            || object_coords_caps != self.object_coords_bind_capacities
        {
            self.rebuild_object_coords_bind_group(device, layout);
            self.object_coords_bind_capacities = object_coords_caps;
        }

        SceneStoreFlushStats::from_marker_flush(
            marker_flush,
            marker_grew,
            self.fragmentation_stats(),
        )
    }

    fn ensure_obj_table_capacity(
        &mut self,
        device: &wgpu::Device,
        growth: SceneStoreGrowthPolicy,
    ) -> bool {
        let needed = self.obj_table_cpu.len().max(ObjectEntry::STRIDE as usize) as u64;
        if needed <= self.obj_table_capacity_bytes && self.obj_table_gpu.is_some() {
            return false;
        }
        let needed_entries = needed.div_ceil(ObjectEntry::STRIDE) as usize;
        let current_entries = (self.obj_table_capacity_bytes / ObjectEntry::STRIDE) as usize;
        let new_cap_entries = growable_capacity_entries(
            needed_entries,
            current_entries,
            ObjectEntry::STRIDE as usize,
            growth,
        )
        .max(8);
        let new_cap = (new_cap_entries as u64).saturating_mul(ObjectEntry::STRIDE);
        self.obj_table_gpu = Some(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("patinae.scene_store.obj_table"),
            size: new_cap,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
        self.obj_table_capacity_bytes = new_cap;
        // Whole CPU range becomes dirty after realloc.
        if !self.obj_table_cpu.is_empty() {
            self.obj_table_dirty = Some((0, self.obj_table_cpu.len()));
        }
        true
    }

    fn rebuild_bind_group(&mut self, device: &wgpu::Device, layout: &SceneStoreLayout) {
        let (
            Some(atoms),
            Some(coords),
            Some(bonds),
            Some(color),
            Some(mask),
            Some(marker),
            Some(csr_off),
            Some(csr_idx),
            Some(obj_table),
        ) = (
            self.atoms.buffer(),
            self.coords.buffer(),
            self.bonds.buffer(),
            self.color_lut.buffer(),
            self.mask_lut.buffer(),
            self.marker_lut.buffer(),
            self.csr_offsets.buffer(),
            self.csr_indices.buffer(),
            self.obj_table_gpu.as_ref(),
        )
        else {
            return;
        };
        self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("patinae.scene_store.bind_group"),
            layout: &layout.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: obj_table,
                        offset: 0,
                        size: std::num::NonZeroU64::new(std::mem::size_of::<ObjectEntry>() as u64),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: atoms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: coords.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: bonds.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: color.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: mask.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: marker.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: csr_off.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: csr_idx.as_entire_binding(),
                },
            ],
        }));
    }

    fn rebuild_object_coords_bind_group(
        &mut self,
        device: &wgpu::Device,
        layout: &SceneStoreLayout,
    ) {
        let (Some(coords), Some(obj_table)) = (self.coords.buffer(), self.obj_table_gpu.as_ref())
        else {
            return;
        };
        self.object_coords_bind_group =
            Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("patinae.scene_store.object_coords_bind_group"),
                layout: &layout.object_coords_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: obj_table,
                            offset: 0,
                            size: std::num::NonZeroU64::new(
                                std::mem::size_of::<ObjectEntry>() as u64
                            ),
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: coords.as_entire_binding(),
                    },
                ],
            }));
    }
}

impl SceneStoreFlushStats {
    fn from_marker_flush(
        marker: BufferFlushStats,
        marker_lut_reallocated: bool,
        fragmentation: SceneStoreFragmentationStats,
    ) -> Self {
        Self {
            marker_lut_upload_ranges: marker.ranges,
            marker_lut_upload_bytes: marker.bytes,
            marker_lut_reallocated,
            fragmentation,
            compaction: SceneStoreCompactionStats::default(),
        }
    }
}

impl Default for SceneStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer(stats: &SceneStoreMemoryStats, kind: SceneStoreBufferKind) -> SceneStoreBufferStats {
        stats
            .buffers
            .iter()
            .copied()
            .find(|buffer| buffer.kind == kind)
            .expect("buffer stats")
    }

    #[test]
    fn resizing_same_object_id_records_orphaned_slots() {
        let mut store = SceneStore::new();
        let object = ObjectId(7);

        let first = store.ensure_slot(object, 3, 2);
        assert_eq!(first.atom_offset, 0);
        assert_eq!(first.bond_offset, 0);

        let second = store.ensure_slot(object, 5, 4);
        assert_eq!(second.atom_offset, 3);
        assert_eq!(second.bond_offset, 2);

        let stats = store.fragmentation_stats();
        assert_eq!(stats.live_atoms, 5);
        assert_eq!(stats.allocated_atoms, 8);
        assert_eq!(stats.orphaned_atoms, 3);
        assert_eq!(stats.live_bonds, 4);
        assert_eq!(stats.allocated_bonds, 6);
        assert_eq!(stats.orphaned_bonds, 2);
        assert_eq!(stats.live_table_slots, 1);
        assert_eq!(stats.allocated_table_slots, 2);
        assert_eq!(stats.orphaned_table_slots, 1);
    }

    #[test]
    fn byte_accounting_reports_live_allocated_and_orphaned_bytes() {
        let mut store = SceneStore::new();
        store.ensure_slot(ObjectId(1), 3, 2);
        store.ensure_slot(ObjectId(2), 5, 4);
        store.ensure_slot(ObjectId(1), 7, 1);

        let stats = store.memory_stats();
        let atoms = buffer(&stats, SceneStoreBufferKind::Atoms);
        let bonds = buffer(&stats, SceneStoreBufferKind::Bonds);
        let object_table = buffer(&stats, SceneStoreBufferKind::ObjectTable);

        assert_eq!(atoms.live_bytes, 12 * std::mem::size_of::<AtomGpu>() as u64);
        assert_eq!(
            atoms.allocated_bytes,
            15 * std::mem::size_of::<AtomGpu>() as u64
        );
        assert_eq!(
            atoms.orphaned_bytes,
            3 * std::mem::size_of::<AtomGpu>() as u64
        );
        assert_eq!(bonds.live_bytes, 5 * std::mem::size_of::<BondGpu>() as u64);
        assert_eq!(
            bonds.orphaned_bytes,
            2 * std::mem::size_of::<BondGpu>() as u64
        );
        assert_eq!(object_table.live_bytes, 2 * ObjectEntry::STRIDE);
        assert_eq!(object_table.orphaned_bytes, ObjectEntry::STRIDE);
        assert_eq!(stats.capacity_bytes, 0);
        assert_eq!(
            stats.largest_orphaned_buffer,
            Some(SceneStoreBufferKind::ObjectTable)
        );
    }

    #[test]
    fn retain_objects_records_removed_slots_as_orphaned() {
        let mut store = SceneStore::new();
        store.ensure_slot(ObjectId(1), 3, 2);
        store.ensure_slot(ObjectId(2), 5, 4);

        let removed = store.retain_objects([ObjectId(2)]);

        assert!(removed);
        assert!(store.slot(ObjectId(1)).is_none());
        assert!(store.slot(ObjectId(2)).is_some());
        let stats = store.fragmentation_stats();
        assert_eq!(stats.live_atoms, 5);
        assert_eq!(stats.allocated_atoms, 8);
        assert_eq!(stats.orphaned_atoms, 3);
        assert_eq!(stats.live_bonds, 4);
        assert_eq!(stats.allocated_bonds, 6);
        assert_eq!(stats.orphaned_bonds, 2);
        assert_eq!(stats.live_table_slots, 1);
        assert_eq!(stats.allocated_table_slots, 2);
        assert_eq!(stats.orphaned_table_slots, 1);
    }

    #[test]
    fn compact_rewrites_dense_slots_mask_bits_and_csr_bond_ids() {
        let mut store = SceneStore::new();
        store.ensure_slot(ObjectId(1), 5, 2);
        let old_live_slot = store.ensure_slot(ObjectId(2), 40, 3);
        let old_mask_word = (old_live_slot.atom_offset / 32) as usize;
        let old_mask_bit = old_live_slot.atom_offset & 31;
        store.mask_lut.set(old_mask_word, 1_u32 << old_mask_bit);
        let old_atom_base = old_live_slot.atom_offset as usize;
        let old_csr_base = old_live_slot.bond_offset * 2;
        store.csr_offsets.set(old_atom_base, old_csr_base);
        store.csr_offsets.set(old_atom_base + 1, old_csr_base + 1);
        store
            .csr_indices
            .set(old_csr_base as usize, old_live_slot.bond_offset);

        store.retain_objects([ObjectId(2)]);
        let compaction = store.compact();

        assert!(compaction.ran);
        assert_eq!(compaction.moved_objects, 1);
        let slot = *store.slot(ObjectId(2)).expect("live slot");
        assert_eq!(slot.atom_offset, 0);
        assert_eq!(slot.bond_offset, 0);
        assert_eq!(slot.table_index, 0);
        assert!(mask_bit(store.mask_lut.cpu(), 0));
        assert!(!mask_bit(store.mask_lut.cpu(), old_live_slot.atom_offset));
        assert_eq!(store.csr_offsets.cpu()[0], 0);
        assert_eq!(store.csr_offsets.cpu()[1], 1);
        assert_eq!(store.csr_indices.cpu()[0], 0);
        let entry = store.object_entry(slot);
        assert_eq!(entry.atom_offset, 0);
        assert_eq!(entry.bond_offset, 0);
        assert_eq!(entry.object_id, 2);

        let stats = store.fragmentation_stats();
        assert_eq!(stats.live_atoms, 40);
        assert_eq!(stats.allocated_atoms, 40);
        assert_eq!(stats.orphaned_atoms, 0);
        assert_eq!(stats.live_bonds, 3);
        assert_eq!(stats.allocated_bonds, 3);
        assert_eq!(stats.orphaned_bonds, 0);
        assert_eq!(stats.live_table_slots, 1);
        assert_eq!(stats.allocated_table_slots, 1);
        assert_eq!(stats.orphaned_table_slots, 0);
    }
}
