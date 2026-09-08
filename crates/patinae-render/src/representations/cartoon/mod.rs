//! Cartoon representation — full GPU rewrite of the orient + extrude flow.
//!
//! ## Architecture
//!
//!   1. CPU `extract_retained_backbone` extracts retained backbone samples
//!      and initial orientation from the molecule (per `BackboneAtom`).
//!   2. CPU `tessellation::process_segment` runs the orient pipeline
//!      (round_helix → refine_normals → flatten_sheets) +
//!      `cartoon_generate_sample` + run classification → outputs
//!      `(ExtrudePoint[], RunDescriptor[])`.
//!   3. Host uploads ExtrudePoints + RunDescriptors to GPU storage buffers.
//!   4. GPU `cartoon_extrude.wgsl` reads both, branches on
//!      `RunDescriptor.car_type`, emits per-CartoonType geometry into the
//!      vertex-buffer prefix. For cartoons, the CPU appends connected
//!      nucleotide rung + plate geometry to the same buffer.
//!   5. Host issues one `draw_indirect` across the combined vertex range.
//!
//! Backbone vertex emission is on GPU. Orient + sample interpolation and the
//! small fixed-size nucleotide accents run on CPU because both are
//! intrinsically residue-local or sequential and small-N (< few hundred
//! residues per typical structure). GPU dispatch remains focused on the
//! high-volume extrusion work.
//!
//! ## SceneStore Integration
//!
//! - Group 2 = `SceneStoreLayout` (scene-wide LUTs, dyn-offset uniform).
//! - Group 3 = `CartoonParams` (per-rep `alpha_mul`).
//! - `CartoonExtrudeCompute` lives on `RenderState` (one pipeline shared
//!   across all CartoonReps); the rep stores buffers + bind group only.
//! - Compute dispatch is unified through `RenderState::dispatch_pending_compute_builds`
//!   like the other reps — `build()` does CPU prep + buffer realloc /
//!   upload and sets `needs_dispatch = true`; the shared encoder records
//!   the actual `cs_main` dispatch.

pub mod backbone;
pub mod frame;
mod nucleic;
pub mod tessellation;
pub mod utils;

use bytemuck::{Pod, Zeroable};
use patinae_settings::{groups::RibbonSettings, ResolvedSettings};
use rayon::prelude::*;
use wgpu::util::DeviceExt;

use crate::compute::cartoon_extrude::{ExtrudeParams, ExtrudePointGpu};
use crate::memory::{buffer_usage, GpuMemoryUsage};
use crate::picking::RepKind;
use crate::pipelines::cartoon::{CartoonParams, CartoonParamsLayout};
use crate::render_input::RenderObjectInput;
use crate::representation_budget::{RepMemoryEstimate, RepQualityLevel};
use crate::representations::cartoon::backbone::{extract_filtered_backbone, BackboneAtom};
use crate::representations::cartoon::nucleic::{
    count_nucleic_residues, prepare_nucleic_geometry, MAX_VERTICES_PER_NUCLEOTIDE,
};
use crate::representations::cartoon::tessellation::{
    from_resolved_settings, process_segment, segments_from_backbone_atoms, BackboneSegment,
    ExtrudePoint, GeomSettings, PipelineOutput, PipelineSettings, RunDescriptor,
};
use crate::representations::mesh::StdVertex;
use crate::representations::{BuildCtx, Representation};
use patinae_mol::DirtyFlags;

use crate::byte_units::bytes_to_gib;

const LARGE_CARTOON_BACKBONE_THRESHOLD: usize = 16_384;
const LARGE_CARTOON_SEGMENT_THRESHOLD: usize = 64;

fn cartoon_storage_buffer_limit(limits: wgpu::Limits) -> u64 {
    limits
        .max_buffer_size
        .min(limits.max_storage_buffer_binding_size)
}

