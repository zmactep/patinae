//! PDB file format parser and writer
//!
//! Provides reading and writing of Protein Data Bank (PDB) format files.

mod chain_ids;
pub mod hybrid36;
mod parser;
mod records;
mod writer;

pub use parser::PdbReader;
pub use records::*;
pub use writer::PdbWriter;

use std::io::Read;
use std::path::Path;

use patinae_mol::ObjectMolecule;

use crate::error::IoResult;
use crate::traits::MoleculeReader;

/// Read a PDB file from a path
pub fn read_pdb(path: &Path) -> IoResult<ObjectMolecule> {
    read_pdb_with_bond_tolerance(path, patinae_mol::DEFAULT_BOND_TOLERANCE)
}

pub fn read_pdb_with_bond_tolerance(path: &Path, bond_tolerance: f32) -> IoResult<ObjectMolecule> {
    let file = crate::compress::open_file(path)?;
    let mut reader = PdbReader::new(file).with_bond_tolerance(bond_tolerance);
    reader.read()
}

pub(crate) fn read_all_pdb_with_bond_tolerance(
    path: &Path,
    bond_tolerance: f32,
) -> IoResult<Vec<ObjectMolecule>> {
    let file = crate::compress::open_file(path)?;
    let mut reader = PdbReader::new(file).with_bond_tolerance(bond_tolerance);
    reader.read_all()
}

/// Read a PDB file from a string
pub fn read_pdb_str(content: &str) -> IoResult<ObjectMolecule> {
    let mut reader =
        PdbReader::new(content.as_bytes()).with_bond_tolerance(patinae_mol::DEFAULT_BOND_TOLERANCE);
    reader.read()
}

pub fn read_pdb_str_with_bond_tolerance(
    content: &str,
    bond_tolerance: f32,
) -> IoResult<ObjectMolecule> {
    let mut reader = PdbReader::new(content.as_bytes()).with_bond_tolerance(bond_tolerance);
    reader.read()
}

/// Read a PDB file from a reader
pub fn read_pdb_from<R: Read>(reader: R) -> IoResult<ObjectMolecule> {
    let mut reader =
        PdbReader::new(reader).with_bond_tolerance(patinae_mol::DEFAULT_BOND_TOLERANCE);
    reader.read()
}

pub fn read_pdb_from_with_bond_tolerance<R: Read>(
    reader: R,
    bond_tolerance: f32,
) -> IoResult<ObjectMolecule> {
    let mut reader = PdbReader::new(reader).with_bond_tolerance(bond_tolerance);
    reader.read()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO_CARBONS: &str = "\
ATOM      1  C1  LIG A   1       0.000   0.000   0.000  1.00  0.00           C
ATOM      2  C2  LIG A   1       1.850   0.000   0.000  1.00  0.00           C
END
";

    #[test]
    fn bond_tolerance_changes_generated_bond_count() {
        let loose = read_pdb_str_with_bond_tolerance(TWO_CARBONS, 0.45).unwrap();
        let strict = read_pdb_str_with_bond_tolerance(TWO_CARBONS, 0.1).unwrap();

        assert_eq!(loose.bond_count(), 1);
        assert_eq!(strict.bond_count(), 0);
    }
}
