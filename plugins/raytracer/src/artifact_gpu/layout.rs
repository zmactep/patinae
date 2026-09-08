use bytemuck::{Pod, Zeroable};
use patinae_scene::GpuHandle;

use crate::gpu::RaytraceParams;

pub(super) const WORKGROUP_SIZE: u32 = 128;
pub(super) const BVH_LEAF_SIZE: u32 = 4;
pub(super) const EMPTY_STORAGE_BYTES: u64 = 16;
pub(super) const COLOR_LUT_STRIDE: u64 = 64;
pub(super) const ATOM_STRIDE: u64 = 16;
pub(super) const SPHERE_INSTANCE_STRIDE: u64 = 32;
pub(super) const STICK_INSTANCE_STRIDE: u64 = 48;
pub(super) const LINE_INSTANCE_STRIDE: u64 = 48;
pub(super) const STD_VERTEX_STRIDE: u64 = 24;
pub(super) const MAX_OUTPUT_READBACK_BYTES: u64 = patinae_render::mib_to_bytes(64);
pub(super) const MAX_ENCODED_PRIMITIVE_INDEX: u32 = 0x3fff_ffff;
pub(super) const RAY_LINE_RADIUS: f32 = 0.035;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct ArtifactTriangle {
    v0: [f32; 4],
    v1: [f32; 4],
    v2: [f32; 4],
    color: [f32; 4],
    normal_oct: [u32; 4],
}

