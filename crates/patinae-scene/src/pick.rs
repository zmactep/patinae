//! Pick hit + selection expansion.
//!
//! GPU color-ID picking now lives entirely in `patinae_render::picking`;
//! the host bridges (`patinae-scene::bridge::resolve_pick`) translate raw
//! GPU pixel hits into [`PickHit`]. This module just defines the
//! `PickHit` shape commands consume and the `mouse_selection_mode`
//! expansion logic.

use lin_alg::f32::Vec3;
use patinae_mol::{AtomIndex, ObjectMolecule};
use patinae_select::{
    format_exact_selector_value, MacroSpec, Pattern, ResiItem, SelectionExpr, SelectionResult,
};
use std::fmt;

use crate::object::ObjectType;

/// A pick hit with object and atom information
#[derive(Debug, Clone)]
pub struct PickHit {
    /// Name of the object that was hit
    pub object_name: String,
    /// Type of the object
    pub object_type: ObjectType,
    /// Atom index if an atom was hit
    pub atom_index: Option<AtomIndex>,
    /// World-space position of the hit
    pub position: Vec3,
    /// Distance from camera
    pub distance: f32,
}

impl PickHit {
    /// Check if this hit an atom
    pub fn is_atom(&self) -> bool {
        self.atom_index.is_some()
    }
}

/// Errors produced while formatting canonical atom paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AtomPathError {
    /// The hit belongs to an object that cannot contain atoms.
    NotMolecule(ObjectType),
    /// The hit does not identify an atom.
    MissingAtomIndex,
    /// The hit identifies no atom in the supplied molecule.
    AtomNotFound(AtomIndex),
    /// The hit and molecule names disagree.
    ObjectNameMismatch {
        /// Object name requested by the caller.
        requested: String,
        /// Name of the supplied molecule.
        molecule: String,
    },
    /// The insertion code cannot be represented by the slash grammar.
    UnrepresentableInsertionCode(char),
    /// The alternate location cannot be represented by the slash grammar.
    UnrepresentableAlternateLocation(char),
}

impl fmt::Display for AtomPathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotMolecule(object_type) => {
                write!(f, "picked object is {object_type}, not a molecule")
            }
            Self::MissingAtomIndex => write!(f, "pick hit does not contain an atom index"),
            Self::AtomNotFound(index) => {
                write!(f, "picked atom index {} does not exist", index.as_usize())
            }
            Self::ObjectNameMismatch {
                requested,
                molecule,
            } => write!(
                f,
                "requested object name {requested:?} does not match molecule name {molecule:?}"
            ),
            Self::UnrepresentableInsertionCode(code) => {
                write!(f, "insertion code {code:?} is not representable")
            }
            Self::UnrepresentableAlternateLocation(alt) => {
                write!(f, "alternate location {alt:?} is not representable")
            }
        }
    }
}

impl std::error::Error for AtomPathError {}

/// Formats a picked atom as a canonical slash path.
///
/// Every string-valued atom address component uses the selection language's
/// exact-value formatter. Blank segment, chain, insertion-code, and alternate
/// location values therefore remain explicit constraints rather than omitted
/// wildcard fields.
///
/// # Errors
///
/// Returns an error when the hit is not a molecule atom, disagrees with the
/// supplied molecule, refers past its atom table, or contains metadata the
/// slash grammar cannot represent losslessly.
pub fn canonical_atom_path_for_hit(
    hit: &PickHit,
    molecule: &ObjectMolecule,
) -> Result<String, AtomPathError> {
    if hit.object_type != ObjectType::Molecule {
        return Err(AtomPathError::NotMolecule(hit.object_type));
    }
    let atom_index = hit.atom_index.ok_or(AtomPathError::MissingAtomIndex)?;
    canonical_atom_path_for_atom(&hit.object_name, molecule, atom_index)
}

