//! Residue substitution: replace a residue's side chain with a target type.
//!
//! Keeps the backbone (N/CA/C/O + amide/HA hydrogens), grafts the target
//! residue's ideal side chain onto the backbone frame, relabels the residue,
//! and regroups atoms so the new ones sit with their residue. Used by the
//! `mutate` tool and the `scan` tool (per candidate).

use std::collections::HashMap;
use std::sync::Arc;

use patinae_mol::{AtomIndex, BondOrder, ObjectMolecule};

use crate::residue_geometry::ideal_side_chain_on;

const BACKBONE_KEEP: &[&str] = &[
    "N", "CA", "C", "O", "OXT", "H", "H1", "H2", "H3", "HA", "HA2", "HA3",
];

/// Builds a copy of `source` with the residue at (`chain`, `resv`, `inscode`)
/// mutated to `target_resn`. Returns an error if the residue lacks a backbone
/// or the target type has no ideal geometry.
pub fn build_mutant(
    source: &ObjectMolecule,
    chain: &str,
    resv: i32,
    inscode: char,
    target_resn: &str,
) -> Result<ObjectMolecule, String> {
    let mut mol = source.clone();

    let in_residue = |atom: &patinae_mol::Atom| {
        atom.residue.chain == chain && atom.residue.resv == resv && atom.residue.inscode == inscode
    };

    let remove: Vec<AtomIndex> = mol
        .atoms_indexed()
        .filter(|(_, atom)| in_residue(atom) && !BACKBONE_KEEP.contains(&atom.name.as_ref()))
        .map(|(idx, _)| idx)
        .collect();
    mol.remove_atoms(&remove);

    // Re-find the residue's backbone (indices shifted after removal).
    let mut n = None;
    let mut ca = None;
    let mut c = None;
    let mut survivors = Vec::new();
    let mut template = None;
    for (idx, atom) in mol.atoms_indexed() {
        if !in_residue(atom) {
            continue;
        }
        survivors.push(idx);
        if template.is_none() {
            template = Some(atom.clone());
        }
        if let Some(coord) = mol.get_coord(idx, 0) {
            match atom.name.as_ref() {
                "N" => n = Some(coord),
                "CA" => ca = Some(coord),
                "C" => c = Some(coord),
                _ => {}
            }
        }
    }
    let (Some(n), Some(ca), Some(c), Some(template)) = (n, ca, c, template) else {
        return Err("target residue is missing backbone N/CA/C".to_string());
    };

    let side_chain = ideal_side_chain_on(target_resn, n, ca, c)
        .ok_or_else(|| format!("residue {target_resn} is not supported"))?;

    // Clone the template residue so file-level ids (segi, mmCIF labels) survive.
    let mut residue = (*template.residue).clone();
    residue.key.resn = target_resn.to_owned();
    let residue = Arc::new(residue);
    let mut name_to_idx: HashMap<String, AtomIndex> = HashMap::new();
    for idx in &survivors {
        if let Some(atom) = mol.get_atom_mut(*idx) {
            atom.residue = residue.clone();
            name_to_idx.insert(atom.name.to_string(), *idx);
        }
    }

    let first_new_id = mol.atoms().map(|a| a.id).max().unwrap_or(0) + 1;
    for (id, (name, element, coord)) in (first_new_id..).zip(&side_chain.atoms) {
        let mut atom = template.clone();
        atom.name = name.as_str().into();
        atom.element = *element;
        atom.residue = residue.clone();
        atom.partial_charge = 0.0;
        atom.id = id;
        let idx = mol.push_atom_with_coord(atom, *coord);
        name_to_idx.insert(name.clone(), idx);
    }
    for (a, b) in &side_chain.bonds {
        if let (Some(&ia), Some(&ib)) = (name_to_idx.get(a), name_to_idx.get(b)) {
            let _ = mol.add_bond(ia, ib, BondOrder::Single);
        }
    }
    mol.regroup_by_residue();
    Ok(mol)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lin_alg::f32::Vec3;
    use patinae_mol::{Atom, AtomResidue, CoordSet, Element};

    #[test]
    fn mutation_keeps_file_level_residue_ids() {
        let mut residue = AtomResidue::from_parts("H", "GLY", 101, ' ', "SEG");
        residue.set_labels(Some(patinae_mol::ResidueLabels::new(
            Arc::new(patinae_mol::ResidueLabelChain::new(
                Some(Arc::from("B")),
                Some(Arc::from("2")),
            )),
            Some(1),
        )));
        let residue = Arc::new(residue);

        let mut source = ObjectMolecule::new("gly");
        for (name, element) in [
            ("N", Element::Nitrogen),
            ("CA", Element::Carbon),
            ("C", Element::Carbon),
        ] {
            let mut atom = Atom::new(name, element);
            atom.residue = residue.clone();
            source.add_atom(atom);
        }
        source.add_coord_set(CoordSet::from_vec3(&[
            Vec3::new(-0.5, 1.4, 0.0),
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.5, 0.0, 0.0),
        ]));

        let mutant = build_mutant(&source, "H", 101, ' ', "ALA").unwrap();

        assert!(mutant.atoms().any(|atom| &*atom.name == "CB"));
        for atom in mutant.atoms() {
            assert_eq!(atom.residue.resn, "ALA");
            assert_eq!(atom.residue.segi, "SEG");
            assert_eq!(atom.residue.label_asym_id(), Some("B"));
            assert_eq!(atom.residue.label_entity_id(), Some("2"));
            assert_eq!(atom.residue.label_seq_id(), Some(1));
        }
    }
}
