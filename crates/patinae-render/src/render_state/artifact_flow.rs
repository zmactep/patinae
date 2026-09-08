//! Builds neutral renderer artifact snapshots for plugins.
//!
//! This layer maps renderer representation buffers into role-tagged artifact
//! descriptors without adding plugin-specific concepts. It synchronizes the
//! renderer first, exposes shared scene-store buffers when present, then lists
//! visible representation buffers with enough metadata for a plugin to decide
//! whether it understands each layout.
//!
//! Sphere, stick, and line representations expose their renderer instance
//! buffers and optional GPU culling count or indirect buffers. Cartoon and
//! ribbon expose direct `StdVertices` triangle lists. Surface exposes
//! `StdVertices` parts with indirect draw buffers because marching-cubes output
//! is GPU-counted. Mesh uses the same surface path but marks topology as
//! `LineList`.
//!
//! Per-representation metadata is deliberately neutral: object id, rep kind,
//! topology, atom range, material color, transparency, direct count, capacity,
//! and optional count-source handles. Raytracing, export, analysis, or overlay
//! plugins decide their own interpretation after checking these fields.

use std::collections::HashMap;

use super::state::*;
use crate::picking::RepKind;
use crate::render_artifacts::{
    RenderArtifactBufferRef, RenderArtifactBufferRole, RenderArtifactPrimitiveTopology,
    RenderArtifactRep, RenderArtifactSnapshot, RENDER_ARTIFACT_LAYOUT_VERSION,
};
use crate::render_input::{ColorLutEntry, RenderInput};
use crate::representations::cartoon::CartoonRep;
use crate::representations::line::LineRep;
use crate::representations::mesh::StdVertex;
use crate::representations::sphere::{SphereInstance, SphereRep};
use crate::representations::stick::{StickInstance, StickRep};
use crate::representations::surface::SurfaceRep;
use crate::scene_store::{AtomGpu, BondGpu, ObjectEntry};

impl RenderState {
    /// Snapshot renderer-owned GPU artifacts for command-scoped plugin use.
    ///
    /// The snapshot borrows buffers owned by `RenderState`. Hosts must turn
    /// these refs into command-scoped opaque handles and expire them before
    /// the next mutable renderer operation. The returned roles and strides are
    /// the renderer/plugin ABI; representation-specific shader bindings and
    /// pipeline details stay private to `patinae-render`.
    pub fn render_artifact_snapshot(
        &mut self,
        input: &RenderInput<'_>,
    ) -> RenderArtifactSnapshot<'_> {
        self.sync(input);

        let mut buffers = Vec::new();
        let mut reps = Vec::new();
        let object_metadata = object_metadata(input, self);
        let (scene_bounds_min, scene_bounds_max) = scene_bounds_min_max(self);
        push_scene_store_buffers(self, &mut buffers);

