//! Residue and chain utilities
//!
//! Provides types and functions for working with residues and chains. Residue
//! and chain information is stored inline in atoms, so this module provides
//! view types for convenient access.

use phf::{phf_map, phf_set};
use serde::{Deserialize, Serialize};

use crate::atom::Atom;
use crate::index::AtomIndex;
use std::ops::Range;

// Re-export iterators from the dedicated iterator module
pub use crate::iterator::{ChainIterator, ResidueIterator, SubchainIterator};
use crate::subchain::PolymerSubchainIterator;

/// Key for uniquely identifying a residue within a molecule
///
/// A residue is uniquely identified by its chain, name, number, and insertion code.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ResidueKey {
    /// Chain identifier
    pub chain: String,
    /// Residue name (e.g., "ALA", "GLY")
    pub resn: String,
    /// Residue sequence number
    pub resv: i32,
    /// Insertion code
    pub inscode: char,
}

impl ResidueKey {
    /// Create a new residue key
    pub fn new(
        chain: impl Into<String>,
        resn: impl Into<String>,
        resv: i32,
        inscode: char,
    ) -> Self {
        ResidueKey {
            chain: chain.into(),
            resn: resn.into(),
            resv,
            inscode,
        }
    }

    /// Create a residue key from an atom
    pub fn from_atom(atom: &Atom) -> Self {
        atom.residue.key.clone()
    }

    /// Get a display string for the residue (e.g., "A/ALA`1")
    pub fn display(&self) -> String {
        if self.inscode != ' ' && self.inscode != '\0' {
            format!("{}/{}'{}{}", self.chain, self.resn, self.resv, self.inscode)
        } else {
            format!("{}/{}'{}", self.chain, self.resn, self.resv)
        }
    }
}

impl std::fmt::Display for ResidueKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display())
    }
}

/// A view into the atoms of a single residue
///
/// This is a borrowed view that does not own the atom data.
/// It provides convenient access to residue-level operations.
#[derive(Debug)]
pub struct ResidueView<'a> {
    /// The residue key
    pub key: ResidueKey,
    /// Slice of atoms in this residue
    pub atoms: &'a [Atom],
    /// Range of atom indices in the parent molecule
    pub atom_range: Range<usize>,
}

impl<'a> ResidueView<'a> {
    /// Create a new residue view
    pub fn new(key: ResidueKey, atoms: &'a [Atom], atom_range: Range<usize>) -> Self {
        ResidueView {
            key,
            atoms,
            atom_range,
        }
    }

    /// Get the chain ID
    #[inline]
    pub fn chain(&self) -> &str {
        &self.key.chain
    }

    /// Get the residue name
    #[inline]
    pub fn resn(&self) -> &str {
        &self.key.resn
    }

    /// Get the residue number
    #[inline]
    pub fn resv(&self) -> i32 {
        self.key.resv
    }

    /// Get the number of atoms in this residue
    #[inline]
    pub fn len(&self) -> usize {
        self.atoms.len()
    }

