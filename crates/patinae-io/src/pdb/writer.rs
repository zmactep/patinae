//! PDB file writer
//!
//! Writes molecular structures in PDB format.

use std::collections::HashMap;
use std::io::Write;

use patinae_mol::{AtomIndex, ObjectMolecule, SecondaryStructure};

use crate::error::{IoError, IoResult};
use crate::pdb::hybrid36::hy36encode;
use crate::traits::MoleculeWriter;

use super::chain_ids::source_chain_id;

/// PDB file writer
pub struct PdbWriter<W> {
    writer: W,
    state: Option<usize>,
}

impl<W: Write> PdbWriter<W> {
    /// Create a new PDB writer
    pub fn new(writer: W) -> Self {
        PdbWriter {
            writer,
            state: None,
        }
    }

    /// Create a PDB writer that writes a specific state
    pub fn with_state(writer: W, state: usize) -> Self {
        PdbWriter {
            writer,
            state: Some(state),
        }
    }

    /// Write title record
    fn write_title(&mut self, title: &str) -> IoResult<()> {
        if !title.is_empty() {
            // Split title into 70-character chunks
            for (i, chunk) in title.as_bytes().chunks(70).enumerate() {
                let cont = if i > 0 {
                    format!("{:>2} ", i + 1)
                } else {
                    "   ".to_string()
                };
                writeln!(
                    self.writer,
                    "TITLE  {}{}",
                    cont,
                    String::from_utf8_lossy(chunk)
                )?;
            }
        }
        Ok(())
    }

    /// Write CRYST1 record
    fn write_cryst1(&mut self, mol: &ObjectMolecule) -> IoResult<()> {
        if let Some(ref sym) = mol.symmetry {
            writeln!(
                self.writer,
                "CRYST1{:9.3}{:9.3}{:9.3}{:7.2}{:7.2}{:7.2} {:11}{:4}",
                sym.cell_lengths[0],
                sym.cell_lengths[1],
                sym.cell_lengths[2],
                sym.cell_angles[0],
                sym.cell_angles[1],
                sym.cell_angles[2],
                sym.space_group,
                sym.z_value
            )?;
        }
        Ok(())
    }

