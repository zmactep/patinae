//! mmCIF file writer
//!
//! Writes molecular structures in mmCIF format.

use std::io::Write;

use patinae_mol::ObjectMolecule;

use crate::error::IoResult;
use crate::traits::MoleculeWriter;

/// mmCIF file writer
pub struct CifWriter<W> {
    writer: W,
    state: Option<usize>,
}

impl<W: Write> CifWriter<W> {
    /// Create a new CIF writer
    pub fn new(writer: W) -> Self {
        CifWriter {
            writer,
            state: None,
        }
    }

    /// Create a CIF writer that writes a specific state
    pub fn with_state(writer: W, state: usize) -> Self {
        CifWriter {
            writer,
            state: Some(state),
        }
    }

    /// Escape a string value for CIF format
    fn escape_value(s: &str) -> String {
        if s.is_empty() {
            return ".".to_string();
        }
        if s.contains(' ') || s.contains('\'') || s.contains('"') || s.contains('\n') {
            if s.contains('\n') {
                // Use semicolon text field
                format!("\n;{}\n;", s)
            } else if s.contains('\'') {
                format!("\"{}\"", s)
            } else {
                format!("'{}'", s)
            }
        } else {
            s.to_string()
        }
    }

    /// Write the molecule
    fn write_molecule(&mut self, mol: &ObjectMolecule) -> IoResult<()> {
        let name = if mol.name.is_empty() {
            "molecule"
        } else {
            &mol.name
        };

        // Data block header
        writeln!(self.writer, "data_{}", name.replace(' ', "_"))?;
        writeln!(self.writer)?;

        // Entry info
        writeln!(self.writer, "_entry.id {}", Self::escape_value(name))?;
        writeln!(self.writer)?;

        // Cell parameters
        if let Some(ref sym) = mol.symmetry {
            writeln!(self.writer, "_cell.length_a {:8.3}", sym.cell_lengths[0])?;
            writeln!(self.writer, "_cell.length_b {:8.3}", sym.cell_lengths[1])?;
            writeln!(self.writer, "_cell.length_c {:8.3}", sym.cell_lengths[2])?;
            writeln!(self.writer, "_cell.angle_alpha {:7.2}", sym.cell_angles[0])?;
            writeln!(self.writer, "_cell.angle_beta {:7.2}", sym.cell_angles[1])?;
            writeln!(self.writer, "_cell.angle_gamma {:7.2}", sym.cell_angles[2])?;
            writeln!(self.writer)?;
            writeln!(
                self.writer,
                "_symmetry.space_group_name_H-M {}",
                Self::escape_value(&sym.space_group)
            )?;
            writeln!(self.writer)?;
        }

        // Atom site loop
        writeln!(self.writer, "loop_")?;
        writeln!(self.writer, "_atom_site.id")?;
        writeln!(self.writer, "_atom_site.type_symbol")?;
        writeln!(self.writer, "_atom_site.label_atom_id")?;
        writeln!(self.writer, "_atom_site.label_comp_id")?;
        writeln!(self.writer, "_atom_site.label_asym_id")?;
        writeln!(self.writer, "_atom_site.label_seq_id")?;
        writeln!(self.writer, "_atom_site.pdbx_PDB_ins_code")?;
        writeln!(self.writer, "_atom_site.Cartn_x")?;
        writeln!(self.writer, "_atom_site.Cartn_y")?;
        writeln!(self.writer, "_atom_site.Cartn_z")?;
        writeln!(self.writer, "_atom_site.occupancy")?;
        writeln!(self.writer, "_atom_site.B_iso_or_equiv")?;
        writeln!(self.writer, "_atom_site.pdbx_formal_charge")?;
        writeln!(self.writer, "_atom_site.auth_atom_id")?;
        writeln!(self.writer, "_atom_site.auth_comp_id")?;
        writeln!(self.writer, "_atom_site.auth_asym_id")?;
        writeln!(self.writer, "_atom_site.auth_seq_id")?;
        writeln!(self.writer, "_atom_site.pdbx_PDB_model_num")?;
        writeln!(self.writer, "_atom_site.group_PDB")?;

        let num_states = mol.state_count();
        let states_to_write: Vec<usize> = if let Some(state) = self.state {
            vec![state.min(num_states.saturating_sub(1))]
        } else {
            (0..num_states.max(1)).collect()
        };

        let mut atom_id = 1;
        for (model_num, state) in states_to_write.iter().enumerate() {
            for (idx, atom) in mol.atoms_indexed() {
                let coord = mol.get_coord(idx, *state).unwrap_or_default();

                let group = if atom.state.hetatm { "HETATM" } else { "ATOM" };
                let chain = if atom.residue.chain.is_empty() {
                    "A"
                } else {
                    &atom.residue.chain
                };
                let resn = if atom.residue.resn.is_empty() {
                    "UNK"
                } else {
                    &atom.residue.resn
                };
                let inscode = if atom.residue.inscode == ' ' {
                    "?"
                } else {
                    &atom.residue.inscode.to_string()
                };

                writeln!(
                    self.writer,
                    "{} {} {} {} {} {} {} {:8.3} {:8.3} {:8.3} {:5.2} {:6.2} {} {} {} {} {} {} {}",
                    atom_id,
                    atom.element.symbol(),
                    Self::escape_value(&atom.name),
                    Self::escape_value(resn),
                    chain,
                    atom.residue.resv,
                    inscode,
                    coord.x,
                    coord.y,
                    coord.z,
                    atom.occupancy,
                    atom.b_factor,
                    if atom.formal_charge == 0 {
                        "?".to_string()
                    } else {
                        atom.formal_charge.to_string()
                    },
                    Self::escape_value(&atom.name),
                    Self::escape_value(resn),
                    chain,
                    atom.residue.resv,
                    model_num + 1,
                    group
                )?;

                atom_id += 1;
            }
        }

        writeln!(self.writer, "#")?;

        Ok(())
    }
}

