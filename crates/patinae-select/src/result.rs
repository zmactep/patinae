//! Selection result types
//!
//! Provides the `SelectionResult` type for efficiently representing
//! which atoms are selected using a bitset.

use bitvec::prelude::*;
use patinae_mol::AtomIndex;

/// A selection result representing which atoms are selected
///
/// Uses a bitset for efficient storage and set operations.
/// Each bit corresponds to an atom index in the molecule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionResult {
    /// Bitset where bit i is set if atom i is selected
    bits: BitVec<usize, Lsb0>,
}

impl SelectionResult {
    /// Create a new empty selection (no atoms selected)
    pub fn new(atom_count: usize) -> Self {
        SelectionResult {
            bits: bitvec![usize, Lsb0; 0; atom_count],
        }
    }

    /// Create a selection with all atoms selected
    pub fn all(atom_count: usize) -> Self {
        SelectionResult {
            bits: bitvec![usize, Lsb0; 1; atom_count],
        }
    }

    /// Create an alias for `new` for clarity
    pub fn none(atom_count: usize) -> Self {
        Self::new(atom_count)
    }

    /// Create a selection from an iterator of atom indices
    pub fn from_indices(atom_count: usize, indices: impl Iterator<Item = AtomIndex>) -> Self {
        let mut result = Self::new(atom_count);
        for idx in indices {
            result.set(idx);
        }
        result
    }

    /// Get the number of atoms this selection covers
    #[inline]
    pub fn atom_count(&self) -> usize {
        self.bits.len()
    }

    /// Check if an atom is selected
    #[inline]
    pub fn contains(&self, idx: AtomIndex) -> bool {
        self.bits.get(idx.as_usize()).map(|b| *b).unwrap_or(false)
    }

    /// Check if an atom at raw index is selected
    #[inline]
    pub fn contains_index(&self, idx: usize) -> bool {
        self.bits.get(idx).map(|b| *b).unwrap_or(false)
    }

    /// Set an atom as selected
    #[inline]
    pub fn set(&mut self, idx: AtomIndex) {
        if let Some(mut bit) = self.bits.get_mut(idx.as_usize()) {
            *bit = true;
        }
    }

    /// Set an atom at raw index as selected
    #[inline]
    pub fn set_index(&mut self, idx: usize) {
        if let Some(mut bit) = self.bits.get_mut(idx) {
            *bit = true;
        }
    }

    /// Unset (deselect) an atom
    #[inline]
    pub fn unset(&mut self, idx: AtomIndex) {
        if let Some(mut bit) = self.bits.get_mut(idx.as_usize()) {
            *bit = false;
        }
    }

    /// Unset an atom at raw index
    #[inline]
    pub fn unset_index(&mut self, idx: usize) {
        if let Some(mut bit) = self.bits.get_mut(idx) {
            *bit = false;
        }
    }

    /// Toggle an atom's selection state
    #[inline]
    pub fn toggle(&mut self, idx: AtomIndex) {
        if let Some(mut bit) = self.bits.get_mut(idx.as_usize()) {
            let current = *bit;
            *bit = !current;
        }
    }

    /// Count the number of selected atoms
    pub fn count(&self) -> usize {
        self.bits.count_ones()
    }

    /// Check if any atoms are selected
    #[inline]
    pub fn any(&self) -> bool {
        self.bits.any()
    }

    /// Check if no atoms are selected
    #[inline]
    pub fn is_empty(&self) -> bool {
        !self.any()
    }

    /// Returns whether every atom covered by this selection is selected.
    ///
    /// An empty selection covers no atoms and returns `true`. Callers applying
    /// a selection to a molecule must also check that their atom counts match.
    #[inline]
    pub fn is_all(&self) -> bool {
        self.bits.all()
    }