    /// Write HELIX and SHEET records from per-atom secondary structure
    fn write_secondary_structure(&mut self, mol: &ObjectMolecule) -> IoResult<()> {
        // Collect contiguous runs of helix/sheet secondary structure per chain.
        // We iterate atoms and detect when ss_type or chain changes to emit records.
        struct SsRun {
            ss: SecondaryStructure,
            chain: String,
            init_resn: String,
            init_resv: i32,
            init_icode: char,
            end_resn: String,
            end_resv: i32,
            end_icode: char,
        }

        let mut runs: Vec<SsRun> = Vec::new();
        let mut current: Option<SsRun> = None;

        // Track last residue identity to avoid counting the same residue multiple times
        let mut last_residue: Option<(String, i32, char)> = None;

        for (_, atom) in mol.atoms_indexed() {
            let residue_id = (
                atom.residue.chain.clone(),
                atom.residue.resv,
                atom.residue.inscode,
            );

            // Skip duplicate atoms within the same residue
            if let Some(ref last) = last_residue {
                if *last == residue_id {
                    continue;
                }
            }
            last_residue = Some(residue_id);

            let ss = atom.ss_type;
            let is_regular = ss.is_helix() || ss.is_sheet();

            if let Some(ref mut run) = current {
                // Continue current run if same ss type and same chain
                let same_type = match (run.ss, ss) {
                    (a, b) if a == b => true,
                    // Group all helix subtypes together for HELIX records
                    (a, b) if a.is_helix() && b.is_helix() => false,
                    _ => false,
                };
                if same_type && atom.residue.chain == run.chain {
                    run.end_resn = atom.residue.resn.to_string();
                    run.end_resv = atom.residue.resv;
                    run.end_icode = atom.residue.inscode;
                    continue;
                }
                // End current run
                let finished = current.take().unwrap();
                if finished.ss.is_helix() || finished.ss.is_sheet() {
                    runs.push(finished);
                }
            }

            if is_regular {
                current = Some(SsRun {
                    ss,
                    chain: atom.residue.chain.to_string(),
                    init_resn: atom.residue.resn.to_string(),
                    init_resv: atom.residue.resv,
                    init_icode: atom.residue.inscode,
                    end_resn: atom.residue.resn.to_string(),
                    end_resv: atom.residue.resv,
                    end_icode: atom.residue.inscode,
                });
            }
        }
        // Flush last run
        if let Some(run) = current {
            if run.ss.is_helix() || run.ss.is_sheet() {
                runs.push(run);
            }
        }

        // Write HELIX records
        let mut helix_serial = 0;
        for run in &runs {
            if !run.ss.is_helix() {
                continue;
            }
            helix_serial += 1;
            let helix_class = match run.ss {
                SecondaryStructure::Helix => 1,
                SecondaryStructure::Helix310 => 5,
                SecondaryStructure::HelixPi => 3,
                _ => 1,
            };
            let chain = pdb_chain_column(&run.chain);
            // PDB HELIX format:
            // HELIX  ser hid iResN iChn iSeq iIC  eResN eChn eSeq eIC cls
            writeln!(
                self.writer,
                "HELIX  {:3} {:>3} {:3} {}{:4}{}  {:3} {}{:4}{}{:2}",
                helix_serial,
                helix_serial,
                if run.init_resn.len() > 3 {
                    &run.init_resn[..3]
                } else {
                    &run.init_resn
                },
                chain,
                run.init_resv,
                if run.init_icode == ' ' {
                    ' '
                } else {
                    run.init_icode
                },
                if run.end_resn.len() > 3 {
                    &run.end_resn[..3]
                } else {
                    &run.end_resn
                },
                chain,
                run.end_resv,
                if run.end_icode == ' ' {
                    ' '
                } else {
                    run.end_icode
                },
                helix_class
            )?;
        }

        // Write SHEET records
        // Group sheet runs by chain to assign sheet IDs
        let mut sheet_serial = 0;
        for run in &runs {
            if !run.ss.is_sheet() {
                continue;
            }
            sheet_serial += 1;
            let chain = pdb_chain_column(&run.chain);
            // PDB SHEET format:
            // SHEET  str shID nStr iResN iChn iSeq iIC  eResN eChn eSeq eIC sense
            writeln!(
                self.writer,
                "SHEET  {:3} {:>3}{:2} {:3} {}{:4}{} {:3} {}{:4}{}{:2}",
                sheet_serial,
                sheet_serial,
                1, // num_strands (1 = single strand, simplified)
                if run.init_resn.len() > 3 {
                    &run.init_resn[..3]
                } else {
                    &run.init_resn
                },
                chain,
                run.init_resv,
                if run.init_icode == ' ' {
                    ' '
                } else {
                    run.init_icode
                },
                if run.end_resn.len() > 3 {
                    &run.end_resn[..3]
                } else {
                    &run.end_resn
                },
                chain,
                run.end_resv,
                if run.end_icode == ' ' {
                    ' '
                } else {
                    run.end_icode
                },
                0 // sense (0 = first strand)
            )?;
        }

        Ok(())
    }

    /// Write ATOM/HETATM record
    fn write_atom(
        &mut self,
        serial: i32,
        atom: &patinae_mol::Atom,
        x: f32,
        y: f32,
        z: f32,
    ) -> IoResult<()> {
        let record_type = if atom.state.hetatm {
            "HETATM"
        } else {
            "ATOM  "
        };

        // Format atom name with proper justification
        let name = format_atom_name(&atom.name, atom.element.symbol());

        // Format chain (single character)
        let chain = pdb_chain_column(&atom.residue.chain);

        // Format insertion code
        let icode = if atom.residue.inscode == ' ' {
            ' '
        } else {
            atom.residue.inscode
        };

        // Format element symbol (right-justified in 2 characters)
        let element = format!("{:>2}", atom.element.symbol());

        // Format charge
        let charge = format_charge(atom.formal_charge);

        let serial_str = hy36encode(5, serial).unwrap_or_else(|| format!("{:5}", serial % 100000));
        let resv_str =
            hy36encode(4, atom.residue.resv).unwrap_or_else(|| format!("{:4}", atom.residue.resv));

        writeln!(
            self.writer,
            "{}{} {:4}{}{:3} {}{}{}   {:8.3}{:8.3}{:8.3}{:6.2}{:6.2}          {}{}",
            record_type,
            serial_str,
            name,
            if atom.alt == ' ' { ' ' } else { atom.alt },
            if atom.residue.resn.len() > 3 {
                &atom.residue.resn[..3]
            } else {
                &atom.residue.resn
            },
            chain,
            resv_str,
            icode,
            x,
            y,
            z,
            atom.occupancy,
            atom.b_factor,
            element,
            charge
        )?;

        Ok(())
    }