const _: () = assert!(std::mem::size_of::<ArtifactTriangle>() == 80);

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct ArtifactTriangleParams {
    pub(super) vertex_capacity: u32,
    pub(super) triangle_offset: u32,
    pub(super) source_triangle_start: u32,
    pub(super) output_triangle_count: u32,
    pub(super) atom_offset: u32,
    pub(super) rep_slot: u32,
    pub(super) transparency: f32,
    pub(super) dispatch_width: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct ArtifactVisibleTriangleParams {
    pub(super) view_matrix: [[f32; 4]; 4],
    pub(super) proj_matrix: [[f32; 4]; 4],
    pub(super) source_vertex_count: u32,
    pub(super) source_triangle_start: u32,
    pub(super) source_triangle_count: u32,
    pub(super) triangle_offset: u32,
    pub(super) output_triangle_capacity: u32,
    pub(super) atom_offset: u32,
    pub(super) rep_slot: u32,
    pub(super) transparency: f32,
    pub(super) dispatch_width: u32,
    pub(super) counter_index: u32,
    pub(super) _pad0: u32,
    pub(super) _pad1: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct ArtifactSphereParams {
    pub(super) instance_capacity: u32,
    pub(super) sphere_offset: u32,
    pub(super) atom_offset: u32,
    pub(super) rep_slot: u32,
    pub(super) transparency: f32,
    pub(super) dispatch_width: u32,
    pub(super) _pad0: u32,
    pub(super) _pad1: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct ArtifactCylinderParams {
    pub(super) instance_capacity: u32,
    pub(super) cylinder_offset: u32,
    pub(super) atom_offset: u32,
    pub(super) rep_slot: u32,
    pub(super) transparency: f32,
    pub(super) radius: f32,
    pub(super) dispatch_width: u32,
    pub(super) _pad0: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct ArtifactCapsuleParams {
    pub(super) instance_capacity: u32,
    pub(super) capsule_offset: u32,
    pub(super) atom_offset: u32,
    pub(super) rep_slot: u32,
    pub(super) transparency: f32,
    pub(super) dispatch_width: u32,
    pub(super) _pad0: u32,
    pub(super) _pad1: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct ArtifactBvhParams {
    pub(super) leaf_slots: u32,
    pub(super) leaf_start: u32,
    pub(super) level_start: u32,
    pub(super) level_count: u32,
    pub(super) dispatch_width: u32,
    pub(super) _pad0: u32,
    pub(super) _pad1: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct ArtifactPrimitiveMetadata {
    pub(super) sphere_count: u32,
    pub(super) cylinder_count: u32,
    pub(super) capsule_count: u32,
    pub(super) triangle_count: u32,
    pub(super) primitive_count: u32,
    pub(super) triangle_capacity: u32,
    pub(super) visible_triangle_count: u32,
    pub(super) overflow: u32,
}

const _: () = assert!(std::mem::size_of::<ArtifactPrimitiveMetadata>() == 32);

impl ArtifactPrimitiveMetadata {
    pub(super) fn from_counts(counts: PrimitiveCounts) -> Result<Self, String> {
        let primitive_count = counts.primitive_count()?;
        Ok(Self {
            sphere_count: counts.spheres,
            cylinder_count: counts.cylinders,
            capsule_count: counts.capsules,
            triangle_count: counts.triangles,
            primitive_count,
            triangle_capacity: counts.triangles,
            visible_triangle_count: 0,
            overflow: 0,
        })
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct FinalizeStreamingMetadataParams {
    pub(super) direct_triangle_count: u32,
    pub(super) visible_triangle_capacity: u32,
    pub(super) counter_index: u32,
    pub(super) bvh_leaf_size: u32,
    pub(super) max_workgroups_per_dimension: u32,
    pub(super) _pad0: u32,
    pub(super) _pad1: u32,
    pub(super) _pad2: u32,
}

const _: () = assert!(std::mem::size_of::<FinalizeStreamingMetadataParams>() == 32);

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct DownsampleParams {
    pub(super) src_width: u32,
    pub(super) dst_width: u32,
    pub(super) dst_height: u32,
    pub(super) factor: u32,
}

#[derive(Clone, Copy)]
pub(super) struct PrimitiveBuffers {
    pub(super) spheres: GpuHandle,
    pub(super) cylinders: GpuHandle,
    pub(super) capsules: GpuHandle,
    pub(super) triangles: GpuHandle,
}

#[derive(Clone, Copy)]
pub(super) struct PrimitiveCounts {
    pub(super) spheres: u32,
    pub(super) cylinders: u32,
    pub(super) capsules: u32,
    pub(super) triangles: u32,
}

impl PrimitiveCounts {
    pub(super) fn primitive_count(self) -> Result<u32, String> {
        self.spheres
            .checked_add(self.cylinders)
            .and_then(|count| count.checked_add(self.capsules))
            .and_then(|count| count.checked_add(self.triangles))
            .ok_or_else(|| "artifact primitive count overflow".to_string())
    }
}

#[derive(Clone, Copy)]
pub(super) struct BvhBuffers {
    pub(super) nodes: GpuHandle,
    pub(super) indices: GpuHandle,
}

pub(super) struct BvhBuildInput<'a> {
    pub(super) primitives: PrimitiveBuffers,
    pub(super) bvh: BvhBuffers,
    pub(super) metadata: GpuHandle,
    pub(super) leaf_dispatch_args: Option<GpuHandle>,
    pub(super) shape: &'a BvhShape,
    pub(super) max_dispatch_dimension: u32,
}

pub(super) struct RaytraceDispatchInput<'a> {
    pub(super) params: &'a RaytraceParams,
    pub(super) primitives: PrimitiveBuffers,
    pub(super) bvh: BvhBuffers,
    pub(super) counts: PrimitiveCounts,
    pub(super) metadata: GpuHandle,
    pub(super) bvh_node_count: u32,
    pub(super) max_dispatch_dimension: u32,
}

pub(super) struct DownsampleInput {
    pub(super) source_buffer: GpuHandle,
    pub(super) src_width: u32,
    pub(super) dst_width: u32,
    pub(super) dst_height: u32,
    pub(super) factor: u32,
    pub(super) max_dispatch_dimension: u32,
}

pub(super) struct BvhShape {
    pub(super) leaf_slots: u32,
    pub(super) leaf_start: u32,
    pub(super) node_count: u32,
}
