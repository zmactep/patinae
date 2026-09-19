//! MOL2 file writer
//!
//! Writes molecular structures in TRIPOS MOL2 format.

use std::io::Write;

use patinae_mol::{BondOrder, ObjectMolecule};

use crate::error::IoResult;
use crate::traits::MoleculeWriter;

/// MOL2 file writer
pub struct Mol2Writer<W> {
    writer: W,
    state: Option<usize>,
}

impl<W: Write> Mol2Writer<W> {
    /// Create a new MOL2 writer
    pub fn new(writer: W) -> Self {
        Mol2Writer {
            writer,
            state: None,
        }
    }

    /// Create a MOL2 writer that writes a specific state
    pub fn with_state(writer: W, state: usize) -> Self {
        Mol2Writer {
            writer,
            state: Some(state),
        }
    }

    /// Get SYBYL atom type for an atom
    fn get_sybyl_type(atom: &patinae_mol::Atom) -> String {
        // Simplified SYBYL type assignment
        // A full implementation would consider hybridization, bonds, etc.
        let base = atom.element.symbol();

        // Common types
        match atom.element {
            patinae_mol::Element::Carbon => "C.3".to_string(),
            patinae_mol::Element::Nitrogen => "N.3".to_string(),
            patinae_mol::Element::Oxygen => "O.3".to_string(),
            patinae_mol::Element::Sulfur => "S.3".to_string(),
            patinae_mol::Element::Phosphorus => "P.3".to_string(),
            patinae_mol::Element::Hydrogen => "H".to_string(),
            _ => base.to_string(),
        }
    }

    /// Write a single molecule
    fn write_molecule(&mut self, mol: &ObjectMolecule) -> IoResult<()> {
        let state = self.state.unwrap_or(0);

        // @<TRIPOS>MOLECULE
        writeln!(self.writer, "@<TRIPOS>MOLECULE")?;
        writeln!(
            self.writer,
            "{}",
            if mol.name.is_empty() {
                "molecule"
            } else {
                &mol.name
            }
        )?;

        // Counts line
        let n_atoms = mol.atom_count();
        let n_bonds = mol.bond_count();
        let n_subst = mol.residue_count().max(1);
        writeln!(self.writer, " {} {} {} 0 0", n_atoms, n_bonds, n_subst)?;

        // Molecule type
        writeln!(self.writer, "SMALL")?;

        // Charge type
        let has_charges = mol.atoms().any(|a| a.partial_charge != 0.0);
        if has_charges {
            writeln!(self.writer, "USER_CHARGES")?;
        } else {
            writeln!(self.writer, "NO_CHARGES")?;
        }
        writeln!(self.writer)?;

        // @<TRIPOS>ATOM
        writeln!(self.writer, "@<TRIPOS>ATOM")?;

        for (idx, atom) in mol.atoms_indexed() {
            let coord = mol.get_coord(idx, state).unwrap_or_default();
            let sybyl_type = Self::get_sybyl_type(atom);

            // subst_name: residue name + residue number
            let subst_name = if atom.residue.resn.is_empty() {
                format!("UNK{}", atom.residue.resv.max(1))
            } else {
                format!("{}{}", atom.residue.resn, atom.residue.resv.max(1))
            };

            writeln!(
                self.writer,
                "{:7} {:<8} {:10.4} {:10.4} {:10.4} {:<8} {:3} {:<8} {:8.4}",
                idx.0 + 1, // 1-indexed
                atom.name,
                coord.x,
                coord.y,
                coord.z,
                sybyl_type,
                atom.residue.resv.max(1),
                subst_name,
                atom.partial_charge
            )?;
        }

        // @<TRIPOS>BOND
        writeln!(self.writer, "@<TRIPOS>BOND")?;

        for (bond_idx, bond) in mol.bonds_indexed() {
            let bond_type = match bond.order {
                BondOrder::Single => "1",
                BondOrder::Double => "2",
                BondOrder::Triple => "3",
                BondOrder::Aromatic => "ar",
                BondOrder::Unknown => "un",
            };

            writeln!(
                self.writer,
                "{:6} {:5} {:5} {}",
                bond_idx.0 + 1, // 1-indexed
                bond.atom1.0 + 1,
                bond.atom2.0 + 1,
                bond_type
            )?;
        }

        Ok(())
    }
}

impl<W: Write> MoleculeWriter for Mol2Writer<W> {
    fn write(&mut self, mol: &ObjectMolecule) -> IoResult<()> {
        self.write_molecule(mol)
    }

    fn write_all(&mut self, molecules: &[ObjectMolecule]) -> IoResult<()> {
        for mol in molecules {
            self.write_molecule(mol)?;
        }
        Ok(())
    }

