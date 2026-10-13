//! Per-frame instrumentation. Gated behind the `stats` cargo feature so
//! production builds compile out everything in this module.
//!
//! Two data paths:
//!
//! - **CPU markers** — `std::time::Instant` captures around the major
//!   spans of `RenderState::sync` / `RenderState::render`. Always
//!   available when the feature is on.
//! - **GPU timestamps** — `wgpu::Features::TIMESTAMP_QUERY` (also gated
//!   on the host adapter actually supporting it). Each pass writes
//!   start and end ticks into a shared `QuerySet`; `resolve_query_set`
//!   dumps them into a readback buffer, and a future frame maps it.
//!
//! The collector uses a **3-frame ring** so GPU readback never blocks
//! the render loop: frame N writes timestamps, frame N+2 (typically)
//! has them resolved. `take_frame_stats()` returns the most recent
//! fully-resolved entry — `None` until the ring fills.

use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc, Mutex,
};
use std::time::Instant;

#[cfg(test)]
mod tests;

/// Result of one fully-resolved frame's instrumentation. All times in
/// **milliseconds**. `-1.0` means the pass did not run that frame (e.g.
/// silhouette skipped, picking_disabled, etc.).
#[derive(Debug, Clone, Copy)]
pub struct FrameStats {
    /// Host-side seconds since the renderer was created — useful for HUD
    /// tagging.
    pub frame_index: u64,

    // ============ CPU spans ============
    /// Wall-clock ms inside `RenderState::sync` (scene_store flush,
    /// per-rep build dispatch records, etc.).
    pub cpu_sync_ms: f32,
    /// Wall-clock ms inside `RenderState::render` (encoder record).
    pub cpu_record_ms: f32,
    /// Wall-clock ms for `queue.submit(...)` (CPU portion; the GPU
    /// continues async after this returns).
    pub cpu_submit_ms: f32,
    /// Sum of the three above.
    pub cpu_total_ms: f32,

    // ============ GPU passes ============
    pub gpu_compute_build_ms: f32,
    pub gpu_shadow_ms: f32,
    pub gpu_picking_ms: f32,
    pub gpu_picking_reproject_ms: f32,
    pub gpu_opaque_ms: f32,
    pub gpu_fast_overlay_ms: f32,
    pub gpu_translucent_ms: f32,
    pub gpu_composite_ms: f32,
    pub gpu_marking_ms: f32,
    pub gpu_silhouette_ms: f32,
    pub gpu_cull_ms: f32,
    pub gpu_atlas_ao_ms: f32,
    pub atlas_ao_rebuilt: u32,
    pub atlas_ao_reused: u32,
    pub atlas_ao_directions: u32,
    pub atlas_ao_tile_size: u32,
    pub gpu_ssao_ms: f32,
    pub gpu_ssao_blur_ms: f32,
    pub gpu_ssao_compose_ms: f32,
    pub gpu_fxaa_ms: f32,
    /// Sum of all GPU passes. Note: not necessarily equal to a wall-clock
    /// "GPU busy" measurement — passes can overlap on tile-based GPUs and
    /// the driver may inject gaps between dispatches.
    pub gpu_total_ms: f32,
}

impl Default for FrameStats {
    fn default() -> Self {
        Self {
            frame_index: 0,
            cpu_sync_ms: -1.0,
            cpu_record_ms: -1.0,
            cpu_submit_ms: -1.0,
            cpu_total_ms: -1.0,
            gpu_compute_build_ms: -1.0,
            gpu_shadow_ms: -1.0,
            gpu_picking_ms: -1.0,
            gpu_picking_reproject_ms: -1.0,
            gpu_opaque_ms: -1.0,
            gpu_fast_overlay_ms: -1.0,
            gpu_translucent_ms: -1.0,
            gpu_composite_ms: -1.0,
            gpu_marking_ms: -1.0,
            gpu_silhouette_ms: -1.0,
            gpu_cull_ms: -1.0,
            gpu_atlas_ao_ms: -1.0,
            atlas_ao_rebuilt: 0,
            atlas_ao_reused: 0,
            atlas_ao_directions: 0,
            atlas_ao_tile_size: 0,
            gpu_ssao_ms: -1.0,
            gpu_ssao_blur_ms: -1.0,
            gpu_ssao_compose_ms: -1.0,
            gpu_fxaa_ms: -1.0,
            gpu_total_ms: -1.0,
        }
    }
}

