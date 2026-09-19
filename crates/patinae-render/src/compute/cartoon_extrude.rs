//! Run-aware cartoon extrusion compute pipeline.
//!
//! Inputs (uploaded by `CartoonRep::build`):
//!   - `ExtrudePoints[]`   storage : per-sample `(position, atom_idx,
//!                                  orientation)` produced by `tessellation`
//!   - `RunDescriptor[]`   storage : per-run `(car_type, sample range,
//!                                  vertex range, body_end, flags)`
//!   - `ExtrudeParams`     uniform : counts + tube profile `quality`
//!
//! Output: the backbone prefix of a shared `StdVertex[]` storage+vertex
//! buffer, exactly `backbone_vertex_count` entries, each one written by
//! exactly one thread. CPU-authored nucleotide geometry may follow the prefix.
//!
//! Dispatch: 1D, `(backbone_vertex_count + 63) / 64` workgroups of 64 threads.
//! Each thread looks up its run via a linear scan over `runs[]` (n_runs
//! is < 100 typically) and emits the right vertex per the per-CartoonType
//! emission rules.

use bytemuck::{Pod, Zeroable};

use crate::shader_source;

pub const WORKGROUP: u32 = 64;

/// Mirrors `ExtrudeParams` in `cartoon_extrude.wgsl`. 48 B (three 16 B chunks).
///
/// `quality` is the number of segments per tube profile (Loop / Oval). It
/// MUST match `geom.quality` used by `compute_run_vertex_layout` on the
/// CPU side — otherwise the per-vertex `local_vidx → (pair, k)` decode
/// diverges from the slot allocation and the ribbon develops holes.
///
/// The six cross-section fields (`helix_width` .. `arrow_tip_scale`) mirror
/// the values previously baked into `cartoon_extrude.wgsl` as WGSL `const`s.
/// They are populated by `CartoonRep::build` from `GeomSettings`, which
/// itself reads user-configurable `cartoon_oval_width` /
/// `cartoon_oval_length` / `cartoon_rect_width` / `cartoon_rect_length` /
/// `cartoon_loop_radius` / `cartoon_arrow_tip_scale` settings.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct ExtrudeParams {
    pub n_runs: u32,
    pub n_samples: u32,
    pub n_atoms: u32,
    pub quality: u32,
    /// Helix oval **narrow** half-axis (along the normal).
    pub helix_width: f32,
    /// Helix oval **wide** half-axis (along the binormal / helix axis).
    pub helix_height: f32,
    /// Sheet rectangle full thickness (halved to obtain the binormal
    /// half-extent in the shader).
    pub sheet_width: f32,
    /// Sheet rectangle wide half-extent (along the normal).
    pub sheet_height: f32,
    /// Loop / coil tube radius (also the uniform-tube radius in Ribbon
    /// mode; the host writes `ribbon_radius` into this slot for Ribbon).
    pub loop_radius: f32,
    /// Multiplier on `sheet_height` for the arrow-head barb width.
    pub arrow_tip_scale: f32,
    /// Exact writable backbone prefix length. The final rounded-up workgroup
    /// must not overwrite CPU-authored nucleotide vertices that follow it.
    pub backbone_vertex_count: u32,
    /// Padding to keep `size_of::<ExtrudeParams>()` a multiple of 16 B
    /// (WGSL uniform buffer requirement).
    pub _pad1: u32,
}

impl ExtrudeParams {
    pub const SIZE: u64 = std::mem::size_of::<Self>() as u64;
}

const _: () = assert!(std::mem::size_of::<ExtrudeParams>() == 48);

/// Mirrors `ExtrudePoint` in `cartoon_extrude.wgsl`. 32 B (vec3 +
/// atom_idx, vec3 + pad).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct ExtrudePointGpu {
    pub position: [f32; 3],
    pub atom_idx: u32,
    pub orientation: [f32; 3],
    pub _pad: u32,
}

impl ExtrudePointGpu {
    pub const SIZE: u64 = std::mem::size_of::<Self>() as u64;
}

const _: () = assert!(std::mem::size_of::<ExtrudePointGpu>() == 32);

pub struct CartoonExtrudeCompute {
    pub pipeline: wgpu::ComputePipeline,
    pub bind_group_layout: wgpu::BindGroupLayout,
}

impl CartoonExtrudeCompute {
    pub fn new(device: &wgpu::Device) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("patinae.cartoon_extrude.bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(ExtrudeParams::SIZE),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(ExtrudePointGpu::SIZE),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        // RunDescriptor = 32 B (8 × u32).
                        min_binding_size: wgpu::BufferSize::new(32),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        // StdVertex = 24 B.
                        min_binding_size: wgpu::BufferSize::new(24),
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("patinae.cartoon_extrude.pl"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("patinae.cartoon_extrude.wgsl"),
            source: wgpu::ShaderSource::Wgsl(
                shader_source::expand(shader_source::CARTOON_EXTRUDE_WGSL).into(),
            ),
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("patinae.cartoon_extrude.pipeline"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("cs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        Self {
            pipeline,
            bind_group_layout,
        }
    }

    pub fn make_bind_group(
        &self,
        device: &wgpu::Device,
        params_buf: &wgpu::Buffer,
        extrude_points_buf: &wgpu::Buffer,
        runs_buf: &wgpu::Buffer,
        vertex_buf: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("patinae.cartoon_extrude.bg"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: extrude_points_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: runs_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: vertex_buf.as_entire_binding(),
                },
            ],
        })
    }

    pub fn dispatch(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        bind_group: &wgpu::BindGroup,
        backbone_vertex_count: u32,
    ) {
        let groups = backbone_vertex_count.div_ceil(WORKGROUP);
        let (wg_x, wg_y) = super::split_1d_dispatch(groups);
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("patinae.cartoon_extrude.dispatch"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.dispatch_workgroups(wg_x, wg_y, 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::util::DeviceExt;

    #[test]
    #[ignore = "requires a real GPU; verifies extrusion cannot overwrite nucleotide suffix vertices"]
    fn gpu_rounded_dispatch_preserves_vertices_after_backbone_prefix() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
            .expect("GPU adapter required; do not silently skip");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default())).unwrap();
        let pipeline = CartoonExtrudeCompute::new(&device);
        let prefix = WORKGROUP + 1;
        let params = ExtrudeParams {
            backbone_vertex_count: prefix,
            ..ExtrudeParams::zeroed()
        };
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: &[0; 32],
            usage: wgpu::BufferUsages::STORAGE,
        });
        // With no runs, every valid prefix invocation writes a degenerate vertex.
        // Rounded lanes must leave the synthetic CPU-owned suffix untouched.
        let sentinel = vec![0x55u8; WORKGROUP as usize * 2 * 24];
        let output = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: &sentinel,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });
        let bind_group = pipeline.make_bind_group(&device, &uniform, &input, &input, &output);
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: sentinel.len() as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        pipeline.dispatch(&mut encoder, &bind_group, prefix);
        encoder.copy_buffer_to_buffer(&output, 0, &staging, 0, sentinel.len() as u64);
        queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .unwrap();
        rx.recv().unwrap().unwrap();
        let mapped = staging.slice(..).get_mapped_range();
        for vertex in mapped[..prefix as usize * 24].as_chunks::<24>().0 {
            assert_ne!(vertex, &[0x55; 24], "prefix lane did not execute");
        }
        assert_eq!(
            &mapped[prefix as usize * 24..],
            &sentinel[prefix as usize * 24..],
            "rounded dispatch corrupted CPU-authored suffix"
        );
    }
}