fn oversized_cartoon_storage_buffer(
    limit: u64,
    buffers: &[(&'static str, u64)],
) -> Option<(&'static str, u64)> {
    buffers.iter().copied().find(|(_, bytes)| *bytes > limit)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CartoonEstimateBufferBytes {
    extrude_points: u64,
    runs: u64,
    vertices: u64,
    capacity: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CartoonVertexLayout {
    backbone_vertex_count: u32,
    total_vertex_count: u32,
    nucleic_byte_offset: u64,
    total_vertex_bytes: u64,
}

fn cartoon_vertex_layout(
    backbone_vertex_count: u32,
    nucleic_vertex_count: usize,
) -> Option<CartoonVertexLayout> {
    let nucleic_vertex_count = u32::try_from(nucleic_vertex_count).ok()?;
    let total_vertex_count = backbone_vertex_count.checked_add(nucleic_vertex_count)?;
    Some(CartoonVertexLayout {
        backbone_vertex_count,
        total_vertex_count,
        nucleic_byte_offset: u64::from(backbone_vertex_count) * StdVertex::SIZE,
        total_vertex_bytes: u64::from(total_vertex_count) * StdVertex::SIZE,
    })
}

fn cartoon_estimate_buffer_bytes(
    backbone_len: usize,
    nucleic_residue_count: usize,
    sampling_rate: u32,
    quality_factor: u32,
) -> CartoonEstimateBufferBytes {
    if backbone_len < 2 {
        return CartoonEstimateBufferBytes {
            extrude_points: 0,
            runs: 0,
            vertices: 0,
            capacity: CartoonParams::SIZE,
        };
    }

    let sample_count = (backbone_len as u64).saturating_mul(u64::from(sampling_rate.max(1)));
    let quality = u64::from(quality_factor.max(4));
    let backbone_vertex_count = sample_count.saturating_mul(quality).saturating_mul(6);
    let nucleic_vertex_count =
        (nucleic_residue_count as u64).saturating_mul(MAX_VERTICES_PER_NUCLEOTIDE);
    let vertex_count = backbone_vertex_count.saturating_add(nucleic_vertex_count);
    let extrude_points = sample_count.saturating_mul(ExtrudePointGpu::SIZE);
    let runs = sample_count.saturating_mul(std::mem::size_of::<RunDescriptor>() as u64);
    let vertices = vertex_count.saturating_mul(StdVertex::SIZE);
    let capacity = CartoonParams::SIZE
        .saturating_add(ExtrudeParams::SIZE)
        .saturating_add(extrude_points)
        .saturating_add(runs)
        .saturating_add(vertices.max(24))
        .saturating_add(16);

    CartoonEstimateBufferBytes {
        extrude_points,
        runs,
        vertices,
        capacity,
    }
}

/// Lazy-allocated GPU resources for one rebuild. Buffers are reused
/// across rebuilds when capacity permits.
struct CartoonResources {
    /// Vertex count emitted by compute before the CPU nucleotide tail.
    backbone_vertex_count: u32,
    params_buf: wgpu::Buffer,
    extrude_points_buf: wgpu::Buffer,
    runs_buf: wgpu::Buffer,
    vertex_buf: wgpu::Buffer,
    indirect_buf: wgpu::Buffer,
    extrude_bg: wgpu::BindGroup,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct BuildSignature {
    n_atoms: u32,
    /// Hash of CPU geometry inputs. Recolour / transparency-only changes do NOT bump this.
    geom_hash: u64,
    coord_hash: u64,
    nucleic_hash: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CartoonMode {
    /// SS-aware profiles. Reads cartoon visibility in WGSL and
    /// `settings.cartoon.*`.
    Cartoon,
    /// Uniform circular tube. Reads ribbon visibility in WGSL and
    /// `settings.ribbon.*`. Forces `GeomSettings.uniform_tube = true` so every
    /// pair becomes CartoonType::Loop.
    Ribbon,
}

impl CartoonMode {
    fn rep_kind(self) -> RepKind {
        match self {
            CartoonMode::Cartoon => RepKind::Cartoon,
            CartoonMode::Ribbon => RepKind::Ribbon,
        }
    }
}

fn nucleic_geometry_enabled(mode: CartoonMode, settings: &ResolvedSettings) -> bool {
    mode == CartoonMode::Cartoon && settings.cartoon.nucleic_ladder
}

pub struct CartoonRep {
    mode: CartoonMode,
    resources: Option<CartoonResources>,
    last_build: Option<BuildSignature>,
    vertex_count: u32,
    /// Per-rep render params (group 3 binding 0).
    render_params_buffer: wgpu::Buffer,
    render_params_bind_group: Option<wgpu::BindGroup>,
    /// `true` ⇒ next `dispatch_compute_build` will record the cs_main
    /// dispatch. Set by `build()` after geometry buffers are uploaded.
    needs_dispatch: bool,
    is_opaque_cache: bool,
}

impl CartoonRep {
    pub fn new(device: &wgpu::Device) -> Self {
        Self::with_mode(device, CartoonMode::Cartoon)
    }
    pub fn new_ribbon(device: &wgpu::Device) -> Self {
        Self::with_mode(device, CartoonMode::Ribbon)
    }
    fn with_mode(device: &wgpu::Device, mode: CartoonMode) -> Self {
        let render_params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("patinae.cartoon.render_params"),
            size: CartoonParams::SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            mode,
            resources: None,
            last_build: None,
            vertex_count: 0,
            render_params_buffer,
            render_params_bind_group: None,
            needs_dispatch: false,
            is_opaque_cache: true,
        }
    }

    pub fn ensure_render_bind_group(
        &mut self,
        device: &wgpu::Device,
        layout: &CartoonParamsLayout,
    ) {
        if self.render_params_bind_group.is_some() {
            return;
        }
        self.render_params_bind_group =
            Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("patinae.cartoon.render_bg"),
                layout: &layout.bind_group_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.render_params_buffer.as_entire_binding(),
                }],
            }));
    }

    pub(crate) fn export_vertices(&self) -> Option<(&wgpu::Buffer, u32)> {
        let resources = self.resources.as_ref()?;
        (self.vertex_count > 0).then_some((&resources.vertex_buf, self.vertex_count))
    }
}

