use serde::Serialize;

use super::types::{BcifCategory, BcifColumn, BcifData, BcifDataBlock, BcifEncoding, BcifFile};

const TYPE_INT32: i32 = 3;
const TYPE_FLOAT32: i32 = 32;

#[derive(Clone)]
pub(crate) struct AtomRow {
    pub(crate) group: &'static str,
    pub(crate) id: i32,
    pub(crate) element: &'static str,
    pub(crate) atom: &'static str,
    pub(crate) resn: &'static str,
    pub(crate) chain: &'static str,
    pub(crate) resv: i32,
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) z: f32,
    pub(crate) model: i32,
}

pub(crate) fn atom_row(
    id: i32,
    atom: &'static str,
    resn: &'static str,
    chain: &'static str,
    x: f32,
    model: i32,
) -> AtomRow {
    AtomRow {
        group: "ATOM",
        id,
        element: atom.chars().next().map_or("X", element_from_atom_name),
        atom,
        resn,
        chain,
        resv: 1,
        x,
        y: 0.0,
        z: 0.0,
        model,
    }
}

fn element_from_atom_name(ch: char) -> &'static str {
    match ch {
        'C' => "C",
        'N' => "N",
        'O' => "O",
        'H' => "H",
        _ => "X",
    }
}

pub(crate) fn atom_site_block(header: &str, rows: &[AtomRow]) -> BcifDataBlock {
    let columns = vec![
        string_column("group_PDB", rows.iter().map(|row| row.group)),
        int_column("id", rows.iter().map(|row| row.id)),
        string_column("type_symbol", rows.iter().map(|row| row.element)),
        string_column("label_atom_id", rows.iter().map(|row| row.atom)),
        string_column("label_comp_id", rows.iter().map(|row| row.resn)),
        string_column("label_asym_id", rows.iter().map(|row| row.chain)),
        int_column("label_seq_id", rows.iter().map(|row| row.resv)),
        float_column("Cartn_x", rows.iter().map(|row| row.x)),
        float_column("Cartn_y", rows.iter().map(|row| row.y)),
        float_column("Cartn_z", rows.iter().map(|row| row.z)),
        int_column("pdbx_PDB_model_num", rows.iter().map(|row| row.model)),
    ];

    BcifDataBlock {
        header: header.to_string(),
        categories: vec![BcifCategory {
            name: "_atom_site".to_string(),
            row_count: rows.len() as u32,
            columns,
        }],
    }
}

pub(crate) fn bcif_file(blocks: Vec<BcifDataBlock>) -> BcifFile {
    BcifFile {
        version: "0.3".to_string(),
        encoder: "patinae-io tests".to_string(),
        data_blocks: blocks,
    }
}

pub(crate) fn encode_bcif_file(blocks: &[BcifDataBlock]) -> Vec<u8> {
    let file = TestBcifFile {
        version: "0.3".to_string(),
        encoder: "patinae-io tests".to_string(),
        data_blocks: blocks.iter().map(TestBcifDataBlock::from).collect(),
    };
    rmp_serde::to_vec_named(&file).expect("test bCIF fixture should serialize")
}

fn int_column(name: &str, values: impl Iterator<Item = i32>) -> BcifColumn {
    BcifColumn {
        name: name.to_string(),
        data: BcifData {
            data: i32_bytes(values),
            encoding: vec![BcifEncoding::ByteArray {
                data_type: TYPE_INT32,
            }],
        },
        mask: None,
    }
}

pub(crate) fn float_column(name: &str, values: impl Iterator<Item = f32>) -> BcifColumn {
    BcifColumn {
        name: name.to_string(),
        data: BcifData {
            data: f32_bytes(values),
            encoding: vec![BcifEncoding::ByteArray {
                data_type: TYPE_FLOAT32,
            }],
        },
        mask: None,
    }
}

