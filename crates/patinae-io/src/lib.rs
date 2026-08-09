//! Molecular file format I/O
//!
//! This crate provides parsers and writers for various molecular file formats:
//!
//! - **PDB** - Protein Data Bank format
//! - **SDF/MOL** - MDL Structure Data File / MOL V2000 format
//! - **MOL2** - TRIPOS MOL2 format
//! - **XYZ** - Simple XYZ coordinate format
//! - **mmCIF** - Macromolecular Crystallographic Information File
//!
//! # Quick Start
//!
//! ```no_run
//! use patinae_io::{read_file, write_file, FileFormat};
//! use std::path::Path;
//!
//! // Read a PDB file
//! let mol = patinae_io::pdb::read_pdb(Path::new("protein.pdb")).unwrap();
//!
//! // Write to a different format
//! // patinae_io::sdf::write_sdf(Path::new("protein.sdf"), &mol).unwrap();
//! ```
//!
//! # Fetching from RCSB PDB
//!
//! With the `fetch` or `fetch-async` feature enabled, you can download structures
//! directly from the RCSB Protein Data Bank:
//!
//! ```no_run
//! # #[cfg(feature = "fetch")]
//! # fn main() -> patinae_io::IoResult<()> {
//! use patinae_io::fetch::{fetch, FetchFormat};
//!
//! // Fetch ubiquitin in mmCIF format
//! let mol = fetch("1ubq", FetchFormat::default())?;
//! # Ok(())
//! # }
//! # #[cfg(not(feature = "fetch"))]
//! # fn main() {}
//! ```
//!
//! # Format-specific modules
//!
//! Each format has its own module with specialized readers and writers:
//!
//! - [`pdb`] - PDB format with ATOM/HETATM/CONECT records
//! - [`sdf`] - SDF/MOL format with connection tables
//! - [`mol2`] - MOL2 format with SYBYL atom types and charges
//! - [`xyz`] - Simple XYZ format (coordinates only)
//! - [`cif`] - mmCIF format with STAR syntax
//!
//! # Features
//!
//! - `fetch` - Enable synchronous fetching from RCSB PDB (uses `ureq`)
//! - `fetch-async` - Enable asynchronous fetching from RCSB PDB (uses `reqwest`)

pub mod bcif;
pub mod ccp4;
pub mod cif;
pub mod compress;
pub mod detect;
pub mod error;
pub mod gro;
mod logical_models;
pub mod mol2;
pub mod pdb;
pub mod sdf;
pub mod traits;
#[cfg(feature = "traj")]
pub mod traj;
pub(crate) mod units;
pub mod xyz;

// Fetch module (requires feature)
#[cfg(any(feature = "fetch", feature = "fetch-async"))]
pub mod fetch;

// Re-exports
pub use error::{IoError, IoResult};
#[cfg(feature = "traj")]
pub use traits::create_trajectory_reader;
pub use traits::{
    create_reader, create_writer, FileFormat, MoleculeReader, MoleculeWriter, ReadOptions,
    TrajectoryReadOptions, TrajectoryReader, WriteOptions,
};

// Fetch re-exports
#[cfg(any(feature = "fetch", feature = "fetch-async"))]
pub use fetch::FetchFormat;
#[cfg(feature = "fetch")]
pub use fetch::{fetch, fetch_with_bond_tolerance};
#[cfg(feature = "fetch-async")]
pub use fetch::{
    fetch_async, fetch_async_with_bond_tolerance, fetch_pdb_metadata, PdbMetadataPreview,
};

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use patinae_mol::ObjectMolecule;

fn first_molecule(molecules: Vec<ObjectMolecule>) -> IoResult<ObjectMolecule> {
    molecules.into_iter().next().ok_or(IoError::empty_file())
}

/// Read a molecule from a file, auto-detecting the format
pub fn read_file(path: &Path) -> IoResult<ObjectMolecule> {
    let format = detect::detect_from_path(path);
    read_file_format(path, format)
}

pub fn read_file_with_bond_tolerance(path: &Path, bond_tolerance: f32) -> IoResult<ObjectMolecule> {
    let format = detect::detect_from_path(path);
    read_file_format_with_bond_tolerance(path, format, bond_tolerance)
}

/// Read a molecule from a file with a specific format
pub fn read_file_format(path: &Path, format: FileFormat) -> IoResult<ObjectMolecule> {
    read_file_format_with_bond_tolerance(path, format, patinae_mol::DEFAULT_BOND_TOLERANCE)
}

