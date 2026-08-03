//! Molecular object container
//!
//! Provides the `ObjectMolecule` struct which is the main container for molecular data.
//! It holds atoms, bonds, coordinate sets, and settings.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use lin_alg::f32::{Mat4, Vec3};
use patinae_settings::{GlobalSettings, UniqueSettings};
use smallvec::SmallVec;

use crate::atom::Atom;
use crate::bond::{Bond, BondOrder};
use crate::coordset::{CoordSet, Symmetry};
use crate::error::{MolError, MolResult};
use crate::index::{AtomIndex, AtomRemap, BondIndex, StateIndex};
use crate::residue::{ChainIterator, ResidueIterator};
use crate::subchain::{PartitionSubchainIter, SubchainPartition};

/// A molecular object containing atoms, bonds, and coordinate sets
///
/// Molecular data model (atoms, bonds, coordinate sets).
/// It provides a container for all molecular data including:
/// - Atom properties (name, element, residue, etc.)
/// - Bond connectivity
/// - Coordinates for one or more states (conformations)
/// - Per-object and per-atom/bond settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectMolecule {
    // =========================================================================
    // Core Data
    // =========================================================================
    /// All atoms in the molecule
    pub(crate) atoms: Vec<Atom>,

    /// All bonds in the molecule
    pub(crate) bonds: Vec<Bond>,

    /// Coordinate sets (one per state/frame)
    pub(crate) coord_sets: Vec<CoordSet>,

    // =========================================================================
    // Bond Lookup
    // =========================================================================
    /// Map from atom index to list of bond indices involving that atom
    /// Uses SmallVec to avoid heap allocation for atoms with 4 or fewer bonds (common case)
    /// Skipped during serialization — rebuilt from bonds via `rebuild_atom_bonds()`.
    #[serde(skip)]
    pub(crate) atom_bonds: Vec<SmallVec<[BondIndex; 4]>>,

    // =========================================================================
    // Metadata
    // =========================================================================
    /// Object name
    pub name: String,

    /// Title or description
    pub title: String,

    /// Current state index (0-based)
    pub current_state: usize,

    // =========================================================================
    // Discrete Mode
    // =========================================================================
    /// Whether this is a discrete object
    /// In discrete mode, each state has independent atoms (not shared)
    pub discrete: bool,

    // =========================================================================
    // Settings
    // =========================================================================
    /// Per-object settings
    pub settings: Option<GlobalSettings>,

    /// Per-atom/bond unique settings
    pub unique_settings: UniqueSettings,

    // =========================================================================
    // Symmetry
    // =========================================================================
    /// Crystallographic symmetry (applies to all states unless overridden)
    pub symmetry: Option<Symmetry>,

    // =========================================================================
    // Caches
    // =========================================================================
    /// Lazily-built bond-aware subchain partition. Populated on first call to
    /// [`subchain_partition`](Self::subchain_partition); invalidated by any
    /// mutation that touches bonds or atom classification flags.
    #[serde(skip)]
    pub(crate) subchain_partition: OnceLock<SubchainPartition>,
}

impl Default for ObjectMolecule {
    fn default() -> Self {
        ObjectMolecule {
            atoms: Vec::new(),
            bonds: Vec::new(),
            coord_sets: Vec::new(),
            atom_bonds: Vec::new(),
            name: String::new(),
            title: String::new(),
            current_state: 0,
            discrete: false,
            settings: None,
            unique_settings: UniqueSettings::new(),
            symmetry: None,
            subchain_partition: OnceLock::new(),
        }
    }
}

impl ObjectMolecule {
    /// Create a new empty molecule with the given name
    pub fn new(name: impl Into<String>) -> Self {
        ObjectMolecule {
            name: name.into(),
            ..Default::default()
        }
    }

    /// Rebuilds derived atom lookup tables.
    ///
    /// This must be called after deserialization because bond lookups are
    /// derived data and skipped during serialization.
    pub fn rebuild_atom_bonds(&mut self) {
        self.atom_bonds = vec![SmallVec::new(); self.atoms.len()];
        for (bi, bond) in self.bonds.iter().enumerate() {
            let bond_idx = BondIndex(bi as u32);
            if bond.atom1.as_usize() < self.atoms.len() {
                self.atom_bonds[bond.atom1.as_usize()].push(bond_idx);
            }
            if bond.atom2.as_usize() < self.atoms.len() {
                self.atom_bonds[bond.atom2.as_usize()].push(bond_idx);
            }
        }
        self.invalidate_subchain_partition();
    }

    /// Bond-aware partition of atoms into subchains. Lazily built and cached.
    ///
    /// Connected HET (Organic / Lipid) components in the covalent-bond graph
    /// are merged into a single subchain — e.g. a multi-residue N-glycan
    /// (NAG–NAG–BMA–MAN…) becomes one entry instead of one per residue.
    /// Biopolymer / solvent / inorganic runs remain grouped by `(chain, resn)`.
    pub fn subchain_partition(&self) -> &SubchainPartition {
        self.subchain_partition
            .get_or_init(|| SubchainPartition::from_molecule(&self.atoms, &self.bonds))
    }

    /// Drop the cached [`SubchainPartition`]. Call this after any mutation
    /// that changes bonds, atom flags, or atom residue/chain identity.
    pub fn invalidate_subchain_partition(&mut self) {
        self.subchain_partition = OnceLock::new();
    }

    /// Create a molecule with pre-allocated capacity
    pub fn with_capacity(name: impl Into<String>, atoms: usize, bonds: usize) -> Self {
        ObjectMolecule {
            atoms: Vec::with_capacity(atoms),
            bonds: Vec::with_capacity(bonds),
            coord_sets: Vec::new(),
            atom_bonds: Vec::with_capacity(atoms),
            name: name.into(),
            title: String::new(),
            current_state: 0,
            discrete: false,
            settings: None,
            unique_settings: UniqueSettings::new(),
            symmetry: None,
            subchain_partition: OnceLock::new(),
        }
    }

    // =========================================================================
    // Atom Operations
    // =========================================================================

    /// Add an atom to the molecule
    ///
    /// Returns the index of the newly added atom.
    pub fn add_atom(&mut self, atom: Atom) -> AtomIndex {
        let index = AtomIndex(self.atoms.len() as u32);
        self.atoms.push(atom);
        self.atom_bonds.push(SmallVec::new());
        self.invalidate_subchain_partition();
        index
    }

    /// Add an atom and give it `coord` in every existing coordinate set.
    ///
    /// Convenience for growing a loaded molecule (e.g. when building hydrogens):
    /// [`add_atom`](Self::add_atom) alone leaves coordinate sets short, which
    /// makes [`set_coord`](Self::set_coord) fail for the new atom.
    pub fn push_atom_with_coord(&mut self, atom: Atom, coord: Vec3) -> AtomIndex {
        let index = self.add_atom(atom);
        for coord_set in &mut self.coord_sets {
            coord_set.add_coord(index, coord);
        }
        index
    }

    /// Get an atom by index
    #[inline]
    pub fn get_atom(&self, index: AtomIndex) -> Option<&Atom> {
        self.atoms.get(index.as_usize())
    }

    /// Get a mutable reference to an atom by index
    #[inline]
    pub fn get_atom_mut(&mut self, index: AtomIndex) -> Option<&mut Atom> {
        self.atoms.get_mut(index.as_usize())
    }

