use std::collections::HashMap;

use bytemuck::Pod;
use patinae_algos::surface::{extract_isomesh, extract_isosurface, ContourGeometry};

use super::state::*;
use crate::geometry_export::analytic::{append_cpu_object_geometry, material_for_atom, oct_decode};
use crate::geometry_export::{
    append_displayed_primitive_to_trace, trace_triangle_material, DisplayedGeometry,
    DisplayedMaterial, DisplayedMesh, DisplayedMeshVertex, DisplayedObjectGeometry,
    DisplayedPrimitive, GeometryExportError, GeometryExportOptions, TraceGeometryChunk,
    TraceLineSegment, TraceMaterial, TraceTriangle,
};
use crate::picking::RepKind;
use crate::render_input::{RenderInput, RenderMapInput, RenderMapMode, RenderObjectInput};
use crate::representations::cartoon::CartoonRep;
use crate::representations::mesh::StdVertex;
use crate::representations::surface::SurfaceRep;

/// Vertices per trace mesh readback.
///
/// The value is divisible by both two and three so trace chunks never split a
/// line segment or triangle. It keeps MessagePack chunks well below the 64 MiB
/// plugin runtime ABI limit while avoiding excessive GPU map calls.
const TRACE_MESH_READBACK_VERTICES: usize = 196_608;

