//! mmCIF file parser
//!
//! Parses macromolecular CIF format files.

use std::collections::{BTreeMap, HashMap};
use std::io::{BufReader, Read};

use lin_alg::f32::Vec3;
use patinae_mol::{Element, ObjectMolecule};

use crate::assembly::{AssemblyRow, AssemblyRows, CATEGORIES};
use crate::error::{IoError, IoResult};
use crate::logical_models::{build_molecules, ParsedAtom, ParsedModel};
use crate::traits::MoleculeReader;

use super::common::{
    apply_secondary_structure, parse_ss_record, SecondaryStructureRange, SsCategory,
};
use super::lexer::{tokenize, Token};

/// Pre-resolved column indices for _atom_site loop — avoids per-atom HashMap lookups
#[derive(Default)]
struct AtomSiteColumns {
    type_symbol: Option<usize>,
    label_atom_id: Option<usize>,
    auth_atom_id: Option<usize>,
    label_comp_id: Option<usize>,
    auth_comp_id: Option<usize>,
    label_asym_id: Option<usize>,
    auth_asym_id: Option<usize>,
    label_seq_id: Option<usize>,
    auth_seq_id: Option<usize>,
    pdbx_pdb_ins_code: Option<usize>,
    label_alt_id: Option<usize>,
    b_iso_or_equiv: Option<usize>,
    occupancy: Option<usize>,
    group_pdb: Option<usize>,
    pdbx_formal_charge: Option<usize>,
    id: Option<usize>,
    cartn_x: Option<usize>,
    cartn_y: Option<usize>,
    cartn_z: Option<usize>,
    pdbx_pdb_model_num: Option<usize>,
}

struct CifBlockData {
    name: String,
    title: String,
    models: BTreeMap<i32, ParsedModel>,
    cell: CellInfo,
    ss_ranges: Vec<SecondaryStructureRange>,
    space_group: Option<String>,
    assemblies: AssemblyRows,
    assembly_singles: BTreeMap<String, AssemblyRow>,
    assembly_error: Option<IoError>,
}

impl CifBlockData {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            title: String::new(),
            models: BTreeMap::new(),
            cell: CellInfo::default(),
            ss_ranges: Vec::new(),
            space_group: None,
            assemblies: AssemblyRows::default(),
            assembly_singles: BTreeMap::new(),
            assembly_error: None,
        }
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

impl AtomSiteColumns {
    /// Build column index struct from the loop column names
    fn from_columns(columns: &[&str]) -> Self {
        let mut cols = AtomSiteColumns::default();
        for (i, name) in columns.iter().enumerate() {
            match *name {
                "_atom_site.type_symbol" => cols.type_symbol = Some(i),
                "_atom_site.label_atom_id" => cols.label_atom_id = Some(i),
                "_atom_site.auth_atom_id" => cols.auth_atom_id = Some(i),
                "_atom_site.label_comp_id" => cols.label_comp_id = Some(i),
                "_atom_site.auth_comp_id" => cols.auth_comp_id = Some(i),
                "_atom_site.label_asym_id" => cols.label_asym_id = Some(i),
                "_atom_site.auth_asym_id" => cols.auth_asym_id = Some(i),
                "_atom_site.label_seq_id" => cols.label_seq_id = Some(i),
                "_atom_site.auth_seq_id" => cols.auth_seq_id = Some(i),
                "_atom_site.pdbx_PDB_ins_code" => cols.pdbx_pdb_ins_code = Some(i),
                "_atom_site.label_alt_id" => cols.label_alt_id = Some(i),
                "_atom_site.B_iso_or_equiv" => cols.b_iso_or_equiv = Some(i),
                "_atom_site.occupancy" => cols.occupancy = Some(i),
                "_atom_site.group_PDB" => cols.group_pdb = Some(i),
                "_atom_site.pdbx_formal_charge" => cols.pdbx_formal_charge = Some(i),
                "_atom_site.id" => cols.id = Some(i),
                "_atom_site.Cartn_x" => cols.cartn_x = Some(i),
                "_atom_site.Cartn_y" => cols.cartn_y = Some(i),
                "_atom_site.Cartn_z" => cols.cartn_z = Some(i),
                "_atom_site.pdbx_PDB_model_num" => cols.pdbx_pdb_model_num = Some(i),
                _ => {}
            }
        }
        cols
    }
}

/// mmCIF file reader
pub struct CifReader<R> {
    reader: BufReader<R>,
    bond_tolerance: f32,
}

