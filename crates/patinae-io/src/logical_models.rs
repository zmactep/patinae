use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::sync::Arc;

use lin_alg::f32::Vec3;
use patinae_mol::{
    Atom, AtomResidue, CoordSet, Element, ObjectMolecule, ResidueLabelChain, ResidueLabels,
};

use crate::error::{IoError, IoResult};

#[derive(Debug, Clone)]
pub(crate) struct ParsedAtom {
    pub(crate) name: String,
    pub(crate) element: Element,
    pub(crate) residue: ResidueId,
    pub(crate) alt: char,
    pub(crate) hetatm: bool,
    pub(crate) serial: Option<i32>,
    pub(crate) formal_charge: Option<i8>,
    pub(crate) occupancy: f32,
    pub(crate) b_factor: f32,
}

/// Index into one parsed model's residue table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResidueId(u32);

/// Borrowed residue metadata from the current input row.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ParsedLabels<'a> {
    pub(crate) asym_id: Option<&'a str>,
    pub(crate) entity_id: Option<&'a str>,
    pub(crate) seq_id: Option<u32>,
}

impl<'a> ParsedLabels<'a> {
    pub(crate) fn new(
        asym_id: Option<&'a str>,
        entity_id: Option<&'a str>,
        seq_id: Option<u32>,
    ) -> Self {
        Self {
            asym_id,
            entity_id,
            seq_id,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ParsedResidue<'a> {
    pub(crate) chain: &'a str,
    pub(crate) resn: &'a str,
    pub(crate) resv: i32,
    pub(crate) icode: char,
    pub(crate) segi: &'a str,
    pub(crate) labels: ParsedLabels<'a>,
}

impl ParsedResidue<'_> {
    fn matches(&self, residue: &AtomResidue) -> bool {
        self.chain == residue.chain
            && self.resn == residue.resn
            && self.resv == residue.resv
            && self.icode == residue.inscode
            && self.segi == residue.segi
            && self.labels.asym_id == residue.label_asym_id()
            && self.labels.entity_id == residue.label_entity_id()
            && self.labels.seq_id == residue.label_seq_id()
    }
}

#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<ParsedAtom>() <= 64);

#[derive(Debug, Clone)]
pub(crate) struct ParsedModel {
    pub(crate) model_number: i32,
    pub(crate) atoms: Vec<ParsedAtom>,
    pub(crate) coords: Vec<Vec3>,
    residues: Vec<Arc<AtomResidue>>,
    residue_ids: HashMap<Arc<AtomResidue>, ResidueId>,
    previous_residue: Option<ResidueId>,
    label_strings: HashSet<Arc<str>>,
    label_chains: HashSet<Arc<ResidueLabelChain>>,
    /// Original assembly chain identifiers, in source atom order.
    pub(crate) source_chains: SourceChains,
}

impl ParsedModel {
    pub(crate) fn new(model_number: i32) -> Self {
        Self {
            model_number,
            atoms: Vec::new(),
            coords: Vec::new(),
            residues: Vec::new(),
            residue_ids: HashMap::new(),
            previous_residue: None,
            label_strings: HashSet::new(),
            label_chains: HashSet::new(),
            source_chains: SourceChains::default(),
        }
    }