    /// Check if the residue is empty
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.atoms.is_empty()
    }

    /// Get an atom by index within the residue
    pub fn get(&self, index: usize) -> Option<&'a Atom> {
        self.atoms.get(index)
    }

    /// Iterate over atoms in the residue
    pub fn iter(&self) -> impl Iterator<Item = &'a Atom> {
        self.atoms.iter()
    }

    /// Iterate over atom indices and atoms
    pub fn iter_indexed(&self) -> impl Iterator<Item = (AtomIndex, &'a Atom)> {
        self.atom_range
            .clone()
            .zip(self.atoms.iter())
            .map(|(idx, atom)| (AtomIndex(idx as u32), atom))
    }

    /// Find an atom by name
    pub fn find_by_name(&self, name: &str) -> Option<(AtomIndex, &'a Atom)> {
        for (idx, atom) in self.iter_indexed() {
            if &*atom.name == name {
                return Some((idx, atom));
            }
        }
        None
    }

    /// Get the C-alpha atom (for proteins)
    pub fn ca(&self) -> Option<(AtomIndex, &'a Atom)> {
        self.find_by_name("CA")
    }

    /// Check if this is a protein residue
    pub fn is_protein(&self) -> bool {
        self.atoms
            .first()
            .map(|a| a.state.flags.contains(crate::flags::AtomFlags::PROTEIN))
            .unwrap_or(false)
    }

    /// Check if this is a nucleic acid residue
    pub fn is_nucleic(&self) -> bool {
        self.atoms
            .first()
            .map(|a| a.state.flags.contains(crate::flags::AtomFlags::NUCLEIC))
            .unwrap_or(false)
    }

    /// Check if this is a biopolymer residue (protein OR nucleic acid).
    /// Distinguishes peptide/nucleic backbones from ligands, ions, solvent,
    /// or other HETATM residues for representations that walk a single
    /// chain (cartoon, ribbon, surface).
    pub fn is_polymer(&self) -> bool {
        self.atoms
            .first()
            .map(|a| a.state.flags.contains(crate::flags::AtomFlags::POLYMER))
            .unwrap_or(false)
    }
}

/// A view into all atoms sharing one chain identifier.
///
/// A `ChainView` covers every atom whose `residue.chain` equals the chain's
/// id, regardless of whether the atom is a biopolymer residue, a HET ligand,
/// or solvent. To descend into the polymer / HET / solvent split use
/// [`subchains`](Self::subchains) (or [`polymer_subchains`](Self::polymer_subchains)
/// for representations that only need biopolymer runs).
#[derive(Debug)]
pub struct ChainView<'a> {
    /// Chain identifier
    pub chain_id: String,
    /// Slice of atoms in this chain
    pub atoms: &'a [Atom],
    /// Range of atom indices in the parent molecule
    pub atom_range: Range<usize>,
}

impl<'a> ChainView<'a> {
    /// Create a new chain view
    pub fn new(chain_id: impl Into<String>, atoms: &'a [Atom], atom_range: Range<usize>) -> Self {
        ChainView {
            chain_id: chain_id.into(),
            atoms,
            atom_range,
        }
    }

    /// Get the chain ID
    #[inline]
    pub fn id(&self) -> &str {
        &self.chain_id
    }

    /// Get the number of atoms in this chain
    #[inline]
    pub fn len(&self) -> usize {
        self.atoms.len()
    }

    /// Check if the chain is empty
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.atoms.is_empty()
    }

    /// Iterate over atoms in the chain
    pub fn iter(&self) -> impl Iterator<Item = &'a Atom> {
        self.atoms.iter()
    }

    /// Iterate over atom indices and atoms
    pub fn iter_indexed(&self) -> impl Iterator<Item = (AtomIndex, &'a Atom)> {
        self.atom_range
            .clone()
            .zip(self.atoms.iter())
            .map(|(idx, atom)| (AtomIndex(idx as u32), atom))
    }

    /// Iterate over residues in this chain (across all subchains).
    pub fn residues(&self) -> ResidueIterator<'a> {
        ResidueIterator::new(self.atoms, self.atom_range.start)
    }

    /// Iterate over subchains in this chain — homogeneous polymer / HET / solvent runs.
    ///
    /// Use this when the consumer cares about the per-subchain kind, e.g. the
    /// Objects panel where each row is a subchain, or the popover-command builder
    /// that scopes the `polymer` qualifier to biopolymer subchains.
    pub fn subchains(&self) -> SubchainIterator<'a> {
        SubchainIterator::new(self.atoms, self.atom_range.start)
    }

    /// Iterate over only the biopolymer subchains of this chain.
    ///
    /// Used by representations like cartoon and ribbon that operate exclusively
    /// on protein / nucleic-acid backbones.
    pub fn polymer_subchains(&self) -> PolymerSubchainIterator<'a> {
        PolymerSubchainIterator::new(self.subchains())
    }
}

