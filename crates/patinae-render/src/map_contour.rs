//! GPU runtime for first-class map contour objects.

use bytemuck::{Pod, Zeroable};
use patinae_algos::surface::{
    extract_isomesh, extract_isosurface, ContourGeometry, ContourTopology,
};
use wgpu::util::DeviceExt;

use crate::memory::{buffer_usage, GpuMemoryUsage};
use crate::pipelines::map::{MapParams, MapParamsLayout};
use crate::render_input::{RenderMapInput, RenderMapMode};

/// GPU vertex payload for map contour geometry.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct MapVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
}

impl MapVertex {
    pub fn vertex_layout() -> wgpu::VertexBufferLayout<'static> {
        const ATTRS: [wgpu::VertexAttribute; 2] =
            wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<MapVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &ATTRS,
        }
    }
}

/// Cached GPU resources for one map contour object.
pub struct MapEntry {
    pub mode: RenderMapMode,
    pub vertex_buffer: Option<wgpu::Buffer>,
    pub index_buffer: Option<wgpu::Buffer>,
    pub index_count: u32,
    pub params_buffer: wgpu::Buffer,
    pub bind_group: wgpu::BindGroup,
    pub geometry_revision: u64,
    pub material_revision: u64,
    pub color: [f32; 4],
    pub transform: [[f32; 4]; 4],
    pub is_opaque: bool,
}

impl MapEntry {
    /// Creates a map entry and uploads its first geometry snapshot.
    pub fn new(
        input: &RenderMapInput<'_>,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &MapParamsLayout,
    ) -> Self {
        let params = MapParams {
            color: input.color,
            model: input.transform,
        };
        let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("patinae.map.params"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("patinae.map.params_bg"),
            layout: &layout.bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: params_buffer.as_entire_binding(),
            }],
        });
        let mut entry = Self {
            mode: input.mode,
            vertex_buffer: None,
            index_buffer: None,
            index_count: 0,
            params_buffer,
            bind_group,
            geometry_revision: 0,
            material_revision: 0,
            color: input.color,
            transform: input.transform,
            is_opaque: input.color[3] >= 0.999,
        };
        entry.sync(input, device, queue);
        entry
    }

    /// Synchronizes cached geometry and material state.
    pub fn sync(
        &mut self,
        input: &RenderMapInput<'_>,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> bool {
        let geometry_changed = self.geometry_revision != input.geometry_revision
            || self.mode != input.mode
            || input.dirty && self.index_count == 0;
        if geometry_changed {
            let geometry = match input.mode {
                RenderMapMode::Isomesh => extract_isomesh(input.grid, input.level),
                RenderMapMode::Isosurface => extract_isosurface(input.grid, input.level),
            };
            self.mode = input.mode;
            self.upload_geometry(&geometry, device);
            self.geometry_revision = input.geometry_revision;
        }

        let material_changed = self.material_revision != input.material_revision
            || self.color != input.color
            || self.transform != input.transform;
        if material_changed {
            self.color = input.color;
            self.transform = input.transform;
            self.material_revision = input.material_revision;
            self.is_opaque = input.color[3] >= 0.999;
            let params = MapParams {
                color: self.color,
                model: self.transform,
            };
            queue.write_buffer(&self.params_buffer, 0, bytemuck::bytes_of(&params));
        }

        geometry_changed || material_changed
    }

    /// Records this map contour into a render pass.
    pub fn record<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        if self.index_count == 0 {
            return;
        }
        let (Some(vertex_buffer), Some(index_buffer)) = (&self.vertex_buffer, &self.index_buffer)
        else {
            return;
        };
        pass.set_bind_group(2, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, vertex_buffer.slice(..));
        pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..self.index_count, 0, 0..1);
    }

    /// Estimated GPU bytes allocated by this map contour.
    pub fn memory_usage(&self) -> GpuMemoryUsage {
        let mut usage = GpuMemoryUsage::default();
        usage.add(buffer_usage(&self.params_buffer));
        if let Some(buffer) = self.vertex_buffer.as_ref() {
            usage.add(buffer_usage(buffer));
        }
        if let Some(buffer) = self.index_buffer.as_ref() {
            usage.add(buffer_usage(buffer));
        }
        usage
    }

    fn upload_geometry(&mut self, geometry: &ContourGeometry, device: &wgpu::Device) {
        self.index_count = geometry.indices.len() as u32;
        if geometry.is_empty() {
            self.vertex_buffer = None;
            self.index_buffer = None;
            return;
        }
        debug_assert!(matches!(
            (geometry.topology, self.mode),
            (ContourTopology::Lines, RenderMapMode::Isomesh)
                | (ContourTopology::Triangles, RenderMapMode::Isosurface)
        ));
        let vertices: Vec<MapVertex> = geometry
            .vertices
            .iter()
            .map(|vertex| MapVertex {
                position: vertex.position,
                normal: vertex.normal,
            })
            .collect();
        self.vertex_buffer = Some(
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("patinae.map.vertices"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
        );
        self.index_buffer = Some(
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("patinae.map.indices"),
                contents: bytemuck::cast_slice(&geometry.indices),
                usage: wgpu::BufferUsages::INDEX,
            }),
        );
    }
}
