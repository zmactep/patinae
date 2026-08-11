//! PDB file parser
//!
//! Parses PDB format files using fixed-width column extraction.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};

use lin_alg::f32::Vec3;
use patinae_mol::{AtomIndex, BondOrder, ObjectMolecule, SecondaryStructure};

use crate::error::{IoError, IoResult};
use crate::logical_models::{build_molecules, ParsedAtom, ParsedModel};
use crate::pdb::hybrid36::hy36decode;
use crate::traits::MoleculeReader;

use super::chain_ids::{source_chain_id, ChainIdTracker};
use super::records::{AtomRecord, ConectRecord, Cryst1Record, HelixRecord, SheetRecord};

/// PDB file reader
pub struct PdbReader<R> {
    reader: BufReader<R>,
    line_number: usize,
    bond_tolerance: f32,
}

impl<R: Read> PdbReader<R> {
    /// Create a new PDB reader
    pub fn new(reader: R) -> Self {
        PdbReader {
            reader: BufReader::new(reader),
            line_number: 0,
            bond_tolerance: patinae_mol::DEFAULT_BOND_TOLERANCE,
        }
    }

    pub fn with_bond_tolerance(mut self, tolerance: f32) -> Self {
        self.bond_tolerance = tolerance;
        self
    }

    /// Read a single line from the file
    fn read_line(&mut self) -> IoResult<Option<String>> {
        let mut line = String::new();
        match self.reader.read_line(&mut line) {
            Ok(0) => Ok(None),
            Ok(_) => {
                self.line_number += 1;
                Ok(Some(line))
            }
            Err(e) => Err(IoError::io(e)),
        }
    }

    fn parse(&mut self) -> IoResult<ObjectMolecule> {
        self.parse_all()?
            .into_iter()
            .next()
            .ok_or(IoError::empty_file())
    }

    /// Parse the PDB file into one or more logical molecules.
    fn parse_all(&mut self) -> IoResult<Vec<ObjectMolecule>> {
        let mut models: Vec<ParsedModel> = Vec::new();
        let mut current_model: Option<ParsedModel> = None;
        let mut conects: Vec<ConectRecord> = Vec::new();
        let mut cryst1: Option<Cryst1Record> = None;
        let mut helices: Vec<HelixRecord> = Vec::new();
        let mut sheets: Vec<SheetRecord> = Vec::new();
        let mut title = String::new();
        let mut next_model_number = 1;
        let mut chain_ids = ChainIdTracker::default();

        while let Some(line) = self.read_line()? {
            let line = line.trim_end();

            // Skip empty lines
            if line.is_empty() {
                continue;
            }

            // Get record type (first 6 characters)
            let record_type = if line.len() >= 6 { &line[0..6] } else { line };

            match record_type {
                "ATOM  " | "HETATM" => {
                    if let Some(record) = parse_atom_record(line) {
                        let coord = Vec3::new(record.x, record.y, record.z);
                        let effective_chain = chain_ids.effective_chain(&record.chain);
                        let model = current_model
                            .get_or_insert_with(|| ParsedModel::new(next_model_number));
                        model
                            .atoms
                            .push(parsed_atom_from_record(&record, effective_chain));
                        model.coords.push(coord);
                    }
                }
                "CONECT" => {
                    conects.push(parse_conect_record(line));
                }
                "CRYST1" => {
                    cryst1 = Some(parse_cryst1_record(line));
                }
                "HELIX " => {
                    helices.push(parse_helix_record(line));
                }
                "SHEET " => {
                    sheets.push(parse_sheet_record(line));
                }
                "TITLE " if line.len() > 10 => {
                    if !title.is_empty() {
                        title.push(' ');
                    }
                    title.push_str(line[10..].trim());
                }
                "MODEL " => {
                    flush_model(&mut current_model, &mut models, &mut chain_ids);
                    let parsed_number = line
                        .get(6..)
                        .and_then(|s| s.trim().parse::<i32>().ok())
                        .unwrap_or(next_model_number);
                    next_model_number = parsed_number.saturating_add(1);
                    current_model = Some(ParsedModel::new(parsed_number));
                }
                "ENDMDL" => {
                    flush_model(&mut current_model, &mut models, &mut chain_ids);
                }
                "TER   " | "TER" => {
                    chain_ids.terminate_active();
                }
                "END   " | "END" => {
                    flush_model(&mut current_model, &mut models, &mut chain_ids);
                    break;
                }
                _ => {
                    // Ignore other record types for now
                }
            }
        }

        flush_model(&mut current_model, &mut models, &mut chain_ids);

        let mut molecules = build_molecules("", &title, models)?;
        for mol in &mut molecules {
            self.apply_annotations(mol, &conects, cryst1.as_ref(), &helices, &sheets);
        }
        Ok(molecules)
    }