fn apply_ribbon_pipeline_settings(pipeline: &mut PipelineSettings, ribbon: &RibbonSettings) {
    if ribbon.sampling > 0 {
        pipeline.sampling = (ribbon.sampling as u32).max(1);
    }
    pipeline.power_a = ribbon.power.max(0.0);
    pipeline.power_b = ribbon.power_b.max(0.0);
    pipeline.throw_factor = ribbon.throw.max(0.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::byte_units::{gib_to_bytes, mib_to_bytes};
    use crate::render_input::SceneLod;
    use crate::representations::cartoon::tessellation::GuidePoint;
    use lin_alg::f32::Vec3;
    use patinae_mol::SecondaryStructure;
    use patinae_settings::{Color, Settings};

    fn geometry_hash_for(settings: &Settings, mode: CartoonMode) -> u64 {
        let resolved = ResolvedSettings::resolve(settings, None);
        let (mut pipeline, mut geom) = from_resolved_settings(&resolved, SceneLod::High);
        if let CartoonMode::Ribbon = mode {
            apply_ribbon_pipeline_settings(&mut pipeline, &resolved.ribbon);
            geom.uniform_tube = true;
            geom.loop_radius = if resolved.ribbon.radius > 0.0 {
                resolved.ribbon.radius
            } else {
                0.3
            };
        }
        hash_cartoon_geometry(mode, &pipeline, &geom)
    }

    #[test]
    fn integrated_nucleotide_geometry_is_enabled_by_default() {
        assert!(Settings::default().cartoon.nucleic_ladder);
    }

    #[test]
    fn cartoon_mode_enables_requested_nucleotide_geometry() {
        let resolved = ResolvedSettings::resolve(&Settings::default(), None);

        assert!(nucleic_geometry_enabled(CartoonMode::Cartoon, &resolved));
    }

    #[test]
    fn cartoon_mode_honors_disabled_nucleotide_geometry() {
        let mut settings = Settings::default();
        settings.cartoon.nucleic_ladder = false;
        let resolved = ResolvedSettings::resolve(&settings, None);

        assert!(!nucleic_geometry_enabled(CartoonMode::Cartoon, &resolved));
    }

    #[test]
    fn ribbon_mode_never_enables_nucleotide_geometry() {
        let resolved = ResolvedSettings::resolve(&Settings::default(), None);

        assert!(!nucleic_geometry_enabled(CartoonMode::Ribbon, &resolved));
    }

    fn synthetic_segment(segment_id: u32, len: usize) -> BackboneSegment {
        let guide_points = (0..len)
            .map(|i| {
                let x = i as f32;
                let y = (segment_id as f32 * 0.25) + (x * 0.15).sin();
                GuidePoint {
                    position: Vec3::new(x, y, segment_id as f32),
                    orientation: Vec3::new(0.0, 1.0, 0.0),
                    ss_type: match i % 3 {
                        0 => SecondaryStructure::Helix,
                        1 => SecondaryStructure::Sheet,
                        _ => SecondaryStructure::Loop,
                    },
                    atom_idx: segment_id * 1000 + i as u32,
                }
            })
            .collect();
        BackboneSegment { guide_points }
    }

    fn assert_pipeline_outputs_match(left: &PipelineOutput, right: &PipelineOutput) {
        assert_eq!(left.total_vertices, right.total_vertices);
        assert_eq!(left.runs.len(), right.runs.len());
        assert_eq!(left.extrude_points.len(), right.extrude_points.len());

        for (a, b) in left.runs.iter().zip(&right.runs) {
            assert_eq!(a.car_type, b.car_type);
            assert_eq!(a.sample_start, b.sample_start);
            assert_eq!(a.sample_end, b.sample_end);
            assert_eq!(a.vertex_offset, b.vertex_offset);
            assert_eq!(a.vertex_count, b.vertex_count);
            assert_eq!(a.body_end, b.body_end);
            assert_eq!(a.flags, b.flags);
        }

        for (a, b) in left.extrude_points.iter().zip(&right.extrude_points) {
            assert_eq!(a.atom_idx, b.atom_idx);
            assert_eq!(a.position, b.position);
            assert_eq!(a.orientation, b.orientation);
        }
    }

    #[test]
    fn geometry_hash_changes_when_smooth_loops_changes() {
        let base = Settings::default();
        let mut changed = Settings::default();
        changed.cartoon.smooth_loops = true;

        assert_ne!(
            geometry_hash_for(&base, CartoonMode::Cartoon),
            geometry_hash_for(&changed, CartoonMode::Cartoon)
        );
    }

    #[test]
    fn parallel_cartoon_segment_prep_matches_serial_output() {
        let resolved = ResolvedSettings::resolve(&Settings::default(), None);
        let (pipeline, geom) = from_resolved_settings(&resolved, SceneLod::High);
        let segments: Vec<_> = (0..80).map(|id| synthetic_segment(id, 8)).collect();

        assert!(should_parallelize_cartoon_prep(
            8 * segments.len(),
            segments.len()
        ));
        let serial = process_cartoon_segments_serial(segments.clone(), &pipeline, &geom);
        let parallel = process_cartoon_segments_parallel(segments, &pipeline, &geom);

        assert_pipeline_outputs_match(&serial, &parallel);
    }

    #[test]
    fn geometry_hash_changes_when_cartoon_geometry_settings_change() {
        let base_hash = geometry_hash_for(&Settings::default(), CartoonMode::Cartoon);

        let mut changed = Settings::default();
        changed.cartoon.smooth_cycles += 1;
        assert_ne!(base_hash, geometry_hash_for(&changed, CartoonMode::Cartoon));

        let mut changed = Settings::default();
        changed.cartoon.sampling = 3;
        assert_ne!(base_hash, geometry_hash_for(&changed, CartoonMode::Cartoon));

        let mut changed = Settings::default();
        changed.cartoon.power = 3.0;
        assert_ne!(base_hash, geometry_hash_for(&changed, CartoonMode::Cartoon));
    }

    #[test]
    fn geometry_hash_ignores_cartoon_color() {
        let base = Settings::default();
        let mut changed = Settings::default();
        changed.cartoon.color = Color(7);

        assert_eq!(
            geometry_hash_for(&base, CartoonMode::Cartoon),
            geometry_hash_for(&changed, CartoonMode::Cartoon)
        );
    }

    #[test]
    fn ribbon_geometry_hash_changes_when_ribbon_pipeline_settings_change() {
        let base_hash = geometry_hash_for(&Settings::default(), CartoonMode::Ribbon);

        let mut changed = Settings::default();
        changed.ribbon.sampling += 1;
        assert_ne!(base_hash, geometry_hash_for(&changed, CartoonMode::Ribbon));

        let mut changed = Settings::default();
        changed.ribbon.power = 3.0;
        assert_ne!(base_hash, geometry_hash_for(&changed, CartoonMode::Ribbon));

        let mut changed = Settings::default();
        changed.ribbon.power_b = 0.9;
        assert_ne!(base_hash, geometry_hash_for(&changed, CartoonMode::Ribbon));

        let mut changed = Settings::default();
        changed.ribbon.throw = 2.0;
        assert_ne!(base_hash, geometry_hash_for(&changed, CartoonMode::Ribbon));
    }

    #[test]
    fn ribbon_geometry_hash_ignores_ribbon_color() {
        let base = Settings::default();
        let mut changed = Settings::default();
        changed.ribbon.color = Color(7);

        assert_eq!(
            geometry_hash_for(&base, CartoonMode::Ribbon),
            geometry_hash_for(&changed, CartoonMode::Ribbon)
        );
    }

    #[test]
    fn cartoon_storage_buffer_limit_uses_lower_device_limit() {
        let limits = wgpu::Limits {
            max_buffer_size: gib_to_bytes(2),
            max_storage_buffer_binding_size: mib_to_bytes(512),
            ..wgpu::Limits::default()
        };

        assert_eq!(cartoon_storage_buffer_limit(limits), mib_to_bytes(512));
    }

    #[test]
    fn oversized_cartoon_storage_buffer_reports_first_excess() {
        let buffers = [("extrude_points", 64), ("runs", 129), ("vertices", 256)];

        assert_eq!(
            oversized_cartoon_storage_buffer(128, &buffers),
            Some(("runs", 129))
        );
        assert_eq!(oversized_cartoon_storage_buffer(256, &buffers), None);
    }

    #[test]
    fn cartoon_estimate_buffer_bytes_is_monotonic() {
        let base = cartoon_estimate_buffer_bytes(100, 10, 7, 32);
        assert!(cartoon_estimate_buffer_bytes(101, 10, 7, 32).capacity >= base.capacity);
        assert!(cartoon_estimate_buffer_bytes(100, 11, 7, 32).capacity >= base.capacity);
        assert!(cartoon_estimate_buffer_bytes(100, 10, 8, 32).capacity >= base.capacity);
        assert!(cartoon_estimate_buffer_bytes(100, 10, 7, 33).capacity >= base.capacity);
    }

    #[test]
    fn cartoon_estimate_buffer_bytes_bounds_pipeline_output() {
        let resolved = ResolvedSettings::resolve(&Settings::default(), None);
        let (pipeline, geom) = from_resolved_settings(&resolved, SceneLod::High);
        let segments: Vec<_> = (0..4).map(|id| synthetic_segment(id, 12)).collect();
        let backbone_len = segments.iter().map(BackboneSegment::len).sum::<usize>();
        let output = process_cartoon_segments(segments, backbone_len, &pipeline, &geom);
        let estimate =
            cartoon_estimate_buffer_bytes(backbone_len, 0, pipeline.sampling, geom.quality);

        assert!(
            estimate.extrude_points >= output.extrude_points.len() as u64 * ExtrudePointGpu::SIZE
        );
        assert!(
            estimate.runs >= output.runs.len() as u64 * std::mem::size_of::<RunDescriptor>() as u64
        );
        assert!(estimate.vertices >= output.total_vertices as u64 * StdVertex::SIZE);
    }

    #[test]
    fn cartoon_estimate_includes_integrated_nucleotide_geometry() {
        let without_nucleotides = cartoon_estimate_buffer_bytes(100, 0, 7, 32);
        let with_nucleotides = cartoon_estimate_buffer_bytes(100, 3, 7, 32);

        assert_eq!(
            with_nucleotides.vertices - without_nucleotides.vertices,
            3 * MAX_VERTICES_PER_NUCLEOTIDE * StdVertex::SIZE
        );
    }

    #[test]
    fn cartoon_vertex_layout_keeps_compute_prefix_separate_from_draw_total() {
        let layout = cartoon_vertex_layout(600, MAX_VERTICES_PER_NUCLEOTIDE as usize)
            .expect("representable layout");

        assert_eq!(layout.backbone_vertex_count, 600);
        assert_eq!(
            layout.total_vertex_count,
            600 + MAX_VERTICES_PER_NUCLEOTIDE as u32
        );
        assert_eq!(layout.nucleic_byte_offset, 600 * StdVertex::SIZE);
        assert_eq!(
            layout.total_vertex_bytes,
            u64::from(layout.total_vertex_count) * StdVertex::SIZE
        );
    }
}

fn hash_backbone(bb: &[BackboneAtom]) -> u64 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for b in bb {
        h.write_u32(b.position[0].to_bits());
        h.write_u32(b.position[1].to_bits());
        h.write_u32(b.position[2].to_bits());
        h.write_u32(b.atom_id);
        h.write_u32(b.flags);
    }
    h.finish()
}