    /// Get the number of atoms
    #[inline]
    pub fn atom_count(&self) -> usize {
        self.atoms.len()
    }

    /// Iterate over all atoms
    pub fn atoms(&self) -> impl Iterator<Item = &Atom> {
        self.atoms.iter()
    }

    /// Iterate over all atoms with indices
    pub fn atoms_indexed(&self) -> impl Iterator<Item = (AtomIndex, &Atom)> {
        self.atoms
            .iter()
            .enumerate()
            .map(|(i, a)| (AtomIndex(i as u32), a))
    }

    /// Get the atoms slice
    pub fn atoms_slice(&self) -> &[Atom] {
        &self.atoms
    }

    /// Get mutable atoms slice
    pub fn atoms_slice_mut(&mut self) -> &mut [Atom] {
        &mut self.atoms
    }

    /// Iterate over all atoms mutably
    pub fn atoms_mut(&mut self) -> impl Iterator<Item = &mut Atom> {
        self.atoms.iter_mut()
    }

    // =========================================================================
    // Bond Operations
    // =========================================================================

    /// Add a bond between two atoms
    ///
    /// Returns the index of the newly added bond.
    /// Returns an error if the atoms don't exist or a bond already exists.
    pub fn add_bond(
        &mut self,
        atom1: AtomIndex,
        atom2: AtomIndex,
        order: BondOrder,
    ) -> MolResult<BondIndex> {
        // Validate atoms exist
        let n_atoms = self.atoms.len();
        if atom1.as_usize() >= n_atoms {
            return Err(MolError::atom_out_of_bounds(atom1.0, n_atoms));
        }
        if atom2.as_usize() >= n_atoms {
            return Err(MolError::atom_out_of_bounds(atom2.0, n_atoms));
        }

        // Don't allow self-loops
        if atom1 == atom2 {
            return Err(MolError::InvalidBond(atom1.0, atom2.0));
        }

        // Check for duplicate bond
        if self.find_bond(atom1, atom2).is_some() {
            return Err(MolError::DuplicateBond(atom1.0, atom2.0));
        }

        // Create the bond
        let bond = Bond::new(atom1, atom2, order);
        let bond_index = BondIndex(self.bonds.len() as u32);
        self.bonds.push(bond);

        // Update atom-to-bond mapping
        self.atom_bonds[atom1.as_usize()].push(bond_index);
        self.atom_bonds[atom2.as_usize()].push(bond_index);

        // Mark atoms as bonded
        if let Some(a1) = self.atoms.get_mut(atom1.as_usize()) {
            a1.state.bonded = true;
        }
        if let Some(a2) = self.atoms.get_mut(atom2.as_usize()) {
            a2.state.bonded = true;
        }

        self.invalidate_subchain_partition();
        Ok(bond_index)
    }

    /// Add a bond (simplified version that ignores errors for duplicate bonds)
    pub fn add_bond_unchecked(
        &mut self,
        atom1: AtomIndex,
        atom2: AtomIndex,
        order: BondOrder,
    ) -> Option<BondIndex> {
        self.add_bond(atom1, atom2, order).ok()
    }

    /// Get a bond by index
    #[inline]
    pub fn get_bond(&self, index: BondIndex) -> Option<&Bond> {
        self.bonds.get(index.as_usize())
    }

    /// Get a mutable reference to a bond by index
    #[inline]
    pub fn get_bond_mut(&mut self, index: BondIndex) -> Option<&mut Bond> {
        self.bonds.get_mut(index.as_usize())
    }

    /// Find a bond between two atoms
    pub fn find_bond(&self, atom1: AtomIndex, atom2: AtomIndex) -> Option<BondIndex> {
        // Use the smaller atom's bond list for efficiency
        let smaller = if self.atom_bonds.get(atom1.as_usize())?.len()
            < self.atom_bonds.get(atom2.as_usize())?.len()
        {
            atom1
        } else {
            atom2
        };

        for &bond_idx in &self.atom_bonds[smaller.as_usize()] {
            if let Some(bond) = self.bonds.get(bond_idx.as_usize()) {
                if bond.connects(atom1, atom2) {
                    return Some(bond_idx);
                }
            }
        }
        None
    }

    /// Get all atoms bonded to a given atom
    pub fn bonded_atoms(&self, atom: AtomIndex) -> Vec<AtomIndex> {
        let mut result = Vec::new();
        if let Some(bond_indices) = self.atom_bonds.get(atom.as_usize()) {
            for &bond_idx in bond_indices {
                if let Some(bond) = self.bonds.get(bond_idx.as_usize()) {
                    if let Some(other) = bond.other(atom) {
                        result.push(other);
                    }
                }
            }
        }
        result
    }

