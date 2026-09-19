//! Primitive types for raytracing
//!
//! GPU-friendly representations of geometric primitives used in molecular visualization.

use bytemuck::{Pod, Zeroable};

/// GPU-friendly sphere primitive
///
/// Represents an atom as a sphere with position, radius, color, and transparency.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct GpuSphere {
    /// Center position (x, y, z)
    pub center: [f32; 3],
    /// Sphere radius
    pub radius: f32,
    /// RGBA color
    pub color: [f32; 4],
    /// Transparency (0 = opaque, 1 = fully transparent)
    pub transparency: f32,
    /// Padding for alignment
    pub _padding: [f32; 3],
}

impl GpuSphere {
    /// Create a new sphere primitive
    pub fn new(center: [f32; 3], radius: f32, color: [f32; 4], transparency: f32) -> Self {
        Self {
            center,
            radius,
            color,
            transparency,
            _padding: [0.0; 3],
        }
    }

    /// Check if sphere is transparent
    pub fn is_transparent(&self) -> bool {
        self.transparency > 0.001 || self.color[3] < 0.999
    }

    /// Get axis-aligned bounding box
    pub fn aabb(&self) -> ([f32; 3], [f32; 3]) {
        let min = [
            self.center[0] - self.radius,
            self.center[1] - self.radius,
            self.center[2] - self.radius,
        ];
        let max = [
            self.center[0] + self.radius,
            self.center[1] + self.radius,
            self.center[2] + self.radius,
        ];
        (min, max)
    }

    /// Get centroid (same as center for spheres)
    pub fn centroid(&self) -> [f32; 3] {
        self.center
    }
}

/// GPU-friendly cylinder primitive
///
/// Represents a bond as a cylinder with two endpoints, radius, and colors.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct GpuCylinder {
    /// Start position (x, y, z)
    pub start: [f32; 3],
    /// Cylinder radius
    pub radius: f32,
    /// End position (x, y, z)
    pub end: [f32; 3],
    /// Padding for alignment
    pub _pad0: f32,
    /// RGBA color at start
    pub color1: [f32; 4],
    /// RGBA color at end
    pub color2: [f32; 4],
    /// Transparency (0 = opaque, 1 = fully transparent)
    pub transparency: f32,
    /// Padding for alignment
    pub _padding: [f32; 3],
}

impl GpuCylinder {
    /// Create a new cylinder primitive
    pub fn new(
        start: [f32; 3],
        end: [f32; 3],
        radius: f32,
        color1: [f32; 4],
        color2: [f32; 4],
        transparency: f32,
    ) -> Self {
        Self {
            start,
            radius,
            end,
            _pad0: 0.0,
            color1,
            color2,
            transparency,
            _padding: [0.0; 3],
        }
    }

    /// Check if cylinder is transparent
    pub fn is_transparent(&self) -> bool {
        self.transparency > 0.001 || self.color1[3] < 0.999 || self.color2[3] < 0.999
    }

    /// Get axis-aligned bounding box
    pub fn aabb(&self) -> ([f32; 3], [f32; 3]) {
        let min = [
            self.start[0].min(self.end[0]) - self.radius,
            self.start[1].min(self.end[1]) - self.radius,
            self.start[2].min(self.end[2]) - self.radius,
        ];
        let max = [
            self.start[0].max(self.end[0]) + self.radius,
            self.start[1].max(self.end[1]) + self.radius,
            self.start[2].max(self.end[2]) + self.radius,
        ];
        (min, max)
    }

    /// Get centroid
    pub fn centroid(&self) -> [f32; 3] {
        [
            (self.start[0] + self.end[0]) * 0.5,
            (self.start[1] + self.end[1]) * 0.5,
            (self.start[2] + self.end[2]) * 0.5,
        ]
    }
}