fn hash_cartoon_geometry(
    mode: CartoonMode,
    pipeline: &PipelineSettings,
    geom: &GeomSettings,
) -> u64 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();

    h.write_u32(mode as u32);
    h.write_u32(pipeline.sampling);
    h.write_u32(pipeline.power_a.to_bits());
    h.write_u32(pipeline.power_b.to_bits());
    h.write_u32(pipeline.throw_factor.to_bits());
    h.write_u32(pipeline.flat_cycles);
    h.write_u32(pipeline.smooth_first);
    h.write_u32(pipeline.smooth_last);
    h.write_u32(pipeline.smooth_cycles);
    h.write_u8(pipeline.smooth_loops as u8);
    h.write_u8(pipeline.refine_normals as u8);
    h.write_u8(pipeline.round_helices as u8);
    h.write_u32(pipeline.refine);

    h.write_u32(geom.helix_width.to_bits());
    h.write_u32(geom.helix_height.to_bits());
    h.write_u32(geom.sheet_width.to_bits());
    h.write_u32(geom.sheet_height.to_bits());
    h.write_u32(geom.loop_radius.to_bits());
    h.write_u32(geom.quality);
    h.write_u8(geom.fancy_sheets as u8);
    h.write_u32(geom.arrow_tip_scale.to_bits());
    h.write_u32(geom.arrow_residues);
    h.write_u8(geom.uniform_tube as u8);

    h.finish()
}