impl<R: Read> CifReader<R> {
    /// Create a new CIF reader
    pub fn new(reader: R) -> Self {
        CifReader {
            reader: BufReader::new(reader),
            bond_tolerance: patinae_mol::DEFAULT_BOND_TOLERANCE,
        }
    }

    pub fn with_bond_tolerance(mut self, tolerance: f32) -> Self {
        self.bond_tolerance = tolerance;
        self
    }

    /// Read and parse the CIF file
    fn parse(&mut self) -> IoResult<ObjectMolecule> {
        // Read entire content
        let mut content = String::new();
        self.reader
            .read_to_string(&mut content)
            .map_err(IoError::io)?;

        // Tokenize
        let tokens = tokenize(&content);

        parse_cif_tokens(&tokens, self.bond_tolerance)
    }

    fn parse_all(&mut self) -> IoResult<Vec<ObjectMolecule>> {
        let mut content = String::new();
        self.reader
            .read_to_string(&mut content)
            .map_err(IoError::io)?;

        let tokens = tokenize(&content);
        parse_cif_tokens_all(&tokens, self.bond_tolerance)
    }
}

impl<R: Read> MoleculeReader for CifReader<R> {
    fn read(&mut self) -> IoResult<ObjectMolecule> {
        self.parse()
    }

    fn read_all(&mut self) -> IoResult<Vec<ObjectMolecule>> {
        self.parse_all()
    }
}

/// Parse CIF tokens into a molecule
fn parse_cif_tokens(tokens: &[Token], bond_tolerance: f32) -> IoResult<ObjectMolecule> {
    parse_cif_tokens_all(tokens, bond_tolerance)?
        .into_iter()
        .next()
        .ok_or(IoError::empty_file())
}

fn parse_cif_tokens_all(tokens: &[Token], bond_tolerance: f32) -> IoResult<Vec<ObjectMolecule>> {
    let mut pos = 0;
    let mut molecules = Vec::new();

    while pos < tokens.len() {
        match &tokens[pos] {
            Token::DataBlock(name) => {
                let block_start = pos + 1;
                let mut block_end = block_start;
                while block_end < tokens.len()
                    && !matches!(&tokens[block_end], Token::DataBlock(_) | Token::Eof)
                {
                    block_end += 1;
                }
                molecules.extend(parse_cif_block(
                    name,
                    &tokens[block_start..block_end],
                    bond_tolerance,
                )?);
                pos = block_end;
            }
            Token::Eof => break,
            _ => pos += 1,
        }
    }

    if molecules.is_empty() {
        Err(IoError::empty_file())
    } else {
        Ok(molecules)
    }
}

fn parse_cif_block(
    name: &str,
    tokens: &[Token],
    bond_tolerance: f32,
) -> IoResult<Vec<ObjectMolecule>> {
    let mut block = CifBlockData::new(name);
    let mut pos = 0;

    while pos < tokens.len() {
        match &tokens[pos] {
            Token::Loop => {
                pos += 1;
                pos = parse_loop(tokens, pos, &mut block)?;
            }
            Token::DataName(name) if name.starts_with("_cell.") => {
                pos = parse_cell(tokens, pos, &mut block.cell);
            }
            Token::DataName(name) if name.starts_with("_symmetry.") => {
                pos = parse_symmetry(tokens, pos, &mut block.space_group);
            }
            Token::DataName(name) if name.starts_with("_entry.") => {
                pos = parse_entry(tokens, pos, &mut block.title);
            }
            // Handle single-item secondary structure format (when not in a loop)
            Token::DataName(name) if name.starts_with("_struct_conf.") => {
                pos = parse_ss_single(tokens, pos, SsCategory::StructConf, &mut block.ss_ranges);
            }
            Token::DataName(name) if name.starts_with("_struct_sheet_range.") => {
                pos = parse_ss_single(
                    tokens,
                    pos,
                    SsCategory::StructSheetRange,
                    &mut block.ss_ranges,
                );
            }
            Token::DataName(name) => {
                if let Some((category, field)) = name.split_once('.') {
                    if CATEGORIES.contains(&category) {
                        if let Some(value) = tokens.get(pos + 1).and_then(token_value_str) {
                            block
                                .assembly_singles
                                .entry(category.to_owned())
                                .or_default()
                                .insert(field.to_owned(), value.to_owned());
                        }
                    }
                }
                pos += 1;
            }
            Token::Eof => break,
            _ => pos += 1,
        }
    }

    if block.models.is_empty() {
        return Ok(Vec::new());
    }

    // Resolve after reading the complete block: categories may follow the atoms.
    // Keep assembly errors deferred, including for blocks without atom data.
    if let Some(error) = block.assembly_error {
        return Err(error);
    }
    for (category, row) in block.assembly_singles {
        block.assemblies.push(&category, row);
    }
    let definitions = block.assemblies.resolve()?;
    if definitions.is_empty() {
        for model in block.models.values_mut() {
            model.source_chains.clear();
        }
    }
    let models = block.models.into_values().collect();
    let mut molecules = build_molecules(&block.name, &block.title, models)?;

    for mol in &mut molecules {
        mol.assembly.definitions = definitions.clone();
        block.cell.apply_to(mol, block.space_group.as_deref());
        apply_secondary_structure(mol, &block.ss_ranges);

        // Classify atoms (protein, nucleic, solvent, etc.)
        // This is needed for cartoon representation to work
        mol.classify_atoms();

        // Generate covalent bonds from distances
        // CIF files typically don't include explicit bond information
        mol.generate_bonds(bond_tolerance);

        // Assign bond orders for known protein residues using atom name templates
        mol.assign_known_residue_bond_orders();
    }

    Ok(molecules)
}