/// Check if two atoms are in the same residue
#[inline]
pub fn atoms_same_residue(a: &Atom, b: &Atom) -> bool {
    // Can use Arc pointer equality for efficiency if atoms share the same residue
    std::sync::Arc::ptr_eq(&a.residue, &b.residue)
        || (a.residue.chain == b.residue.chain
            && a.residue.resv == b.residue.resv
            && a.residue.inscode == b.residue.inscode
            && a.residue.resn == b.residue.resn)
}

/// Check if two atoms share the same `segi` segment identifier.
///
/// Note: this is the `segi` segment identifier, unrelated to
/// [`crate::SubchainView`] which represents a polymer / HET / solvent run.
#[inline]
pub fn atoms_same_segment(a: &Atom, b: &Atom) -> bool {
    a.residue.segi == b.residue.segi
}

/// Convert 3-letter amino acid code to 1-letter code
pub fn three_to_one(resn: &str) -> Option<char> {
    match resn {
        "ALA" => Some('A'),
        "ARG" => Some('R'),
        "ASN" => Some('N'),
        "ASP" => Some('D'),
        "CYS" => Some('C'),
        "GLN" => Some('Q'),
        "GLU" => Some('E'),
        "GLY" => Some('G'),
        "HIS" => Some('H'),
        "ILE" => Some('I'),
        "LEU" => Some('L'),
        "LYS" => Some('K'),
        "MET" => Some('M'),
        "PHE" => Some('F'),
        "PRO" => Some('P'),
        "SER" => Some('S'),
        "THR" => Some('T'),
        "TRP" => Some('W'),
        "TYR" => Some('Y'),
        "VAL" => Some('V'),
        // Histidine protonation variants
        "HID" | "HIE" | "HIP" => Some('H'),
        // Cysteine variants
        "CYX" => Some('C'),
        // Non-standard but common
        "MSE" => Some('M'),
        "SEC" => Some('U'),
        "PYL" => Some('O'),
        // Phosphorylated amino acids
        "SEP" => Some('S'),
        "TPO" => Some('T'),
        "PTR" => Some('Y'),
        // Methylated / acetylated / other common modifications
        "MLY" | "M3L" | "MLZ" | "KCX" => Some('K'),
        "OCS" | "CSO" | "CSS" | "CSD" | "CME" | "SCH" | "SMC" | "CSX" => Some('C'),
        "NEP" => Some('H'),
        "HSD" | "HSE" | "HSP" => Some('H'),
        // Charged variants (force-field specific)
        "ARGP" => Some('R'),
        "ASPM" => Some('D'),
        "GLUM" => Some('E'),
        "LYSP" => Some('K'),
        "ASH" => Some('D'),
        "GLH" => Some('E'),
        "LYN" => Some('K'),
        "ARN" => Some('R'),
        "TYS" => Some('Y'),
        "FME" => Some('M'),
        "SAM" => Some('M'),
        "HYP" => Some('P'),
        "UNK" => Some('X'),
        _ => None,
    }
}

/// Convert nucleotide residue name to single character
pub fn nucleotide_to_char(resn: &str) -> Option<char> {
    match resn {
        // Standard bases
        "DA" | "A" | "ADE" => Some('A'),
        "DT" | "T" | "THY" => Some('T'),
        "DG" | "G" | "GUA" => Some('G'),
        "DC" | "C" | "CYT" => Some('C'),
        "U" | "URA" => Some('U'),
        "DI" | "I" => Some('I'),
        // Generic
        "N" => Some('n'),
        "DN" => Some('n'),
        // Modified RNA — map to lowercase parent base or 'n'
        "PSU" | "H2U" | "5MU" | "4SU" | "5BU" | "BRU" | "CBR" => Some('u'),
        "5MC" | "OMC" => Some('c'),
        "OMG" | "M2G" | "7MG" | "2MG" | "YYG" => Some('g'),
        "1MA" => Some('a'),
        // Modified DNA
        "5CM" => Some('c'),
        "8OG" => Some('g'),
        // More modified RNA/DNA
        "2MA" | "6MA" | "A23" | "DZ" => Some('a'),
        "IU" | "5IU" | "UFT" => Some('u'),
        "5IC" | "3MC" | "CFZ" => Some('c'),
        "G7M" | "O2G" | "RSQ" => Some('g'),
        "LCA" => Some('a'),
        "LCG" => Some('g'),
        "LCC" => Some('c'),
        "LCT" => Some('t'),
        _ => None,
    }
}

