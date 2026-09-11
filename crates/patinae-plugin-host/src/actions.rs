//! Prepare validated viewer actions and apply them atomically on the host thread.

use patinae_cmd::tasks::{TaskEffects, TaskError};
use patinae_mol::{Atom, AtomIndex, Element, SecondaryStructure};
use patinae_plugin::{
    registrar::ViewerMutation,
    wire::{self, WireViewerAction},
};
use patinae_scene::{ObjectRegistry, ViewerLike};
use std::{collections::BTreeMap, sync::Arc};

pub(crate) struct ActionEffects {
    pub scene: TaskEffects,
    pub panel_update: bool,
}

pub(crate) fn apply_task_action(
    kernel: &mut patinae_framework::kernel::AppKernel,
    action: WireViewerAction,
) -> Result<ActionEffects, TaskError> {
    let mut panel_update = false;
    let changed = match action {
        WireViewerAction::ApplyAtomPropertyChanges(changes) => {
            let prepared = PreparedAtomChanges::new(&kernel.session.registry, &changes)?;
            let changed = prepared.changed;
            kernel.mutate_viewer(None, (1, 1), |viewer| {
                prepared.apply(viewer);
            });
            changed
        }
        action => {
            let changed = match &action {
                WireViewerAction::SetViewportImage(image) => kernel
                    .session
                    .viewport_image
                    .as_ref()
                    .is_none_or(|current| {
                        current.width != image.width
                            || current.height != image.height
                            || current.data != image.data
                    }),
                WireViewerAction::ClearViewportImage => kernel.session.viewport_image.is_some(),
                _ => false,
            };
            let mut mutations = Vec::new();
            apply_local_viewer_action(action, &mut mutations, &mut panel_update);
            for mutation in mutations {
                kernel.mutate_viewer(None, (1, 1), mutation);
            }
            changed
        }
    };
    Ok(ActionEffects {
        scene: if changed {
            TaskEffects::Applied
        } else {
            TaskEffects::None
        },
        panel_update,
    })
}

struct PreparedAtomChanges {
    atoms: BTreeMap<(String, u32), (Atom, bool)>,
    changed: bool,
    identity_changed: bool,
    applied: bool,
}

impl PreparedAtomChanges {
    fn new(
        registry: &ObjectRegistry,
        changes: &[wire::WireAtomPropertyChange],
    ) -> Result<Self, TaskError> {
        let mut prepared = Self {
            atoms: BTreeMap::new(),
            changed: false,
            identity_changed: false,
            applied: false,
        };
        for change in changes {
            let object = registry.get_molecule(&change.object).ok_or_else(|| {
                TaskError::new(
                    "object_not_found",
                    format!("object {} no longer exists", change.object),
                )
            })?;
            object
                .require_explicit()
                .map_err(|e| TaskError::new("invalid_storage", e.to_string()))?;
            let key = (change.object.clone(), change.atom_index);
            let (atom, changed) = match prepared.atoms.entry(key) {
                std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::btree_map::Entry::Vacant(entry) => {
                    let atom = object
                        .molecule()
                        .get_atom(AtomIndex(change.atom_index))
                        .ok_or_else(|| {
                            TaskError::new("atom_not_found", "atom index is no longer valid")
                        })?;
                    entry.insert((atom.clone(), false))
                }
            };
            for (key, value) in &change.changes {
                let outcome = apply_atom_property_change(atom, key, value);
                if !outcome.applied {
                    return Err(TaskError::new(
                        "invalid_atom_property",
                        format!("unsupported value for {key}"),
                    ));
                }
                *changed |= outcome.changed;
                prepared.changed |= outcome.changed;
                prepared.identity_changed |= outcome.identity_changed;
                prepared.applied |= outcome.applied;
            }
        }
        Ok(prepared)
    }