    fn apply_annotations(
        &self,
        mol: &mut ObjectMolecule,
        conects: &[ConectRecord],
        cryst1: Option<&Cryst1Record>,
        helices: &[HelixRecord],
        sheets: &[SheetRecord],
    ) {
        let mut serial_to_index: HashMap<i32, AtomIndex> = HashMap::new();
        for (idx, atom) in mol.atoms_indexed() {
            if atom.id != 0 {
                serial_to_index.insert(atom.id, idx);
            }
        }

        // Add bonds from CONECT records
        for conect in conects {
            if let Some(&atom1_idx) = serial_to_index.get(&conect.atom) {
                for &bonded_serial in &conect.bonded {
                    if let Some(&atom2_idx) = serial_to_index.get(&bonded_serial) {
                        // Avoid duplicate bonds (CONECT records are symmetric)
                        if atom1_idx.0 < atom2_idx.0 {
                            let _ = mol.add_bond_unchecked(atom1_idx, atom2_idx, BondOrder::Single);
                        }
                    }
                }
            }
        }

        // Apply secondary structure annotations
        apply_secondary_structure(mol, helices, sheets);

        // Apply crystallographic symmetry
        if let Some(cryst) = cryst1 {
            use patinae_mol::Symmetry;
            mol.symmetry = Some(Symmetry::new(
                cryst.space_group.clone(),
                [cryst.a, cryst.b, cryst.c],
                [cryst.alpha, cryst.beta, cryst.gamma],
            ));
        }

        // Classify atoms (protein, nucleic, solvent, etc.)
        // This is needed for cartoon representation to work
        mol.classify_atoms();

        // Generate covalent bonds from distances
        // CONECT records only contain special bonds (disulfide, etc.), not standard covalent bonds
        mol.generate_bonds(self.bond_tolerance);

        // Assign bond orders for known protein residues using atom name templates
        mol.assign_known_residue_bond_orders();
    }
}

impl<R: Read> MoleculeReader for PdbReader<R> {
    fn read(&mut self) -> IoResult<ObjectMolecule> {
        self.parse()
    }

    fn read_all(&mut self) -> IoResult<Vec<ObjectMolecule>> {
        self.parse_all()
    }
}

fn flush_model(
    current_model: &mut Option<ParsedModel>,
    models: &mut Vec<ParsedModel>,
    chain_ids: &mut ChainIdTracker,
) {
    if let Some(model) = current_model.take() {
        if !model.atoms.is_empty() {
            models.push(model);
        }
    }
    chain_ids.reset();
}

fn parsed_atom_from_record(record: &AtomRecord, effective_chain: String) -> ParsedAtom {
    ParsedAtom {
        name: record.name.clone(),
        element: record.get_element(),
        chain: effective_chain,
        resn: record.resn.clone(),
        resv: record.resv,
        icode: record.icode,
        alt: record.alt_loc,
        hetatm: record.hetatm,
        serial: Some(record.serial),
        formal_charge: (!record.charge.trim().is_empty()).then(|| record.get_formal_charge()),
        occupancy: record.occupancy,
        b_factor: record.b_factor,
        segi: record.segi.clone(),
    }
}

