//! Shared mmCIF entity decoding (`_entity`, `_entity_poly`, `_entity_poly_seq`).

use patinae_mol::{Entity, EntityKind, EntityPolymer, PolymerKind};

use crate::error::{IoError, IoResult};

/// Keeps a corrupt `num` from allocating a huge dense sequence (titin is ~35 000 residues).
const MAX_ENTITY_POLY_SEQ_NUM: u32 = 1_000_000;

/// Entity categories read from mmCIF and bCIF.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EntityCategory {
    Entity,
    Poly,
    PolySeq,
}

impl EntityCategory {
    pub(crate) fn from_name(category: &str) -> Option<Self> {
        match category {
            "_entity" => Some(Self::Entity),
            "_entity_poly" => Some(Self::Poly),
            "_entity_poly_seq" => Some(Self::PolySeq),
            _ => None,
        }
    }

    /// Category of an mmCIF data name such as `_entity_poly.type`.
    pub(crate) fn of_data_name(name: &str) -> Option<Self> {
        name.split_once('.')
            .and_then(|(category, _)| Self::from_name(category))
    }
}

/// `_entity` / `_entity_poly` row: an entity id and its type.
struct KindRow {
    id: Option<String>,
    kind: Option<String>,
}

/// `_entity_poly_seq` row.
struct MonomerRow {
    entity_id: Option<String>,
    num: Option<String>,
    mon_id: Option<String>,
}

/// Rows are validated in [`EntityRows::resolve`], so metadata-only blocks still load.
#[derive(Default)]
pub(crate) struct EntityRows {
    entities: Vec<KindRow>,
    polymers: Vec<KindRow>,
    monomers: Vec<MonomerRow>,
    error: Option<IoError>,
}

fn required<'a>(value: &'a Option<String>, field: &str) -> IoResult<&'a str> {
    value
        .as_deref()
        .ok_or_else(|| IoError::parse_msg(format!("Missing field '{field}'")))
}

impl EntityRows {
    /// Add one row from a field lookup keyed by the name after the category prefix.
    pub(crate) fn push(
        &mut self,
        category: EntityCategory,
        field: impl Fn(&str) -> Option<String>,
    ) {
        match category {
            EntityCategory::Entity => self.entities.push(KindRow {
                id: field("id"),
                kind: field("type"),
            }),
            EntityCategory::Poly => self.polymers.push(KindRow {
                id: field("entity_id"),
                kind: field("type"),
            }),
            EntityCategory::PolySeq => self.monomers.push(MonomerRow {
                entity_id: field("entity_id"),
                num: field("num"),
                mon_id: field("mon_id"),
            }),
        }
    }

    /// Record a malformed category; reported by [`EntityRows::resolve`].
    pub(crate) fn fail(&mut self, error: IoError) {
        self.error.get_or_insert(error);
    }

    /// Entities in `_entity` order, followed by ids that only the polymer tables mention.
    pub(crate) fn resolve(self) -> IoResult<Vec<Entity>> {
        if let Some(error) = self.error {
            return Err(error);
        }
        let mut entities: Vec<Entity> = Vec::new();
        for row in &self.entities {
            let id = required(&row.id, "_entity.id")?;
            if entities.iter().any(|entity| entity.id == id) {
                return Err(IoError::parse_msg(format!("Duplicate entity '{id}'")));
            }
            entities.push(Entity {
                id: id.to_owned(),
                kind: row.kind.as_deref().map(EntityKind::from_mmcif),
                polymer: None,
            });
        }

        for row in &self.polymers {
            let id = required(&row.id, "_entity_poly.entity_id")?;
            polymer_mut(&mut entities, id).kind = row.kind.as_deref().map(PolymerKind::from_mmcif);
        }

        for row in &self.monomers {
            let id = required(&row.entity_id, "_entity_poly_seq.entity_id")?;
            let num = required(&row.num, "_entity_poly_seq.num")?;
            let mon_id = required(&row.mon_id, "_entity_poly_seq.mon_id")?;
            let num = num
                .parse::<u32>()
                .ok()
                .filter(|num| (1..=MAX_ENTITY_POLY_SEQ_NUM).contains(num))
                .ok_or_else(|| {
                    IoError::parse_msg(format!("Invalid _entity_poly_seq.num '{num}'"))
                })?;

            let polymer = polymer_mut(&mut entities, id);
            let index = num as usize - 1;
            if polymer.sequence.len() <= index {
                polymer.sequence.resize(index + 1, String::new());
            }
            // Microheterogeneity: the first monomer is the primary one.
            if polymer.sequence[index].is_empty() {
                polymer.sequence[index] = mon_id.to_owned();
            } else if polymer.sequence[index] != mon_id {
                let alternatives = polymer.alternatives.entry(num).or_default();
                if !alternatives.iter().any(|alternative| alternative == mon_id) {
                    alternatives.push(mon_id.to_owned());
                }
            }
        }

        // The dictionary numbers monomers 1..N; a skipped `num` means rows were lost.
        for entity in &entities {
            let Some(polymer) = &entity.polymer else {
                continue;
            };
            if let Some(index) = polymer.sequence.iter().position(String::is_empty) {
                return Err(IoError::parse_msg(format!(
                    "Entity '{}' skips _entity_poly_seq.num {}",
                    entity.id,
                    index + 1
                )));
            }
        }

        Ok(entities)
    }
}

fn polymer_mut<'a>(entities: &'a mut Vec<Entity>, id: &str) -> &'a mut EntityPolymer {
    let index = match entities.iter().position(|entity| entity.id == id) {
        Some(index) => index,
        None => {
            entities.push(Entity {
                id: id.to_owned(),
                kind: None,
                polymer: None,
            });
            entities.len() - 1
        }
    };
    entities[index].polymer.get_or_insert_with(Default::default)
}

