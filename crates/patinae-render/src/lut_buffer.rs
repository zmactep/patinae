//! `GrowableStorageBuffer` — capacity-doubling storage buffer.
//!
//! Used for color, mask, and selection LUTs. Reallocates only when growth is
//! required; writes are amortised O(N) per `flush`.

use crate::memory::{checked_buffer_capacity, BufferSizeError};
use bytemuck::Pod;

use crate::memory::GpuMemoryUsage;
use crate::memory::SceneStoreGrowthPolicy;

// Sparse hover updates normally touch one or two marker entries. If a caller
// dirties many disjoint ranges, a single full upload is cheaper than issuing a
// long stream of tiny queue writes.
const MAX_DIRTY_SPANS: usize = 64;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BufferFlushStats {
    pub ranges: u32,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DirtySpans {
    Clean,
    Full,
    Spans(Vec<(usize, usize)>),
}

/// A storage buffer that owns its CPU-side staging vector and lazily
/// reallocates the GPU buffer on growth. Callers stage edits via `cpu_mut`
/// and flush dirty ranges with `flush`.
pub struct GrowableStorageBuffer<T: Pod> {
    label: &'static str,
    cpu: Vec<T>,
    gpu: Option<wgpu::Buffer>,
    capacity: usize,
    dirty: DirtySpans,
}

impl<T: Pod> GrowableStorageBuffer<T> {
    pub fn new(label: &'static str) -> Self {
        Self {
            label,
            cpu: Vec::new(),
            gpu: None,
            capacity: 0,
            dirty: DirtySpans::Clean,
        }
    }

    pub fn len(&self) -> usize {
        self.cpu.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cpu.is_empty()
    }

    /// Resize the CPU buffer. Newly-added entries are filled with `default`.
    /// Marks the entire buffer dirty if growth occurred.
    pub fn resize(&mut self, len: usize, default: T) {
        if len > self.cpu.len() {
            self.mark_dirty(self.cpu.len(), len);
        }
        self.cpu.resize(len, default);
    }

    /// Read-only access to the CPU staging vec.
    pub fn cpu(&self) -> &[T] {
        &self.cpu
    }

    /// Borrows a staging range and marks only that range for upload.
    ///
    /// # Panics
    /// Panics if the range is outside the CPU buffer.
    pub(crate) fn range_mut(&mut self, range: std::ops::Range<usize>) -> &mut [T] {
        assert!(range.start <= range.end && range.end <= self.cpu.len());
        self.mark_dirty(range.start, range.end);
        &mut self.cpu[range]
    }

    /// Mutable access to a single entry. Marks it dirty.
    pub fn set(&mut self, index: usize, value: T) {
        if index >= self.cpu.len() {
            return;
        }
        self.cpu[index] = value;
        self.mark_dirty(index, index + 1);
    }

    /// Mutable access to the entire CPU vec for bulk writes (e.g. on rebuild).
    /// Marks the whole buffer dirty.
    pub fn replace_all(&mut self, values: Vec<T>) {
        self.cpu = values;
        self.mark_dirty(0, self.cpu.len());
    }

    /// Replaces CPU data and drops GPU capacity.
    pub(crate) fn replace_all_and_discard_gpu(&mut self, values: Vec<T>) {
        self.cpu = values;
        self.gpu = None;
        self.capacity = 0;
        self.dirty = DirtySpans::Full;
    }

    fn mark_dirty(&mut self, lo: usize, hi: usize) {
        if hi <= lo {
            return;
        }
        if lo == 0 && hi >= self.cpu.len() {
            self.dirty = DirtySpans::Full;
            return;
        }
        let DirtySpans::Spans(spans) = &mut self.dirty else {
            if matches!(&self.dirty, DirtySpans::Clean) {
                self.dirty = DirtySpans::Spans(vec![(lo, hi)]);
            }
            return;
        };

        let mut lo = lo;
        let mut hi = hi;
        let mut insert_at = spans.len();
        let mut i = 0;
        while i < spans.len() {
            let (span_lo, span_hi) = spans[i];
            if span_hi < lo {
                i += 1;
                continue;
            }
            if hi < span_lo {
                insert_at = i;
                break;
            }
            lo = lo.min(span_lo);
            hi = hi.max(span_hi);
            spans.remove(i);
            insert_at = i;
        }
        spans.insert(insert_at, (lo, hi));
        if spans.len() > MAX_DIRTY_SPANS {
            self.dirty = DirtySpans::Full;
        }
    }

    pub(crate) fn planned_capacity(
        &self,
        limits: &wgpu::Limits,
        growth: SceneStoreGrowthPolicy,
    ) -> Result<usize, BufferSizeError> {
        checked_buffer_capacity(
            self.label,
            self.cpu.len(),
            self.capacity,
            std::mem::size_of::<T>(),
            growth,
            limits
                .max_buffer_size
                .min(limits.max_storage_buffer_binding_size),
        )
    }

    /// Ensure the GPU buffer holds at least `cpu.len()` entries. Returns
    /// `true` iff a reallocation occurred — the caller must rebuild any bind
    /// group that referenced the old buffer.
    ///
    /// # Errors
    /// Returns an error before allocation if the data exceeds device limits.
    pub fn ensure_capacity(
        &mut self,
        device: &wgpu::Device,
        growth: SceneStoreGrowthPolicy,
    ) -> Result<bool, BufferSizeError> {
        let new_cap = self.planned_capacity(&device.limits(), growth)?;
        let needed = self.cpu.len().max(1);
        if needed <= self.capacity && self.gpu.is_some() {
            return Ok(false);
        }
        let size = (new_cap * std::mem::size_of::<T>()) as u64;
        let usages = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;
        self.gpu = Some(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(self.label),
            size,
            usage: usages,
            mapped_at_creation: false,
        }));
        self.capacity = new_cap;
        // Whole buffer is dirty after reallocation.
        self.dirty = DirtySpans::Full;
        Ok(true)
    }

    /// Upload the dirty range to the GPU. No-op if clean.
    pub fn flush(&mut self, queue: &wgpu::Queue) -> BufferFlushStats {
        let dirty = std::mem::replace(&mut self.dirty, DirtySpans::Clean);
        let buffer = match self.gpu.as_ref() {
            Some(b) => b,
            None => return BufferFlushStats::default(),
        };
        let item = std::mem::size_of::<T>();
        match dirty {
            DirtySpans::Clean => BufferFlushStats::default(),
            DirtySpans::Full => {
                if self.cpu.is_empty() {
                    return BufferFlushStats::default();
                }
                let bytes = bytemuck::cast_slice(&self.cpu);
                queue.write_buffer(buffer, 0, bytes);
                BufferFlushStats {
                    ranges: 1,
                    bytes: bytes.len() as u64,
                }
            }
            DirtySpans::Spans(spans) => {
                let mut stats = BufferFlushStats::default();
                for (lo, hi) in spans {
                    if hi <= lo {
                        continue;
                    }
                    let offset = (lo * item) as u64;
                    let bytes = bytemuck::cast_slice(&self.cpu[lo..hi]);
                    queue.write_buffer(buffer, offset, bytes);
                    stats.ranges = stats.ranges.saturating_add(1);
                    stats.bytes = stats.bytes.saturating_add(bytes.len() as u64);
                }
                stats
            }
        }
    }

    pub fn buffer(&self) -> Option<&wgpu::Buffer> {
        self.gpu.as_ref()
    }

    /// GPU bytes currently allocated for this storage buffer.
    pub fn memory_usage(&self) -> GpuMemoryUsage {
        if self.gpu.is_none() {
            return GpuMemoryUsage::default();
        }
        let item_size = std::mem::size_of::<T>() as u64;
        GpuMemoryUsage::live_capacity(
            (self.cpu.len() as u64).saturating_mul(item_size),
            (self.capacity as u64).saturating_mul(item_size),
        )
    }

    /// Capacity in entries (not bytes). Useful for sizing `min_binding_size`.
    pub fn capacity_entries(&self) -> usize {
        self.capacity
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirty_spans_coalesce_adjacent_and_overlapping_ranges() {
        let mut buffer = GrowableStorageBuffer::<u32>::new("test");
        buffer.resize(16, 0);
        buffer.dirty = DirtySpans::Clean;

        buffer.set(4, 1);
        buffer.set(5, 2);
        buffer.set(8, 3);
        buffer.set(7, 4);

        assert_eq!(buffer.dirty, DirtySpans::Spans(vec![(4, 6), (7, 9)]));
    }

    #[test]
    fn dirty_spans_fall_back_to_full_upload_after_many_ranges() {
        let mut buffer = GrowableStorageBuffer::<u32>::new("test");
        buffer.resize(256, 0);
        buffer.dirty = DirtySpans::Clean;

        for i in 0..=MAX_DIRTY_SPANS {
            buffer.set(i * 2, i as u32);
        }

        assert_eq!(buffer.dirty, DirtySpans::Full);
    }
}
