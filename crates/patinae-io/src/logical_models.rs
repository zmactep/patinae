use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;

use lin_alg::f32::Vec3;
use patinae_mol::{Atom, AtomResidue, CoordSet, Element, ObjectMolecule};

use crate::error::{IoError, IoResult};

#[derive(Debug, Clone)]
pub(crate) struct ParsedAtom {
    pub(crate) name: String,
    pub(crate) element: Element,
    pub(crate) chain: String,
    pub(crate) resn: String,
    pub(crate) resv: i32,
    pub(crate) icode: char,
    pub(crate) alt: char,
    pub(crate) hetatm: bool,
    pub(crate) serial: Option<i32>,
    pub(crate) formal_charge: Option<i8>,
    pub(crate) occupancy: f32,
    pub(crate) b_factor: f32,
    pub(crate) segi: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ParsedModel {
    pub(crate) model_number: i32,
    pub(crate) atoms: Vec<ParsedAtom>,
    pub(crate) coords: Vec<Vec3>,
    /// Original assembly chain identifiers, in source atom order.
    pub(crate) source_chains: Vec<String>,
}

impl ParsedModel {
    pub(crate) fn new(model_number: i32) -> Self {
        Self {
            model_number,
            atoms: Vec::new(),
            coords: Vec::new(),
            source_chains: Vec::new(),
        }
    }
}

// File-level atom serials are metadata and may continue across compatible models.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct AtomIdentity {
    name: String,
    element: Element,
    chain: String,
    resn: String,
    resv: i32,
    icode: char,
    alt: char,
    hetatm: bool,
    formal_charge: Option<i8>,
}

impl From<&ParsedAtom> for AtomIdentity {
    fn from(atom: &ParsedAtom) -> Self {
        Self {
            name: atom.name.clone(),
            element: atom.element,
            chain: atom.chain.clone(),
            resn: atom.resn.clone(),
            resv: atom.resv,
            icode: atom.icode,
            alt: atom.alt,
            hetatm: atom.hetatm,
            formal_charge: atom.formal_charge,
        }
    }
}

struct TopologyGroup {
    models: Vec<ParsedModel>,
}

pub(crate) fn build_molecules(
    base_name: &str,
    title: &str,
    models: Vec<ParsedModel>,
) -> IoResult<Vec<ObjectMolecule>> {
    let mut groups = group_models_by_topology(models)?;
    if groups.is_empty() {
        return Err(IoError::empty_file());
    }

    let multiple_outputs = groups.len() > 1;
    groups
        .drain(..)
        .map(|group| build_group_molecule(base_name, title, group, multiple_outputs))
        .collect()
}

fn group_models_by_topology(models: Vec<ParsedModel>) -> IoResult<Vec<TopologyGroup>> {
    let mut groups: Vec<TopologyGroup> = Vec::new();
    let mut group_index_by_signature: HashMap<(Vec<AtomIdentity>, Vec<String>), usize> =
        HashMap::new();

    for model in models {
        if model.atoms.is_empty() {
            continue;
        }
        if model.atoms.len() != model.coords.len() {
            return Err(IoError::parse_msg(format!(
                "Model {} has {} atoms but {} coordinates",
                model.model_number,
                model.atoms.len(),
                model.coords.len()
            )));
        }

        if !model.source_chains.is_empty() && model.source_chains.len() != model.atoms.len() {
            return Err(IoError::parse_msg(
                "Assembly chain membership does not match atoms",
            ));
        }
        let signature = (
            model.atoms.iter().map(AtomIdentity::from).collect(),
            model.source_chains.clone(),
        );
        if let Some(&idx) = group_index_by_signature.get(&signature) {
            groups[idx].models.push(model);
        } else {
            let idx = groups.len();
            group_index_by_signature.insert(signature.clone(), idx);
            groups.push(TopologyGroup {
                models: vec![model],
            });
        }
    }

    Ok(groups)
}

fn build_group_molecule(
    base_name: &str,
    title: &str,
    group: TopologyGroup,
    multiple_outputs: bool,
) -> IoResult<ObjectMolecule> {
    let Some(first_model) = group.models.first() else {
        return Err(IoError::empty_file());
    };

    let name = if multiple_outputs {
        suffixed_name(base_name, first_model.model_number)
    } else {
        base_name.to_string()
    };

    let mut mol = ObjectMolecule::with_capacity(name, first_model.atoms.len(), 0);
    mol.title = title.to_string();
    for (index, chain) in first_model.source_chains.iter().enumerate() {
        let index = u32::try_from(index)
            .map_err(|_| IoError::parse_msg("Assembly atom index exceeds u32"))?;
        mol.assembly
            .chains
            .entry(chain.clone())
            .or_default()
            .push(index);
    }

    let mut residue_cache: HashMap<AtomResidue, Arc<AtomResidue>> = HashMap::new();
    for parsed in &first_model.atoms {
        let mut atom = Atom::new(parsed.name.as_str(), parsed.element);
        let residue_data = AtomResidue::from_parts(
            parsed.chain.clone(),
            parsed.resn.clone(),
            parsed.resv,
            parsed.icode,
            parsed.segi.clone(),
        );
        atom.residue = residue_cache
            .entry(residue_data.clone())
            .or_insert_with(|| Arc::new(residue_data))
            .clone();
        atom.alt = parsed.alt;
        atom.b_factor = parsed.b_factor;
        atom.occupancy = parsed.occupancy;
        atom.state.hetatm = parsed.hetatm;
        atom.formal_charge = parsed.formal_charge.unwrap_or(0);
        atom.id = parsed.serial.unwrap_or(0);
        mol.add_atom(atom);
    }

    for model in group.models {
        mol.add_coord_set(CoordSet::from_vec3(&model.coords));
    }

    Ok(mol)
}

fn suffixed_name(base_name: &str, model_number: i32) -> String {
    if base_name.is_empty() {
        format!("model_{model_number}")
    } else {
        format!("{base_name}_model_{model_number}")
    }
}