#[cfg(test)]
mod tests {
    use patinae_mol::ObjectMolecule;

    use super::*;

    const ATOMS: &str = "loop_
_atom_site.id
_atom_site.type_symbol
_atom_site.label_atom_id
_atom_site.label_comp_id
_atom_site.label_asym_id
_atom_site.label_entity_id
_atom_site.label_seq_id
_atom_site.Cartn_x
_atom_site.Cartn_y
_atom_site.Cartn_z
1 C CA GLY A 1 2 0.0 0.0 0.0
2 O O HOH B 2 . 5.0 0.0 0.0
";

    fn cif(text: &str) -> IoResult<ObjectMolecule> {
        crate::cif::read_cif_str(&format!("data_test\n{text}{ATOMS}"))
    }

    fn polymer(mol: &ObjectMolecule, id: &str) -> EntityPolymer {
        let entity = mol.entities.iter().find(|e| e.id == id).unwrap();
        entity.polymer.clone().unwrap()
    }

    #[test]
    fn entities_keep_kinds_and_full_sequence_with_unresolved_residues() {
        let mol = cif("loop_
_entity.id
_entity.type
1 polymer
2 water
_entity_poly.entity_id 1
_entity_poly.type 'polypeptide(L)'
loop_
_entity_poly_seq.entity_id
_entity_poly_seq.num
_entity_poly_seq.mon_id
1 1 MET
1 2 GLY
1 3 SER
")
        .unwrap();

        assert_eq!(mol.entities.len(), 2);
        assert_eq!(mol.entities[0].kind, Some(EntityKind::Polymer));
        assert_eq!(mol.entities[1].kind, Some(EntityKind::Water));
        assert_eq!(mol.entities[1].polymer, None);

        let chain = polymer(&mol, "1");
        assert_eq!(chain.kind, Some(PolymerKind::PeptideL));
        assert_eq!(chain.sequence, ["MET", "GLY", "SER"]);
        let atom = &mol.atoms_slice()[0];
        assert_eq!(atom.residue.label_entity_id.as_deref(), Some("1"));
        assert_eq!(
            chain.monomer(atom.residue.label_seq_id.unwrap()),
            Some("GLY")
        );
    }

    #[test]
    fn unsorted_and_microheterogeneous_sequences_keep_positions() {
        let mol = cif("loop_
_entity_poly_seq.entity_id
_entity_poly_seq.num
_entity_poly_seq.mon_id
_entity_poly_seq.hetero
1 3 SER n
1 1 MET n
1 2 GLY y
1 2 ALA y
1 2 GLY y
1 4 LYS n
")
        .unwrap();

        let chain = polymer(&mol, "1");
        assert_eq!(chain.kind, None);
        assert_eq!(chain.sequence, ["MET", "GLY", "SER", "LYS"]);
        assert_eq!(chain.alternatives.len(), 1);
        assert_eq!(chain.alternatives[&2], ["ALA"]);
        assert_eq!(chain.monomer(4), Some("LYS"));
        assert_eq!(mol.entities[0].kind, None);
    }

    #[test]
    fn single_item_entity_categories_are_read() {
        let mol = cif("_entity.id 1
_entity.type polymer
_entity_poly.entity_id 1
_entity_poly.type polyribonucleotide
_entity_poly_seq.entity_id 1
_entity_poly_seq.num 1
_entity_poly_seq.mon_id G
")
        .unwrap();

        let chain = polymer(&mol, "1");
        assert_eq!(chain.kind, Some(PolymerKind::Rna));
        assert_eq!(chain.sequence, ["G"]);
    }

    #[test]
    fn corrupt_sequence_numbers_are_rejected() {
        for num in ["0", "-1", "x", "1000001"] {
            let text = format!(
                "_entity_poly_seq.entity_id 1\n_entity_poly_seq.num {num}\n_entity_poly_seq.mon_id MET\n"
            );
            let error = cif(&text).err().unwrap();
            assert!(
                error.to_string().contains("Invalid _entity_poly_seq.num"),
                "{num}"
            );
        }
    }

    #[test]
    fn skipped_sequence_numbers_are_rejected() {
        let error = cif("loop_
_entity_poly_seq.entity_id
_entity_poly_seq.num
_entity_poly_seq.mon_id
1 1 MET
1 3 SER
")
        .err()
        .unwrap();

        assert!(error
            .to_string()
            .contains("Entity '1' skips _entity_poly_seq.num 2"));
    }

    #[test]
    fn duplicate_entities_and_missing_ids_are_rejected() {
        let duplicate = cif("loop_\n_entity.id\n_entity.type\n1 polymer\n1 water\n");
        assert!(duplicate
            .err()
            .unwrap()
            .to_string()
            .contains("Duplicate entity"));

        let missing = cif("_entity_poly_seq.entity_id 1\n_entity_poly_seq.num 1\n");
        assert!(missing
            .err()
            .unwrap()
            .to_string()
            .contains("_entity_poly_seq.mon_id"));
    }

    #[test]
    fn formats_without_entities_leave_them_empty() {
        let mol = crate::cif::read_cif_str(&format!("data_test\n{ATOMS}")).unwrap();
        assert!(mol.entities.is_empty());

        let pdb =
            "ATOM      1  CA  GLY A   1       0.000   0.000   0.000  1.00  0.00           C\n";
        assert!(crate::pdb::read_pdb_str(pdb).unwrap().entities.is_empty());
    }
}