/// Apply secondary structure annotations to atoms
fn apply_secondary_structure(
    mol: &mut ObjectMolecule,
    helices: &[HelixRecord],
    sheets: &[SheetRecord],
) {
    // This is a simplified implementation
    // A full implementation would iterate through atoms and check if they fall within
    // the residue ranges specified in HELIX/SHEET records

    // Collect helix atom indices first
    let helix_indices: Vec<AtomIndex> = helices
        .iter()
        .flat_map(|helix| {
            mol.atoms_indexed()
                .filter(|(_, atom)| {
                    source_chain_id(&atom.residue.chain) == helix.init_chain
                        && atom.residue.resv >= helix.init_seq
                        && atom.residue.resv <= helix.end_seq
                })
                .map(|(idx, _)| idx)
                .collect::<Vec<_>>()
        })
        .collect();

    // Apply helix secondary structure
    for idx in helix_indices {
        if let Some(atom_mut) = mol.get_atom_mut(idx) {
            atom_mut.ss_type = SecondaryStructure::Helix;
        }
    }

    // Collect sheet atom indices
    let sheet_indices: Vec<AtomIndex> = sheets
        .iter()
        .flat_map(|sheet| {
            mol.atoms_indexed()
                .filter(|(_, atom)| {
                    source_chain_id(&atom.residue.chain) == sheet.init_chain
                        && atom.residue.resv >= sheet.init_seq
                        && atom.residue.resv <= sheet.end_seq
                })
                .map(|(idx, _)| idx)
                .collect::<Vec<_>>()
        })
        .collect();

    // Apply sheet secondary structure
    for idx in sheet_indices {
        if let Some(atom_mut) = mol.get_atom_mut(idx) {
            atom_mut.ss_type = SecondaryStructure::Sheet;
        }
    }
}

// ============================================================================
// Nom parsers for PDB records
// ============================================================================

/// Parse an ATOM or HETATM record
fn parse_atom_record(input: &str) -> Option<AtomRecord> {
    // Ensure line is at least 54 characters (minimum for coordinates)
    if input.len() < 54 {
        return None;
    }

    let hetatm = input.starts_with("HETATM");

    // Fixed column positions (0-indexed):
    // 6-10: serial (5)
    // 11: space
    // 12-15: name (4)
    // 16: altLoc (1)
    // 17-19: resName (3)
    // 20: space
    // 21: chainID (1)
    // 22-25: resSeq (4)
    // 26: iCode (1)
    // 27-29: spaces
    // 30-37: x (8)
    // 38-45: y (8)
    // 46-53: z (8)
    // 54-59: occupancy (6)
    // 60-65: tempFactor (6)
    // 66-71: spaces
    // 72-75: segment (4)
    // 76-77: element (2)
    // 78-79: charge (2)

    let serial: i32 = input.get(6..11).and_then(|s| hy36decode(5, s)).unwrap_or(0);
    let name = input.get(12..16).unwrap_or("    ").trim().to_string();
    let alt_loc = input.chars().nth(16).unwrap_or(' ');
    let resn = input.get(17..20).unwrap_or("   ").trim().to_string();
    let chain = input.get(21..22).unwrap_or(" ").trim().to_string();
    let resv: i32 = input
        .get(22..26)
        .and_then(|s| hy36decode(4, s))
        .unwrap_or(0);
    let icode = input.chars().nth(26).unwrap_or(' ');

    let x: f32 = input
        .get(30..38)
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0.0);
    let y: f32 = input
        .get(38..46)
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0.0);
    let z: f32 = input
        .get(46..54)
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0.0);

    let occupancy: f32 = input
        .get(54..60)
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(1.0);
    let b_factor: f32 = input
        .get(60..66)
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0.0);

    let segi = input.get(72..76).unwrap_or("    ").trim().to_string();
    let element = input.get(76..78).unwrap_or("  ").trim().to_string();
    let charge = input.get(78..80).unwrap_or("  ").trim().to_string();

    let record = AtomRecord {
        hetatm,
        serial,
        name,
        alt_loc,
        resn,
        chain,
        resv,
        icode,
        x,
        y,
        z,
        occupancy,
        b_factor,
        segi,
        element,
        charge,
    };

    Some(record)
}

/// Parse a CONECT record
fn parse_conect_record(input: &str) -> ConectRecord {
    // CONECT record format:
    // 6-10: atom serial number
    // 11-15, 16-20, 21-25, 26-30: bonded atom serial numbers

    let atom: i32 = input.get(6..11).and_then(|s| hy36decode(5, s)).unwrap_or(0);

    let mut bonded = Vec::new();

    for start in [11, 16, 21, 26] {
        if let Some(field) = input.get(start..start + 5) {
            if let Some(serial) = hy36decode(5, field) {
                if serial > 0 {
                    bonded.push(serial);
                }
            }
        }
    }

    ConectRecord { atom, bonded }
}

