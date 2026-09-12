//! Shared assembly metadata decoding; coordinates remain in the asymmetric unit.

use std::collections::BTreeMap;

use patinae_mol::{
    multiply_instance_transforms, AssemblyDefinition, AssemblyGroup, InstanceTransform,
    IDENTITY_INSTANCE, MAX_INSTANCE_COUNT,
};

use crate::error::{IoError, IoResult};

pub(crate) const CATEGORIES: [&str; 3] = [
    "_pdbx_struct_assembly",
    "_pdbx_struct_assembly_gen",
    "_pdbx_struct_oper_list",
];

pub(crate) type AssemblyRow = BTreeMap<String, String>;

#[derive(Default)]
pub(crate) struct AssemblyRows {
    pub(crate) definitions: Vec<AssemblyRow>,
    pub(crate) generators: Vec<AssemblyRow>,
    pub(crate) operators: Vec<AssemblyRow>,
}

fn required<'a>(row: &'a AssemblyRow, field: &str) -> IoResult<&'a str> {
    row.get(field)
        .map(String::as_str)
        .filter(|s| !s.is_empty() && *s != "." && *s != "?")
        .ok_or_else(|| IoError::parse_msg(format!("Missing assembly field '{field}'")))
}

impl AssemblyRows {
    pub(crate) fn push(&mut self, category: &str, row: AssemblyRow) {
        match category {
            "_pdbx_struct_assembly" => self.definitions.push(row),
            "_pdbx_struct_assembly_gen" => self.generators.push(row),
            "_pdbx_struct_oper_list" => self.operators.push(row),
            _ => {}
        }
    }

    pub(crate) fn resolve(self) -> IoResult<Vec<AssemblyDefinition>> {
        // Operator tables can describe crystallographic operations without assemblies.
        if self.definitions.is_empty() && self.generators.is_empty() {
            return Ok(Vec::new());
        }
        let mut operators = BTreeMap::new();
        for row in self.operators {
            let id = required(&row, "id")?.to_owned();
            let mut transform = IDENTITY_INSTANCE;
            for r in 0..3 {
                for (c, column) in transform.iter_mut().enumerate() {
                    let field = if c == 3 {
                        format!("vector[{}]", r + 1)
                    } else {
                        format!("matrix[{}][{}]", r + 1, c + 1)
                    };
                    column[r] = parse_number(required(&row, &field)?)?;
                }
            }
            if operators.insert(id.clone(), transform).is_some() {
                return Err(IoError::parse_msg(format!(
                    "Duplicate assembly operator '{id}'"
                )));
            }
        }
        let mut definitions: Vec<AssemblyDefinition> = Vec::new();
        for row in self.definitions {
            let id = required(&row, "id")?.to_owned();
            if definitions.iter().any(|d| d.id == id) {
                return Err(IoError::parse_msg(format!("Duplicate assembly '{id}'")));
            }
            definitions.push(AssemblyDefinition {
                id,
                details: row.get("details").cloned().unwrap_or_default(),
                groups: Vec::new(),
            });
        }
        for row in self.generators {
            let id = required(&row, "assembly_id")?;
            let definition = definitions
                .iter_mut()
                .find(|d| d.id == id)
                .ok_or_else(|| IoError::parse_msg(format!("Unknown assembly '{id}'")))?;
            let chains = parse_chains(required(&row, "asym_id_list")?, false)?;
            let transforms = resolve_expression(required(&row, "oper_expression")?, &operators)?;
            let total = definition
                .groups
                .iter()
                .map(|g| g.transforms.len())
                .sum::<usize>();
            if transforms.len() > MAX_INSTANCE_COUNT.saturating_sub(total) {
                return Err(IoError::parse_msg("Assembly exceeds 65535 copies"));
            }
            definition.groups.push(AssemblyGroup { chains, transforms });
        }
        Ok(definitions)
    }
}

fn parse_number(value: &str) -> IoResult<f32> {
    value
        .parse::<f32>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or_else(|| IoError::parse_msg(format!("Invalid assembly matrix number '{value}'")))
}