/// Formats one molecule atom as a canonical slash path.
///
/// # Errors
///
/// Returns an error when `object_name` disagrees with the molecule name, the
/// atom index is invalid, or atom metadata cannot be represented losslessly by
/// the slash grammar.
pub fn canonical_atom_path_for_atom(
    object_name: &str,
    molecule: &ObjectMolecule,
    atom_index: AtomIndex,
) -> Result<String, AtomPathError> {
    if object_name != molecule.name {
        return Err(AtomPathError::ObjectNameMismatch {
            requested: object_name.to_string(),
            molecule: molecule.name.clone(),
        });
    }

    let atom = molecule
        .get_atom(atom_index)
        .ok_or(AtomPathError::AtomNotFound(atom_index))?;
    let inscode = atom.residue.key.inscode;
    if !inscode.is_alphabetic() && inscode != ' ' {
        return Err(AtomPathError::UnrepresentableInsertionCode(inscode));
    }
    if atom.alt == '\0' {
        return Err(AtomPathError::UnrepresentableAlternateLocation(atom.alt));
    }

    let residue_identifier = format!("{}{inscode}", atom.residue.key.resv);
    let alternate_location_value = atom.alt.to_string();
    Ok(format_canonical_atom_path(
        object_name,
        &atom.residue.segi,
        &atom.residue.key.chain,
        &atom.residue.key.resn,
        &residue_identifier,
        &atom.name,
        &alternate_location_value,
    ))
}

pub(crate) fn format_canonical_atom_path(
    model: &str,
    segment: &str,
    chain: &str,
    residue_name: &str,
    residue_identifier: &str,
    atom_name: &str,
    alternate_location: &str,
) -> String {
    let model = format_exact_selector_value(model);
    let segment = format_exact_selector_value(segment);
    let chain = format_exact_selector_value(chain);
    let residue_name = format_exact_selector_value(residue_name);
    let residue_identifier = format_exact_selector_value(residue_identifier);
    let atom_name = format_exact_selector_value(atom_name);
    let alternate_location = format_exact_selector_value(alternate_location);

    format!(
        "/{model}/{segment}/{chain}/{residue_name}`{residue_identifier}/{atom_name}`{alternate_location}"
    )
}

/// Formats a canonical atom path for user-facing display.
///
/// Persisted paths keep quoted exact values for blank fields, insertion codes,
/// and alternate locations. This formatter hides those internal sentinels while
/// preserving quotes that are required for non-blank special characters.
///
/// # Examples
///
/// ```
/// use patinae_scene::display_atom_path;
///
/// let path = r#"/1fsd/""/A/LYS`"16 "/HZ2`" ""#;
/// assert_eq!(display_atom_path(path), "/1fsd//A/LYS`16/HZ2");
/// ```
#[must_use]
pub fn display_atom_path(path: &str) -> String {
    let Ok(SelectionExpr::Macro(spec)) = patinae_select::parse(path) else {
        return path.to_string();
    };
    display_exact_atom_macro(&spec).unwrap_or_else(|| path.to_string())
}

fn display_exact_atom_macro(spec: &MacroSpec) -> Option<String> {
    let model = display_exact_pattern(&spec.model)?;
    let segment = display_exact_pattern(&spec.segi)?;
    let chain = display_exact_pattern(&spec.chain)?;
    let residue_name = display_exact_pattern(&spec.resn)?;
    let residue_identifier = display_residue_identifier(spec)?;
    let atom_name = display_exact_pattern(&spec.name)?;
    let alternate_location = exact_pattern(&spec.alt)?;
    let alternate_location_suffix = if alternate_location.is_empty() || alternate_location == " " {
        String::new()
    } else {
        format!("`{}", display_exact_value(alternate_location))
    };

    Some(format!(
        "/{model}/{segment}/{chain}/{residue_name}`{residue_identifier}/{atom_name}{alternate_location_suffix}"
    ))
}

fn display_exact_pattern(pattern: &Option<Pattern>) -> Option<String> {
    exact_pattern(pattern).map(display_exact_value)
}