/// GPU-friendly capsule primitive.
///
/// Represents a rounded stick with cylindrical body and spherical end caps.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct GpuCapsule {
    /// Start position (x, y, z)
    pub start: [f32; 3],
    /// Capsule radius
    pub radius: f32,
    /// End position (x, y, z)
    pub end: [f32; 3],
    /// Padding for alignment
    pub _pad0: f32,
    /// RGBA color at start
    pub color1: [f32; 4],
    /// RGBA color at end
    pub color2: [f32; 4],
    /// Transparency (0 = opaque, 1 = fully transparent)
    pub transparency: f32,
    /// Padding for alignment
    pub _padding: [f32; 3],
}

impl GpuCapsule {
    /// Create a new capsule primitive.
    pub fn new(
        start: [f32; 3],
        end: [f32; 3],
        radius: f32,
        color1: [f32; 4],
        color2: [f32; 4],
        transparency: f32,
    ) -> Self {
        Self {
            start,
            radius,
            end,
            _pad0: 0.0,
            color1,
            color2,
            transparency,
            _padding: [0.0; 3],
        }
    }

    /// Check if capsule is transparent.
    pub fn is_transparent(&self) -> bool {
        self.transparency > 0.001 || self.color1[3] < 0.999 || self.color2[3] < 0.999
    }

    /// Get axis-aligned bounding box.
    pub fn aabb(&self) -> ([f32; 3], [f32; 3]) {
        let min = [
            self.start[0].min(self.end[0]) - self.radius,
            self.start[1].min(self.end[1]) - self.radius,
            self.start[2].min(self.end[2]) - self.radius,
        ];
        let max = [
            self.start[0].max(self.end[0]) + self.radius,
            self.start[1].max(self.end[1]) + self.radius,
            self.start[2].max(self.end[2]) + self.radius,
        ];
        (min, max)
    }

    /// Get centroid.
    pub fn centroid(&self) -> [f32; 3] {
        [
            (self.start[0] + self.end[0]) * 0.5,
            (self.start[1] + self.end[1]) * 0.5,
            (self.start[2] + self.end[2]) * 0.5,
        ]
    }
}

/// GPU-friendly triangle primitive
///
/// Represents a surface triangle with vertices, normals, and color.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct GpuTriangle {
    /// Vertex 0 position
    pub v0: [f32; 3],
    pub _pad0: f32,
    /// Vertex 1 position
    pub v1: [f32; 3],
    pub _pad1: f32,
    /// Vertex 2 position
    pub v2: [f32; 3],
    pub _pad2: f32,
    /// Normal at vertex 0
    pub n0: [f32; 3],
    pub _pad3: f32,
    /// Normal at vertex 1
    pub n1: [f32; 3],
    pub _pad4: f32,
    /// Normal at vertex 2
    pub n2: [f32; 3],
    pub _pad5: f32,
    /// RGBA color
    pub color: [f32; 4],
    /// Transparency (0 = opaque, 1 = fully transparent)
    pub transparency: f32,
    /// Padding for alignment
    pub _padding: [f32; 3],
}

