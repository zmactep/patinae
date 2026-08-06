//! Compact selected-atom dots and recent-atom spheres.

use std::collections::{BTreeMap, HashSet};
use std::num::NonZeroU64;

use bytemuck::{Pod, Zeroable};
use patinae_mol::DirtyFlags;

use crate::context::RenderContext;
use crate::frame::DEPTH_FORMAT;
use crate::memory::{buffer_usage, GpuMemoryUsage};
use crate::memory_policy::{RenderMemoryPolicy, RenderMemoryProfile};
use crate::picking::ObjectId;
use crate::render_input::MarkerUpdate;
use crate::scene_store::marker::{MARKER_RECENT, MARKER_SELECTED};
use crate::scene_store::{ObjectSlot, SceneStore, SceneStoreLayout};
use crate::shader_source;

const MARKER_INSTANCE_MIN_CAPACITY: usize = 16;
const DOT_RADIUS_BASE_PX: f32 = 7.0;
const DOT_RADIUS_PER_MARKING_PX: f32 = 3.0;
const DOT_RADIUS_MIN_PX: f32 = 8.0;
const DOT_RADIUS_MAX_PX: f32 = 20.0;
// Push the dot center slightly toward the camera so selected atoms remain
// visible on top of their own molecular surface without becoming X-ray marks.
const DOT_VIEW_BIAS_ANGSTROM: f32 = 0.25;
// Keep the marker outside visible sphere reps without adding a bulky halo.
const RECENT_SPHERE_SHELL_ANGSTROM: f32 = 0.12;
const MARKER_KIND_SELECTED: u32 = 0;
const MARKER_KIND_RECENT: u32 = 1;

#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct AtomMarkerParams {
    radius_px: f32,
    view_bias: f32,
    recent_shell: f32,
    _pad0: f32,
}

impl AtomMarkerParams {
    const SIZE: u64 = std::mem::size_of::<Self>() as u64;
}

struct AtomMarkerObject {
    selected_indices: Vec<u32>,
    recent_indices: Vec<u32>,
    recent_radius_scale: f32,
    buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    capacity: usize,
}

