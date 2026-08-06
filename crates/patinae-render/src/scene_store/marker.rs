//! Per-atom marker bits stored in `SceneStore.marker_lut` (group 2,
//! binding 6). One `u32` per global atom id; bits encode UI state that
//! representation shaders consume.
//!
//! Bit layout (LE):
//!
//! | bits  | meaning                                                       |
//! |-------|---------------------------------------------------------------|
//! | 0     | selected — atom is in any visible host selection              |
//! | 1     | hover — atom is the current hover target (≤ 1 atom per scene) |
//! | 2     | recent — atom is present in the session's recent atom list    |
//! | 3     | reserved (transparency class / future lighting flags)         |
//! | 4-7   | reserved (marker color palette index)                         |
//! | 8-31  | reserved                                                      |
//!
//! Mirror in `shaders/common/scene.wgsl::scene_marker_*` helpers.

pub const MARKER_SELECTED: u32 = 1 << 0;
pub const MARKER_HOVER: u32 = 1 << 1;
pub const MARKER_RECENT: u32 = 1 << 2;

/// Pack the MVP marker bits (selected, hover) into a single `u32`.
#[inline]
pub fn pack(selected: bool, hover: bool) -> u32 {
    let mut bits = 0u32;
    if selected {
        bits |= MARKER_SELECTED;
    }
    if hover {
        bits |= MARKER_HOVER;
    }
    bits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_bits() {
        assert_eq!(pack(false, false), 0);
        assert_eq!(pack(true, false), MARKER_SELECTED);
        assert_eq!(pack(false, true), MARKER_HOVER);
        assert_eq!(pack(true, true), MARKER_SELECTED | MARKER_HOVER);
    }
}