impl GpuTriangle {
    /// Create a new triangle primitive with per-vertex normals
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        v0: [f32; 3],
        v1: [f32; 3],
        v2: [f32; 3],
        n0: [f32; 3],
        n1: [f32; 3],
        n2: [f32; 3],
        color: [f32; 4],
        transparency: f32,
    ) -> Self {
        Self {
            v0,
            _pad0: 0.0,
            v1,
            _pad1: 0.0,
            v2,
            _pad2: 0.0,
            n0,
            _pad3: 0.0,
            n1,
            _pad4: 0.0,
            n2,
            _pad5: 0.0,
            color,
            transparency,
            _padding: [0.0; 3],
        }
    }

    /// Create a triangle with flat shading (computed normal)
    pub fn flat(
        v0: [f32; 3],
        v1: [f32; 3],
        v2: [f32; 3],
        color: [f32; 4],
        transparency: f32,
    ) -> Self {
        // Compute face normal
        let e1 = [v1[0] - v0[0], v1[1] - v0[1], v1[2] - v0[2]];
        let e2 = [v2[0] - v0[0], v2[1] - v0[1], v2[2] - v0[2]];
        let n = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        let n = if len > 1e-10 {
            [n[0] / len, n[1] / len, n[2] / len]
        } else {
            [0.0, 1.0, 0.0]
        };

        Self::new(v0, v1, v2, n, n, n, color, transparency)
    }

    /// Check if triangle is transparent
    pub fn is_transparent(&self) -> bool {
        self.transparency > 0.001 || self.color[3] < 0.999
    }

    /// Get axis-aligned bounding box
    pub fn aabb(&self) -> ([f32; 3], [f32; 3]) {
        let min = [
            self.v0[0].min(self.v1[0]).min(self.v2[0]),
            self.v0[1].min(self.v1[1]).min(self.v2[1]),
            self.v0[2].min(self.v1[2]).min(self.v2[2]),
        ];
        let max = [
            self.v0[0].max(self.v1[0]).max(self.v2[0]),
            self.v0[1].max(self.v1[1]).max(self.v2[1]),
            self.v0[2].max(self.v1[2]).max(self.v2[2]),
        ];
        (min, max)
    }

    /// Get centroid
    pub fn centroid(&self) -> [f32; 3] {
        [
            (self.v0[0] + self.v1[0] + self.v2[0]) / 3.0,
            (self.v0[1] + self.v1[1] + self.v2[1]) / 3.0,
            (self.v0[2] + self.v1[2] + self.v2[2]) / 3.0,
        ]
    }
}

/// Collection of all primitives for raytracing
#[derive(Clone, Debug, Default)]
pub struct Primitives {
    /// Sphere primitives (atoms)
    pub spheres: Vec<GpuSphere>,
    /// Cylinder primitives (line-like segments)
    pub cylinders: Vec<GpuCylinder>,
    /// Capsule primitives (sticks)
    pub capsules: Vec<GpuCapsule>,
    /// Triangle primitives (surfaces, cartoons)
    pub triangles: Vec<GpuTriangle>,
}

impl Primitives {
    /// Create empty primitive collection
    pub fn new() -> Self {
        Self::default()
    }

    /// Total number of primitives
    pub fn total_count(&self) -> usize {
        self.spheres.len() + self.cylinders.len() + self.capsules.len() + self.triangles.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.spheres.is_empty()
            && self.cylinders.is_empty()
            && self.capsules.is_empty()
            && self.triangles.is_empty()
    }

    /// Compute overall bounding box
    pub fn aabb(&self) -> Option<([f32; 3], [f32; 3])> {
        let mut min = [f32::MAX; 3];
        let mut max = [f32::MIN; 3];
        let mut has_any = false;

        for sphere in &self.spheres {
            let (smin, smax) = sphere.aabb();
            for i in 0..3 {
                min[i] = min[i].min(smin[i]);
                max[i] = max[i].max(smax[i]);
            }
            has_any = true;
        }

        for cyl in &self.cylinders {
            let (cmin, cmax) = cyl.aabb();
            for i in 0..3 {
                min[i] = min[i].min(cmin[i]);
                max[i] = max[i].max(cmax[i]);
            }
            has_any = true;
        }

        for capsule in &self.capsules {
            let (cmin, cmax) = capsule.aabb();
            for i in 0..3 {
                min[i] = min[i].min(cmin[i]);
                max[i] = max[i].max(cmax[i]);
            }
            has_any = true;
        }

        for tri in &self.triangles {
            let (tmin, tmax) = tri.aabb();
            for i in 0..3 {
                min[i] = min[i].min(tmin[i]);
                max[i] = max[i].max(tmax[i]);
            }
            has_any = true;
        }

        if has_any {
            Some((min, max))
        } else {
            None
        }
    }
}

/// Builder for collecting primitives from scene data
#[derive(Default)]
pub struct PrimitiveCollector {
    primitives: Primitives,
}

impl PrimitiveCollector {
    /// Create a new primitive collector
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a sphere
    pub fn add_sphere(&mut self, sphere: GpuSphere) {
        self.primitives.spheres.push(sphere);
    }

