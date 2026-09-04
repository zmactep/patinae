//! Extracts contour geometry from scalar grids.
//!
//! This module provides a renderer-neutral marching-cubes backend for map
//! isomesh and isosurface objects. Geometry is emitted in world coordinates
//! using [`Grid3D`]'s origin and per-axis spacing.

mod mc_tables;

use std::collections::HashSet;

use mc_tables::{EDGE_TABLE, EDGE_VERTICES, TRI_COUNT, TRI_TABLE};

use super::Grid3D;

/// Geometry topology emitted by contour extraction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContourTopology {
    /// Independent line segments.
    Lines,
    /// Independent triangles.
    Triangles,
}

/// Vertex emitted by contour extraction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContourVertex {
    /// Vertex position in world coordinates.
    pub position: [f32; 3],
    /// Unit-ish normal in world coordinates.
    pub normal: [f32; 3],
}

/// Indexed contour geometry.
#[derive(Debug, Clone, PartialEq)]
pub struct ContourGeometry {
    /// Primitive topology.
    pub topology: ContourTopology,
    /// Vertex buffer payload.
    pub vertices: Vec<ContourVertex>,
    /// Index buffer payload.
    pub indices: Vec<u32>,
}

impl ContourGeometry {
    /// Creates empty contour geometry with `topology`.
    #[must_use]
    pub fn empty(topology: ContourTopology) -> Self {
        Self {
            topology,
            vertices: Vec::new(),
            indices: Vec::new(),
        }
    }

    /// Returns true when no drawable primitives were emitted.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }
}

/// Extracts a line isomesh from `grid` at `level`.
#[must_use]
pub fn extract_isomesh(grid: &Grid3D, level: f32) -> ContourGeometry {
    extract_contour(grid, level, ContourTopology::Lines)
}

/// Extracts a triangle isosurface from `grid` at `level`.
#[must_use]
pub fn extract_isosurface(grid: &Grid3D, level: f32) -> ContourGeometry {
    extract_contour(grid, level, ContourTopology::Triangles)
}

fn extract_contour(grid: &Grid3D, level: f32, topology: ContourTopology) -> ContourGeometry {
    if grid.values.len() != grid.vertex_count() || grid.dims.contains(&0) {
        return ContourGeometry::empty(topology);
    }

    let estimated_cells = grid.dims[0] * grid.dims[1] * grid.dims[2];
    let mut geometry = ContourGeometry {
        topology,
        vertices: Vec::with_capacity(estimated_cells.saturating_mul(3)),
        indices: Vec::with_capacity(estimated_cells.saturating_mul(6)),
    };
    let mut emitted_lines = HashSet::new();

    for z in 0..grid.dims[2] {
        for y in 0..grid.dims[1] {
            for x in 0..grid.dims[0] {
                let cube = sample_cube(grid, x, y, z, level);
                if cube.edge_mask == 0 {
                    continue;
                }
                match topology {
                    ContourTopology::Triangles => emit_triangles(&mut geometry, &cube),
                    ContourTopology::Lines => {
                        emit_lines(&mut geometry, &cube, &mut emitted_lines);
                    }
                }
            }
        }
    }

    geometry
}

#[derive(Clone, Copy)]
struct EdgeVertex {
    position: [f32; 3],
    normal: [f32; 3],
}

struct CubeContour {
    case_index: usize,
    edge_mask: u16,
    edge_vertices: [Option<EdgeVertex>; 12],
}

fn sample_cube(grid: &Grid3D, x: usize, y: usize, z: usize, level: f32) -> CubeContour {
    let values = grid.cube_values(x, y, z);
    let positions = grid.cube_positions(x, y, z);
    let mut gradients = [[0.0; 3]; 8];
    for (corner, offset) in mc_tables::CORNER_OFFSETS.iter().enumerate() {
        gradients[corner] = gradient_at_vertex(
            grid,
            x + offset[0] as usize,
            y + offset[1] as usize,
            z + offset[2] as usize,
        );
    }

    let mut case_index = 0usize;
    for (corner, value) in values.iter().enumerate() {
        if *value >= level {
            case_index |= 1usize << corner;
        }
    }

    let edge_mask = EDGE_TABLE[case_index];
    let mut edge_vertices = [None; 12];
    for edge in 0..12 {
        if (edge_mask & (1u16 << edge)) == 0 {
            continue;
        }
        let [a, b] = EDGE_VERTICES[edge];
        edge_vertices[edge] = Some(interpolate_edge(
            values[a as usize],
            values[b as usize],
            positions[a as usize],
            positions[b as usize],
            gradients[a as usize],
            gradients[b as usize],
            level,
        ));
    }

    CubeContour {
        case_index,
        edge_mask,
        edge_vertices,
    }
}

