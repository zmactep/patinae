//! Chain assignment for molecules without chain identifiers
//!
//! Extracted from molecule.rs — provides automatic chain ID assignment
//! for formats like GRO that lack chain information.

use std::collections::HashMap;
use std::sync::Arc;

use crate::atom::AtomResidue;
use crate::index::AtomIndex;
use crate::molecule::ObjectMolecule;
use crate::residue::{classify_residue, ResidueCategory};

// ============================================================================
// Chain ID Generator
// ============================================================================

/// Generates chain IDs: A, B, C, ..., Z, AA, AB, ..., AZ, BA, ...
struct ChainIdGenerator {
    counter: usize,
}

impl ChainIdGenerator {
    fn new() -> Self {
        Self { counter: 0 }
    }

    fn next(&mut self) -> String {
        let id = self.counter;
        self.counter += 1;

        if id < 26 {
            String::from((b'A' + id as u8) as char)
        } else {
            // Bijective base-26: 26=AA, 27=AB, ..., 51=AZ, 52=BA, ...
            let mut result = String::new();
            let mut n = id;
            loop {
                result.insert(0, (b'A' + (n % 26) as u8) as char);
                n /= 26;
                if n == 0 {
                    break;
                }
                n -= 1;
            }
            result
        }
    }
}

/// Assign chain IDs to atoms that have no chain information.
///
/// Designed for formats like GRO that lack chain identifiers.
/// Chain breaks occur when:
/// - Residue category changes
/// - Residue number decreases (GROMACS wraps at 99999)
/// - No backbone bond between consecutive polymer residues
///
/// All solvent residues share one chain. All ions share one chain.
///
/// Must be called after `classify_atoms()` and `generate_bonds()`.
pub(crate) fn assign_chains(mol: &mut ObjectMolecule) {
    // Only act if all atoms have empty chain IDs
    if mol.atoms.iter().any(|a| !a.residue.chain.is_empty()) {
        return;
    }

    // Collect residue info.
    let residues: Vec<(std::ops::Range<usize>, ResidueCategory, i32)> = mol
        .residues()
        .map(|r| {
            let cat = classify_residue(r.resn());
            let resv = r.resv();
            (r.atom_range, cat, resv)
        })
        .collect();

    if residues.is_empty() {
        return;
    }

    // Assign chain labels.
    let mut chain_labels: Vec<String> = vec![String::new(); residues.len()];
    let mut chain_gen = ChainIdGenerator::new();

    let mut solvent_indices: Vec<usize> = Vec::new();
    let mut ion_indices: Vec<usize> = Vec::new();

    let mut i = 0;
    while i < residues.len() {
        let cat = residues[i].1;

        match cat {
            ResidueCategory::Protein | ResidueCategory::Nucleic => {
                let chain_id = chain_gen.next();
                chain_labels[i] = chain_id.clone();

                let mut j = i + 1;
                while j < residues.len() {
                    let next_cat = residues[j].1;
                    let next_resv = residues[j].2;
                    let prev_resv = residues[j - 1].2;

                    if next_cat != cat {
                        break;
                    }
                    if next_resv < prev_resv {
                        break;
                    }
                    if !has_backbone_bond(mol, &residues[j - 1].0, &residues[j].0, cat) {
                        break;
                    }

                    chain_labels[j] = chain_id.clone();
                    j += 1;
                }
                i = j;
            }
            ResidueCategory::Solvent => {
                solvent_indices.push(i);
                i += 1;
            }
            ResidueCategory::Ion => {
                ion_indices.push(i);
                i += 1;
            }
            ResidueCategory::Lipid => {
                let chain_id = chain_gen.next();
                chain_labels[i] = chain_id.clone();

                let mut j = i + 1;
                while j < residues.len() {
                    if residues[j].1 != ResidueCategory::Lipid {
                        break;
                    }
                    if residues[j].2 < residues[j - 1].2 {
                        break;
                    }
                    chain_labels[j] = chain_id.clone();
                    j += 1;
                }
                i = j;
            }
            ResidueCategory::Other => {
                let chain_id = chain_gen.next();
                chain_labels[i] = chain_id.clone();

                let mut j = i + 1;
                while j < residues.len() {
                    if residues[j].1 != ResidueCategory::Other {
                        break;
                    }
                    if residues[j].2 < residues[j - 1].2 {
                        break;
                    }
                    chain_labels[j] = chain_id.clone();
                    j += 1;
                }
                i = j;
            }
        }
    }

    // Assign shared chain for solvent and ions
    if !solvent_indices.is_empty() {
        let solvent_chain = chain_gen.next();
        for &idx in &solvent_indices {
            chain_labels[idx] = solvent_chain.clone();
        }
    }
    if !ion_indices.is_empty() {
        let ion_chain = chain_gen.next();
        for &idx in &ion_indices {
            chain_labels[idx] = ion_chain.clone();
        }
    }

    // Apply chain labels to atoms.
    let residue_ranges: Vec<std::ops::Range<usize>> =
        mol.residues().map(|r| r.atom_range).collect();

    for (res_idx, range) in residue_ranges.iter().enumerate() {
        let chain = &chain_labels[res_idx];
        if chain.is_empty() {
            continue;
        }

        // One display residue can contain several distinct source label identities.
        let mut replacements: HashMap<Arc<AtomResidue>, Arc<AtomResidue>> = HashMap::new();
        for atom_idx in range.clone() {
            let original = &mol.atoms[atom_idx].residue;
            let replacement = replacements.entry(Arc::clone(original)).or_insert_with(|| {
                let mut residue = (**original).clone();
                residue.key.chain = chain.clone();
                Arc::new(residue)
            });
            mol.atoms[atom_idx].residue = Arc::clone(replacement);
        }
    }
}

/// Check if there's a backbone bond between two consecutive residues.
fn has_backbone_bond(
    mol: &ObjectMolecule,
    prev_range: &std::ops::Range<usize>,
    curr_range: &std::ops::Range<usize>,
    cat: ResidueCategory,
) -> bool {
    let (prev_name, curr_name) = match cat {
        ResidueCategory::Protein => ("C", "N"),
        ResidueCategory::Nucleic => ("O3'", "P"),
        _ => return false,
    };

    let prev_atom_idx = mol.atoms[prev_range.clone()]
        .iter()
        .enumerate()
        .find(|(_, a)| &*a.name == prev_name)
        .map(|(local, _)| AtomIndex((prev_range.start + local) as u32));

    let curr_atom_idx = mol.atoms[curr_range.clone()]
        .iter()
        .enumerate()
        .find(|(_, a)| &*a.name == curr_name)
        .map(|(local, _)| AtomIndex((curr_range.start + local) as u32));

    match (prev_atom_idx, curr_atom_idx) {
        (Some(p), Some(c)) => mol.find_bond(p, c).is_some(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chain_id_generator() {
        let mut gen = ChainIdGenerator::new();
        assert_eq!(gen.next(), "A");
        assert_eq!(gen.next(), "B");
        // Skip to Z
        for _ in 2..26 {
            gen.next();
        }
        assert_eq!(gen.next(), "AA");
        assert_eq!(gen.next(), "AB");
    }
}
