//! SDF/MOL file writer
//!
//! Writes molecular structures in MDL SDF/MOL V2000 format.

use std::io::Write;

use patinae_mol::{BondOrder, ObjectMolecule};

use crate::error::{IoError, IoResult};
use crate::traits::MoleculeWriter;

/// SDF/MOL file writer
pub struct SdfWriter<W> {
    writer: W,
    state: Option<usize>,
}

impl<W: Write> SdfWriter<W> {
    /// Create a new SDF writer
    pub fn new(writer: W) -> Self {
        SdfWriter {
            writer,
            state: None,
        }
    }

    /// Create an SDF writer that writes a specific state
    pub fn with_state(writer: W, state: usize) -> Self {
        SdfWriter {
            writer,
            state: Some(state),
        }
    }

    /// Write a single molecule
    fn write_molecule(&mut self, mol: &ObjectMolecule) -> IoResult<()> {
        let state = self.state.unwrap_or(0);

        // Line 1: Molecule name
        writeln!(self.writer, "{}", mol.name)?;

        // Line 2: Program/timestamp (blank or informational)
        writeln!(self.writer, "  patinae-io          3D")?;

        // Line 3: Comment
        if mol.title.is_empty() {
            writeln!(self.writer)?;
        } else {
            writeln!(self.writer, "{}", &mol.title[..mol.title.len().min(80)])?;
        }

        // Line 4: Counts line
        let n_atoms = mol.atom_count();
        let n_bonds = mol.bond_count();

        if n_atoms > 999 || n_bonds > 999 {
            return Err(IoError::unsupported(
                "Molecule too large for V2000 format (max 999 atoms/bonds)",
            ));
        }

        writeln!(
            self.writer,
            "{:3}{:3}  0  0  0  0  0  0  0  0999 V2000",
            n_atoms, n_bonds
        )?;

        // Atom block
        for (idx, atom) in mol.atoms_indexed() {
            let coord = mol.get_coord(idx, state).unwrap_or_default();

            writeln!(
                self.writer,
                "{:10.4}{:10.4}{:10.4} {:<3} 0  0  0  0  0  0  0  0  0  0  0  0",
                coord.x,
                coord.y,
                coord.z,
                atom.element.symbol()
            )?;
        }

        // Bond block
        for (_, bond) in mol.bonds_indexed() {
            let bond_type = match bond.order {
                BondOrder::Single => 1,
                BondOrder::Double => 2,
                BondOrder::Triple => 3,
                BondOrder::Aromatic => 4,
                BondOrder::Unknown => 1,
            };

            writeln!(
                self.writer,
                "{:3}{:3}{:3}  0  0  0  0",
                bond.atom1.0 + 1, // Convert to 1-indexed
                bond.atom2.0 + 1,
                bond_type
            )?;
        }

        // Collect atoms with non-zero formal charges
        let charged_atoms: Vec<_> = mol
            .atoms_indexed()
            .filter(|(_, atom)| atom.formal_charge != 0)
            .collect();

        // Write M  CHG lines (max 8 per line)
        for chunk in charged_atoms.chunks(8) {
            write!(self.writer, "M  CHG{:3}", chunk.len())?;
            for (idx, atom) in chunk {
                write!(self.writer, " {:3} {:3}", idx.0 + 1, atom.formal_charge)?;
            }
            writeln!(self.writer)?;
        }

        // End of molecule
        writeln!(self.writer, "M  END")?;

        Ok(())
    }
}

impl<W: Write> MoleculeWriter for SdfWriter<W> {
    fn write(&mut self, mol: &ObjectMolecule) -> IoResult<()> {
        self.write_molecule(mol)?;
        // Write SDF separator
        writeln!(self.writer, "$$$$")?;
        Ok(())
    }

    fn write_all(&mut self, molecules: &[ObjectMolecule]) -> IoResult<()> {
        for mol in molecules {
            self.write_molecule(mol)?;
            writeln!(self.writer, "$$$$")?;
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
                None => SdfWriter::new(&mut output),
                Some(state) => SdfWriter::with_state(&mut output, state),
            };
            writer.write(&mol).unwrap();
            let text = String::from_utf8(output).unwrap();
            let lines: Vec<String> = text
                .lines()
                .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
                .collect();
            assert_eq!(
                &lines[..4],
                [
                    "asymmetric",
                    "patinae-io 3D",
                    "",
                    "4 3 0 0 0 0 0 0 0 0999 V2000"
                ]
            );
            assert_eq!(
                lines[4],
                if selected.is_none() {
                    "1.2346 -2.3457 3.4568 C 0 0 0 0 0 0 0 0 0 0 0 0"
                } else {
                    "9.5000 1.2500 -2.7500 C 0 0 0 0 0 0 0 0 0 0 0 0"
                }
            );
            assert_eq!(
                &lines[8..],
                [
                    "1 3 1 0 0 0 0",
                    "1 2 2 0 0 0 0",
                    "3 4 3 0 0 0 0",
                    "M END",
                    "$$$$"
                ]
            );
            let parsed = crate::sdf::SdfReader::new(text.as_bytes()).read().unwrap();
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