fn should_parallelize_cartoon_prep(backbone_len: usize, segment_count: usize) -> bool {
    backbone_len >= LARGE_CARTOON_BACKBONE_THRESHOLD
        || segment_count >= LARGE_CARTOON_SEGMENT_THRESHOLD
}

fn empty_pipeline_output() -> PipelineOutput {
    PipelineOutput {
        extrude_points: Vec::new(),
        runs: Vec::new(),
        total_vertices: 0,
    }
}

fn process_cartoon_segments_serial(
    segments: Vec<BackboneSegment>,
    pipeline_settings: &PipelineSettings,
    geom_settings: &GeomSettings,
) -> PipelineOutput {
    let mut output = empty_pipeline_output();
    for mut seg in segments {
        if seg.len() < 2 {
            continue;
        }
        process_segment(&mut seg, pipeline_settings, geom_settings, &mut output);
    }
    output
}

fn process_cartoon_segments_parallel(
    segments: Vec<BackboneSegment>,
    pipeline_settings: &PipelineSettings,
    geom_settings: &GeomSettings,
) -> PipelineOutput {
    let partials: Vec<_> = segments
        .into_par_iter()
        .map(|mut seg| {
            let mut output = empty_pipeline_output();
            if seg.len() >= 2 {
                process_segment(&mut seg, pipeline_settings, geom_settings, &mut output);
            }
            output
        })
        .collect();

    merge_pipeline_outputs(partials)
}

fn process_cartoon_segments(
    segments: Vec<BackboneSegment>,
    backbone_len: usize,
    pipeline_settings: &PipelineSettings,
    geom_settings: &GeomSettings,
) -> PipelineOutput {
    if should_parallelize_cartoon_prep(backbone_len, segments.len()) {
        process_cartoon_segments_parallel(segments, pipeline_settings, geom_settings)
    } else {
        process_cartoon_segments_serial(segments, pipeline_settings, geom_settings)
    }
}

fn merge_pipeline_outputs(partials: Vec<PipelineOutput>) -> PipelineOutput {
    let total_points = partials
        .iter()
        .map(|part| part.extrude_points.len())
        .sum::<usize>();
    let total_runs = partials.iter().map(|part| part.runs.len()).sum::<usize>();
    let mut output = PipelineOutput {
        extrude_points: Vec::with_capacity(total_points),
        runs: Vec::with_capacity(total_runs),
        total_vertices: 0,
    };

    for mut part in partials {
        let sample_offset = output.extrude_points.len() as u32;
        let vertex_offset = output.total_vertices;
        for run in &mut part.runs {
            run.sample_start += sample_offset;
            run.sample_end += sample_offset;
            run.vertex_offset += vertex_offset;
        }
        output.total_vertices += part.total_vertices;
        output.runs.extend(part.runs);
        output.extrude_points.extend(part.extrude_points);
    }

    output
}

fn make_indirect_buf(device: &wgpu::Device, vertex_count: u32) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("patinae.cartoon.indirect"),
        contents: bytemuck::cast_slice(&[vertex_count, 1u32, 0u32, 0u32]),
        usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST,
    })
}

