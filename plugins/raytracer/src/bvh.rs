//! Bounding Volume Hierarchy for ray-scene intersection acceleration
//!
//! Implements a BVH using Surface Area Heuristic (SAH) for optimal splits.

use bytemuck::{Pod, Zeroable};

use crate::error::{RaytraceError, RaytraceResult};
use crate::primitive::Primitives;

/// BVH node for GPU traversal
///
/// Uses a compact representation where internal nodes store child indices
/// and leaf nodes store primitive ranges.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct BvhNode {
    /// Minimum bounds (x, y, z)
    pub min: [f32; 3],
    /// For internal: left child index, for leaf: first primitive index
    pub left_or_first: u32,
    /// Maximum bounds (x, y, z)
    pub max: [f32; 3],
    /// For internal: 0, for leaf: primitive count (>0)
    pub count: u32,
}

impl BvhNode {
    /// Create an empty node
    fn empty() -> Self {
        Self {
            min: [f32::MAX; 3],
            max: [f32::MIN; 3],
            left_or_first: 0,
            count: 0,
        }
    }

    /// Check if this is a leaf node
    pub fn is_leaf(&self) -> bool {
        self.count > 0
    }

    /// Expand bounds to include an AABB
    fn grow_aabb(&mut self, min: [f32; 3], max: [f32; 3]) {
        for i in 0..3 {
            self.min[i] = self.min[i].min(min[i]);
            self.max[i] = self.max[i].max(max[i]);
        }
    }

    /// Surface area of bounds
    fn surface_area(&self) -> f32 {
        let dx = self.max[0] - self.min[0];
        let dy = self.max[1] - self.min[1];
        let dz = self.max[2] - self.min[2];
        2.0 * (dx * dy + dy * dz + dz * dx)
    }
}

/// Primitive reference with cached AABB and centroid
#[derive(Clone, Copy)]
struct PrimitiveRef {
    /// Index into the original primitive arrays (encoded with type)
    /// High 2 bits: type (0=sphere, 1=cylinder, 2=triangle, 3=capsule)
    /// Low 30 bits: index
    pub encoded_index: u32,
    /// Cached centroid for binning
    pub centroid: [f32; 3],
    /// Cached AABB min
    pub aabb_min: [f32; 3],
    /// Cached AABB max
    pub aabb_max: [f32; 3],
}

impl PrimitiveRef {
    fn new(
        prim_type: u32,
        index: u32,
        centroid: [f32; 3],
        aabb_min: [f32; 3],
        aabb_max: [f32; 3],
    ) -> Self {
        Self {
            encoded_index: (prim_type << 30) | (index & 0x3FFFFFFF),
            centroid,
            aabb_min,
            aabb_max,
        }
    }
}

/// Bounding Volume Hierarchy
pub struct Bvh {
    /// Flattened node array for GPU upload
    pub nodes: Vec<BvhNode>,
    /// Reordered primitive indices
    pub primitive_indices: Vec<u32>,
}

impl Bvh {
    /// Build BVH from primitives using SAH
    pub fn build(primitives: &Primitives) -> RaytraceResult<Self> {
        if primitives.is_empty() {
            return Err(RaytraceError::NoPrimitives);
        }

        // Create primitive references
        let mut refs = Vec::with_capacity(primitives.total_count());

        // Add spheres (type 0)
        for (i, sphere) in primitives.spheres.iter().enumerate() {
            let (aabb_min, aabb_max) = sphere.aabb();
            refs.push(PrimitiveRef::new(
                0,
                i as u32,
                sphere.centroid(),
                aabb_min,
                aabb_max,
            ));
        }

        // Add cylinders (type 1)
        for (i, cyl) in primitives.cylinders.iter().enumerate() {
            let (aabb_min, aabb_max) = cyl.aabb();
            refs.push(PrimitiveRef::new(
                1,
                i as u32,
                cyl.centroid(),
                aabb_min,
                aabb_max,
            ));
        }

        // Add triangles (type 2)
        for (i, tri) in primitives.triangles.iter().enumerate() {
            let (aabb_min, aabb_max) = tri.aabb();
            refs.push(PrimitiveRef::new(
                2,
                i as u32,
                tri.centroid(),
                aabb_min,
                aabb_max,
            ));
        }

        // Add capsules (type 3)
        for (i, capsule) in primitives.capsules.iter().enumerate() {
            let (aabb_min, aabb_max) = capsule.aabb();
            refs.push(PrimitiveRef::new(
                3,
                i as u32,
                capsule.centroid(),
                aabb_min,
                aabb_max,
            ));
        }

        // Allocate nodes (at most 2N-1 nodes for N primitives)
        let max_nodes = refs.len() * 2;
        let mut nodes = Vec::with_capacity(max_nodes);
        nodes.push(BvhNode::empty()); // Root node

        // Build recursively
        let mut builder = BvhBuilder {
            refs,
            nodes,
            nodes_used: 1,
        };
        builder.build_recursive(0, 0, builder.refs.len());

        // Extract final primitive order
        let primitive_indices: Vec<u32> = builder.refs.iter().map(|r| r.encoded_index).collect();

        Ok(Self {
            nodes: builder.nodes,
            primitive_indices,
        })
    }

