//! Picking — `RepKind`, `ObjectId`, `PackedId` (the sole bit-shuffler), `PickHit`.
//!
//! `PackedId` is the **one** place that knows the `Rg32Uint` bit layout. Any
//! shader or CPU code that produces or consumes a picking pixel goes through
//! `pack`/`unpack` — never hand-rolls the shifts.

pub mod pass;
pub mod readback;
pub mod reproject;

use crate::memory_policy::RenderMemoryPolicy;

pub use pass::{decode_pixel, PickingPass};
pub use readback::PickingReadback;

/// How the picking pass is recorded each frame.
///
/// Picking is the heaviest extra GPU pass we run on every dirty/camera-changed
/// frame. The mode is chosen at construction time and affects:
///
/// - whether the picking texture + picking_depth target are allocated at all;
/// - whether `PickingPass` is built;
/// - whether readback resources that read the hit-test picking texture are
///   wired in.
///
/// Hosts that never need pick / hover readback should pick `Disabled` to save
/// the Rg32Uint half-res hit-test target. Visual overlays are controlled
/// separately and use their own visible-id target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickingMode {
    /// No picking infrastructure. `RenderState::pick(...)` returns `None`;
    /// readbacks are silently disabled.
    Disabled,
    /// Re-record the full geometry picking pass on every
    /// `scene_dirty || camera_changed`. Higher fidelity, higher cost.
    FullRecord,
    /// Full re-record only on `scene_dirty`. On camera-only changes, run a
    /// compute reprojection kernel that warps the previous picking texture
    /// to the new view (with 2-pass atomic-min depth resolve, see
    /// `picking::reproject`). A watchdog forces a full re-record after a
    /// configurable angular drift or frame count to bound accumulated
    /// projection error.
    Reprojected,
}

/// Construction-time configuration for [`crate::RenderState`].
#[derive(Debug, Clone, Copy)]
pub struct RenderConfig {
    pub picking: PickingMode,
    pub selection_overlay: bool,
    pub memory: RenderMemoryPolicy,
}

impl Default for RenderConfig {
    fn default() -> Self {
        // Default to Reprojected — best FPS for the standard interactive
        // viewer path. Hosts that need the legacy `FullRecord` (e.g.
        // strict pixel-match testing) can pass it explicitly.
        Self {
            picking: PickingMode::Reprojected,
            selection_overlay: true,
            memory: RenderMemoryPolicy::performance(),
        }
    }
}

/// Which representation a picked pixel came from.
///
/// Stored in the top 4 bits of the picking pixel — at most 16 reps. Update
/// `RepKind::from_raw` if you add a variant.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RepKind {
    None = 0,
    Sphere = 1,
    Stick = 2,
    Line = 3,
    Dot = 4,
    Mesh = 5,
    Cartoon = 6,
    Surface = 7,
    Ribbon = 8,
    Ellipsoid = 9,
}

impl RepKind {
    pub fn as_raw(self) -> u8 {
        self as u8
    }

    pub fn from_raw(raw: u8) -> Self {
        match raw {
            1 => RepKind::Sphere,
            2 => RepKind::Stick,
            3 => RepKind::Line,
            4 => RepKind::Dot,
            5 => RepKind::Mesh,
            6 => RepKind::Cartoon,
            7 => RepKind::Surface,
            8 => RepKind::Ribbon,
            9 => RepKind::Ellipsoid,
            _ => RepKind::None,
        }
    }
}

/// Stable index of an object inside a `RenderInput`. Chosen by the host —
/// typically a slot id or the position in the `objects` array. Picking maps
/// pixels back to this id; the host resolves it to its own `ObjectMolecule`.
///
/// `ObjectId(0)` is reserved as the "no hit" sentinel — never assign 0 to a
/// real object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ObjectId(pub u32);

/// Single source of truth for the `Rg32Uint` picking layout.
///
/// ```text
/// .r = (rep_kind:4) | (object_id:12) | (atom_id_low:16)   // 32
/// .g = (atom_id_high:16) | (copy_plus_one:16)                  // 32
/// ```
///
/// The shader and the CPU both call `pack` / `unpack`. If you find yourself
/// shifting bits anywhere else, you are about to bypass the single-source
/// packing invariant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackedId {
    pub r: u32,
    pub g: u32,
}

impl PackedId {
    pub const NONE: PackedId = PackedId { r: 0, g: 0 };