    /// Write TER record
    fn write_ter(&mut self, serial: i32, resn: &str, chain: &str, resv: i32) -> IoResult<()> {
        let serial_str = hy36encode(5, serial).unwrap_or_else(|| format!("{:5}", serial % 100000));
        let resv_str = hy36encode(4, resv).unwrap_or_else(|| format!("{:4}", resv));
        writeln!(
            self.writer,
            "TER   {}      {:3} {:1}{}",
            serial_str,
            if resn.len() > 3 { &resn[..3] } else { resn },
            pdb_chain_column(chain),
            resv_str
        )?;
        Ok(())
    }

    /// Write CONECT records for bonds involving at least one HETATM atom
    fn write_conect(
        &mut self,
        mol: &ObjectMolecule,
        index_to_serial: &HashMap<AtomIndex, i32>,
    ) -> IoResult<()> {
        // Group bonds by atom serial, only including bonds where at least one
        // atom is a HETATM (per PDB convention — standard covalent bonds are
        // regenerated from distances by readers)
        let mut bonds_by_serial: HashMap<i32, Vec<i32>> = HashMap::new();

        for (_, bond) in mol.bonds_indexed() {
            let (Some(atom1), Some(atom2)) = (mol.get_atom(bond.atom1), mol.get_atom(bond.atom2))
            else {
                continue;
            };

            // Only write CONECT for bonds where at least one atom is HETATM
            if !atom1.state.hetatm && !atom2.state.hetatm {
                continue;
            }

            let (Some(&serial1), Some(&serial2)) = (
                index_to_serial.get(&bond.atom1),
                index_to_serial.get(&bond.atom2),
            ) else {
                continue;
            };

            bonds_by_serial.entry(serial1).or_default().push(serial2);
            bonds_by_serial.entry(serial2).or_default().push(serial1);
        }

        // Write CONECT records sorted by serial
        let mut serials: Vec<i32> = bonds_by_serial.keys().copied().collect();
        serials.sort();

        for serial in serials {
            if let Some(bonded) = bonds_by_serial.get(&serial) {
                let serial_str =
                    hy36encode(5, serial).unwrap_or_else(|| format!("{:5}", serial % 100000));
                // Write in chunks of 4 bonded atoms per CONECT record
                for chunk in bonded.chunks(4) {
                    write!(self.writer, "CONECT{}", serial_str)?;
                    for &bonded_serial in chunk {
                        let bonded_str = hy36encode(5, bonded_serial)
                            .unwrap_or_else(|| format!("{:5}", bonded_serial % 100000));
                        write!(self.writer, "{}", bonded_str)?;
                    }
                    writeln!(self.writer)?;
                }
            }
        }

        Ok(())
    }

    /// Write END record
    fn write_end(&mut self) -> IoResult<()> {
        writeln!(self.writer, "END")?;
        Ok(())
    }