pub(crate) fn string_column<'a>(name: &str, values: impl Iterator<Item = &'a str>) -> BcifColumn {
    let mut table: Vec<&'a str> = Vec::new();
    let mut indices = Vec::new();

    for value in values {
        let idx = table
            .iter()
            .position(|known| *known == value)
            .unwrap_or_else(|| {
                table.push(value);
                table.len() - 1
            });
        indices.push(idx as i32);
    }

    let mut string_data = String::new();
    let mut offsets = Vec::with_capacity(table.len() + 1);
    offsets.push(0);
    for value in table {
        string_data.push_str(value);
        offsets.push(string_data.len() as i32);
    }

    BcifColumn {
        name: name.to_string(),
        data: BcifData {
            data: i32_bytes(indices.into_iter()),
            encoding: vec![BcifEncoding::StringArray {
                data_encoding: vec![BcifEncoding::ByteArray {
                    data_type: TYPE_INT32,
                }],
                string_data,
                offset_encoding: vec![BcifEncoding::ByteArray {
                    data_type: TYPE_INT32,
                }],
                offsets: i32_bytes(offsets.into_iter()),
            }],
        },
        mask: None,
    }
}

fn i32_bytes(values: impl Iterator<Item = i32>) -> Vec<u8> {
    values.flat_map(i32::to_le_bytes).collect()
}

fn f32_bytes(values: impl Iterator<Item = f32>) -> Vec<u8> {
    values.flat_map(f32::to_le_bytes).collect()
}

#[derive(Serialize)]
struct TestBcifFile {
    version: String,
    encoder: String,
    #[serde(rename = "dataBlocks")]
    data_blocks: Vec<TestBcifDataBlock>,
}

#[derive(Serialize)]
struct TestBcifDataBlock {
    header: String,
    categories: Vec<TestBcifCategory>,
}

impl From<&BcifDataBlock> for TestBcifDataBlock {
    fn from(block: &BcifDataBlock) -> Self {
        Self {
            header: block.header.clone(),
            categories: block
                .categories
                .iter()
                .map(TestBcifCategory::from)
                .collect(),
        }
    }
}

#[derive(Serialize)]
struct TestBcifCategory {
    name: String,
    #[serde(rename = "rowCount")]
    row_count: u32,
    columns: Vec<TestBcifColumn>,
}

impl From<&BcifCategory> for TestBcifCategory {
    fn from(category: &BcifCategory) -> Self {
        Self {
            name: category.name.clone(),
            row_count: category.row_count,
            columns: category.columns.iter().map(TestBcifColumn::from).collect(),
        }
    }
}

#[derive(Serialize)]
struct TestBcifColumn {
    name: String,
    data: TestBcifData,
    mask: Option<TestBcifData>,
}

impl From<&BcifColumn> for TestBcifColumn {
    fn from(column: &BcifColumn) -> Self {
        Self {
            name: column.name.clone(),
            data: TestBcifData::from(&column.data),
            mask: column.mask.as_ref().map(TestBcifData::from),
        }
    }
}

#[derive(Serialize)]
struct TestBcifData {
    data: Vec<u8>,
    encoding: Vec<TestBcifEncoding>,
}

impl From<&BcifData> for TestBcifData {
    fn from(data: &BcifData) -> Self {
        Self {
            data: data.data.clone(),
            encoding: data.encoding.iter().map(TestBcifEncoding::from).collect(),
        }
    }
}

#[derive(Serialize)]
#[serde(untagged)]
enum TestBcifEncoding {
    ByteArray {
        kind: &'static str,
        #[serde(rename = "type")]
        data_type: i32,
    },
    StringArray {
        kind: &'static str,
        #[serde(rename = "dataEncoding")]
        data_encoding: Vec<TestBcifEncoding>,
        #[serde(rename = "stringData")]
        string_data: String,
        #[serde(rename = "offsetEncoding")]
        offset_encoding: Vec<TestBcifEncoding>,
        offsets: Vec<u8>,
    },
}

impl From<&BcifEncoding> for TestBcifEncoding {
    fn from(encoding: &BcifEncoding) -> Self {
        match encoding {
            BcifEncoding::ByteArray { data_type } => TestBcifEncoding::ByteArray {
                kind: "ByteArray",
                data_type: *data_type,
            },
            BcifEncoding::StringArray {
                data_encoding,
                string_data,
                offset_encoding,
                offsets,
            } => TestBcifEncoding::StringArray {
                kind: "StringArray",
                data_encoding: data_encoding.iter().map(TestBcifEncoding::from).collect(),
                string_data: string_data.clone(),
                offset_encoding: offset_encoding.iter().map(TestBcifEncoding::from).collect(),
                offsets: offsets.clone(),
            },
            _ => panic!("test fixtures only encode ByteArray and StringArray"),
        }
    }
}