        for &(object_id, rep_kind) in &self.scene.draw_order {
            let Some(entry) = self.scene.reps.get(&(object_id, rep_kind)) else {
                continue;
            };
            match rep_kind {
                RepKind::Sphere => {
                    let Some(rep) = entry.rep.as_any().downcast_ref::<SphereRep>() else {
                        continue;
                    };
                    push_instance_rep(
                        InstanceArtifactSource {
                            object_id,
                            rep_kind,
                            topology: RenderArtifactPrimitiveTopology::SphereInstances,
                            geometry_role: RenderArtifactBufferRole::SphereInstances,
                            stride: SphereInstance::SIZE,
                            exported: rep.export_instances(),
                            metadata: object_metadata.get(&object_id).copied().unwrap_or_default(),
                        },
                        InstanceArtifactSink {
                            buffers: &mut buffers,
                            reps: &mut reps,
                        },
                    );
                }
                RepKind::Stick => {
                    let Some(rep) = entry.rep.as_any().downcast_ref::<StickRep>() else {
                        continue;
                    };
                    push_instance_rep(
                        InstanceArtifactSource {
                            object_id,
                            rep_kind,
                            topology: RenderArtifactPrimitiveTopology::CylinderInstances,
                            geometry_role: RenderArtifactBufferRole::StickInstances,
                            stride: StickInstance::SIZE,
                            exported: rep.export_instances(),
                            metadata: object_metadata.get(&object_id).copied().unwrap_or_default(),
                        },
                        InstanceArtifactSink {
                            buffers: &mut buffers,
                            reps: &mut reps,
                        },
                    );
                }
                RepKind::Line => {
                    let Some(rep) = entry.rep.as_any().downcast_ref::<LineRep>() else {
                        continue;
                    };
                    push_instance_rep(
                        InstanceArtifactSource {
                            object_id,
                            rep_kind,
                            topology: RenderArtifactPrimitiveTopology::LineInstances,
                            geometry_role: RenderArtifactBufferRole::LineInstances,
                            stride: crate::representations::line::LineInstance::SIZE,
                            exported: rep.export_instances(),
                            metadata: object_metadata.get(&object_id).copied().unwrap_or_default(),
                        },
                        InstanceArtifactSink {
                            buffers: &mut buffers,
                            reps: &mut reps,
                        },
                    );
                }
                RepKind::Cartoon | RepKind::Ribbon => {
                    let Some(rep) = entry.rep.as_any().downcast_ref::<CartoonRep>() else {
                        continue;
                    };
                    let Some((buffer, vertex_count)) = rep.export_vertices() else {
                        continue;
                    };
                    let geometry_index = push_buffer(
                        &mut buffers,
                        RenderArtifactBufferRole::StdVertices,
                        buffer,
                        StdVertex::SIZE,
                        u64::from(vertex_count),
                    );
                    reps.push(RenderArtifactRep {
                        object_id,
                        rep_kind,
                        topology: RenderArtifactPrimitiveTopology::TriangleList,
                        geometry_buffer_index: geometry_index,
                        count_buffer_index: None,
                        indirect_buffer_index: None,
                        element_count: u64::from(vertex_count),
                        max_element_count: u64::from(vertex_count),
                        atom_offset: object_metadata
                            .get(&object_id)
                            .copied()
                            .unwrap_or_default()
                            .atom_offset,
                        atom_count: object_metadata
                            .get(&object_id)
                            .copied()
                            .unwrap_or_default()
                            .atom_count,
                        material_rgba: object_metadata
                            .get(&object_id)
                            .copied()
                            .unwrap_or_default()
                            .material_rgba,
                        transparency: object_metadata
                            .get(&object_id)
                            .copied()
                            .unwrap_or_default()
                            .transparency_for(rep_kind),
                    });
                }
                RepKind::Surface | RepKind::Mesh => {
                    let Some(rep) = entry.rep.as_any().downcast_ref::<SurfaceRep>() else {
                        continue;
                    };
                    let topology = if rep_kind == RepKind::Mesh {
                        RenderArtifactPrimitiveTopology::LineList
                    } else {
                        RenderArtifactPrimitiveTopology::TriangleList
                    };
                    for part in rep.export_parts() {
                        let geometry_index = push_buffer(
                            &mut buffers,
                            RenderArtifactBufferRole::StdVertices,
                            part.vertex_buf,
                            StdVertex::SIZE,
                            u64::from(part.max_vertices),
                        );
                        let indirect_index = push_buffer(
                            &mut buffers,
                            RenderArtifactBufferRole::IndirectDraw,
                            part.indirect_buf,
                            16,
                            1,
                        );
                        reps.push(RenderArtifactRep {
                            object_id,
                            rep_kind,
                            topology,
                            geometry_buffer_index: geometry_index,
                            count_buffer_index: None,
                            indirect_buffer_index: Some(indirect_index),
                            element_count: 0,
                            max_element_count: u64::from(part.max_vertices),
                            atom_offset: object_metadata
                                .get(&object_id)
                                .copied()
                                .unwrap_or_default()
                                .atom_offset,
                            atom_count: object_metadata
                                .get(&object_id)
                                .copied()
                                .unwrap_or_default()
                                .atom_count,
                            material_rgba: object_metadata
                                .get(&object_id)
                                .copied()
                                .unwrap_or_default()
                                .material_rgba,
                            transparency: object_metadata
                                .get(&object_id)
                                .copied()
                                .unwrap_or_default()
                                .transparency_for(rep_kind),
                        });
                    }
                }
                _ => {}
            }
        }

