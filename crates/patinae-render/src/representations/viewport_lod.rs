use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};

use crate::memory::{buffer_usage, GpuMemoryUsage};

const VIEWPORT_LOD_READBACK_BYTES: u64 = 4;
const VIEWPORT_LOD_IDLE: u8 = 0;
const VIEWPORT_LOD_NEEDS_MAP: u8 = 1;
const VIEWPORT_LOD_PENDING: u8 = 2;
const VIEWPORT_LOD_READY: u8 = 3;
const VIEWPORT_LOD_FAILED: u8 = 4;

pub(super) struct ViewportLodReadback {
    buffer: wgpu::Buffer,
    state: Arc<AtomicU8>,
}

impl ViewportLodReadback {
    pub(super) fn new(device: &wgpu::Device, label: &'static str) -> Self {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: VIEWPORT_LOD_READBACK_BYTES,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Self {
            buffer,
            state: Arc::new(AtomicU8::new(VIEWPORT_LOD_IDLE)),
        }
    }

    pub(super) fn kick_map_if_needed(&self) {
        if self
            .state
            .compare_exchange(
                VIEWPORT_LOD_NEEDS_MAP,
                VIEWPORT_LOD_PENDING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            return;
        }
        let state = Arc::clone(&self.state);
        self.buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                state.store(
                    if result.is_ok() {
                        VIEWPORT_LOD_READY
                    } else {
                        VIEWPORT_LOD_FAILED
                    },
                    Ordering::Release,
                );
            });
    }

    pub(super) fn collect(&self) -> Option<u32> {
        match self.state.load(Ordering::Acquire) {
            VIEWPORT_LOD_READY => {}
            VIEWPORT_LOD_FAILED => {
                self.state.store(VIEWPORT_LOD_IDLE, Ordering::Release);
                return None;
            }
            _ => return None,
        }
        let slice = self.buffer.slice(..);
        let mapped = slice
            .get_mapped_range()
            .expect("readback range is mapped and has no overlapping views");
        let count = u32::from_le_bytes([mapped[0], mapped[1], mapped[2], mapped[3]]);
        drop(mapped);
        self.buffer.unmap();
        self.state.store(VIEWPORT_LOD_IDLE, Ordering::Release);
        Some(count)
    }

    pub(super) fn record_count_copy(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        count_buffer: &wgpu::Buffer,
    ) {
        if self
            .state
            .compare_exchange(
                VIEWPORT_LOD_IDLE,
                VIEWPORT_LOD_NEEDS_MAP,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            return;
        }
        encoder.copy_buffer_to_buffer(
            count_buffer,
            0,
            &self.buffer,
            0,
            VIEWPORT_LOD_READBACK_BYTES,
        );
    }

    pub(super) fn memory_usage(&self) -> GpuMemoryUsage {
        buffer_usage(&self.buffer)
    }
}