/// Parse a CRYST1 record
fn parse_cryst1_record(input: &str) -> Cryst1Record {
    // CRYST1 record format:
    // 6-14: a (9)
    // 15-23: b (9)
    // 24-32: c (9)
    // 33-39: alpha (7)
    // 40-46: beta (7)
    // 47-53: gamma (7)
    // 55-65: space group (11)
    // 66-69: z (4)

    let a: f32 = input
        .get(6..15)
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(1.0);
    let b: f32 = input
        .get(15..24)
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(1.0);
    let c: f32 = input
        .get(24..33)
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(1.0);
    let alpha: f32 = input
        .get(33..40)
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(90.0);
    let beta: f32 = input
        .get(40..47)
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(90.0);
    let gamma: f32 = input
        .get(47..54)
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(90.0);
    let space_group = input.get(55..66).unwrap_or("P 1").trim().to_string();
    let z: i32 = input
        .get(66..70)
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(1);

    Cryst1Record {
        a,
        b,
        c,
        alpha,
        beta,
        gamma,
        space_group,
        z,
    }
}

/// Parse a HELIX record
fn parse_helix_record(input: &str) -> HelixRecord {
    // HELIX record format (simplified)
    let serial: i32 = input
        .get(7..10)
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    let helix_id = input.get(11..14).unwrap_or("").trim().to_string();
    let init_resn = input.get(15..18).unwrap_or("").trim().to_string();
    let init_chain = input.get(19..20).unwrap_or("").trim().to_string();
    let init_seq: i32 = input
        .get(21..25)
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    let init_icode = input.chars().nth(25).unwrap_or(' ');
    let end_resn = input.get(27..30).unwrap_or("").trim().to_string();
    let end_chain = input.get(31..32).unwrap_or("").trim().to_string();
    let end_seq: i32 = input
        .get(33..37)
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    let end_icode = input.chars().nth(37).unwrap_or(' ');
    let helix_class: i32 = input
        .get(38..40)
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(1);

    HelixRecord {
        serial,
        helix_id,
        init_resn,
        init_chain,
        init_seq,
        init_icode,
        end_resn,
        end_chain,
        end_seq,
        end_icode,
        helix_class,
    }
}