    /// Number of nodes in the BVH
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Decode a primitive index
    ///
    /// Returns (primitive_type, index) where:
    /// - 0 = sphere
    /// - 1 = cylinder
    /// - 2 = triangle
    /// - 3 = capsule
    pub fn decode_index(encoded: u32) -> (u32, u32) {
        let prim_type = encoded >> 30;
        let index = encoded & 0x3FFFFFFF;
        (prim_type, index)
    }
}

/// Internal builder state
struct BvhBuilder {
    refs: Vec<PrimitiveRef>,
    nodes: Vec<BvhNode>,
    nodes_used: usize,
}

impl BvhBuilder {
    /// Recursively build BVH
    fn build_recursive(&mut self, node_idx: usize, start: usize, end: usize) {
        let count = end - start;

        // Compute bounds for this node
        let mut node = BvhNode::empty();
        for i in start..end {
            node.grow_aabb(self.refs[i].aabb_min, self.refs[i].aabb_max);
        }

        // Leaf threshold
        const MAX_LEAF_SIZE: usize = 4;
        if count <= MAX_LEAF_SIZE {
            node.left_or_first = start as u32;
            node.count = count as u32;
            self.nodes[node_idx] = node;
            return;
        }

        // Find best split using SAH
        let (best_axis, best_pos, best_cost) = self.find_best_split(start, end, &node);

        // Cost of not splitting (leaf cost)
        let leaf_cost = count as f32;

        // If splitting isn't beneficial, create leaf
        if best_cost >= leaf_cost {
            node.left_or_first = start as u32;
            node.count = count as u32;
            self.nodes[node_idx] = node;
            return;
        }

        // Partition primitives
        let mid = self.partition(start, end, best_axis, best_pos);

        // Handle degenerate cases
        if mid == start || mid == end {
            // Fall back to median split
            let mid = start + count / 2;
            self.refs[start..end].select_nth_unstable_by(mid - start, |a, b| {
                a.centroid[best_axis]
                    .partial_cmp(&b.centroid[best_axis])
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

            // Allocate child nodes
            let left_idx = self.nodes_used;
            let right_idx = self.nodes_used + 1;
            self.nodes_used += 2;
            self.nodes.resize(self.nodes_used, BvhNode::empty());

            node.left_or_first = left_idx as u32;
            node.count = 0;
            self.nodes[node_idx] = node;

            self.build_recursive(left_idx, start, mid);
            self.build_recursive(right_idx, mid, end);
            return;
        }

        // Allocate child nodes
        let left_idx = self.nodes_used;
        let right_idx = self.nodes_used + 1;
        self.nodes_used += 2;
        self.nodes.resize(self.nodes_used, BvhNode::empty());

        node.left_or_first = left_idx as u32;
        node.count = 0;
        self.nodes[node_idx] = node;

        // Recurse
        self.build_recursive(left_idx, start, mid);
        self.build_recursive(right_idx, mid, end);
    }

    /// Find best split using SAH with binning
    fn find_best_split(&self, start: usize, end: usize, parent: &BvhNode) -> (usize, f32, f32) {
        const NUM_BINS: usize = 12;

        let mut best_axis = 0;
        let mut best_pos = 0.0f32;
        let mut best_cost = f32::MAX;

        let parent_area = parent.surface_area();
        if parent_area <= 0.0 {
            return (0, 0.0, f32::MAX);
        }

        for axis in 0..3 {
            // Find centroid bounds
            let mut min_c = f32::MAX;
            let mut max_c = f32::MIN;
            for i in start..end {
                let c = self.refs[i].centroid[axis];
                min_c = min_c.min(c);
                max_c = max_c.max(c);
            }

            if max_c - min_c < 1e-6 {
                continue; // No variation along this axis
            }

            // Initialize bins
            let mut bins = [(BvhNode::empty(), 0usize); NUM_BINS];
            let scale = NUM_BINS as f32 / (max_c - min_c);

            // Populate bins
            for i in start..end {
                let c = self.refs[i].centroid[axis];
                let bin_idx = ((c - min_c) * scale).min(NUM_BINS as f32 - 1.0) as usize;
                bins[bin_idx]
                    .0
                    .grow_aabb(self.refs[i].aabb_min, self.refs[i].aabb_max);
                bins[bin_idx].1 += 1;
            }

            // Compute prefix costs (left to right)
            let mut left_area = [0.0f32; NUM_BINS - 1];
            let mut left_count = [0usize; NUM_BINS - 1];
            let mut left_box = BvhNode::empty();
            let mut count = 0;
            for i in 0..NUM_BINS - 1 {
                left_box.grow_aabb(bins[i].0.min, bins[i].0.max);
                count += bins[i].1;
                left_area[i] = left_box.surface_area();
                left_count[i] = count;
            }

            // Compute suffix costs (right to left) and find best split
            let mut right_box = BvhNode::empty();
            let mut right_count = 0;
            for i in (0..NUM_BINS - 1).rev() {
                right_box.grow_aabb(bins[i + 1].0.min, bins[i + 1].0.max);
                right_count += bins[i + 1].1;
                let right_area = right_box.surface_area();

                let cost = left_count[i] as f32 * left_area[i] + right_count as f32 * right_area;
                if cost < best_cost {
                    best_cost = cost;
                    best_axis = axis;
                    best_pos = min_c + (i + 1) as f32 * (max_c - min_c) / NUM_BINS as f32;
                }
            }
        }

        // Normalize cost by parent area
        best_cost /= parent_area;
        (best_axis, best_pos, best_cost)
    }

    /// Partition primitives by split plane
    fn partition(&mut self, start: usize, end: usize, axis: usize, pos: f32) -> usize {
        let mut i = start;
        let mut j = end;
        while i < j {
            if self.refs[i].centroid[axis] < pos {
                i += 1;
            } else {
                j -= 1;
                self.refs.swap(i, j);
            }
        }
        i
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitive::GpuSphere;

    #[test]
    fn test_bvh_build_single() {
        let mut prims = Primitives::new();
        prims
            .spheres
            .push(GpuSphere::new([0.0; 3], 1.0, [1.0; 4], 0.0));
        let bvh = Bvh::build(&prims).unwrap();
        assert_eq!(bvh.nodes.len(), 1);
        assert_eq!(bvh.nodes[0].min, [-1.0; 3]);
        assert_eq!(bvh.nodes[0].max, [1.0; 3]);
        assert_eq!(bvh.nodes[0].count, 1);
        assert_eq!(bvh.primitive_indices, [0]);
        assert!(bvh.nodes[0].is_leaf());
    }

    #[test]
    fn test_bvh_build_multiple() {
        let mut prims = Primitives::new();
        for i in 0..100 {
            prims
                .spheres
                .push(GpuSphere::new([i as f32, 0.0, 0.0], 0.5, [1.0; 4], 0.0));
        }
        let bvh = Bvh::build(&prims).unwrap();
        assert!(bvh.nodes.len() > 1);
        assert_eq!(bvh.nodes[0].min, [-0.5; 3]);
        assert_eq!(bvh.nodes[0].max, [99.5, 0.5, 0.5]);
        let mut indices = bvh.primitive_indices.clone();
        indices.sort_unstable();
        assert_eq!(indices, (0..100).collect::<Vec<_>>());
        for node in &bvh.nodes {
            if node.is_leaf() {
                for encoded in &bvh.primitive_indices
                    [node.left_or_first as usize..(node.left_or_first + node.count) as usize]
                {
                    let (kind, index) = Bvh::decode_index(*encoded);
                    assert_eq!(kind, 0);
                    let (min, max) = prims.spheres[index as usize].aabb();
                    for axis in 0..3 {
                        assert!(node.min[axis] <= min[axis]);
                        assert!(node.max[axis] >= max[axis]);
                    }
                }
            }
        }
    }

    #[test]
    fn test_decode_index() {
        let encoded = (1u32 << 30) | 42;
        let (prim_type, index) = Bvh::decode_index(encoded);
        assert_eq!(prim_type, 1);
        assert_eq!(index, 42);
    }
}