/// Get display character for any residue (amino acid, nucleotide, or '?' for unknown)
pub fn residue_to_char(resn: &str) -> char {
    three_to_one(resn)
        .or_else(|| nucleotide_to_char(resn))
        .unwrap_or('?')
}

// ============================================================================
// Residue Classification Sets (compile-time perfect hash)
// ============================================================================
//
// AMINO_ACIDS and NUCLEOTIDES are phf::Map<&str, bool> where:
//   true  = standard (canonical 20 AAs / unmodified nucleotides)
//   false = variant  (protonation states, modifications, etc.)
//
// Invariants:
//   is_standard_amino_acid(x)  ⟹  is_amino_acid(x)
//   is_standard_nucleotide(x)  ⟹  is_nucleotide(x)
//   is_amino_acid(x)           ⟹  three_to_one(x).is_some()
//   is_nucleotide(x)           ⟹  nucleotide_to_char(x).is_some()

/// Amino acids: canonical (true) + variants (false).
static AMINO_ACIDS: phf::Map<&str, bool> = phf_map! {
    // 20 canonical (standard = true)
    "ALA" => true, "ARG" => true, "ASN" => true, "ASP" => true, "CYS" => true,
    "GLN" => true, "GLU" => true, "GLY" => true, "HIS" => true, "ILE" => true,
    "LEU" => true, "LYS" => true, "MET" => true, "PHE" => true, "PRO" => true,
    "SER" => true, "THR" => true, "TRP" => true, "TYR" => true, "VAL" => true,
    // Histidine protonation variants (false)
    "HID" => false, "HIE" => false, "HIP" => false,
    "HSP" => false, "HSD" => false, "HSE" => false,
    // Cysteine variants (false)
    "CYX" => false,
    // Protonation variants (false)
    "ASH" => false, "GLH" => false,
    // Non-standard but common (false)
    "MSE" => false, "SEC" => false, "PYL" => false,
    // Charged variants (false)
    "ARGP" => false, "ASPM" => false, "GLUM" => false, "LYSP" => false,
};

/// Nucleotides: standard (true) + modified (false).
/// Single source of truth — replaces separate STANDARD_NUCLEOTIDES and NUCLEOTIDES sets.
static NUCLEOTIDES: phf::Map<&str, bool> = phf_map! {
    // Standard DNA (true)
    "DA" => true, "DC" => true, "DG" => true, "DT" => true, "DI" => true,
    // Standard RNA (true)
    "A" => true, "C" => true, "G" => true, "U" => true, "I" => true,
    // Generic (true)
    "N" => true, "DN" => true,
    // Alternative 3-letter names (true)
    "ADE" => true, "CYT" => true, "GUA" => true, "THY" => true, "URA" => true,
    // Common modified RNA (false)
    "PSU" => false, "5MC" => false, "OMC" => false, "OMG" => false,
    "M2G" => false, "5MU" => false, "7MG" => false, "2MG" => false,
    "H2U" => false, "YYG" => false, "1MA" => false, "4SU" => false,
    "5BU" => false, "BRU" => false, "CBR" => false,
    // Modified DNA (false)
    "5CM" => false, "8OG" => false,
};

/// Common water residue names.
static WATER_NAMES: phf::Set<&str> = phf_set! {
    "HOH", "WAT", "H2O", "DOD", "TIP", "TIP3", "SPC", "SOL",
};