fn exact_pattern(pattern: &Option<Pattern>) -> Option<&str> {
    match pattern {
        Some(Pattern::Exact(value)) => Some(value),
        _ => None,
    }
}

fn display_residue_identifier(spec: &MacroSpec) -> Option<String> {
    let identifier = match spec.resi.as_ref()?.items.as_slice() {
        [ResiItem::Single(value)] => value.to_string(),
        [ResiItem::InsCode(value, ' ')] => value.to_string(),
        [ResiItem::InsCode(value, code)] => format!("{value}{code}"),
        _ => return None,
    };
    Some(display_exact_value(&identifier))
}

fn display_exact_value(value: &str) -> String {
    if value.is_empty() {
        String::new()
    } else {
        format_exact_selector_value(value).into_owned()
    }
}

/// Expand a pick hit to a selection based on `mouse_selection_mode`.
///
/// Modes: 0=atoms, 1=residues, 2=chains, 3=segments, 4=objects,
///        5=molecules, 6=C-alphas.
pub fn expand_pick_to_selection(
    hit: &PickHit,
    mode: i32,
    molecule: &ObjectMolecule,
) -> SelectionResult {
    let atom_count = molecule.atom_count();

    // Objects / molecules: select everything
    if mode == 4 || mode == 5 {
        return SelectionResult::all(atom_count);
    }

    // Need a hit atom for the remaining modes
    let hit_idx = match hit.atom_index {
        Some(idx) => idx,
        None => return SelectionResult::all(atom_count),
    };

    // Atoms: single atom
    if mode == 0 {
        return SelectionResult::from_indices(atom_count, std::iter::once(hit_idx));
    }

    // Get the reference atom for grouping
    let ref_atom = match molecule.get_atom(hit_idx) {
        Some(a) => a,
        None => return SelectionResult::none(atom_count),
    };

    // C-alphas: select the CA atom of the hit residue
    if mode == 6 {
        let indices = molecule.atoms_indexed().filter_map(|(idx, atom)| {
            if &*atom.name == "CA"
                && atom.residue.key.chain == ref_atom.residue.key.chain
                && atom.residue.key.resv == ref_atom.residue.key.resv
                && atom.residue.key.inscode == ref_atom.residue.key.inscode
            {
                Some(idx)
            } else {
                None
            }
        });
        return SelectionResult::from_indices(atom_count, indices);
    }

    let indices = molecule.atoms_indexed().filter_map(|(idx, atom)| {
        let matches = match mode {
            1 => {
                // Residues: same chain + resv + inscode
                atom.residue.key.chain == ref_atom.residue.key.chain
                    && atom.residue.key.resv == ref_atom.residue.key.resv
                    && atom.residue.key.inscode == ref_atom.residue.key.inscode
            }
            2 => {
                // Chains: same chain
                atom.residue.key.chain == ref_atom.residue.key.chain
            }
            3 => {
                // Segments: same segi
                atom.residue.segi == ref_atom.residue.segi
            }
            _ => false,
        };
        if matches {
            Some(idx)
        } else {
            None
        }
    });

    SelectionResult::from_indices(atom_count, indices)
}