/// Index into the per-frame `QuerySet`. 2 ticks (start/end) per pass.
#[derive(Debug, Clone, Copy)]
#[repr(u32)]
pub enum Pass {
    ComputeBuild = 0,
    Picking = 1,
    PickingReproject = 2,
    Opaque = 3,
    Translucent = 4,
    Composite = 5,
    Silhouette = 6,
    Cull = 7,
    Ssao = 8,
    SsaoBlur = 9,
    SsaoCompose = 10,
    Fxaa = 11,
    Shadow = 12,
    Marking = 13,
    AtlasAo = 14,
    FastOverlay = 15,
}

/// Two timestamps per pass.
pub const TIMESTAMPS_PER_FRAME: u32 = 32;
/// Frames in flight before a slot is reused. 3 is the canonical
/// double-buffering + 1 budget.
pub const IN_FLIGHT_FRAMES: usize = 3;

fn written_query_pairs(written_mask: u32) -> impl Iterator<Item = std::ops::Range<u32>> {
    (0..TIMESTAMPS_PER_FRAME / 2)
        .filter(move |pass| written_mask & (1 << pass) != 0)
        .map(|pass| pass * 2..pass * 2 + 2)
}

/// One ring slot — a frame's worth of CPU markers + GPU resolution
/// buffer. State flips through `Recording → Pending → Ready → Drained`
/// as the GPU catches up.
struct FrameSlot {
    state: AtomicU8,
    cpu_sync_start: Option<Instant>,
    cpu_sync_end: Option<Instant>,
    cpu_record_start: Option<Instant>,
    cpu_record_end: Option<Instant>,
    cpu_submit_start: Option<Instant>,
    cpu_submit_end: Option<Instant>,
    frame_index: u64,
    /// `u64 × TIMESTAMPS_PER_FRAME` GPU buffer that `resolve_query_set`
    /// writes into. Cannot be MAP_READ on the same buffer, so we
    /// `copy_buffer_to_buffer` into `readback_buffer` for host read.
    resolve_buffer: wgpu::Buffer,
    /// Mappable copy of `resolve_buffer` for `map_async`.
    readback_buffer: wgpu::Buffer,
    /// Which timestamps were actually written this frame (driver may
    /// merge passes that didn't run). Bitmask indexed by `Pass`.
    written_mask: u32,
    atlas_ao_rebuilt: u32,
    atlas_ao_reused: u32,
    atlas_ao_directions: u32,
    atlas_ao_tile_size: u32,
}

const SLOT_IDLE: u8 = 0;
const SLOT_RECORDING: u8 = 1;
/// Resolve + copy_buffer_to_buffer have been recorded into the user's
/// encoder. `map_async` must NOT be called until the user actually
/// `queue.submit`s that encoder, otherwise validation rejects the
/// submit ("buffer is still mapped"). Transition happens at the start
/// of the next frame inside `frame_begin`.
const SLOT_NEEDS_MAP: u8 = 2;
const SLOT_PENDING_READ: u8 = 3;

/// Per-frame instrumentation accumulator. One per `RenderState`. The
/// collector is `Sync` so the readback callback (which fires on a wgpu
/// worker thread) can flip slot state.
///
/// `gpu_supported = false` ⇒ CPU markers only; all `gpu_*` fields of
/// `FrameStats` stay at the default `-1.0`. Hosts that need GPU times
/// must request `Features::TIMESTAMP_QUERY` when building the
/// `wgpu::Device`.
pub struct FrameStatsCollector {
    pub gpu_supported: bool,
    pub(crate) query_sets: Vec<wgpu::QuerySet>,
    slots: Vec<Arc<Mutex<FrameSlot>>>,
    cursor: usize,
    frame_counter: u64,
    /// `false` when the current slot is still pending readback from a
    /// prior frame; in that case we skip all instrumentation for this
    /// frame (no timestamp writes, no resolve, no CPU markers) until
    /// the slot frees up.
    recording: bool,
    /// Nanoseconds-per-tick conversion from the queue. Constant after
    /// init.
    ticks_to_ns: f32,
    /// Last fully-resolved frame, taken out via `take_latest`.
    latest: Arc<Mutex<Option<FrameStats>>>,
}