    /// Iterate over indices of selected atoms
    pub fn indices(&self) -> impl Iterator<Item = AtomIndex> + '_ {
        self.bits.iter_ones().map(|i| AtomIndex(i as u32))
    }

    /// Iterate over raw indices of selected atoms
    pub fn raw_indices(&self) -> impl Iterator<Item = usize> + '_ {
        self.bits.iter_ones()
    }

    /// Get the first selected atom index
    pub fn first(&self) -> Option<AtomIndex> {
        self.bits.first_one().map(|i| AtomIndex(i as u32))
    }

    /// Get the last selected atom index
    pub fn last(&self) -> Option<AtomIndex> {
        self.bits.last_one().map(|i| AtomIndex(i as u32))
    }

    // =========================================================================
    // Set Operations
    // =========================================================================

    /// Union of two selections (OR)
    ///
    /// Returns atoms selected in either selection.
    pub fn union(&self, other: &Self) -> Self {
        assert_eq!(
            self.bits.len(),
            other.bits.len(),
            "Selection sizes must match"
        );
        let mut result = self.clone();
        result.bits |= &other.bits;
        result
    }

    /// Intersection of two selections (AND)
    ///
    /// Returns atoms selected in both selections.
    pub fn intersection(&self, other: &Self) -> Self {
        assert_eq!(
            self.bits.len(),
            other.bits.len(),
            "Selection sizes must match"
        );
        let mut result = self.clone();
        result.bits &= &other.bits;
        result
    }

    /// Difference of two selections (self AND NOT other)
    ///
    /// Returns atoms selected in self but not in other.
    pub fn difference(&self, other: &Self) -> Self {
        assert_eq!(
            self.bits.len(),
            other.bits.len(),
            "Selection sizes must match"
        );
        let mut result = self.clone();
        for i in other.bits.iter_ones() {
            result.bits.set(i, false);
        }
        result
    }

    /// Symmetric difference (XOR)
    ///
    /// Returns atoms selected in exactly one of the selections.
    pub fn symmetric_difference(&self, other: &Self) -> Self {
        assert_eq!(
            self.bits.len(),
            other.bits.len(),
            "Selection sizes must match"
        );
        let mut result = self.clone();
        result.bits ^= &other.bits;
        result
    }

    /// Complement (NOT)
    ///
    /// Returns atoms not in this selection.
    pub fn complement(&self) -> Self {
        let mut result = self.clone();
        result.bits = !result.bits;
        result
    }

    // =========================================================================
    // In-place Set Operations
    // =========================================================================

    /// In-place union
    pub fn union_with(&mut self, other: &Self) {
        assert_eq!(
            self.bits.len(),
            other.bits.len(),
            "Selection sizes must match"
        );
        self.bits |= &other.bits;
    }

    /// In-place intersection
    pub fn intersect_with(&mut self, other: &Self) {
        assert_eq!(
            self.bits.len(),
            other.bits.len(),
            "Selection sizes must match"
        );
        self.bits &= &other.bits;
    }

    /// In-place difference
    pub fn subtract(&mut self, other: &Self) {
        assert_eq!(
            self.bits.len(),
            other.bits.len(),
            "Selection sizes must match"
        );
        for i in other.bits.iter_ones() {
            self.bits.set(i, false);
        }
    }

    /// In-place complement (invert)
    pub fn invert(&mut self) {
        self.bits = !self.bits.clone();
    }

    /// Clear all selections
    pub fn clear(&mut self) {
        self.bits.fill(false);
    }

    /// Select all atoms
    pub fn select_all(&mut self) {
        self.bits.fill(true);
    }
}

impl Default for SelectionResult {
    fn default() -> Self {
        Self::new(0)
    }
}