    /// Pack `(rep_kind, object_id, atom_id)` into 64 bits.
    ///
    /// Caller guarantees `object_id < 4096` (12 bits) and that `rep_kind`
    /// fits in 4 bits — both are enforced via mask, not panic, so the
    /// behaviour is well-defined under wraparound.
    #[inline]
    pub fn pack(rep_kind: RepKind, object_id: ObjectId, atom_id: u32) -> Self {
        let kind = (rep_kind.as_raw() as u32) & 0xF;
        let obj = object_id.0 & 0xFFF;
        let atom_lo = atom_id & 0xFFFF;
        let atom_hi = (atom_id >> 16) & 0xFFFF;
        Self {
            r: (kind << 28) | (obj << 16) | atom_lo,
            g: atom_hi << 16,
        }
    }

    /// Adds a zero-based copy identity while preserving the source atom bits.
    ///
    /// # Panics
    /// Panics when the copy index exceeds the 65,535-copy representation limit.
    pub fn with_instance(mut self, instance: Option<u32>) -> Self {
        let encoded = instance.map_or(0, |index| {
            assert!(index < 65_535, "copy index exceeds picking capacity");
            index + 1
        });
        self.g = (self.g & 0xffff_0000) | encoded;
        self
    }

    /// Returns the zero-based copy identity encoded in the reserved low bits.
    pub fn instance(self) -> Option<u32> {
        (self.g & 0xffff).checked_sub(1)
    }

    /// Inverse of `pack`. Returns `None` for the cleared-pixel sentinel
    /// (rep_kind == 0, object_id == 0).
    #[inline]
    pub fn unpack(self) -> Option<(RepKind, ObjectId, u32)> {
        if self.r == 0 && self.g == 0 {
            return None;
        }
        let kind = RepKind::from_raw(((self.r >> 28) & 0xF) as u8);
        let obj = ObjectId((self.r >> 16) & 0xFFF);
        let atom_lo = self.r & 0xFFFF;
        let atom_hi = (self.g >> 16) & 0xFFFF;
        Some((kind, obj, (atom_hi << 16) | atom_lo))
    }
}

/// Resolved picking hit returned to the host. The host applies its own
/// selection-mode policy (atom / residue / chain / object) on top.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickHit {
    pub rep_kind: RepKind,
    pub object_id: ObjectId,
    /// Raw atom index as seen by the representation. For sphere this is the
    /// atom impostor's index; for cartoon it would be the Cα residue's
    /// representative atom.
    pub atom_id: u32,
    /// Zero-based copy identity; absent for explicit objects.
    pub instance: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_unpack_roundtrip() {
        let id = PackedId::pack(RepKind::Sphere, ObjectId(7), 12345);
        let (kind, obj, atom) = id.unpack().unwrap();
        assert_eq!(kind, RepKind::Sphere);
        assert_eq!(obj, ObjectId(7));
        assert_eq!(atom, 12345);
    }

    #[test]
    fn pack_unpack_high_atom_id() {
        // atom_id > 65535 forces the high half to be used.
        let id = PackedId::pack(RepKind::Cartoon, ObjectId(1), 1_500_000);
        let (kind, _obj, atom) = id.unpack().unwrap();
        assert_eq!(kind, RepKind::Cartoon);
        assert_eq!(atom, 1_500_000);
    }

    #[test]
    fn copy_identity_preserves_high_source_atom_bits() {
        for copy in [None, Some(0), Some(9), Some(65_534)] {
            let pixel =
                PackedId::pack(RepKind::Sphere, ObjectId(4095), u32::MAX).with_instance(copy);
            assert_eq!(
                pixel.unpack(),
                Some((RepKind::Sphere, ObjectId(4095), u32::MAX))
            );
            assert_eq!(pixel.instance(), copy);
            assert_eq!(
                super::decode_pixel(pixel.r, pixel.g).unwrap().instance,
                copy
            );
        }
    }

    #[test]
    fn cleared_pixel_unpacks_none() {
        assert!(PackedId::NONE.unpack().is_none());
    }

    #[test]
    fn object_id_truncation_documented() {
        // 12 bits → ids 0..=4095. Anything beyond wraps; documented behaviour.
        let id = PackedId::pack(RepKind::Sphere, ObjectId(0x1003), 0);
        let (_, obj, _) = id.unpack().unwrap();
        assert_eq!(obj, ObjectId(0x003));
    }

    #[test]
    fn removed_nucleic_ladder_raw_id_decodes_as_none() {
        assert_eq!(RepKind::from_raw(10), RepKind::None);
    }
}