/// Parse a loop_ construct
fn parse_loop(tokens: &[Token], mut pos: usize, block: &mut CifBlockData) -> IoResult<usize> {
    // Collect column names
    let mut columns: Vec<&str> = Vec::new();
    while pos < tokens.len() {
        match &tokens[pos] {
            Token::DataName(name) => {
                columns.push(name);
                pos += 1;
            }
            _ => break,
        }
    }

    if columns.is_empty() {
        return Ok(pos);
    }

    let assembly_category = columns
        .first()
        .and_then(|name| name.split_once('.'))
        .map(|(category, _)| category)
        .filter(|category| CATEGORIES.contains(category));
    let data_start = pos;

    // Check loop category
    let is_atom_site = columns.iter().any(|c| c.starts_with("_atom_site."));
    let is_struct_conf = columns.iter().any(|c| c.starts_with("_struct_conf."));
    let is_struct_sheet = columns
        .iter()
        .any(|c| c.starts_with("_struct_sheet_range."));

    if is_atom_site {
        pos = parse_atom_site_loop(tokens, pos, &columns, &mut block.models)?;
    } else if is_struct_conf {
        pos = parse_ss_loop(
            tokens,
            pos,
            &columns,
            SsCategory::StructConf,
            &mut block.ss_ranges,
        )?;
    } else if is_struct_sheet {
        pos = parse_ss_loop(
            tokens,
            pos,
            &columns,
            SsCategory::StructSheetRange,
            &mut block.ss_ranges,
        )?;
    } else {
        // Skip other loops
        while pos < tokens.len() {
            match &tokens[pos] {
                Token::Loop | Token::DataName(_) | Token::DataBlock(_) | Token::Eof => break,
                _ => pos += 1,
            }
        }
    }

    if let Some(category) = assembly_category {
        if let Err(error) = parse_assembly_loop(
            &tokens[data_start..pos],
            &columns,
            category,
            &mut block.assemblies,
        ) {
            block.assembly_error.get_or_insert(error);
        }
    }

    Ok(pos)
}

/// Extract a `&str` from a value token (zero-copy)
fn token_value_str<'a>(token: &'a Token<'a>) -> Option<&'a str> {
    match token {
        Token::Value(v) | Token::SingleQuoted(v) | Token::DoubleQuoted(v) | Token::TextField(v) => {
            Some(v)
        }
        _ => None,
    }
}