    /// Get all atoms bonded to a given atom as a non-allocating iterator
    pub fn bonded_atoms_iter(&self, atom: AtomIndex) -> impl Iterator<Item = AtomIndex> + '_ {
        self.atom_bonds
            .get(atom.as_usize())
            .into_iter()
            .flat_map(|indices| indices.iter())
            .filter_map(move |&bi| self.bonds.get(bi.as_usize())?.other(atom))
    }

    /// Get the bond indices for a given atom
    pub fn atom_bond_indices(&self, atom: AtomIndex) -> &[BondIndex] {
        self.atom_bonds
            .get(atom.as_usize())
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Get the number of bonds
    #[inline]
    pub fn bond_count(&self) -> usize {
        self.bonds.len()
    }

    /// Iterate over all bonds
    pub fn bonds(&self) -> impl Iterator<Item = &Bond> {
        self.bonds.iter()
    }

    /// Iterate over all bonds with indices
    pub fn bonds_indexed(&self) -> impl Iterator<Item = (BondIndex, &Bond)> {
        self.bonds
            .iter()
            .enumerate()
            .map(|(i, b)| (BondIndex(i as u32), b))
    }

    /// Generate bonds based on inter-atomic distances
    ///
    /// Creates bonds between atoms that are within bonding distance based on
    /// their covalent radii (Cordero et al. 2008). A bond is created if:
    ///   distance < cov_radius(a) + cov_radius(b) + tolerance
    ///
    /// Use `DEFAULT_BOND_TOLERANCE` (0.45 Å) for standard bonding.
    ///
    /// This method operates on the first coordinate set (state 0).
    pub fn generate_bonds(&mut self, tolerance: f32) {
        crate::bonding::generate_bonds_for_state(self, tolerance, 0);
        self.invalidate_subchain_partition();
    }

    /// Remove all bonds from the molecule.
    ///
    /// Clears the bond list, atom-to-bond lookup table, and per-atom bonded
    /// flags while keeping atoms, residues, and coordinates intact.
    pub fn clear_bonds(&mut self) {
        self.bonds.clear();
        for ab in &mut self.atom_bonds {
            ab.clear();
        }
        for atom in &mut self.atoms {
            atom.state.bonded = false;
        }
        self.invalidate_subchain_partition();
    }

    /// Assign bond orders for known protein residues based on atom names
    ///
    /// This function uses PDB atom name conventions to assign bond orders for
    /// standard amino acids. Unlike distance-based inference, this method is
    /// chemically accurate because it uses the known structure of amino acids.
    pub fn assign_known_residue_bond_orders(&mut self) {
        crate::bonding::assign_known_residue_bond_orders(self);
        self.invalidate_subchain_partition();
    }

    /// Clear all bonds and re-generate from scratch.
    ///
    /// This is used after loading CCD templates to replace distance-based bonds
    /// with template-based bonds for known HETATM residues.
    pub fn rebond(&mut self, tolerance: f32) {
        self.clear_bonds();
        self.generate_bonds(tolerance);
        self.assign_known_residue_bond_orders();
        self.invalidate_subchain_partition();
    }

    // =========================================================================
    // Coordinate Set Operations
    // =========================================================================

    /// Add a coordinate set
    ///
    /// Returns the state index of the newly added coordinate set.
    pub fn add_coord_set(&mut self, coord_set: CoordSet) -> StateIndex {
        let index = StateIndex(self.coord_sets.len() as u32);
        self.coord_sets.push(coord_set);
        index
    }

    /// Remove all coordinate sets
    pub fn clear_coord_sets(&mut self) {
        self.coord_sets.clear();
        self.current_state = 0;
    }

    /// Remove specific states (coordinate sets) by 0-indexed indices.
    ///
    /// Indices must be sorted and deduplicated. Out-of-range indices are
    /// silently ignored. After removal, `current_state` is clamped to the
    /// valid range.
    pub fn remove_states(&mut self, indices: &[usize]) {
        for &idx in indices.iter().rev() {
            if idx < self.coord_sets.len() {
                self.coord_sets.remove(idx);
            }
        }
        if self.coord_sets.is_empty() {
            self.current_state = 0;
        } else {
            self.current_state = self.current_state.min(self.coord_sets.len() - 1);
        }
    }

    /// Get a coordinate set by state index
    #[inline]
    pub fn get_coord_set(&self, state: usize) -> Option<&CoordSet> {
        self.coord_sets.get(state)
    }

    /// Get the current coordinate set
    pub fn current_coord_set(&self) -> Option<&CoordSet> {
        self.coord_sets.get(self.current_state)
    }

    /// Get the number of states (coordinate sets)
    #[inline]
    pub fn state_count(&self) -> usize {
        self.coord_sets.len()
    }

    /// Check if the molecule has any coordinates
    #[inline]
    pub fn has_coords(&self) -> bool {
        !self.coord_sets.is_empty()
    }

    // =========================================================================
    // Coordinate Access (Convenience)
    // =========================================================================

    /// Get coordinates for an atom in a specific state
    pub fn get_coord(&self, atom: AtomIndex, state: usize) -> Option<Vec3> {
        self.coord_sets.get(state)?.get_atom_coord(atom)
    }

    /// Set coordinates for an atom in a specific state
    pub fn set_coord(&mut self, atom: AtomIndex, state: usize, coord: Vec3) -> bool {
        if let Some(cs) = self.coord_sets.get_mut(state) {
            cs.set_atom_coord(atom, coord)
        } else {
            false
        }
    }

    // =========================================================================
    // Residue/Chain Iteration
    // =========================================================================

    /// Iterate over residues in the molecule
    pub fn residues(&self) -> ResidueIterator<'_> {
        ResidueIterator::new(&self.atoms, 0)
    }

    /// Iterate over chains in the molecule.
    ///
    /// Each yielded `ChainView` covers all atoms sharing one chain identifier
    /// — including biopolymer, HET, and solvent atoms. Use
    /// [`ChainView::subchains`](crate::ChainView::subchains) to descend into the
    /// polymer / HET / solvent split, or [`subchains`](Self::subchains) below for
    /// a flat subchain iterator over the entire molecule.
    pub fn chains(&self) -> ChainIterator<'_> {
        ChainIterator::new(&self.atoms, 0)
    }

    /// Iterate over chain subchains in the molecule.
    ///
    /// Walks the bond-aware [`SubchainPartition`], so connected HET components
    /// (e.g. multi-residue glycans) appear as a single `SubchainView` rather
    /// than one per residue.
    pub fn subchains(&self) -> PartitionSubchainIter<'_> {
        PartitionSubchainIter::new(&self.atoms, self.subchain_partition())
    }

    /// Count the number of residues
    pub fn residue_count(&self) -> usize {
        self.residues().count()
    }

    // =========================================================================
    // Analysis
    // =========================================================================

    /// Calculate the center of mass (geometric center) for a state
    pub fn center(&self, state: usize) -> Option<Vec3> {
        self.coord_sets.get(state).map(|cs| cs.center())
    }

    /// Calculate the bounding box for a state
    pub fn bounding_box(&self, state: usize) -> Option<(Vec3, Vec3)> {
        self.coord_sets.get(state)?.bounding_box()
    }

    // =========================================================================
    // Atom Classification
    // =========================================================================

    /// Classify atoms as protein, nucleic acid, solvent, organic, or inorganic
    ///
    /// This method sets `AtomFlags` on atoms based on their residue names and
    /// other properties. Classification logic:
    ///
    /// - Known amino acid residues (non-HETATM) → `PROTEIN | POLYMER`
    /// - Known nucleic acid residues (non-HETATM) → `NUCLEIC | POLYMER`
    /// - Water residues → `SOLVENT`
    /// - CA atoms in protein residues → `GUIDE` flag (for cartoon)
    ///
    /// This method should be called after loading atoms from a file or
    /// after programmatic molecule creation to enable cartoon representation.
    pub fn classify_atoms(&mut self) {
        use crate::flags::AtomFlags;
        use crate::residue::{is_amino_acid, is_nucleotide, is_water};

        // We need to collect residue information first since we can't borrow
        // self.atoms mutably while iterating through residues
        let residue_info: Vec<(std::ops::Range<usize>, AtomFlags, Option<usize>)> = self
            .residues()
            .map(|residue| {
                let resn = residue.resn();
                let is_hetatm = residue
                    .atoms
                    .first()
                    .map(|a| a.state.hetatm)
                    .unwrap_or(true);

                // Determine classification flags
                // Note: nucleotides are recognised even with HETATM (common for
                // modified bases), but proteins keep the !is_hetatm guard because
                // small peptides can legitimately be HETATM ligands.
                let mask = if !is_hetatm && is_amino_acid(resn) {
                    AtomFlags::POLYMER | AtomFlags::PROTEIN
                } else if is_nucleotide(resn) {
                    AtomFlags::POLYMER | AtomFlags::NUCLEIC
                } else if is_water(resn) {
                    AtomFlags::SOLVENT
                } else {
                    // Non-polymer atoms: ligands, ions, etc.
                    // Classify based on content (carbon → organic, otherwise inorganic)
                    if residue.atoms.iter().any(|a| a.element.is_carbon()) {
                        AtomFlags::ORGANIC
                    } else {
                        AtomFlags::INORGANIC
                    }
                };

                // Find guide atom (CA for proteins, C4' for nucleic acids)
                let guide_atom_local_idx = if mask.contains(AtomFlags::PROTEIN) {
                    // Find CA atom
                    residue.atoms.iter().position(|a| &*a.name == "CA")
                } else if mask.contains(AtomFlags::NUCLEIC) {
                    // Find C4' or C4* atom
                    residue
                        .atoms
                        .iter()
                        .position(|a| &*a.name == "C4'" || &*a.name == "C4*")
                } else {
                    None
                };

                // Convert local index to global index
                let guide_atom_global_idx =
                    guide_atom_local_idx.map(|local| residue.atom_range.start + local);

                (residue.atom_range, mask, guide_atom_global_idx)
            })
            .collect();

        // Apply flags to atoms
        for (range, mask, guide_idx) in residue_info {
            let is_non_polymer = !mask.contains(AtomFlags::POLYMER);
            for idx in range {
                if let Some(atom) = self.atoms.get_mut(idx) {
                    atom.state.flags |= mask;
                    // Mark non-polymer atoms as hetatm for proper representation
                    // Non-polymer atoms need HETATM classification for proper representation
                    if is_non_polymer {
                        atom.state.hetatm = true;
                    }
                }
            }
            // Set guide flag on the guide atom
            if let Some(guide) = guide_idx {
                if let Some(atom) = self.atoms.get_mut(guide) {
                    atom.state.flags |= AtomFlags::GUIDE;
                }
            }
        }
        self.invalidate_subchain_partition();
    }

    // =========================================================================
    // Chain Assignment
    // =========================================================================

    /// Assign chain IDs to atoms that have no chain information
    ///
    /// This is designed for formats like GRO that lack chain identifiers.
    /// The algorithm classifies residues by type (protein, nucleic, solvent,
    /// ion, lipid, other) and assigns sequential chain letters.
    ///
    /// Must be called after `classify_atoms()` and `generate_bonds()`.
    pub fn assign_chains(&mut self) {
        crate::chains::assign_chains(self);
    }

    // =========================================================================
    // Transformation
    // =========================================================================

    /// Translate all coordinates in a state
    pub fn translate(&mut self, state: usize, delta: Vec3) {
        if let Some(cs) = self.coord_sets.get_mut(state) {
            cs.translate(delta);
        }
    }

    /// Center coordinates at the origin for a state
    pub fn center_origin(&mut self, state: usize) {
        if let Some(cs) = self.coord_sets.get_mut(state) {
            cs.center_origin();
        }
    }

    /// Translate specific atoms in a state
    pub fn translate_atoms(&mut self, state: usize, atoms: &[AtomIndex], delta: Vec3) {
        if let Some(cs) = self.coord_sets.get_mut(state) {
            cs.translate_atoms(atoms, delta);
        }
    }

    /// Rotate all coordinates in a state
    ///
    /// # Arguments
    /// * `state` - State index (0-based)
    /// * `axis` - Rotation axis (will be normalized)
    /// * `angle` - Rotation angle in radians
    /// * `origin` - Center of rotation
    pub fn rotate(&mut self, state: usize, axis: Vec3, angle: f32, origin: Vec3) {
        if let Some(cs) = self.coord_sets.get_mut(state) {
            cs.rotate(axis, angle, origin);
        }
    }

    /// Rotate specific atoms in a state
    pub fn rotate_atoms(
        &mut self,
        state: usize,
        atoms: &[AtomIndex],
        axis: Vec3,
        angle: f32,
        origin: Vec3,
    ) {
        if let Some(cs) = self.coord_sets.get_mut(state) {
            cs.rotate_atoms(atoms, axis, angle, origin);
        }
    }

    /// Apply a 4x4 transformation matrix to all coordinates in a state
    pub fn transform(&mut self, state: usize, matrix: &Mat4) {
        if let Some(cs) = self.coord_sets.get_mut(state) {
            cs.transform(matrix);
        }
    }

    /// Apply a 4x4 transformation matrix to specific atoms in a state
    pub fn transform_atoms(&mut self, state: usize, atoms: &[AtomIndex], matrix: &Mat4) {
        if let Some(cs) = self.coord_sets.get_mut(state) {
            cs.transform_atoms(atoms, matrix);
        }
    }

    /// Apply a 4x4 transformation matrix to all atoms in all states
    pub fn transform_all_states(&mut self, matrix: &Mat4) {
        for cs in &mut self.coord_sets {
            cs.transform(matrix);
        }
    }

    /// Apply a TTT (Translate-Transform-Translate) matrix to all coordinates in a state
    pub fn transform_ttt(&mut self, state: usize, ttt: &[f32; 16]) {
        if let Some(cs) = self.coord_sets.get_mut(state) {
            cs.transform_ttt(ttt);
        }
    }

    /// Apply a TTT (Translate-Transform-Translate) matrix to specific atoms in a state
    pub fn transform_ttt_atoms(&mut self, state: usize, atoms: &[AtomIndex], ttt: &[f32; 16]) {
        if let Some(cs) = self.coord_sets.get_mut(state) {
            cs.transform_ttt_atoms(atoms, ttt);
        }
    }

    // =========================================================================
    // Atom Removal
    // =========================================================================

    /// Remove atoms by indices, reindexing remaining atoms and bonds.
    ///
    /// This removes the specified atoms, any bonds involving those atoms,
    /// and updates all coordinate sets. Remaining atom/bond indices are
    /// compacted so they remain contiguous.
    pub fn remove_atoms(&mut self, indices: &[AtomIndex]) -> AtomRemap {
        if indices.is_empty() {
            return AtomRemap::identity(self.atoms.len());
        }

        let n_atoms = self.atoms.len();

        // Build a set of atoms to remove for O(1) lookup
        let mut remove_set = vec![false; n_atoms];
        for &idx in indices {
            if idx.as_usize() < n_atoms {
                remove_set[idx.as_usize()] = true;
            }
        }

        // Build old→new index mapping (-1 means removed)
        let mut old_to_new = vec![None; n_atoms];
        let mut new_idx = 0u32;
        for old_idx in 0..n_atoms {
            if !remove_set[old_idx] {
                old_to_new[old_idx] = Some(AtomIndex(new_idx));
                new_idx += 1;
            }
        }

        // Filter atoms (keep only non-removed)
        let mut new_atoms = Vec::with_capacity(new_idx as usize);
        for (i, atom) in self.atoms.drain(..).enumerate() {
            if !remove_set[i] {
                new_atoms.push(atom);
            }
        }
        self.atoms = new_atoms;

        // Filter bonds: keep only bonds where both atoms survived, and remap indices
        let mut new_bonds = Vec::new();
        for bond in &self.bonds {
            if let (Some(new_a1), Some(new_a2)) = (
                old_to_new[bond.atom1.as_usize()],
                old_to_new[bond.atom2.as_usize()],
            ) {
                let mut new_bond = bond.clone();
                new_bond.atom1 = new_a1;
                new_bond.atom2 = new_a2;
                new_bonds.push(new_bond);
            }
        }
        self.bonds = new_bonds;

        // Rebuild atom_bonds mapping
        self.atom_bonds = vec![SmallVec::new(); self.atoms.len()];
        for (bi, bond) in self.bonds.iter().enumerate() {
            let bond_idx = BondIndex(bi as u32);
            self.atom_bonds[bond.atom1.as_usize()].push(bond_idx);
            self.atom_bonds[bond.atom2.as_usize()].push(bond_idx);
        }

        // Update coordinate sets: extract coords for surviving atoms only
        for cs in &mut self.coord_sets {
            let mut new_coords = Vec::with_capacity(self.atoms.len() * 3);
            for (old_idx, &removed) in remove_set.iter().enumerate().take(n_atoms) {
                if removed {
                    continue;
                }
                if let Some(v) = cs.get_atom_coord(AtomIndex(old_idx as u32)) {
                    new_coords.push(v.x);
                    new_coords.push(v.y);
                    new_coords.push(v.z);
                } else {
                    new_coords.extend_from_slice(&[0.0, 0.0, 0.0]);
                }
            }
            let mut new_cs = CoordSet::from_coords(new_coords);
            new_cs.name = std::mem::take(&mut cs.name);
            new_cs.symmetry = cs.symmetry.take();
            new_cs.settings = cs.settings.take();
            *cs = new_cs;
        }
        self.invalidate_subchain_partition();
        AtomRemap::from_mapping(old_to_new)
    }

    /// Reorders atoms so each residue's atoms are contiguous, preserving the
    /// original residue order and slotting newly-appended atoms (e.g. rebuilt
    /// hydrogens or side chains) directly after the residue they belong to.
    ///
    /// Without this, atoms pushed onto the end form separate trailing subchains
    /// because the bond-aware partition keys off contiguous index ranges.
    pub fn regroup_by_residue(&mut self) -> AtomRemap {
        let n = self.atoms.len();
        if n == 0 {
            return AtomRemap::identity(0);
        }
        // First original index seen for each residue defines its position.
        let mut first: std::collections::HashMap<(String, i32, char), usize> =
            std::collections::HashMap::new();
        for (i, atom) in self.atoms.iter().enumerate() {
            let key = (
                atom.residue.chain.clone(),
                atom.residue.resv,
                atom.residue.inscode,
            );
            first.entry(key).or_insert(i);
        }
        let residue_rank = |atom: &Atom| -> usize {
            *first
                .get(&(
                    atom.residue.chain.clone(),
                    atom.residue.resv,
                    atom.residue.inscode,
                ))
                .unwrap_or(&0)
        };
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by_key(|&i| (residue_rank(&self.atoms[i]), i));
        self.apply_atom_permutation(order)
    }

    /// Like [`regroup_by_residue`](Self::regroup_by_residue), but additionally
    /// orders residues *within each chain* by `(resv, inscode)` rather than by
    /// first appearance. Use this when newly-appended atoms form a residue that
    /// must slot into sequence — e.g. an N-terminal ACE cap (resv before the
    /// first residue) or a C-terminal NME cap (resv after the last). Without
    /// the sequence ordering a freshly-appended cap residue lands after every
    /// other chain and the bond-aware partition splits it into its own subchain.
    ///
    /// Chains keep their original first-appearance order; atoms within a
    /// residue keep their original order.
    pub fn regroup_residues_in_sequence(&mut self) -> AtomRemap {
        let n = self.atoms.len();
        if n == 0 {
            return AtomRemap::identity(0);
        }
        // Chain ordering follows first appearance so the overall chain layout
        // is preserved; only residues within a chain are sequence-sorted.
        let mut chain_rank: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for (i, atom) in self.atoms.iter().enumerate() {
            chain_rank.entry(atom.residue.chain.clone()).or_insert(i);
        }
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by_key(|&i| {
            let atom = &self.atoms[i];
            let crank = *chain_rank.get(&atom.residue.chain).unwrap_or(&0);
            (crank, atom.residue.resv, atom.residue.inscode, i)
        });
        self.apply_atom_permutation(order)
    }

    /// Applies a precomputed atom permutation (`order[new] = old`), remapping
    /// bonds, the per-atom bond index, and every coordinate set, then
    /// invalidates the subchain partition. No-op if `order` is the identity.
    fn apply_atom_permutation(&mut self, order: Vec<usize>) -> AtomRemap {
        let n = self.atoms.len();
        if order.iter().enumerate().all(|(new, &old)| new == old) {
            return AtomRemap::identity(n);
        }

        let mut old_to_new = vec![None; n];
        for (new, &old) in order.iter().enumerate() {
            let new = u32::try_from(new).expect("atom count must fit into AtomIndex");
            old_to_new[old] = Some(AtomIndex(new));
        }

        let new_atoms: Vec<Atom> = order.iter().map(|&old| self.atoms[old].clone()).collect();
        self.atoms = new_atoms;

        for bond in &mut self.bonds {
            bond.atom1 =
                old_to_new[bond.atom1.as_usize()].expect("atom permutation must map every atom");
            bond.atom2 =
                old_to_new[bond.atom2.as_usize()].expect("atom permutation must map every atom");
        }
        self.atom_bonds = vec![SmallVec::new(); n];
        for (bi, bond) in self.bonds.iter().enumerate() {
            let bond_idx = BondIndex(bi as u32);
            self.atom_bonds[bond.atom1.as_usize()].push(bond_idx);
            self.atom_bonds[bond.atom2.as_usize()].push(bond_idx);
        }

        for cs in &mut self.coord_sets {
            let mut new_coords = Vec::with_capacity(n * 3);
            for &old in &order {
                if let Some(v) = cs.get_atom_coord(AtomIndex(old as u32)) {
                    new_coords.push(v.x);
                    new_coords.push(v.y);
                    new_coords.push(v.z);
                } else {
                    new_coords.extend_from_slice(&[0.0, 0.0, 0.0]);
                }
            }
            let mut new_cs = CoordSet::from_coords(new_coords);
            new_cs.name = std::mem::take(&mut cs.name);
            new_cs.symmetry = cs.symmetry.take();
            new_cs.settings = cs.settings.take();
            *cs = new_cs;
        }
        self.invalidate_subchain_partition();
        AtomRemap::from_mapping(old_to_new)
    }

    // =========================================================================
    // Selection Support
    // =========================================================================

    /// Get atoms matching a predicate
    pub fn select<F>(&self, predicate: F) -> Vec<AtomIndex>
    where
        F: Fn(&Atom) -> bool,
    {
        self.atoms
            .iter()
            .enumerate()
            .filter(|(_, a)| predicate(a))
            .map(|(i, _)| AtomIndex(i as u32))
            .collect()
    }

    // =========================================================================
    // Validation
    // =========================================================================

    /// Validate the molecule structure
    pub fn validate(&self) -> Vec<String> {
        let mut issues = Vec::new();

        // Check bond atom indices
        for (i, bond) in self.bonds.iter().enumerate() {
            if bond.atom1.as_usize() >= self.atoms.len() {
                issues.push(format!(
                    "Bond {} references invalid atom1: {}",
                    i, bond.atom1
                ));
            }
            if bond.atom2.as_usize() >= self.atoms.len() {
                issues.push(format!(
                    "Bond {} references invalid atom2: {}",
                    i, bond.atom2
                ));
            }
        }

        // Check atom_bonds consistency
        if self.atom_bonds.len() != self.atoms.len() {
            issues.push(format!(
                "atom_bonds length ({}) doesn't match atom count ({})",
                self.atom_bonds.len(),
                self.atoms.len()
            ));
        }

        issues
    }
}

