//! Deterministic GPU memory estimates.
//!
//! The renderer cannot query portable driver VRAM usage today, so this module
//! accounts for resources from descriptors and known buffer capacities.

use crate::memory::bytes_to_mib;

/// Number of tracked GPU memory categories.
pub const GPU_MEMORY_CATEGORY_COUNT: usize = 12;

/// High-level owner buckets for renderer GPU allocations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GpuMemoryCategory {
    /// Per-viewport WBOIT and depth targets.
    FrameTargets,
    /// Desktop host textures handed to the UI toolkit.
    ViewportHandoff,
    /// Scene-wide atom, bond, color, mask, marker, and object buffers.
    SceneStore,
    /// Persistent per-representation draw/build buffers.
    Representation,
    /// Persistent representation scratch buffers.
    RepresentationScratch,
    /// Surface-specific persistent scratch or support buffers.
    SurfaceScratch,
    /// Hit-test picking, reprojection, and id-pass resources.
    Picking,
    /// Visual overlay id, mask, silhouette, and marking resources.
    Overlay,
    /// Full-screen postprocess resources.
    Postprocess,
    /// Shadow-map and atlas-occlusion resources.
    Shadow,
    /// GPU-to-CPU staging resources.
    Readback,
    /// Plugin or artifact-owned GPU resources.
    PluginOrArtifact,
}

impl GpuMemoryCategory {
    /// Categories in deterministic reporting order.
    pub const ALL: [Self; GPU_MEMORY_CATEGORY_COUNT] = [
        Self::FrameTargets,
        Self::ViewportHandoff,
        Self::SceneStore,
        Self::Representation,
        Self::RepresentationScratch,
        Self::SurfaceScratch,
        Self::Picking,
        Self::Overlay,
        Self::Postprocess,
        Self::Shadow,
        Self::Readback,
        Self::PluginOrArtifact,
    ];

    /// Short label used in one-line diagnostic output.
    pub const fn label(self) -> &'static str {
        match self {
            Self::FrameTargets => "frame",
            Self::ViewportHandoff => "handoff",
            Self::SceneStore => "scene",
            Self::Representation => "reps",
            Self::RepresentationScratch => "rep_scratch",
            Self::SurfaceScratch => "surface_scratch",
            Self::Picking => "picking",
            Self::Overlay => "overlay",
            Self::Postprocess => "postprocess",
            Self::Shadow => "shadow",
            Self::Readback => "readback",
            Self::PluginOrArtifact => "plugin",
        }
    }

    const fn index(self) -> usize {
        match self {
            Self::FrameTargets => 0,
            Self::ViewportHandoff => 1,
            Self::SceneStore => 2,
            Self::Representation => 3,
            Self::RepresentationScratch => 4,
            Self::SurfaceScratch => 5,
            Self::Picking => 6,
            Self::Overlay => 7,
            Self::Postprocess => 8,
            Self::Shadow => 9,
            Self::Readback => 10,
            Self::PluginOrArtifact => 11,
        }
    }
}

/// Byte counters for one allocation or aggregate bucket.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GpuMemoryUsage {
    /// Bytes currently used by live payloads when known.
    pub live_bytes: u64,
    /// Bytes currently allocated for capacity.
    pub capacity_bytes: u64,
    /// Highest known allocation watermark for this snapshot.
    pub high_water_bytes: u64,
    /// Number of allocations represented by this usage.
    pub allocation_count: u64,
}

impl GpuMemoryUsage {
    /// Creates usage from explicit live and capacity byte counts.
    pub const fn new(
        live_bytes: u64,
        capacity_bytes: u64,
        high_water_bytes: u64,
        allocation_count: u64,
    ) -> Self {
        Self {
            live_bytes,
            capacity_bytes,
            high_water_bytes,
            allocation_count,
        }
    }

    /// Creates usage for a fully occupied allocation.
    pub const fn allocation(bytes: u64) -> Self {
        Self::new(bytes, bytes, bytes, 1)
    }

    /// Creates usage for a capacity-backed allocation.
    pub const fn live_capacity(live_bytes: u64, capacity_bytes: u64) -> Self {
        Self::new(live_bytes, capacity_bytes, capacity_bytes, 1)
    }

    /// Adds another usage with saturating arithmetic.
    pub fn add(&mut self, other: Self) {
        self.live_bytes = self.live_bytes.saturating_add(other.live_bytes);
        self.capacity_bytes = self.capacity_bytes.saturating_add(other.capacity_bytes);
        self.high_water_bytes = self.high_water_bytes.saturating_add(other.high_water_bytes);
        self.allocation_count = self.allocation_count.saturating_add(other.allocation_count);
    }