impl<W: Write> MoleculeWriter for CifWriter<W> {
    fn write(&mut self, mol: &ObjectMolecule) -> IoResult<()> {
        self.write_molecule(mol)
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
    use patinae_mol::{Atom, CoordSet, Element};

    fn create_alanine() -> ObjectMolecule {
        let mut mol = ObjectMolecule::new("alanine");

        let mut n = Atom::new("N", Element::Nitrogen);
        n.set_residue("ALA", 1, "A");
        mol.add_atom(n);

        let mut ca = Atom::new("CA", Element::Carbon);
        ca.set_residue("ALA", 1, "A");
        mol.add_atom(ca);

        let coords = CoordSet::from_vec3(&[Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.5, 0.0, 0.0)]);
        mol.add_coord_set(coords);

        mol
    }

    #[test]
    fn test_write_cif() {
        let mol = create_alanine();
        let mut output = Vec::new();

        {
            let mut writer = CifWriter::new(&mut output);
            writer.write(&mol).unwrap();
        }

        let cif_string = String::from_utf8(output).unwrap();

        assert!(cif_string.contains("data_alanine"));
        assert!(cif_string.contains("_atom_site.id"));
        assert!(cif_string.contains("ALA"));
        let parsed = crate::cif::CifReader::new(cif_string.as_bytes())
            .read()
            .unwrap();
        assert_eq!(parsed.atom_count(), mol.atom_count());
        for ((index, expected), actual) in mol.atoms_indexed().zip(parsed.atoms()) {
            assert_eq!(actual.element, expected.element);
            assert_eq!(actual.name, expected.name);
            assert_eq!(actual.residue.resn, expected.residue.resn);
            assert_eq!(actual.residue.chain, expected.residue.chain);
            assert_eq!(actual.residue.resv, expected.residue.resv);
            assert_eq!(parsed.get_coord(index, 0), mol.get_coord(index, 0));
        }
    }

    #[test]
    fn test_escape_value() {
        assert_eq!(CifWriter::<Vec<u8>>::escape_value(""), ".");
        assert_eq!(CifWriter::<Vec<u8>>::escape_value("simple"), "simple");
        assert_eq!(
            CifWriter::<Vec<u8>>::escape_value("with space"),
            "'with space'"
        );
    }
}