impl FrameStatsCollector {
    /// Build a collector. GPU timestamps are enabled iff the device
    /// advertises `Features::TIMESTAMP_QUERY`; otherwise CPU-only mode.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let gpu_supported = device.features().contains(wgpu::Features::TIMESTAMP_QUERY);
        let ticks_to_ns = if gpu_supported {
            queue.get_timestamp_period()
        } else {
            0.0
        };
        let mut query_sets = Vec::with_capacity(IN_FLIGHT_FRAMES);
        let mut slots = Vec::with_capacity(IN_FLIGHT_FRAMES);
        for i in 0..IN_FLIGHT_FRAMES {
            if gpu_supported {
                let qs = device.create_query_set(&wgpu::QuerySetDescriptor {
                    label: Some(&format!("patinae.stats.qs{i}")),
                    ty: wgpu::QueryType::Timestamp,
                    count: TIMESTAMPS_PER_FRAME,
                });
                query_sets.push(qs);
            }
            let resolve = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&format!("patinae.stats.resolve{i}")),
                size: (TIMESTAMPS_PER_FRAME as u64) * 8,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&format!("patinae.stats.readback{i}")),
                size: (TIMESTAMPS_PER_FRAME as u64) * 8,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            slots.push(Arc::new(Mutex::new(FrameSlot {
                state: AtomicU8::new(SLOT_IDLE),
                cpu_sync_start: None,
                cpu_sync_end: None,
                cpu_record_start: None,
                cpu_record_end: None,
                cpu_submit_start: None,
                cpu_submit_end: None,
                frame_index: 0,
                resolve_buffer: resolve,
                readback_buffer: readback,
                written_mask: 0,
                atlas_ao_rebuilt: 0,
                atlas_ao_reused: 0,
                atlas_ao_directions: 0,
                atlas_ao_tile_size: 0,
            })));
        }
        Self {
            gpu_supported,
            query_sets,
            slots,
            cursor: 0,
            frame_counter: 0,
            recording: false,
            ticks_to_ns,
            latest: Arc::new(Mutex::new(None)),
        }
    }

    /// `RenderPassTimestampWrites` for the given pass, or `None` when
    /// GPU timestamps are unavailable or this frame's slot is busy.
    pub fn render_pass_timestamp_writes(
        &self,
        pass: Pass,
    ) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        if !self.gpu_supported || !self.recording {
            return None;
        }
        self.mark_pass_written(pass);
        let slot = pass as u32 * 2;
        Some(wgpu::RenderPassTimestampWrites {
            query_set: &self.query_sets[self.cursor],
            beginning_of_pass_write_index: Some(slot),
            end_of_pass_write_index: Some(slot + 1),
        })
    }

    /// `ComputePassTimestampWrites` for the given pass.
    pub fn compute_pass_timestamp_writes(
        &self,
        pass: Pass,
    ) -> Option<wgpu::ComputePassTimestampWrites<'_>> {
        if !self.gpu_supported || !self.recording {
            return None;
        }
        self.mark_pass_written(pass);
        let slot = pass as u32 * 2;
        Some(wgpu::ComputePassTimestampWrites {
            query_set: &self.query_sets[self.cursor],
            beginning_of_pass_write_index: Some(slot),
            end_of_pass_write_index: Some(slot + 1),
        })
    }

    /// Slot index used for the *current* frame's records. Caller is
    /// expected to wrap calls with `frame_begin` / `frame_end` so the
    /// cursor advances exactly once per frame.
    pub fn current_slot_idx(&self) -> usize {
        self.cursor
    }

    pub fn current_query_set(&self) -> Option<&wgpu::QuerySet> {
        if self.gpu_supported {
            Some(&self.query_sets[self.cursor])
        } else {
            None
        }
    }

    /// Mark `pass` as written this frame, so `resolve` later only fetches
    /// the slots actually populated.
    pub fn mark_pass_written(&self, pass: Pass) {
        if let Ok(mut slot) = self.slots[self.cursor].lock() {
            slot.written_mask |= 1 << (pass as u32);
        }
    }

    /// Begin a new frame. Kicks `map_async` for any slot in `NEEDS_MAP`
    /// state (its resolve+copy was recorded last frame and the user has
    /// since submitted that encoder). Then decides whether the current
    /// ring slot is free to record this frame's data.
    pub fn frame_begin(&mut self) {
        self.frame_counter += 1;
        if self.gpu_supported {
            self.kick_pending_maps();
        }
        let slot_state = self.slots[self.cursor]
            .lock()
            .unwrap()
            .state
            .load(Ordering::Acquire);
        if slot_state != SLOT_IDLE {
            // Previous frame's readback still in flight on this slot.
            // Drop instrumentation for this frame.
            self.recording = false;
            return;
        }
        self.recording = true;
        let slot = self.slots[self.cursor].clone();
        let mut s = slot.lock().unwrap();
        s.cpu_sync_start = None;
        s.cpu_sync_end = None;
        s.cpu_record_start = None;
        s.cpu_record_end = None;
        s.cpu_submit_start = None;
        s.cpu_submit_end = None;
        s.written_mask = 0;
        s.atlas_ao_rebuilt = 0;
        s.atlas_ao_reused = 0;
        s.atlas_ao_directions = 0;
        s.atlas_ao_tile_size = 0;
        s.frame_index = self.frame_counter;
        s.state.store(SLOT_RECORDING, Ordering::Release);
    }

    pub fn mark_sync_start(&self) {
        if !self.recording {
            return;
        }
        if let Ok(mut s) = self.slots[self.cursor].lock() {
            s.cpu_sync_start = Some(Instant::now());
        }
    }
    pub fn mark_sync_end(&self) {
        if !self.recording {
            return;
        }
        if let Ok(mut s) = self.slots[self.cursor].lock() {
            s.cpu_sync_end = Some(Instant::now());
        }
    }
    pub fn mark_record_start(&self) {
        if !self.recording {
            return;
        }
        if let Ok(mut s) = self.slots[self.cursor].lock() {
            s.cpu_record_start = Some(Instant::now());
        }
    }
    pub fn mark_record_end(&self) {
        if !self.recording {
            return;
        }
        if let Ok(mut s) = self.slots[self.cursor].lock() {
            s.cpu_record_end = Some(Instant::now());
        }
    }
    pub fn mark_submit_start(&self) {
        if !self.recording {
            return;
        }
        if let Ok(mut s) = self.slots[self.cursor].lock() {
            s.cpu_submit_start = Some(Instant::now());
        }
    }
    pub fn mark_submit_end(&self) {
        if !self.recording {
            return;
        }
        if let Ok(mut s) = self.slots[self.cursor].lock() {
            s.cpu_submit_end = Some(Instant::now());
        }
    }

    pub fn record_atlas_ao_cache(&self, rebuilt: bool, directions: u32, tile_size: u32) {
        if !self.recording {
            return;
        }
        if let Ok(mut s) = self.slots[self.cursor].lock() {
            if rebuilt {
                s.atlas_ao_rebuilt = s.atlas_ao_rebuilt.saturating_add(1);
            } else {
                s.atlas_ao_reused = s.atlas_ao_reused.saturating_add(1);
            }
            s.atlas_ao_directions = directions;
            s.atlas_ao_tile_size = tile_size;
        }
    }

    /// Record the GPU-side timestamp resolve + copy into the user's
    /// encoder. Does **not** call `map_async` — that has to happen
    /// after the user submits the encoder (wgpu rejects submitting a
    /// buffer that's already in the mapping pipeline). `frame_begin`
    /// on the next frame kicks the map for any slot in `NEEDS_MAP`
    /// state.
    ///
    /// In CPU-only mode (`!gpu_supported`) this publishes stats from
    /// CPU markers immediately.
    pub fn resolve_and_map(&mut self, encoder: &mut wgpu::CommandEncoder, _queue: &wgpu::Queue) {
        if !self.recording {
            // frame_begin decided to skip; nothing to resolve.
            return;
        }
        let slot_arc = self.slots[self.cursor].clone();
        if !self.gpu_supported {
            // CPU-only path: publish stats from markers immediately.
            self.publish_cpu_only_stats();
            self.cursor = (self.cursor + 1) % IN_FLIGHT_FRAMES;
            return;
        }
        let (resolve_buf, readback_buf, written_mask) = {
            let slot = slot_arc.lock().unwrap();
            (
                slot.resolve_buffer.clone(),
                slot.readback_buffer.clone(),
                slot.written_mask,
            )
        };
        let qs = &self.query_sets[self.cursor];
        for queries in written_query_pairs(written_mask) {
            let readback_offset = u64::from(queries.start) * u64::from(wgpu::QUERY_SIZE);
            // Resolving unwritten queries is invalid on Vulkan with wgpu 29.
            // Resolve destinations require 256-byte alignment, so reuse offset
            // zero and copy each pair before the next resolve overwrites it.
            encoder.resolve_query_set(qs, queries, &resolve_buf, 0);
            encoder.copy_buffer_to_buffer(
                &resolve_buf,
                0,
                &readback_buf,
                readback_offset,
                2 * u64::from(wgpu::QUERY_SIZE),
            );
        }
        // Unwritten pairs may contain an older frame's ticks, including when
        // the mask is empty. Readback uses this frame's mask to ignore them.
        slot_arc
            .lock()
            .unwrap()
            .state
            .store(SLOT_NEEDS_MAP, Ordering::Release);
        self.cursor = (self.cursor + 1) % IN_FLIGHT_FRAMES;
    }

    /// For every slot in `NEEDS_MAP` state, kick `map_async`. Called
    /// from `frame_begin` of the following frame so the host has
    /// already submitted the encoder containing the resolve+copy
    /// (wgpu rejects submitting a buffer with a pending map).
    fn kick_pending_maps(&self) {
        for slot_arc in &self.slots {
            let (needs, readback_buf) = {
                let slot = slot_arc.lock().unwrap();
                let s = slot.state.load(Ordering::Acquire) == SLOT_NEEDS_MAP;
                (s, slot.readback_buffer.clone())
            };
            if !needs {
                continue;
            }
            slot_arc
                .lock()
                .unwrap()
                .state
                .store(SLOT_PENDING_READ, Ordering::Release);
            let slot_for_cb = slot_arc.clone();
            let latest_for_cb = self.latest.clone();
            let ticks_to_ns = self.ticks_to_ns;
            let buf_for_cb = readback_buf.clone();
            readback_buf
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    if result.is_err() {
                        if let Ok(s) = slot_for_cb.lock() {
                            s.state.store(SLOT_IDLE, Ordering::Release);
                        }
                        return;
                    }
                    let mut stats = FrameStats::default();
                    let written_mask;
                    if let Ok(s) = slot_for_cb.lock() {
                        let mb_dur = |a: Option<Instant>, b: Option<Instant>| -> f32 {
                            match (a, b) {
                                (Some(x), Some(y)) => y.duration_since(x).as_secs_f32() * 1000.0,
                                _ => -1.0,
                            }
                        };
                        stats.frame_index = s.frame_index;
                        stats.cpu_sync_ms = mb_dur(s.cpu_sync_start, s.cpu_sync_end);
                        stats.cpu_record_ms = mb_dur(s.cpu_record_start, s.cpu_record_end);
                        stats.cpu_submit_ms = mb_dur(s.cpu_submit_start, s.cpu_submit_end);
                        stats.cpu_total_ms =
                            [stats.cpu_sync_ms, stats.cpu_record_ms, stats.cpu_submit_ms]
                                .iter()
                                .copied()
                                .filter(|v| *v >= 0.0)
                                .sum();
                        stats.atlas_ao_rebuilt = s.atlas_ao_rebuilt;
                        stats.atlas_ao_reused = s.atlas_ao_reused;
                        stats.atlas_ao_directions = s.atlas_ao_directions;
                        stats.atlas_ao_tile_size = s.atlas_ao_tile_size;
                        written_mask = s.written_mask;
                    } else {
                        written_mask = 0;
                    }
                    let slice = buf_for_cb.slice(..);
                    let mut ticks_owned: [u64; TIMESTAMPS_PER_FRAME as usize] =
                        [0; TIMESTAMPS_PER_FRAME as usize];
                    {
                        let mapped = slice.get_mapped_range();
                        let src: &[u64] = bytemuck::cast_slice(&mapped);
                        let n = ticks_owned.len().min(src.len());
                        ticks_owned[..n].copy_from_slice(&src[..n]);
                    }
                    buf_for_cb.unmap();
                    let pass_ms = |start_i: usize, end_i: usize, mask_bit: u32| -> f32 {
                        if (written_mask >> mask_bit) & 1 == 0 {
                            return -1.0;
                        }
                        let dt = ticks_owned[end_i].saturating_sub(ticks_owned[start_i]);
                        (dt as f32) * ticks_to_ns / 1_000_000.0
                    };
                    stats.gpu_compute_build_ms = pass_ms(0, 1, Pass::ComputeBuild as u32);
                    stats.gpu_picking_ms = pass_ms(2, 3, Pass::Picking as u32);
                    stats.gpu_picking_reproject_ms = pass_ms(4, 5, Pass::PickingReproject as u32);
                    stats.gpu_opaque_ms = pass_ms(6, 7, Pass::Opaque as u32);
                    stats.gpu_translucent_ms = pass_ms(8, 9, Pass::Translucent as u32);
                    stats.gpu_composite_ms = pass_ms(10, 11, Pass::Composite as u32);
                    stats.gpu_silhouette_ms = pass_ms(12, 13, Pass::Silhouette as u32);
                    stats.gpu_cull_ms = pass_ms(14, 15, Pass::Cull as u32);
                    stats.gpu_ssao_ms = pass_ms(16, 17, Pass::Ssao as u32);
                    stats.gpu_ssao_blur_ms = pass_ms(18, 19, Pass::SsaoBlur as u32);
                    stats.gpu_ssao_compose_ms = pass_ms(20, 21, Pass::SsaoCompose as u32);
                    stats.gpu_fxaa_ms = pass_ms(22, 23, Pass::Fxaa as u32);
                    stats.gpu_shadow_ms = pass_ms(24, 25, Pass::Shadow as u32);
                    stats.gpu_marking_ms = pass_ms(26, 27, Pass::Marking as u32);
                    stats.gpu_atlas_ao_ms = pass_ms(28, 29, Pass::AtlasAo as u32);
                    stats.gpu_fast_overlay_ms = pass_ms(30, 31, Pass::FastOverlay as u32);
                    stats.gpu_total_ms = [
                        stats.gpu_compute_build_ms,
                        stats.gpu_shadow_ms,
                        stats.gpu_picking_ms,
                        stats.gpu_picking_reproject_ms,
                        stats.gpu_opaque_ms,
                        stats.gpu_fast_overlay_ms,
                        stats.gpu_translucent_ms,
                        stats.gpu_composite_ms,
                        stats.gpu_marking_ms,
                        stats.gpu_silhouette_ms,
                        stats.gpu_cull_ms,
                        stats.gpu_atlas_ao_ms,
                        stats.gpu_ssao_ms,
                        stats.gpu_ssao_blur_ms,
                        stats.gpu_ssao_compose_ms,
                        stats.gpu_fxaa_ms,
                    ]
                    .iter()
                    .copied()
                    .filter(|v| *v >= 0.0)
                    .sum();
                    if let Ok(mut latest) = latest_for_cb.lock() {
                        *latest = Some(stats);
                    }
                    if let Ok(s) = slot_for_cb.lock() {
                        s.state.store(SLOT_IDLE, Ordering::Release);
                    }
                });
        }
    }

    /// Pop the most recent fully-resolved frame, if any.
    pub fn take_latest(&self) -> Option<FrameStats> {
        self.latest.lock().ok().and_then(|mut g| g.take())
    }

    fn publish_cpu_only_stats(&self) {
        let slot_arc = self.slots[self.cursor].clone();
        let mut stats = FrameStats::default();
        if let Ok(s) = slot_arc.lock() {
            let mb_dur = |a: Option<Instant>, b: Option<Instant>| -> f32 {
                match (a, b) {
                    (Some(x), Some(y)) => y.duration_since(x).as_secs_f32() * 1000.0,
                    _ => -1.0,
                }
            };
            stats.frame_index = s.frame_index;
            stats.cpu_sync_ms = mb_dur(s.cpu_sync_start, s.cpu_sync_end);
            stats.cpu_record_ms = mb_dur(s.cpu_record_start, s.cpu_record_end);
            stats.cpu_submit_ms = mb_dur(s.cpu_submit_start, s.cpu_submit_end);
            stats.cpu_total_ms = [stats.cpu_sync_ms, stats.cpu_record_ms, stats.cpu_submit_ms]
                .iter()
                .copied()
                .filter(|v| *v >= 0.0)
                .sum();
            stats.atlas_ao_rebuilt = s.atlas_ao_rebuilt;
            stats.atlas_ao_reused = s.atlas_ao_reused;
            stats.atlas_ao_directions = s.atlas_ao_directions;
            stats.atlas_ao_tile_size = s.atlas_ao_tile_size;
        }
        if let Ok(mut latest) = self.latest.lock() {
            *latest = Some(stats);
        }
    }
}