    /// Returns `true` when no allocation is represented.
    pub const fn is_empty(self) -> bool {
        self.allocation_count == 0 && self.capacity_bytes == 0 && self.live_bytes == 0
    }
}

/// One labeled allocation estimate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpuAllocationEstimate {
    /// Category receiving this estimate.
    pub category: GpuMemoryCategory,
    /// Stable owner label for diagnostics.
    pub label: &'static str,
    /// Byte counters for the allocation.
    pub usage: GpuMemoryUsage,
}

/// Aggregated counters for one category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpuMemoryBucket {
    /// Bucket category.
    pub category: GpuMemoryCategory,
    /// Aggregated byte counters.
    pub usage: GpuMemoryUsage,
}

impl GpuMemoryBucket {
    const fn new(category: GpuMemoryCategory) -> Self {
        Self {
            category,
            usage: GpuMemoryUsage::new(0, 0, 0, 0),
        }
    }
}

/// Deterministic GPU-memory snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuMemorySnapshot {
    buckets: [GpuMemoryBucket; GPU_MEMORY_CATEGORY_COUNT],
}

impl GpuMemorySnapshot {
    /// Creates an empty snapshot with deterministic category order.
    pub fn new() -> Self {
        Self {
            buckets: GpuMemoryCategory::ALL.map(GpuMemoryBucket::new),
        }
    }

    /// Records an aggregate usage into a category.
    pub fn add_usage(&mut self, category: GpuMemoryCategory, usage: GpuMemoryUsage) {
        self.buckets[category.index()].usage.add(usage);
    }

    /// Records one fully occupied allocation.
    pub fn add_allocation(&mut self, category: GpuMemoryCategory, bytes: u64) {
        if bytes == 0 {
            return;
        }
        self.add_usage(category, GpuMemoryUsage::allocation(bytes));
    }

    /// Iterates over buckets in deterministic reporting order.
    pub fn buckets(&self) -> impl Iterator<Item = &GpuMemoryBucket> {
        self.buckets.iter()
    }

    /// Returns usage for one category.
    pub fn category_usage(&self, category: GpuMemoryCategory) -> GpuMemoryUsage {
        self.buckets[category.index()].usage
    }

    /// Returns total estimated allocated capacity.
    pub fn total_capacity_bytes(&self) -> u64 {
        self.buckets
            .iter()
            .map(|bucket| bucket.usage.capacity_bytes)
            .sum()
    }

    /// Returns total known live payload bytes.
    pub fn total_live_bytes(&self) -> u64 {
        self.buckets
            .iter()
            .map(|bucket| bucket.usage.live_bytes)
            .sum()
    }

    /// Formats a one-line `PATINAE_TIMING` diagnostic.
    pub fn timing_line(&self) -> String {
        let mut line = format!(
            "[gpu-mem] total_est={} live_known={}",
            format_mib(self.total_capacity_bytes()),
            format_mib(self.total_live_bytes())
        );
        for bucket in self.buckets() {
            if bucket.usage.capacity_bytes == 0 {
                continue;
            }
            line.push(' ');
            line.push_str(bucket.category.label());
            line.push('=');
            line.push_str(&format_mib(bucket.usage.capacity_bytes));
        }
        line
    }
}

impl Default for GpuMemorySnapshot {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for memory snapshots.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GpuMemoryLedger {
    snapshot: GpuMemorySnapshot,
}

impl GpuMemoryLedger {
    /// Creates an empty ledger.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records an aggregate usage into a category.
    pub fn add_usage(&mut self, category: GpuMemoryCategory, usage: GpuMemoryUsage) {
        self.snapshot.add_usage(category, usage);
    }

    /// Records one fully occupied allocation.
    pub fn add_allocation(&mut self, category: GpuMemoryCategory, bytes: u64) {
        self.snapshot.add_allocation(category, bytes);
    }