impl std::fmt::Display for ObjectMolecule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ObjectMolecule({}: {} atoms, {} bonds, {} states)",
            self.name,
            self.atom_count(),
            self.bond_count(),
            self.state_count()
        )
    }
}

/// Builder for creating molecules with a fluent interface
#[derive(Debug, Default)]
pub struct MoleculeBuilder {
    mol: ObjectMolecule,
    pending_coords: Vec<Vec3>,
}

impl MoleculeBuilder {
    /// Create a new molecule builder
    pub fn new(name: impl Into<String>) -> Self {
        MoleculeBuilder {
            mol: ObjectMolecule::new(name),
            pending_coords: Vec::new(),
        }
    }

    /// Add an atom with coordinates
    pub fn add_atom(mut self, atom: Atom, coord: Vec3) -> Self {
        self.mol.add_atom(atom);
        self.pending_coords.push(coord);
        self
    }

    /// Add a bond
    pub fn add_bond(mut self, atom1: AtomIndex, atom2: AtomIndex, order: BondOrder) -> Self {
        let _ = self.mol.add_bond(atom1, atom2, order);
        self
    }

    /// Set the title
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.mol.title = title.into();
        self
    }

    /// Build the molecule
    pub fn build(mut self) -> ObjectMolecule {
        // Create coordinate set from pending coords
        if !self.pending_coords.is_empty() {
            let cs = CoordSet::from_vec3(&self.pending_coords);
            self.mol.add_coord_set(cs);
        }
        self.mol
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::Element;

    fn create_water() -> ObjectMolecule {
        let mut mol = ObjectMolecule::new("water");

        // Add atoms
        let o = mol.add_atom(Atom::new("O", Element::Oxygen));
        let h1 = mol.add_atom(Atom::new("H1", Element::Hydrogen));
        let h2 = mol.add_atom(Atom::new("H2", Element::Hydrogen));

        // Add bonds
        mol.add_bond(o, h1, BondOrder::Single).unwrap();
        mol.add_bond(o, h2, BondOrder::Single).unwrap();

        // Add coordinates
        let cs = CoordSet::from_vec3(&[
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.96, 0.0, 0.0),
            Vec3::new(-0.24, 0.93, 0.0),
        ]);
        mol.add_coord_set(cs);

        mol
    }

    #[test]
    fn atom_remap_tracks_removal_and_compaction() {
        let mut mol = ObjectMolecule::new("remap");
        let first = mol.add_atom(Atom::new("A", Element::Carbon));
        let second = mol.add_atom(Atom::new("B", Element::Carbon));
        let third = mol.add_atom(Atom::new("C", Element::Carbon));

        let remap = mol.remove_atoms(&[second]);

        assert_eq!(remap.remap(first), Some(AtomIndex(0)));
        assert_eq!(remap.remap(second), None);
        assert_eq!(remap.remap(third), Some(AtomIndex(1)));
        assert_eq!(
            mol.get_atom(AtomIndex(1)).map(|atom| atom.name.as_ref()),
            Some("C")
        );
    }

    #[test]
    fn atom_remap_tracks_permutations() {
        let mut mol = ObjectMolecule::new("permutation");
        let first = mol.add_atom(Atom::new("A", Element::Carbon));
        let second = mol.add_atom(Atom::new("B", Element::Carbon));
        let third = mol.add_atom(Atom::new("C", Element::Carbon));

        let remap = mol.apply_atom_permutation(vec![2, 0, 1]);

        assert_eq!(remap.remap(third), Some(AtomIndex(0)));
        assert_eq!(remap.remap(first), Some(AtomIndex(1)));
        assert_eq!(remap.remap(second), Some(AtomIndex(2)));
        let names = mol
            .atoms()
            .map(|atom| atom.name.as_ref())
            .collect::<Vec<_>>();
        assert_eq!(names, ["C", "A", "B"]);
    }

    #[test]
    fn test_molecule_creation() {
        let mol = create_water();
        assert_eq!(mol.name, "water");
        assert_eq!(mol.atom_count(), 3);
        assert_eq!(mol.bond_count(), 2);
        assert_eq!(mol.state_count(), 1);
    }

    #[test]
    fn test_atom_access() {
        let mol = create_water();
        let atom = mol.get_atom(AtomIndex(0)).unwrap();
        assert_eq!(&*atom.name, "O");
        assert_eq!(atom.element, Element::Oxygen);
    }

    #[test]
    fn test_bond_lookup() {
        let mol = create_water();

        // Find bond between O and H1
        let bond_idx = mol.find_bond(AtomIndex(0), AtomIndex(1)).unwrap();
        let bond = mol.get_bond(bond_idx).unwrap();
        assert_eq!(bond.order, BondOrder::Single);

        // No bond between H1 and H2
        assert!(mol.find_bond(AtomIndex(1), AtomIndex(2)).is_none());
    }

    #[test]
    fn test_clear_bonds() {
        let mut mol = create_water();

        mol.clear_bonds();

        assert_eq!(mol.bond_count(), 0);
        assert!(mol.atom_bond_indices(AtomIndex(0)).is_empty());
        assert!(mol.atom_bond_indices(AtomIndex(1)).is_empty());
        assert!(mol.atom_bond_indices(AtomIndex(2)).is_empty());
        assert!(mol.atoms().all(|atom| !atom.state.bonded));
    }

    #[test]
    fn test_bonded_atoms() {
        let mol = create_water();
        let bonded = mol.bonded_atoms(AtomIndex(0));
        assert_eq!(bonded.len(), 2);
        assert!(bonded.contains(&AtomIndex(1)));
        assert!(bonded.contains(&AtomIndex(2)));
    }

    #[test]
    fn test_coordinate_access() {
        let mol = create_water();
        let coord = mol.get_coord(AtomIndex(0), 0).unwrap();
        assert_eq!(coord, Vec3::new(0.0, 0.0, 0.0));
    }

    #[test]
    fn test_duplicate_bond() {
        let mut mol = ObjectMolecule::new("test");
        let a1 = mol.add_atom(Atom::new("A", Element::Carbon));
        let a2 = mol.add_atom(Atom::new("B", Element::Carbon));

        mol.add_bond(a1, a2, BondOrder::Single).unwrap();
        let result = mol.add_bond(a1, a2, BondOrder::Single);
        assert!(matches!(result, Err(MolError::DuplicateBond(_, _))));
    }

    #[test]
    fn test_builder() {
        let mol = MoleculeBuilder::new("test")
            .add_atom(Atom::new("C", Element::Carbon), Vec3::new(0.0, 0.0, 0.0))
            .add_atom(Atom::new("C", Element::Carbon), Vec3::new(1.5, 0.0, 0.0))
            .add_bond(AtomIndex(0), AtomIndex(1), BondOrder::Single)
            .title("Test molecule")
            .build();

        assert_eq!(mol.atom_count(), 2);
        assert_eq!(mol.bond_count(), 1);
        assert_eq!(mol.state_count(), 1);
        assert_eq!(mol.title, "Test molecule");
    }

    #[test]
    fn test_molecule_display() {
        let mol = create_water();
        let display = format!("{}", mol);
        assert!(display.contains("water"));
        assert!(display.contains("3 atoms"));
        assert!(display.contains("2 bonds"));
    }

    #[test]
    fn test_validation() {
        let mol = create_water();
        let issues = mol.validate();
        assert!(issues.is_empty());
    }

    #[test]
    fn test_classify_atoms_protein() {
        use crate::atom::AtomResidue;
        use crate::flags::AtomFlags;
        use std::sync::Arc;

        // Create a simple peptide with amino acid residues
        let mut mol = ObjectMolecule::new("peptide");

        // Create shared residue
        let ala_res = Arc::new(AtomResidue::from_parts("A", "ALA", 1, ' ', ""));

        // Add ALA residue atoms
        let mut n = Atom::new("N", Element::Nitrogen);
        n.residue = ala_res.clone();
        mol.add_atom(n);

        let mut ca = Atom::new("CA", Element::Carbon);
        ca.residue = ala_res.clone();
        mol.add_atom(ca);

        let mut c = Atom::new("C", Element::Carbon);
        c.residue = ala_res.clone();
        mol.add_atom(c);

        let mut o = Atom::new("O", Element::Oxygen);
        o.residue = ala_res.clone();
        mol.add_atom(o);

        // Add coordinate set
        let cs = CoordSet::from_vec3(&[
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.45, 0.0, 0.0),
            Vec3::new(2.0, 1.4, 0.0),
            Vec3::new(1.4, 2.4, 0.0),
        ]);
        mol.add_coord_set(cs);

        // Before classification, atoms should NOT have PROTEIN flag
        assert!(!mol
            .get_atom(AtomIndex(0))
            .unwrap()
            .state
            .flags
            .contains(AtomFlags::PROTEIN));

        // Classify atoms
        mol.classify_atoms();

        // After classification, all atoms should have PROTEIN and POLYMER flags
        for atom in mol.atoms() {
            assert!(
                atom.state.flags.contains(AtomFlags::PROTEIN),
                "Atom {} should have PROTEIN flag",
                atom.name
            );
            assert!(
                atom.state.flags.contains(AtomFlags::POLYMER),
                "Atom {} should have POLYMER flag",
                atom.name
            );
        }

        // CA atom should also have GUIDE flag
        let ca_atom = mol.get_atom(AtomIndex(1)).unwrap();
        assert!(
            ca_atom.state.flags.contains(AtomFlags::GUIDE),
            "CA atom should have GUIDE flag"
        );
    }

    #[test]
    fn regroup_in_sequence_slots_appended_cap_into_chain() {
        use crate::atom::AtomResidue;
        use std::sync::Arc;

        // Two chains; a cap residue (chain A, resv 0) is appended at the very
        // end — after chain B — as the cap commands do.
        let mut mol = ObjectMolecule::new("capped");
        let add = |mol: &mut ObjectMolecule, chain: &str, resn: &str, resv: i32, name: &str| {
            let res = Arc::new(AtomResidue::from_parts(chain, resn, resv, ' ', ""));
            let mut atom = Atom::new(name, Element::Carbon);
            atom.residue = res;
            mol.add_atom(atom)
        };
        add(&mut mol, "A", "ALA", 1, "CA");
        add(&mut mol, "A", "GLY", 2, "CA");
        add(&mut mol, "B", "SER", 1, "CA");
        // Appended out of sequence: ACE cap (resv 0) belongs before A/1.
        add(&mut mol, "A", "ACE", 0, "CH3");

        mol.regroup_residues_in_sequence();

        // Chain A atoms must be contiguous and resv-ordered: ACE(0), ALA(1), GLY(2).
        let chains: Vec<(String, i32)> = mol
            .atoms()
            .map(|a| (a.residue.chain.to_string(), a.residue.resv))
            .collect();
        assert_eq!(
            chains,
            vec![
                ("A".into(), 0),
                ("A".into(), 1),
                ("A".into(), 2),
                ("B".into(), 1),
            ],
            "cap residue must slot into chain A in sequence, not trail chain B"
        );
    }

    #[test]
    fn test_classify_atoms_water() {
        use crate::atom::AtomResidue;
        use crate::flags::AtomFlags;
        use std::sync::Arc;

        // Create water molecule
        let mut mol = ObjectMolecule::new("water");

        // Create shared residue for water
        let hoh_res = Arc::new(AtomResidue::from_parts("", "HOH", 1, ' ', ""));

        let mut o = Atom::new("O", Element::Oxygen);
        o.residue = hoh_res.clone();
        mol.add_atom(o);

        let mut h1 = Atom::new("H1", Element::Hydrogen);
        h1.residue = hoh_res.clone();
        mol.add_atom(h1);

        let cs = CoordSet::from_vec3(&[Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.96, 0.0, 0.0)]);
        mol.add_coord_set(cs);

        mol.classify_atoms();

        // Water should have SOLVENT flag, not PROTEIN
        for atom in mol.atoms() {
            assert!(
                atom.state.flags.contains(AtomFlags::SOLVENT),
                "Atom {} should have SOLVENT flag",
                atom.name
            );
            assert!(
                !atom.state.flags.contains(AtomFlags::PROTEIN),
                "Atom {} should NOT have PROTEIN flag",
                atom.name
            );
        }
    }

    #[test]
    fn test_residue_is_protein_after_classification() {
        use crate::atom::AtomResidue;
        use std::sync::Arc;

        // Create a peptide and verify that is_protein() returns true after classification
        let mut mol = ObjectMolecule::new("peptide");

        let gly_res = Arc::new(AtomResidue::from_parts("A", "GLY", 1, ' ', ""));

        let mut ca = Atom::new("CA", Element::Carbon);
        ca.residue = gly_res.clone();
        mol.add_atom(ca);

        let cs = CoordSet::from_vec3(&[Vec3::new(0.0, 0.0, 0.0)]);
        mol.add_coord_set(cs);

        // Before classification
        let residue = mol.residues().next().unwrap();
        assert!(
            !residue.is_protein(),
            "Residue should NOT be protein before classification"
        );

        // After classification
        mol.classify_atoms();
        let residue = mol.residues().next().unwrap();
        assert!(
            residue.is_protein(),
            "Residue should be protein after classification"
        );
    }

    #[test]
    fn test_assign_chains_protein_solvent_ions() {
        use crate::atom::AtomResidue;
        use std::sync::Arc;

        let mut mol = ObjectMolecule::new("test");

        // Protein: ALA(1) - GLY(2)
        let ala_res = Arc::new(AtomResidue::from_parts("", "ALA", 1, ' ', ""));
        for name in &["N", "CA", "C", "O"] {
            let mut atom = Atom::new(
                *name,
                if *name == "N" {
                    Element::Nitrogen
                } else if *name == "O" {
                    Element::Oxygen
                } else {
                    Element::Carbon
                },
            );
            atom.residue = ala_res.clone();
            mol.add_atom(atom);
        }
        let gly_res = Arc::new(AtomResidue::from_parts("", "GLY", 2, ' ', ""));
        for name in &["N", "CA", "C", "O"] {
            let mut atom = Atom::new(
                *name,
                if *name == "N" {
                    Element::Nitrogen
                } else if *name == "O" {
                    Element::Oxygen
                } else {
                    Element::Carbon
                },
            );
            atom.residue = gly_res.clone();
            mol.add_atom(atom);
        }

        // Ion: NA(3)
        let na_res = Arc::new(AtomResidue::from_parts("", "NA", 3, ' ', ""));
        let mut na = Atom::new("NA", Element::Sodium);
        na.residue = na_res.clone();
        mol.add_atom(na);

        // Solvent: SOL(4), SOL(5)
        for resv in [4, 5] {
            let sol_res = Arc::new(AtomResidue::from_parts("", "SOL", resv, ' ', ""));
            for name in &["OW", "HW1", "HW2"] {
                let mut atom = Atom::new(
                    *name,
                    if *name == "OW" {
                        Element::Oxygen
                    } else {
                        Element::Hydrogen
                    },
                );
                atom.residue = sol_res.clone();
                mol.add_atom(atom);
            }
        }

        // Add coordinates (needed for bond generation)
        // ALA: N(0,0,0) CA(1.47,0,0) C(2.5,1.2,0) O(2.5,2.4,0)
        // GLY: N(3.8,0.5,0) CA(5.2,0.5,0) C(6.3,1.7,0) O(6.3,2.9,0)
        // Place C of ALA and N of GLY close enough for a bond (~1.33 Å)
        let coords = vec![
            // ALA
            Vec3::new(0.0, 0.0, 0.0),  // N
            Vec3::new(1.47, 0.0, 0.0), // CA
            Vec3::new(2.5, 1.2, 0.0),  // C
            Vec3::new(2.5, 2.4, 0.0),  // O
            // GLY — N close to ALA's C for backbone bond
            Vec3::new(3.8, 1.2, 0.0), // N (1.3 Å from C)
            Vec3::new(5.2, 1.2, 0.0), // CA
            Vec3::new(6.3, 2.4, 0.0), // C
            Vec3::new(6.3, 3.6, 0.0), // O
            // NA ion far away
            Vec3::new(20.0, 20.0, 20.0), // NA
            // SOL 1
            Vec3::new(30.0, 30.0, 30.0), // OW
            Vec3::new(30.5, 30.5, 30.0), // HW1
            Vec3::new(30.5, 29.5, 30.0), // HW2
            // SOL 2
            Vec3::new(35.0, 35.0, 35.0), // OW
            Vec3::new(35.5, 35.5, 35.0), // HW1
            Vec3::new(35.5, 34.5, 35.0), // HW2
        ];
        let cs = CoordSet::from_vec3(&coords);
        mol.add_coord_set(cs);

        mol.classify_atoms();
        mol.generate_bonds(crate::bonding::DEFAULT_BOND_TOLERANCE);
        mol.assign_chains();

        // Protein should be chain A
        assert_eq!(mol.atoms_slice()[0].residue.chain, "A"); // ALA
        assert_eq!(mol.atoms_slice()[4].residue.chain, "A"); // GLY

        // Ion should share a chain
        let ion_chain = &mol.atoms_slice()[8].residue.chain;
        assert!(!ion_chain.is_empty());

        // Solvent should share a chain
        let sol_chain = &mol.atoms_slice()[9].residue.chain;
        assert!(!sol_chain.is_empty());

        // Ion and solvent chains should be different from protein and each other
        assert_ne!(ion_chain, "A");
        assert_ne!(sol_chain, "A");
        assert_ne!(ion_chain, sol_chain);
    }

    #[test]
    fn test_assign_chains_noop_when_chains_exist() {
        use crate::atom::AtomResidue;
        use std::sync::Arc;

        let mut mol = ObjectMolecule::new("test");

        let res = Arc::new(AtomResidue::from_parts("X", "ALA", 1, ' ', ""));
        let mut atom = Atom::new("CA", Element::Carbon);
        atom.residue = res;
        mol.add_atom(atom);

        let cs = CoordSet::from_vec3(&[Vec3::new(0.0, 0.0, 0.0)]);
        mol.add_coord_set(cs);

        mol.assign_chains();

        // Chain should remain "X" (not overwritten)
        assert_eq!(mol.atoms_slice()[0].residue.chain, "X");
    }

    #[test]
    fn test_assign_chains_resnum_wrap() {
        use crate::atom::AtomResidue;
        use std::sync::Arc;

        let mut mol = ObjectMolecule::new("test");
        let mut coords = Vec::new();

        // First protein block: ALA(99998), GLY(99999) — bonded backbone
        let ala1 = Arc::new(AtomResidue::from_parts("", "ALA", 99998, ' ', ""));
        let ala1_names = [
            ("N", Element::Nitrogen),
            ("CA", Element::Carbon),
            ("C", Element::Carbon),
            ("O", Element::Oxygen),
        ];
        let ala1_coords = [
            Vec3::new(0.0, 0.0, 0.0),   // N
            Vec3::new(1.47, 0.0, 0.0),  // CA
            Vec3::new(2.52, 1.25, 0.0), // C
            Vec3::new(2.52, 2.40, 0.0), // O
        ];
        for ((name, elem), coord) in ala1_names.iter().zip(&ala1_coords) {
            let mut atom = Atom::new(*name, *elem);
            atom.residue = ala1.clone();
            mol.add_atom(atom);
            coords.push(*coord);
        }

        let gly1 = Arc::new(AtomResidue::from_parts("", "GLY", 99999, ' ', ""));
        let gly1_names = [
            ("N", Element::Nitrogen),
            ("CA", Element::Carbon),
            ("C", Element::Carbon),
            ("O", Element::Oxygen),
        ];
        let gly1_coords = [
            Vec3::new(3.80, 1.25, 0.0), // N — 1.28 Å from ALA's C (bonded)
            Vec3::new(5.24, 1.25, 0.0), // CA
            Vec3::new(6.30, 2.45, 0.0), // C
            Vec3::new(6.30, 3.60, 0.0), // O
        ];
        for ((name, elem), coord) in gly1_names.iter().zip(&gly1_coords) {
            let mut atom = Atom::new(*name, *elem);
            atom.residue = gly1.clone();
            mol.add_atom(atom);
            coords.push(*coord);
        }

        // Second protein block (resnum wraps): ALA(1) — far from first block
        let ala2 = Arc::new(AtomResidue::from_parts("", "ALA", 1, ' ', ""));
        let ala2_names = [
            ("N", Element::Nitrogen),
            ("CA", Element::Carbon),
            ("C", Element::Carbon),
            ("O", Element::Oxygen),
        ];
        let ala2_coords = [
            Vec3::new(20.0, 0.0, 0.0),
            Vec3::new(21.47, 0.0, 0.0),
            Vec3::new(22.52, 1.25, 0.0),
            Vec3::new(22.52, 2.40, 0.0),
        ];
        for ((name, elem), coord) in ala2_names.iter().zip(&ala2_coords) {
            let mut atom = Atom::new(*name, *elem);
            atom.residue = ala2.clone();
            mol.add_atom(atom);
            coords.push(*coord);
        }

        let cs = CoordSet::from_vec3(&coords);
        mol.add_coord_set(cs);

        mol.classify_atoms();
        mol.generate_bonds(crate::bonding::DEFAULT_BOND_TOLERANCE);
        mol.assign_chains();

        // First block = chain A, second block = chain B (due to resnum decrease)
        assert_eq!(mol.atoms_slice()[0].residue.chain, "A");
        assert_eq!(mol.atoms_slice()[4].residue.chain, "A");
        assert_eq!(mol.atoms_slice()[8].residue.chain, "B");
    }

    #[test]
    fn test_serde_roundtrip() {
        let mol = create_water();

        // Serialize (MessagePack)
        let bytes = rmp_serde::to_vec(&mol).expect("serialize");

        // Deserialize
        let mut deserialized: ObjectMolecule = rmp_serde::from_slice(&bytes).expect("deserialize");

        // atom_bonds is #[serde(skip)], so bond lookups are broken before rebuild
        assert!(deserialized.bonded_atoms(AtomIndex(0)).is_empty());

        // Rebuild and verify
        deserialized.rebuild_atom_bonds();
        assert_eq!(deserialized.atom_count(), mol.atom_count());
        assert_eq!(deserialized.bond_count(), mol.bond_count());
        assert_eq!(deserialized.state_count(), mol.state_count());

        // Bond lookup should work after rebuild
        let bonded = deserialized.bonded_atoms(AtomIndex(0));
        assert_eq!(bonded.len(), 2);
    }
}