impl RenderState {
    /// Export the geometry currently displayed by the renderer.
    ///
    /// The method syncs the normal [`RenderInput`], dispatches the existing
    /// representation builders, then performs blocking readback for
    /// GPU-generated mesh buffers. It is intended for offline/capture-style
    /// workflows and should not be called from the interactive frame loop.
    pub fn export_displayed_geometry(
        &mut self,
        input: &RenderInput<'_>,
        options: &GeometryExportOptions,
    ) -> Result<DisplayedGeometry, GeometryExportError> {
        self.sync(input);

        let mut geometry = DisplayedGeometry {
            objects: Vec::with_capacity(input.objects.len() + input.maps.len()),
        };
        geometry
            .objects
            .extend(input.objects.iter().map(|object| DisplayedObjectGeometry {
                object_id: object.object_id,
                primitives: Vec::new(),
            }));
        geometry
            .objects
            .extend(input.maps.iter().map(|map| DisplayedObjectGeometry {
                object_id: map.object_id,
                primitives: Vec::new(),
            }));
        let mut object_lookup: HashMap<u32, (usize, &RenderObjectInput<'_>)> =
            HashMap::with_capacity(input.objects.len());

        for (idx, object) in input.objects.iter().enumerate() {
            append_cpu_object_geometry(&mut geometry.objects[idx], object, input.settings, options);
            object_lookup.insert(object.object_id.0, (idx, object));
        }

        if options.include_meshes {
            for (map_idx, map) in input.maps.iter().enumerate() {
                let target_idx = input.objects.len() + map_idx;
                append_map_geometry(&mut geometry.objects[target_idx], map);
            }

            for &(object_id, rep_kind) in &self.scene.draw_order {
                let Some(entry) = self.scene.reps.get(&(object_id, rep_kind)) else {
                    continue;
                };
                let Some((target_idx, object_input)) = object_lookup.get(&object_id).copied()
                else {
                    continue;
                };

                match rep_kind {
                    RepKind::Cartoon | RepKind::Ribbon => {
                        let Some(rep) = entry.rep.as_any().downcast_ref::<CartoonRep>() else {
                            continue;
                        };
                        let Some((buffer, vertex_count)) = rep.export_vertices() else {
                            continue;
                        };
                        let vertices = read_buffer_prefix::<StdVertex>(
                            &self.ctx.device,
                            &self.ctx.queue,
                            buffer,
                            vertex_count as usize,
                            "patinae.export.cartoon_vertices",
                        )?;
                        append_mesh_vertices(
                            &mut geometry.objects[target_idx],
                            rep_kind,
                            vertices,
                            object_input,
                            input.settings,
                        );
                    }
                    RepKind::Surface | RepKind::Mesh => {
                        let Some(rep) = entry.rep.as_any().downcast_ref::<SurfaceRep>() else {
                            continue;
                        };
                        for (part_index, part) in rep.export_parts().enumerate() {
                            let count = read_surface_vertex_count(
                                &self.ctx.device,
                                &self.ctx.queue,
                                part.indirect_buf,
                                part.max_vertices,
                            )?;
                            if count == 0 {
                                continue;
                            }
                            let vertices = read_buffer_prefix::<StdVertex>(
                                &self.ctx.device,
                                &self.ctx.queue,
                                part.vertex_buf,
                                count as usize,
                                &format!("patinae.export.surface_vertices.{part_index}"),
                            )?;
                            if rep_kind == RepKind::Mesh {
                                append_mesh_lines(
                                    &mut geometry.objects[target_idx],
                                    vertices,
                                    object_input,
                                    input.settings,
                                );
                            } else {
                                append_mesh_vertices(
                                    &mut geometry.objects[target_idx],
                                    rep_kind,
                                    vertices,
                                    object_input,
                                    input.settings,
                                );
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        geometry
            .objects
            .retain(|object| !object.primitives.is_empty());
        Ok(geometry)
    }

    /// Visit compact trace geometry for the currently displayed scene.
    ///
    /// This export follows the same renderer sync and representation builders
    /// as [`Self::export_displayed_geometry`], but it converts directly into
    /// compact trace chunks. GPU mesh buffers are read back in bounded ranges
    /// and never assembled into one whole-scene displayed-geometry payload.
    pub fn for_each_trace_geometry_chunk(
        &mut self,
        input: &RenderInput<'_>,
        options: &GeometryExportOptions,
        visitor: &mut dyn FnMut(TraceGeometryChunk) -> Result<(), String>,
    ) -> Result<(), GeometryExportError> {
        self.sync(input);

        let mut object_lookup: HashMap<u32, &RenderObjectInput<'_>> =
            HashMap::with_capacity(input.objects.len());

        for object in input.objects {
            object_lookup.insert(object.object_id.0, object);
            let mut displayed = DisplayedObjectGeometry {
                object_id: object.object_id,
                primitives: Vec::new(),
            };
            append_cpu_object_geometry(&mut displayed, object, input.settings, options);
            visit_displayed_object_as_trace(&displayed, visitor)?;
        }

        if options.include_meshes {
            for map in input.maps {
                let mut displayed = DisplayedObjectGeometry {
                    object_id: map.object_id,
                    primitives: Vec::new(),
                };
                append_map_geometry(&mut displayed, map);
                visit_displayed_object_as_trace(&displayed, visitor)?;
            }

            for &(object_id, rep_kind) in &self.scene.draw_order {
                let Some(entry) = self.scene.reps.get(&(object_id, rep_kind)) else {
                    continue;
                };
                let Some(object_input) = object_lookup.get(&object_id).copied() else {
                    continue;
                };

                match rep_kind {
                    RepKind::Cartoon | RepKind::Ribbon => {
                        let Some(rep) = entry.rep.as_any().downcast_ref::<CartoonRep>() else {
                            continue;
                        };
                        let Some((buffer, vertex_count)) = rep.export_vertices() else {
                            continue;
                        };
                        for_each_buffer_range::<StdVertex>(
                            &self.ctx.device,
                            &self.ctx.queue,
                            buffer,
                            vertex_count as usize,
                            TRACE_MESH_READBACK_VERTICES,
                            "patinae.trace.cartoon_vertices",
                            &mut |vertices| {
                                let chunk = trace_chunk_from_mesh_vertices(
                                    rep_kind,
                                    vertices,
                                    object_input,
                                    input.settings,
                                );
                                visit_trace_chunk(chunk, visitor)
                            },
                        )?;
                    }
                    RepKind::Surface | RepKind::Mesh => {
                        let Some(rep) = entry.rep.as_any().downcast_ref::<SurfaceRep>() else {
                            continue;
                        };
                        for part in rep.export_parts() {
                            let count = read_surface_vertex_count(
                                &self.ctx.device,
                                &self.ctx.queue,
                                part.indirect_buf,
                                part.max_vertices,
                            )?;
                            if count == 0 {
                                continue;
                            }
                            for_each_buffer_range::<StdVertex>(
                                &self.ctx.device,
                                &self.ctx.queue,
                                part.vertex_buf,
                                count as usize,
                                TRACE_MESH_READBACK_VERTICES,
                                "patinae.trace.surface_vertices",
                                &mut |vertices| {
                                    let chunk = if rep_kind == RepKind::Mesh {
                                        trace_chunk_from_mesh_lines(
                                            vertices,
                                            object_input,
                                            input.settings,
                                        )
                                    } else {
                                        trace_chunk_from_mesh_vertices(
                                            rep_kind,
                                            vertices,
                                            object_input,
                                            input.settings,
                                        )
                                    };
                                    visit_trace_chunk(chunk, visitor)
                                },
                            )?;
                        }
                    }
                    _ => {}
                }
            }
        }

        Ok(())
    }
}

fn append_map_geometry(object: &mut DisplayedObjectGeometry, input: &RenderMapInput<'_>) {
    let contour = match input.mode {
        RenderMapMode::Isomesh => extract_isomesh(input.grid, input.level),
        RenderMapMode::Isosurface => extract_isosurface(input.grid, input.level),
    };
    match input.mode {
        RenderMapMode::Isomesh => append_map_lines(object, &contour, input),
        RenderMapMode::Isosurface => append_map_mesh(object, contour, input),
    }
}

fn append_map_mesh(
    object: &mut DisplayedObjectGeometry,
    contour: ContourGeometry,
    input: &RenderMapInput<'_>,
) {
    let material = DisplayedMaterial::from_rgba(input.color);
    let vertices = contour
        .indices
        .iter()
        .filter_map(|&idx| contour.vertices.get(idx as usize))
        .map(|vertex| DisplayedMeshVertex {
            position: transform_point(&input.transform, vertex.position),
            normal: transform_dir(&input.transform, vertex.normal),
            owner_atom_id: u32::MAX,
            material,
            flags: 0,
        })
        .collect::<Vec<_>>();
    if vertices.is_empty() {
        return;
    }
    object.primitives.push(DisplayedPrimitive::Mesh {
        rep: RepKind::Surface,
        mesh: DisplayedMesh { vertices },
    });
}

fn append_map_lines(
    object: &mut DisplayedObjectGeometry,
    contour: &ContourGeometry,
    input: &RenderMapInput<'_>,
) {
    let material = DisplayedMaterial::from_rgba(input.color);
    for pair in contour.indices.chunks_exact(2) {
        let Some(start) = contour.vertices.get(pair[0] as usize) else {
            continue;
        };
        let Some(end) = contour.vertices.get(pair[1] as usize) else {
            continue;
        };
        object.primitives.push(DisplayedPrimitive::LineSegment {
            rep: RepKind::Mesh,
            owner_atom_ids: [u32::MAX, u32::MAX],
            start: transform_point(&input.transform, start.position),
            end: transform_point(&input.transform, end.position),
            width_px: 1.0,
            material_start: material,
            material_end: material,
        });
    }
}

fn append_mesh_vertices(
    object: &mut DisplayedObjectGeometry,
    rep: RepKind,
    vertices: Vec<StdVertex>,
    input: &RenderObjectInput<'_>,
    scene_settings: &patinae_settings::ResolvedSettings,
) {
    let exported: Vec<DisplayedMeshVertex> = vertices
        .into_iter()
        .map(|v| {
            let owner_atom_id = v.group_id;
            DisplayedMeshVertex {
                position: transform_point(&input.transform, v.position),
                normal: transform_dir(&input.transform, oct_decode(v.normal_oct)),
                owner_atom_id,
                material: material_for_atom(input, scene_settings, rep, owner_atom_id),
                flags: v.flags,
            }
        })
        .collect();
    if exported.is_empty() {
        return;
    }
    object.primitives.push(DisplayedPrimitive::Mesh {
        rep,
        mesh: DisplayedMesh { vertices: exported },
    });
}

fn append_mesh_lines(
    object: &mut DisplayedObjectGeometry,
    vertices: Vec<StdVertex>,
    input: &RenderObjectInput<'_>,
    scene_settings: &patinae_settings::ResolvedSettings,
) {
    for pair in vertices.chunks_exact(2) {
        let a = pair[0];
        let b = pair[1];
        object.primitives.push(DisplayedPrimitive::LineSegment {
            rep: RepKind::Mesh,
            owner_atom_ids: [a.group_id, b.group_id],
            start: transform_point(&input.transform, a.position),
            end: transform_point(&input.transform, b.position),
            width_px: 1.0,
            material_start: material_for_atom(input, scene_settings, RepKind::Mesh, a.group_id),
            material_end: material_for_atom(input, scene_settings, RepKind::Mesh, b.group_id),
        });
    }
}

fn visit_displayed_object_as_trace(
    object: &DisplayedObjectGeometry,
    visitor: &mut dyn FnMut(TraceGeometryChunk) -> Result<(), String>,
) -> Result<(), GeometryExportError> {
    let mut chunk = TraceGeometryChunk::default();
    for primitive in &object.primitives {
        append_displayed_primitive_to_trace(primitive, &mut chunk);
    }
    visit_trace_chunk(chunk, visitor)
}

fn visit_trace_chunk(
    chunk: TraceGeometryChunk,
    visitor: &mut dyn FnMut(TraceGeometryChunk) -> Result<(), String>,
) -> Result<(), GeometryExportError> {
    if chunk.is_empty() {
        return Ok(());
    }
    visitor(chunk).map_err(GeometryExportError::Visitor)
}

fn trace_chunk_from_mesh_vertices(
    rep: RepKind,
    vertices: Vec<StdVertex>,
    input: &RenderObjectInput<'_>,
    scene_settings: &patinae_settings::ResolvedSettings,
) -> TraceGeometryChunk {
    let mut chunk = TraceGeometryChunk {
        triangles: Vec::with_capacity(vertices.len() / 3),
        ..TraceGeometryChunk::default()
    };

    for tri in vertices.chunks_exact(3) {
        let a = trace_vertex_from_std(tri[0], rep, input, scene_settings);
        let b = trace_vertex_from_std(tri[1], rep, input, scene_settings);
        let c = trace_vertex_from_std(tri[2], rep, input, scene_settings);
        chunk.triangles.push(TraceTriangle {
            positions: [a.position, b.position, c.position],
            normals: [a.normal, b.normal, c.normal],
            material: trace_triangle_material(a.material, b.material, c.material),
        });
    }

    chunk
}

fn trace_chunk_from_mesh_lines(
    vertices: Vec<StdVertex>,
    input: &RenderObjectInput<'_>,
    scene_settings: &patinae_settings::ResolvedSettings,
) -> TraceGeometryChunk {
    let mut chunk = TraceGeometryChunk {
        line_segments: Vec::with_capacity(vertices.len() / 2),
        ..TraceGeometryChunk::default()
    };

    for pair in vertices.chunks_exact(2) {
        let a = pair[0];
        let b = pair[1];
        chunk.line_segments.push(TraceLineSegment {
            start: transform_point(&input.transform, a.position),
            end: transform_point(&input.transform, b.position),
            width_px: 1.0,
            material_start: TraceMaterial::from_displayed(material_for_atom(
                input,
                scene_settings,
                RepKind::Mesh,
                a.group_id,
            )),
            material_end: TraceMaterial::from_displayed(material_for_atom(
                input,
                scene_settings,
                RepKind::Mesh,
                b.group_id,
            )),
        });
    }

    chunk
}

struct TraceVertex {
    position: [f32; 3],
    normal: [f32; 3],
    material: DisplayedMaterial,
}

fn trace_vertex_from_std(
    vertex: StdVertex,
    rep: RepKind,
    input: &RenderObjectInput<'_>,
    scene_settings: &patinae_settings::ResolvedSettings,
) -> TraceVertex {
    TraceVertex {
        position: transform_point(&input.transform, vertex.position),
        normal: transform_dir(&input.transform, oct_decode(vertex.normal_oct)),
        material: material_for_atom(input, scene_settings, rep, vertex.group_id),
    }
}

fn transform_point(m: &[[f32; 4]; 4], p: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * p[0] + m[1][0] * p[1] + m[2][0] * p[2] + m[3][0],
        m[0][1] * p[0] + m[1][1] * p[1] + m[2][1] * p[2] + m[3][1],
        m[0][2] * p[0] + m[1][2] * p[1] + m[2][2] * p[2] + m[3][2],
    ]
}

fn transform_dir(m: &[[f32; 4]; 4], n: [f32; 3]) -> [f32; 3] {
    let out = [
        m[0][0] * n[0] + m[1][0] * n[1] + m[2][0] * n[2],
        m[0][1] * n[0] + m[1][1] * n[1] + m[2][1] * n[2],
        m[0][2] * n[0] + m[1][2] * n[1] + m[2][2] * n[2],
    ];
    let len = (out[0] * out[0] + out[1] * out[1] + out[2] * out[2]).sqrt();
    if len <= f32::EPSILON {
        [0.0, 0.0, 1.0]
    } else {
        [out[0] / len, out[1] / len, out[2] / len]
    }
}

fn read_surface_vertex_count(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    indirect: &wgpu::Buffer,
    max_vertices: u32,
) -> Result<u32, GeometryExportError> {
    let values = read_buffer_prefix::<u32>(
        device,
        queue,
        indirect,
        1,
        "patinae.export.surface_indirect",
    )?;
    Ok(values.first().copied().unwrap_or(0).min(max_vertices))
}

fn read_buffer_prefix<T: Pod>(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    source: &wgpu::Buffer,
    count: usize,
    label: &str,
) -> Result<Vec<T>, GeometryExportError> {
    read_buffer_range(device, queue, source, 0, count, label)
}

fn for_each_buffer_range<T: Pod>(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    source: &wgpu::Buffer,
    count: usize,
    chunk_len: usize,
    label: &str,
    visitor: &mut dyn FnMut(Vec<T>) -> Result<(), GeometryExportError>,
) -> Result<(), GeometryExportError> {
    if count == 0 || chunk_len == 0 {
        return Ok(());
    }

    let mut start = 0usize;
    while start < count {
        let len = (count - start).min(chunk_len);
        let vertices = read_buffer_range(device, queue, source, start, len, label)?;
        visitor(vertices)?;
        start += len;
    }

    Ok(())
}

fn read_buffer_range<T: Pod>(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    source: &wgpu::Buffer,
    start: usize,
    count: usize,
    label: &str,
) -> Result<Vec<T>, GeometryExportError> {
    if count == 0 {
        return Ok(Vec::new());
    }
    let elem_size = std::mem::size_of::<T>();
    let byte_offset = start
        .checked_mul(elem_size)
        .ok_or_else(|| GeometryExportError::Gpu("GPU readback range overflow".to_string()))?
        as u64;
    let byte_len = count
        .checked_mul(elem_size)
        .ok_or_else(|| GeometryExportError::Gpu("GPU readback range overflow".to_string()))?
        as u64;
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: byte_len.max(4),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("patinae.export.readback"),
    });
    encoder.copy_buffer_to_buffer(source, byte_offset, &staging, 0, byte_len);
    queue.submit(std::iter::once(encoder.finish()));

    let slice = staging.slice(0..byte_len);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .map_err(|err| GeometryExportError::Gpu(err.to_string()))?;
    rx.recv()
        .map_err(|err| GeometryExportError::Gpu(err.to_string()))?
        .map_err(|err| GeometryExportError::Gpu(err.to_string()))?;

    let data = slice.get_mapped_range();
    let values = bytemuck::cast_slice(&data).to_vec();
    drop(data);
    staging.unmap();
    Ok(values)
}

#[cfg(test)]
mod tests {
    use patinae_algos::surface::Grid3D;

    use super::*;
    use crate::picking::ObjectId;

    const IDENTITY: [[f32; 4]; 4] = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];

    fn crossing_grid() -> Grid3D {
        Grid3D::from_dims(
            [0.0; 3],
            [1.0; 3],
            [1, 1, 1],
            vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        )
    }

    fn map_input<'a>(grid: &'a Grid3D, mode: RenderMapMode) -> RenderMapInput<'a> {
        RenderMapInput {
            object_id: ObjectId(7),
            grid,
            mode,
            level: 0.5,
            color: [0.2, 0.4, 0.8, 1.0],
            transform: IDENTITY,
            geometry_revision: 1,
            material_revision: 1,
            dirty: true,
        }
    }

    #[test]
    fn map_isomesh_export_emits_line_segments() {
        let grid = crossing_grid();
        let input = map_input(&grid, RenderMapMode::Isomesh);
        let mut object = DisplayedObjectGeometry {
            object_id: input.object_id,
            primitives: Vec::new(),
        };

        append_map_geometry(&mut object, &input);

        assert!(!object.primitives.is_empty());
        assert!(object.primitives.iter().all(|primitive| matches!(
            primitive,
            DisplayedPrimitive::LineSegment {
                rep: RepKind::Mesh,
                owner_atom_ids: [u32::MAX, u32::MAX],
                ..
            }
        )));
    }

    #[test]
    fn map_isosurface_export_emits_triangle_mesh() {
        let grid = crossing_grid();
        let input = map_input(&grid, RenderMapMode::Isosurface);
        let mut object = DisplayedObjectGeometry {
            object_id: input.object_id,
            primitives: Vec::new(),
        };

        append_map_geometry(&mut object, &input);

        let [DisplayedPrimitive::Mesh {
            rep: RepKind::Surface,
            mesh,
        }] = object.primitives.as_slice()
        else {
            panic!("expected one surface mesh primitive");
        };
        assert!(!mesh.vertices.is_empty());
        assert!(mesh
            .vertices
            .iter()
            .all(|vertex| vertex.owner_atom_id == u32::MAX));
    }
}