    fn flush(&mut self) -> IoResult<()> {
        self.writer.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::MoleculeReader;
    use lin_alg::f32::Vec3;
    use patinae_mol::{Atom, AtomIndex, BondOrder, CoordSet, Element};

    fn asymmetric_molecule() -> ObjectMolecule {
        let mut mol = ObjectMolecule::new("asymmetric");
        for (name, element) in [
            ("C1", Element::Carbon),
            ("O2", Element::Oxygen),
            ("N3", Element::Nitrogen),
            ("S4", Element::Sulfur),
        ] {
            mol.add_atom(Atom::new(name, element));
        }
        for (a, b, order) in [
            (0, 2, BondOrder::Single),
            (0, 1, BondOrder::Double),
            (2, 3, BondOrder::Triple),
        ] {
            mol.add_bond(AtomIndex(a), AtomIndex(b), order).unwrap();
        }
        mol.add_coord_set(CoordSet::from_vec3(&[
            Vec3::new(1.234567, -2.345678, 3.456789),
            Vec3::new(-4.567891, 5.678912, -6.789123),
            Vec3::new(7.891234, -8.912345, 9.123456),
            Vec3::new(-0.123456, 2.987654, -3.876543),
        ]));
        mol.add_coord_set(CoordSet::from_vec3(&[
            Vec3::new(9.5, 1.25, -2.75),
            Vec3::new(-3.25, 8.5, 1.125),
            Vec3::new(2.625, -4.75, 6.5),
            Vec3::new(-7.5, 3.125, -0.625),
        ]));
        mol
    }

    #[test]
    fn golden_records_and_roundtrip_preserve_atom_identity_coordinates_and_states() {
        let mol = asymmetric_molecule();
        for selected in [None, Some(1)] {
            let mut output = Vec::new();
            let mut writer = match selected {
                None => Mol2Writer::new(&mut output),
                Some(state) => Mol2Writer::with_state(&mut output, state),
            };
            writer.write(&mol).unwrap();
            let text = String::from_utf8(output).unwrap();
            let lines: Vec<String> = text
                .lines()
                .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
                .collect();
            assert_eq!(
                &lines[..7],
                [
                    "@<TRIPOS>MOLECULE",
                    "asymmetric",
                    "4 3 1 0 0",
                    "SMALL",
                    "NO_CHARGES",
                    "",
                    "@<TRIPOS>ATOM"
                ]
            );
            assert_eq!(
                lines[7],
                if selected.is_none() {
                    "1 C1 1.2346 -2.3457 3.4568 C.3 1 UNK1 0.0000"
                } else {
                    "1 C1 9.5000 1.2500 -2.7500 C.3 1 UNK1 0.0000"
                }
            );
            assert_eq!(
                &lines[11..],
                ["@<TRIPOS>BOND", "1 1 3 1", "2 1 2 2", "3 3 4 3"]
            );
            let parsed = crate::mol2::Mol2Reader::new(text.as_bytes())
                .read()
                .unwrap();
            assert_eq!(parsed.atom_count(), mol.atom_count());
            assert_eq!(
                parsed.atoms().map(|a| a.element).collect::<Vec<_>>(),
                mol.atoms().map(|a| a.element).collect::<Vec<_>>()
            );
            let states: &[usize] = if selected.is_none() { &[0] } else { &[1] };
            assert_eq!(parsed.state_count(), states.len());
            // Half the decimal output unit, plus one f32 rounding unit at this coordinate scale.
            const COORD_TOLERANCE: f32 = 0.5e-4 + f32::EPSILON * 10.0;
            for (parsed_state, &source_state) in states.iter().enumerate() {
                for (index, _) in mol.atoms_indexed() {
                    let actual = parsed.get_coord(index, parsed_state).unwrap();
                    let expected = mol.get_coord(index, source_state).unwrap();
                    for (axis, (a, e)) in [actual.x, actual.y, actual.z]
                        .into_iter()
                        .zip([expected.x, expected.y, expected.z])
                        .enumerate()
                    {
                        assert!(
                            (a - e).abs() <= COORD_TOLERANCE,
                            "state={source_state}, atom={index:?}, axis={axis}: {a} != {e}"
                        );
                    }
                }
            }
            assert_eq!(
                parsed
                    .bonds()
                    .map(|b| (b.atom1, b.atom2, b.order))
                    .collect::<Vec<_>>(),
                mol.bonds()
                    .map(|b| (b.atom1, b.atom2, b.order))
                    .collect::<Vec<_>>()
            );
        }
    }
}
