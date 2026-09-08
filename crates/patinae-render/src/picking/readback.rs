//! Async picking readback. One staging buffer, small neighborhood per pick.
//!
//! On `submit`, we issue a `copy_texture_to_buffer` for a small region around
//! the cursor, then `mapAsync`. The host calls `try_collect` next frame; if
//! ready, the nearest non-empty pixel decodes to a `PickHit` (or `None`).

use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};

use crate::picking::{decode_pixel, PickHit, RepKind};
use crate::{memory::buffer_usage, GpuMemoryUsage};

/// 256-byte aligned bytes_per_row for `Rg32Uint` (8 B / pixel). 256 is the
/// `COPY_BYTES_PER_ROW_ALIGNMENT` constant.
const READBACK_BYTES_PER_ROW: u32 = 256;
const BYTES_PER_PICKING_PIXEL: usize = 8;
const PICK_READBACK_RADIUS: i32 = 3;
const PICK_READBACK_DIAMETER: u32 = (PICK_READBACK_RADIUS as u32) * 2 + 1;
pub(crate) const PICKING_READBACK_BUFFER_BYTES: u64 =
    (READBACK_BYTES_PER_ROW * PICK_READBACK_DIAMETER) as u64;
const READBACK_IDLE: u8 = 0;
const READBACK_PENDING: u8 = 1;
const READBACK_READY: u8 = 2;
const READBACK_FAILED: u8 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickReadbackTarget {
    Hover,
    Click,
}

pub struct PickingReadback {
    buffer: wgpu::Buffer,
    origin: std::cell::Cell<(u32, u32)>,
    center: std::cell::Cell<(u32, u32)>,
    dims: std::cell::Cell<(u32, u32)>,
    state: Arc<AtomicU8>,
    token: Arc<()>,
}

impl PickingReadback {
    pub fn new(device: &wgpu::Device) -> Self {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("patinae.picking.readback"),
            size: PICKING_READBACK_BUFFER_BYTES,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Self {
            buffer,
            origin: std::cell::Cell::new((0, 0)),
            center: std::cell::Cell::new((0, 0)),
            dims: std::cell::Cell::new((1, 1)),
            state: Arc::new(AtomicU8::new(READBACK_IDLE)),
            token: Arc::new(()),
        }
    }

    /// Encode a small neighborhood copy from the picking texture into the staging buffer.
    /// `(x, y)` is in **picking-texture** coordinates (already scaled by
    /// `PICKING_SCALE`).
    pub fn issue_copy(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        picking_texture: &wgpu::Texture,
        x: u32,
        y: u32,
        target: PickReadbackTarget,
    ) -> Option<PendingPick> {
        if self
            .state
            .compare_exchange(
                READBACK_IDLE,
                READBACK_PENDING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            return None;
        }
        let extent = picking_texture.size();
        let x = x.min(extent.width.saturating_sub(1));
        let y = y.min(extent.height.saturating_sub(1));
        let x0 = x.saturating_sub(PICK_READBACK_RADIUS as u32);
        let y0 = y.saturating_sub(PICK_READBACK_RADIUS as u32);
        let x1 = (x + PICK_READBACK_RADIUS as u32).min(extent.width.saturating_sub(1));
        let y1 = (y + PICK_READBACK_RADIUS as u32).min(extent.height.saturating_sub(1));
        let width = (x1 - x0 + 1).max(1);
        let height = (y1 - y0 + 1).max(1);
        self.origin.set((x0, y0));
        self.center.set((x, y));
        self.dims.set((width, height));
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: picking_texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: x0, y: y0, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(READBACK_BYTES_PER_ROW),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        Some(PendingPick {
            state: Arc::clone(&self.state),
            token: Arc::clone(&self.token),
            target,
        })
    }