    /// Write a molecule
    fn write_molecule(&mut self, mol: &ObjectMolecule) -> IoResult<()> {
        // Write header records
        self.write_title(&mol.title)?;
        self.write_cryst1(mol)?;
        self.write_secondary_structure(mol)?;

        let state = self.state.unwrap_or(0);
        let num_states = mol.state_count();

        if num_states == 0 {
            return Err(IoError::empty_file());
        }

        // Write multi-model file if more than one state and no specific state requested
        let write_models = num_states > 1 && self.state.is_none();

        let states_to_write: Vec<usize> = if write_models {
            (0..num_states).collect()
        } else {
            vec![state.min(num_states.saturating_sub(1))]
        };

        // Build atom index → serial number mapping (used for CONECT records)
        let mut index_to_serial: HashMap<AtomIndex, i32> = HashMap::new();

        for (model_num, state_idx) in states_to_write.iter().enumerate() {
            if write_models {
                writeln!(self.writer, "MODEL     {:4}", model_num + 1)?;
            }

            let mut serial = 1;
            let mut last_chain: Option<String> = None;

            for (atom_idx, atom) in mol.atoms_indexed() {
                // Get coordinates for this state
                let coord = mol.get_coord(atom_idx, *state_idx).unwrap_or_default();

                // Write TER between chains
                if last_chain
                    .as_deref()
                    .is_some_and(|chain| atom.residue.chain != chain)
                {
                    // Get the previous atom's info for TER record
                    if let Some(prev_atom) = mol.get_atom(AtomIndex(atom_idx.0.saturating_sub(1))) {
                        self.write_ter(
                            serial,
                            &prev_atom.residue.resn,
                            &prev_atom.residue.chain,
                            prev_atom.residue.resv,
                        )?;
                        serial += 1;
                    }
                }
                last_chain = Some(atom.residue.chain.clone());

                // Record the mapping from atom index to written serial number
                index_to_serial.insert(atom_idx, serial);

                self.write_atom(serial, atom, coord.x, coord.y, coord.z)?;
                serial += 1;
            }

            // Write final TER
            if let Some(last_atom) = mol
                .atom_count()
                .checked_sub(1)
                .and_then(|i| mol.get_atom(AtomIndex(i as u32)))
            {
                self.write_ter(
                    serial,
                    &last_atom.residue.resn,
                    &last_atom.residue.chain,
                    last_atom.residue.resv,
                )?;
            }

            if write_models {
                writeln!(self.writer, "ENDMDL")?;
            }
        }

        // Write CONECT records (only for single model, only HETATM-involved bonds)
        if !write_models && mol.bond_count() > 0 {
            self.write_conect(mol, &index_to_serial)?;
        }

        self.write_end()?;

        Ok(())
    }
}

impl<W: Write> MoleculeWriter for PdbWriter<W> {
    fn write(&mut self, mol: &ObjectMolecule) -> IoResult<()> {
        self.write_molecule(mol)
    }

    fn flush(&mut self) -> IoResult<()> {
        self.writer.flush()?;
        Ok(())
    }
}

/// Format atom name according to PDB conventions
fn format_atom_name(name: &str, element: &str) -> String {
    let name = name.trim();
    let element_len = element.len();

    // PDB convention: 1-letter elements start in column 14 (index 1 of 4-char field)
    // 2-letter elements start in column 13 (index 0 of 4-char field)
    if name.len() >= 4 {
        name[..4].to_string()
    } else if element_len == 1 && !name.starts_with(char::is_numeric) {
        format!(" {:<3}", name)
    } else {
        format!("{:<4}", name)
    }
}

/// Format one internal chain identifier for a fixed-width PDB column.
fn pdb_chain_column(effective: &str) -> &str {
    let source = source_chain_id(effective);
    let source_len = source.chars().next().map_or(0, char::len_utf8);
    if source_len == 0 {
        " "
    } else {
        &source[..source_len]
    }
}

