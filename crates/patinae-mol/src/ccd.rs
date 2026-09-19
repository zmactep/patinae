//! Chemical Component Dictionary (CCD) bond templates.
//!
//! Provides bond connectivity for known HETATM residues (HEM, ATP, etc.)
//! from a pre-built cache file (`~/.patinae/resources/components.bin`).
//!
//! Two-tier lookup:
//! 1. **Cache** — loaded explicitly via [`load_cache`] (triggered by `setup_ccd` command)
//! 2. **Fallback** — `None` → caller uses distance-based bonding
//!
//! The cache is built from PDB's CCD using `scripts/build_ccd_cache.py`.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use crate::bond::BondOrder;

// ============================================================================
// Data Structures
// ============================================================================

/// A bond in a CCD template (owned, deserialized from cache).
pub struct CcdBondOwned {
    pub atom1: String,
    pub atom2: String,
    pub order: u8, // 1=Single, 2=Double, 3=Triple, 4=Aromatic
}

/// A CCD residue template (owned, deserialized from cache).
pub struct CcdTemplateOwned {
    pub atoms: Vec<String>,
    pub bonds: Vec<CcdBondOwned>,
}

impl CcdTemplateOwned {
    /// Iterate bonds yielding (atom1_name, atom2_name, BondOrder).
    pub fn iter_bonds(&self) -> impl Iterator<Item = (&str, &str, BondOrder)> {
        self.bonds.iter().map(|b| {
            let order = match b.order {
                1 => BondOrder::Single,
                2 => BondOrder::Double,
                3 => BondOrder::Triple,
                4 => BondOrder::Aromatic,
                _ => BondOrder::Single,
            };
            (b.atom1.as_str(), b.atom2.as_str(), order)
        })
    }

    /// Check if all heavy (non-hydrogen) atoms from the template are present.
    pub fn validate_heavy_atoms(&self, contains: impl Fn(&str) -> bool) -> bool {
        self.atoms
            .iter()
            .filter(|a| !is_hydrogen_name(a))
            .all(|a| contains(a.as_str()))
    }
}

/// Check if an atom name looks like a hydrogen.
fn is_hydrogen_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    if bytes.is_empty() {
        return false;
    }
    bytes[0] == b'H' || (bytes.len() > 1 && bytes[0].is_ascii_digit() && bytes[1] == b'H')
}

// ============================================================================
// Cache
// ============================================================================

static CCD_CACHE: OnceLock<HashMap<String, CcdTemplateOwned>> = OnceLock::new();

/// Load the full CCD cache from `~/.patinae/resources/components.bin`.
///
/// Returns the number of loaded components, or an error message.
/// If already loaded, returns the existing count.
pub fn load_cache() -> Result<usize, String> {
    if let Some(cache) = CCD_CACHE.get() {
        return Ok(cache.len());
    }

    let path = patinae_settings::paths::resources_dir().join("components.bin");
    let data = std::fs::read(&path).map_err(|_| {
        format!(
            "CCD cache not found at {}\n\n\
             To install:\n\
             1. Download: curl -L -o components.cif.gz \
                https://files.wwpdb.org/pub/pdb/data/monomers/components.cif.gz\n\
             2. Convert: python scripts/build_ccd_cache.py components.cif.gz {}\n\
             3. Copy: cp components.bin $PATINAE_RESOURCES_DIR/",
            path.display(),
            path.display(),
        )
    })?;

    let cache = deserialize_cache(&data)?;
    let count = cache.len();
    let _ = CCD_CACHE.set(cache);
    log::info!(
        "Loaded CCD cache: {} components from {}",
        count,
        path.display()
    );
    Ok(CCD_CACHE.get().map_or(0, |c| c.len()))
}

