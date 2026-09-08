//! Restore per-molecule sharing while decoding atoms, without changing wire data.

use std::fmt;
use std::sync::Arc;

use ahash::AHashSet;
use serde::de::{Deserializer, SeqAccess, Visitor};

use crate::{Atom, AtomResidue};

pub(crate) fn deserialize_atoms<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<Atom>, D::Error> {
    struct AtomsVisitor;

    impl<'de> Visitor<'de> for AtomsVisitor {
        type Value = Vec<Atom>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a sequence of atoms")
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            // Do not trust an input-controlled size hint for an unbounded
            // allocation. Ordinary Vec growth also handles streaming formats.
            let mut atoms = Vec::new();
            let mut names: AHashSet<Arc<str>> = AHashSet::new();
            let mut residues: AHashSet<Arc<AtomResidue>> = AHashSet::new();
            while let Some(mut atom) = seq.next_element::<Atom>()? {
                if let Some(name) = names.get(atom.name.as_ref()) {
                    atom.name = Arc::clone(name);
                } else {
                    names.insert(Arc::clone(&atom.name));
                }
                if let Some(residue) = residues.get(atom.residue.as_ref()) {
                    atom.residue = Arc::clone(residue);
                } else {
                    residues.insert(Arc::clone(&atom.residue));
                }
                atoms.push(atom);
            }
            Ok(atoms)
        }
    }

    deserializer.deserialize_seq(AtomsVisitor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AtomIndex, CoordSet, Element, ObjectMolecule};

    #[test]
    fn molecule_round_trip_shares_equal_values_without_merging_identity_or_mutations() {
        let identities = [
            ("A", "ALA", 1, ' ', ""),
            ("B", "ALA", 1, ' ', ""),
            ("A", "GLY", 1, ' ', ""),
            ("A", "ALA", 2, ' ', ""),
            ("A", "ALA", 1, 'A', ""),
            ("A", "ALA", 1, ' ', "segment"),
            ("A", "ALA", 1, ' ', ""),
        ];
        let mut molecule = ObjectMolecule::new("sharing");
        for (i, (chain, resn, resv, icode, segi)) in identities.into_iter().enumerate() {
            let mut atom = Atom::new(if i == 3 { "N" } else { "CA" }, Element::Carbon);
            atom.residue = Arc::new(AtomResidue::from_parts(chain, resn, resv, icode, segi));
            atom.b_factor = i as f32 + 0.25;
            atom.alt = if i == 6 { 'B' } else { ' ' };
            atom.anisou = Some([i as f32; 6]);
            atom.repr.label = format!("atom-{i}");
            atom.repr.sphere_transparency = Some(i as f32 / 10.0);
            molecule.add_atom(atom);
        }
        for frame in 0..3 {
            molecule.add_coord_set(CoordSet::from_coords(
                (0..21).map(|i| i as f32 + frame as f32 / 4.0).collect(),
            ));
        }
        for named in [false, true] {
            let bytes = if named {
                rmp_serde::to_vec_named(&molecule).unwrap()
            } else {
                rmp_serde::to_vec(&molecule).unwrap()
            };
            let mut loaded: ObjectMolecule = rmp_serde::from_slice(&bytes).unwrap();
            // All serialized fields and every coordinate frame must survive.
            assert_eq!(
                rmp_serde::to_vec(&loaded).unwrap(),
                rmp_serde::to_vec(&molecule).unwrap()
            );
            let atoms = loaded.atoms_slice();
            assert!(Arc::ptr_eq(&atoms[0].residue, &atoms[6].residue));
            for atom in &atoms[1..6] {
                assert!(!Arc::ptr_eq(&atoms[0].residue, &atom.residue));
            }
            assert!(Arc::ptr_eq(&atoms[0].name, &atoms[6].name));
            assert!(!Arc::ptr_eq(&atoms[0].name, &atoms[3].name));
            let atom = loaded.get_atom_mut(AtomIndex(0)).unwrap();
            Arc::make_mut(&mut atom.residue).segi = "edited".to_string();
            atom.name = Arc::from("CB");
            assert_eq!(loaded.atoms_slice()[6].residue.segi, "");
            assert_eq!(loaded.atoms_slice()[6].name.as_ref(), "CA");
            assert_eq!(molecule.atoms_slice()[0].residue.segi, "");
        }
    }
}
