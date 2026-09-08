//! Shared source data, rigid copies, and biological assembly definitions.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use lin_alg::f32::Vec3;
use serde::{Deserialize, Serialize};

use crate::{AtomIndex, AtomResidue, CoordSet, MolError, MolResult, ObjectMolecule};

/// A column-major rigid affine transformation.
pub type InstanceTransform = [[f32; 4]; 4];

/// The identity copy transformation.
pub const IDENTITY_INSTANCE: InstanceTransform = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

/// Copy identities occupy sixteen bits in the picking attachment; zero means explicit.
pub const MAX_INSTANCE_COUNT: usize = u16::MAX as usize;

/// The physical storage mode, independent of an object's semantic type.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StorageMode {
    /// Every element is stored independently.
    #[default]
    Explicit,
    /// Copies share source data and carry rigid transformations.
    Instanced,
}

impl std::fmt::Display for StorageMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Explicit => "explicit",
            Self::Instanced => "instanced",
        })
    }
}

/// A reusable subset of source elements; an empty list means the entire source.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstanceGroup {
    /// Sorted, unique source indices, shared by all copies of this group.
    pub indices: Vec<u32>,
}

/// One copy of a reusable source group.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectInstance {
    /// Index into the table's groups.
    pub group: u32,
    /// Source-to-copy transformation, before the object-to-world transformation.
    pub transform: InstanceTransform,
}

/// Compact copy table usable by any scene object.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct InstanceTable {
    /// Distinct source subsets, stored once each.
    pub groups: Vec<InstanceGroup>,
    /// Stable copy identities are zero-based positions in this list.
    pub copies: Vec<ObjectInstance>,
}

/// A group of input chains and the operations applied to them.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AssemblyGroup {
    /// Original label-asym identifiers, or author chain identifiers for PDB.
    pub chains: Vec<String>,
    /// Composed transformations in assembly order.
    pub transforms: Vec<InstanceTransform>,
}

/// One assembly definition supplied by a structure file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AssemblyDefinition {
    /// File-defined assembly identifier.
    pub id: String,
    /// Optional description supplied by the file.
    pub details: String,
    /// Chain groups and transformations.
    pub groups: Vec<AssemblyGroup>,
}

/// Assembly recipes and original chain membership, without expanded coordinates.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AssemblyMetadata {
    /// Available assembly recipes.
    pub definitions: Vec<AssemblyDefinition>,
    /// Source atom membership keyed by original file chain identifiers.
    pub chains: BTreeMap<String, Vec<u32>>,
}

/// Applies a column-major affine transformation to a point.
pub fn transform_instance_point(m: &InstanceTransform, p: Vec3) -> Vec3 {
    Vec3::new(
        m[0][0] * p.x + m[1][0] * p.y + m[2][0] * p.z + m[3][0],
        m[0][1] * p.x + m[1][1] * p.y + m[2][1] * p.z + m[3][1],
        m[0][2] * p.x + m[1][2] * p.y + m[2][2] * p.z + m[3][2],
    )
}

/// Composes transformations, applying `right` first.
pub fn multiply_instance_transforms(
    left: &InstanceTransform,
    right: &InstanceTransform,
) -> InstanceTransform {
    std::array::from_fn(|column| {
        std::array::from_fn(|row| (0..4).map(|k| left[k][row] * right[column][k]).sum())
    })
}

fn invalid(message: impl Into<String>) -> MolError {
    MolError::Parse(message.into())
}