    pub(crate) fn intern_residue(&mut self, input: ParsedResidue<'_>) -> IoResult<ResidueId> {
        if let Some(id) = self.previous_residue {
            if input.matches(&self.residues[id.0 as usize]) {
                return Ok(id);
            }
        }
        let mut residue =
            AtomResidue::from_parts(input.chain, input.resn, input.resv, input.icode, input.segi);
        if input.labels.asym_id.is_some()
            || input.labels.entity_id.is_some()
            || input.labels.seq_id.is_some()
        {
            let chain = ResidueLabelChain::new(
                intern_string(&mut self.label_strings, input.labels.asym_id),
                intern_string(&mut self.label_strings, input.labels.entity_id),
            );
            let chain = if let Some(shared) = self.label_chains.get(&chain) {
                Arc::clone(shared)
            } else {
                let chain = Arc::new(chain);
                self.label_chains.insert(Arc::clone(&chain));
                chain
            };
            residue.set_labels(Some(ResidueLabels::new(chain, input.labels.seq_id)));
        }
        let id = if let Some(&id) = self.residue_ids.get(&residue) {
            id
        } else {
            let id = ResidueId(
                u32::try_from(self.residues.len())
                    .map_err(|_| IoError::parse_msg("Residue index exceeds u32"))?,
            );
            let residue = Arc::new(residue);
            self.residue_ids.insert(Arc::clone(&residue), id);
            self.residues.push(residue);
            id
        };
        self.previous_residue = Some(id);
        Ok(id)
    }
}

fn intern_string(pool: &mut HashSet<Arc<str>>, value: Option<&str>) -> Option<Arc<str>> {
    let value = value?;
    if let Some(shared) = pool.get(value) {
        return Some(Arc::clone(shared));
    }
    let shared: Arc<str> = Arc::from(value);
    pool.insert(Arc::clone(&shared));
    Some(shared)
}

/// Interned assembly chain identifiers, assigned in first-appearance order.
#[derive(Debug, Clone, Default)]
pub(crate) struct SourceChains {
    names: Vec<String>,
    atom_chains: Vec<usize>,
    index_by_name: HashMap<String, usize>,
}

impl SourceChains {
    pub(crate) fn push(&mut self, name: &str) {
        let index = if let Some(&index) = self.index_by_name.get(name) {
            index
        } else {
            let index = self.names.len();
            self.names.push(name.to_owned());
            self.index_by_name.insert(name.to_owned(), index);
            index
        };
        self.atom_chains.push(index);
    }

    pub(crate) fn clear(&mut self) {
        self.names.clear();
        self.atom_chains.clear();
        self.index_by_name.clear();
    }

    fn is_empty(&self) -> bool {
        self.atom_chains.is_empty()
    }

