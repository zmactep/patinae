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
use crate::commands::selecting::{evaluate_selection, visit_selected_instances};
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
    visit_selected_instances(viewer, selection, |_, object, instance, atoms| {
        let mut part_min = Vec3::new(f32::MAX, f32::MAX, f32::MAX);
        let mut part_max = Vec3::new(f32::MIN, f32::MIN, f32::MIN);
        let mut found = false;
        for atom in atoms.indices() {
            if let Some(coord) = object.instance_world_coord(atom, instance) {
                part_min.x = part_min.x.min(coord.x);
                part_min.y = part_min.y.min(coord.y);
                part_min.z = part_min.z.min(coord.z);
                part_max.x = part_max.x.max(coord.x);
                part_max.y = part_max.y.max(coord.y);
                part_max.z = part_max.z.max(coord.z);
                found = true;
            }
        }
        if found {
            min.x = min.x.min(part_min.x);
            min.y = min.y.min(part_min.y);
            min.z = min.z.min(part_min.z);
            max.x = max.x.max(part_max.x);
            max.y = max.y.max(part_max.y);
            max.z = max.z.max(part_max.z);
            has_coords = true;
        }
    })?;
    Ok(has_coords.then_some((min, max)))
}

/// Collect all 3D coordinates of atoms matching a selection expression
/// from the current coordinate state of each molecule.
pub fn collect_selection_coords(viewer: &dyn ViewerLike, selection: &str) -> CmdResult<Vec<Vec3>> {
    let mut coords = Vec::new();

    visit_selection_coords(viewer, selection, |coord| coords.push(coord))?;
    Ok(coords)
}