impl InstanceTable {
    /// Checks copy limits, proper rigid transforms, and source subset indices.
    ///
    /// # Errors
    /// Rejects empty tables, malformed subsets, non-rigid matrices, and index overflow.
    pub fn validate(&self, source_count: usize) -> MolResult<()> {
        if self.copies.is_empty() || self.copies.len() > MAX_INSTANCE_COUNT {
            return Err(invalid("instance table must contain 1..65535 copies"));
        }
        if self.groups.is_empty() || self.groups.len() > MAX_INSTANCE_COUNT {
            return Err(invalid(
                "instance table must contain 1..65535 source groups",
            ));
        }
        for group in &self.groups {
            if group.indices.iter().any(|&i| i as usize >= source_count)
                || group.indices.windows(2).any(|w| w[0] >= w[1])
            {
                return Err(invalid(
                    "instance group indices must be sorted, unique, and in bounds",
                ));
            }
        }
        for copy in &self.copies {
            if copy.group as usize >= self.groups.len() {
                return Err(invalid("invalid instance group"));
            }
            let m = &copy.transform;
            // Structure files round rotation coefficients; allow that rounding, not scale/shear.
            const TOLERANCE: f32 = 0.002;
            if m.iter().flatten().any(|v| !v.is_finite())
                || (m[3][3] - 1.0).abs() > TOLERANCE
                || (0..3).any(|i| m[i][3].abs() > TOLERANCE)
            {
                return Err(invalid("instance matrix must be finite and affine"));
            }
            for a in 0..3 {
                for b in 0..3 {
                    let dot: f32 = (0..3).map(|r| m[a][r] * m[b][r]).sum();
                    if (dot - if a == b { 1.0 } else { 0.0 }).abs() > TOLERANCE {
                        return Err(invalid("instance matrix must be rigid"));
                    }
                }
            }
            let determinant = m[0][0] * (m[1][1] * m[2][2] - m[2][1] * m[1][2])
                - m[1][0] * (m[0][1] * m[2][2] - m[2][1] * m[0][2])
                + m[2][0] * (m[0][1] * m[1][2] - m[1][1] * m[0][2]);
            if (determinant - 1.0).abs() > TOLERANCE {
                return Err(invalid("instance matrix must preserve handedness"));
            }
        }
        self.displayed_count(source_count)?;
        Ok(())
    }

    /// Returns the expanded element count without allocating expanded data.
    ///
    /// # Errors
    /// Returns an error for invalid groups or counts beyond signed atom identifiers.
    pub fn displayed_count(&self, source_count: usize) -> MolResult<usize> {
        self.copies.iter().try_fold(0usize, |total, copy| {
            let group = self
                .groups
                .get(copy.group as usize)
                .ok_or_else(|| invalid("invalid instance group"))?;
            let count = if group.indices.is_empty() {
                source_count
            } else {
                group.indices.len()
            };
            total
                .checked_add(count)
                .filter(|&n| n <= i32::MAX as usize)
                .ok_or_else(|| invalid("materialized atom count exceeds supported identifiers"))
        })
    }

    /// Tests membership of a source element in a copy.
    pub fn contains(&self, copy: u32, index: u32, source_count: usize) -> bool {
        (index as usize) < source_count
            && self
                .copies
                .get(copy as usize)
                .and_then(|copy| self.groups.get(copy.group as usize))
                .is_some_and(|group| {
                    group.indices.is_empty() || group.indices.binary_search(&index).is_ok()
                })
    }

    /// Maps a source atom and copy to its eventual explicit atom index.
    pub fn materialized_index(
        &self,
        copy: u32,
        atom: AtomIndex,
        source_count: usize,
    ) -> Option<AtomIndex> {
        if !self.contains(copy, atom.0, source_count) {
            return None;
        }
        let mut offset = 0usize;
        for (i, instance) in self.copies.iter().enumerate() {
            let indices = &self.groups.get(instance.group as usize)?.indices;
            if i == copy as usize {
                let local = if indices.is_empty() {
                    atom.as_usize()
                } else {
                    indices.binary_search(&atom.0).ok()?
                };
                return u32::try_from(offset.checked_add(local)?)
                    .ok()
                    .map(AtomIndex);
            }
            offset = offset.checked_add(if indices.is_empty() {
                source_count
            } else {
                indices.len()
            })?;
        }
        None
    }
}

impl AssemblyMetadata {
    /// Keeps original chain membership aligned with atom removal or reordering.
    pub(crate) fn remap_atoms(&mut self, mapping: &[Option<AtomIndex>]) {
        for indices in self.chains.values_mut() {
            *indices = indices
                .iter()
                .filter_map(|&old| {
                    mapping
                        .get(old as usize)
                        .copied()
                        .flatten()
                        .map(|atom| atom.0)
                })
                .collect();
            indices.sort_unstable();
            indices.dedup();
        }
    }