impl AtomMarkerObject {
    fn marker_count(&self) -> u32 {
        (self.selected_indices.len() + self.recent_indices.len()) as u32
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
struct MarkerInstance {
    atom_index: u32,
    kind: u32,
    radius_scale: f32,
    _pad0: f32,
}

pub(crate) struct AtomMarkersPass {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    params_buffer: wgpu::Buffer,
    objects: BTreeMap<u32, AtomMarkerObject>,
}

impl AtomMarkersPass {
    pub(crate) fn new(ctx: &RenderContext, scene_layout: &SceneStoreLayout) -> Self {
        let device = &ctx.device;
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("patinae.atom_markers.bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(AtomMarkerParams::SIZE),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("patinae.atom_markers.params"),
            size: AtomMarkerParams::SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("patinae.atom_markers.shader"),
            source: wgpu::ShaderSource::Wgsl(
                shader_source::expand(shader_source::SELECTION_DOTS_WGSL).into(),
            ),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("patinae.atom_markers.pipeline_layout"),
            bind_group_layouts: &[
                Some(&ctx.frame.bind_group_layout),
                Some(&ctx.lighting.bind_group_layout),
                Some(&scene_layout.bind_group_layout),
                Some(&bind_group_layout),
            ],
            immediate_size: 0,
        });
        let color_targets = [Some(wgpu::ColorTargetState {
            format: ctx.color_format,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        })];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("patinae.atom_markers.pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &color_targets,
            }),
            multiview_mask: None,
            cache: None,
        });

        Self {
            pipeline,
            bind_group_layout,
            params_buffer,
            objects: BTreeMap::new(),
        }
    }

    pub(crate) fn upload_params(&self, queue: &wgpu::Queue, marking_width: f32) {
        let radius_px = selection_dot_radius_px(marking_width);
        queue.write_buffer(
            &self.params_buffer,
            0,
            bytemuck::bytes_of(&AtomMarkerParams {
                radius_px,
                view_bias: DOT_VIEW_BIAS_ANGSTROM,
                recent_shell: RECENT_SPHERE_SHELL_ANGSTROM,
                _pad0: 0.0,
            }),
        );
    }

    pub(crate) fn selected_indices(&self, object_id: u32) -> &[u32] {
        self.objects
            .get(&object_id)
            .map(|object| object.selected_indices.as_slice())
            .unwrap_or(&[])
    }

    pub(crate) fn recent_indices(&self, object_id: u32) -> &[u32] {
        self.objects
            .get(&object_id)
            .map(|object| object.recent_indices.as_slice())
            .unwrap_or(&[])
    }

    pub(crate) fn sync_object(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        object_id: u32,
        marker_bits: &[u32],
        include_selection: bool,
        recent_radius_scale: f32,
    ) -> bool {
        let (selected_indices, recent_indices) =
            collect_marker_indices(marker_bits, include_selection);
        self.sync_indices(
            device,
            queue,
            object_id,
            selected_indices,
            recent_indices,
            recent_radius_scale,
        )
    }

    pub(crate) fn sync_object_updates(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        object_id: u32,
        marker_updates: &[MarkerUpdate],
        include_selection: bool,
        recent_radius_scale: f32,
    ) -> bool {
        let (mut selected_indices, mut recent_indices) = self
            .objects
            .get(&object_id)
            .map(|object| {
                (
                    object.selected_indices.clone(),
                    object.recent_indices.clone(),
                )
            })
            .unwrap_or_default();
        for update in marker_updates {
            set_sorted_membership(
                &mut selected_indices,
                update.atom_index,
                include_selection && update.bits & MARKER_SELECTED != 0,
            );
            set_sorted_membership(
                &mut recent_indices,
                update.atom_index,
                update.bits & MARKER_RECENT != 0,
            );
        }
        self.sync_indices(
            device,
            queue,
            object_id,
            selected_indices,
            recent_indices,
            recent_radius_scale,
        )
    }

    fn sync_indices(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        object_id: u32,
        selected_indices: Vec<u32>,
        recent_indices: Vec<u32>,
        recent_radius_scale: f32,
    ) -> bool {
        let Some(existing) = self.objects.get_mut(&object_id) else {
            if selected_indices.is_empty() && recent_indices.is_empty() {
                return false;
            }
            let object = make_atom_markers_object(
                device,
                queue,
                &self.bind_group_layout,
                &self.params_buffer,
                &selected_indices,
                &recent_indices,
                recent_radius_scale,
            );
            self.objects.insert(object_id, object);
            return true;
        };

        if existing.selected_indices == selected_indices
            && existing.recent_indices == recent_indices
            && existing.recent_radius_scale == recent_radius_scale
        {
            return false;
        }
        if selected_indices.is_empty() && recent_indices.is_empty() {
            self.objects.remove(&object_id);
            return true;
        }

        let marker_count = selected_indices.len() + recent_indices.len();
        if marker_count > existing.capacity {
            *existing = make_atom_markers_object(
                device,
                queue,
                &self.bind_group_layout,
                &self.params_buffer,
                &selected_indices,
                &recent_indices,
                recent_radius_scale,
            );
        } else {
            let instances =
                marker_instances(&selected_indices, &recent_indices, recent_radius_scale);
            queue.write_buffer(&existing.buffer, 0, bytemuck::cast_slice(&instances));
            existing.selected_indices = selected_indices;
            existing.recent_indices = recent_indices;
            existing.recent_radius_scale = recent_radius_scale;
        }
        true
    }

    pub(crate) fn retain_objects<I>(&mut self, live_object_ids: I) -> bool
    where
        I: IntoIterator<Item = u32>,
    {
        if self.objects.is_empty() {
            return false;
        }
        let live: HashSet<u32> = live_object_ids.into_iter().collect();
        let before = self.objects.len();
        self.objects.retain(|object_id, _| live.contains(object_id));
        self.objects.len() != before
    }

    pub(crate) fn has_markers(&self) -> bool {
        !self.objects.is_empty()
    }

    pub(crate) fn memory_usage(&self) -> GpuMemoryUsage {
        let mut usage = buffer_usage(&self.params_buffer);
        for object in self.objects.values() {
            usage.add(buffer_usage(&object.buffer));
        }
        usage
    }

    pub(crate) fn record(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        frame_bind_group: &wgpu::BindGroup,
        lighting_bind_group: &wgpu::BindGroup,
        scene_store: &SceneStore,
    ) {
        if !self.has_markers() {
            return;
        }
        let Some(scene_bind_group) = scene_store.bind_group() else {
            return;
        };

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("patinae.atom_markers_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, frame_bind_group, &[]);
        pass.set_bind_group(1, lighting_bind_group, &[]);
        for (&object_id, object) in &self.objects {
            let Some(slot) = scene_store.slot(ObjectId(object_id)) else {
                continue;
            };
            pass.set_bind_group(2, scene_bind_group, &[slot.dynamic_offset()]);
            pass.set_bind_group(3, &object.bind_group, &[]);
            pass.draw(0..6, 0..object.marker_count());
        }
    }
}

