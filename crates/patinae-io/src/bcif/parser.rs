//! BinaryCIF file parser
//!
//! Parses BinaryCIF format files into ObjectMolecule.

use std::collections::{BTreeMap, HashMap};
use std::io::Read;

use lin_alg::f32::Vec3;
use patinae_mol::{Element, ObjectMolecule, SecondaryStructure};

use crate::assembly::{AssemblyRow, AssemblyRows, CATEGORIES};
use crate::cif::common::{apply_secondary_structure, SecondaryStructureRange, SsCategory};
use crate::error::{IoError, IoResult};
use crate::logical_models::{build_molecules, ParsedAtom, ParsedModel};
use crate::traits::MoleculeReader;

use super::decode::{decode_column, decode_mask, ColumnMask, DecodedColumn};
use super::types::{BcifCategory, BcifDataBlock, BcifFile};

/// BinaryCIF file reader
pub struct BcifReader<R> {
    reader: R,
    bond_tolerance: f32,
}

impl<R: Read> BcifReader<R> {
    pub fn new(reader: R) -> Self {
        BcifReader {
            reader,
            bond_tolerance: patinae_mol::DEFAULT_BOND_TOLERANCE,
        }
    }

    pub fn with_bond_tolerance(mut self, tolerance: f32) -> Self {
        self.bond_tolerance = tolerance;
        self
    }

    fn parse(&mut self) -> IoResult<ObjectMolecule> {
        self.parse_all()?
            .into_iter()
            .next()
            .ok_or(IoError::empty_file())
    }

    fn parse_all(&mut self) -> IoResult<Vec<ObjectMolecule>> {
        let mut bytes = Vec::new();
        self.reader.read_to_end(&mut bytes).map_err(IoError::io)?;

        let file: BcifFile = rmp_serde::from_slice(&bytes)
            .map_err(|e| IoError::parse_msg(format!("MessagePack error: {}", e)))?;

        parse_bcif_file(file, self.bond_tolerance)
    }
}

impl<R: Read> MoleculeReader for BcifReader<R> {
    fn read(&mut self) -> IoResult<ObjectMolecule> {
        self.parse()
    }

    fn read_all(&mut self) -> IoResult<Vec<ObjectMolecule>> {
        self.parse_all()
    }
}

/// Decoded columns for a category, keyed by column name
struct CategoryColumns {
    columns: HashMap<String, (DecodedColumn, Option<ColumnMask>)>,
}

impl CategoryColumns {
    fn decode(category: &BcifCategory) -> IoResult<Self> {
        let mut columns = HashMap::new();
        for col in &category.columns {
            let decoded = decode_column(&col.data)?;
            let mask = decode_mask(&col.mask)?;
            columns.insert(col.name.clone(), (decoded, mask));
        }
        Ok(CategoryColumns { columns })
    }

    /// Decode only specific columns by name
    fn decode_selected(category: &BcifCategory, names: &[&str]) -> IoResult<Self> {
        let mut columns = HashMap::new();
        for col in &category.columns {
            if names.contains(&col.name.as_str()) {
                let decoded = decode_column(&col.data)?;
                let mask = decode_mask(&col.mask)?;
                columns.insert(col.name.clone(), (decoded, mask));
            }
        }
        Ok(CategoryColumns { columns })
    }

    fn str_at(&self, name: &str, i: usize) -> Option<&str> {
        let (col, mask) = self.columns.get(name)?;
        if let Some(m) = mask {
            if !m.is_present(i) {
                return None;
            }
        }
        col.str_at(i).filter(|s| !s.is_empty())
    }

    fn int_at(&self, name: &str, i: usize) -> Option<i32> {
        let (col, mask) = self.columns.get(name)?;
        if let Some(m) = mask {
            if !m.is_present(i) {
                return None;
            }
        }
        col.int_at(i)
    }

    fn float_at(&self, name: &str, i: usize) -> Option<f32> {
        let (col, mask) = self.columns.get(name)?;
        if let Some(m) = mask {
            if !m.is_present(i) {
                return None;
            }
        }
        col.float_at(i)
    }
}