    /// Resolves a file-defined assembly into source subsets and copy matrices.
    ///
    /// # Errors
    /// Rejects missing assemblies, unknown or empty chains, and invalid copy tables.
    pub fn instance_table(&self, id: &str, source_count: usize) -> MolResult<InstanceTable> {
        let definition = self
            .definitions
            .iter()
            .find(|d| d.id == id)
            .ok_or_else(|| invalid(format!("assembly '{id}' is unavailable")))?;
        let mut table = InstanceTable::default();
        for group in &definition.groups {
            let mut indices = Vec::new();
            for chain in &group.chains {
                indices.extend(
                    self.chains
                        .get(chain)
                        .filter(|indices| !indices.is_empty())
                        .ok_or_else(|| {
                            invalid(format!("assembly chain '{chain}' is unavailable"))
                        })?,
                );
            }
            indices.sort_unstable();
            indices.dedup();
            if indices.iter().any(|&i| i as usize >= source_count) {
                return Err(invalid("assembly chain contains an out-of-bounds atom"));
            }
            if indices.is_empty() {
                return Err(invalid("assembly group has no atoms"));
            }
            if indices.len() == source_count {
                indices.clear();
            }
            let group_id = match table.groups.iter().position(|g| g.indices == indices) {
                Some(i) => i,
                None => {
                    table.groups.push(InstanceGroup { indices });
                    table.groups.len() - 1
                }
            };
            for transform in &group.transforms {
                table.copies.push(ObjectInstance {
                    group: group_id as u32,
                    transform: *transform,
                });
            }
        }
        table.validate(source_count)?;
        Ok(table)
    }
}

/// Expands one coordinate state without changing its source molecule.
///
/// # Errors
/// Rejects invalid tables, missing coordinates, allocation failure, and invalid bonds.
pub fn materialize_molecule(
    source: &ObjectMolecule,
    state: usize,
    table: &InstanceTable,
) -> MolResult<ObjectMolecule> {
    table.validate(source.atom_count())?;
    let coords = source
        .get_coord_set(state)
        .ok_or(MolError::StateIndexOutOfBounds(state, source.state_count()))?;
    let count = table.displayed_count(source.atom_count())?;
    let mut result = ObjectMolecule::new(source.name.clone());
    result.title = source.title.clone();
    result.settings = source.settings.clone();
    let mut next_unique_id = 1i32;
    result
        .atoms
        .try_reserve(count)
        .map_err(|e| invalid(format!("cannot allocate materialized atoms: {e}")))?;
    result
        .atom_bonds
        .try_reserve(count)
        .map_err(|e| invalid(format!("cannot allocate materialized bonds: {e}")))?;
    let mut positions = Vec::new();
    positions
        .try_reserve(
            count
                .checked_mul(3)
                .ok_or_else(|| invalid("coordinate count overflow"))?,
        )
        .map_err(|e| invalid(format!("cannot allocate materialized coordinates: {e}")))?;
    let mut used_chain_names = HashSet::new();
    for (copy_index, copy) in table.copies.iter().enumerate() {
        let mut map = HashMap::new();
        let mut chain_names = HashMap::<String, String>::new();
        let mut residues: HashMap<usize, Arc<AtomResidue>> = HashMap::new();
        let indices = &table.groups[copy.group as usize].indices;
        let iter: Box<dyn Iterator<Item = u32> + '_> = if indices.is_empty() {
            Box::new(0..source.atom_count() as u32)
        } else {
            Box::new(indices.iter().copied())
        };
        for old in iter {
            let mut atom = source.atoms[old as usize].clone();
            let key = Arc::as_ptr(&atom.residue) as usize;
            atom.residue = Arc::clone(residues.entry(key).or_insert_with(|| {
                let mut residue = (*atom.residue).clone();
                residue.key.chain = chain_names
                    .entry(residue.key.chain.clone())
                    .or_insert_with(|| {
                        let base = format!("{}{}", residue.key.chain, copy_index + 1);
                        let mut name = base.clone();
                        let mut suffix = 2;
                        // Numeric source names can collide: A1/copy 1 and A/copy 11.
                        while !used_chain_names.insert(name.clone()) {
                            name = format!("{base}_{suffix}");
                            suffix += 1;
                        }
                        name
                    })
                    .clone();
                Arc::new(residue)
            }));
            let point = coords
                .get_atom_coord(AtomIndex(old))
                .ok_or(MolError::NoCoordinates { atom: old, state })?;
            let point = transform_instance_point(&copy.transform, point);
            atom.id = (result.atom_count() + 1) as i32;
            atom.rank = result.atom_count() as i32;
            atom.discrete_state = 0;
            if let Some(old) = atom.repr.unique_id {
                atom.repr.unique_id = Some(copy_unique_settings(
                    source,
                    &mut result,
                    old,
                    &mut next_unique_id,
                )?);
            }
            if let Some(anisou) = atom.anisou {
                atom.anisou = Some(rotate_anisou(anisou, &copy.transform));
            }
            let new = result.add_atom(atom);
            map.insert(old, new);
            positions.extend_from_slice(&[point.x, point.y, point.z]);
        }
        for bond in source.bonds() {
            if let (Some(&a), Some(&b)) = (map.get(&bond.atom1.0), map.get(&bond.atom2.0)) {
                let mut bond = bond.clone();
                bond.atom1 = a;
                bond.atom2 = b;
                if let Some(old) = bond.unique_id {
                    bond.unique_id = Some(copy_unique_settings(
                        source,
                        &mut result,
                        old,
                        &mut next_unique_id,
                    )?);
                }
                result.bonds.push(bond);
            }
        }
    }
    result.add_coord_set(CoordSet::from_coords(positions));
    result.rebuild_atom_bonds();
    Ok(result)
}