    /// Returns the accumulated snapshot.
    pub fn snapshot(self) -> GpuMemorySnapshot {
        self.snapshot
    }
}

/// Estimates bytes for a wgpu texture descriptor.
pub fn estimate_texture_descriptor_bytes(desc: &wgpu::TextureDescriptor<'_>) -> u64 {
    estimate_texture_bytes(
        desc.size,
        desc.dimension,
        desc.format,
        desc.mip_level_count,
        desc.sample_count,
    )
}

/// Estimates texture bytes from descriptor parts.
pub fn estimate_texture_bytes(
    size: wgpu::Extent3d,
    dimension: wgpu::TextureDimension,
    format: wgpu::TextureFormat,
    mip_level_count: u32,
    sample_count: u32,
) -> u64 {
    let mip_count = mip_level_count.max(1);
    let samples = u64::from(sample_count.max(1));
    let bytes_per_block = u64::from(conservative_block_copy_size(format));
    let (block_width, block_height) = format.block_dimensions();
    let mut total = 0_u64;

    for level in 0..mip_count {
        let logical = mip_level_size(size, dimension, level);
        let physical = logical.physical_size(format);
        let width_blocks = u64::from((physical.width / block_width).max(1));
        let height_blocks = u64::from((physical.height / block_height).max(1));
        let depth_or_layers = u64::from(physical.depth_or_array_layers.max(1));
        let level_bytes = width_blocks
            .saturating_mul(height_blocks)
            .saturating_mul(depth_or_layers)
            .saturating_mul(bytes_per_block)
            .saturating_mul(samples);
        total = total.saturating_add(level_bytes);
    }

    total
}

/// Estimates bytes for a single 2D texture allocation.
pub fn estimate_texture_2d_bytes(width: u32, height: u32, format: wgpu::TextureFormat) -> u64 {
    estimate_texture_bytes(
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        wgpu::TextureDimension::D2,
        format,
        1,
        1,
    )
}

/// Creates usage from an allocated wgpu buffer.
pub fn buffer_usage(buffer: &wgpu::Buffer) -> GpuMemoryUsage {
    GpuMemoryUsage::allocation(buffer.size())
}

fn mip_level_size(
    size: wgpu::Extent3d,
    dimension: wgpu::TextureDimension,
    level: u32,
) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: (size.width >> level).max(1),
        height: match dimension {
            wgpu::TextureDimension::D1 => 1,
            _ => (size.height >> level).max(1),
        },
        depth_or_array_layers: match dimension {
            wgpu::TextureDimension::D1 => 1,
            wgpu::TextureDimension::D2 => size.depth_or_array_layers.max(1),
            wgpu::TextureDimension::D3 => (size.depth_or_array_layers >> level).max(1),
        },
    }
}

fn conservative_block_copy_size(format: wgpu::TextureFormat) -> u32 {
    if let Some(bytes) = format.block_copy_size(None) {
        return bytes;
    }
    match format {
        // `Depth24Plus*` is implementation-defined. Use a conservative
        // 32-bit depth estimate, and 64 bits for combined depth/stencil.
        wgpu::TextureFormat::Depth24Plus => 4,
        wgpu::TextureFormat::Depth24PlusStencil8 => 8,
        // Multiplanar formats have plane-specific copy footprints. Treat the
        // full texture as rounded-up bytes per luma pixel.
        wgpu::TextureFormat::NV12 => 2,
        wgpu::TextureFormat::P010 => 4,
        wgpu::TextureFormat::Depth32FloatStencil8 => 8,
        _ => 16,
    }
}

fn format_mib(bytes: u64) -> String {
    format!("{:.1} MiB", bytes_to_mib(bytes))
}

// Rust guideline compliant 2026-02-21

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimates_simple_rgba_texture() {
        let bytes = estimate_texture_2d_bytes(1920, 1080, wgpu::TextureFormat::Rgba8Unorm);
        assert_eq!(bytes, 1920 * 1080 * 4);
    }

    #[test]
    fn estimates_mip_chain_for_3d_texture() {
        let bytes = estimate_texture_bytes(
            wgpu::Extent3d {
                width: 4,
                height: 4,
                depth_or_array_layers: 4,
            },
            wgpu::TextureDimension::D3,
            wgpu::TextureFormat::R8Unorm,
            3,
            1,
        );
        assert_eq!(bytes, 64 + 8 + 1);
    }

    #[test]
    fn estimates_compressed_block_padding() {
        let bytes = estimate_texture_2d_bytes(5, 5, wgpu::TextureFormat::Bc1RgbaUnorm);
        assert_eq!(bytes, 32);
    }

    #[test]
    fn estimates_depth24_plus_stencil_conservatively() {
        let bytes = estimate_texture_2d_bytes(8, 8, wgpu::TextureFormat::Depth24PlusStencil8);
        assert_eq!(bytes, 8 * 8 * 8);
    }

    #[test]
    fn ledger_keeps_deterministic_totals() {
        let mut ledger = GpuMemoryLedger::new();
        ledger.add_allocation(GpuMemoryCategory::Representation, 256);
        ledger.add_usage(
            GpuMemoryCategory::SceneStore,
            GpuMemoryUsage::live_capacity(64, 128),
        );
        let snapshot = ledger.snapshot();

        assert_eq!(snapshot.total_live_bytes(), 320);
        assert_eq!(snapshot.total_capacity_bytes(), 384);
        assert_eq!(
            snapshot
                .buckets()
                .map(|bucket| bucket.category)
                .collect::<Vec<_>>(),
            GpuMemoryCategory::ALL.to_vec()
        );
    }
}