fn parse_chains(value: &str, pdb: bool) -> IoResult<Vec<String>> {
    let mut chains = Vec::new();
    for chain in value.split(',') {
        let chain = chain.trim();
        if chain.is_empty() {
            if pdb {
                continue;
            }
            return Err(IoError::parse_msg("Empty assembly chain identifier"));
        }
        let chain = if pdb && chain == "NULL" { "" } else { chain };
        if !chains.iter().any(|s| s == chain) {
            chains.push(chain.to_owned());
        }
    }
    if chains.is_empty() {
        return Err(IoError::parse_msg("Assembly has no chain identifiers"));
    }
    Ok(chains)
}

fn expression_factor(value: &str) -> IoResult<Vec<String>> {
    let mut ids = Vec::new();
    for term in value.split(',') {
        if term.is_empty() || term.contains(['(', ')']) {
            return Err(IoError::parse_msg(
                "Malformed assembly operation expression",
            ));
        }
        if let Some((first, last)) = term.split_once('-') {
            let first = first
                .parse::<u32>()
                .map_err(|_| IoError::parse_msg("Invalid operator range"))?;
            let last = last
                .parse::<u32>()
                .map_err(|_| IoError::parse_msg("Invalid operator range"))?;
            let count = last
                .checked_sub(first)
                .map(|n| u64::from(n) + 1)
                .ok_or_else(|| IoError::parse_msg("Descending operator range"))?;
            if count > MAX_INSTANCE_COUNT.saturating_sub(ids.len()) as u64 {
                return Err(IoError::parse_msg(
                    "Assembly expression exceeds 65535 copies",
                ));
            }
            ids.extend((first..=last).map(|n| n.to_string()));
        } else {
            if !term.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                return Err(IoError::parse_msg("Invalid assembly operator identifier"));
            }
            if ids.len() == MAX_INSTANCE_COUNT {
                return Err(IoError::parse_msg(
                    "Assembly expression exceeds 65535 copies",
                ));
            }
            ids.push(term.to_owned());
        }
    }
    Ok(ids)
}

fn resolve_expression(
    expression: &str,
    operators: &BTreeMap<String, InstanceTransform>,
) -> IoResult<Vec<InstanceTransform>> {
    let expression: String = expression.chars().filter(|c| !c.is_whitespace()).collect();
    let mut rest = expression.as_str();
    let mut result = vec![IDENTITY_INSTANCE];
    let parenthesized = rest.starts_with('(');
    loop {
        let factor = if parenthesized {
            rest = rest
                .strip_prefix('(')
                .ok_or_else(|| IoError::parse_msg("Malformed assembly operation product"))?;
            let (factor, tail) = rest
                .split_once(')')
                .ok_or_else(|| IoError::parse_msg("Unclosed assembly operation factor"))?;
            rest = tail;
            factor
        } else {
            let factor = rest;
            rest = "";
            factor
        };
        let ids = expression_factor(factor)?;
        if ids.len() > MAX_INSTANCE_COUNT / result.len() {
            return Err(IoError::parse_msg(
                "Assembly expression exceeds 65535 copies",
            ));
        }
        let mut expanded = Vec::with_capacity(result.len() * ids.len());
        for left in &result {
            for id in &ids {
                let right = operators.get(id).ok_or_else(|| {
                    IoError::parse_msg(format!("Unknown assembly operator '{id}'"))
                })?;
                // CIF products apply their rightmost operator first.
                expanded.push(multiply_instance_transforms(left, right));
            }
        }
        result = expanded;
        if rest.is_empty() {
            return Ok(result);
        }
    }
}

#[derive(Default)]
pub(crate) struct PdbAssemblies {
    definitions: Vec<AssemblyDefinition>,
    active: Vec<String>,
    chains: Vec<String>,
    operations: BTreeMap<String, (InstanceTransform, u8)>,
}