/// Format charge for PDB format (e.g., "2+" or "1-")
fn format_charge(charge: i8) -> String {
    match charge {
        0 => "  ".to_string(),
        c if c > 0 => format!("{}+", c.abs()),
        c => format!("{}-", c.abs()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdb::read_pdb_str;
    use lin_alg::f32::Vec3;
    use patinae_mol::{Atom, BondOrder, CoordSet, Element};

    fn create_test_molecule() -> ObjectMolecule {
        let mut mol = ObjectMolecule::new("test");

        let mut atom1 = Atom::new("N", Element::Nitrogen);
        atom1.set_residue("ALA", 1, "A");
        atom1.id = 1;
        mol.add_atom(atom1);

        let mut atom2 = Atom::new("CA", Element::Carbon);
        atom2.set_residue("ALA", 1, "A");
        atom2.id = 2;
        mol.add_atom(atom2);

        let coords = CoordSet::from_vec3(&[Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.5, 0.0, 0.0)]);
        mol.add_coord_set(coords);

        mol
    }

    fn molecule_with_chains(
        atoms: &[(&str, i32, SecondaryStructure, bool)],
        state_count: usize,
    ) -> ObjectMolecule {
        let mut mol = ObjectMolecule::new("chain_segments");

        for (chain, resv, ss_type, hetatm) in atoms {
            let mut atom = Atom::new("CA", Element::Carbon);
            atom.set_residue("ALA", *resv, *chain);
            atom.ss_type = *ss_type;
            atom.state.hetatm = *hetatm;
            mol.add_atom(atom);
        }

        for state in 0..state_count {
            let coords = atoms
                .iter()
                .enumerate()
                .map(|(index, _)| Vec3::new(index as f32, state as f32, 0.0))
                .collect::<Vec<_>>();
            mol.add_coord_set(CoordSet::from_vec3(&coords));
        }

        mol
    }

    fn write_to_string(mol: &ObjectMolecule) -> String {
        let mut output = Vec::new();
        PdbWriter::new(&mut output).write(mol).unwrap();
        String::from_utf8(output).unwrap()
    }

    fn chains(mol: &ObjectMolecule) -> Vec<&str> {
        mol.atoms_slice()
            .iter()
            .map(|atom| atom.residue.chain.as_str())
            .collect()
    }

    #[test]
    fn test_format_atom_name() {
        assert_eq!(format_atom_name("N", "N"), " N  ");
        assert_eq!(format_atom_name("CA", "C"), " CA ");
        assert_eq!(format_atom_name("FE", "Fe"), "FE  ");
    }

    #[test]
    fn test_format_charge() {
        assert_eq!(format_charge(0), "  ");
        assert_eq!(format_charge(1), "1+");
        assert_eq!(format_charge(2), "2+");
        assert_eq!(format_charge(-1), "1-");
        assert_eq!(format_charge(-2), "2-");
    }

    #[test]
    fn test_write_pdb() {
        let mol = create_test_molecule();
        let mut output = Vec::new();

        {
            let mut writer = PdbWriter::new(&mut output);
            writer.write(&mol).unwrap();
        }

        let pdb_string = String::from_utf8(output).unwrap();
        assert!(pdb_string.contains("ATOM"));
        assert!(pdb_string.contains("ALA"));
        assert!(pdb_string.contains("END"));
    }

    #[test]
    fn test_write_secondary_structure() {
        let mut mol = ObjectMolecule::new("ss_test");

        // Chain A: helix residues 1-3, then sheet residues 4-5
        for resv in 1..=3 {
            let mut atom = Atom::new("CA", Element::Carbon);
            atom.set_residue("ALA", resv, "A");
            atom.ss_type = SecondaryStructure::Helix;
            mol.add_atom(atom);
        }
        for resv in 4..=5 {
            let mut atom = Atom::new("CA", Element::Carbon);
            atom.set_residue("VAL", resv, "A");
            atom.ss_type = SecondaryStructure::Sheet;
            mol.add_atom(atom);
        }

        let coords = CoordSet::from_vec3(&[Vec3::new(0.0, 0.0, 0.0); 5]);
        mol.add_coord_set(coords);

        let mut output = Vec::new();
        {
            let mut writer = PdbWriter::new(&mut output);
            writer.write(&mol).unwrap();
        }

        let pdb_string = String::from_utf8(output).unwrap();
        assert!(
            pdb_string.contains("HELIX"),
            "Expected HELIX record in output:\n{}",
            pdb_string
        );
        assert!(
            pdb_string.contains("SHEET"),
            "Expected SHEET record in output:\n{}",
            pdb_string
        );

        // HELIX should cover residues 1-3
        let helix_line = pdb_string.lines().find(|l| l.starts_with("HELIX")).unwrap();
        assert!(helix_line.contains("ALA"));

        // SHEET should cover residues 4-5
        let sheet_line = pdb_string.lines().find(|l| l.starts_with("SHEET")).unwrap();
        assert!(sheet_line.contains("VAL"));
    }

    #[test]
    fn ter_round_trip_preserves_contextual_chain_segments_and_states() {
        let mol = molecule_with_chains(
            &[
                ("A", 1, SecondaryStructure::Loop, false),
                ("A2", 2, SecondaryStructure::Loop, true),
                ("A3", 3, SecondaryStructure::Loop, false),
            ],
            2,
        );

        let pdb = write_to_string(&mol);
        let reparsed = read_pdb_str(&pdb).unwrap();

        assert_eq!(chains(&reparsed), ["A", "A2", "A3"]);
        assert_eq!(reparsed.state_count(), 2);
        assert_eq!(
            pdb.lines().filter(|line| line.starts_with("TER")).count(),
            6
        );
        assert!(pdb
            .lines()
            .filter(|line| line.starts_with("ATOM") || line.starts_with("HETATM"))
            .all(|line| line.as_bytes().get(21) == Some(&b'A')));
    }

    #[test]
    fn ter_round_trip_preserves_blank_and_literal_underscore_segments() {
        let mol = molecule_with_chains(
            &[
                ("", 1, SecondaryStructure::Loop, false),
                ("_2", 2, SecondaryStructure::Loop, false),
                ("_3", 3, SecondaryStructure::Loop, false),
                ("_", 4, SecondaryStructure::Loop, false),
                ("__2", 5, SecondaryStructure::Loop, false),
                ("__3", 6, SecondaryStructure::Loop, false),
            ],
            1,
        );

        let pdb = write_to_string(&mol);
        let reparsed = read_pdb_str(&pdb).unwrap();

        assert_eq!(chains(&reparsed), ["", "_2", "_3", "_", "__2", "__3"]);
        let chain_columns = pdb
            .lines()
            .filter(|line| line.starts_with("ATOM"))
            .map(|line| line.as_bytes()[21])
            .collect::<Vec<_>>();
        assert_eq!(chain_columns, [b' ', b' ', b' ', b'_', b'_', b'_']);
    }

    #[test]
    fn ter_subset_exports_restart_contextual_ordinals() {
        let a2 = molecule_with_chains(&[("A2", 2, SecondaryStructure::Loop, false)], 1);
        let a2_a3 = molecule_with_chains(
            &[
                ("A2", 2, SecondaryStructure::Loop, false),
                ("A3", 3, SecondaryStructure::Loop, false),
            ],
            1,
        );

        let reparsed_a2 = read_pdb_str(&write_to_string(&a2)).unwrap();
        let reparsed_a2_a3 = read_pdb_str(&write_to_string(&a2_a3)).unwrap();

        assert_eq!(chains(&reparsed_a2), ["A"]);
        assert_eq!(chains(&reparsed_a2_a3), ["A", "A2"]);
    }

    #[test]
    fn ter_secondary_structure_applies_to_all_contextual_siblings() {
        let mol = molecule_with_chains(
            &[
                ("A", 1, SecondaryStructure::Helix, false),
                ("A", 2, SecondaryStructure::Sheet, false),
                ("A2", 1, SecondaryStructure::Helix, false),
                ("A2", 2, SecondaryStructure::Sheet, false),
                ("A3", 1, SecondaryStructure::Helix, false),
                ("A3", 2, SecondaryStructure::Sheet, false),
            ],
            1,
        );

        let pdb = write_to_string(&mol);
        let reparsed = read_pdb_str(&pdb).unwrap();
        let secondary = reparsed
            .atoms_slice()
            .iter()
            .map(|atom| atom.ss_type)
            .collect::<Vec<_>>();

        assert_eq!(chains(&reparsed), ["A", "A", "A2", "A2", "A3", "A3"]);
        assert_eq!(
            secondary,
            [
                SecondaryStructure::Helix,
                SecondaryStructure::Sheet,
                SecondaryStructure::Helix,
                SecondaryStructure::Sheet,
                SecondaryStructure::Helix,
                SecondaryStructure::Sheet,
            ]
        );
    }

    #[test]
    fn ter_contextual_chain_columns_use_the_source_identifier() {
        let mol = molecule_with_chains(
            &[
                ("A2", 1, SecondaryStructure::Helix, false),
                ("A2", 2, SecondaryStructure::Sheet, true),
            ],
            1,
        );

        let pdb = write_to_string(&mol);
        let atom = pdb.lines().find(|line| line.starts_with("ATOM")).unwrap();
        let hetatm = pdb.lines().find(|line| line.starts_with("HETATM")).unwrap();
        let ter = pdb.lines().find(|line| line.starts_with("TER")).unwrap();
        let helix = pdb.lines().find(|line| line.starts_with("HELIX")).unwrap();
        let sheet = pdb.lines().find(|line| line.starts_with("SHEET")).unwrap();

        assert_eq!(atom.as_bytes()[21], b'A');
        assert_eq!(hetatm.as_bytes()[21], b'A');
        assert_eq!(ter.as_bytes()[21], b'A');
        assert_eq!(helix.as_bytes()[19], b'A');
        assert_eq!(helix.as_bytes()[31], b'A');
        assert_eq!(sheet.as_bytes()[21], b'A');
        assert_eq!(sheet.as_bytes()[32], b'A');
    }

    #[test]
    fn test_conect_only_hetatm_bonds() {
        let mut mol = ObjectMolecule::new("conect_test");

        // Two regular ATOM atoms
        let mut atom1 = Atom::new("N", Element::Nitrogen);
        atom1.set_residue("ALA", 1, "A");
        let idx1 = mol.add_atom(atom1);

        let mut atom2 = Atom::new("CA", Element::Carbon);
        atom2.set_residue("ALA", 1, "A");
        let idx2 = mol.add_atom(atom2);

        // One HETATM atom
        let mut atom3 = Atom::new("O", Element::Oxygen);
        atom3.set_residue("HOH", 2, "A");
        atom3.state.hetatm = true;
        let idx3 = mol.add_atom(atom3);

        let coords = CoordSet::from_vec3(&[
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.5, 0.0, 0.0),
            Vec3::new(3.0, 0.0, 0.0),
        ]);
        mol.add_coord_set(coords);

        // Bond between two regular atoms (should NOT appear in CONECT)
        let _ = mol.add_bond_unchecked(idx1, idx2, BondOrder::Single);
        // Bond involving HETATM (should appear in CONECT)
        let _ = mol.add_bond_unchecked(idx2, idx3, BondOrder::Single);

        let mut output = Vec::new();
        {
            let mut writer = PdbWriter::new(&mut output);
            writer.write(&mol).unwrap();
        }

        let pdb_string = String::from_utf8(output).unwrap();
        let conect_lines: Vec<&str> = pdb_string
            .lines()
            .filter(|l| l.starts_with("CONECT"))
            .collect();

        // Should have CONECT records (for the HETATM bond)
        assert!(
            !conect_lines.is_empty(),
            "Expected CONECT records for HETATM bonds"
        );

        // CONECT should reference serials 2 and 3 (the atoms involved in the HETATM bond)
        // Serial 1 should NOT appear as a CONECT center (it's only bonded to another ATOM)
        let conect_text = conect_lines.join("\n");
        assert!(conect_text.contains("    2"), "Expected serial 2 in CONECT");
        assert!(conect_text.contains("    3"), "Expected serial 3 in CONECT");
    }

    #[test]
    fn test_conect_serial_numbers_match_atoms() {
        // Verify that CONECT records use the same serial numbers as ATOM records,
        // not the original atom.id values
        let mut mol = ObjectMolecule::new("serial_test");

        // Create atoms with non-sequential original IDs
        let mut atom1 = Atom::new("O", Element::Oxygen);
        atom1.set_residue("HOH", 1, "A");
        atom1.state.hetatm = true;
        atom1.id = 500; // Original ID that should NOT appear in output
        let idx1 = mol.add_atom(atom1);

        let mut atom2 = Atom::new("H", Element::Hydrogen);
        atom2.set_residue("HOH", 1, "A");
        atom2.state.hetatm = true;
        atom2.id = 501; // Original ID that should NOT appear in output
        let idx2 = mol.add_atom(atom2);

        let coords = CoordSet::from_vec3(&[Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.96, 0.0, 0.0)]);
        mol.add_coord_set(coords);

        let _ = mol.add_bond_unchecked(idx1, idx2, BondOrder::Single);

        let mut output = Vec::new();
        {
            let mut writer = PdbWriter::new(&mut output);
            writer.write(&mol).unwrap();
        }

        let pdb_string = String::from_utf8(output).unwrap();

        // ATOM records should have serials 1 and 2
        let conect_lines: Vec<&str> = pdb_string
            .lines()
            .filter(|l| l.starts_with("CONECT"))
            .collect();

        assert!(!conect_lines.is_empty());

        // Should NOT contain the original IDs (500, 501)
        let conect_text = conect_lines.join("\n");
        assert!(
            !conect_text.contains("  500"),
            "CONECT should not contain original atom ID 500"
        );
        assert!(
            !conect_text.contains("  501"),
            "CONECT should not contain original atom ID 501"
        );

        // Should contain the sequential serials (1, 2)
        assert!(
            conect_text.contains("    1"),
            "CONECT should contain serial 1"
        );
        assert!(
            conect_text.contains("    2"),
            "CONECT should contain serial 2"
        );
    }
}