    // Preparation and application occur in the same host call, without yielding.
    fn apply(self, viewer: &mut (impl ViewerLike + ?Sized)) -> bool {
        for ((object, index), (atom, changed)) in self.atoms {
            if changed {
                *viewer
                    .objects_mut()
                    .get_molecule_mut(&object)
                    .expect("prepared object")
                    .molecule_mut()
                    .get_atom_mut(AtomIndex(index))
                    .expect("prepared atom") = atom;
            }
        }
        if self.identity_changed {
            viewer.reconcile_recent_atoms();
        }
        if self.changed {
            viewer.request_redraw();
        }
        self.applied
    }
}

pub(crate) fn apply_atom_property_change_batch<V: ViewerLike + ?Sized>(
    viewer: &mut V,
    changes: &[wire::WireAtomPropertyChange],
) -> bool {
    match PreparedAtomChanges::new(viewer.objects(), changes) {
        Ok(prepared) => prepared.apply(viewer),
        Err(error) => {
            log::warn!("{error}");
            false
        }
    }
}

pub(crate) fn apply_local_viewer_action(
    action: WireViewerAction,
    mutation_queue: &mut Vec<ViewerMutation>,
    panel_update_requested: &mut bool,
) {
    match action {
        WireViewerAction::SetViewportImage(image) => {
            mutation_queue.push(Box::new(move |viewer| {
                viewer.set_viewport_image(Some(image));
            }));
        }
        WireViewerAction::ClearViewportImage => {
            mutation_queue.push(Box::new(|viewer| {
                viewer.set_viewport_image(None);
            }));
        }
        WireViewerAction::RequestRedraw => {
            mutation_queue.push(Box::new(|viewer| viewer.request_redraw()));
        }
        WireViewerAction::RequestPanelUpdate => {
            *panel_update_requested = true;
        }
        WireViewerAction::ApplyAtomPropertyChanges(changes) => {
            mutation_queue.push(Box::new(move |viewer| {
                apply_atom_property_change_batch(viewer, &changes);
            }));
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct AtomPropertyChangeOutcome {
    applied: bool,
    changed: bool,
    identity_changed: bool,
}

#[cfg(test)]
fn apply_atom_property_changes(
    atom: &mut Atom,
    changes: &[(String, wire::WireAtomPropertyValue)],
) -> AtomPropertyChangeOutcome {
    let mut outcome = AtomPropertyChangeOutcome::default();
    for (key, value) in changes {
        let change = apply_atom_property_change(atom, key, value);
        outcome.applied |= change.applied;
        outcome.changed |= change.changed;
        outcome.identity_changed |= change.identity_changed;
    }
    outcome
}

fn apply_atom_property_change(
    atom: &mut Atom,
    key: &str,
    value: &wire::WireAtomPropertyValue,
) -> AtomPropertyChangeOutcome {
    let (changed, identity_changed) = match (key, value) {
        ("name", wire::WireAtomPropertyValue::Str(value)) => {
            let changed = atom.name.as_ref() != value.as_str();
            if changed {
                atom.name = Arc::from(value.as_str());
            }
            (changed, changed)
        }
        ("b", wire::WireAtomPropertyValue::F32(value)) => {
            let changed = atom.b_factor.to_bits() != value.to_bits();
            if changed {
                atom.b_factor = *value;
            }
            (changed, false)
        }
        ("q", wire::WireAtomPropertyValue::F32(value)) => {
            let changed = atom.occupancy.to_bits() != value.to_bits();
            if changed {
                atom.occupancy = *value;
            }
            (changed, false)
        }
        ("vdw", wire::WireAtomPropertyValue::F32(value)) => {
            let changed = atom.vdw.to_bits() != value.to_bits();
            if changed {
                atom.vdw = *value;
            }
            (changed, false)
        }
        ("partial_charge", wire::WireAtomPropertyValue::F32(value)) => {
            let changed = atom.partial_charge.to_bits() != value.to_bits();
            if changed {
                atom.partial_charge = *value;
            }
            (changed, false)
        }
        ("formal_charge", wire::WireAtomPropertyValue::I8(value)) => {
            let changed = atom.formal_charge != *value;
            if changed {
                atom.formal_charge = *value;
            }
            (changed, false)
        }
        ("color", wire::WireAtomPropertyValue::I32(value)) => {
            let changed = atom.repr.colors.base != *value;
            if changed {
                atom.repr.colors.base = *value;
            }
            (changed, false)
        }
        ("elem", wire::WireAtomPropertyValue::Str(value)) => {
            let Some(element) = Element::from_symbol(value) else {
                return AtomPropertyChangeOutcome::default();
            };
            let changed = atom.element != element;
            if changed {
                atom.element = element;
            }
            (changed, false)
        }
        ("ss", wire::WireAtomPropertyValue::Str(value)) => {
            let ss_type = match value.as_str() {
                "H" => SecondaryStructure::Helix,
                "S" => SecondaryStructure::Sheet,
                _ => SecondaryStructure::Loop,
            };
            let changed = atom.ss_type != ss_type;
            if changed {
                atom.ss_type = ss_type;
            }
            (changed, false)
        }
        ("type", wire::WireAtomPropertyValue::Str(value)) => {
            let hetatm = value == "HETATM";
            let changed = atom.state.hetatm != hetatm;
            if changed {
                atom.state.hetatm = hetatm;
            }
            (changed, false)
        }
        ("alt", wire::WireAtomPropertyValue::Str(value)) => {
            let alt = value.chars().next().unwrap_or(' ');
            let changed = atom.alt != alt;
            if changed {
                atom.alt = alt;
            }
            (changed, changed)
        }
        ("chain", wire::WireAtomPropertyValue::Str(value)) => {
            let changed = atom.residue.key.chain.as_str() != value.as_str();
            if changed {
                let mut residue = (*atom.residue).clone();
                residue.key.chain = value.clone();
                atom.residue = Arc::new(residue);
            }
            (changed, changed)
        }
        ("resn", wire::WireAtomPropertyValue::Str(value)) => {
            let changed = atom.residue.key.resn.as_str() != value.as_str();
            if changed {
                let mut residue = (*atom.residue).clone();
                residue.key.resn = value.clone();
                atom.residue = Arc::new(residue);
            }
            (changed, changed)
        }
        ("resv", wire::WireAtomPropertyValue::I32(value)) => {
            let changed = atom.residue.key.resv != *value;
            if changed {
                let mut residue = (*atom.residue).clone();
                residue.key.resv = *value;
                atom.residue = Arc::new(residue);
            }
            (changed, changed)
        }
        ("segi", wire::WireAtomPropertyValue::Str(value)) => {
            let changed = atom.residue.segi.as_str() != value.as_str();
            if changed {
                let mut residue = (*atom.residue).clone();
                residue.segi = value.clone();
                atom.residue = Arc::new(residue);
            }
            (changed, changed)
        }
        _ => return AtomPropertyChangeOutcome::default(),
    };
    AtomPropertyChangeOutcome {
        applied: true,
        changed,
        identity_changed,
    }
}

#[cfg(test)]
mod recent_atom_tests {
    use super::*;
    use patinae_mol::{AtomBuilder, ObjectMolecule};
    use patinae_scene::{MoleculeObject, PickHit, Session, SessionAdapter};
    use patinae_settings::groups::RecentPickLimit;

    #[test]
    fn batch_preparation_is_atomic_and_accumulates_changes_to_the_same_atom() {
        let mut session = Session::new();
        let mut molecule = ObjectMolecule::new("obj");
        molecule.add_atom(patinae_mol::Atom::new("CA", Element::Carbon));
        session.registry.add(MoleculeObject::new(molecule));
        let mut redraw = false;
        let mut viewer = SessionAdapter {
            session: &mut session,
            render_context: None,
            default_size: (1, 1),
            needs_redraw: &mut redraw,
        };
        let change = |atom_index, key: &str, value| wire::WireAtomPropertyChange {
            object: "obj".into(),
            atom_index,
            changes: vec![(key.into(), wire::WireAtomPropertyValue::F32(value))],
        };
        let before = viewer.session().mutation_revision();
        assert!(!apply_atom_property_change_batch(
            &mut viewer,
            &[change(0, "b", 5.0), change(10, "q", 0.5)]
        ));
        assert_eq!(viewer.session().mutation_revision(), before);
        assert!(!*viewer.needs_redraw);
        assert!(apply_atom_property_change_batch(
            &mut viewer,
            &[change(0, "b", 5.0), change(0, "q", 0.5)]
        ));
        let atom = viewer
            .objects()
            .get_molecule("obj")
            .unwrap()
            .molecule()
            .get_atom(AtomIndex(0))
            .unwrap();
        assert_eq!(atom.b_factor, 5.0);
        assert_eq!(atom.occupancy, 0.5);
    }

    #[test]
    fn atom_identity_outcome_ignores_no_op_assignments() {
        let mut atom = AtomBuilder::new()
            .name("CA")
            .element_symbol("C")
            .resn("GLY")
            .resv(1)
            .chain("A")
            .build();

        let no_op = apply_atom_property_changes(
            &mut atom,
            &[(
                "name".to_string(),
                wire::WireAtomPropertyValue::Str("CA".to_string()),
            )],
        );
        assert_eq!(
            no_op,
            AtomPropertyChangeOutcome {
                applied: true,
                changed: false,
                identity_changed: false,
            }
        );

        let changed = apply_atom_property_changes(
            &mut atom,
            &[(
                "chain".to_string(),
                wire::WireAtomPropertyValue::Str("B".to_string()),
            )],
        );
        assert_eq!(
            changed,
            AtomPropertyChangeOutcome {
                applied: true,
                changed: true,
                identity_changed: true,
            }
        );
    }

    #[test]
    fn plugin_identity_changes_reconcile_recent_atoms_but_visual_changes_do_not() {
        let mut molecule = ObjectMolecule::new("obj");
        molecule.add_atom(
            AtomBuilder::new()
                .name("CA")
                .element_symbol("C")
                .resn("GLY")
                .resv(1)
                .chain("A")
                .build(),
        );
        let mut session = Session::new();
        session
            .registry
            .add(MoleculeObject::with_name(molecule, "obj"));
        let path = patinae_scene::canonical_atom_path_for_hit(
            &PickHit {
                object_name: "obj".to_string(),
                object_type: patinae_scene::ObjectType::Molecule,
                atom_index: Some(AtomIndex(0)),

                instance: None,
                position: Default::default(),
                distance: 0.0,
            },
            session.registry.get_molecule("obj").unwrap().molecule(),
        )
        .unwrap();
        session
            .recent_atoms
            .insert(path, RecentPickLimit::Unlimited);
        let mut needs_redraw = false;
        let mut viewer = SessionAdapter {
            session: &mut session,
            render_context: None,
            default_size: (64, 64),
            needs_redraw: &mut needs_redraw,
        };

        assert!(apply_atom_property_change_batch(
            &mut viewer,
            &[wire::WireAtomPropertyChange {
                object: "obj".to_string(),
                atom_index: 0,
                changes: vec![(
                    "name".to_string(),
                    wire::WireAtomPropertyValue::Str("CA".to_string()),
                )],
            }],
        ));
        assert!(!*viewer.needs_redraw);
        assert_eq!(viewer.session().recent_atoms.len(), 1);

        apply_atom_property_change_batch(
            &mut viewer,
            &[wire::WireAtomPropertyChange {
                object: "obj".to_string(),
                atom_index: 0,
                changes: vec![("b".to_string(), wire::WireAtomPropertyValue::F32(5.0))],
            }],
        );
        assert!(*viewer.needs_redraw);
        assert_eq!(viewer.session().recent_atoms.len(), 1);

        apply_atom_property_change_batch(
            &mut viewer,
            &[wire::WireAtomPropertyChange {
                object: "obj".to_string(),
                atom_index: 0,
                changes: vec![(
                    "name".to_string(),
                    wire::WireAtomPropertyValue::Str("CB".to_string()),
                )],
            }],
        );
        assert!(viewer.session().recent_atoms.is_empty());
    }
}