impl PdbAssemblies {
    pub(crate) fn line(&mut self, line: &str) -> IoResult<()> {
        let Some(text) = line.strip_prefix("REMARK 350") else {
            return Ok(());
        };
        let text = text.trim();
        if let Some(ids) = text.strip_prefix("BIOMOLECULE:") {
            self.flush()?;
            self.active = parse_chains(ids, false)?;
            self.chains.clear();
            for id in &self.active {
                if !self.definitions.iter().any(|d| &d.id == id) {
                    self.definitions.push(AssemblyDefinition {
                        id: id.clone(),
                        ..Default::default()
                    });
                }
            }
        } else if let Some(chains) = text.strip_prefix("APPLY THE FOLLOWING TO CHAINS:") {
            self.flush()?;
            self.chains = parse_chains(chains, true)?;
        } else if let Some(chains) = text.strip_prefix("AND CHAINS:") {
            if !self.operations.is_empty() {
                return Err(IoError::parse_msg(
                    "PDB chain continuation follows BIOMT rows",
                ));
            }
            self.chains.extend(parse_chains(chains, true)?);
        } else if text.starts_with("BIOMT") {
            let fields: Vec<_> = text.split_whitespace().collect();
            let row = match fields.first().copied() {
                Some("BIOMT1") => 0,
                Some("BIOMT2") => 1,
                Some("BIOMT3") => 2,
                _ => return Err(IoError::parse_msg("Invalid PDB BIOMT row")),
            };
            if fields.len() != 6 || self.active.is_empty() || self.chains.is_empty() {
                return Err(IoError::parse_msg("Incomplete PDB BIOMT context or row"));
            }
            if !self.operations.contains_key(fields[1])
                && self.operations.len() == MAX_INSTANCE_COUNT
            {
                return Err(IoError::parse_msg("PDB assembly exceeds 65535 copies"));
            }
            let (matrix, mask) = self
                .operations
                .entry(fields[1].to_owned())
                .or_insert((IDENTITY_INSTANCE, 0));
            if *mask & (1 << row) != 0 {
                return Err(IoError::parse_msg("Duplicate PDB BIOMT row"));
            }
            for (column, value) in matrix.iter_mut().zip(&fields[2..]) {
                column[row] = parse_number(value)?;
            }
            *mask |= 1 << row;
        } else if text.contains("BIOLOGICAL UNIT:") {
            for definition in &mut self.definitions {
                if self.active.contains(&definition.id) {
                    if !definition.details.is_empty() {
                        definition.details.push(' ');
                    }
                    definition.details.push_str(text);
                }
            }
        }
        Ok(())
    }

    fn flush(&mut self) -> IoResult<()> {
        if self.operations.is_empty() {
            return Ok(());
        }
        if self.operations.values().any(|(_, mask)| *mask != 7) {
            return Err(IoError::parse_msg("Incomplete PDB BIOMT matrix"));
        }
        let transforms: Vec<_> = self
            .operations
            .values()
            .map(|(matrix, _)| *matrix)
            .collect();
        for definition in &mut self.definitions {
            if self.active.contains(&definition.id) {
                let total = definition
                    .groups
                    .iter()
                    .map(|g| g.transforms.len())
                    .sum::<usize>();
                if transforms.len() > MAX_INSTANCE_COUNT.saturating_sub(total) {
                    return Err(IoError::parse_msg("PDB assembly exceeds 65535 copies"));
                }
                definition.groups.push(AssemblyGroup {
                    chains: self.chains.clone(),
                    transforms: transforms.clone(),
                });
            }
        }
        self.operations.clear();
        Ok(())
    }

    pub(crate) fn finish(mut self) -> IoResult<Vec<AssemblyDefinition>> {
        self.flush()?;
        Ok(self.definitions)
    }
}

#[cfg(test)]
mod tests {
    use lin_alg::f32::Vec3;
    use patinae_mol::{transform_instance_point, AtomIndex, ObjectMolecule};

    use super::*;
    use crate::traits::MoleculeReader;

    const CIF_ATOMS: &str = "loop_\n_atom_site.id\n_atom_site.type_symbol\n_atom_site.label_atom_id\n_atom_site.label_comp_id\n_atom_site.label_asym_id\n_atom_site.auth_asym_id\n_atom_site.label_seq_id\n_atom_site.Cartn_x\n_atom_site.Cartn_y\n_atom_site.Cartn_z\n_atom_site.pdbx_PDB_model_num\n1 C CA GLY L-1 A 1 1 0 0 1\n2 C CA GLY L-2 A 2 4 0 0 1\n";
    const CIF_ASSEMBLY: &str = "_pdbx_struct_assembly.id 1\n_pdbx_struct_assembly.details 'test dimer'\n_pdbx_struct_assembly_gen.assembly_id 1\n_pdbx_struct_assembly_gen.asym_id_list L-1\n_pdbx_struct_assembly_gen.oper_expression '(1)(2)'\nloop_\n_pdbx_struct_oper_list.id\n_pdbx_struct_oper_list.matrix[1][1]\n_pdbx_struct_oper_list.matrix[1][2]\n_pdbx_struct_oper_list.matrix[1][3]\n_pdbx_struct_oper_list.vector[1]\n_pdbx_struct_oper_list.matrix[2][1]\n_pdbx_struct_oper_list.matrix[2][2]\n_pdbx_struct_oper_list.matrix[2][3]\n_pdbx_struct_oper_list.vector[2]\n_pdbx_struct_oper_list.matrix[3][1]\n_pdbx_struct_oper_list.matrix[3][2]\n_pdbx_struct_oper_list.matrix[3][3]\n_pdbx_struct_oper_list.vector[3]\n1 0 -1 0 0 1 0 0 0 0 0 1 0\n2 1 0 0 10 0 1 0 0 0 0 1 0\n";

