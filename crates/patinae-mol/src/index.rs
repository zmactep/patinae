//! Type-safe index wrappers
//!
//! Provides newtype wrappers around indices to prevent accidentally mixing
//! atom indices with bond indices or coordinate set indices.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Invalid index marker value
pub const INVALID_INDEX: u32 = u32::MAX;

/// Macro to generate type-safe index types with common implementations.
/// This eliminates code duplication across AtomIndex, BondIndex, StateIndex, and CoordIndex.
macro_rules! define_index {
    (
        $(#[$meta:meta])*
        $name:ident, $debug_name:literal
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
        #[repr(transparent)]
        pub struct $name(pub u32);

        impl $name {
            /// Create a new index
            #[inline]
            pub const fn new(index: u32) -> Self {
                $name(index)
            }

            /// Get the raw index value as usize
            #[inline]
            pub const fn as_usize(&self) -> usize {
                self.0 as usize
            }

            /// Get the raw u32 value
            #[inline]
            pub const fn as_u32(&self) -> u32 {
                self.0
            }

            /// Check if this is a valid index
            #[inline]
            pub const fn is_valid(&self) -> bool {
                self.0 != INVALID_INDEX
            }

            /// Create an invalid index
            #[inline]
            pub const fn invalid() -> Self {
                $name(INVALID_INDEX)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                if self.is_valid() {
                    write!(f, "{}({})", $debug_name, self.0)
                } else {
                    write!(f, "{}(INVALID)", $debug_name)
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                if self.is_valid() {
                    write!(f, "{}", self.0)
                } else {
                    write!(f, "INVALID")
                }
            }
        }

        impl From<u32> for $name {
            #[inline]
            fn from(index: u32) -> Self {
                $name(index)
            }
        }

        impl From<usize> for $name {
            #[inline]
            fn from(index: usize) -> Self {
                $name(index as u32)
            }
        }

        impl From<$name> for u32 {
            #[inline]
            fn from(index: $name) -> Self {
                index.0
            }
        }

        impl From<$name> for usize {
            #[inline]
            fn from(index: $name) -> Self {
                index.0 as usize
            }
        }
    };
}

// Generate all index types using the macro

define_index!(
    /// Type-safe index into an atom array
    ///
    /// Prevents accidentally using a bond index where an atom index is expected.
    AtomIndex, "AtomIndex"
);

/// Maps atom indices across one topology edit.
///
/// Entries use indices from the molecule before the edit. A missing target
/// means that the source atom was removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtomRemap {
    old_to_new: Vec<Option<AtomIndex>>,
}

impl AtomRemap {
    /// Creates an identity mapping for `atom_count` atoms.
    pub(crate) fn identity(atom_count: usize) -> Self {
        let old_to_new = (0..atom_count)
            .map(|index| {
                let index = u32::try_from(index).expect("atom count must fit into AtomIndex");
                Some(AtomIndex(index))
            })
            .collect();
        Self { old_to_new }
    }

    pub(crate) fn from_mapping(old_to_new: Vec<Option<AtomIndex>>) -> Self {
        Self { old_to_new }
    }

    /// Returns the new index for one pre-edit atom.
    #[must_use]
    pub fn remap(&self, old_index: AtomIndex) -> Option<AtomIndex> {
        self.old_to_new.get(old_index.as_usize()).copied().flatten()
    }

    /// Returns whether this mapping leaves every atom index unchanged.
    #[must_use]
    pub fn is_identity(&self) -> bool {
        self.old_to_new
            .iter()
            .enumerate()
            .all(|(old, new)| new.is_some_and(|new| new.as_usize() == old))
    }
}

define_index!(
    /// Type-safe index into a bond array
    ///
    /// Prevents accidentally using an atom index where a bond index is expected.
    BondIndex, "BondIndex"
);

define_index!(
    /// Type-safe index into a coordinate set array
    ///
    /// Used to identify a specific state/frame in a multi-state molecule.
    StateIndex, "StateIndex"
);

define_index!(
    /// Index within a coordinate set (maps to atom index via lookup table)
    CoordIndex, "CoordIndex"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_atom_index() {
        let idx = AtomIndex::new(42);
        assert_eq!(idx.as_usize(), 42);
        assert_eq!(idx.as_u32(), 42);
        assert!(idx.is_valid());

        let invalid = AtomIndex::invalid();
        assert!(!invalid.is_valid());
    }

    #[test]
    fn test_bond_index() {
        let idx = BondIndex::new(10);
        assert_eq!(idx.as_usize(), 10);
        assert!(idx.is_valid());
    }

    #[test]
    fn test_state_index() {
        let idx = StateIndex::new(5);
        assert_eq!(idx.as_usize(), 5);
        assert!(idx.is_valid());
    }

    #[test]
    fn test_index_conversions() {
        let atom_idx: AtomIndex = 100u32.into();
        assert_eq!(u32::from(atom_idx), 100);

        let atom_idx: AtomIndex = 50usize.into();
        assert_eq!(usize::from(atom_idx), 50);
    }

    #[test]
    fn test_index_display() {
        assert_eq!(format!("{}", AtomIndex::new(5)), "5");
        assert_eq!(format!("{}", AtomIndex::invalid()), "INVALID");
        assert_eq!(format!("{:?}", AtomIndex::new(5)), "AtomIndex(5)");
        assert_eq!(format!("{:?}", AtomIndex::invalid()), "AtomIndex(INVALID)");
    }

    #[test]
    fn test_index_ordering() {
        let a = AtomIndex::new(1);
        let b = AtomIndex::new(2);
        assert!(a < b);
        assert!(b > a);
    }

    #[test]
    fn test_coord_index() {
        let idx = CoordIndex::new(7);
        assert_eq!(idx.as_usize(), 7);
        assert!(idx.is_valid());
        assert_eq!(format!("{:?}", idx), "CoordIndex(7)");
    }
}