fn copy_unique_settings(
    source: &ObjectMolecule,
    result: &mut ObjectMolecule,
    old: i32,
    next: &mut i32,
) -> MolResult<i32> {
    let id = *next;
    *next = next
        .checked_add(1)
        .ok_or_else(|| invalid("materialized setting ID overflow"))?;
    for (setting, value) in source.unique_settings.get_all(old) {
        result
            .unique_settings
            .set(id, setting, value.clone())
            .map_err(|e| invalid(e.to_string()))?;
    }
    Ok(id)
}

fn rotate_anisou(u: [f32; 6], m: &InstanceTransform) -> [f32; 6] {
    let tensor = [[u[0], u[3], u[4]], [u[3], u[1], u[5]], [u[4], u[5], u[2]]];
    // U' = R U R^T, with R stored by columns.
    let rotated: [[f32; 3]; 3] = std::array::from_fn(|a| {
        std::array::from_fn(|b| {
            (0..3)
                .flat_map(|i| (0..3).map(move |j| m[i][a] * tensor[i][j] * m[j][b]))
                .sum()
        })
    });
    [
        rotated[0][0],
        rotated[1][1],
        rotated[2][2],
        rotated[0][1],
        rotated[0][2],
        rotated[1][2],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Atom, BondOrder, Element};

    #[test]
    fn source_membership_tracks_atom_removal_and_permutation() {
        use crate::AtomBuilder;
        let mut source = ObjectMolecule::new("source");
        for (chain, residue) in [("A", 1), ("B", 1), ("A", 1)] {
            source.add_atom(
                AtomBuilder::new()
                    .name("CA")
                    .chain(chain)
                    .resv(residue)
                    .build(),
            );
        }
        source.assembly.chains.insert("labelA".into(), vec![0, 2]);
        source.assembly.chains.insert("labelB".into(), vec![1]);
        source.regroup_by_residue();
        assert_eq!(source.assembly.chains["labelA"], [0, 1]);
        assert_eq!(source.assembly.chains["labelB"], [2]);
        source.remove_atoms(&[AtomIndex(0)]);
        assert_eq!(source.assembly.chains["labelA"], [0]);
        assert_eq!(source.assembly.chains["labelB"], [1]);
    }

    #[test]
    fn materialization_preserves_coordinates_bonds_and_independent_chains() {
        let mut source = ObjectMolecule::new("source");
        for name in ["CA", "CB"] {
            source.add_atom(Atom::new(name, Element::Carbon));
        }
        source
            .add_bond(AtomIndex(0), AtomIndex(1), BondOrder::Single)
            .unwrap();
        source.add_coord_set(CoordSet::from_coords(vec![1., 0., 0., 2., 0., 0.]));
        let mut moved = IDENTITY_INSTANCE;
        moved[0] = [0., 1., 0., 0.];
        moved[1] = [-1., 0., 0., 0.];
        moved[3][0] = 10.;
        let table = InstanceTable {
            groups: vec![InstanceGroup::default()],
            copies: vec![
                ObjectInstance {
                    group: 0,
                    transform: IDENTITY_INSTANCE,
                },
                ObjectInstance {
                    group: 0,
                    transform: moved,
                },
            ],
        };
        let output = materialize_molecule(&source, 0, &table).unwrap();
        assert_eq!(source.atom_count(), 2);
        assert_eq!(output.atom_count(), 4);
        assert_eq!(output.bond_count(), 2);
        let p = output
            .get_coord_set(0)
            .unwrap()
            .get_atom_coord(AtomIndex(2))
            .unwrap();
        assert_eq!((p.x, p.y, p.z), (10., 1., 0.));
        assert_ne!(output.atoms[0].residue.chain, output.atoms[2].residue.chain);
        assert_eq!(
            table.materialized_index(1, AtomIndex(1), 2),
            Some(AtomIndex(3))
        );
        assert_eq!(output.bonds[1].atom1, AtomIndex(2));
    }

    #[test]
    fn materialized_chain_names_omit_separator_and_resolve_numeric_collisions() {
        let mut source = ObjectMolecule::new("source");
        for chain in ["A", "A", "A1"] {
            source.add_atom(crate::AtomBuilder::new().name("CA").chain(chain).build());
        }
        source.add_coord_set(CoordSet::from_coords(vec![0.; 9]));
        let table = InstanceTable {
            groups: vec![InstanceGroup::default()],
            copies: vec![
                ObjectInstance {
                    group: 0,
                    transform: IDENTITY_INSTANCE
                };
                11
            ],
        };
        let output = materialize_molecule(&source, 0, &table).unwrap();
        assert_eq!(output.atoms[0].residue.chain, "A1");
        assert_eq!(output.atoms[1].residue.chain, "A1");
        assert_eq!(output.atoms[3].residue.chain, "A2");
        assert_eq!(output.atoms[2].residue.chain, "A11");
        assert_ne!(output.atoms[30].residue.chain, "A11");
        let chains: HashSet<_> = output
            .atoms()
            .map(|atom| atom.residue.chain.as_str())
            .collect();
        assert_eq!(chains.len(), 22);
        assert!(chains.iter().all(|name| !name.contains('@')));
    }

    #[test]
    fn rejects_invalid_groups_and_non_rigid_transforms() {
        let mut table = InstanceTable {
            groups: vec![InstanceGroup {
                indices: vec![1, 1],
            }],
            copies: vec![ObjectInstance {
                group: 0,
                transform: IDENTITY_INSTANCE,
            }],
        };
        assert!(table.validate(2).is_err());
        table.groups[0].indices = vec![1];
        assert!(table.validate(2).is_ok());
        assert!(!table.contains(0, 0, 2));
        assert_eq!(
            table.materialized_index(0, AtomIndex(1), 2),
            Some(AtomIndex(0))
        );
        table.copies[0].transform[0][0] = 2.;
        assert!(table.validate(2).is_err());
    }

    #[test]
    fn materialization_rotates_ellipsoids_and_detaches_unique_settings() {
        let mut source = ObjectMolecule::new("source");
        let mut atom = Atom::new("CA", Element::Carbon);
        atom.anisou = Some([1., 2., 3., 0., 0., 0.]);
        atom.repr.unique_id = Some(7);
        source.add_atom(atom);
        source.add_coord_set(CoordSet::from_coords(vec![0., 0., 0.]));
        let setting = patinae_settings::id::sphere_scale;
        source
            .unique_settings
            .set(7, setting, patinae_settings::SettingValue::Float(2.))
            .unwrap();
        let mut rotated = IDENTITY_INSTANCE;
        rotated[0] = [0., 1., 0., 0.];
        rotated[1] = [-1., 0., 0., 0.];
        let table = InstanceTable {
            groups: vec![InstanceGroup::default()],
            copies: vec![
                ObjectInstance {
                    group: 0,
                    transform: IDENTITY_INSTANCE,
                },
                ObjectInstance {
                    group: 0,
                    transform: rotated,
                },
            ],
        };
        let mut output = materialize_molecule(&source, 0, &table).unwrap();
        assert_eq!(output.atoms[1].anisou, Some([2., 1., 3., 0., 0., 0.]));
        let a = output.atoms[0].repr.unique_id.unwrap();
        let b = output.atoms[1].repr.unique_id.unwrap();
        assert_ne!(a, b);
        output
            .unique_settings
            .set(a, setting, patinae_settings::SettingValue::Float(5.))
            .unwrap();
        assert_eq!(
            output.unique_settings.get(b, setting).unwrap().as_float(),
            Some(2.)
        );
    }
}