fn extrude_point_to_gpu(p: &ExtrudePoint) -> ExtrudePointGpu {
    ExtrudePointGpu {
        position: [p.position.x, p.position.y, p.position.z],
        atom_idx: p.atom_idx,
        orientation: [p.orientation.x, p.orientation.y, p.orientation.z],
        _pad: 0,
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct PosFlagsRecord {
    position: [f32; 3],
    flags_as_f32: f32,
}
const _: () = assert!(std::mem::size_of::<PosFlagsRecord>() == 16);

impl Representation for CartoonRep {
    fn kind(&self) -> RepKind {
        self.mode.rep_kind()
    }

    fn build(
        &mut self,
        input: &RenderObjectInput,
        settings: &ResolvedSettings,
        dirty: DirtyFlags,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) {
        let s = input.object_settings.as_ref().unwrap_or(settings);
        let cartoon_transparency = match self.mode {
            CartoonMode::Cartoon => s.cartoon.transparency,
            CartoonMode::Ribbon => 0.0,
        };
        let alpha_mul = (1.0 - cartoon_transparency).clamp(0.0, 1.0);

        // Per-rep render params upload.
        queue.write_buffer(
            &self.render_params_buffer,
            0,
            bytemuck::bytes_of(&CartoonParams {
                alpha_mul,
                color_slot: match self.mode {
                    CartoonMode::Cartoon => 0,
                    CartoonMode::Ribbon => 1,
                },
                _pad1: 0,
                _pad2: 0,
            }),
        );

        let mesh_reuse_dirty = dirty.is_lut_only()
            || (!dirty.is_empty()
                && dirty
                    .difference(DirtyFlags::LUT_ONLY_MASK.union(DirtyFlags::DRAW_MASK))
                    .is_empty());
        let can_reuse_mesh = mesh_reuse_dirty && self.resources.is_some();
        if can_reuse_mesh && !dirty.intersects(DirtyFlags::TRANSPARENCY) {
            return;
        }

        // Opacity heuristic. Per-atom `cartoon_transparency` is now
        // resolved on the GPU via `scene_atom_cartoon_alpha`, but the
        // CPU-side decision routes the rep to opaque-fast or WBOIT path.
        // Conservatively flip to translucent if the per-rep multiplier
        // dips OR any atom carries a sub-unit override.
        let mut opaque = alpha_mul >= 0.999;
        if opaque {
            for atom in input.molecule.atoms() {
                if let Some(t) = atom.repr.cartoon_transparency {
                    if (1.0 - t.clamp(0.0, 1.0)) < 0.999 {
                        opaque = false;
                        break;
                    }
                }
            }
        }
        self.is_opaque_cache = opaque;

        // Fast path: only LUT bits dirty and we already have a built mesh.
        // Skip backbone extraction, hashing, and the entire compute pipeline.
        if can_reuse_mesh {
            return;
        }

        let gap_cutoff = s.cartoon.gap_cutoff;
        let subset_coords = subset_coordinates(input);
        let source_coords = subset_coords.as_ref().unwrap_or(input.coord_set);
        let bb = extract_filtered_backbone(input.molecule, source_coords, gap_cutoff, &|index| {
            input.includes_source_atom(index)
        });
        if bb.len() < 2 {
            self.resources = None;
            self.last_build = None;
            self.vertex_count = 0;
            return;
        }
        let n_atoms = bb.len() as u32;

        // CPU path: orient, sample, and classify runs.
        // LOD-aware defaults plus typed cartoon settings. Recolour /
        // transparency-only settings are intentionally outside this hash.
        let (mut pipeline_settings, mut geom_settings) = from_resolved_settings(s, input.lod);
        if let CartoonMode::Ribbon = self.mode {
            apply_ribbon_pipeline_settings(&mut pipeline_settings, &s.ribbon);
            geom_settings.uniform_tube = true;
            geom_settings.loop_radius = if s.ribbon.radius > 0.0 {
                s.ribbon.radius
            } else {
                0.3
            };
        }
        let coord_hash = hash_backbone(&bb);
        let geom_hash = hash_cartoon_geometry(self.mode, &pipeline_settings, &geom_settings);
        let nucleic_geometry = prepare_nucleic_geometry(
            input.molecule,
            source_coords,
            &bb,
            nucleic_geometry_enabled(self.mode, s),
        );
        let sig = BuildSignature {
            n_atoms,
            geom_hash,
            coord_hash,
            nucleic_hash: nucleic_geometry.signature,
        };
        if self.last_build == Some(sig) {
            return;
        }
        self.last_build = Some(sig);

        let segments = segments_from_backbone_atoms(&bb);
        let output =
            process_cartoon_segments(segments, bb.len(), &pipeline_settings, &geom_settings);
        if output.total_vertices == 0 || output.runs.is_empty() {
            self.resources = None;
            self.vertex_count = 0;
            self.needs_dispatch = false;
            return;
        }
        let nucleic_vertices = nucleic_geometry.emit_vertices();
        let Some(vertex_layout) =
            cartoon_vertex_layout(output.total_vertices, nucleic_vertices.len())
        else {
            log::warn!(
                "patinae-render: combined cartoon geometry exceeds the u32 draw limit - \
                 skipping cartoon for this object"
            );
            self.resources = None;
            self.vertex_count = 0;
            self.needs_dispatch = false;
            return;
        };
        let backbone_vertex_count = vertex_layout.backbone_vertex_count;
        let total_vertices = vertex_layout.total_vertex_count;
        let vertex_bytes = vertex_layout.total_vertex_bytes;

        // ────── Host upload: convert to GPU types + upload ──────
        let extrude_points_gpu: Vec<ExtrudePointGpu> = output
            .extrude_points
            .iter()
            .map(extrude_point_to_gpu)
            .collect();
        let extrude_params = ExtrudeParams {
            n_runs: output.runs.len() as u32,
            n_samples: extrude_points_gpu.len() as u32,
            n_atoms,
            quality: geom_settings.quality,
            // Cross-section dimensions used to live in `cartoon_extrude.wgsl`
            // as `const` literals. They now travel through this uniform so
            // `set cartoon_oval_length`, `set cartoon_rect_length`, etc. take
            // effect. `geom_settings` already applied the `0.0 = auto` fallback
            // in `tessellation::from_resolved_settings`; the Ribbon override
            // above additionally forced `loop_radius = ribbon.radius`.
            helix_width: geom_settings.helix_width,
            helix_height: geom_settings.helix_height,
            sheet_width: geom_settings.sheet_width,
            sheet_height: geom_settings.sheet_height,
            loop_radius: geom_settings.loop_radius,
            arrow_tip_scale: geom_settings.arrow_tip_scale,
            backbone_vertex_count,
            _pad1: 0,
        };
        let extrude_points_bytes = (extrude_points_gpu.len() as u64) * ExtrudePointGpu::SIZE;
        let runs_bytes = (output.runs.len() as u64) * std::mem::size_of::<RunDescriptor>() as u64;
        let storage_buffer_limit = cartoon_storage_buffer_limit(device.limits());
        let storage_buffers = [
            ("extrude_points", extrude_points_bytes),
            ("runs", runs_bytes),
            ("vertices", vertex_bytes),
        ];
        // Requesting adapter-aware limits can make the per-device storage
        // binding cap lower than high-end GPUs. Drop this rep gracefully
        // instead of letting buffer allocation or bind-group creation panic.
        if let Some((name, bytes)) =
            oversized_cartoon_storage_buffer(storage_buffer_limit, &storage_buffers)
        {
            log::warn!(
                "patinae-render: cartoon {name} buffer ({:.2} GiB) exceeds device \
                 storage-buffer limit ({:.2} GiB) at LOD {:?} - skipping cartoon for this object. \
                 Reduce structure size, switch to a coarser LOD, or wait for chunked emission.",
                bytes_to_gib(bytes),
                bytes_to_gib(storage_buffer_limit),
                input.lod,
            );
            self.resources = None;
            self.vertex_count = 0;
            self.needs_dispatch = false;
            return;
        }

        let need_realloc = match &self.resources {
            None => true,
            Some(r) => {
                r.params_buf.size() < ExtrudeParams::SIZE
                    || r.extrude_points_buf.size() < extrude_points_bytes
                    || r.runs_buf.size() < runs_bytes
                    || r.vertex_buf.size() < vertex_bytes
            }
        };

        if need_realloc {
            // Fresh allocation. The compute bind group is rebuilt against
            // the new buffers; lazily here using a placeholder (rebuilt
            // again in `dispatch_compute_build` against the actual
            // `CartoonExtrudeCompute` from `RenderState` to ensure the
            // BGL matches).
            let params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("patinae.cartoon.params"),
                contents: bytemuck::bytes_of(&extrude_params),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
            let extrude_points_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("patinae.cartoon.extrude_points"),
                contents: bytemuck::cast_slice(&extrude_points_gpu),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            });
            let runs_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("patinae.cartoon.runs"),
                contents: bytemuck::cast_slice(&output.runs),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            });
            let vertex_buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("patinae.cartoon.vertices"),
                size: vertex_bytes.max(24),
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::VERTEX
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let indirect_buf = make_indirect_buf(device, total_vertices);
            // Bind group will be (re)built in `dispatch_compute_build`
            // against the shared compute pipeline.
            self.resources = Some(CartoonResources {
                backbone_vertex_count,
                params_buf,
                extrude_points_buf,
                runs_buf,
                vertex_buf,
                indirect_buf,
                // Placeholder — replaced on first dispatch.
                extrude_bg: device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("patinae.cartoon.placeholder_bg"),
                    layout: &device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                        label: Some("patinae.cartoon.placeholder_bgl"),
                        entries: &[],
                    }),
                    entries: &[],
                }),
            });
        } else {
            let r = self.resources.as_mut().unwrap();
            queue.write_buffer(&r.params_buf, 0, bytemuck::bytes_of(&extrude_params));
            queue.write_buffer(
                &r.extrude_points_buf,
                0,
                bytemuck::cast_slice(&extrude_points_gpu),
            );
            queue.write_buffer(&r.runs_buf, 0, bytemuck::cast_slice(&output.runs));
            queue.write_buffer(
                &r.indirect_buf,
                0,
                bytemuck::cast_slice(&[total_vertices, 1u32, 0u32, 0u32]),
            );
            r.backbone_vertex_count = backbone_vertex_count;
        }

        if !nucleic_vertices.is_empty() {
            let r = self
                .resources
                .as_ref()
                .expect("cartoon resources allocated");
            queue.write_buffer(
                &r.vertex_buf,
                vertex_layout.nucleic_byte_offset,
                bytemuck::cast_slice(&nucleic_vertices),
            );
        }

        self.vertex_count = total_vertices;
        self.needs_dispatch = backbone_vertex_count > 0;
    }

    fn is_opaque(&self) -> bool {
        self.is_opaque_cache
    }

    fn memory_usage(&self) -> GpuMemoryUsage {
        let mut usage = GpuMemoryUsage::default();
        usage.add(buffer_usage(&self.render_params_buffer));
        if let Some(resources) = self.resources.as_ref() {
            usage.add(buffer_usage(&resources.params_buf));
            usage.add(buffer_usage(&resources.extrude_points_buf));
            usage.add(buffer_usage(&resources.runs_buf));
            usage.add(buffer_usage(&resources.vertex_buf));
            usage.add(buffer_usage(&resources.indirect_buf));
        }
        usage
    }

    fn record_translucent<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        let r = match self.resources.as_ref() {
            Some(r) => r,
            None => return,
        };
        if self.vertex_count == 0 {
            return;
        }
        let bg = match self.render_params_bind_group.as_ref() {
            Some(b) => b,
            None => return,
        };
        pass.set_bind_group(3, bg, &[]);
        pass.set_vertex_buffer(0, r.vertex_buf.slice(..));
        pass.draw_indirect(&r.indirect_buf, 0);
    }

    fn record_picking<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        let r = match self.resources.as_ref() {
            Some(r) => r,
            None => return,
        };
        if self.vertex_count == 0 {
            return;
        }
        pass.set_vertex_buffer(0, r.vertex_buf.slice(..));
        pass.draw_indirect(&r.indirect_buf, 0);
    }

    /// Record the cartoon extrude compute dispatch into the shared
    /// encoder. `ctx.scene_bg` and `ctx.obj_dynamic_offset` are unused —
    /// cartoon extrude reads its own pre-computed `ExtrudePoint` buffer,
    /// not the scene-wide coords.
    fn record_compute_build(&mut self, ctx: &mut BuildCtx<'_>) -> bool {
        self.ensure_render_bind_group(ctx.device, &ctx.pipelines.cartoon_params_layout);
        if !self.needs_dispatch {
            return false;
        }
        let r = match self.resources.as_mut() {
            Some(r) => r,
            None => {
                self.needs_dispatch = false;
                return false;
            }
        };
        if r.backbone_vertex_count == 0 {
            self.needs_dispatch = false;
            return false;
        }
        let compute_pipeline = &ctx.pipelines.cartoon_compute;
        // Rebuild the compute bind group against the shared pipeline's
        // BGL. We do it on every dispatch since `build()` may have
        // reallocated the vertex/extrude/runs buffers and the previous
        // bind group references stale buffers — cheap (4 entries).
        r.extrude_bg = compute_pipeline.make_bind_group(
            ctx.device,
            &r.params_buf,
            &r.extrude_points_buf,
            &r.runs_buf,
            &r.vertex_buf,
        );
        compute_pipeline.dispatch(ctx.encoder, &r.extrude_bg, r.backbone_vertex_count);
        self.needs_dispatch = false;
        true
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

pub(crate) fn budget_estimates(
    input: &RenderObjectInput<'_>,
    settings: &ResolvedSettings,
    mode: CartoonMode,
) -> Vec<RepMemoryEstimate> {
    let s = input.object_settings.as_ref().unwrap_or(settings);
    vec![cartoon_estimate(input, s, mode)]
}

fn cartoon_estimate(
    input: &RenderObjectInput<'_>,
    settings: &ResolvedSettings,
    mode: CartoonMode,
) -> RepMemoryEstimate {
    let bb = subset_backbone(input, settings.cartoon.gap_cutoff);
    if bb.len() < 2 {
        return RepMemoryEstimate {
            required_bytes: 0,
            scratch_bytes: 0,
            capacity_bytes: CartoonParams::SIZE,
            quality: RepQualityLevel::Full,
            can_chunk: false,
            can_skip: true,
        };
    }

    let (mut pipeline_settings, mut geom_settings) = from_resolved_settings(settings, input.lod);
    if let CartoonMode::Ribbon = mode {
        apply_ribbon_pipeline_settings(&mut pipeline_settings, &settings.ribbon);
        geom_settings.uniform_tube = true;
        geom_settings.loop_radius = if settings.ribbon.radius > 0.0 {
            settings.ribbon.radius
        } else {
            0.3
        };
    }
    let nucleic_residue_count = if nucleic_geometry_enabled(mode, settings) {
        count_nucleic_residues(&bb)
    } else {
        0
    };
    let buffers = cartoon_estimate_buffer_bytes(
        bb.len(),
        nucleic_residue_count,
        pipeline_settings.sampling,
        geom_settings.quality,
    );

    RepMemoryEstimate {
        required_bytes: buffers
            .extrude_points
            .max(buffers.runs)
            .max(buffers.vertices),
        scratch_bytes: 0,
        capacity_bytes: buffers.capacity,
        quality: RepQualityLevel::Full,
        can_chunk: false,
        can_skip: true,
    }
}

/// Build temporary source-indexed coordinates once per distinct subset build.
fn subset_coordinates(input: &RenderObjectInput<'_>) -> Option<patinae_mol::CoordSet> {
    if input.object_id.0 >> 12 == 0 {
        return None;
    }
    let mut coords = patinae_mol::CoordSet::new();
    for (index, position) in input.coord_set.iter_with_atoms() {
        if input.includes_source_atom(index.as_u32()) {
            coords.add_coord(index, position);
        }
    }
    Some(coords)
}

fn subset_backbone(input: &RenderObjectInput<'_>, gap_cutoff: i32) -> Vec<BackboneAtom> {
    let subset_coords = subset_coordinates(input);
    extract_filtered_backbone(
        input.molecule,
        subset_coords.as_ref().unwrap_or(input.coord_set),
        gap_cutoff,
        &|index| input.includes_source_atom(index),
    )
}