    /// Map the staging buffer asynchronously. The pending handle flips to
    /// ready once the GPU copy can be read, or to failed if mapping fails.
    pub fn map_async(&self, pending: &PendingPick) {
        let state_for_cb = Arc::clone(&pending.state);
        self.buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |res| {
                if res.is_ok() {
                    state_for_cb.store(READBACK_READY, Ordering::Release);
                } else {
                    state_for_cb.store(READBACK_FAILED, Ordering::Release);
                }
            });
    }

    /// Estimated GPU bytes allocated by this readback staging buffer.
    pub fn memory_usage(&self) -> GpuMemoryUsage {
        buffer_usage(&self.buffer)
    }

    /// Collect the mapped pixel and unmap. Returns `None` if no atom was hit
    /// (cleared pixel == sentinel).
    pub fn try_collect(&self, pending: &PendingPick) -> Option<Option<PickHit>> {
        if !pending.belongs_to(&self.token) {
            return Some(None);
        }
        match pending.status() {
            READBACK_PENDING | READBACK_IDLE => return None,
            READBACK_FAILED => {
                self.state.store(READBACK_IDLE, Ordering::Release);
                return Some(None);
            }
            READBACK_READY => {}
            _ => {
                self.state.store(READBACK_IDLE, Ordering::Release);
                return Some(None);
            }
        }
        let slice = self.buffer.slice(..);
        let data = slice.get_mapped_range();
        let (origin_x, origin_y) = self.origin.get();
        let (center_x, center_y) = self.center.get();
        let (width, height) = self.dims.get();

        let mut best: Option<RankedHit> = None;
        for row in 0..height {
            let row_base = row as usize * READBACK_BYTES_PER_ROW as usize;
            for col in 0..width {
                let off = row_base + col as usize * BYTES_PER_PICKING_PIXEL;
                let r =
                    u32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]);
                let g = u32::from_le_bytes([
                    data[off + 4],
                    data[off + 5],
                    data[off + 6],
                    data[off + 7],
                ]);
                let Some(hit) = decode_pixel(r, g) else {
                    continue;
                };
                let px = origin_x + col;
                let py = origin_y + row;
                let dx = px as i32 - center_x as i32;
                let dy = py as i32 - center_y as i32;
                let candidate = RankedHit::new((dx * dx + dy * dy) as u32, hit);
                if best.map(|b| candidate < b).unwrap_or(true) {
                    best = Some(candidate);
                }
            }
        }
        drop(data);
        self.buffer.unmap();
        self.state.store(READBACK_IDLE, Ordering::Release);
        Some(best.map(|ranked| ranked.hit))
    }
}

#[derive(Clone, Copy, Debug)]
struct RankedHit {
    priority: u32,
    dist2: u32,
    hit: PickHit,
}

impl RankedHit {
    fn new(dist2: u32, hit: PickHit) -> Self {
        Self {
            priority: rep_pick_priority(hit.rep_kind),
            dist2,
            hit,
        }
    }
}

impl PartialEq for RankedHit {
    fn eq(&self, other: &Self) -> bool {
        (self.priority, self.dist2) == (other.priority, other.dist2)
    }
}

impl Eq for RankedHit {}

impl PartialOrd for RankedHit {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for RankedHit {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.priority, self.dist2).cmp(&(other.priority, other.dist2))
    }
}

fn rep_pick_priority(kind: RepKind) -> u32 {
    match kind {
        RepKind::Sphere => 0,
        RepKind::Stick | RepKind::Ellipsoid => 1,
        RepKind::Cartoon | RepKind::Ribbon => 2,
        RepKind::Surface | RepKind::Mesh => 3,
        RepKind::Line => 4,
        RepKind::Dot => 5,
        RepKind::None => 6,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicU8, Ordering},
        Arc,
    };

    use crate::picking::{ObjectId, PickHit, RepKind};

    use super::{PendingPick, PickReadbackTarget, RankedHit, READBACK_READY};

    fn hit(rep_kind: RepKind) -> PickHit {
        PickHit {
            instance: None,
            rep_kind,
            object_id: ObjectId(1),
            atom_id: 7,
        }
    }

    #[test]
    fn sphere_wins_local_neighborhood_over_broad_cartoon() {
        let center_cartoon = RankedHit::new(0, hit(RepKind::Cartoon));
        let nearby_sphere = RankedHit::new(1, hit(RepKind::Sphere));

        assert!(nearby_sphere < center_cartoon);
    }

    #[test]
    fn equal_rep_uses_nearest_pixel() {
        let near = RankedHit::new(1, hit(RepKind::Sphere));
        let far = RankedHit::new(4, hit(RepKind::Sphere));

        assert!(near < far);
    }

    #[test]
    fn pending_pick_matches_only_issuing_readback_token() {
        let issuing_token = Arc::new(());
        let replacement_token = Arc::new(());
        let pending = PendingPick {
            state: Arc::new(AtomicU8::new(READBACK_READY)),
            token: Arc::clone(&issuing_token),
            target: PickReadbackTarget::Hover,
        };

        assert!(pending.belongs_to(&issuing_token));
        assert!(!pending.belongs_to(&replacement_token));
        assert!(pending.is_ready());
        assert_eq!(pending.state.load(Ordering::Acquire), READBACK_READY);
    }
}

#[derive(Clone)]
pub struct PendingPick {
    state: Arc<AtomicU8>,
    token: Arc<()>,
    target: PickReadbackTarget,
}

impl PendingPick {
    pub fn is_ready(&self) -> bool {
        matches!(
            self.state.load(Ordering::Acquire),
            READBACK_READY | READBACK_FAILED
        )
    }

    pub fn target(&self) -> PickReadbackTarget {
        self.target
    }

    fn status(&self) -> u8 {
        self.state.load(Ordering::Acquire)
    }

    fn belongs_to(&self, token: &Arc<()>) -> bool {
        Arc::ptr_eq(&self.token, token)
    }
}