fn make_atom_markers_object(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    bind_group_layout: &wgpu::BindGroupLayout,
    params_buffer: &wgpu::Buffer,
    selected_indices: &[u32],
    recent_indices: &[u32],
    recent_radius_scale: f32,
) -> AtomMarkerObject {
    let instances = marker_instances(selected_indices, recent_indices, recent_radius_scale);
    let capacity = instances
        .len()
        .next_power_of_two()
        .max(MARKER_INSTANCE_MIN_CAPACITY);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("patinae.atom_markers.instances"),
        size: (capacity * std::mem::size_of::<MarkerInstance>()) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&buffer, 0, bytemuck::cast_slice(&instances));
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("patinae.atom_markers.bg"),
        layout: bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: params_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: buffer.as_entire_binding(),
            },
        ],
    });
    AtomMarkerObject {
        selected_indices: selected_indices.to_vec(),
        recent_indices: recent_indices.to_vec(),
        recent_radius_scale,
        buffer,
        bind_group,
        capacity,
    }
}

pub(crate) fn uses_selection_dots_fallback(policy: RenderMemoryPolicy) -> bool {
    matches!(
        policy.profile,
        RenderMemoryProfile::Lite | RenderMemoryProfile::Manual { .. }
    ) && !policy.overlays.selection_enabled
}

pub(crate) fn should_rebuild_marker_indices(
    dirty: DirtyFlags,
    old_selected_indices: &[u32],
    old_recent_indices: &[u32],
    marker_updates: &[MarkerUpdate],
    include_selection: bool,
) -> bool {
    if dirty.intersects(DirtyFlags::TOPOLOGY | DirtyFlags::REPS | DirtyFlags::DRAW_MASK) {
        return true;
    }
    if !marker_updates.is_empty() {
        return marker_updates_change_visible(
            old_selected_indices,
            old_recent_indices,
            marker_updates,
            include_selection,
        );
    }
    dirty.contains(DirtyFlags::SELECTION)
}

pub(crate) fn object_marker_bits(marker_lut: &[u32], slot: ObjectSlot) -> &[u32] {
    let start = slot.atom_offset as usize;
    if start >= marker_lut.len() {
        return &[];
    }
    let end = start
        .saturating_add(slot.atom_count as usize)
        .min(marker_lut.len());
    &marker_lut[start..end]
}