/// Parse a SHEET record
fn parse_sheet_record(input: &str) -> SheetRecord {
    // SHEET record format (simplified)
    let strand: i32 = input
        .get(7..10)
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    let sheet_id = input.get(11..14).unwrap_or("").trim().to_string();
    let num_strands: i32 = input
        .get(14..16)
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    let init_resn = input.get(17..20).unwrap_or("").trim().to_string();
    let init_chain = input.get(21..22).unwrap_or("").trim().to_string();
    let init_seq: i32 = input
        .get(22..26)
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    let init_icode = input.chars().nth(26).unwrap_or(' ');
    let end_resn = input.get(28..31).unwrap_or("").trim().to_string();
    let end_chain = input.get(32..33).unwrap_or("").trim().to_string();
    let end_seq: i32 = input
        .get(33..37)
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    let end_icode = input.chars().nth(37).unwrap_or(' ');
    let sense: i32 = input
        .get(38..40)
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);

    SheetRecord {
        strand,
        sheet_id,
        num_strands,
        init_resn,
        init_chain,
        init_seq,
        init_icode,
        end_resn,
        end_chain,
        end_seq,
        end_icode,
        sense,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_atom_record() {
        let line =
            "ATOM      1  N   ALA A   1       1.000   2.000   3.000  1.00 20.00           N  ";
        let record = parse_atom_record(line).unwrap();

        assert_eq!(record.serial, 1);
        assert_eq!(record.name.trim(), "N");
        assert_eq!(record.resn, "ALA");
        assert_eq!(record.chain, "A");
        assert_eq!(record.resv, 1);
        assert!((record.x - 1.0).abs() < 0.001);
        assert!((record.y - 2.0).abs() < 0.001);
        assert!((record.z - 3.0).abs() < 0.001);
        assert!((record.b_factor - 20.0).abs() < 0.001);
        assert_eq!(record.element, "N");
    }

    #[test]
    fn test_parse_atom_record_blank_chain_id() {
        let line =
            "ATOM      1  N   THR     4      18.962 -15.255   3.000  1.00 20.00           N  ";
        let record = parse_atom_record(line).unwrap();

        assert_eq!(record.resn, "THR");
        assert_eq!(record.chain, "");
        assert_eq!(record.resv, 4);
    }

    #[test]
    fn test_parse_hetatm_record() {
        let line =
            "HETATM    1  O   HOH A   1       1.000   2.000   3.000  1.00 20.00           O  ";
        let record = parse_atom_record(line).unwrap();

        assert!(record.hetatm);
        assert_eq!(record.resn, "HOH");
    }

    #[test]
    fn test_parse_conect_record() {
        let line = "CONECT    1    2    3    4    5";
        let record = parse_conect_record(line);

        assert_eq!(record.atom, 1);
        assert_eq!(record.bonded, vec![2, 3, 4, 5]);
    }

    #[test]
    fn test_read_simple_pdb() {
        let pdb = r#"ATOM      1  N   ALA A   1       0.000   0.000   0.000  1.00 20.00           N
ATOM      2  CA  ALA A   1       1.458   0.000   0.000  1.00 20.00           C
ATOM      3  C   ALA A   1       2.009   1.420   0.000  1.00 20.00           C
ATOM      4  O   ALA A   1       1.251   2.390   0.000  1.00 20.00           O
END
"#;

        let mut reader = PdbReader::new(pdb.as_bytes());
        let mol = reader.read().unwrap();

        assert_eq!(mol.atom_count(), 4);
        assert_eq!(mol.state_count(), 1);
    }

    #[test]
    fn test_read_pdb_preserves_blank_chain_id() {
        let pdb = r#"ATOM      1  N   THR     4      18.962 -15.255   3.000  1.00 20.00           N
ATOM      2  CA  THR     4      18.185 -14.856   3.000  1.00 20.00           C
END
"#;

        let mut reader = PdbReader::new(pdb.as_bytes());
        let mol = reader.read().unwrap();

        assert_eq!(mol.atom_count(), 2);
        assert_eq!(mol.atoms_slice()[0].residue.chain, "");
        assert_eq!(mol.atoms_slice()[1].residue.chain, "");
    }

    #[test]
    fn ter_separates_repeated_atom_and_hetatm_chains_once() {
        let pdb = r#"TER
ATOM      1  N   ALA A   1       0.000   0.000   0.000  1.00 20.00           N
TER
TER
HETATM    2  O   HOH A   2       1.000   0.000   0.000  1.00 20.00           O
TER
ATOM      3  N   GLY A   3       2.000   0.000   0.000  1.00 20.00           N
END
"#;

        let mut reader = PdbReader::new(pdb.as_bytes());
        let mol = reader.read().unwrap();
        let chains: Vec<_> = mol
            .atoms_slice()
            .iter()
            .map(|atom| atom.residue.chain.as_str())
            .collect();

        assert_eq!(chains, ["A", "A2", "A3"]);
        assert!(!mol.atoms_slice()[0].state.hetatm);
        assert!(mol.atoms_slice()[1].state.hetatm);
    }

    #[test]
    fn ter_tracks_explicit_and_implicit_source_reopening() {
        let pdb = r#"ATOM      1  N   ALA A   1       0.000   0.000   0.000  1.00 20.00           N
TER
ATOM      2  N   ALA B   1       1.000   0.000   0.000  1.00 20.00           N
ATOM      3  N   GLY A   2       2.000   0.000   0.000  1.00 20.00           N
ATOM      4  N   GLY B   2       3.000   0.000   0.000  1.00 20.00           N
END
"#;

        let mut reader = PdbReader::new(pdb.as_bytes());
        let mol = reader.read().unwrap();
        let chains: Vec<_> = mol
            .atoms_slice()
            .iter()
            .map(|atom| atom.residue.chain.as_str())
            .collect();

        assert_eq!(chains, ["A", "B", "A2", "B2"]);
    }

    #[test]
    fn ter_ignores_malformed_atom_when_advancing_segments() {
        let pdb = r#"ATOM      1  N   ALA A   1       0.000   0.000   0.000  1.00 20.00           N
TER
ATOM                A
TER
ATOM      2  N   GLY A   2       1.000   0.000   0.000  1.00 20.00           N
END
"#;

        let mut reader = PdbReader::new(pdb.as_bytes());
        let mol = reader.read().unwrap();
        let chains: Vec<_> = mol
            .atoms_slice()
            .iter()
            .map(|atom| atom.residue.chain.as_str())
            .collect();

        assert_eq!(chains, ["A", "A2"]);
    }

    #[test]
    fn ter_keeps_blank_and_underscore_segment_names_disjoint() {
        let pdb = r#"ATOM      1  N   ALA     1       0.000   0.000   0.000  1.00 20.00           N
TER
ATOM      2  N   GLY     2       1.000   0.000   0.000  1.00 20.00           N
TER
ATOM      3  N   SER     3       2.000   0.000   0.000  1.00 20.00           N
ATOM      4  N   ALA _   1       3.000   0.000   0.000  1.00 20.00           N
TER
ATOM      5  N   GLY _   2       4.000   0.000   0.000  1.00 20.00           N
TER
ATOM      6  N   SER _   3       5.000   0.000   0.000  1.00 20.00           N
END
"#;

        let mut reader = PdbReader::new(pdb.as_bytes());
        let mol = reader.read().unwrap();
        let chains: Vec<_> = mol
            .atoms_slice()
            .iter()
            .map(|atom| atom.residue.chain.as_str())
            .collect();

        assert_eq!(chains, ["", "_2", "_3", "_", "__2", "__3"]);
    }

    #[test]
    fn ter_state_resets_for_matching_models() {
        let pdb = r#"MODEL        1
ATOM      1  N   ALA A   1       0.000   0.000   0.000  1.00 20.00           N
TER
ATOM      2  N   GLY A   2       1.000   0.000   0.000  1.00 20.00           N
ENDMDL
MODEL        2
ATOM      1  N   ALA A   1       0.500   0.000   0.000  1.00 20.00           N
TER
ATOM      2  N   GLY A   2       1.500   0.000   0.000  1.00 20.00           N
ENDMDL
"#;

        let mut reader = PdbReader::new(pdb.as_bytes());
        let molecules = reader.read_all().unwrap();

        assert_eq!(molecules.len(), 1);
        assert_eq!(molecules[0].state_count(), 2);
        assert_eq!(molecules[0].atoms_slice()[0].residue.chain, "A");
        assert_eq!(molecules[0].atoms_slice()[1].residue.chain, "A2");
    }

    #[test]
    fn ter_layout_distinguishes_model_topologies() {
        let pdb = r#"MODEL        1
ATOM      1  N   ALA A   1       0.000   0.000   0.000  1.00 20.00           N
TER
ATOM      2  N   GLY A   2       1.000   0.000   0.000  1.00 20.00           N
ENDMDL
MODEL        2
ATOM      1  N   ALA A   1       0.500   0.000   0.000  1.00 20.00           N
ATOM      2  N   GLY A   2       1.500   0.000   0.000  1.00 20.00           N
ENDMDL
"#;

        let mut reader = PdbReader::new(pdb.as_bytes());
        let molecules = reader.read_all().unwrap();

        assert_eq!(molecules.len(), 2);
        assert_eq!(molecules[0].atoms_slice()[1].residue.chain, "A2");
        assert_eq!(molecules[1].atoms_slice()[1].residue.chain, "A");
    }

    /// Regression test: multi-model PDB files were previously returning
    /// EmptyFile because MODEL 1 atoms were silently dropped.
    /// The condition `current_model > 0` routed model-1 atoms to
    /// `models.last_mut()` which was None (nothing pushed yet),
    /// leaving `atom_records` empty → EmptyFile error.
    #[test]
    fn test_read_multimodel_pdb() {
        let pdb = r#"MODEL        1
ATOM      1  N   ALA A   1       0.000   0.000   0.000  1.00 20.00           N
ATOM      2  CA  ALA A   1       1.458   0.000   0.000  1.00 20.00           C
ENDMDL
MODEL        2
ATOM      1  N   ALA A   1       0.100   0.100   0.100  1.00 20.00           N
ATOM      2  CA  ALA A   1       1.558   0.100   0.100  1.00 20.00           C
ENDMDL
MODEL        3
ATOM      1  N   ALA A   1       0.200   0.200   0.200  1.00 20.00           N
ATOM      2  CA  ALA A   1       1.658   0.200   0.200  1.00 20.00           C
ENDMDL
"#;

        let mut reader = PdbReader::new(pdb.as_bytes());
        let mol = reader.read().unwrap();

        // All 3 models must be loaded; previously model 1 was silently dropped → EmptyFile
        assert_eq!(mol.atom_count(), 2);
        assert_eq!(mol.state_count(), 3);

        // Model 1 coords (state 0)
        let coord0 = mol
            .get_coord(patinae_mol::AtomIndex::from(0usize), 0)
            .unwrap();
        assert!((coord0.x - 0.000).abs() < 0.001);

        // Model 3 coords (state 2) — different position
        let coord2 = mol
            .get_coord(patinae_mol::AtomIndex::from(0usize), 2)
            .unwrap();
        assert!((coord2.x - 0.200).abs() < 0.001);
    }

    #[test]
    fn read_all_groups_same_topology_models_as_states() {
        let pdb = r#"MODEL        1
ATOM      1  N   ALA A   1       0.000   0.000   0.000  1.00 20.00           N
ATOM      2  CA  ALA A   1       1.000   0.000   0.000  1.00 20.00           C
ENDMDL
MODEL        2
ATOM      1  N   ALA A   1       0.500   0.000   0.000  1.00 20.00           N
ATOM      2  CA  ALA A   1       1.500   0.000   0.000  1.00 20.00           C
ENDMDL
"#;

        let mut reader = PdbReader::new(pdb.as_bytes());
        let molecules = reader.read_all().unwrap();

        assert_eq!(molecules.len(), 1);
        assert_eq!(molecules[0].atom_count(), 2);
        assert_eq!(molecules[0].state_count(), 2);
    }

    #[test]
    fn read_all_splits_models_with_different_atom_counts() {
        let pdb = r#"MODEL        1
ATOM      1  N   ALA A   1       0.000   0.000   0.000  1.00 20.00           N
ATOM      2  CA  ALA A   1       1.000   0.000   0.000  1.00 20.00           C
ENDMDL
MODEL        2
ATOM      1  N   ALA A   1       0.500   0.000   0.000  1.00 20.00           N
ENDMDL
"#;

        let mut reader = PdbReader::new(pdb.as_bytes());
        let molecules = reader.read_all().unwrap();

        assert_eq!(molecules.len(), 2);
        assert_eq!(molecules[0].atom_count(), 2);
        assert_eq!(molecules[1].atom_count(), 1);
    }

    #[test]
    fn read_all_splits_models_with_changed_atom_identity() {
        let pdb = r#"MODEL        1
ATOM      1  N   ALA A   1       0.000   0.000   0.000  1.00 20.00           N
ATOM      2  CA  ALA A   1       1.000   0.000   0.000  1.00 20.00           C
ENDMDL
MODEL        2
ATOM      1  N   GLY A   1       0.500   0.000   0.000  1.00 20.00           N
ATOM      2  CA  GLY A   1       1.500   0.000   0.000  1.00 20.00           C
ENDMDL
"#;

        let mut reader = PdbReader::new(pdb.as_bytes());
        let molecules = reader.read_all().unwrap();

        assert_eq!(molecules.len(), 2);
        assert_eq!(molecules[0].atoms_slice()[0].residue.resn, "ALA");
        assert_eq!(molecules[1].atoms_slice()[0].residue.resn, "GLY");
    }

    #[test]
    fn read_pdb_str_returns_first_logical_molecule() {
        let pdb = r#"MODEL        1
ATOM      1  N   ALA A   1       0.000   0.000   0.000  1.00 20.00           N
ATOM      2  CA  ALA A   1       1.000   0.000   0.000  1.00 20.00           C
ENDMDL
MODEL        2
ATOM      1  N   ALA B   1       0.500   0.000   0.000  1.00 20.00           N
ENDMDL
"#;

        let molecule = crate::pdb::read_pdb_str(pdb).unwrap();

        assert_eq!(molecule.atom_count(), 2);
        assert_eq!(molecule.state_count(), 1);
        assert_eq!(molecule.atoms_slice()[0].residue.chain, "A");
    }
}