    /// Add multiple spheres
    pub fn add_spheres(&mut self, spheres: impl IntoIterator<Item = GpuSphere>) {
        self.primitives.spheres.extend(spheres);
    }

    /// Add a cylinder
    pub fn add_cylinder(&mut self, cylinder: GpuCylinder) {
        self.primitives.cylinders.push(cylinder);
    }

    /// Add multiple cylinders
    pub fn add_cylinders(&mut self, cylinders: impl IntoIterator<Item = GpuCylinder>) {
        self.primitives.cylinders.extend(cylinders);
    }

    /// Add a capsule.
    pub fn add_capsule(&mut self, capsule: GpuCapsule) {
        self.primitives.capsules.push(capsule);
    }

    /// Add multiple capsules.
    pub fn add_capsules(&mut self, capsules: impl IntoIterator<Item = GpuCapsule>) {
        self.primitives.capsules.extend(capsules);
    }

    /// Add a triangle
    pub fn add_triangle(&mut self, triangle: GpuTriangle) {
        self.primitives.triangles.push(triangle);
    }

    /// Add multiple triangles
    pub fn add_triangles(&mut self, triangles: impl IntoIterator<Item = GpuTriangle>) {
        self.primitives.triangles.extend(triangles);
    }

    /// Build the final primitive collection
    pub fn build(self) -> Primitives {
        self.primitives
    }

    /// Get reference to current primitives
    pub fn primitives(&self) -> &Primitives {
        &self.primitives
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sphere_aabb() {
        let sphere = GpuSphere::new([1.0, 2.0, 3.0], 0.5, [1.0, 0.0, 0.0, 1.0], 0.0);
        let (min, max) = sphere.aabb();
        assert_eq!(min, [0.5, 1.5, 2.5]);
        assert_eq!(max, [1.5, 2.5, 3.5]);
    }

    #[test]
    fn test_cylinder_aabb() {
        let cyl = GpuCylinder::new(
            [0.0, 0.0, 0.0],
            [2.0, 2.0, 2.0],
            0.5,
            [1.0, 0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0, 1.0],
            0.0,
        );
        let (min, max) = cyl.aabb();
        assert_eq!(min, [-0.5, -0.5, -0.5]);
        assert_eq!(max, [2.5, 2.5, 2.5]);
    }

    #[test]
    fn test_capsule_aabb() {
        let capsule = GpuCapsule::new(
            [0.0, 0.0, 0.0],
            [2.0, 2.0, 2.0],
            0.5,
            [1.0, 0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0, 1.0],
            0.0,
        );
        let (min, max) = capsule.aabb();
        assert_eq!(min, [-0.5, -0.5, -0.5]);
        assert_eq!(max, [2.5, 2.5, 2.5]);
        assert_eq!(capsule.centroid(), [1.0, 1.0, 1.0]);
    }

    #[test]
    fn test_triangle_normal() {
        let tri = GpuTriangle::flat(
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 1.0, 1.0, 1.0],
            0.0,
        );
        // Normal should point in +Z direction
        assert!((tri.n0[2] - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_primitive_collector() {
        let mut collector = PrimitiveCollector::new();
        collector.add_sphere(GpuSphere::new([0.0; 3], 1.0, [1.0; 4], 0.0));
        collector.add_cylinder(GpuCylinder::new(
            [0.0; 3], [1.0; 3], 0.1, [1.0; 4], [1.0; 4], 0.0,
        ));
        collector.add_capsule(GpuCapsule::new(
            [0.0; 3], [1.0; 3], 0.1, [1.0; 4], [1.0; 4], 0.0,
        ));
        let prims = collector.build();
        assert_eq!(prims.total_count(), 3);
        assert_eq!(prims.spheres[0].center, [0.0; 3]);
        assert_eq!(prims.spheres[0].radius, 1.0);
        assert_eq!(prims.cylinders[0].start, [0.0; 3]);
        assert_eq!(prims.cylinders[0].end, [1.0; 3]);
        assert_eq!(prims.capsules[0].start, [0.0; 3]);
        assert_eq!(prims.capsules[0].end, [1.0; 3]);
    }
}