        RenderArtifactSnapshot {
            layout_version: RENDER_ARTIFACT_LAYOUT_VERSION,
            scene_generation: 0,
            scene_bounds_min,
            scene_bounds_max,
            cull_pass_initialized: self.scene.cull_pass_initialized,
            buffers,
            reps,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ArtifactObjectMetadata {
    atom_offset: u32,
    atom_count: u32,
    material_rgba: [f32; 4],
    sphere_transparency: f32,
    stick_transparency: f32,
    line_transparency: f32,
    cartoon_transparency: f32,
    surface_transparency: f32,
    mesh_transparency: f32,
    ellipsoid_transparency: f32,
}

impl Default for ArtifactObjectMetadata {
    fn default() -> Self {
        Self {
            atom_offset: 0,
            atom_count: 0,
            material_rgba: [0.55, 0.85, 0.95, 1.0],
            sphere_transparency: 0.0,
            stick_transparency: 0.0,
            line_transparency: 0.0,
            cartoon_transparency: 0.0,
            surface_transparency: 0.0,
            mesh_transparency: 0.0,
            ellipsoid_transparency: 0.0,
        }
    }
}

impl ArtifactObjectMetadata {
    fn transparency_for(self, rep_kind: RepKind) -> f32 {
        match rep_kind {
            RepKind::Sphere => self.sphere_transparency,
            RepKind::Stick => self.stick_transparency,
            RepKind::Line => self.line_transparency,
            RepKind::Cartoon | RepKind::Ribbon => self.cartoon_transparency,
            RepKind::Surface => self.surface_transparency,
            RepKind::Mesh => self.mesh_transparency,
            RepKind::Ellipsoid => self.ellipsoid_transparency,
            _ => 0.0,
        }
    }
}

fn object_metadata(
    input: &RenderInput<'_>,
    state: &RenderState,
) -> HashMap<u32, ArtifactObjectMetadata> {
    let mut out = HashMap::with_capacity(input.objects.len());
    for object in input.objects {
        let mut metadata = ArtifactObjectMetadata::default();
        if let Some(slot) = state.scene.scene_store.slot(object.object_id) {
            metadata.atom_offset = slot.atom_offset;
            metadata.atom_count = slot.atom_count;
            if slot.atom_count > 0 {
                metadata.material_rgba =
                    state.scene.scene_store.color_lut.cpu()[slot.atom_offset as usize].base;
            }
        }
        let settings = object.object_settings.as_ref().unwrap_or(input.settings);
        metadata.sphere_transparency = settings.sphere.transparency;
        metadata.stick_transparency = settings.stick.transparency;
        metadata.line_transparency = 0.0;
        metadata.cartoon_transparency = settings.cartoon.transparency;
        metadata.surface_transparency = settings.surface.transparency;
        metadata.mesh_transparency = settings.mesh.transparency;
        metadata.ellipsoid_transparency = settings.ellipsoid.transparency;
        out.insert(object.object_id.0, metadata);
    }
    out
}

fn scene_bounds_min_max(state: &RenderState) -> ([f32; 3], [f32; 3]) {
    if let Some(bounds) = state.scene.scene_bounds {
        let r = bounds.radius;
        let c = bounds.center;
        (
            [c[0] - r, c[1] - r, c[2] - r],
            [c[0] + r, c[1] + r, c[2] + r],
        )
    } else {
        ([0.0, 0.0, 0.0], [0.0, 0.0, 0.0])
    }
}

fn push_scene_store_buffers<'a>(
    state: &'a RenderState,
    buffers: &mut Vec<RenderArtifactBufferRef<'a>>,
) {
    // Scene-store buffers are shared inputs for many representations. They are
    // listed before per-representation geometry so consumers can locate global
    // metadata such as atom colors and object ranges without knowing draw
    // order.
    let store = &state.scene.scene_store;
    push_optional_buffer(
        buffers,
        RenderArtifactBufferRole::FrameUniforms,
        Some(&state.ctx.frame.buffer),
        crate::FrameUniforms::SIZE,
        1,
    );
    push_optional_buffer(
        buffers,
        RenderArtifactBufferRole::SceneAtoms,
        store.atoms.buffer(),
        std::mem::size_of::<AtomGpu>() as u64,
        store.atoms.capacity_entries() as u64,
    );
    push_optional_buffer(
        buffers,
        RenderArtifactBufferRole::SceneCoords,
        store.coords.buffer(),
        16,
        store.coords.capacity_entries() as u64,
    );
    push_optional_buffer(
        buffers,
        RenderArtifactBufferRole::SceneBonds,
        store.bonds.buffer(),
        std::mem::size_of::<BondGpu>() as u64,
        store.bonds.capacity_entries() as u64,
    );
    push_optional_buffer(
        buffers,
        RenderArtifactBufferRole::SceneColorLut,
        store.color_lut.buffer(),
        std::mem::size_of::<ColorLutEntry>() as u64,
        store.color_lut.capacity_entries() as u64,
    );
    push_optional_buffer(
        buffers,
        RenderArtifactBufferRole::SceneMaskLut,
        store.mask_lut.buffer(),
        4,
        store.mask_lut.capacity_entries() as u64,
    );
    push_optional_buffer(
        buffers,
        RenderArtifactBufferRole::SceneMarkerLut,
        store.marker_lut.buffer(),
        4,
        store.marker_lut.capacity_entries() as u64,
    );
    push_optional_buffer(
        buffers,
        RenderArtifactBufferRole::SceneCsrOffsets,
        store.csr_offsets.buffer(),
        4,
        store.csr_offsets.capacity_entries() as u64,
    );
    push_optional_buffer(
        buffers,
        RenderArtifactBufferRole::SceneCsrIndices,
        store.csr_indices.buffer(),
        4,
        store.csr_indices.capacity_entries() as u64,
    );
    push_optional_buffer(
        buffers,
        RenderArtifactBufferRole::SceneObjectTable,
        store.obj_table_buffer(),
        ObjectEntry::STRIDE,
        store
            .obj_table_buffer()
            .map(|buffer| buffer.size() / ObjectEntry::STRIDE)
            .unwrap_or(0),
    );
}

struct InstanceArtifactSource<'a> {
    object_id: u32,
    rep_kind: RepKind,
    topology: RenderArtifactPrimitiveTopology,
    geometry_role: RenderArtifactBufferRole,
    stride: u64,
    exported: Option<(
        &'a wgpu::Buffer,
        Option<&'a wgpu::Buffer>,
        Option<&'a wgpu::Buffer>,
        u32,
    )>,
    metadata: ArtifactObjectMetadata,
}