fn parse_bcif_file(file: BcifFile, bond_tolerance: f32) -> IoResult<Vec<ObjectMolecule>> {
    let mut molecules = Vec::new();
    for block in file.data_blocks {
        molecules.extend(parse_data_block(block, bond_tolerance)?);
    }

    if molecules.is_empty() {
        Err(IoError::empty_file())
    } else {
        Ok(molecules)
    }
}

#[derive(Clone, Copy)]
struct CellInfo {
    a: f32,
    b: f32,
    c: f32,
    alpha: f32,
    beta: f32,
    gamma: f32,
}

impl Default for CellInfo {
    fn default() -> Self {
        Self {
            a: 1.0,
            b: 1.0,
            c: 1.0,
            alpha: 90.0,
            beta: 90.0,
            gamma: 90.0,
        }
    }
}

impl CellInfo {
    fn has_non_default_lengths(&self) -> bool {
        (self.a - 1.0).abs() > 0.001 || (self.b - 1.0).abs() > 0.001 || (self.c - 1.0).abs() > 0.001
    }

    fn apply_to(self, mol: &mut ObjectMolecule, space_group: Option<&str>) {
        if self.has_non_default_lengths() {
            use patinae_mol::Symmetry;
            mol.symmetry = Some(Symmetry::new(
                space_group.unwrap_or("P 1"),
                [self.a, self.b, self.c],
                [self.alpha, self.beta, self.gamma],
            ));
        }
    }
}

fn parse_data_block(block: BcifDataBlock, bond_tolerance: f32) -> IoResult<Vec<ObjectMolecule>> {
    let mut models: BTreeMap<i32, ParsedModel> = BTreeMap::new();
    let mut ss_ranges: Vec<SecondaryStructureRange> = Vec::new();
    let mut space_group: Option<String> = None;
    let mut cell = CellInfo::default();
    let mut title = String::new();
    let mut assemblies = AssemblyRows::default();

    for category in &block.categories {
        if CATEGORIES.contains(&category.name.as_str()) {
            parse_assembly_category(category, &mut assemblies)?;
        }
        match category.name.as_str() {
            "_atom_site" => parse_atom_site(category, &mut models)?,
            "_cell" => parse_cell(category, &mut cell)?,
            "_symmetry" => parse_symmetry_category(category, &mut space_group)?,
            "_struct_conf" => parse_ss(category, SsCategory::StructConf, &mut ss_ranges)?,
            "_struct_sheet_range" => {
                parse_ss(category, SsCategory::StructSheetRange, &mut ss_ranges)?;
            }
            "_entry" => parse_entry(category, &mut title)?,
            _ => {}
        }
    }

    if models.is_empty() {
        return Ok(Vec::new());
    }

    let definitions = assemblies.resolve()?;
    if definitions.is_empty() {
        for model in models.values_mut() {
            model.source_chains.clear();
        }
    }
    let models = models.into_values().collect();
    let mut molecules = build_molecules(&block.header, &title, models)?;

    for mol in &mut molecules {
        mol.assembly.definitions = definitions.clone();
        cell.apply_to(mol, space_group.as_deref());
        apply_secondary_structure(mol, &ss_ranges);
        mol.classify_atoms();
        mol.generate_bonds(bond_tolerance);
        mol.assign_known_residue_bond_orders();
    }

    Ok(molecules)
}