fn interpolate_edge(
    value_a: f32,
    value_b: f32,
    position_a: [f32; 3],
    position_b: [f32; 3],
    normal_a: [f32; 3],
    normal_b: [f32; 3],
    level: f32,
) -> EdgeVertex {
    let denom = value_b - value_a;
    let t = if denom.abs() <= f32::EPSILON {
        0.5
    } else {
        ((level - value_a) / denom).clamp(0.0, 1.0)
    };
    let position = lerp3(position_a, position_b, t);
    let normal = normalize3(lerp3(normal_a, normal_b, t));
    EdgeVertex { position, normal }
}

fn emit_triangles(geometry: &mut ContourGeometry, cube: &CubeContour) {
    let row = TRI_TABLE[cube.case_index];
    for tri in 0..TRI_COUNT[cube.case_index] as usize {
        for slot in (0..3).rev() {
            let edge = row[tri * 3 + slot] as usize;
            let Some(vertex) = cube.edge_vertices[edge] else {
                continue;
            };
            push_vertex(geometry, vertex);
        }
    }
}

fn emit_lines(
    geometry: &mut ContourGeometry,
    cube: &CubeContour,
    emitted_lines: &mut HashSet<LineKey>,
) {
    let row = TRI_TABLE[cube.case_index];
    let mut local_segments = Vec::with_capacity(TRI_COUNT[cube.case_index] as usize * 3);

    for tri in 0..TRI_COUNT[cube.case_index] as usize {
        let edges = [
            row[tri * 3 + 2] as u8,
            row[tri * 3 + 1] as u8,
            row[tri * 3] as u8,
        ];
        local_segments.push(edge_pair(edges[0], edges[1]));
        local_segments.push(edge_pair(edges[1], edges[2]));
        local_segments.push(edge_pair(edges[2], edges[0]));
    }

    for (idx, pair) in local_segments.iter().enumerate() {
        if local_segments
            .iter()
            .filter(|candidate| *candidate == pair)
            .count()
            > 1
        {
            continue;
        }
        let Some(start) = cube.edge_vertices[pair[0] as usize] else {
            continue;
        };
        let Some(end) = cube.edge_vertices[pair[1] as usize] else {
            continue;
        };
        let key = LineKey::new(start.position, end.position);
        if !emitted_lines.insert(key) {
            continue;
        }
        push_vertex(geometry, start);
        push_vertex(geometry, end);
        debug_assert!(idx < local_segments.len());
    }
}

fn push_vertex(geometry: &mut ContourGeometry, vertex: EdgeVertex) {
    let index = geometry.vertices.len() as u32;
    geometry.vertices.push(ContourVertex {
        position: vertex.position,
        normal: vertex.normal,
    });
    geometry.indices.push(index);
}

