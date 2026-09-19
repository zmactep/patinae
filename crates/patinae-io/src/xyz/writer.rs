//! XYZ file writer
//!
//! Writes molecular structures in XYZ coordinate format.

use std::io::Write;

use patinae_mol::ObjectMolecule;

use crate::error::IoResult;
use crate::traits::MoleculeWriter;

/// XYZ file writer
pub struct XyzWriter<W> {
    writer: W,
    state: Option<usize>,
}

impl<W: Write> XyzWriter<W> {
    /// Create a new XYZ writer
    pub fn new(writer: W) -> Self {
        XyzWriter {
            writer,
            state: None,
        }
    }

    /// Create an XYZ writer that writes a specific state
    pub fn with_state(writer: W, state: usize) -> Self {
        XyzWriter {
            writer,
            state: Some(state),
        }
    }

    /// Write a single frame
    fn write_frame(&mut self, mol: &ObjectMolecule, state: usize) -> IoResult<()> {
        let n_atoms = mol.atom_count();

        // Line 1: Number of atoms
        writeln!(self.writer, "{}", n_atoms)?;

        // Line 2: Comment line (use title or name)
        let comment = if !mol.title.is_empty() {
            &mol.title
        } else if !mol.name.is_empty() {
            &mol.name
        } else {
            "molecule"
        };
        writeln!(self.writer, "{}", comment)?;

        // Atom lines
        for (idx, atom) in mol.atoms_indexed() {
            let coord = mol.get_coord(idx, state).unwrap_or_default();
            writeln!(
                self.writer,
                "{:2}  {:14.8}  {:14.8}  {:14.8}",
                atom.element.symbol(),
                coord.x,
                coord.y,
                coord.z
            )?;
        }

        Ok(())
    }

    /// Write a molecule (all states as trajectory or single state)
    fn write_molecule(&mut self, mol: &ObjectMolecule) -> IoResult<()> {
        let num_states = mol.state_count();

        if let Some(state) = self.state {
            // Write specific state
            let state = state.min(num_states.saturating_sub(1));
            self.write_frame(mol, state)?;
        } else {
            // Write all states (trajectory)
            for state in 0..num_states.max(1) {
                self.write_frame(mol, state)?;
            }
        }

        Ok(())
    }
}

impl<W: Write> MoleculeWriter for XyzWriter<W> {
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
                None => XyzWriter::new(&mut output),
                Some(state) => XyzWriter::with_state(&mut output, state),
            };
            writer.write(&mol).unwrap();
            let text = String::from_utf8(output).unwrap();
            let lines: Vec<String> = text
                .lines()
                .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
                .collect();
            let expected_first = match selected {
                None => "C 1.23456705 -2.34567809 3.45678902",
                Some(_) => "C 9.50000000 1.25000000 -2.75000000",
            };
            assert_eq!(&lines[..3], ["4", "asymmetric", expected_first]);
            assert_eq!(lines.len(), if selected.is_none() { 12 } else { 6 });
            if selected.is_none() {
                assert_eq!(
                    &lines[6..9],
                    ["4", "asymmetric", "C 9.50000000 1.25000000 -2.75000000"]
                );
            }
            let parsed = crate::xyz::XyzReader::new(text.as_bytes()).read().unwrap();
            assert_eq!(parsed.atom_count(), mol.atom_count());
            assert_eq!(
                parsed.atoms().map(|a| a.element).collect::<Vec<_>>(),
                mol.atoms().map(|a| a.element).collect::<Vec<_>>()
            );
            let states: &[usize] = if selected.is_none() { &[0, 1] } else { &[1] };
            assert_eq!(parsed.state_count(), states.len());
            // Half the decimal output unit, plus one f32 rounding unit at this coordinate scale.
            const COORD_TOLERANCE: f32 = 0.5e-8 + f32::EPSILON * 10.0;
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
        }
    }
}