fn parse_atom_site(
    category: &BcifCategory,
    models: &mut BTreeMap<i32, ParsedModel>,
) -> IoResult<()> {
    let row_count = category.row_count as usize;
    let cols = CategoryColumns::decode(category)?;

    for i in 0..row_count {
        // Element
        let element_str = cols
            .str_at("type_symbol", i)
            .or_else(|| cols.str_at("label_atom_id", i))
            .unwrap_or("X");
        let element = Element::from_symbol(element_str).unwrap_or(Element::Unknown);

        // Atom name
        let atom_name = cols
            .str_at("auth_atom_id", i)
            .or_else(|| cols.str_at("label_atom_id", i))
            .unwrap_or("X");

        // Residue info
        let resn = cols
            .str_at("auth_comp_id", i)
            .or_else(|| cols.str_at("label_comp_id", i))
            .unwrap_or("");

        let resv = cols
            .int_at("auth_seq_id", i)
            .or_else(|| cols.int_at("label_seq_id", i))
            .unwrap_or(0);

        let chain = cols
            .str_at("auth_asym_id", i)
            .or_else(|| cols.str_at("label_asym_id", i))
            .unwrap_or("");

        let inscode = cols
            .str_at("pdbx_PDB_ins_code", i)
            .and_then(|s| s.chars().next())
            .unwrap_or(' ');

        // Strip hyphens from chain IDs so assembly chains like "A-2"
        // become "A2", which is valid in the selection language.
        let chain_id = chain.replace('-', "");

        // Alt loc
        let alt = cols
            .str_at("label_alt_id", i)
            .and_then(|s| s.chars().next())
            .unwrap_or(' ');

        // B-factor
        let b_factor = cols.float_at("B_iso_or_equiv", i).unwrap_or(0.0);

        // Occupancy
        let occupancy = cols.float_at("occupancy", i).unwrap_or(1.0);

        // HETATM flag
        let hetatm = cols
            .str_at("group_PDB", i)
            .map(|s| s == "HETATM")
            .unwrap_or(false);

        // Formal charge
        let formal_charge = cols
            .int_at("pdbx_formal_charge", i)
            .map(|charge| charge as i8);

        // Serial number
        let serial = cols.int_at("id", i);

        // Coordinates
        let x = cols.float_at("Cartn_x", i).unwrap_or(0.0);
        let y = cols.float_at("Cartn_y", i).unwrap_or(0.0);
        let z = cols.float_at("Cartn_z", i).unwrap_or(0.0);

        // Model number
        let model_num = cols.int_at("pdbx_PDB_model_num", i).unwrap_or(1);

        let model = models
            .entry(model_num)
            .or_insert_with(|| ParsedModel::new(model_num));
        model
            .source_chains
            .push(cols.str_at("label_asym_id", i).unwrap_or("").to_owned());
        model.atoms.push(ParsedAtom {
            name: atom_name.to_string(),
            element,
            chain: chain_id,
            resn: resn.to_string(),
            resv,
            icode: inscode,
            alt,
            hetatm,
            serial,
            formal_charge,
            occupancy,
            b_factor,
            segi: String::new(),
        });
        model.coords.push(Vec3::new(x, y, z));
    }

    Ok(())
}

fn parse_assembly_category(category: &BcifCategory, assemblies: &mut AssemblyRows) -> IoResult<()> {
    let columns = CategoryColumns::decode(category)?;
    for i in 0..category.row_count as usize {
        let mut row = AssemblyRow::new();
        for name in columns.columns.keys() {
            let value = columns
                .str_at(name, i)
                .map(str::to_owned)
                .or_else(|| columns.int_at(name, i).map(|value| value.to_string()))
                .or_else(|| columns.float_at(name, i).map(|value| value.to_string()));
            if let Some(value) = value {
                row.insert(name.clone(), value);
            }
        }
        assemblies.push(&category.name, row);
    }
    Ok(())
}

fn parse_cell(category: &BcifCategory, cell: &mut CellInfo) -> IoResult<()> {
    let cols = CategoryColumns::decode_selected(
        category,
        &[
            "length_a",
            "length_b",
            "length_c",
            "angle_alpha",
            "angle_beta",
            "angle_gamma",
        ],
    )?;

    cell.a = cols.float_at("length_a", 0).unwrap_or(1.0);
    cell.b = cols.float_at("length_b", 0).unwrap_or(1.0);
    cell.c = cols.float_at("length_c", 0).unwrap_or(1.0);
    cell.alpha = cols.float_at("angle_alpha", 0).unwrap_or(90.0);
    cell.beta = cols.float_at("angle_beta", 0).unwrap_or(90.0);
    cell.gamma = cols.float_at("angle_gamma", 0).unwrap_or(90.0);

    Ok(())
}