    fn len(&self) -> usize {
        self.atom_chains.len()
    }
}

// File-level atom serials are metadata and may continue across compatible models.
// mmCIF label ids are metadata too: grouped models keep the first model's labels.
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

impl AtomIdentity {
    fn new(atom: &ParsedAtom, residue: &AtomResidue) -> Self {
        Self {
            name: atom.name.clone(),
            element: atom.element,
            chain: residue.chain.clone(),
            resn: residue.resn.clone(),
            resv: residue.resv,
            icode: residue.inscode,
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
    let models: Vec<_> = models
        .into_iter()
        .filter(|model| !model.atoms.is_empty())
        .collect();
    for model in &models {
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
    }

    // A single model cannot have a topology conflict; retain the same validation
    // without allocating and hashing a second copy of every atom identity.
    if models.len() <= 1 {
        return Ok(if models.is_empty() {
            Vec::new()
        } else {
            vec![TopologyGroup { models }]
        });
    }

    let mut groups: Vec<TopologyGroup> = Vec::new();
    let mut group_index_by_signature: HashMap<_, usize> = HashMap::new();
    for model in models {
        let signature = (
            model
                .atoms
                .iter()
                .map(|atom| AtomIdentity::new(atom, &model.residues[atom.residue.0 as usize]))
                .collect::<Vec<_>>(),
            model.source_chains.names.clone(),
            model.source_chains.atom_chains.clone(),
        );
        if let Some(&idx) = group_index_by_signature.get(&signature) {
            groups[idx].models.push(model);
        } else {
            let idx = groups.len();
            group_index_by_signature.insert(signature, idx);
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
    let mut chain_members = vec![Vec::new(); first_model.source_chains.names.len()];
    for (index, &chain) in first_model.source_chains.atom_chains.iter().enumerate() {
        let index = u32::try_from(index)
            .map_err(|_| IoError::parse_msg("Assembly atom index exceeds u32"))?;
        chain_members[chain].push(index);
    }
    for (name, members) in first_model.source_chains.names.iter().zip(chain_members) {
        mol.assembly.chains.insert(name.clone(), members);
    }

    for parsed in &first_model.atoms {
        let mut atom = Atom::with_residue(
            parsed.name.as_str(),
            parsed.element,
            Arc::clone(&first_model.residues[parsed.residue.0 as usize]),
        );
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

#[cfg(test)]
mod tests {
    use super::*;

    fn input(resv: i32, labels: ParsedLabels<'_>) -> ParsedResidue<'_> {
        ParsedResidue {
            chain: "A",
            resn: "GLY",
            resv,
            icode: ' ',
            segi: "",
            labels,
        }
    }

    fn model() -> ParsedModel {
        let mut model = ParsedModel::new(1);
        let residue = model
            .intern_residue(input(1, ParsedLabels::default()))
            .unwrap();
        model.atoms.push(ParsedAtom {
            name: "CA".into(),
            element: Element::Carbon,
            residue,
            alt: ' ',
            hetatm: false,
            serial: None,
            formal_charge: None,
            occupancy: 1.0,
            b_factor: 0.0,
        });
        model.coords.push(Vec3::new(1.0, 2.0, 3.0));
        model
    }

    #[test]
    fn single_model_still_validates_coordinates_and_assembly_membership() {
        let mut missing_coord = model();
        missing_coord.coords.clear();
        assert!(build_molecules("bad", "", vec![missing_coord])
            .unwrap_err()
            .to_string()
            .contains("1 atoms but 0 coordinates"));
        let mut extra_chain = model();
        extra_chain.source_chains.push("A");
        extra_chain.source_chains.push("B");
        assert!(build_molecules("bad", "", vec![extra_chain])
            .unwrap_err()
            .to_string()
            .contains("Assembly chain membership"));
    }

    #[test]
    fn empty_models_do_not_change_single_model_output() {
        let molecules =
            build_molecules("one", "title", vec![ParsedModel::new(0), model()]).unwrap();
        assert_eq!(molecules.len(), 1);
        assert_eq!(molecules[0].name, "one");
        assert_eq!(molecules[0].title, "title");
        assert_eq!(molecules[0].state_count(), 1);
        assert_eq!(molecules[0].atom_count(), 1);
    }

    #[test]
    fn residues_are_shared_by_value_not_by_adjacency() {
        let mut model = model();
        let template = model.atoms[0].clone();
        for (resv, seq_id) in [(1, None), (1, Some(1)), (2, None), (1, None)] {
            let residue = model
                .intern_residue(input(resv, ParsedLabels::new(None, None, seq_id)))
                .unwrap();
            model.atoms.push(ParsedAtom {
                residue,
                ..template.clone()
            });
            model.coords.push(Vec3::new(0.0, 0.0, 0.0));
        }
        let molecule = build_molecules("mol", "", vec![model]).unwrap().remove(0);
        let residues: Vec<_> = molecule.atoms().map(|atom| &atom.residue).collect();
        assert!(Arc::ptr_eq(residues[0], residues[1]));
        assert!(!Arc::ptr_eq(residues[1], residues[2]));
        assert_eq!(residues[2].label_seq_id(), Some(1));
        assert!(Arc::ptr_eq(residues[0], residues[4]));
        assert!(!Arc::ptr_eq(residues[3], residues[4]));
    }

    #[test]
    fn topology_resolves_model_local_ids_and_keeps_first_labels() {
        let mut first = model();
        let mut second = model();
        second.model_number = 2;
        let labels = ParsedLabels::new(Some("L-01"), Some("001x"), Some(9));
        first.atoms[0].residue = first.intern_residue(input(1, labels)).unwrap();
        second.intern_residue(input(99, labels)).unwrap();
        second.atoms[0].residue = second
            .intern_residue(input(1, ParsedLabels::new(Some("other"), None, None)))
            .unwrap();
        let molecules = build_molecules("mol", "", vec![first, second]).unwrap();
        assert_eq!(molecules.len(), 1);
        assert_eq!(molecules[0].state_count(), 2);
        assert_eq!(
            molecules[0].atoms().next().unwrap().residue.label_asym_id(),
            Some("L-01")
        );
    }
}