struct InstanceArtifactSink<'a, 'b> {
    buffers: &'b mut Vec<RenderArtifactBufferRef<'a>>,
    reps: &'b mut Vec<RenderArtifactRep>,
}

fn push_instance_rep<'a>(source: InstanceArtifactSource<'a>, sink: InstanceArtifactSink<'a, '_>) {
    let Some((geometry, count, indirect, capacity)) = source.exported else {
        return;
    };
    // Instance representations keep renderer-native instance formats. Plugins
    // that need triangles must either understand the instance role or reject
    // the representation instead of assuming `StdVertex` packing.
    let geometry_index = push_buffer(
        sink.buffers,
        source.geometry_role,
        geometry,
        source.stride,
        u64::from(capacity),
    );
    let count_buffer_index = count.map(|buffer| {
        push_buffer(
            sink.buffers,
            RenderArtifactBufferRole::InstanceCount,
            buffer,
            4,
            1,
        )
    });
    let indirect_buffer_index = indirect.map(|buffer| {
        push_buffer(
            sink.buffers,
            RenderArtifactBufferRole::IndirectDraw,
            buffer,
            16,
            1,
        )
    });
    sink.reps.push(RenderArtifactRep {
        object_id: source.object_id,
        rep_kind: source.rep_kind,
        topology: source.topology,
        geometry_buffer_index: geometry_index,
        count_buffer_index,
        indirect_buffer_index,
        element_count: 0,
        max_element_count: u64::from(capacity),
        atom_offset: source.metadata.atom_offset,
        atom_count: source.metadata.atom_count,
        material_rgba: source.metadata.material_rgba,
        transparency: source.metadata.transparency_for(source.rep_kind),
    });
}

fn push_optional_buffer<'a>(
    buffers: &mut Vec<RenderArtifactBufferRef<'a>>,
    role: RenderArtifactBufferRole,
    buffer: Option<&'a wgpu::Buffer>,
    stride: u64,
    element_count: u64,
) -> Option<usize> {
    let buffer = buffer?;
    Some(push_buffer(buffers, role, buffer, stride, element_count))
}

fn push_buffer<'a>(
    buffers: &mut Vec<RenderArtifactBufferRef<'a>>,
    role: RenderArtifactBufferRole,
    buffer: &'a wgpu::Buffer,
    stride: u64,
    element_count: u64,
) -> usize {
    let index = buffers.len();
    buffers.push(RenderArtifactBufferRef::new(
        role,
        buffer,
        stride,
        element_count,
    ));
    index
}