/// Generate a selection expression string for a pick hit.
///
/// The expression depends on `mouse_selection_mode`:
/// - 0 (atoms): `model OBJ and index N`
/// - 1 (residues): `model OBJ and chain C and resi R[inscode]`
/// - 2 (chains): `model OBJ and chain C`
/// - 3 (segments): `model OBJ and segi S`
/// - 4 (objects): `model OBJ`
/// - 5 (molecules): `bymolecule (model OBJ and index N)` (connected component)
/// - 6 (C-alphas): `model OBJ and chain C and resi R[inscode] and name CA`
pub fn pick_expression_for_hit(
    hit: &PickHit,
    mode: i32,
    molecule: &ObjectMolecule,
) -> Option<String> {
    let obj = format_exact_selector_value(&hit.object_name);

    // Object mode: just the model name
    if mode == 4 {
        return Some(format!("model {obj}"));
    }

    let atom_idx = hit.atom_index?;
    let atom = molecule.get_atom(atom_idx)?;
    let idx = atom_idx.as_usize();

    match mode {
        0 => Some(format!("model {obj} and index {idx}")),
        5 => Some(format!("bymolecule (model {obj} and index {idx})")),
        1 => {
            let resi = format_resi(atom.residue.key.resv, atom.residue.key.inscode);
            let chain = format_exact_selector_value(&atom.residue.key.chain);
            Some(format!("model {obj} and chain {chain} and resi {resi}"))
        }
        2 => {
            let chain = format_exact_selector_value(&atom.residue.key.chain);
            Some(format!("model {obj} and chain {chain}"))
        }
        3 => {
            let segi = format_exact_selector_value(&atom.residue.segi);
            Some(format!("model {obj} and segi {segi}"))
        }
        6 => {
            let resi = format_resi(atom.residue.key.resv, atom.residue.key.inscode);
            let chain = format_exact_selector_value(&atom.residue.key.chain);
            Some(format!(
                "model {obj} and chain {chain} and resi {resi} and name CA"
            ))
        }
        _ => None,
    }
}

