//! mmCIF entities, referenced from atoms by `label_entity_id` and `label_seq_id`.
//!
//! Entities describe the structure as loaded from the file. Edits made afterwards
//! (mutation, atom removal) do not update them, so an edited residue may differ from
//! [`EntityPolymer::monomer`] at its `label_seq_id`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// A chemically distinct part of the structure (`_entity`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entity {
    /// `_entity.id`, matched by `AtomResidue::label_entity_id`
    pub id: String,
    /// `_entity.type`; `None` when the file does not state it
    pub kind: Option<EntityKind>,
    /// Polymer data from `_entity_poly` and `_entity_poly_seq`
    pub polymer: Option<EntityPolymer>,
}

/// `_entity.type`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntityKind {
    Polymer,
    NonPolymer,
    Branched,
    Macrolide,
    Water,
    /// Value outside the PDBx/mmCIF enumeration, kept verbatim
    Other(String),
}

impl EntityKind {
    /// Parse an `_entity.type` value (case-insensitive).
    pub fn from_mmcif(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "polymer" => Self::Polymer,
            "non-polymer" => Self::NonPolymer,
            "branched" => Self::Branched,
            "macrolide" => Self::Macrolide,
            "water" => Self::Water,
            _ => Self::Other(value.to_owned()),
        }
    }
}

/// `_entity_poly.type`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolymerKind {
    PeptideL,
    PeptideD,
    Dna,
    Rna,
    DnaRnaHybrid,
    PeptideNucleicAcid,
    CyclicPseudoPeptide,
    /// Value outside the PDBx/mmCIF enumeration (including `other`), kept verbatim
    Other(String),
}

impl PolymerKind {
    /// Parse an `_entity_poly.type` value (case-insensitive).
    pub fn from_mmcif(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "polypeptide(l)" => Self::PeptideL,
            "polypeptide(d)" => Self::PeptideD,
            "polydeoxyribonucleotide" => Self::Dna,
            "polyribonucleotide" => Self::Rna,
            "polydeoxyribonucleotide/polyribonucleotide hybrid" => Self::DnaRnaHybrid,
            "peptide nucleic acid" => Self::PeptideNucleicAcid,
            "cyclic-pseudo-peptide" => Self::CyclicPseudoPeptide,
            _ => Self::Other(value.to_owned()),
        }
    }
}

/// Polymer description of an entity.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityPolymer {
    /// `_entity_poly.type`; `None` when the file has no `_entity_poly` row
    pub kind: Option<PolymerKind>,
    /// `_entity_poly_seq` by `num - 1`, unresolved residues included
    pub sequence: Vec<String>,
    /// Microheterogeneous monomers after the first, keyed by `num`
    pub alternatives: BTreeMap<u32, Vec<String>>,
}

impl EntityPolymer {
    /// Monomer at a `label_seq_id` / `_entity_poly_seq.num` position.
    pub fn monomer(&self, seq_id: u32) -> Option<&str> {
        let index = usize::try_from(seq_id).ok()?.checked_sub(1)?;
        self.sequence.get(index).map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_parse_dictionary_values_and_keep_unknown_ones() {
        assert_eq!(
            EntityKind::from_mmcif("non-polymer"),
            EntityKind::NonPolymer
        );
        assert_eq!(EntityKind::from_mmcif("Water"), EntityKind::Water);
        assert_eq!(
            EntityKind::from_mmcif("ligand"),
            EntityKind::Other("ligand".to_owned())
        );
        assert_eq!(
            PolymerKind::from_mmcif("polypeptide(L)"),
            PolymerKind::PeptideL
        );
        assert_eq!(
            PolymerKind::from_mmcif("other"),
            PolymerKind::Other("other".to_owned())
        );
    }

    #[test]
    fn monomer_is_looked_up_by_one_based_position() {
        let polymer = EntityPolymer {
            sequence: vec!["MET".to_owned(), "GLY".to_owned()],
            ..Default::default()
        };

        assert_eq!(polymer.monomer(1), Some("MET"));
        assert_eq!(polymer.monomer(2), Some("GLY"));
        assert_eq!(polymer.monomer(0), None);
        assert_eq!(polymer.monomer(3), None);
    }
}