/// Parse _atom_site loop
fn parse_atom_site_loop(
    tokens: &[Token],
    mut pos: usize,
    columns: &[&str],
    models: &mut BTreeMap<i32, ParsedModel>,
) -> IoResult<usize> {
    // Pre-resolve column indices — O(1) lookups per field instead of HashMap
    let cols = AtomSiteColumns::from_columns(columns);

    let n_cols = columns.len();

    // Row buffer — allocated once and reused for every atom (zero-copy borrows)
    let mut row: Vec<Option<&str>> = vec![None; n_cols];

    // Helper: get column value by pre-resolved index
    macro_rules! col {
        ($idx:expr) => {
            $idx.and_then(|i| row[i])
        };
    }

    // Parse rows
    loop {
        // Check if we've reached end of loop data
        if pos >= tokens.len() {
            break;
        }
        match &tokens[pos] {
            Token::Loop | Token::DataName(_) | Token::DataBlock(_) | Token::Eof => break,
            _ => {}
        }

        // Reset row
        row.fill(None);

        // Read one row — zero-copy borrows from tokens
        let mut n_read = 0;
        for _ in 0..n_cols {
            if pos >= tokens.len() {
                break;
            }
            match &tokens[pos] {
                Token::Value(v)
                | Token::SingleQuoted(v)
                | Token::DoubleQuoted(v)
                | Token::TextField(v) => {
                    row[n_read] = Some(v);
                }
                Token::Missing | Token::Unknown => {
                    row[n_read] = None;
                }
                Token::Loop | Token::DataName(_) | Token::DataBlock(_) | Token::Eof => {
                    break;
                }
            }
            n_read += 1;
            pos += 1;
        }

        if n_read < n_cols {
            // Incomplete row - end of loop
            break;
        }

        // Get element (prefer type_symbol over label_atom_id)
        let element_str = col!(cols.type_symbol)
            .or_else(|| col!(cols.label_atom_id))
            .unwrap_or("X");
        let element = Element::from_symbol(element_str).unwrap_or(Element::Unknown);

        // Get atom name (prefer auth_atom_id over label_atom_id)
        let atom_name = col!(cols.auth_atom_id)
            .or_else(|| col!(cols.label_atom_id))
            .unwrap_or("X");

        let resn_str = col!(cols.auth_comp_id)
            .or_else(|| col!(cols.label_comp_id))
            .unwrap_or("");

        let resv: i32 = col!(cols.auth_seq_id)
            .or_else(|| col!(cols.label_seq_id))
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        let chain_str = col!(cols.auth_asym_id)
            .or_else(|| col!(cols.label_asym_id))
            .unwrap_or("");

        let inscode = col!(cols.pdbx_pdb_ins_code)
            .and_then(|s| s.chars().next())
            .unwrap_or(' ');

        let alt = col!(cols.label_alt_id)
            .and_then(|s| s.chars().next())
            .unwrap_or(' ');

        let b_factor = col!(cols.b_iso_or_equiv)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);

        let occupancy = col!(cols.occupancy)
            .and_then(|s| s.parse().ok())
            .unwrap_or(1.0);

        let hetatm = col!(cols.group_pdb).map(|s| s == "HETATM").unwrap_or(false);

        let formal_charge = col!(cols.pdbx_formal_charge).and_then(|s| s.parse().ok());
        let serial = col!(cols.id).and_then(|s| s.parse().ok());

        // Coordinates
        let x: f32 = col!(cols.cartn_x)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);
        let y: f32 = col!(cols.cartn_y)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);
        let z: f32 = col!(cols.cartn_z)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);

        // Model number
        let model_num: i32 = col!(cols.pdbx_pdb_model_num)
            .and_then(|s| s.parse().ok())
            .unwrap_or(1);

        let model = models
            .entry(model_num)
            .or_insert_with(|| ParsedModel::new(model_num));
        model
            .source_chains
            .push(col!(cols.label_asym_id).unwrap_or(""));
        model.atoms.push(ParsedAtom {
            name: atom_name.to_string(),
            element,
            chain: chain_str.replace('-', ""),
            resn: resn_str.to_string(),
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

    Ok(pos)
}

/// Collect assembly rows from a loop already located by the block parser.
fn parse_assembly_loop(
    tokens: &[Token],
    columns: &[&str],
    category: &str,
    rows: &mut AssemblyRows,
) -> IoResult<()> {
    let mut chunks = tokens.chunks_exact(columns.len());
    for tokens in &mut chunks {
        let mut row = AssemblyRow::new();
        for (name, token) in columns.iter().zip(tokens) {
            if let (Some((_, field)), Some(value)) = (name.split_once('.'), token_value_str(token))
            {
                row.insert(field.to_owned(), value.to_owned());
            }
        }
        rows.push(category, row);
    }
    if !chunks.remainder().is_empty() {
        return Err(IoError::parse_msg("Incomplete assembly loop row"));
    }
    Ok(())
}

/// Parse _cell data items
fn parse_cell(tokens: &[Token], mut pos: usize, cell: &mut CellInfo) -> usize {
    while pos < tokens.len() {
        match &tokens[pos] {
            Token::DataName(name) if name.starts_with("_cell.") => {
                let field = &name[6..];
                pos += 1;
                if let Some(token) = tokens.get(pos) {
                    if let Some(val) = token_value_str(token) {
                        match field {
                            "length_a" => cell.a = val.parse().unwrap_or(1.0),
                            "length_b" => cell.b = val.parse().unwrap_or(1.0),
                            "length_c" => cell.c = val.parse().unwrap_or(1.0),
                            "angle_alpha" => cell.alpha = val.parse().unwrap_or(90.0),
                            "angle_beta" => cell.beta = val.parse().unwrap_or(90.0),
                            "angle_gamma" => cell.gamma = val.parse().unwrap_or(90.0),
                            _ => {}
                        }
                    }
                    pos += 1;
                }
            }
            _ => break,
        }
    }

    pos
}

/// Parse _symmetry data items (space group name)
fn parse_symmetry(tokens: &[Token], mut pos: usize, space_group: &mut Option<String>) -> usize {
    while pos < tokens.len() {
        match &tokens[pos] {
            Token::DataName(name) if name.starts_with("_symmetry.") => {
                let field = &name["_symmetry.".len()..];
                pos += 1;
                if let Some(token) = tokens.get(pos) {
                    if let Some(val) = token_value_str(token) {
                        if field == "space_group_name_H-M"
                            || (field == "space_group_name_Hall" && space_group.is_none())
                        {
                            let sg = val.trim().to_string();
                            if !sg.is_empty() && sg != "?" {
                                *space_group = Some(sg);
                            }
                        }
                    }
                    pos += 1;
                }
            }
            _ => break,
        }
    }
    pos
}

fn parse_entry(tokens: &[Token], mut pos: usize, title: &mut String) -> usize {
    while pos < tokens.len() {
        match &tokens[pos] {
            Token::DataName(name) if name.starts_with("_entry.") => {
                pos += 1;
                if let Some(token) = tokens.get(pos) {
                    if title.is_empty() {
                        if let Some(val) = token_value_str(token) {
                            *title = val.to_string();
                        }
                    }
                    pos += 1;
                }
            }
            _ => break,
        }
    }
    pos
}

/// Parse secondary structure loop (_struct_conf or _struct_sheet_range)
///
/// This unified function handles both helix and sheet annotations in loop format.
fn parse_ss_loop(
    tokens: &[Token],
    mut pos: usize,
    columns: &[&str],
    category: SsCategory,
    ss_ranges: &mut Vec<SecondaryStructureRange>,
) -> IoResult<usize> {
    let prefix = category.prefix();

    // Map column indices to short field names (without prefix)
    let col_to_field: Vec<Option<&str>> =
        columns.iter().map(|col| col.strip_prefix(prefix)).collect();

    let n_cols = columns.len();

    // Row buffer — allocated once and reused
    let mut row: Vec<Option<&str>> = vec![None; n_cols];

    loop {
        if pos >= tokens.len() {
            break;
        }
        match &tokens[pos] {
            Token::Loop | Token::DataName(_) | Token::DataBlock(_) | Token::Eof => break,
            _ => {}
        }

        // Reset row
        row.fill(None);

        // Read one row — zero-copy
        let mut col = 0;
        for _ in 0..n_cols {
            if pos >= tokens.len() {
                break;
            }
            match &tokens[pos] {
                Token::Value(v)
                | Token::SingleQuoted(v)
                | Token::DoubleQuoted(v)
                | Token::TextField(v) => {
                    row[col] = Some(v);
                }
                Token::Missing | Token::Unknown => {
                    row[col] = None;
                }
                Token::Loop | Token::DataName(_) | Token::DataBlock(_) | Token::Eof => {
                    break;
                }
            }
            col += 1;
            pos += 1;
        }

        if col < n_cols {
            break;
        }

        // Build field map from row values (zero-copy borrows)
        let mut fields: HashMap<&str, &str> = HashMap::new();
        for (i, field_name) in col_to_field.iter().enumerate() {
            if let (Some(name), Some(value)) = (field_name, row[i]) {
                fields.insert(name, value);
            }
        }

        // Use unified record parser
        if let Some(range) = parse_ss_record(category, &fields) {
            ss_ranges.push(range);
        }
    }

    Ok(pos)
}

/// Parse secondary structure from single-item (non-loop) format
///
/// This handles cases where there's only one helix or sheet entry,
/// stored as individual data items rather than a loop.
fn parse_ss_single(
    tokens: &[Token],
    mut pos: usize,
    category: SsCategory,
    ss_ranges: &mut Vec<SecondaryStructureRange>,
) -> usize {
    let prefix = category.prefix();
    let mut fields: HashMap<&str, &str> = HashMap::new();

    // Collect consecutive data items with matching prefix
    while pos < tokens.len() {
        if let Token::DataName(name) = &tokens[pos] {
            if let Some(field) = name.strip_prefix(prefix) {
                pos += 1;
                // Read the value token
                if pos < tokens.len() {
                    if let Some(val) = token_value_str(&tokens[pos]) {
                        fields.insert(field, val);
                        pos += 1;
                    } else if matches!(&tokens[pos], Token::Missing | Token::Unknown) {
                        // Skip missing/unknown values
                        pos += 1;
                    }
                }
                continue;
            }
        }
        break;
    }

    // Use unified record parser
    if !fields.is_empty() {
        if let Some(range) = parse_ss_record(category, &fields) {
            ss_ranges.push(range);
        }
    }

    pos
}

#[cfg(test)]
mod tests {
    use super::*;
    use patinae_mol::SecondaryStructure;

    #[test]
    fn test_read_simple_cif() {
        let cif_data = r#"data_TEST
_entry.id TEST
loop_
_atom_site.id
_atom_site.type_symbol
_atom_site.label_atom_id
_atom_site.label_comp_id
_atom_site.label_asym_id
_atom_site.label_seq_id
_atom_site.Cartn_x
_atom_site.Cartn_y
_atom_site.Cartn_z
_atom_site.occupancy
_atom_site.B_iso_or_equiv
1 N N ALA A 1 0.000 0.000 0.000 1.00 20.00
2 C CA ALA A 1 1.458 0.000 0.000 1.00 20.00
3 C C ALA A 1 2.009 1.420 0.000 1.00 20.00
"#;

        let mut reader = CifReader::new(cif_data.as_bytes());
        let mol = reader.read().unwrap();

        assert_eq!(mol.name, "TEST");
        assert_eq!(mol.atom_count(), 3);
        assert_eq!(mol.state_count(), 1);
    }

    #[test]
    fn test_cif_loads_all_model_states() {
        let cif_data = r#"data_MODELS
_entry.id MODELS
loop_
_atom_site.id
_atom_site.type_symbol
_atom_site.label_atom_id
_atom_site.label_comp_id
_atom_site.label_asym_id
_atom_site.label_seq_id
_atom_site.Cartn_x
_atom_site.Cartn_y
_atom_site.Cartn_z
_atom_site.occupancy
_atom_site.B_iso_or_equiv
_atom_site.pdbx_PDB_model_num
1 C CA ALA A 1 0.000 0.000 0.000 1.00 20.00 1
2 O O  ALA A 1 1.000 0.000 0.000 1.00 20.00 1
3 C CA ALA A 1 0.500 0.000 0.000 1.00 20.00 2
4 O O  ALA A 1 1.500 0.000 0.000 1.00 20.00 2
"#;
        let mut reader = CifReader::new(cif_data.as_bytes());
        let mol = reader.read().unwrap();

        assert_eq!(mol.atom_count(), 2);
        assert_eq!(mol.state_count(), 2);

        let state_1 = mol
            .get_coord_set(0)
            .unwrap()
            .get_atom_coord(patinae_mol::AtomIndex(0));
        let state_2 = mol
            .get_coord_set(1)
            .unwrap()
            .get_atom_coord(patinae_mol::AtomIndex(0));
        assert_ne!(state_1, state_2);
    }

    #[test]
    fn test_parse_with_cell() {
        let cif_data = r#"data_1ABC
_cell.length_a 50.0
_cell.length_b 60.0
_cell.length_c 70.0
_cell.angle_alpha 90.0
_cell.angle_beta 90.0
_cell.angle_gamma 90.0
loop_
_atom_site.id
_atom_site.type_symbol
_atom_site.Cartn_x
_atom_site.Cartn_y
_atom_site.Cartn_z
1 C 0.0 0.0 0.0
"#;

        let mut reader = CifReader::new(cif_data.as_bytes());
        let mol = reader.read().unwrap();

        assert!(mol.symmetry.is_some());
        let sym = mol.symmetry.as_ref().unwrap();
        assert!((sym.cell_lengths[0] - 50.0).abs() < 0.001);
    }

    #[test]
    fn test_parse_single_item_secondary_structure() {
        // Test single-item (non-loop) secondary structure format
        let cif_data = r#"data_TEST
loop_
_atom_site.id
_atom_site.type_symbol
_atom_site.label_atom_id
_atom_site.label_comp_id
_atom_site.auth_asym_id
_atom_site.auth_seq_id
_atom_site.Cartn_x
_atom_site.Cartn_y
_atom_site.Cartn_z
1 N N ALA A 1 0.0 0.0 0.0
2 C CA ALA A 1 1.5 0.0 0.0
3 N N GLU A 2 3.0 0.0 0.0
4 C CA GLU A 2 4.5 0.0 0.0
5 N N LEU A 3 6.0 0.0 0.0
6 C CA LEU A 3 7.5 0.0 0.0
#
_struct_conf.conf_type_id HELX_P
_struct_conf.id HELX_P1
_struct_conf.beg_auth_asym_id A
_struct_conf.beg_auth_seq_id 1
_struct_conf.end_auth_asym_id A
_struct_conf.end_auth_seq_id 3
"#;

        let mut reader = CifReader::new(cif_data.as_bytes());
        let mol = reader.read().unwrap();

        assert_eq!(mol.atom_count(), 6);

        // All atoms should have helix secondary structure
        for atom in mol.atoms() {
            assert_eq!(
                atom.ss_type,
                SecondaryStructure::Helix,
                "Atom {} resv={} should be Helix",
                atom.name,
                atom.residue.resv
            );
        }
    }

    #[test]
    fn test_parse_dna_nucleotides() {
        // Test that DNA nucleotides are parsed and classified correctly
        let cif_data = r#"data_TEST
loop_
_atom_site.group_PDB
_atom_site.id
_atom_site.type_symbol
_atom_site.label_atom_id
_atom_site.label_comp_id
_atom_site.auth_asym_id
_atom_site.auth_seq_id
_atom_site.auth_comp_id
_atom_site.Cartn_x
_atom_site.Cartn_y
_atom_site.Cartn_z
ATOM 1  P P   DA I 1 DA 0.0 0.0 0.0
ATOM 2  C "C4'" DA I 1 DA 1.0 0.0 0.0
ATOM 3  P P   DT I 2 DT 2.0 0.0 0.0
ATOM 4  C "C4'" DT I 2 DT 3.0 0.0 0.0
ATOM 5  P P   DG I 3 DG 4.0 0.0 0.0
ATOM 6  C "C4'" DG I 3 DG 5.0 0.0 0.0
ATOM 7  P P   DC I 4 DC 6.0 0.0 0.0
ATOM 8  C "C4'" DC I 4 DC 7.0 0.0 0.0
"#;

        let mut reader = CifReader::new(cif_data.as_bytes());
        let mol = reader.read().unwrap();

        assert_eq!(mol.atom_count(), 8);

        // All atoms should be classified as nucleic
        use patinae_mol::AtomFlags;
        for atom in mol.atoms() {
            assert!(
                atom.state.flags.contains(AtomFlags::NUCLEIC),
                "Atom {} in residue {} should have NUCLEIC flag",
                atom.name,
                atom.residue.resn
            );
            assert!(
                atom.state.flags.contains(AtomFlags::POLYMER),
                "Atom {} in residue {} should have POLYMER flag",
                atom.name,
                atom.residue.resn
            );
        }

        // Check chain iteration: should have 1 chain (I) with 4 residues
        let chains: Vec<_> = mol.chains().collect();
        assert_eq!(chains.len(), 1, "Should have 1 chain");
        assert_eq!(chains[0].id(), "I");

        let residues: Vec<_> = chains[0].residues().collect();
        assert_eq!(residues.len(), 4, "Chain I should have 4 residues");
        assert_eq!(residues[0].resn(), "DA");
        assert_eq!(residues[1].resn(), "DT");
        assert_eq!(residues[2].resn(), "DG");
        assert_eq!(residues[3].resn(), "DC");

        // All should be classified as nucleic
        for r in &residues {
            assert!(r.is_nucleic(), "Residue {} should be nucleic", r.resn());
        }
    }

    #[test]
    fn test_parse_loop_secondary_structure() {
        // Test loop format secondary structure (existing behavior)
        let cif_data = r#"data_TEST
loop_
_atom_site.id
_atom_site.type_symbol
_atom_site.label_atom_id
_atom_site.label_comp_id
_atom_site.auth_asym_id
_atom_site.auth_seq_id
_atom_site.Cartn_x
_atom_site.Cartn_y
_atom_site.Cartn_z
1 N N ALA A 1 0.0 0.0 0.0
2 C CA ALA A 1 1.5 0.0 0.0
3 N N GLU A 2 3.0 0.0 0.0
4 C CA GLU A 2 4.5 0.0 0.0
#
loop_
_struct_conf.conf_type_id
_struct_conf.id
_struct_conf.beg_auth_asym_id
_struct_conf.beg_auth_seq_id
_struct_conf.end_auth_asym_id
_struct_conf.end_auth_seq_id
HELX_P HELX_P1 A 1 A 2
"#;

        let mut reader = CifReader::new(cif_data.as_bytes());
        let mol = reader.read().unwrap();

        assert_eq!(mol.atom_count(), 4);

        // All atoms should have helix secondary structure
        for atom in mol.atoms() {
            assert_eq!(
                atom.ss_type,
                SecondaryStructure::Helix,
                "Atom {} resv={} should be Helix",
                atom.name,
                atom.residue.resv
            );
        }
    }

    #[test]
    fn read_all_parses_multiple_data_blocks() {
        let cif_data = r#"data_FIRST
loop_
_atom_site.id
_atom_site.type_symbol
_atom_site.label_atom_id
_atom_site.label_comp_id
_atom_site.label_asym_id
_atom_site.label_seq_id
_atom_site.Cartn_x
_atom_site.Cartn_y
_atom_site.Cartn_z
1 C C LIG A 1 0.0 0.0 0.0
data_SECOND
loop_
_atom_site.id
_atom_site.type_symbol
_atom_site.label_atom_id
_atom_site.label_comp_id
_atom_site.label_asym_id
_atom_site.label_seq_id
_atom_site.Cartn_x
_atom_site.Cartn_y
_atom_site.Cartn_z
1 N N LIG B 1 1.0 0.0 0.0
"#;

        let mut reader = CifReader::new(cif_data.as_bytes());
        let molecules = reader.read_all().unwrap();

        assert_eq!(molecules.len(), 2);
        assert_eq!(molecules[0].name, "FIRST");
        assert_eq!(molecules[1].name, "SECOND");
    }

    #[test]
    fn read_all_groups_same_topology_models_as_states() {
        let cif_data = r#"data_TEST
loop_
_atom_site.id
_atom_site.type_symbol
_atom_site.label_atom_id
_atom_site.label_comp_id
_atom_site.label_asym_id
_atom_site.label_seq_id
_atom_site.Cartn_x
_atom_site.Cartn_y
_atom_site.Cartn_z
_atom_site.pdbx_PDB_model_num
1 N N ALA A 1 0.0 0.0 0.0 1
2 C CA ALA A 1 1.0 0.0 0.0 1
3 N N ALA A 1 0.5 0.0 0.0 2
4 C CA ALA A 1 1.5 0.0 0.0 2
"#;

        let mut reader = CifReader::new(cif_data.as_bytes());
        let molecules = reader.read_all().unwrap();

        assert_eq!(molecules.len(), 1);
        assert_eq!(molecules[0].atom_count(), 2);
        assert_eq!(molecules[0].state_count(), 2);
        assert_eq!(molecules[0].atoms_slice()[0].id, 1);
        assert_eq!(molecules[0].atoms_slice()[1].id, 2);
    }

    #[test]
    fn read_all_splits_incompatible_models() {
        let cif_data = r#"data_TEST
loop_
_atom_site.id
_atom_site.type_symbol
_atom_site.label_atom_id
_atom_site.label_comp_id
_atom_site.label_asym_id
_atom_site.label_seq_id
_atom_site.Cartn_x
_atom_site.Cartn_y
_atom_site.Cartn_z
_atom_site.pdbx_PDB_model_num
1 N N ALA A 1 0.0 0.0 0.0 1
2 C CA ALA A 1 1.0 0.0 0.0 1
1 N N GLY A 1 0.5 0.0 0.0 2
2 C CA GLY A 1 1.5 0.0 0.0 2
"#;

        let mut reader = CifReader::new(cif_data.as_bytes());
        let molecules = reader.read_all().unwrap();

        assert_eq!(molecules.len(), 2);
        assert_eq!(molecules[0].name, "TEST_model_1");
        assert_eq!(molecules[1].name, "TEST_model_2");
        assert_eq!(molecules[0].atoms_slice()[0].residue.resn, "ALA");
        assert_eq!(molecules[1].atoms_slice()[0].residue.resn, "GLY");
    }

    #[test]
    fn read_cif_str_returns_first_logical_molecule() {
        let cif_data = r#"data_TEST
loop_
_atom_site.id
_atom_site.type_symbol
_atom_site.label_atom_id
_atom_site.label_comp_id
_atom_site.label_asym_id
_atom_site.label_seq_id
_atom_site.Cartn_x
_atom_site.Cartn_y
_atom_site.Cartn_z
_atom_site.pdbx_PDB_model_num
1 N N ALA A 1 0.0 0.0 0.0 1
2 C CA ALA A 1 1.0 0.0 0.0 1
1 N N ALA B 1 0.5 0.0 0.0 2
"#;

        let molecule = crate::cif::read_cif_str(cif_data).unwrap();

        assert_eq!(molecule.atom_count(), 2);
        assert_eq!(molecule.state_count(), 1);
        assert_eq!(molecule.atoms_slice()[0].residue.chain, "A");
    }
}
