//! Shared helper functions for command implementations.
//!
//! Eliminates duplicated patterns across command modules:
//! - Object name resolution (exact → glob → selection fallback)
//! - Selection → molecule iteration with automatic invalidation
//! - Enable/disable with group awareness
//! - 1-based → 0-based state index conversion
//! - Selection result filtering (single/all molecule)
//! - Coordinate collection from selections

use lin_alg::f32::{Mat4, Vec3};
use patinae_mol::AtomIndex;
use patinae_scene::{DirtyFlags, MoleculeObject, ObjectRegistry};
use patinae_select::SelectionResult;

use crate::command::ViewerLike;
use crate::commands::selecting::{evaluate_atom_anchors, evaluate_selection};
use crate::error::{CmdError, CmdResult};

// ============================================================================
// Object name resolution
// ============================================================================

/// The result of resolving a user-supplied name against the object registry.
pub enum ResolvedNames {
    /// The literal "all" / "*" wildcard was used.
    All,
    /// One or more object names matched (exact name or glob pattern).
    Matched(Vec<String>),
    /// Nothing matched as an object name — the caller should interpret
    /// the input as a selection expression.
    Unresolved,
}

/// Resolve a user-supplied name against the object registry.
///
/// Tries, in order:
/// 1. Literal "all" / "*" → [`ResolvedNames::All`]
/// 2. Exact object name → [`ResolvedNames::Matched`] with one element
/// 3. Glob pattern via [`ObjectRegistry::matching`] → [`ResolvedNames::Matched`]
/// 4. Nothing found → [`ResolvedNames::Unresolved`]
pub fn resolve_object_names(objects: &ObjectRegistry, name: &str) -> ResolvedNames {
    if name == "all" || name == "*" {
        return ResolvedNames::All;
    }

    if objects.contains(name) {
        return ResolvedNames::Matched(vec![name.to_string()]);
    }

    let matches: Vec<String> = objects
        .matching(name)
        .iter()
        .map(|s| s.to_string())
        .collect();

    if !matches.is_empty() {
        ResolvedNames::Matched(matches)
    } else {
        ResolvedNames::Unresolved
    }
}

// ============================================================================
// Selection → molecule iteration
// ============================================================================

/// Evaluate a selection expression, then for each molecule object that has
/// matching atoms, invoke a closure with mutable access to the molecule
/// object and the selection result.
///
/// After the closure returns for each object, `invalidate(dirty_flags)` is
/// called automatically. Returns the total number of selected atoms across
/// all objects.
pub fn for_each_selected_molecule_mut(
    viewer: &mut dyn ViewerLike,
    selection: &str,
    dirty_flags: DirtyFlags,
    mut f: impl FnMut(&mut MoleculeObject, &SelectionResult),
) -> CmdResult<usize> {
    // Evaluate selection with an immutable borrow; the borrow ends once
    // results are owned.
    let selection_results = evaluate_selection(viewer, selection)?;

    // Mutate selected objects.
    let mut total = 0usize;
    for (obj_name, selected) in &selection_results {
        let count = selected.count();
        if count > 0 {
            if let Some(mol_obj) = viewer.objects_mut().get_molecule_mut(obj_name) {
                f(mol_obj, selected);
                mol_obj.invalidate(dirty_flags);
                total += count;
            }
        }
    }

    Ok(total)
}

// ============================================================================
// Enable/disable with group awareness
// ============================================================================

/// Enable or disable an object, handling groups correctly.
///
/// If the named object is a group, calls `set_group_enabled` (which
/// recursively enables/disables children); otherwise calls `enable`.
pub fn set_enabled_with_group_awareness(objects: &mut ObjectRegistry, name: &str, enabled: bool) {
    if objects.get_group(name).is_some() {
        let _ = objects.set_group_enabled(name, enabled);
    } else {
        let _ = objects.enable(name, enabled);
    }
}

// ============================================================================
// State index conversion
// ============================================================================