/// Format a residue number with optional insertion code.
fn format_resi(resv: i32, inscode: char) -> String {
    if inscode != ' ' && inscode != '\0' {
        format!("{resv}{inscode}")
    } else {
        resv.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use patinae_mol::{Atom, AtomBuilder, Element, MoleculeBuilder, RepMask};

    #[test]
    fn test_pick_hit() {
        let hit = PickHit {
            object_name: "protein".to_string(),
            object_type: ObjectType::Molecule,
            atom_index: Some(AtomIndex::from(42usize)),
            position: Vec3::new(1.0, 2.0, 3.0),
            distance: 5.0,
        };

        assert!(hit.is_atom());
        assert_eq!(hit.object_name, "protein");
    }

    /// Build a small molecule with 2 residues across 2 chains for testing.
    ///
    /// Layout: chain A / ALA 1 (atoms 0,1), chain A / GLY 2 (atom 2),
    ///         chain B / ALA 1 (atom 3) with segi "S2"
    fn test_molecule() -> ObjectMolecule {
        use patinae_mol::{AtomBuilder, Element, MoleculeBuilder};

        MoleculeBuilder::new("test")
            .add_atom(
                AtomBuilder::new()
                    .name("N")
                    .element(Element::Nitrogen)
                    .chain("A")
                    .resn("ALA")
                    .resv(1)
                    .build(),
                Vec3::new(0.0, 0.0, 0.0),
            )
            .add_atom(
                AtomBuilder::new()
                    .name("CA")
                    .element(Element::Carbon)
                    .chain("A")
                    .resn("ALA")
                    .resv(1)
                    .build(),
                Vec3::new(1.0, 0.0, 0.0),
            )
            .add_atom(
                AtomBuilder::new()
                    .name("N")
                    .element(Element::Nitrogen)
                    .chain("A")
                    .resn("GLY")
                    .resv(2)
                    .build(),
                Vec3::new(2.0, 0.0, 0.0),
            )
            .add_atom(
                AtomBuilder::new()
                    .name("CA")
                    .element(Element::Carbon)
                    .chain("B")
                    .resn("ALA")
                    .resv(1)
                    .segi("S2")
                    .build(),
                Vec3::new(3.0, 0.0, 0.0),
            )
            .build()
    }

    fn make_hit(atom_index: usize) -> PickHit {
        PickHit {
            object_name: "test".to_string(),
            object_type: ObjectType::Molecule,
            atom_index: Some(AtomIndex::from(atom_index)),
            position: Vec3::new(0.0, 0.0, 0.0),
            distance: 1.0,
        }
    }

    fn atom_with_path_metadata(
        name: &str,
        segi: &str,
        chain: &str,
        resn: &str,
        resv: i32,
        inscode: char,
        alt: char,
    ) -> Atom {
        let mut atom = AtomBuilder::new()
            .name(name)
            .element(Element::Carbon)
            .segi(segi)
            .chain(chain)
            .resn(resn)
            .resv(resv)
            .inscode(inscode)
            .build();
        atom.alt = alt;
        atom
    }

    fn path_hit(object_name: &str, atom_index: usize) -> PickHit {
        PickHit {
            object_name: object_name.to_string(),
            object_type: ObjectType::Molecule,
            atom_index: Some(AtomIndex::from(atom_index)),
            position: Vec3::new(0.0, 0.0, 0.0),
            distance: 1.0,
        }
    }

    fn assert_path_selects_only_first(path: &str, target: &ObjectMolecule, other: &ObjectMolecule) {
        let expr = patinae_select::parse(path).unwrap();
        let context = patinae_select::EvalContext::multi(vec![target, other]);
        let selected = patinae_select::evaluate(&expr, &context).unwrap();

        assert_eq!(selected.count(), 1, "path {path}");
        assert!(selected.contains_index(0), "path {path}");
    }

    #[test]
    fn canonical_atom_path_round_trips_ordinary_hit_among_near_collisions() {
        let target = MoleculeBuilder::new("ordinary")
            .add_atom(
                atom_with_path_metadata("CA", "", "A", "GLY", 42, ' ', ' '),
                Vec3::new(0.0, 0.0, 0.0),
            )
            .add_atom(
                atom_with_path_metadata("CA", "SEG", "A", "GLY", 42, ' ', ' '),
                Vec3::new(1.0, 0.0, 0.0),
            )
            .add_atom(
                atom_with_path_metadata("CA", "", "B", "GLY", 42, ' ', ' '),
                Vec3::new(2.0, 0.0, 0.0),
            )
            .add_atom(
                atom_with_path_metadata("CA", "", "A", "ALA", 42, ' ', ' '),
                Vec3::new(3.0, 0.0, 0.0),
            )
            .add_atom(
                atom_with_path_metadata("CA", "", "A", "GLY", 43, ' ', ' '),
                Vec3::new(4.0, 0.0, 0.0),
            )
            .add_atom(
                atom_with_path_metadata("CA", "", "A", "GLY", 42, 'A', ' '),
                Vec3::new(5.0, 0.0, 0.0),
            )
            .add_atom(
                atom_with_path_metadata("CB", "", "A", "GLY", 42, ' ', ' '),
                Vec3::new(6.0, 0.0, 0.0),
            )
            .add_atom(
                atom_with_path_metadata("CA", "", "A", "GLY", 42, ' ', 'B'),
                Vec3::new(7.0, 0.0, 0.0),
            )
            .build();
        let other = MoleculeBuilder::new("other")
            .add_atom(
                atom_with_path_metadata("CA", "", "A", "GLY", 42, ' ', ' '),
                Vec3::new(0.0, 0.0, 0.0),
            )
            .build();

        let path = canonical_atom_path_for_hit(&path_hit("ordinary", 0), &target).unwrap();

        assert_eq!(path, "/ordinary/\"\"/A/GLY`\"42 \"/CA`\" \"");
        assert_eq!(display_atom_path(&path), "/ordinary//A/GLY`42/CA");
        assert_path_selects_only_first(&path, &target, &other);
    }

    #[test]
    fn canonical_atom_path_round_trips_special_hit_as_exact_values() {
        let model = "model/\"quoted\"\\tail";
        let target = MoleculeBuilder::new(model)
            .add_atom(
                atom_with_path_metadata("C/A*?\"\\", "*", "?", "G/L\"Y\\", -42, 'A', '?'),
                Vec3::new(0.0, 0.0, 0.0),
            )
            .add_atom(
                atom_with_path_metadata("C/Axx\"\\", "*", "?", "G/L\"Y\\", -42, 'A', '?'),
                Vec3::new(1.0, 0.0, 0.0),
            )
            .build();
        let other = MoleculeBuilder::new("different/model")
            .add_atom(
                atom_with_path_metadata("C/A*?\"\\", "*", "?", "G/L\"Y\\", -42, 'A', '?'),
                Vec3::new(0.0, 0.0, 0.0),
            )
            .build();

        let path = canonical_atom_path_for_hit(&path_hit(model, 0), &target).unwrap();

        assert!(path.starts_with(r#"/"model/\"quoted\"\\tail"/"*"/"?"/"#));
        assert!(path.ends_with(r#"/"C/A*?\"\\"`"?""#));
        assert_eq!(
            display_atom_path(&path),
            r#"/"model/\"quoted\"\\tail"/"*"/"?"/"G/L\"Y\\"`"-42A"/"C/A*?\"\\"`"?""#
        );
        assert_path_selects_only_first(&path, &target, &other);
    }

    #[test]
    fn canonical_atom_path_rejects_incomplete_or_unrepresentable_hit() {
        let molecule = MoleculeBuilder::new("test")
            .add_atom(
                atom_with_path_metadata("CA", "", "A", "GLY", 42, ' ', ' '),
                Vec3::new(0.0, 0.0, 0.0),
            )
            .build();

        let mut hit = path_hit("test", 0);
        hit.atom_index = None;
        assert_eq!(
            canonical_atom_path_for_hit(&hit, &molecule),
            Err(AtomPathError::MissingAtomIndex)
        );

        let hit = path_hit("test", 10);
        assert_eq!(
            canonical_atom_path_for_hit(&hit, &molecule),
            Err(AtomPathError::AtomNotFound(AtomIndex::from(10usize)))
        );

        let mut hit = path_hit("test", 0);
        hit.object_type = ObjectType::Map;
        assert_eq!(
            canonical_atom_path_for_hit(&hit, &molecule),
            Err(AtomPathError::NotMolecule(ObjectType::Map))
        );

        let hit = path_hit("different", 0);
        assert!(matches!(
            canonical_atom_path_for_hit(&hit, &molecule),
            Err(AtomPathError::ObjectNameMismatch { .. })
        ));

        let invalid_inscode = MoleculeBuilder::new("test")
            .add_atom(
                atom_with_path_metadata("CA", "", "A", "GLY", 42, '*', ' '),
                Vec3::new(0.0, 0.0, 0.0),
            )
            .build();
        assert_eq!(
            canonical_atom_path_for_hit(&path_hit("test", 0), &invalid_inscode),
            Err(AtomPathError::UnrepresentableInsertionCode('*'))
        );

        let mut molecule = molecule;
        molecule.get_atom_mut(AtomIndex::from(0usize)).unwrap().alt = '\0';
        assert_eq!(
            canonical_atom_path_for_hit(&path_hit("test", 0), &molecule),
            Err(AtomPathError::UnrepresentableAlternateLocation('\0'))
        );
    }

    fn blank_chain_molecule() -> ObjectMolecule {
        use patinae_mol::{AtomBuilder, Element, MoleculeBuilder};

        MoleculeBuilder::new("test")
            .add_atom(
                AtomBuilder::new()
                    .name("N")
                    .element(Element::Nitrogen)
                    .chain("")
                    .resn("THR")
                    .resv(4)
                    .build(),
                Vec3::new(0.0, 0.0, 0.0),
            )
            .add_atom(
                AtomBuilder::new()
                    .name("CA")
                    .element(Element::Carbon)
                    .chain("")
                    .resn("THR")
                    .resv(4)
                    .build(),
                Vec3::new(1.0, 0.0, 0.0),
            )
            .add_atom(
                AtomBuilder::new()
                    .name("N")
                    .element(Element::Nitrogen)
                    .chain("A")
                    .resn("THR")
                    .resv(4)
                    .build(),
                Vec3::new(2.0, 0.0, 0.0),
            )
            .add_atom(
                AtomBuilder::new()
                    .name("CA")
                    .element(Element::Carbon)
                    .chain("A")
                    .resn("THR")
                    .resv(4)
                    .build(),
                Vec3::new(3.0, 0.0, 0.0),
            )
            .build()
    }

    #[test]
    fn test_expand_atoms_mode() {
        let mol = test_molecule();
        let hit = make_hit(0);
        let sel = expand_pick_to_selection(&hit, 0, &mol);
        assert_eq!(sel.count(), 1);
        assert!(sel.contains_index(0));
    }

    #[test]
    fn test_expand_residues_mode() {
        let mol = test_molecule();
        let hit = make_hit(0); // atom 0 is in ALA 1 chain A
        let sel = expand_pick_to_selection(&hit, 1, &mol);
        // Should select atoms 0 and 1 (both ALA 1 chain A)
        assert_eq!(sel.count(), 2);
        assert!(sel.contains_index(0));
        assert!(sel.contains_index(1));
        assert!(!sel.contains_index(2));
    }

    #[test]
    fn test_expand_chains_mode() {
        let mol = test_molecule();
        let hit = make_hit(0); // chain A
        let sel = expand_pick_to_selection(&hit, 2, &mol);
        // Should select atoms 0, 1, 2 (all chain A)
        assert_eq!(sel.count(), 3);
        assert!(!sel.contains_index(3));
    }

    #[test]
    fn test_expand_segments_mode() {
        let mol = test_molecule();
        // Hit atom 3 which has segi "S2"
        let hit = make_hit(3);
        let sel = expand_pick_to_selection(&hit, 3, &mol);
        // Only atom 3 has segi "S2"
        assert_eq!(sel.count(), 1);
        assert!(sel.contains_index(3));
    }

    #[test]
    fn test_expand_objects_mode() {
        let mol = test_molecule();
        let hit = make_hit(0);
        let sel = expand_pick_to_selection(&hit, 4, &mol);
        assert_eq!(sel.count(), 4); // all atoms
    }

    #[test]
    fn test_expand_molecules_mode() {
        let mol = test_molecule();
        let hit = make_hit(0);
        let sel = expand_pick_to_selection(&hit, 5, &mol);
        assert_eq!(sel.count(), 4); // all atoms
    }

    #[test]
    fn test_expand_c_alphas_mode() {
        let mol = test_molecule();
        // Hit atom 0 (N in ALA 1, chain A) → select only CA of that residue (atom 1)
        let hit = make_hit(0);
        let sel = expand_pick_to_selection(&hit, 6, &mol);
        assert_eq!(sel.count(), 1);
        assert!(sel.contains_index(1));
        assert!(!sel.contains_index(3)); // CA in chain B — different residue

        // Hit atom 3 (CA in ALA 1, chain B) → select itself
        let hit2 = make_hit(3);
        let sel2 = expand_pick_to_selection(&hit2, 6, &mol);
        assert_eq!(sel2.count(), 1);
        assert!(sel2.contains_index(3));
    }

    // ========================================================================
    // pick_expression_for_hit tests
    // ========================================================================

    #[test]
    fn test_pick_expr_atom_mode() {
        let mol = test_molecule();
        let hit = make_hit(2); // atom 2 = N in GLY 2, chain A
        let expr = pick_expression_for_hit(&hit, 0, &mol).unwrap();
        assert_eq!(expr, "model test and index 2");
    }

    #[test]
    fn test_pick_expr_residue_mode() {
        let mol = test_molecule();
        let hit = make_hit(0); // atom 0 = N in ALA 1, chain A
        let expr = pick_expression_for_hit(&hit, 1, &mol).unwrap();
        assert_eq!(expr, "model test and chain A and resi 1");
    }

    #[test]
    fn test_pick_expr_chain_mode() {
        let mol = test_molecule();
        let hit = make_hit(3); // atom 3 = CA in ALA 1, chain B
        let expr = pick_expression_for_hit(&hit, 2, &mol).unwrap();
        assert_eq!(expr, "model test and chain B");
    }

    #[test]
    fn test_pick_expr_segment_mode() {
        let mol = test_molecule();
        let hit = make_hit(3); // atom 3 has segi "S2"
        let expr = pick_expression_for_hit(&hit, 3, &mol).unwrap();
        assert_eq!(expr, "model test and segi S2");
    }

    #[test]
    fn test_pick_expr_object_mode() {
        let mol = test_molecule();
        let hit = make_hit(0);
        let expr = pick_expression_for_hit(&hit, 4, &mol).unwrap();
        assert_eq!(expr, "model test");
    }

    #[test]
    fn test_pick_expr_molecule_mode() {
        let mol = test_molecule();
        let hit = make_hit(1); // atom 1
        let expr = pick_expression_for_hit(&hit, 5, &mol).unwrap();
        assert_eq!(expr, "bymolecule (model test and index 1)");
    }

    #[test]
    fn test_pick_expr_calpha_mode() {
        let mol = test_molecule();
        let hit = make_hit(0); // atom 0 = N in ALA 1, chain A
        let expr = pick_expression_for_hit(&hit, 6, &mol).unwrap();
        assert_eq!(expr, "model test and chain A and resi 1 and name CA");
    }

    #[test]
    fn test_pick_expr_residue_mode_blank_chain() {
        let mol = blank_chain_molecule();
        let hit = make_hit(0);
        let expr = pick_expression_for_hit(&hit, 1, &mol).unwrap();
        assert_eq!(expr, "model test and chain \"\" and resi 4");

        let sel = patinae_select::select(&mol, &expr).unwrap();
        assert_eq!(sel.count(), 2);
        assert!(sel.contains_index(0));
        assert!(sel.contains_index(1));
        assert!(!sel.contains_index(2));
        assert!(!sel.contains_index(3));
    }

    #[test]
    fn test_pick_expr_chain_mode_blank_chain() {
        let mol = blank_chain_molecule();
        let hit = make_hit(0);
        let expr = pick_expression_for_hit(&hit, 2, &mol).unwrap();
        assert_eq!(expr, "model test and chain \"\"");

        let sel = patinae_select::select(&mol, &expr).unwrap();
        assert_eq!(sel.count(), 2);
        assert!(sel.contains_index(0));
        assert!(sel.contains_index(1));
    }

    #[test]
    fn test_pick_expr_calpha_mode_blank_chain() {
        let mol = blank_chain_molecule();
        let hit = make_hit(0);
        let expr = pick_expression_for_hit(&hit, 6, &mol).unwrap();
        assert_eq!(expr, "model test and chain \"\" and resi 4 and name CA");

        let sel = patinae_select::select(&mol, &expr).unwrap();
        assert_eq!(sel.count(), 1);
        assert!(sel.contains_index(1));
        assert!(!sel.contains_index(3));
    }

    #[test]
    fn test_pick_expr_no_atom_hit() {
        let mol = test_molecule();
        // Hit without atom_index (non-molecule object)
        let hit = PickHit {
            object_name: "test".to_string(),
            object_type: ObjectType::Molecule,
            atom_index: None,
            position: Vec3::new(0.0, 0.0, 0.0),
            distance: 1.0,
        };
        // Atom-level modes return None when no atom is hit
        assert!(pick_expression_for_hit(&hit, 0, &mol).is_none());
        // Object mode still works without an atom
        assert_eq!(
            pick_expression_for_hit(&hit, 4, &mol).unwrap(),
            "model test"
        );
    }

    #[test]
    fn test_invisible_atom_not_pickable() {
        let obj_reps = RepMask::CARTOON.union(RepMask::STICKS);

        // Atom with cartoon visible — pickable
        assert!(RepMask::CARTOON.intersection(obj_reps) != RepMask::NONE);

        // Atom with only lines — not pickable (lines not enabled at object level)
        assert!(RepMask::LINES.intersection(obj_reps) == RepMask::NONE);

        // Atom with no reps — not pickable
        assert!(RepMask::NONE.intersection(obj_reps) == RepMask::NONE);

        // Object with no reps — nothing pickable
        assert!(RepMask::CARTOON.intersection(RepMask::NONE) == RepMask::NONE);
    }
}