/// Common ion residue names (GROMACS / CHARMM / AMBER / OPLS conventions).
static ION_NAMES: phf::Set<&str> = phf_set! {
    // Monatomic cations
    "NA", "NA+", "K", "K+", "CA2", "MG2",
    "ZN", "ZN2", "FE", "FE2", "CU", "CU2",
    "MN", "MN2", "NI", "CO", "CD",
    "LI", "LI+", "RB", "CS", "BA", "SR",
    // Monatomic anions
    "CL", "CL-", "BR", "I-",
    // CHARMM / OPLS force field variants
    "SOD", "POT", "CLA", "CAL",
};

/// Common lipid residue names (CHARMM36 / AMBER / Slipids / Martini conventions).
static LIPID_NAMES: phf::Set<&str> = phf_set! {
    // Phosphatidylcholines
    "POPC", "DPPC", "DMPC", "DSPC", "DOPC", "DLPC", "DAPC", "DEPC",
    // Phosphatidylethanolamines
    "POPE", "DPPE", "DMPE", "DSPE", "DOPE", "DLPE",
    // Phosphatidylglycerols / serines / inositols
    "POPG", "DPPG", "DMPG", "DOPG", "POPS", "DPPS", "DOPS", "POPI",
    // Sphingomyelins
    "PSM", "SSM", "NSM",
    // Cholesterol
    "CHOL", "CHL1", "CLR",
    // Ceramides / Cardiolipin
    "CER", "CERA", "CDL1", "CDL2",
};

/// N/C-terminal capping groups.
static CAPPING_GROUPS: phf::Set<&str> = phf_set! {
    "ACE", "NME", "NHH", "FOR", "NH2",
};

// ============================================================================
// Classification Functions
// ============================================================================

/// Returns `true` only for the 20 canonical amino acids.
pub fn is_standard_amino_acid(resn: &str) -> bool {
    AMINO_ACIDS.get(resn) == Some(&true)
}

/// Check if a residue name is an amino acid (canonical + variants).
pub fn is_amino_acid(resn: &str) -> bool {
    AMINO_ACIDS.contains_key(resn)
}

/// Returns `true` only for standard unmodified nucleotides.
pub fn is_standard_nucleotide(resn: &str) -> bool {
    NUCLEOTIDES.get(resn) == Some(&true)
}

/// Check if a residue name is a nucleotide (standard + modified).
pub fn is_nucleotide(resn: &str) -> bool {
    NUCLEOTIDES.contains_key(resn)
}

/// Check if a residue name is water.
pub fn is_water(resn: &str) -> bool {
    WATER_NAMES.contains(resn)
}

/// Check if a residue name is a known ion.
pub fn is_ion(resn: &str) -> bool {
    ION_NAMES.contains(resn)
}

/// Check if a residue name is a known lipid.
pub fn is_lipid(resn: &str) -> bool {
    LIPID_NAMES.contains(resn)
}

/// Check if a residue name is a capping group.
pub fn is_capping_group(resn: &str) -> bool {
    CAPPING_GROUPS.contains(resn)
}

/// Residue category for chain assignment
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResidueCategory {
    Protein,
    Nucleic,
    Solvent,
    Ion,
    Lipid,
    Other,
}