/// Convert a 1-based user-facing state number to a 0-based internal index.
///
/// - `0` -> `None` (all states)
/// - Positive N → `Some(N - 1)`
/// - Negative values → `None`
pub fn state_index_from_user(state_num: i64) -> Option<usize> {
    if state_num > 0 {
        Some((state_num - 1) as usize)
    } else {
        None
    }
}

// ============================================================================
// Selection result filtering
// ============================================================================

/// Get a single molecule object and its selected atom indices from selection results.
///
/// Returns an error if zero objects match or if more than one object has
/// selected atoms.
pub fn single_molecule_selection(
    results: &[(String, SelectionResult)],
    sel_name: &str,
) -> CmdResult<(String, Vec<AtomIndex>)> {
    let non_empty: Vec<(&String, Vec<AtomIndex>)> = results
        .iter()
        .filter_map(|(obj_name, sel_result)| {
            let indices: Vec<AtomIndex> = sel_result.indices().collect();
            if indices.is_empty() {
                None
            } else {
                Some((obj_name, indices))
            }
        })
        .collect();

    match non_empty.len() {
        0 => Err(CmdError::selection(format!(
            "No atoms matching '{}'",
            sel_name
        ))),
        1 => Ok((non_empty[0].0.clone(), non_empty[0].1.clone())),
        n => Err(CmdError::invalid_arg(
            "target",
            format!(
                "target must select atoms from a single object, but '{}' matches {} objects",
                sel_name, n
            ),
        )),
    }
}

/// Get all molecule objects and their selected atom indices from selection results.
///
/// Returns an error if no objects have selected atoms.
pub fn all_molecule_selections(
    results: &[(String, SelectionResult)],
    sel_name: &str,
) -> CmdResult<Vec<(String, Vec<AtomIndex>)>> {
    let selections: Vec<(String, Vec<AtomIndex>)> = results
        .iter()
        .filter_map(|(obj_name, sel_result)| {
            let indices: Vec<AtomIndex> = sel_result.indices().collect();
            if indices.is_empty() {
                None
            } else {
                Some((obj_name.clone(), indices))
            }
        })
        .collect();

    if selections.is_empty() {
        return Err(CmdError::selection(format!(
            "No atoms matching '{}'",
            sel_name
        )));
    }
    Ok(selections)
}

// ============================================================================
// Coordinate helpers
// ============================================================================

/// Transform a vector from camera space to model space.
///
/// Multiplies by the transpose of the rotation matrix (which is its inverse
/// for orthogonal matrices). Used when a command like `translate` or `rotate`
/// specifies `camera=1`.
pub fn camera_to_model_vec(rotation: &Mat4, v: Vec3) -> Vec3 {
    let r = &rotation.data;
    Vec3::new(
        r[0] * v.x + r[4] * v.y + r[8] * v.z,
        r[1] * v.x + r[5] * v.y + r[9] * v.z,
        r[2] * v.x + r[6] * v.y + r[10] * v.z,
    )
}

/// Compute the bounding box (min, max) of atoms matching a selection expression.
///
/// Returns `None` if no atoms match or all atoms lack coordinates.
pub fn selection_extent(
    viewer: &dyn ViewerLike,
    selection: &str,
) -> CmdResult<Option<(Vec3, Vec3)>> {
    let mut min = Vec3::new(f32::MAX, f32::MAX, f32::MAX);
    let mut max = Vec3::new(f32::MIN, f32::MIN, f32::MIN);
    let mut has_coords = false;

    for anchor in evaluate_atom_anchors(viewer, selection)? {
        if let Some(mol_obj) = viewer.objects().get_molecule(&anchor.object_name) {
            if let Some(coord) = mol_obj.instance_world_coord(anchor.atom_index, anchor.instance) {
                min.x = min.x.min(coord.x);
                min.y = min.y.min(coord.y);
                min.z = min.z.min(coord.z);
                max.x = max.x.max(coord.x);
                max.y = max.y.max(coord.y);
                max.z = max.z.max(coord.z);
                has_coords = true;
            }
        }
    }

    if has_coords {
        Ok(Some((min, max)))
    } else {
        Ok(None)
    }
}