/// Consume world coordinates without constructing persistent atom identities.
fn visit_selection_coords(
    viewer: &dyn ViewerLike,
    selection: &str,
    mut visit: impl FnMut(Vec3),
) -> CmdResult {
    visit_selected_instances(viewer, selection, |_, object, instance, atoms| {
        for atom in atoms.indices() {
            if let Some(coord) = object.instance_world_coord(atom, instance) {
                visit(coord);
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::selecting::evaluate_atom_anchors;
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
        };
        let coords = collect_selection_coords(&adapter, "obj and instance 2").unwrap();
        assert_eq!(coords.len(), 1);
        assert_eq!((coords[0].x, coords[0].y), (16., 20.));
        let (min, max) = selection_extent(&adapter, "obj and instance 2")
            .unwrap()
            .unwrap();
        assert_eq!((min.x, max.x, min.y, max.y), (16., 16., 20., 20.));
    }

    fn session_with_transformed_subset_copies() -> Session {
        use patinae_mol::{InstanceGroup, InstanceTable, ObjectInstance, IDENTITY_INSTANCE};
        use patinae_scene::Object;

        let mut mol = ObjectMolecule::new("obj");
        for name in ["CA", "N", "O"] {
            mol.add_atom(AtomBuilder::new().name(name).element_symbol("C").build());
        }
        mol.add_coord_set(CoordSet::from_vec3(&[Vec3::new(-1., -1., -1.); 3]));
        mol.add_coord_set(CoordSet::from_vec3(&[
            Vec3::new(1., 2., 3.),
            Vec3::new(4., 5., 6.),
            Vec3::new(7., 8., 9.),
        ]));
        let mut object = MoleculeObject::new(mol);
        assert!(object.set_display_state(1));
        // Copy 2 rotates 90 degrees around Z, then translates by (10, 0, 0).
        let mut copy_transform = IDENTITY_INSTANCE;
        copy_transform[0] = [0., 1., 0., 0.];
        copy_transform[1] = [-1., 0., 0., 0.];
        copy_transform[3][0] = 10.;
        object.state_mut().instances = Some(InstanceTable {
            groups: vec![
                InstanceGroup {
                    indices: vec![0, 2],
                },
                InstanceGroup {
                    indices: vec![1, 2],
                },
            ],
            copies: vec![
                ObjectInstance {
                    group: 0,
                    transform: IDENTITY_INSTANCE,
                },
                ObjectInstance {
                    group: 1,
                    transform: copy_transform,
                },
            ],
        });
        // The object applies another 90-degree Z rotation and (0, 20, 0).
        // Noncommuting transforms catch an accidental world/copy order swap.
        let mut world_transform = IDENTITY_INSTANCE;
        world_transform[0] = [0., 1., 0., 0.];
        world_transform[1] = [-1., 0., 0., 0.];
        world_transform[3][1] = 20.;
        object.state_mut().transform = Mat4 {
            data: std::array::from_fn(|i| world_transform[i / 4][i % 4]),
        };
        let mut session = Session::new();
        session.registry.add(object);
        session
    }

    fn assert_selection_geometry(
        adapter: &SessionAdapter<'_>,
        selection: &str,
        expected_anchors: &[(u32, u32)],
        expected_sources: &[usize],
        expected_coords: &[(f32, f32, f32)],
    ) {
        let anchors = evaluate_atom_anchors(adapter, selection).unwrap();
        assert!(anchors.iter().all(|anchor| anchor.object_name == "obj"));
        assert_eq!(
            anchors
                .iter()
                .map(|anchor| (anchor.instance.unwrap(), anchor.atom_index.0))
                .collect::<Vec<_>>(),
            expected_anchors,
            "anchors: {selection}"
        );
        let sources = evaluate_selection(adapter, selection).unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].0, "obj");
        assert_eq!(
            sources[0].1.raw_indices().collect::<Vec<_>>(),
            expected_sources,
            "source projection: {selection}"
        );
        let coords = collect_selection_coords(adapter, selection).unwrap();
        assert_eq!(
            coords.iter().map(|p| (p.x, p.y, p.z)).collect::<Vec<_>>(),
            expected_coords,
            "coordinates: {selection}"
        );
        let expected_bounds = expected_coords
            .iter()
            .copied()
            .fold(None, |bounds, (x, y, z)| {
                Some(match bounds {
                    None => ((x, y, z), (x, y, z)),
                    Some(((a, b, c), (d, e, f))) => (
                        (x.min(a), y.min(b), z.min(c)),
                        (x.max(d), y.max(e), z.max(f)),
                    ),
                })
            });
        assert_eq!(
            selection_extent(adapter, selection)
                .unwrap()
                .map(|(min, max)| ((min.x, min.y, min.z), (max.x, max.y, max.z))),
            expected_bounds,
            "extent: {selection}"
        );
    }

    #[test]
    fn selection_geometry_respects_copy_subsets_display_state_and_transform_order() {
        let mut session = session_with_transformed_subset_copies();
        let mut needs_redraw = false;
        let adapter = SessionAdapter {
            session: &mut session,
            render_context: None,
            default_size: (800, 600),
            needs_redraw: &mut needs_redraw,
        };
        assert_selection_geometry(
            &adapter,
            "all",
            &[(0, 0), (0, 2), (1, 1), (1, 2)],
            &[0, 1, 2],
            &[
                (-2., 21., 3.),
                (-8., 27., 9.),
                (-4., 25., 6.),
                (-7., 22., 9.),
            ],
        );
        assert_selection_geometry(&adapter, "obj and instance 2 and name CA", &[], &[], &[]);
        assert_selection_geometry(&adapter, "none", &[], &[], &[]);
    }

    #[test]
    fn wildcard_and_exact_object_selections_share_subset_and_zero_copy_semantics() {
        use patinae_scene::Object;

        let mut session = session_with_transformed_subset_copies();
        session
            .registry
            .get_molecule_mut("obj")
            .unwrap()
            .state_mut()
            .instances
            .as_mut()
            .unwrap()
            .copies
            .truncate(1);
        let mut needs_redraw = false;
        let adapter = SessionAdapter {
            session: &mut session,
            render_context: None,
            default_size: (800, 600),
            needs_redraw: &mut needs_redraw,
        };
        // Wildcards must select represented source atoms just like an exact
        // object name; atom 1 exists in storage but belongs to no displayed copy.
        for selection in ["obj", "obj*"] {
            assert_selection_geometry(
                &adapter,
                selection,
                &[(0, 0), (0, 2)],
                &[0, 2],
                &[(-2., 21., 3.), (-8., 27., 9.)],
            );
        }
        adapter
            .session
            .registry
            .get_molecule_mut("obj")
            .unwrap()
            .state_mut()
            .instances
            .as_mut()
            .unwrap()
            .copies
            .clear();
        // Zero copies retain the source object but cannot produce selected
        // displayed atoms, phantom anchors, coordinates, or an extent.
        for selection in ["obj", "obj*"] {
            assert_selection_geometry(&adapter, selection, &[], &[], &[]);
        }
    }

    #[test]
    fn selection_geometry_resolves_named_boolean_expressions_before_source_projection() {
        let mut session = session_with_transformed_subset_copies();
        let expression = "(instance 1 and name O) or (instance 2 and name N)";
        // The cached mask has lost copy identity. Geometry and Boolean algebra
        // must resolve the expression per copy instead of reusing this union.
        session.selections.define_with_results(
            "chosen",
            expression,
            vec![(
                "obj".to_string(),
                SelectionResult::from_indices(3, [AtomIndex(1), AtomIndex(2)].into_iter()),
            )],
        );
        session.selections.define("second", "chosen and instance 2");
        let mut needs_redraw = false;
        let adapter = SessionAdapter {
            session: &mut session,
            render_context: None,
            default_size: (800, 600),
            needs_redraw: &mut needs_redraw,
        };
        assert_selection_geometry(
            &adapter,
            expression,
            &[(0, 2), (1, 1)],
            &[1, 2],
            &[(-8., 27., 9.), (-4., 25., 6.)],
        );
        assert_selection_geometry(
            &adapter,
            "chosen",
            &[(0, 2), (1, 1)],
            &[1, 2],
            &[(-8., 27., 9.), (-4., 25., 6.)],
        );
        assert_selection_geometry(&adapter, "second", &[(1, 1)], &[1], &[(-4., 25., 6.)]);
        assert_selection_geometry(
            &adapter,
            "chosen and not instance 2",
            &[(0, 2)],
            &[2],
            &[(-8., 27., 9.)],
        );
        assert_selection_geometry(
            &adapter,
            "not chosen",
            &[(0, 0), (1, 2)],
            &[0, 2],
            &[(-2., 21., 3.), (-7., 22., 9.)],
        );
    }
}