/// Classify a residue by its name into a category
pub fn classify_residue(resn: &str) -> ResidueCategory {
    if is_amino_acid(resn) {
        ResidueCategory::Protein
    } else if is_nucleotide(resn) {
        ResidueCategory::Nucleic
    } else if is_water(resn) {
        ResidueCategory::Solvent
    } else if is_ion(resn) {
        ResidueCategory::Ion
    } else if is_lipid(resn) {
        ResidueCategory::Lipid
    } else {
        ResidueCategory::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atom::AtomResidue;
    use crate::element::Element;
    use std::sync::Arc;

    fn make_test_atoms() -> Vec<Atom> {
        let mut atoms = Vec::new();

        // Chain A, residue 1 (ALA)
        let res_ala = Arc::new(AtomResidue::from_parts("A", "ALA", 1, ' ', ""));
        for name in &["N", "CA", "C", "O", "CB"] {
            let mut atom = Atom::new(*name, Element::Carbon);
            atom.residue = res_ala.clone();
            atoms.push(atom);
        }

        // Chain A, residue 2 (GLY)
        let res_gly = Arc::new(AtomResidue::from_parts("A", "GLY", 2, ' ', ""));
        for name in &["N", "CA", "C", "O"] {
            let mut atom = Atom::new(*name, Element::Carbon);
            atom.residue = res_gly.clone();
            atoms.push(atom);
        }

        // Chain B, residue 1 (SER)
        let res_ser = Arc::new(AtomResidue::from_parts("B", "SER", 1, ' ', ""));
        for name in &["N", "CA", "C", "O", "CB", "OG"] {
            let mut atom = Atom::new(*name, Element::Carbon);
            atom.residue = res_ser.clone();
            atoms.push(atom);
        }

        atoms
    }

    #[test]
    fn test_residue_key() {
        let key = ResidueKey::new("A", "ALA", 1, ' ');
        assert_eq!(key.chain, "A");
        assert_eq!(key.resn, "ALA");
        assert_eq!(key.resv, 1);
    }

    #[test]
    fn test_chain_iterator_does_not_split_hetatm() {
        // Companion to `test_subchain_iterator_splits_hetatm` in iterator.rs:
        // verifies that ChainView itself spans the full chain regardless of
        // polymer / HET / solvent mix. The split lives in SubchainIterator.
        let mut atoms = Vec::new();

        // Chain A, biopolymer (hetatm = false)
        let res_ala = Arc::new(AtomResidue::from_parts("A", "ALA", 1, ' ', ""));
        for name in &["N", "CA", "C", "O", "CB"] {
            let mut atom = Atom::new(*name, Element::Carbon);
            atom.residue = res_ala.clone();
            atoms.push(atom);
        }

        // Chain A, PO4 (hetatm = true)
        let res_po4 = Arc::new(AtomResidue::from_parts("A", "PO4", 300, ' ', ""));
        let mut po4 = Atom::new("P", Element::Phosphorus);
        po4.residue = res_po4;
        po4.state.hetatm = true;
        atoms.push(po4);

        // Chain A, HEM (hetatm = true, different resn)
        let res_hem = Arc::new(AtomResidue::from_parts("A", "HEM", 301, ' ', ""));
        for name in &["FE", "NA", "NB", "NC"] {
            let mut atom = Atom::new(*name, Element::Iron);
            atom.residue = res_hem.clone();
            atom.state.hetatm = true;
            atoms.push(atom);
        }

        let chains: Vec<_> = ChainIterator::new(&atoms, 0).collect();

        // One chain "A" with all 10 atoms.
        assert_eq!(chains.len(), 1);
        assert_eq!(chains[0].id(), "A");
        assert_eq!(chains[0].len(), 10);

        // The split is visible at the subchain level.
        let subchains: Vec<_> = chains[0].subchains().collect();
        assert_eq!(subchains.len(), 3);
    }

    #[test]
    fn test_residue_view_find() {
        let atoms = make_test_atoms();
        let residue = ResidueIterator::new(&atoms, 0).next().unwrap();

        let (idx, ca) = residue.ca().unwrap();
        assert_eq!(&*ca.name, "CA");
        assert_eq!(idx.as_usize(), 1);
    }

    #[test]
    fn test_atoms_same_residue() {
        let atoms = make_test_atoms();
        assert!(atoms_same_residue(&atoms[0], &atoms[1]));
        assert!(!atoms_same_residue(&atoms[0], &atoms[5]));
    }

    #[test]
    fn test_atoms_same_residue_shared() {
        let atoms = make_test_atoms();
        // Atoms 0 and 1 share the same Arc<AtomResidue>
        assert!(Arc::ptr_eq(&atoms[0].residue, &atoms[1].residue));
        assert!(atoms_same_residue(&atoms[0], &atoms[1]));
    }

    #[test]
    fn test_is_amino_acid() {
        assert!(is_amino_acid("ALA"));
        assert!(is_amino_acid("GLY"));
        assert!(!is_amino_acid("HOH"));
    }

    #[test]
    fn test_is_water() {
        assert!(is_water("HOH"));
        assert!(is_water("WAT"));
        assert!(!is_water("ALA"));
    }

    #[test]
    fn test_three_to_one_standard() {
        assert_eq!(three_to_one("ALA"), Some('A'));
        assert_eq!(three_to_one("ARG"), Some('R'));
        assert_eq!(three_to_one("ASN"), Some('N'));
        assert_eq!(three_to_one("ASP"), Some('D'));
        assert_eq!(three_to_one("CYS"), Some('C'));
        assert_eq!(three_to_one("GLN"), Some('Q'));
        assert_eq!(three_to_one("GLU"), Some('E'));
        assert_eq!(three_to_one("GLY"), Some('G'));
        assert_eq!(three_to_one("HIS"), Some('H'));
        assert_eq!(three_to_one("ILE"), Some('I'));
        assert_eq!(three_to_one("LEU"), Some('L'));
        assert_eq!(three_to_one("LYS"), Some('K'));
        assert_eq!(three_to_one("MET"), Some('M'));
        assert_eq!(three_to_one("PHE"), Some('F'));
        assert_eq!(three_to_one("PRO"), Some('P'));
        assert_eq!(three_to_one("SER"), Some('S'));
        assert_eq!(three_to_one("THR"), Some('T'));
        assert_eq!(three_to_one("TRP"), Some('W'));
        assert_eq!(three_to_one("TYR"), Some('Y'));
        assert_eq!(three_to_one("VAL"), Some('V'));
    }

    #[test]
    fn test_three_to_one_variants() {
        // Histidine protonation variants
        assert_eq!(three_to_one("HID"), Some('H'));
        assert_eq!(three_to_one("HIE"), Some('H'));
        assert_eq!(three_to_one("HIP"), Some('H'));
        // Cysteine variant
        assert_eq!(three_to_one("CYX"), Some('C'));
        // Non-standard
        assert_eq!(three_to_one("MSE"), Some('M'));
        assert_eq!(three_to_one("SEC"), Some('U'));
        assert_eq!(three_to_one("PYL"), Some('O'));
    }

    #[test]
    fn test_three_to_one_unknown() {
        assert_eq!(three_to_one("HOH"), None);
        assert_eq!(three_to_one("ZZZ"), None);
        assert_eq!(three_to_one(""), None);
    }

    #[test]
    fn test_nucleotide_to_char() {
        // DNA
        assert_eq!(nucleotide_to_char("DA"), Some('A'));
        assert_eq!(nucleotide_to_char("DT"), Some('T'));
        assert_eq!(nucleotide_to_char("DG"), Some('G'));
        assert_eq!(nucleotide_to_char("DC"), Some('C'));
        assert_eq!(nucleotide_to_char("DI"), Some('I'));
        // RNA
        assert_eq!(nucleotide_to_char("A"), Some('A'));
        assert_eq!(nucleotide_to_char("U"), Some('U'));
        assert_eq!(nucleotide_to_char("G"), Some('G'));
        assert_eq!(nucleotide_to_char("C"), Some('C'));
        assert_eq!(nucleotide_to_char("I"), Some('I'));
        // Alternative names
        assert_eq!(nucleotide_to_char("ADE"), Some('A'));
        assert_eq!(nucleotide_to_char("THY"), Some('T'));
        assert_eq!(nucleotide_to_char("GUA"), Some('G'));
        assert_eq!(nucleotide_to_char("CYT"), Some('C'));
        assert_eq!(nucleotide_to_char("URA"), Some('U'));
        // Generic
        assert_eq!(nucleotide_to_char("N"), Some('n'));
        assert_eq!(nucleotide_to_char("DN"), Some('n'));
        // Modified bases
        assert_eq!(nucleotide_to_char("PSU"), Some('u'));
        assert_eq!(nucleotide_to_char("5MC"), Some('c'));
        assert_eq!(nucleotide_to_char("OMG"), Some('g'));
        assert_eq!(nucleotide_to_char("1MA"), Some('a'));
        assert_eq!(nucleotide_to_char("8OG"), Some('g'));
        // Unknown
        assert_eq!(nucleotide_to_char("HOH"), None);
    }

    #[test]
    fn test_residue_to_char() {
        assert_eq!(residue_to_char("ALA"), 'A');
        assert_eq!(residue_to_char("DA"), 'A');
        assert_eq!(residue_to_char("HOH"), '?');
        assert_eq!(residue_to_char("UNK"), 'X');
    }

    #[test]
    fn test_is_ion() {
        assert!(is_ion("NA"));
        assert!(is_ion("CL"));
        assert!(is_ion("MG2"));
        assert!(is_ion("SOD"));
        assert!(is_ion("CLA"));
        assert!(!is_ion("ALA"));
        assert!(!is_ion("SOL"));
    }

    #[test]
    fn test_is_lipid() {
        assert!(is_lipid("POPC"));
        assert!(is_lipid("CHOL"));
        assert!(is_lipid("DPPC"));
        assert!(is_lipid("CHL1"));
        assert!(!is_lipid("ALA"));
        assert!(!is_lipid("SOL"));
        assert!(!is_lipid("NA"));
    }

    #[test]
    fn test_three_to_one_noncanonical() {
        assert_eq!(three_to_one("SEP"), Some('S'));
        assert_eq!(three_to_one("TPO"), Some('T'));
        assert_eq!(three_to_one("PTR"), Some('Y'));
        assert_eq!(three_to_one("MLY"), Some('K'));
        assert_eq!(three_to_one("HYP"), Some('P'));
        assert_eq!(three_to_one("UNK"), Some('X'));
    }

    #[test]
    fn test_is_capping_group() {
        assert!(is_capping_group("ACE"));
        assert!(is_capping_group("NME"));
        assert!(!is_capping_group("ALA"));
        assert!(!is_capping_group("HOH"));
    }

    #[test]
    fn test_classify_residue() {
        assert_eq!(classify_residue("ALA"), ResidueCategory::Protein);
        assert_eq!(classify_residue("DA"), ResidueCategory::Nucleic);
        assert_eq!(classify_residue("SOL"), ResidueCategory::Solvent);
        assert_eq!(classify_residue("NA"), ResidueCategory::Ion);
        assert_eq!(classify_residue("POPC"), ResidueCategory::Lipid);
        assert_eq!(classify_residue("UNK"), ResidueCategory::Other);
    }

    #[test]
    fn test_is_standard_amino_acid() {
        assert!(is_standard_amino_acid("ALA"));
        assert!(is_standard_amino_acid("HIS"));
        assert!(!is_standard_amino_acid("HIP")); // protonation variant
        assert!(!is_standard_amino_acid("CYX")); // disulfide cysteine
        assert!(!is_standard_amino_acid("MSE")); // selenoMet
        assert!(!is_standard_amino_acid("SEP")); // phosphoSer
        assert!(!is_standard_amino_acid("ACE")); // cap
        assert!(!is_standard_amino_acid("HOH"));
    }

    #[test]
    fn test_is_standard_nucleotide() {
        assert!(is_standard_nucleotide("DA"));
        assert!(is_standard_nucleotide("A"));
        assert!(is_standard_nucleotide("GUA"));
        assert!(!is_standard_nucleotide("PSU")); // pseudouridine
        assert!(!is_standard_nucleotide("5MC")); // 5-methylcytosine
        assert!(!is_standard_nucleotide("8OG")); // 8-oxoguanine
    }
}
