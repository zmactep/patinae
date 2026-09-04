//! Renderer-neutral export of currently displayed molecular geometry.
//!
//! This module deliberately exports display geometry, not raytracer
//! primitives. Downstream consumers can adapt it to ray tracing, Blender,
//! VR/AR, debug visualisation, or file formats without coupling
//! `patinae-render` to any one consumer.

use thiserror::Error;

use crate::picking::{ObjectId, RepKind};

pub(crate) mod analytic;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GeometryExportOptions {
    /// Include renderer mesh outputs such as cartoon/ribbon/surface.
    pub include_meshes: bool,
    /// Include analytic atoms/bonds where the renderer semantics are
    /// naturally analytic.
    pub include_analytic: bool,
    /// Include semantic samples for screen-space representations.
    pub include_semantic_samples: bool,
}

impl Default for GeometryExportOptions {
    fn default() -> Self {
        Self {
            include_meshes: true,
            include_analytic: true,
            include_semantic_samples: true,
        }
    }
}

/// Whole-scene displayed geometry grouped by renderer object id.
#[derive(Debug, Clone, Default)]
pub struct DisplayedGeometry {
    pub objects: Vec<DisplayedObjectGeometry>,
}

impl DisplayedGeometry {
    pub fn is_empty(&self) -> bool {
        self.objects.iter().all(|obj| obj.primitives.is_empty())
    }

    pub fn primitive_count(&self) -> usize {
        self.objects.iter().map(|obj| obj.primitives.len()).sum()
    }
}

/// Display geometry for one renderer object.
#[derive(Debug, Clone)]
pub struct DisplayedObjectGeometry {
    pub object_id: ObjectId,
    pub primitives: Vec<DisplayedPrimitive>,
}

/// A renderer-neutral displayed primitive.
#[derive(Debug, Clone)]
pub enum DisplayedPrimitive {
    /// Non-indexed triangle-list mesh. Used for Cartoon, Ribbon, Surface.
    Mesh { rep: RepKind, mesh: DisplayedMesh },
    /// Analytic sphere. Used for Spheres and also stick end caps.
    Sphere {
        rep: RepKind,
        owner_atom_id: u32,
        center: [f32; 3],
        radius: f32,
        material: DisplayedMaterial,
    },
    /// Analytic cylinder body. Consumers that need capsules can pair stick
    /// cylinders with the exported stick cap spheres.
    Cylinder {
        rep: RepKind,
        owner_atom_ids: [u32; 2],
        start: [f32; 3],
        end: [f32; 3],
        radius: f32,
        material_start: DisplayedMaterial,
        material_end: DisplayedMaterial,
    },
    /// Semantic line segment for screen-space line or mesh-wire displays.
    LineSegment {
        rep: RepKind,
        owner_atom_ids: [u32; 2],
        start: [f32; 3],
        end: [f32; 3],
        width_px: f32,
        material_start: DisplayedMaterial,
        material_end: DisplayedMaterial,
    },
    /// Semantic dot sample for screen-space dot displays.
    PointSample {
        rep: RepKind,
        owner_atom_id: u32,
        position: [f32; 3],
        radius_px: f32,
        material: DisplayedMaterial,
    },
}

impl DisplayedPrimitive {
    pub fn rep_kind(&self) -> RepKind {
        match self {
            DisplayedPrimitive::Mesh { rep, .. }
            | DisplayedPrimitive::Sphere { rep, .. }
            | DisplayedPrimitive::Cylinder { rep, .. }
            | DisplayedPrimitive::LineSegment { rep, .. }
            | DisplayedPrimitive::PointSample { rep, .. } => *rep,
        }
    }
}

/// Non-indexed triangle-list mesh data.
#[derive(Debug, Clone, Default)]
pub struct DisplayedMesh {
    pub vertices: Vec<DisplayedMeshVertex>,
}

/// Vertex emitted by a displayed mesh.
#[derive(Debug, Clone)]
pub struct DisplayedMeshVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub owner_atom_id: u32,
    pub material: DisplayedMaterial,
    pub flags: u32,
}

/// Renderer-resolved material for an exported primitive or vertex.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DisplayedMaterial {
    /// Host-resolved atom base colour before representation override/alpha.
    pub base_rgba: [f32; 4],
    /// Representation colour after rep-specific override, before alpha.
    pub rep_rgba: [f32; 4],
    /// Final display colour after per-rep/per-atom alpha resolution.
    pub rgba: [f32; 4],
    /// Transparency in PyMOL terms: 0 = opaque, 1 = fully transparent.
    pub transparency: f32,
}

impl DisplayedMaterial {
    pub fn from_rgba(rgba: [f32; 4]) -> Self {
        Self {
            base_rgba: rgba,
            rep_rgba: rgba,
            rgba,
            transparency: 1.0 - rgba[3].clamp(0.0, 1.0),
        }
    }
}

/// Whole-scene trace geometry in a bounded runtime chunk.
#[derive(Debug, Clone, Default)]
pub struct TraceGeometryChunk {
    /// Analytic spheres.
    pub spheres: Vec<TraceSphere>,
    /// Analytic cylinders.
    pub cylinders: Vec<TraceCylinder>,
    /// Triangle-list mesh primitives.
    pub triangles: Vec<TraceTriangle>,
    /// Semantic line samples for screen-space line-like reps.
    pub line_segments: Vec<TraceLineSegment>,
    /// Semantic point samples for screen-space point-like reps.
    pub point_samples: Vec<TracePointSample>,
}

impl TraceGeometryChunk {
    /// Returns true when the chunk has no trace primitives.
    pub fn is_empty(&self) -> bool {
        self.spheres.is_empty()
            && self.cylinders.is_empty()
            && self.triangles.is_empty()
            && self.line_segments.is_empty()
            && self.point_samples.is_empty()
    }