fn collect_marker_indices(marker_bits: &[u32], include_selection: bool) -> (Vec<u32>, Vec<u32>) {
    let mut selected = Vec::new();
    let mut recent = Vec::new();
    for (index, bits) in marker_bits.iter().enumerate() {
        let index = index as u32;
        if include_selection && bits & MARKER_SELECTED != 0 {
            selected.push(index);
        }
        if bits & MARKER_RECENT != 0 {
            recent.push(index);
        }
    }
    (selected, recent)
}

fn marker_instances(
    selected_indices: &[u32],
    recent_indices: &[u32],
    recent_radius_scale: f32,
) -> Vec<MarkerInstance> {
    let mut instances = Vec::with_capacity(selected_indices.len() + recent_indices.len());
    instances.extend(selected_indices.iter().map(|&atom_index| MarkerInstance {
        atom_index,
        kind: MARKER_KIND_SELECTED,
        radius_scale: 1.0,
        _pad0: 0.0,
    }));
    instances.extend(recent_indices.iter().map(|&atom_index| MarkerInstance {
        atom_index,
        kind: MARKER_KIND_RECENT,
        radius_scale: recent_radius_scale,
        _pad0: 0.0,
    }));
    instances
}

fn set_sorted_membership(indices: &mut Vec<u32>, atom_index: u32, present: bool) {
    match (indices.binary_search(&atom_index), present) {
        (Err(position), true) => indices.insert(position, atom_index),
        (Ok(position), false) => {
            indices.remove(position);
        }
        _ => {}
    }
}

fn marker_updates_change_visible(
    old_selected_indices: &[u32],
    old_recent_indices: &[u32],
    marker_updates: &[MarkerUpdate],
    include_selection: bool,
) -> bool {
    marker_updates.iter().any(|update| {
        let was_selected = old_selected_indices
            .binary_search(&update.atom_index)
            .is_ok();
        let is_selected = include_selection && update.bits & MARKER_SELECTED != 0;
        let was_recent = old_recent_indices.binary_search(&update.atom_index).is_ok();
        let is_recent = update.bits & MARKER_RECENT != 0;
        was_selected != is_selected || was_recent != is_recent
    })
}