    fn cif(text: &str) -> IoResult<Vec<ObjectMolecule>> {
        crate::cif::CifReader::new(text.as_bytes()).read_all()
    }

    fn assert_composed_copy(molecule: &ObjectMolecule) {
        assert_eq!(molecule.atom_count(), 2);
        assert_eq!(molecule.assembly.chains["L-1"], vec![0]);
        let table = molecule
            .assembly
            .instance_table("1", molecule.atom_count())
            .unwrap();
        assert_eq!(table.groups[0].indices, vec![0]);
        assert_eq!(table.copies.len(), 1);
        let source = molecule.get_coord(AtomIndex(0), 0).unwrap();
        let point = transform_instance_point(&table.copies[0].transform, source);
        assert_eq!((point.x, point.y, point.z), (0.0, 11.0, 0.0));
        assert_eq!(source.x, 1.0);
    }

    #[test]
    fn cif_assembly_preserves_label_membership_and_product_order() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("assembly.cif");
        std::fs::write(&path, format!("data_assembly\n{CIF_ATOMS}{CIF_ASSEMBLY}")).unwrap();
        let molecule = crate::cif::read_cif(&path).unwrap();
        assert_composed_copy(&molecule);
        assert_eq!(molecule.assembly.definitions[0].details, "test dimer");
        assert_eq!(molecule.get_atom(AtomIndex(0)).unwrap().residue.chain, "A");
    }

    #[test]
    fn cif_assembly_categories_can_surround_atoms_and_unrelated_loops() {
        let (singles, operators) = CIF_ASSEMBLY.split_once("loop_\n").unwrap();
        let unrelated = "loop_\n_audit.id\n_audit.details\n1 ignored\n";
        for body in [
            format!("{CIF_ASSEMBLY}{CIF_ATOMS}"),
            format!("loop_\n{operators}{CIF_ATOMS}{unrelated}{singles}"),
            format!("{singles}{CIF_ATOMS}{unrelated}loop_\n{operators}"),
        ] {
            let molecules = cif(&format!("data_order\n{body}")).unwrap();
            assert_composed_copy(&molecules[0]);
            assert_eq!(molecules[0].assembly.definitions[0].details, "test dimer");
        }
    }

    #[test]
    fn cif_incomplete_assembly_loop_keeps_error_and_empty_block_behavior() {
        let incomplete = "loop_\n_pdbx_struct_assembly.id\n_pdbx_struct_assembly.details\n1\n";
        for body in [
            format!("{CIF_ATOMS}{incomplete}"),
            format!("{incomplete}{CIF_ATOMS}"),
        ] {
            let error = cif(&format!("data_bad\n{body}")).err().unwrap();
            assert!(error.to_string().contains("Incomplete assembly loop row"));
        }
        // Metadata-only blocks are ignored by molecule loading, as before.
        let molecules = cif(&format!(
            "data_metadata\n{incomplete}data_valid\n{CIF_ATOMS}{CIF_ASSEMBLY}"
        ))
        .unwrap();
        assert_eq!(molecules.len(), 1);
        assert_composed_copy(&molecules[0]);
    }

    #[test]
    fn cif_interned_chains_keep_names_and_atom_order_across_models() {
        let atoms = format!(
            "{CIF_ATOMS}3 C CA GLY L-1 A 1 2 0 0 2\n4 C CA GLY L-2 A 2 5 0 0 2\n5 C CA GLY L-2 A 1 3 0 0 3\n6 C CA GLY L-1 A 2 6 0 0 3\n"
        );
        let molecules = cif(&format!("data_chains\n{atoms}{CIF_ASSEMBLY}")).unwrap();
        assert_eq!(molecules.len(), 2);
        assert_eq!(molecules[0].state_count(), 2);
        assert_eq!(molecules[1].state_count(), 1);
        assert_eq!(molecules[0].assembly.chains["L-1"], vec![0]);
        assert_eq!(molecules[0].assembly.chains["L-2"], vec![1]);
        assert_eq!(molecules[1].assembly.chains["L-1"], vec![1]);
        assert_eq!(molecules[1].assembly.chains["L-2"], vec![0]);
    }

    #[test]
    fn cif_single_operator_category_is_supported() {
        let mut single = String::from("data_single\n");
        single.push_str(CIF_ATOMS);
        single.push_str("_pdbx_struct_assembly.id 1\n_pdbx_struct_assembly_gen.assembly_id 1\n_pdbx_struct_assembly_gen.asym_id_list L-1\n_pdbx_struct_assembly_gen.oper_expression 1\n_pdbx_struct_oper_list.id 1\n");
        for r in 0..3 {
            for c in 0..3 {
                single.push_str(&format!(
                    "_pdbx_struct_oper_list.matrix[{}][{}] {}\n",
                    r + 1,
                    c + 1,
                    u8::from(r == c)
                ));
            }
            single.push_str(&format!("_pdbx_struct_oper_list.vector[{}] 0\n", r + 1));
        }
        let molecules = cif(&single).unwrap();
        let table = molecules[0].assembly.instance_table("1", 2).unwrap();
        assert_eq!(table.copies[0].transform, IDENTITY_INSTANCE);
    }

    #[test]
    fn assembly_membership_is_specific_to_each_logical_model() {
        let atoms = CIF_ATOMS.replace(
            "2 C CA GLY L-2 A 2 4 0 0 1",
            "2 C CA GLY L-2 A 2 4 0 0 1\n3 C CA GLY L-2 A 1 1 0 0 2\n4 C CA GLY L-2 A 2 4 0 0 2",
        );
        let molecules = cif(&format!("data_models\n{atoms}{CIF_ASSEMBLY}")).unwrap();
        assert_eq!(molecules.len(), 2);
        assert!(molecules[0].assembly.instance_table("1", 2).is_ok());
        assert!(molecules[1].assembly.instance_table("1", 2).is_err());
        assert_eq!(molecules[1].assembly.definitions.len(), 1);
        // Without assemblies, existing author-based topology grouping is preserved.
        let plain = cif(&format!("data_models\n{atoms}")).unwrap();
        assert_eq!(plain.len(), 1);
        assert_eq!(plain[0].state_count(), 2);
        assert!(plain[0].assembly.chains.is_empty());
    }

    #[test]
    fn malformed_assembly_operators_do_not_become_identity() {
        for expression in [
            "9",
            "(1",
            "1,",
            "(1)junk",
            "2-1",
            "1-4294967295",
            "(1-300)(1-300)",
        ] {
            let text = format!(
                "data_bad\n{CIF_ATOMS}{}",
                CIF_ASSEMBLY.replace("(1)(2)", expression)
            );
            assert!(cif(&text).is_err(), "accepted {expression}");
        }
        let text = format!(
            "data_bad\n{CIF_ATOMS}{}",
            CIF_ASSEMBLY.replace("1 0 -1 0 0", "1 NaN -1 0 0")
        );
        assert!(cif(&text).is_err());
    }

    #[test]
    fn comma_ranges_and_cartesian_products_expand_in_order() {
        let mut translation = IDENTITY_INSTANCE;
        translation[3][0] = 10.0;
        let rotation = [
            [0.0, 1.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let operators = BTreeMap::from([
            ("1".to_owned(), rotation),
            ("2".to_owned(), IDENTITY_INSTANCE),
            ("3".to_owned(), translation),
            ("4".to_owned(), IDENTITY_INSTANCE),
        ]);
        let transforms = resolve_expression("(1-2)(3,4)", &operators).unwrap();
        assert_eq!(transforms.len(), 4);
        let coordinates: Vec<_> = transforms
            .iter()
            .map(|m| {
                let p = transform_instance_point(m, Vec3::new(1.0, 0.0, 0.0));
                (p.x, p.y)
            })
            .collect();
        assert_eq!(
            coordinates,
            vec![(0.0, 11.0), (0.0, 1.0), (11.0, 0.0), (1.0, 0.0)]
        );
    }

    #[test]
    fn pdb_biomt_chain_continuation_and_original_chain_segments() {
        let pdb = "REMARK 350 BIOMOLECULE: 1\nREMARK 350 APPLY THE FOLLOWING TO CHAINS: A,\nREMARK 350                    AND CHAINS: B\nREMARK 350   BIOMT1   1  1.000000 0.000000 0.000000 10.00000\nREMARK 350   BIOMT2   1  0.000000 1.000000 0.000000  0.00000\nREMARK 350   BIOMT3   1  0.000000 0.000000 1.000000  0.00000\nATOM      1  CA  GLY A   1       1.000   0.000   0.000  1.00  0.00           C\nTER\nATOM      2  CA  GLY A   2       4.000   0.000   0.000  1.00  0.00           C\nATOM      3  CA  GLY B   1       7.000   0.000   0.000  1.00  0.00           C\nEND\n";
        let molecule = crate::pdb::PdbReader::new(pdb.as_bytes()).read().unwrap();
        assert_eq!(molecule.atom_count(), 3);
        assert_eq!(molecule.assembly.chains["A"], vec![0, 1]);
        assert_eq!(molecule.assembly.chains["B"], vec![2]);
        let table = molecule.assembly.instance_table("1", 3).unwrap();
        assert_eq!(table.displayed_count(3).unwrap(), 3);
        let point = transform_instance_point(
            &table.copies[0].transform,
            molecule.get_coord(AtomIndex(0), 0).unwrap(),
        );
        assert_eq!(point.x, 11.0);
        let incomplete = pdb.replace(
            "REMARK 350   BIOMT3   1  0.000000 0.000000 1.000000  0.00000\n",
            "",
        );
        assert!(crate::pdb::PdbReader::new(incomplete.as_bytes())
            .read()
            .is_err());
    }

    #[test]
    fn bcif_assembly_uses_label_chains_and_decodes_numeric_matrices() {
        use crate::bcif::test_support::{
            atom_row, atom_site_block, encode_bcif_file, float_column, string_column,
        };
        use crate::bcif::types::BcifCategory;

        let mut block = atom_site_block(
            "assembly",
            &[
                atom_row(1, "CA", "GLY", "L-1", 1.0, 1),
                atom_row(2, "CA", "GLY", "L-2", 4.0, 1),
            ],
        );
        block.categories[0]
            .columns
            .push(string_column("auth_asym_id", ["A", "A"].into_iter()));
        block.categories.push(BcifCategory {
            name: "_pdbx_struct_assembly".to_owned(),
            row_count: 1,
            columns: vec![string_column("id", ["1"].into_iter())],
        });
        block.categories.push(BcifCategory {
            name: "_pdbx_struct_assembly_gen".to_owned(),
            row_count: 1,
            columns: vec![
                string_column("assembly_id", ["1"].into_iter()),
                string_column("asym_id_list", ["L-1"].into_iter()),
                string_column("oper_expression", ["(1)(2)"].into_iter()),
            ],
        });
        let matrices = [
            [
                [0.0, 1.0, 0.0, 0.0],
                [-1.0, 0.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [10.0, 0.0, 0.0, 1.0],
            ],
        ];
        let mut columns = vec![string_column("id", ["1", "2"].into_iter())];
        for r in 0..3 {
            for c in 0..4 {
                let name = if c == 3 {
                    format!("vector[{}]", r + 1)
                } else {
                    format!("matrix[{}][{}]", r + 1, c + 1)
                };
                columns.push(float_column(&name, matrices.iter().map(|m| m[c][r])));
            }
        }
        block.categories.push(BcifCategory {
            name: "_pdbx_struct_oper_list".to_owned(),
            row_count: 2,
            columns,
        });
        let bytes = encode_bcif_file(&[block]);
        let molecule = crate::bcif::read_bcif_bytes(&bytes).unwrap();
        assert_composed_copy(&molecule);
    }
}