fn edge_pair(a: u8, b: u8) -> [u8; 2] {
    if a <= b {
        [a, b]
    } else {
        [b, a]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct LineKey {
    a: [i64; 3],
    b: [i64; 3],
}

impl LineKey {
    fn new(a: [f32; 3], b: [f32; 3]) -> Self {
        let qa = quantize3(a);
        let qb = quantize3(b);
        if qa <= qb {
            Self { a: qa, b: qb }
        } else {
            Self { a: qb, b: qa }
        }
    }
}

fn quantize3(v: [f32; 3]) -> [i64; 3] {
    const SCALE: f32 = 1_000_000.0;
    [
        (v[0] * SCALE).round() as i64,
        (v[1] * SCALE).round() as i64,
        (v[2] * SCALE).round() as i64,
    ]
}

fn gradient_at_vertex(grid: &Grid3D, x: usize, y: usize, z: usize) -> [f32; 3] {
    [
        axis_gradient(grid, x, y, z, 0),
        axis_gradient(grid, x, y, z, 1),
        axis_gradient(grid, x, y, z, 2),
    ]
}

fn axis_gradient(grid: &Grid3D, x: usize, y: usize, z: usize, axis: usize) -> f32 {
    let vd = grid.vertex_dims();
    let spacing = grid.spacing[axis].abs().max(f32::EPSILON);
    let mut prev = [x, y, z];
    let mut next = [x, y, z];

    if [x, y, z][axis] > 0 {
        prev[axis] -= 1;
    }
    if [x, y, z][axis] + 1 < vd[axis] {
        next[axis] += 1;
    }

    if prev[axis] == next[axis] {
        return 0.0;
    }

    let prev_value = grid.get(prev[0], prev[1], prev[2]);
    let next_value = grid.get(next[0], next[1], next[2]);
    let distance = (next[axis] as f32 - prev[axis] as f32) * spacing;
    (next_value - prev_value) / distance
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

fn normalize3(v: [f32; 3]) -> [f32; 3] {
    let len2 = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
    if len2 <= f32::EPSILON {
        return [0.0, 0.0, 1.0];
    }
    let inv = len2.sqrt().recip();
    [v[0] * inv, v[1] * inv, v[2] * inv]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plane_x_grid(dims: [usize; 3], spacing: [f32; 3]) -> Grid3D {
        let vd = [dims[0] + 1, dims[1] + 1, dims[2] + 1];
        let mut values = Vec::with_capacity(vd[0] * vd[1] * vd[2]);
        for z in 0..vd[2] {
            for y in 0..vd[1] {
                for x in 0..vd[0] {
                    let _ = (y, z);
                    values.push(x as f32 * spacing[0]);
                }
            }
        }
        Grid3D::from_dims([10.0, 20.0, 30.0], spacing, dims, values)
    }

    #[test]
    fn no_crossing_emits_no_geometry() {
        let grid = Grid3D::from_dims([0.0; 3], [1.0; 3], [1, 1, 1], vec![0.0; 8]);

        let surface = extract_isosurface(&grid, 1.0);
        let mesh = extract_isomesh(&grid, 1.0);

        assert!(surface.is_empty());
        assert!(mesh.is_empty());
    }

    #[test]
    fn isosurface_uses_world_space_and_anisotropic_spacing() {
        let grid = plane_x_grid([1, 1, 1], [2.0, 3.0, 4.0]);

        let surface = extract_isosurface(&grid, 1.0);

        assert!(!surface.is_empty());
        for vertex in &surface.vertices {
            assert!((vertex.position[0] - 11.0).abs() < 1e-5);
            assert!(vertex.normal.iter().all(|v| v.is_finite()));
        }
    }

    #[test]
    fn isomesh_has_no_duplicate_line_segments() {
        let grid = plane_x_grid([2, 2, 2], [1.0, 1.0, 1.0]);

        let mesh = extract_isomesh(&grid, 1.0);

        assert_eq!(mesh.topology, ContourTopology::Lines);
        assert_eq!(mesh.indices.len() % 2, 0);
        let mut seen = HashSet::new();
        for pair in mesh.indices.as_chunks::<2>().0 {
            let a = mesh.vertices[pair[0] as usize].position;
            let b = mesh.vertices[pair[1] as usize].position;
            assert!(seen.insert(LineKey::new(a, b)));
        }
    }

    #[test]
    fn isosurface_normals_are_finite() {
        let grid = plane_x_grid([2, 2, 2], [1.0, 1.5, 2.0]);

        let surface = extract_isosurface(&grid, 1.0);

        assert!(!surface.is_empty());
        for vertex in surface.vertices {
            assert!(vertex.normal.iter().all(|v| v.is_finite()));
            let len = (vertex.normal[0] * vertex.normal[0]
                + vertex.normal[1] * vertex.normal[1]
                + vertex.normal[2] * vertex.normal[2])
                .sqrt();
            assert!((len - 1.0).abs() < 1e-4);
        }
    }
}