fn selection_dot_radius_px(marking_width: f32) -> f32 {
    (marking_width.clamp(0.5, 20.0) * DOT_RADIUS_PER_MARKING_PX + DOT_RADIUS_BASE_PX)
        .clamp(DOT_RADIUS_MIN_PX, DOT_RADIUS_MAX_PX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::byte_units::mib_to_bytes;
    use crate::memory_policy::RenderMemoryPolicy;
    use crate::scene_store::marker::MARKER_HOVER;

    #[test]
    fn selected_indices_ignore_hover_bits() {
        let (selected, recent) = collect_marker_indices(
            &[
                0,
                MARKER_SELECTED,
                MARKER_HOVER,
                MARKER_SELECTED | MARKER_HOVER,
            ],
            true,
        );
        assert_eq!(selected, [1, 3]);
        assert!(recent.is_empty());
    }

    #[test]
    fn recent_indices_are_kept_when_selection_dots_are_disabled() {
        let marker_bits = [
            MARKER_SELECTED,
            MARKER_RECENT,
            MARKER_SELECTED | MARKER_RECENT,
            MARKER_HOVER,
        ];

        let (selected, recent) = collect_marker_indices(&marker_bits, false);
        assert!(selected.is_empty());
        assert_eq!(recent, [1, 2]);

        let (selected, recent) = collect_marker_indices(&marker_bits, true);
        assert_eq!(selected, [0, 2]);
        assert_eq!(recent, [1, 2]);
    }

    #[test]
    fn sparse_marker_membership_stays_sorted_and_deduplicated() {
        let mut indices = vec![1, 4];

        set_sorted_membership(&mut indices, 3, true);
        set_sorted_membership(&mut indices, 1, true);
        set_sorted_membership(&mut indices, 4, false);
        set_sorted_membership(&mut indices, 9, false);

        assert_eq!(indices, [1, 3]);
    }

    #[test]
    fn recent_instances_carry_resolved_sphere_radius_scale() {
        let instances = marker_instances(&[1], &[2], 3.5);

        assert_eq!(instances[0].kind, MARKER_KIND_SELECTED);
        assert_eq!(instances[0].radius_scale, 1.0);
        assert_eq!(instances[1].kind, MARKER_KIND_RECENT);
        assert_eq!(instances[1].radius_scale, 3.5);
        assert_eq!(std::mem::size_of::<MarkerInstance>(), 16);
    }

    #[test]
    fn hover_only_dirty_reuses_selected_indices_when_selection_bits_stay_put() {
        let old_selected = [1, 3];
        let updates = [
            MarkerUpdate {
                atom_index: 0,
                bits: MARKER_HOVER,
            },
            MarkerUpdate {
                atom_index: 1,
                bits: MARKER_SELECTED | MARKER_HOVER,
            },
        ];

        assert!(!should_rebuild_marker_indices(
            DirtyFlags::HOVER,
            &old_selected,
            &[],
            &updates,
            true,
        ));
    }

    #[test]
    fn hover_dirty_rebuilds_if_sparse_update_changes_selection_bit() {
        let add_selected = [MarkerUpdate {
            atom_index: 2,
            bits: MARKER_SELECTED | MARKER_HOVER,
        }];
        let clear_selected = [MarkerUpdate {
            atom_index: 3,
            bits: MARKER_HOVER,
        }];

        assert!(should_rebuild_marker_indices(
            DirtyFlags::HOVER,
            &[],
            &[],
            &add_selected,
            true,
        ));
        assert!(should_rebuild_marker_indices(
            DirtyFlags::HOVER,
            &[3],
            &[],
            &clear_selected,
            true,
        ));
    }

    #[test]
    fn selection_and_topology_dirty_rebuild_selected_indices() {
        assert!(should_rebuild_marker_indices(
            DirtyFlags::SELECTION,
            &[],
            &[],
            &[],
            true,
        ));
        assert!(should_rebuild_marker_indices(
            DirtyFlags::TOPOLOGY,
            &[],
            &[],
            &[],
            true,
        ));
        assert!(should_rebuild_marker_indices(
            DirtyFlags::REPS | DirtyFlags::DRAW_MASK,
            &[],
            &[],
            &[],
            true,
        ));
        assert!(!should_rebuild_marker_indices(
            DirtyFlags::COLOR,
            &[],
            &[],
            &[],
            true,
        ));
        assert!(!should_rebuild_marker_indices(
            DirtyFlags::empty(),
            &[],
            &[],
            &[],
            true,
        ));
    }

    #[test]
    fn object_marker_slice_clamps_to_marker_lut_capacity() {
        let slot = ObjectSlot {
            atom_offset: 2,
            atom_count: 4,
            bond_offset: 0,
            bond_count: 0,
            table_index: 0,
        };

        assert_eq!(object_marker_bits(&[0, 0, 1, 2], slot), &[1, 2]);
    }

    #[test]
    fn selection_dots_fallback_is_only_lite_and_manual() {
        assert!(!uses_selection_dots_fallback(
            RenderMemoryPolicy::performance()
        ));
        assert!(!uses_selection_dots_fallback(RenderMemoryPolicy::balanced()));
        assert!(uses_selection_dots_fallback(RenderMemoryPolicy::lite()));
        assert!(uses_selection_dots_fallback(RenderMemoryPolicy::manual(
            mib_to_bytes(2048)
        )));
    }

    #[test]
    fn default_selection_dot_radius_is_readable() {
        assert_eq!(selection_dot_radius_px(1.0), 10.0);
        assert!(selection_dot_radius_px(0.0) >= DOT_RADIUS_MIN_PX);
        assert_eq!(selection_dot_radius_px(20.0), DOT_RADIUS_MAX_PX);
    }
}