    /// Counts all trace primitives in the chunk.
    pub fn primitive_count(&self) -> usize {
        self.spheres.len()
            + self.cylinders.len()
            + self.triangles.len()
            + self.line_segments.len()
            + self.point_samples.len()
    }

    /// Builds a compact trace chunk from displayed geometry.
    pub fn from_displayed(displayed: &DisplayedGeometry) -> Self {
        let mut chunk = Self::default();
        for object in &displayed.objects {
            for primitive in &object.primitives {
                append_displayed_primitive_to_trace(primitive, &mut chunk);
            }
        }
        chunk
    }
}

/// Ray-ready material reduced to color plus transparency.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TraceMaterial {
    /// Final resolved RGBA color.
    pub rgba: [f32; 4],
    /// PyMOL-style transparency: 0 = opaque, 1 = fully transparent.
    pub transparency: f32,
}

impl TraceMaterial {
    /// Reduces a displayed material to trace material.
    pub fn from_displayed(material: DisplayedMaterial) -> Self {
        Self {
            rgba: material.rgba,
            transparency: material.transparency,
        }
    }
}

/// Compact sphere primitive for trace export.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TraceSphere {
    /// Sphere center.
    pub center: [f32; 3],
    /// Sphere radius.
    pub radius: f32,
    /// Resolved material.
    pub material: TraceMaterial,
}

/// Compact cylinder primitive for trace export.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TraceCylinder {
    /// Cylinder start.
    pub start: [f32; 3],
    /// Cylinder end.
    pub end: [f32; 3],
    /// Cylinder radius.
    pub radius: f32,
    /// Material at the start.
    pub material_start: TraceMaterial,
    /// Material at the end.
    pub material_end: TraceMaterial,
}

/// Compact triangle primitive for trace export.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TraceTriangle {
    /// Triangle vertex positions.
    pub positions: [[f32; 3]; 3],
    /// Triangle vertex normals.
    pub normals: [[f32; 3]; 3],
    /// Pre-reduced triangle material.
    pub material: TraceMaterial,
}

/// Semantic line sample for trace export.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TraceLineSegment {
    /// Line start.
    pub start: [f32; 3],
    /// Line end.
    pub end: [f32; 3],
    /// Screen-space width in pixels.
    pub width_px: f32,
    /// Material at the start.
    pub material_start: TraceMaterial,
    /// Material at the end.
    pub material_end: TraceMaterial,
}

/// Semantic point sample for trace export.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TracePointSample {
    /// Point position.
    pub position: [f32; 3],
    /// Screen-space radius in pixels.
    pub radius_px: f32,
    /// Resolved material.
    pub material: TraceMaterial,
}

pub(crate) fn append_displayed_primitive_to_trace(
    primitive: &DisplayedPrimitive,
    chunk: &mut TraceGeometryChunk,
) {
    match primitive {
        DisplayedPrimitive::Mesh { mesh, .. } => {
            for tri in mesh.vertices.as_chunks::<3>().0 {
                chunk
                    .triangles
                    .push(trace_triangle_from_vertices(&tri[0], &tri[1], &tri[2]));
            }
        }
        DisplayedPrimitive::Sphere {
            center,
            radius,
            material,
            ..
        } => chunk.spheres.push(TraceSphere {
            center: *center,
            radius: *radius,
            material: TraceMaterial::from_displayed(*material),
        }),
        DisplayedPrimitive::Cylinder {
            start,
            end,
            radius,
            material_start,
            material_end,
            ..
        } => chunk.cylinders.push(TraceCylinder {
            start: *start,
            end: *end,
            radius: *radius,
            material_start: TraceMaterial::from_displayed(*material_start),
            material_end: TraceMaterial::from_displayed(*material_end),
        }),
        DisplayedPrimitive::LineSegment {
            start,
            end,
            width_px,
            material_start,
            material_end,
            ..
        } => chunk.line_segments.push(TraceLineSegment {
            start: *start,
            end: *end,
            width_px: *width_px,
            material_start: TraceMaterial::from_displayed(*material_start),
            material_end: TraceMaterial::from_displayed(*material_end),
        }),
        DisplayedPrimitive::PointSample {
            position,
            radius_px,
            material,
            ..
        } => chunk.point_samples.push(TracePointSample {
            position: *position,
            radius_px: *radius_px,
            material: TraceMaterial::from_displayed(*material),
        }),
    }
}

pub(crate) fn trace_triangle_from_vertices(
    a: &DisplayedMeshVertex,
    b: &DisplayedMeshVertex,
    c: &DisplayedMeshVertex,
) -> TraceTriangle {
    TraceTriangle {
        positions: [a.position, b.position, c.position],
        normals: [a.normal, b.normal, c.normal],
        material: trace_triangle_material(a.material, b.material, c.material),
    }
}

pub(crate) fn trace_triangle_material(
    a: DisplayedMaterial,
    b: DisplayedMaterial,
    c: DisplayedMaterial,
) -> TraceMaterial {
    TraceMaterial {
        rgba: [
            (a.rgba[0] + b.rgba[0] + c.rgba[0]) / 3.0,
            (a.rgba[1] + b.rgba[1] + c.rgba[1]) / 3.0,
            (a.rgba[2] + b.rgba[2] + c.rgba[2]) / 3.0,
            (a.rgba[3] + b.rgba[3] + c.rgba[3]) / 3.0,
        ],
        transparency: a.transparency.max(b.transparency).max(c.transparency),
    }
}

/// Errors from blocking GPU readback or export conversion.
#[derive(Debug, Error)]
pub enum GeometryExportError {
    #[error("GPU readback failed: {0}")]
    Gpu(String),
    #[error("trace geometry visitor failed: {0}")]
    Visitor(String),
}