impl std::fmt::Display for SelectionResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "SelectionResult({} of {} atoms)",
            self.count(),
            self.atom_count()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_selection_ignores_unused_storage_bits() {
        for atom_count in [0, 1, 63, 64, 65, 129] {
            let mut selected = SelectionResult::all(atom_count);
            assert!(selected.is_all());
            if atom_count != 0 {
                selected.unset_index(atom_count - 1);
                assert!(!selected.is_all());
                selected.set_index(atom_count - 1);
                assert!(selected.is_all());
                assert!(!SelectionResult::none(atom_count).is_all());
            }
        }
    }

    #[test]
    fn test_new_selection() {
        let sel = SelectionResult::new(100);
        assert_eq!(sel.atom_count(), 100);
        assert_eq!(sel.count(), 0);
        assert!(sel.is_empty());
    }

    #[test]
    fn test_all_selection() {
        let sel = SelectionResult::all(100);
        assert_eq!(sel.atom_count(), 100);
        assert_eq!(sel.count(), 100);
        assert!(!sel.is_empty());
    }

    #[test]
    fn test_set_unset() {
        let mut sel = SelectionResult::new(10);

        sel.set(AtomIndex(5));
        assert!(sel.contains(AtomIndex(5)));
        assert_eq!(sel.count(), 1);

        sel.unset(AtomIndex(5));
        assert!(!sel.contains(AtomIndex(5)));
        assert_eq!(sel.count(), 0);
    }

    #[test]
    fn test_toggle() {
        let mut sel = SelectionResult::new(10);

        sel.toggle(AtomIndex(5));
        assert!(sel.contains(AtomIndex(5)));

        sel.toggle(AtomIndex(5));
        assert!(!sel.contains(AtomIndex(5)));
    }

    #[test]
    fn test_indices() {
        let mut sel = SelectionResult::new(10);
        sel.set(AtomIndex(1));
        sel.set(AtomIndex(5));
        sel.set(AtomIndex(9));

        let indices: Vec<AtomIndex> = sel.indices().collect();
        assert_eq!(indices, vec![AtomIndex(1), AtomIndex(5), AtomIndex(9)]);
    }

    #[test]
    fn test_first_last() {
        let mut sel = SelectionResult::new(10);
        sel.set(AtomIndex(3));
        sel.set(AtomIndex(7));

        assert_eq!(sel.first(), Some(AtomIndex(3)));
        assert_eq!(sel.last(), Some(AtomIndex(7)));

        let empty = SelectionResult::new(10);
        assert_eq!(empty.first(), None);
        assert_eq!(empty.last(), None);
    }

    #[test]
    fn test_union() {
        let mut sel1 = SelectionResult::new(10);
        sel1.set(AtomIndex(1));
        sel1.set(AtomIndex(2));

        let mut sel2 = SelectionResult::new(10);
        sel2.set(AtomIndex(2));
        sel2.set(AtomIndex(3));

        let union = sel1.union(&sel2);
        assert!(union.contains(AtomIndex(1)));
        assert!(union.contains(AtomIndex(2)));
        assert!(union.contains(AtomIndex(3)));
        assert_eq!(union.count(), 3);
    }

    #[test]
    fn test_intersection() {
        let mut sel1 = SelectionResult::new(10);
        sel1.set(AtomIndex(1));
        sel1.set(AtomIndex(2));

        let mut sel2 = SelectionResult::new(10);
        sel2.set(AtomIndex(2));
        sel2.set(AtomIndex(3));

        let intersection = sel1.intersection(&sel2);
        assert!(!intersection.contains(AtomIndex(1)));
        assert!(intersection.contains(AtomIndex(2)));
        assert!(!intersection.contains(AtomIndex(3)));
        assert_eq!(intersection.count(), 1);
    }

    #[test]
    fn test_difference() {
        let mut sel1 = SelectionResult::new(10);
        sel1.set(AtomIndex(1));
        sel1.set(AtomIndex(2));

        let mut sel2 = SelectionResult::new(10);
        sel2.set(AtomIndex(2));
        sel2.set(AtomIndex(3));

        let diff = sel1.difference(&sel2);
        assert!(diff.contains(AtomIndex(1)));
        assert!(!diff.contains(AtomIndex(2)));
        assert!(!diff.contains(AtomIndex(3)));
        assert_eq!(diff.count(), 1);
    }

    #[test]
    fn test_complement() {
        let mut sel = SelectionResult::new(5);
        sel.set(AtomIndex(1));
        sel.set(AtomIndex(3));

        let comp = sel.complement();
        assert!(comp.contains(AtomIndex(0)));
        assert!(!comp.contains(AtomIndex(1)));
        assert!(comp.contains(AtomIndex(2)));
        assert!(!comp.contains(AtomIndex(3)));
        assert!(comp.contains(AtomIndex(4)));
        assert_eq!(comp.count(), 3);
    }

    #[test]
    fn test_from_indices() {
        let indices = vec![AtomIndex(1), AtomIndex(5), AtomIndex(9)];
        let sel = SelectionResult::from_indices(10, indices.into_iter());

        assert!(sel.contains(AtomIndex(1)));
        assert!(sel.contains(AtomIndex(5)));
        assert!(sel.contains(AtomIndex(9)));
        assert_eq!(sel.count(), 3);
    }

    #[test]
    fn test_display() {
        let mut sel = SelectionResult::new(100);
        sel.set(AtomIndex(1));
        sel.set(AtomIndex(2));

        assert_eq!(format!("{}", sel), "SelectionResult(2 of 100 atoms)");
    }
}