fn parse_ss(
    category: &BcifCategory,
    ss_category: SsCategory,
    ss_ranges: &mut Vec<SecondaryStructureRange>,
) -> IoResult<()> {
    let cols = CategoryColumns::decode(category)?;
    let row_count = category.row_count as usize;

    // Column names depend on category
    let (conf_type_col, beg_chain_col, beg_seq_col, _end_chain_col, end_seq_col) = match ss_category
    {
        SsCategory::StructConf => (
            Some("conf_type_id"),
            "beg_auth_asym_id",
            "beg_auth_seq_id",
            "end_auth_asym_id",
            "end_auth_seq_id",
        ),
        SsCategory::StructSheetRange => (
            None,
            "beg_auth_asym_id",
            "beg_auth_seq_id",
            "end_auth_asym_id",
            "end_auth_seq_id",
        ),
    };

    for i in 0..row_count {
        let ss_type = match ss_category {
            SsCategory::StructConf => {
                let conf_type = cols.str_at(conf_type_col.unwrap(), i).unwrap_or("");
                if !conf_type.starts_with("HELX") {
                    continue;
                }
                if conf_type.contains("310") || conf_type == "HELX_LH_3T_P" {
                    SecondaryStructure::Helix310
                } else if conf_type.contains("PI") || conf_type == "HELX_LH_PI_P" {
                    SecondaryStructure::HelixPi
                } else {
                    SecondaryStructure::Helix
                }
            }
            SsCategory::StructSheetRange => SecondaryStructure::Sheet,
        };

        let chain = cols
            .str_at(beg_chain_col, i)
            .or_else(|| cols.str_at("beg_label_asym_id", i))
            .filter(|s| !s.is_empty());

        let chain = match chain {
            Some(c) => c.to_string(),
            None => continue,
        };

        let auth_chain = cols.str_at(beg_chain_col, i).map(|s| s.to_string());

        let start_seq = cols
            .int_at(beg_seq_col, i)
            .or_else(|| cols.int_at("beg_label_seq_id", i))
            .filter(|&n| n > 0);
        let end_seq = cols
            .int_at(end_seq_col, i)
            .or_else(|| cols.int_at("end_label_seq_id", i))
            .filter(|&n| n > 0);

        if let (Some(start), Some(end)) = (start_seq, end_seq) {
            ss_ranges.push(SecondaryStructureRange {
                ss_type,
                chain,
                auth_chain,
                start_seq: start,
                end_seq: end,
            });
        }
    }

    Ok(())
}

fn parse_symmetry_category(
    category: &BcifCategory,
    space_group: &mut Option<String>,
) -> IoResult<()> {
    let cols = CategoryColumns::decode_selected(
        category,
        &["space_group_name_H-M", "space_group_name_Hall"],
    )?;

    if let Some(sg) = cols.str_at("space_group_name_H-M", 0) {
        let sg = sg.trim().to_string();
        if !sg.is_empty() {
            *space_group = Some(sg);
        }
    } else if let Some(sg) = cols.str_at("space_group_name_Hall", 0) {
        let sg = sg.trim().to_string();
        if !sg.is_empty() {
            *space_group = Some(sg);
        }
    }

    Ok(())
}

