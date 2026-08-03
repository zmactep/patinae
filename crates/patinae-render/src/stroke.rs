//! GPU runtime for reusable object-owned annotation strokes.

use wgpu::util::DeviceExt;

use crate::memory::{buffer_usage, GpuMemoryUsage};
use crate::render_input::RenderStrokeInput;

/// Cached GPU resources for one semantic stroke owner.
pub struct StrokeEntry {
    segment_buffer: wgpu::Buffer,
    segment_count: u32,
    geometry_revision: u64,
    material_revision: u64,
}

impl StrokeEntry {
    /// Creates and uploads one non-empty stroke owner.
    pub fn new(input: &RenderStrokeInput<'_>, device: &wgpu::Device) -> Self {
        debug_assert!(!input.segments.is_empty());
        let segment_buffer = create_segment_buffer(device, input);
        Self {
            segment_buffer,
            segment_count: input.segments.len() as u32,
            geometry_revision: input.geometry_revision,
            material_revision: input.material_revision,
        }
    }

    /// Synchronizes cached geometry and per-segment material resources.
    pub fn sync(
        &mut self,
        input: &RenderStrokeInput<'_>,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> bool {
        let changed = needs_upload(
            self.geometry_revision,
            self.material_revision,
            self.segment_count,
            input,
        );
        if changed {
            if self.segment_count == input.segments.len() as u32 {
                queue.write_buffer(
                    &self.segment_buffer,
                    0,
                    bytemuck::cast_slice(input.segments),
                );
            } else {
                self.segment_buffer = create_segment_buffer(device, input);
            }
            self.segment_count = input.segments.len() as u32;
            self.geometry_revision = input.geometry_revision;
            self.material_revision = input.material_revision;
        }
        changed
    }

    /// Records screen-space stroke instances.
    pub fn record<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        pass.set_vertex_buffer(0, self.segment_buffer.slice(..));
        pass.draw(0..6, 0..self.segment_count);
    }

    /// Estimates persistent GPU memory owned by this entry.
    pub fn memory_usage(&self) -> GpuMemoryUsage {
        buffer_usage(&self.segment_buffer)
    }
}

fn create_segment_buffer(device: &wgpu::Device, input: &RenderStrokeInput<'_>) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("patinae.stroke.segments"),
        contents: bytemuck::cast_slice(input.segments),
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
    })
}

fn needs_upload(
    geometry_revision: u64,
    material_revision: u64,
    segment_count: u32,
    input: &RenderStrokeInput<'_>,
) -> bool {
    geometry_revision != input.geometry_revision
        || material_revision != input.material_revision
        || segment_count != input.segments.len() as u32
}

#[cfg(test)]
mod tests {
    use super::needs_upload;
    use crate::{ObjectId, RenderStrokeInput, StrokeSegment};

    #[test]
    fn upload_decision_tracks_geometry_material_and_segment_count() {
        let segments = [StrokeSegment::new(
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 1.0, 1.0],
            2.0,
            true,
        )];
        let input = RenderStrokeInput {
            object_id: ObjectId(7),
            segments: &segments,
            bounds: Some(([0.0; 3], [1.0, 0.0, 0.0])),
            geometry_revision: 11,
            material_revision: 13,
        };

        assert!(!needs_upload(11, 13, 1, &input));
        assert!(needs_upload(10, 13, 1, &input));
        assert!(needs_upload(11, 12, 1, &input));
        assert!(needs_upload(11, 13, 0, &input));
    }
}