pub fn read_file_format_with_bond_tolerance(
    path: &Path,
    format: FileFormat,
    bond_tolerance: f32,
) -> IoResult<ObjectMolecule> {
    read_all_format_with_bond_tolerance(path, format, bond_tolerance).and_then(first_molecule)
}

/// Read all molecules from a file, auto-detecting the format.
pub fn read_all(path: &Path) -> IoResult<Vec<ObjectMolecule>> {
    read_all_with_bond_tolerance(path, patinae_mol::DEFAULT_BOND_TOLERANCE)
}

pub fn read_all_with_bond_tolerance(
    path: &Path,
    bond_tolerance: f32,
) -> IoResult<Vec<ObjectMolecule>> {
    let format = detect::detect_from_path(path);
    read_all_format_with_bond_tolerance(path, format, bond_tolerance)
}

/// Read all molecules from a file with a specific format.
pub fn read_all_format(path: &Path, format: FileFormat) -> IoResult<Vec<ObjectMolecule>> {
    read_all_format_with_bond_tolerance(path, format, patinae_mol::DEFAULT_BOND_TOLERANCE)
}

pub fn read_all_format_with_bond_tolerance(
    path: &Path,
    format: FileFormat,
    bond_tolerance: f32,
) -> IoResult<Vec<ObjectMolecule>> {
    match format {
        FileFormat::Pdb => pdb::read_all_pdb_with_bond_tolerance(path, bond_tolerance),
        FileFormat::Sdf => sdf::read_sdf(path),
        FileFormat::Mol2 => mol2::read_mol2(path),
        FileFormat::Xyz => xyz::read_xyz(path).map(|m| vec![m]),
        FileFormat::Cif => cif::read_all_cif_with_bond_tolerance(path, bond_tolerance),
        FileFormat::Bcif => bcif::read_all_bcif_with_bond_tolerance(path, bond_tolerance),
        FileFormat::Gro => gro::read_gro_with_bond_tolerance(path, bond_tolerance).map(|m| vec![m]),
        FileFormat::Xtc | FileFormat::Trr => Err(IoError::unsupported(
            "Trajectory-only format; use load_traj instead".to_string(),
        )),
        FileFormat::Ccp4 => Err(IoError::unsupported(
            "CCP4 is a map format; use ccp4::read_ccp4 instead".to_string(),
        )),
        FileFormat::Unknown => Err(IoError::unknown_format(path.to_string_lossy().into_owned())),
    }
}

/// Write a molecule to a file, auto-detecting the format from extension
pub fn write_file(path: &Path, mol: &ObjectMolecule) -> IoResult<()> {
    let format = detect::detect_from_path(path);
    write_file_format(path, mol, format)
}

fn write_path_format(
    path: &Path,
    format: FileFormat,
    write: impl FnOnce(&mut dyn MoleculeWriter) -> IoResult<()>,
) -> IoResult<()> {
    let file = File::create(path)?;
    let writer = BufWriter::new(file);
    let mut output = if compress::is_gzip_path(path) {
        compress::MaybeGzWriter::Gzip(compress::gzip_writer(writer))
    } else {
        compress::MaybeGzWriter::Plain(writer)
    };

    {
        let mut mol_writer = create_writer(&mut output, format)?;
        write(mol_writer.as_mut())?;
        mol_writer.flush()?;
    }

    let mut writer = output.finish()?;
    writer.flush()?;
    Ok(())
}

/// Write a molecule to a file with a specific format
pub fn write_file_format(path: &Path, mol: &ObjectMolecule, format: FileFormat) -> IoResult<()> {
    write_path_format(path, format, |writer| writer.write(mol))
}

/// Write multiple molecules to a file (for multi-molecule formats like SDF)
pub fn write_all(path: &Path, molecules: &[ObjectMolecule]) -> IoResult<()> {
    let format = detect::detect_from_path(path);
    write_all_format(path, molecules, format)
}

/// Write multiple molecules to a file with a specific format
pub fn write_all_format(
    path: &Path,
    molecules: &[ObjectMolecule],
    format: FileFormat,
) -> IoResult<()> {
    write_path_format(path, format, |writer| writer.write_all(molecules))
}

#[cfg(feature = "traj")]
/// Read trajectory frames from a file, auto-detecting the format
pub fn read_trajectory(
    path: &Path,
    opts: &TrajectoryReadOptions,
) -> IoResult<Vec<patinae_mol::CoordSet>> {
    let format = detect::detect_from_path(path);
    read_trajectory_format(path, format, opts)
}