/// Load only specific residue names from the CCD cache.
///
/// More memory-efficient than [`load_cache`] when only a few ligands are needed.
/// If cache is already loaded (full), returns existing count.
pub fn load_cache_filtered(resnames: &HashSet<String>) -> Result<usize, String> {
    if let Some(cache) = CCD_CACHE.get() {
        return Ok(cache.len());
    }

    let path = patinae_settings::paths::resources_dir().join("components.bin");
    let data = std::fs::read(&path).map_err(|_| {
        format!(
            "CCD cache not found at {}\n\n\
             To install:\n\
             1. Download: curl -L -o components.cif.gz \
                https://files.wwpdb.org/pub/pdb/data/monomers/components.cif.gz\n\
             2. Convert: python scripts/build_ccd_cache.py components.cif.gz {}",
            path.display(),
            path.display(),
        )
    })?;

    let full = deserialize_cache(&data)?;
    let filtered: HashMap<String, CcdTemplateOwned> = full
        .into_iter()
        .filter(|(k, _)| resnames.contains(k))
        .collect();

    let count = filtered.len();
    let _ = CCD_CACHE.set(filtered);
    log::info!(
        "Loaded CCD cache (filtered): {} of {} requested components",
        count,
        resnames.len()
    );
    Ok(CCD_CACHE.get().map_or(0, |c| c.len()))
}

/// Look up a CCD template by residue name.
///
/// Returns `None` if cache is not loaded or the residue is not in the cache.
pub(crate) fn get_template(resn: &str) -> Option<&'static CcdTemplateOwned> {
    CCD_CACHE.get()?.get(resn)
}

/// Check if the CCD cache has been loaded.
pub fn is_loaded() -> bool {
    CCD_CACHE.get().is_some()
}

// ============================================================================
// Deserialization
// ============================================================================

/// Msgpack shape: { resn: { "a": [str], "b": [[str, str, u8]] } }
#[derive(serde::Deserialize)]
struct RawComponent {
    a: Vec<String>,
    b: Vec<(String, String, u8)>,
}

fn deserialize_cache(data: &[u8]) -> Result<HashMap<String, CcdTemplateOwned>, String> {
    let raw: HashMap<String, RawComponent> =
        rmp_serde::from_slice(data).map_err(|e| format!("Failed to parse CCD cache: {}", e))?;

    let mut result = HashMap::with_capacity(raw.len());
    for (resn, comp) in raw {
        let bonds = comp
            .b
            .into_iter()
            .map(|(a1, a2, order)| CcdBondOwned {
                atom1: a1,
                atom2: a2,
                order,
            })
            .collect();
        result.insert(
            resn,
            CcdTemplateOwned {
                atoms: comp.a,
                bonds,
            },
        );
    }
    Ok(result)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_hydrogen_name() {
        assert!(is_hydrogen_name("H"));
        assert!(is_hydrogen_name("HA"));
        assert!(is_hydrogen_name("HHA"));
        assert!(is_hydrogen_name("1HB"));
        assert!(!is_hydrogen_name("C1A"));
        assert!(!is_hydrogen_name("NA"));
        assert!(!is_hydrogen_name("FE"));
    }

    #[test]
    fn unknown_residue_has_no_template() {
        assert!(get_template("ZZZZZ").is_none());
    }

    #[test]
    fn test_bond_order_mapping() {
        let bond = CcdBondOwned {
            atom1: "C".into(),
            atom2: "O".into(),
            order: 2,
        };
        let template = CcdTemplateOwned {
            atoms: vec!["C".into(), "O".into()],
            bonds: vec![bond],
        };
        let (a1, a2, order) = template.iter_bonds().next().unwrap();
        assert_eq!(a1, "C");
        assert_eq!(a2, "O");
        assert_eq!(order, BondOrder::Double);
    }

    #[test]
    fn test_validate_heavy_atoms() {
        let template = CcdTemplateOwned {
            atoms: vec!["C".into(), "O".into(), "HA".into(), "HB".into()],
            bonds: vec![],
        };
        let mut names = HashMap::new();
        names.insert("C", 0);
        names.insert("O", 1);
        // H atoms missing — should still pass (only heavy atoms validated)
        assert!(template.validate_heavy_atoms(|n| names.contains_key(n)));

        // Remove a heavy atom — should fail
        names.remove("O");
        assert!(!template.validate_heavy_atoms(|n| names.contains_key(n)));
    }
}