/// Collect all 3D coordinates of atoms matching a selection expression
/// from the current coordinate state of each molecule.
pub fn collect_selection_coords(viewer: &dyn ViewerLike, selection: &str) -> CmdResult<Vec<Vec3>> {
    let mut coords = Vec::new();

    for anchor in evaluate_atom_anchors(viewer, selection)? {
        if let Some(mol_obj) = viewer.objects().get_molecule(&anchor.object_name) {
            if let Some(coord) = mol_obj.instance_world_coord(anchor.atom_index, anchor.instance) {
                coords.push(coord);
            }
        }
    }

    Ok(coords)
}

#[cfg(test)]
mod tests {
    use super::*;
    use patinae_mol::{AtomBuilder, CoordSet, ObjectMolecule};
    use patinae_scene::{MoleculeObject, Session, SessionAdapter};

    fn session_with_display_state(state: usize) -> Session {
        let mut mol = ObjectMolecule::new("obj");
        mol.add_atom(AtomBuilder::new().name("CA").element_symbol("C").build());
        mol.add_coord_set(CoordSet::from_vec3(&[Vec3::new(0.0, 0.0, 0.0)]));
        mol.add_coord_set(CoordSet::from_vec3(&[Vec3::new(6.0, 0.0, 0.0)]));

        let mut obj = MoleculeObject::with_name(mol, "obj");
        assert!(obj.set_display_state(state));

        let mut session = Session::new();
        session.registry.add(obj);
        session
    }

    #[test]
    fn collect_selection_coords_uses_display_state() {
        let mut session = session_with_display_state(1);
        let mut needs_redraw = false;
        let adapter = SessionAdapter {
            session: &mut session,
            render_context: None,
            default_size: (800, 600),
            needs_redraw: &mut needs_redraw,
            async_fetch_fn: None,
        };

        let coords = collect_selection_coords(&adapter, "all").unwrap();

        assert_eq!(coords.len(), 1);
        assert_eq!(coords[0].x, 6.0);
        assert_eq!(
            adapter
                .session
                .registry
                .get_molecule("obj")
                .unwrap()
                .molecule()
                .current_state,
            0
        );
    }

    #[test]
    fn camera_selection_helpers_resolve_the_selected_copy_in_world_space() {
        use patinae_mol::{InstanceGroup, InstanceTable, ObjectInstance, IDENTITY_INSTANCE};
        use patinae_scene::Object;
        let mut session = session_with_display_state(1);
        let object = session.registry.get_molecule_mut("obj").unwrap();
        let mut shifted = IDENTITY_INSTANCE;
        shifted[3][0] = 10.;
        object.state_mut().instances = Some(InstanceTable {
            groups: vec![InstanceGroup::default()],
            copies: vec![
                ObjectInstance {
                    group: 0,
                    transform: IDENTITY_INSTANCE,
                },
                ObjectInstance {
                    group: 0,
                    transform: shifted,
                },
            ],
        });
        object.state_mut().transform.data[13] = 20.;
        let mut needs_redraw = false;
        let adapter = SessionAdapter {
            session: &mut session,
            render_context: None,
            default_size: (800, 600),
            needs_redraw: &mut needs_redraw,
            async_fetch_fn: None,
        };
        let coords = collect_selection_coords(&adapter, "obj and instance 2").unwrap();
        assert_eq!(coords.len(), 1);
        assert_eq!((coords[0].x, coords[0].y), (16., 20.));
        let (min, max) = selection_extent(&adapter, "obj and instance 2")
            .unwrap()
            .unwrap();
        assert_eq!((min.x, max.x, min.y, max.y), (16., 16., 20., 20.));
    }
}