#[cfg(feature = "traj")]
/// Read trajectory frames from a file with a specific format
pub fn read_trajectory_format(
    path: &Path,
    format: FileFormat,
    opts: &TrajectoryReadOptions,
) -> IoResult<Vec<patinae_mol::CoordSet>> {
    let file = compress::open_file(path)?;
    let mut reader = create_trajectory_reader(file, format)?;
    reader.read_frames(opts)
}

/// Parse a molecule from a string with the given format
pub fn parse_str(content: &str, format: FileFormat) -> IoResult<ObjectMolecule> {
    match format {
        FileFormat::Pdb => pdb::read_pdb_str(content),
        FileFormat::Sdf => sdf::read_sdf_str(content).and_then(first_molecule),
        FileFormat::Mol2 => mol2::read_mol2_str(content).and_then(first_molecule),
        FileFormat::Xyz => xyz::read_xyz_str(content),
        FileFormat::Cif => cif::read_cif_str(content),
        FileFormat::Bcif => Err(IoError::unsupported(
            "bCIF is a binary format; use read_file instead".to_string(),
        )),
        FileFormat::Gro => gro::read_gro_str(content),
        FileFormat::Xtc | FileFormat::Trr => Err(IoError::unsupported(
            "Trajectory formats are binary; use load_traj with a file path".to_string(),
        )),
        FileFormat::Ccp4 => Err(IoError::unsupported(
            "CCP4 is a binary map format; use ccp4::read_ccp4 with a file path".to_string(),
        )),
        FileFormat::Unknown => Err(IoError::unknown_format("string input".to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bcif::test_support::{atom_row, atom_site_block, encode_bcif_file};
    use flate2::read::GzDecoder;
    use lin_alg::f32::Vec3;
    use patinae_mol::{Atom, CoordSet, Element};
    use std::io::Read;
    use tempfile::tempdir;

    fn single_atom_molecule(name: &str, x: f32) -> ObjectMolecule {
        let mut mol = ObjectMolecule::new(name);
        mol.add_atom(Atom::new("C", Element::Carbon));
        mol.add_coord_set(CoordSet::from_vec3(&[Vec3::new(x, 0.0, 0.0)]));
        mol
    }

    fn write_sdf_fixture(path: &Path, molecules: &[ObjectMolecule]) {
        let mut output = Vec::new();
        {
            let mut writer = sdf::SdfWriter::new(&mut output);
            writer
                .write_all(molecules)
                .expect("SDF fixture should be writable");
        }
        std::fs::write(path, output).expect("SDF fixture should be written to disk");
    }

    fn write_mol2_fixture(path: &Path, molecules: &[ObjectMolecule]) {
        let mut output = Vec::new();
        {
            let mut writer = mol2::Mol2Writer::new(&mut output);
            writer
                .write_all(molecules)
                .expect("MOL2 fixture should be writable");
        }
        std::fs::write(path, output).expect("MOL2 fixture should be written to disk");
    }

    fn assert_gzip_file(path: &Path) -> String {
        let bytes = std::fs::read(path).expect("written file should be readable");
        assert_eq!(&bytes[..2], &[0x1f, 0x8b]);

        let mut text = String::new();
        GzDecoder::new(bytes.as_slice())
            .read_to_string(&mut text)
            .expect("gzip stream should be complete and decompressible");
        text
    }

    #[test]
    fn write_file_gzip_supports_all_builtin_aliases() {
        let dir = tempdir().expect("temporary directory should be created");
        let molecule = single_atom_molecule("mixed-case", 1.25);

        for suffix in [
            "pdb.gz", "ENT.GZ", "sdf.gz", "mol.gz", "sd.gz", "mol2.gz", "ml2.gz", "cif.gz",
            "mmcif.gz", "xyz.gz", "gro.gz",
        ] {
            let path = dir.path().join(format!("molecule.{suffix}"));

            write_file(&path, &molecule)
                .unwrap_or_else(|error| panic!("{suffix} should be writable: {error}"));

            assert_gzip_file(&path);
            let parsed = read_file(&path)
                .unwrap_or_else(|error| panic!("{suffix} should read back: {error}"));
            assert_eq!(parsed.atom_count(), 1, "suffix: {suffix}");
        }
    }

    #[test]
    fn write_file_format_gzip_keeps_explicit_format_authoritative() {
        let dir = tempdir().expect("temporary directory should be created");
        let path = dir.path().join("molecule.data.gz");
        let molecule = single_atom_molecule("explicit-xyz", 2.5);

        write_file_format(&path, &molecule, FileFormat::Xyz)
            .expect("explicit-format gzip XYZ should be written");

        let text = assert_gzip_file(&path);
        assert!(text.starts_with("1\nexplicit-xyz\n"));
        let parsed = read_file_format(&path, FileFormat::Xyz)
            .expect("explicit-format gzip XYZ should read back");
        assert_eq!(parsed.name, "explicit-xyz");
    }

    #[test]
    fn write_file_format_plain_output_stays_uncompressed() {
        let dir = tempdir().expect("temporary directory should be created");
        let path = dir.path().join("molecule.xyz");
        let molecule = single_atom_molecule("plain-xyz", 3.75);

        write_file_format(&path, &molecule, FileFormat::Xyz).expect("plain XYZ should be written");

        let bytes = std::fs::read(&path).expect("plain XYZ should be readable");
        assert_ne!(&bytes[..2], &[0x1f, 0x8b]);
        let parsed =
            read_file_format(&path, FileFormat::Xyz).expect("plain XYZ should remain readable");
        assert_eq!(parsed.name, "plain-xyz");
    }

    #[test]
    fn write_all_gzip_roundtrips_multiple_molecules() {
        let dir = tempdir().expect("temporary directory should be created");
        let path = dir.path().join("molecules.SD.GZ");
        let molecules = [
            single_atom_molecule("first", 0.0),
            single_atom_molecule("second", 1.0),
        ];

        write_all(&path, &molecules).expect("multi-record gzip SDF should be written");

        let text = assert_gzip_file(&path);
        assert_eq!(text.matches("$$$$").count(), 2);
        let parsed = read_all(&path).expect("multi-record gzip SDF should read back");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name, "first");
        assert_eq!(parsed[1].name, "second");
    }

    #[test]
    fn write_all_format_gzip_uses_explicit_format() {
        let dir = tempdir().expect("temporary directory should be created");
        let path = dir.path().join("molecules.data.gz");
        let molecules = [
            single_atom_molecule("first", 0.0),
            single_atom_molecule("second", 1.0),
        ];

        write_all_format(&path, &molecules, FileFormat::Sdf)
            .expect("explicit-format multi-record gzip SDF should be written");

        let text = assert_gzip_file(&path);
        assert_eq!(text.matches("$$$$").count(), 2);
        let parsed = read_all_format(&path, FileFormat::Sdf)
            .expect("explicit-format multi-record gzip SDF should read back");
        assert_eq!(parsed.len(), 2);
    }

    #[test]
    fn test_format_detection() {
        assert_eq!(
            FileFormat::from_path(Path::new("test.pdb")),
            FileFormat::Pdb
        );
        assert_eq!(
            FileFormat::from_path(Path::new("test.sdf")),
            FileFormat::Sdf
        );
        assert_eq!(
            FileFormat::from_path(Path::new("test.mol2")),
            FileFormat::Mol2
        );
        assert_eq!(
            FileFormat::from_path(Path::new("test.xyz")),
            FileFormat::Xyz
        );
        assert_eq!(
            FileFormat::from_path(Path::new("test.cif")),
            FileFormat::Cif
        );
    }

    #[test]
    fn test_gzip_format_detection() {
        assert_eq!(
            FileFormat::from_path(Path::new("test.pdb.gz")),
            FileFormat::Pdb
        );
        assert_eq!(
            FileFormat::from_path(Path::new("test.cif.gz")),
            FileFormat::Cif
        );
        assert_eq!(
            FileFormat::from_path(Path::new("test.sdf.gz")),
            FileFormat::Sdf
        );
        assert_eq!(
            FileFormat::from_path(Path::new("test.mol2.gz")),
            FileFormat::Mol2
        );
        assert_eq!(
            FileFormat::from_path(Path::new("test.xyz.gz")),
            FileFormat::Xyz
        );
        assert_eq!(
            FileFormat::from_path(Path::new("test.gro.gz")),
            FileFormat::Gro
        );
        assert_eq!(
            FileFormat::from_path(Path::new("test.ent.gz")),
            FileFormat::Pdb
        );
    }

    #[test]
    fn parse_str_empty_sdf_returns_empty_file() {
        let result = parse_str("", FileFormat::Sdf);

        assert!(matches!(result, Err(err) if err.is_empty_file()));
    }

    #[test]
    fn parse_str_empty_mol2_returns_empty_file() {
        let result = parse_str("", FileFormat::Mol2);

        assert!(matches!(result, Err(err) if err.is_empty_file()));
    }

    #[test]
    fn read_file_format_empty_sdf_returns_empty_file() {
        let dir = tempdir().expect("temporary directory should be created");
        let path = dir.path().join("empty.sdf");
        std::fs::write(&path, "").expect("empty SDF fixture should be writable");

        let result = read_file_format(&path, FileFormat::Sdf);

        assert!(matches!(result, Err(err) if err.is_empty_file()));
    }

    #[test]
    fn read_file_format_empty_mol2_returns_empty_file() {
        let dir = tempdir().expect("temporary directory should be created");
        let path = dir.path().join("empty.mol2");
        std::fs::write(&path, "").expect("empty MOL2 fixture should be writable");

        let result = read_file_format(&path, FileFormat::Mol2);

        assert!(matches!(result, Err(err) if err.is_empty_file()));
    }

    #[test]
    fn read_file_detects_empty_sdf_as_empty_file() {
        let dir = tempdir().expect("temporary directory should be created");
        let path = dir.path().join("empty.sdf");
        std::fs::write(&path, "").expect("empty SDF fixture should be writable");

        let result = read_file(&path);

        assert!(matches!(result, Err(err) if err.is_empty_file()));
    }

    #[test]
    fn read_file_detects_empty_mol2_as_empty_file() {
        let dir = tempdir().expect("temporary directory should be created");
        let path = dir.path().join("empty.mol2");
        std::fs::write(&path, "").expect("empty MOL2 fixture should be writable");

        let result = read_file(&path);

        assert!(matches!(result, Err(err) if err.is_empty_file()));
    }

    #[test]
    fn read_all_format_empty_sdf_returns_empty_file() {
        let dir = tempdir().expect("temporary directory should be created");
        let path = dir.path().join("empty.sdf");
        std::fs::write(&path, "").expect("empty SDF fixture should be writable");

        let result = read_all_format(&path, FileFormat::Sdf);

        assert!(matches!(result, Err(err) if err.is_empty_file()));
    }

    #[test]
    fn read_all_with_bond_tolerance_empty_sdf_returns_empty_file() {
        let dir = tempdir().expect("temporary directory should be created");
        let path = dir.path().join("empty.sdf");
        std::fs::write(&path, "").expect("empty SDF fixture should be writable");

        let result = read_all_with_bond_tolerance(&path, 0.1);

        assert!(matches!(result, Err(err) if err.is_empty_file()));
    }

    #[test]
    fn read_file_format_with_bond_tolerance_empty_sdf_returns_empty_file() {
        let dir = tempdir().expect("temporary directory should be created");
        let path = dir.path().join("empty.sdf");
        std::fs::write(&path, "").expect("empty SDF fixture should be writable");

        let result = read_file_format_with_bond_tolerance(&path, FileFormat::Sdf, 0.1);

        assert!(matches!(result, Err(err) if err.is_empty_file()));
    }

    #[test]
    fn read_all_format_empty_mol2_returns_empty_file() {
        let dir = tempdir().expect("temporary directory should be created");
        let path = dir.path().join("empty.mol2");
        std::fs::write(&path, "").expect("empty MOL2 fixture should be writable");

        let result = read_all_format(&path, FileFormat::Mol2);

        assert!(matches!(result, Err(err) if err.is_empty_file()));
    }

    #[test]
    fn read_all_format_with_bond_tolerance_empty_mol2_returns_empty_file() {
        let dir = tempdir().expect("temporary directory should be created");
        let path = dir.path().join("empty.mol2");
        std::fs::write(&path, "").expect("empty MOL2 fixture should be writable");

        let result = read_all_format_with_bond_tolerance(&path, FileFormat::Mol2, 0.1);

        assert!(matches!(result, Err(err) if err.is_empty_file()));
    }

    #[test]
    fn read_all_format_with_bond_tolerance_sdf_returns_all_records() {
        let dir = tempdir().expect("temporary directory should be created");
        let path = dir.path().join("molecules.sdf");
        let molecules = [
            single_atom_molecule("first", 0.0),
            single_atom_molecule("second", 1.0),
        ];
        write_sdf_fixture(&path, &molecules);

        let parsed = read_all_format_with_bond_tolerance(&path, FileFormat::Sdf, 0.1)
            .expect("multi-record SDF should parse");

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name, "first");
        assert_eq!(parsed[1].name, "second");
    }

    #[test]
    fn read_all_format_with_bond_tolerance_mol2_returns_all_records() {
        let dir = tempdir().expect("temporary directory should be created");
        let path = dir.path().join("molecules.mol2");
        let molecules = [
            single_atom_molecule("first", 0.0),
            single_atom_molecule("second", 1.0),
        ];
        write_mol2_fixture(&path, &molecules);

        let parsed = read_all_format_with_bond_tolerance(&path, FileFormat::Mol2, 0.1)
            .expect("multi-record MOL2 should parse");

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name, "first");
        assert_eq!(parsed[1].name, "second");
    }

    #[test]
    fn read_file_format_with_bond_tolerance_sdf_returns_first_record() {
        let dir = tempdir().expect("temporary directory should be created");
        let path = dir.path().join("molecules.sdf");
        let molecules = [
            single_atom_molecule("first", 0.0),
            single_atom_molecule("second", 1.0),
        ];
        write_sdf_fixture(&path, &molecules);

        let parsed = read_file_format_with_bond_tolerance(&path, FileFormat::Sdf, 0.1)
            .expect("multi-record SDF should parse");

        assert_eq!(parsed.name, "first");
    }

    #[test]
    fn read_file_format_with_bond_tolerance_mol2_returns_first_record() {
        let dir = tempdir().expect("temporary directory should be created");
        let path = dir.path().join("molecules.mol2");
        let molecules = [
            single_atom_molecule("first", 0.0),
            single_atom_molecule("second", 1.0),
        ];
        write_mol2_fixture(&path, &molecules);

        let parsed = read_file_format_with_bond_tolerance(&path, FileFormat::Mol2, 0.1)
            .expect("multi-record MOL2 should parse");

        assert_eq!(parsed.name, "first");
    }

    #[test]
    fn read_all_format_with_bond_tolerance_pdb_returns_logical_molecules() {
        let dir = tempdir().expect("temporary directory should be created");
        let path = dir.path().join("models.pdb");
        let pdb = r#"MODEL        1
ATOM      1  N   ALA A   1       0.000   0.000   0.000  1.00 20.00           N
ATOM      2  CA  ALA A   1       1.000   0.000   0.000  1.00 20.00           C
ENDMDL
MODEL        2
ATOM      1  N   GLY A   1       0.500   0.000   0.000  1.00 20.00           N
ATOM      2  CA  GLY A   1       1.500   0.000   0.000  1.00 20.00           C
ENDMDL
"#;
        std::fs::write(&path, pdb).expect("PDB fixture should be writable");

        let parsed = read_all_format_with_bond_tolerance(&path, FileFormat::Pdb, 0.1)
            .expect("PDB fixture should parse");

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].atoms_slice()[0].residue.resn, "ALA");
        assert_eq!(parsed[1].atoms_slice()[0].residue.resn, "GLY");
    }

    #[test]
    fn read_all_format_with_bond_tolerance_cif_returns_logical_molecules() {
        let dir = tempdir().expect("temporary directory should be created");
        let path = dir.path().join("blocks.cif");
        let cif = r#"data_FIRST
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
        std::fs::write(&path, cif).expect("CIF fixture should be writable");

        let parsed = read_all_format_with_bond_tolerance(&path, FileFormat::Cif, 0.1)
            .expect("CIF fixture should parse");

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name, "FIRST");
        assert_eq!(parsed[1].name, "SECOND");
    }

    #[test]
    fn read_all_format_with_bond_tolerance_bcif_returns_logical_molecules() {
        let dir = tempdir().expect("temporary directory should be created");
        let path = dir.path().join("blocks.bcif");
        let blocks = vec![
            atom_site_block("FIRST", &[atom_row(1, "C", "LIG", "A", 0.0, 1)]),
            atom_site_block("SECOND", &[atom_row(1, "N", "LIG", "B", 1.0, 1)]),
        ];
        let bytes = encode_bcif_file(&blocks);
        std::fs::write(&path, bytes).expect("bCIF fixture should be writable");

        let parsed = read_all_format_with_bond_tolerance(&path, FileFormat::Bcif, 0.1)
            .expect("bCIF fixture should parse");

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name, "FIRST");
        assert_eq!(parsed[1].name, "SECOND");
    }
}
