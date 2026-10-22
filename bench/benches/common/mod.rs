use std::io::Read;
use std::path::{Path, PathBuf};

use once_cell::sync::Lazy;

use patinae_color::NamedPalette;
use patinae_color::ThemedPalette;
use patinae_mol::ObjectMolecule;
use patinae_scene::ColorResolver;

fn project_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("bench crate should be inside project root")
        .to_path_buf()
}

pub fn test_file_path() -> PathBuf {
    let val = std::env::var("BENCH_FILE").unwrap_or_else(|_| {
        panic!(
            "BENCH_FILE environment variable is required.\n\
             Usage: BENCH_FILE=path/to/structure.cif.gz cargo bench -p patinae-bench"
        )
    });
    let path = {
        let p = PathBuf::from(&val);
        if p.is_absolute() {
            p
        } else {
            project_root().join(p)
        }
    };
    assert!(path.exists(), "Bench file not found: {}", path.display());
    path
}

/// Decompressed CIF text, cached for the process lifetime.
pub static RAW_TEXT: Lazy<String> = Lazy::new(|| {
    let path = test_file_path();
    let file = std::fs::File::open(&path).expect("failed to open test file");
    let mut decoder = flate2::read::GzDecoder::new(file);
    let mut text = String::new();
    decoder
        .read_to_string(&mut text)
        .expect("failed to decompress test file");
    text
});

/// Parsed molecule (with bonds and atom classification, no DSS).
pub static MOLECULE: Lazy<ObjectMolecule> = Lazy::new(|| {
    let path = test_file_path();
    patinae_io::cif::read_cif(&path).expect("failed to parse CIF")
});

pub struct ColorResolverOwned {
    pub named: NamedPalette,
    pub palette: ThemedPalette,
}

impl ColorResolverOwned {
    pub fn new() -> Self {
        Self {
            named: NamedPalette::new(),
            palette: ThemedPalette::dark(),
        }
    }

    pub fn resolver(&self) -> ColorResolver<'_> {
        ColorResolver::new(&self.named, &self.palette)
    }
}