fn parse_entry(category: &BcifCategory, title: &mut String) -> IoResult<()> {
    let cols = CategoryColumns::decode_selected(category, &["id"])?;
    if let Some(id) = cols.str_at("id", 0) {
        if title.is_empty() {
            *title = id.to_string();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bcif::test_support::{atom_row, atom_site_block, bcif_file, encode_bcif_file};
    use std::collections::HashMap;

    /// Verify that CategoryColumns::str_at filters out empty strings,
    /// treating them as None (missing) — matching text CIF behavior
    /// where `.` and `?` tokens map to None.
    #[test]
    fn test_str_at_filters_empty_strings() {
        let mut columns = HashMap::new();
        columns.insert(
            "type_symbol".to_string(),
            (
                DecodedColumn::String(vec![
                    "".to_string(),  // index 0: empty (placeholder)
                    "O".to_string(), // index 1: oxygen
                    "H".to_string(), // index 2: hydrogen
                ]),
                None, // no mask
            ),
        );
        let cols = CategoryColumns { columns };

        // Empty string at index 0 should be filtered to None
        assert_eq!(cols.str_at("type_symbol", 0), None);
        // Valid strings should pass through
        assert_eq!(cols.str_at("type_symbol", 1), Some("O"));
        assert_eq!(cols.str_at("type_symbol", 2), Some("H"));
        // Out of bounds should be None
        assert_eq!(cols.str_at("type_symbol", 99), None);
        // Nonexistent column should be None
        assert_eq!(cols.str_at("nonexistent", 0), None);
    }

    /// Verify that masked entries return None regardless of string value
    #[test]
    fn test_str_at_respects_mask() {
        let mut columns = HashMap::new();
        columns.insert(
            "label_seq_id".to_string(),
            (
                DecodedColumn::String(vec![
                    "1".to_string(),
                    "2".to_string(),
                    "".to_string(), // masked entry
                ]),
                Some(ColumnMask(vec![0, 0, 1])), // 0=present, 1=missing
            ),
        );
        let cols = CategoryColumns { columns };

        assert_eq!(cols.str_at("label_seq_id", 0), Some("1"));
        assert_eq!(cols.str_at("label_seq_id", 1), Some("2"));
        // Masked entry returns None (mask takes priority)
        assert_eq!(cols.str_at("label_seq_id", 2), None);
    }

    #[test]
    fn parse_file_reads_multiple_data_blocks() {
        let first = atom_site_block("FIRST", &[atom_row(1, "C", "LIG", "A", 0.0, 1)]);
        let second = atom_site_block("SECOND", &[atom_row(1, "N", "LIG", "B", 1.0, 1)]);

        let molecules = parse_bcif_file(bcif_file(vec![first, second]), 0.1).unwrap();

        assert_eq!(molecules.len(), 2);
        assert_eq!(molecules[0].name, "FIRST");
        assert_eq!(molecules[1].name, "SECOND");
    }

    #[test]
    fn parse_block_groups_same_topology_models_as_states() {
        let rows = [
            atom_row(1, "N", "ALA", "A", 0.0, 1),
            atom_row(2, "CA", "ALA", "A", 1.0, 1),
            atom_row(1, "N", "ALA", "A", 0.5, 2),
            atom_row(2, "CA", "ALA", "A", 1.5, 2),
        ];
        let block = atom_site_block("TEST", &rows);

        let molecules = parse_data_block(block, 0.1).unwrap();

        assert_eq!(molecules.len(), 1);
        assert_eq!(molecules[0].atom_count(), 2);
        assert_eq!(molecules[0].state_count(), 2);
    }

    #[test]
    fn parse_block_splits_incompatible_models() {
        let rows = [
            atom_row(1, "N", "ALA", "A", 0.0, 1),
            atom_row(2, "CA", "ALA", "A", 1.0, 1),
            atom_row(1, "N", "GLY", "A", 0.5, 2),
            atom_row(2, "CA", "GLY", "A", 1.5, 2),
        ];
        let block = atom_site_block("TEST", &rows);

        let molecules = parse_data_block(block, 0.1).unwrap();

        assert_eq!(molecules.len(), 2);
        assert_eq!(molecules[0].name, "TEST_model_1");
        assert_eq!(molecules[1].name, "TEST_model_2");
        assert_eq!(molecules[0].atoms_slice()[0].residue.resn, "ALA");
        assert_eq!(molecules[1].atoms_slice()[0].residue.resn, "GLY");
    }

    #[test]
    fn read_bcif_bytes_returns_first_logical_molecule() {
        let rows = [
            atom_row(1, "N", "ALA", "A", 0.0, 1),
            atom_row(2, "CA", "ALA", "A", 1.0, 1),
            atom_row(1, "N", "ALA", "B", 0.5, 2),
        ];
        let blocks = vec![atom_site_block("TEST", &rows)];
        let bytes = encode_bcif_file(&blocks);

        let molecule = crate::bcif::read_bcif_bytes_with_bond_tolerance(&bytes, 0.1).unwrap();

        assert_eq!(molecule.atom_count(), 2);
        assert_eq!(molecule.state_count(), 1);
        assert_eq!(molecule.atoms_slice()[0].residue.chain, "A");
    }
}
