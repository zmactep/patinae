//! Crystal symmetry commands: symexp
//!
//! Generates symmetry-related copies of a molecule using space group operations
//! and unit cell translations. Standard crystallographic technique — see
//! Rupp, "Biomolecular Crystallography", Garland Science, 2010, Ch. 6.

use std::sync::Arc;

use lin_alg::f32::{Mat4, Vec3, Vec4};

use patinae_algos::crystal::CrystalCell;
use patinae_mol::spatial::SpatialGrid;
use patinae_mol::{translation_matrix, RepMask};
use patinae_scene::{MoleculeObject, Object};

use crate::args::ParsedCommand;
use crate::command::{ArgHint, Command, CommandContext, CommandRegistry, ViewerLike};
use crate::command_help;
use crate::commands::selecting::evaluate_selection;
use crate::error::{CmdError, CmdResult};

pub fn register(registry: &mut CommandRegistry) {
    registry.register(SymExpCommand);
    registry.register(BioUnitCommand);
    registry.register(MaterializeCommand);
}

// ============================================================================
// symexp command
// ============================================================================

struct SymExpCommand;

impl Command for SymExpCommand {
    fn name(&self) -> &str {
        "symexp"
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[
            ArgHint::None,      // prefix
            ArgHint::Object,    // object
            ArgHint::Selection, // selection
            ArgHint::None,      // cutoff
        ]
    }

    command_help! {
        CMD "symexp"
        DESCRIPTION [
            "generates symmetry-related copies of an object within a",
            "specified distance cutoff. The new objects are named using the",
            "given prefix followed by symmetry operation and translation codes.",
        ]
        USAGE [
            "symexp prefix, object, selection, cutoff [, segi [, quiet]]",
        ]
        REQUIRED [
            { "prefix", "string", "prefix for new object names" },
            { "object", "string", "source object with crystallographic symmetry" },
            { "selection", "string", "atom selection defining the region of interest" },
        ]
        OPTIONAL [
            { "cutoff", "float", "distance cutoff in Angstroms", "10.0" },
            { "segi", "0/1", "store symmetry info in segment identifier", "0" },
            { "quiet", "0/1", "suppress output messages", "1" },
        ]
        EXAMPLES [
            "symexp sym, 1oky, chain A, 20.0",
            "symexp s, myprotein, all, 15.0, segi=1",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let prefix = args
            .str_arg(0, "prefix")
            .ok_or_else(|| CmdError::missing_argument("prefix".to_string()))?;
        let object = args
            .str_arg(1, "object")
            .ok_or_else(|| CmdError::missing_argument("object".to_string()))?;
        let selection = args
            .str_arg(2, "selection")
            .ok_or_else(|| CmdError::missing_argument("selection".to_string()))?;
        let cutoff = args.float_arg_or(3, "cutoff", 10.0) as f32;
        let segi = args.int_arg_or(4, "segi", 0) != 0;
        let quiet = args
            .int_arg(5, "quiet")
            .map(|v| v != 0)
            .unwrap_or(ctx.quiet);

        // Clone source data to release borrow on ctx.viewer
        let source = extract_source_data(ctx, object)?;

        let results = evaluate_selection(ctx.viewer, selection)?;
        let (sel_center, sel_coords) = collect_selection_coords(ctx.viewer, &results);

        if sel_coords.is_empty() {
            return Err(CmdError::execution("No atoms in selection!"));
        }

        let sel_frac = source.cell.to_fractional(sel_center);
        let grid = build_spatial_grid(&sel_coords, cutoff);
        let cutoff_sq = cutoff * cutoff;

        if !quiet {
            ctx.print(" SymExp: Generating symmetry mates...");
        }

        let mates = generate_symmetry_mates(
            &source,
            sel_frac,
            &grid,
            &sel_coords,
            cutoff_sq,
            prefix,
            segi,
        );

        let count = mates.len();
        let mut objects: Vec<Box<dyn Object>> = Vec::with_capacity(count);
        for mate in mates {
            let mut obj = MoleculeObject::from_raw_with_name(mate.molecule, &mate.name);
            obj.set_visible_reps(source.visible_reps);
            objects.push(Box::new(obj));
        }
        ctx.viewer.insert_objects(objects);

        ctx.viewer.request_redraw();

        if !quiet {
            ctx.print(&format!(
                " SymExp: Created {} symmetry mate objects.",
                count
            ));
        }

        Ok(())
    }
}

// ============================================================================
// Data extraction
// ============================================================================

/// All data needed from the source molecule, cloned to avoid borrow conflicts.
struct SourceData {
    molecule: patinae_mol::ObjectMolecule,
    visible_reps: RepMask,
    symops: Vec<Mat4>,
    cell: CrystalCell,
    state_centers: Vec<Vec3>,
}

fn extract_source_data(
    ctx: &CommandContext<'_, '_, dyn ViewerLike + '_>,
    object: &str,
) -> Result<SourceData, CmdError> {
    let mol_obj = ctx
        .viewer
        .objects()
        .get_molecule(object)
        .ok_or_else(|| CmdError::object_not_found(object.to_string()))?;

    mol_obj.require_explicit().map_err(CmdError::execution)?;
    let mol = mol_obj.molecule();

    let symmetry = mol
        .symmetry
        .as_ref()
        .or_else(|| {
            (0..mol.state_count())
                .find_map(|s| mol.get_coord_set(s).and_then(|cs| cs.symmetry.as_ref()))
        })
        .ok_or_else(|| CmdError::execution("No symmetry loaded!"))?;

    let symops =
        patinae_algos::space_groups::get_symops(&symmetry.space_group).ok_or_else(|| {
            CmdError::execution(format!("Unknown space group: '{}'", symmetry.space_group))
        })?;

    let cell = CrystalCell::new(
        Vec3::new(
            symmetry.cell_lengths[0],
            symmetry.cell_lengths[1],
            symmetry.cell_lengths[2],
        ),
        Vec3::new(
            symmetry.cell_angles[0],
            symmetry.cell_angles[1],
            symmetry.cell_angles[2],
        ),
    );

    let state_centers: Vec<Vec3> = (0..mol.state_count())
        .map(|s| {
            mol.get_coord_set(s)
                .map(|cs| cs.center())
                .unwrap_or(Vec3::new(0.0, 0.0, 0.0))
        })
        .collect();

    Ok(SourceData {
        molecule: mol.clone(),
        visible_reps: mol_obj.visible_reps(),
        symops,
        cell,
        state_centers,
    })
}

// ============================================================================
// Selection helpers
// ============================================================================

/// Collect coordinates and centroid from selection results.
fn collect_selection_coords(
    viewer: &dyn ViewerLike,
    results: &[(String, patinae_select::SelectionResult)],
) -> (Vec3, Vec<Vec3>) {
    let mut coords = Vec::new();
    let mut sum = Vec3::new(0.0, 0.0, 0.0);

    for (obj_name, selection) in results {
        if selection.count() == 0 {
            continue;
        }
        let Some(mol_obj) = viewer.objects().get_molecule(obj_name) else {
            continue;
        };
        let mol = mol_obj.molecule();

        for state in 0..mol.state_count() {
            let Some(cs) = mol.get_coord_set(state) else {
                continue;
            };
            for idx in selection.indices() {
                if let Some(coord) = cs.get_atom_coord(idx) {
                    sum += coord;
                    coords.push(coord);
                }
            }
        }
    }

    let center = if coords.is_empty() {
        Vec3::new(0.0, 0.0, 0.0)
    } else {
        sum * (1.0 / coords.len() as f32)
    };

    (center, coords)
}

fn build_spatial_grid(coords: &[Vec3], cutoff: f32) -> SpatialGrid {
    let mut grid = SpatialGrid::with_capacity(cutoff, coords.len());
    for (i, coord) in coords.iter().enumerate() {
        grid.insert(*coord, i);
    }
    grid
}

// ============================================================================
// Symmetry mate generation
// ============================================================================

/// A single symmetry-expanded molecule ready to be added to the scene.
struct SymmetryMate {
    name: String,
    molecule: patinae_mol::ObjectMolecule,
}

/// Unit cell translation offsets to search: -1, 0, +1 in each axis.
const CELL_OFFSETS: std::ops::RangeInclusive<i32> = -1..=1;

fn generate_symmetry_mates(
    source: &SourceData,
    sel_frac: Vec3,
    grid: &SpatialGrid,
    sel_coords: &[Vec3],
    cutoff_sq: f32,
    prefix: &str,
    segi: bool,
) -> Vec<SymmetryMate> {
    let r2f = source.cell.real_to_frac_4x4();
    let f2r = source.cell.frac_to_real_4x4();
    let mut mates = Vec::new();

    for x in CELL_OFFSETS {
        for y in CELL_OFFSETS {
            for z in CELL_OFFSETS {
                let cell_shift = Vec3::new(x as f32, y as f32, z as f32);
                for (op_idx, symop) in source.symops.iter().enumerate() {
                    let matrices = compute_state_transforms(
                        &r2f,
                        &f2r,
                        symop,
                        &source.state_centers,
                        sel_frac,
                        cell_shift,
                    );

                    if matrices.iter().all(is_approx_identity) {
                        continue;
                    }

                    if !any_atom_within_cutoff(
                        &source.molecule,
                        &matrices,
                        grid,
                        sel_coords,
                        cutoff_sq,
                    ) {
                        continue;
                    }

                    let molecule =
                        build_transformed_mol(&source.molecule, &matrices, segi, op_idx, x, y, z);
                    let name = format!(
                        "{}{:02}{:02}{:02}{:02}",
                        prefix,
                        op_idx,
                        x + 1,
                        y + 1,
                        z + 1
                    );

                    mates.push(SymmetryMate { name, molecule });
                }
            }
        }
    }

    mates
}

/// Compute the Cartesian transformation matrix for each state.
///
/// For each state center, the pipeline is:
///   real→frac → apply symop → shift into selection's unit cell → frac→real
fn compute_state_transforms(
    r2f: &Mat4,
    f2r: &Mat4,
    symop: &Mat4,
    state_centers: &[Vec3],
    sel_frac: Vec3,
    cell_shift: Vec3,
) -> Vec<Mat4> {
    state_centers
        .iter()
        .map(|center| {
            // Combine symop with real-to-fractional
            let frac_transform = symop.clone() * r2f.clone();

            // Find where this state's center lands in fractional space
            let center_frac = transform_point(&frac_transform, *center);

            // Round into the same unit cell as the selection, plus explicit cell offset
            let rounding_shift = Vec3::new(
                (sel_frac.x - center_frac.x).round() + cell_shift.x,
                (sel_frac.y - center_frac.y).round() + cell_shift.y,
                (sel_frac.z - center_frac.z).round() + cell_shift.z,
            );

            // Full pipeline: frac→real * shift * symop * real→frac
            f2r.clone() * translation_matrix(rounding_shift) * frac_transform
        })
        .collect()
}

// ============================================================================
// Distance check
// ============================================================================

/// Returns true if any transformed source atom is within `cutoff_sq` of a selection atom.
fn any_atom_within_cutoff(
    mol: &patinae_mol::ObjectMolecule,
    matrices: &[Mat4],
    grid: &SpatialGrid,
    sel_coords: &[Vec3],
    cutoff_sq: f32,
) -> bool {
    let mut neighbors = Vec::new();

    for (state_idx, mat) in matrices.iter().enumerate() {
        let Some(cs) = mol.get_coord_set(state_idx) else {
            continue;
        };

        for coord in cs.iter() {
            let transformed = transform_point(mat, coord);

            grid.query_neighbors(transformed, &mut neighbors);
            for &idx in &neighbors {
                if (transformed - sel_coords[idx]).magnitude_squared() <= cutoff_sq {
                    return true;
                }
            }
        }
    }

    false
}

// ============================================================================
// Molecule construction
// ============================================================================

fn build_transformed_mol(
    src: &patinae_mol::ObjectMolecule,
    matrices: &[Mat4],
    segi: bool,
    op_idx: usize,
    x: i32,
    y: i32,
    z: i32,
) -> patinae_mol::ObjectMolecule {
    let mut mol = src.clone();

    for (state_idx, mat) in matrices.iter().enumerate() {
        if !is_approx_identity(mat) {
            mol.transform(state_idx, mat);
        }
    }

    if segi {
        let label = segi_label(op_idx, x, y, z);
        for atom in mol.atoms_mut() {
            Arc::make_mut(&mut atom.residue).segi = label.clone();
        }
    }

    mol
}

// ============================================================================
// Utilities
// ============================================================================

/// Transform a point by a 4×4 matrix (homogeneous, w=1).
fn transform_point(m: &Mat4, v: Vec3) -> Vec3 {
    let r = m.clone() * Vec4::new(v.x, v.y, v.z, 1.0);
    Vec3::new(r.x, r.y, r.z)
}

fn is_approx_identity(m: &Mat4) -> bool {
    const EPS: f32 = 1e-4;
    m.data
        .iter()
        .zip(Mat4::new_identity().data.iter())
        .all(|(a, b)| (a - b).abs() < EPS)
}

/// Encode a symmetry operation + cell offset as a 4-character segment identifier.
///
/// Format: `<op><x+1><y+1><z+1>` where op is `A`..`Z`, `0`..`9`, `a`..`z`.
fn segi_label(symop_idx: usize, x: i32, y: i32, z: i32) -> String {
    let op_char = match symop_idx {
        0..26 => (b'A' + symop_idx as u8) as char,
        26..36 => (b'0' + (symop_idx - 26) as u8) as char,
        _ => (b'a' + (symop_idx - 36) as u8) as char,
    };
    format!("{}{}{}{}", op_char, x + 1, y + 1, z + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_segi_label() {
        assert_eq!(segi_label(0, 0, 0, 0), "A111");
        assert_eq!(segi_label(3, -1, 0, 1), "D012");
        assert_eq!(segi_label(25, 1, 1, 1), "Z222");
    }
}

/// Creates an assembly snapshot without duplicating its source atom table.
struct BioUnitCommand;

impl Command for BioUnitCommand {
    fn name(&self) -> &str {
        "biounit"
    }
    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Object, ArgHint::None, ArgHint::None]
    }
    command_help! {
        CMD "biounit"
        DESCRIPTION ["creates an instanced biological assembly from the selected source state.", "Styles are shared by copies; materialize enables independent structural editing."]
        USAGE ["biounit source [, name [, assembly [, state]]]"]
        REQUIRED [{ "source", "object", "source molecule containing assembly definitions" }]
        OPTIONAL [
            { "name", "string", "new unique object name", "source_biounit" },
            { "assembly", "string", "file-defined assembly identifier", "first available" },
            { "state", "integer", "one-based source state", "current" },
        ]
        EXAMPLES ["biounit protein", "biounit protein, name=assembly, assembly=1"]
    }
    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let source = args
            .str_arg(0, "source")
            .ok_or_else(|| CmdError::missing_argument("source"))?;
        if matches!(args.arg(1, "name"), Some(value) if !matches!(value, crate::args::ArgValue::String(_) | crate::args::ArgValue::None))
        {
            return Err(CmdError::invalid_arg("name", "expected an object name"));
        }
        if matches!(args.arg(3, "state"), Some(value) if !matches!(value, crate::args::ArgValue::Int(_) | crate::args::ArgValue::None))
        {
            return Err(CmdError::invalid_arg(
                "state",
                "expected a positive integer state",
            ));
        }
        let name = if let Some(name) = args.str_arg(1, "name") {
            if name.trim().is_empty() {
                return Err(CmdError::invalid_arg(
                    "name",
                    "object name must not be empty",
                ));
            }
            if ctx.viewer.objects().get(name).is_some() {
                return Err(CmdError::execution(format!(
                    "object '{name}' already exists"
                )));
            }
            name.to_string()
        } else {
            let base = format!("{source}_biounit");
            let mut name = base.clone();
            let mut ordinal = 2u64;
            while ctx.viewer.objects().get(&name).is_some() {
                name = format!("{base}_{ordinal}");
                ordinal += 1;
            }
            name
        };
        let object = ctx
            .viewer
            .objects()
            .get_molecule(source)
            .ok_or_else(|| CmdError::object_not_found(source))?;
        object.require_explicit().map_err(CmdError::execution)?;
        let molecule = object.molecule();
        let metadata = &molecule.assembly;
        let assembly = match args.arg(2, "assembly") {
            Some(crate::args::ArgValue::String(value)) => value.clone(),
            Some(crate::args::ArgValue::Int(value)) => value.to_string(),
            None | Some(crate::args::ArgValue::None) => metadata
                .definitions
                .first()
                .map(|definition| definition.id.clone())
                .ok_or_else(|| {
                    CmdError::execution("source has no biological assembly definitions")
                })?,
            Some(_) => {
                return Err(CmdError::invalid_arg(
                    "assembly",
                    "expected an assembly identifier",
                ))
            }
        };
        let table = metadata
            .instance_table(&assembly, molecule.atom_count())
            .map_err(|error| CmdError::execution(error.to_string()))?;
        let state = match args.int_arg(3, "state") {
            Some(state) if state > 0 => usize::try_from(state - 1)
                .map_err(|_| CmdError::invalid_arg("state", "state is out of range"))?,
            Some(_) => return Err(CmdError::invalid_arg("state", "state must be positive")),
            None => object.display_state(),
        };
        let coordinates = molecule
            .get_coord_set(state)
            .ok_or_else(|| CmdError::invalid_arg("state", "source state does not exist"))?
            .clone();
        let mut snapshot = molecule.clone();
        snapshot.clear_coord_sets();
        snapshot.add_coord_set(coordinates);
        snapshot.name = name.clone();
        let mut result = MoleculeObject::from_raw(snapshot);
        *result.state_mut() = object.state().clone();
        result.set_surface_quality(object.surface_quality());
        result.state_mut().instances = Some(table);
        if let Some(overrides) = object.overrides() {
            *result.get_or_create_overrides() = overrides.clone();
        }
        ctx.viewer.insert_object(Box::new(result));
        ctx.viewer.request_redraw();
        if !ctx.quiet {
            ctx.print(&format!(" Created biological assembly '{name}'."));
        }
        Ok(())
    }
}

struct MaterializeCommand;

impl Command for MaterializeCommand {
    fn name(&self) -> &str {
        "materialize"
    }
    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Object]
    }
    command_help! {
        CMD "materialize"
        DESCRIPTION ["expands an instanced molecule in place into independently editable copies."]
        REQUIRED [{ "object", "string", "assembly object to expand" }]
        OPTIONAL []
        EXAMPLES ["materialize protein_biounit"]
    }
    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let name = args
            .str_arg(0, "object")
            .ok_or_else(|| CmdError::missing_argument("object"))?;
        let already_explicit = ctx
            .viewer
            .objects()
            .get(name)
            .is_some_and(|object| object.state().instances.is_none());
        ctx.viewer
            .session_mut()
            .materialize_object(name)
            .map_err(CmdError::execution)?;
        if already_explicit && !ctx.quiet {
            ctx.print(&format!(" Object '{name}' is already explicit."));
        }
        ctx.viewer.request_redraw();
        Ok(())
    }
}

#[cfg(test)]
mod assembly_command_tests {
    use super::*;
    use crate::{CommandExecutor, CommandOutput};
    use patinae_mol::{
        AssemblyDefinition, AssemblyGroup, AtomBuilder, AtomIndex, CoordSet, Element,
        ObjectMolecule, IDENTITY_INSTANCE,
    };
    use patinae_scene::{
        resolve_measurement_entity_value, MeasurementResolveOptions, Session, SessionAdapter,
    };

    fn fixture() -> Session {
        let mut molecule = ObjectMolecule::new("source");
        molecule.add_atom(
            AtomBuilder::new()
                .name("CA")
                .element(Element::Carbon)
                .resn("GLY")
                .resv(1)
                .chain("A")
                .build(),
        );
        molecule.add_coord_set(CoordSet::from_vec3(&[Vec3::new(1.0, 0.0, 0.0)]));
        molecule.add_coord_set(CoordSet::from_vec3(&[Vec3::new(3.0, 0.0, 0.0)]));
        let mut shifted = IDENTITY_INSTANCE;
        shifted[3][0] = 10.0;
        molecule.assembly.chains.insert("A".into(), vec![0]);
        molecule.assembly.definitions.push(AssemblyDefinition {
            id: "1".into(),
            details: "dimer".into(),
            groups: vec![AssemblyGroup {
                chains: vec!["A".into()],
                transforms: vec![IDENTITY_INSTANCE, shifted],
            }],
        });
        let mut session = Session::new();
        session.registry.add(MoleculeObject::from_raw(molecule));
        session
    }

    fn run(session: &mut Session, command: &str) -> Result<CommandOutput, CmdError> {
        let mut needs_redraw = false;
        let mut adapter = SessionAdapter {
            session,
            render_context: None,
            default_size: (64, 64),
            needs_redraw: &mut needs_redraw,
        };
        CommandExecutor::new().do_with_options(&mut adapter, command, false)
    }

    fn distance(session: &Session, name: &str) -> f64 {
        let measurement = session.registry.get_measurement(name).unwrap();
        resolve_measurement_entity_value(
            &session.registry,
            measurement.kind(),
            &measurement.entries()[0],
            MeasurementResolveOptions::default(),
        )
        .unwrap()
    }

    #[test]
    fn assembly_is_a_single_state_snapshot_with_shared_copy_styles() {
        let mut session = fixture();
        run(&mut session, "biounit source, assembly_view, state=2").unwrap();
        let assembly = session.registry.get_molecule("assembly_view").unwrap();
        assert_eq!(assembly.molecule().atom_count(), 1);
        assert_eq!(assembly.molecule().state_count(), 1);
        assert_eq!(assembly.displayed_atom_count(), 2);
        assert_eq!(
            assembly.instance_coord(AtomIndex(0), Some(1)).unwrap().x,
            13.0
        );
        assert_eq!(
            session
                .registry
                .get_molecule("source")
                .unwrap()
                .molecule()
                .state_count(),
            2
        );
        run(&mut session, "color red, assembly_view and instance 2").unwrap();
        run(&mut session, "show spheres, assembly_view and instance 1").unwrap();
        let assembly = session.registry.get_molecule("assembly_view").unwrap();
        assert_eq!(assembly.molecule().atom_count(), 1);
        assert!(assembly
            .molecule()
            .get_atom(AtomIndex(0))
            .unwrap()
            .repr
            .visible_reps
            .is_visible(RepMask::SPHERES));
        run(&mut session, "materialize assembly_view").unwrap();
        let expanded = session
            .registry
            .get_molecule("assembly_view")
            .unwrap()
            .molecule();
        assert_eq!(expanded.atom_count(), 2);
        assert_eq!(
            expanded.get_atom(AtomIndex(0)).unwrap().repr.colors.base,
            expanded.get_atom(AtomIndex(1)).unwrap().repr.colors.base
        );
        assert!(expanded
            .atoms()
            .all(|atom| atom.repr.visible_reps.is_visible(RepMask::SPHERES)));
    }

    #[test]
    fn copy_picks_and_measurements_keep_world_positions_after_materialization() {
        let mut session = fixture();
        run(&mut session, "biounit source, assembly_view").unwrap();
        let mut transform = Mat4::new_identity();
        transform.data[12] = 7.0;
        session
            .registry
            .get_molecule_mut("assembly_view")
            .unwrap()
            .state_mut()
            .set_transform(transform);
        let render_id = session.registry.render_id("assembly_view");
        run(
            &mut session,
            "pick assembly_view and name CA and instance 1",
        )
        .unwrap();
        run(
            &mut session,
            "pick assembly_view and name CA and instance 2",
        )
        .unwrap();
        run(
            &mut session,
            "distance across, pk1 and name CA, pk2 and name CA",
        )
        .unwrap();
        assert!((distance(&session, "across") - 10.0).abs() < 1e-8);
        let before = session
            .resolved_recent_atom_anchors()
            .iter()
            .map(|anchor| {
                session
                    .registry
                    .get_molecule(&anchor.object_name)
                    .unwrap()
                    .instance_world_coord(anchor.atom_index, anchor.instance)
                    .unwrap()
            })
            .collect::<Vec<_>>();
        assert_eq!(before[0].x, 8.0);
        run(&mut session, "materialize assembly_view").unwrap();
        assert_eq!(session.registry.render_id("assembly_view"), render_id);
        assert!((distance(&session, "across") - 10.0).abs() < 1e-8);
        let anchors = session.resolved_recent_atom_anchors();
        assert_eq!(anchors.len(), 2);
        for (anchor, expected) in anchors.iter().zip(before) {
            assert_eq!(anchor.instance, None);
            assert!(!anchor.is_orphaned());
            let actual = session
                .registry
                .get_molecule(&anchor.object_name)
                .unwrap()
                .instance_world_coord(anchor.atom_index, None)
                .unwrap();
            assert_eq!(actual, expected);
        }
        let restored: Session =
            rmp_serde::from_slice(&rmp_serde::to_vec_named(&session).unwrap()).unwrap();
        assert!((distance(&restored, "across") - 10.0).abs() < 1e-8);
    }

    #[test]
    fn structural_batches_fail_before_any_source_or_destination_change() {
        let mut session = fixture();
        run(&mut session, "biounit source, assembly_view").unwrap();
        for command in [
            "remove all",
            "extract stolen, all",
            "delete_states *, 1",
            "dss all",
            "translate [1, 0, 0], source or (assembly_view and instance 1), camera=0",
        ] {
            let before = rmp_serde::to_vec_named(&session).unwrap();
            let error = run(&mut session, command).unwrap_err();
            assert!(
                error.to_string().contains("materialize"),
                "{command}: {error}"
            );
            assert_eq!(
                rmp_serde::to_vec_named(&session).unwrap(),
                before,
                "{command}"
            );
        }
        assert!(session.registry.get("stolen").is_none());
        assert!(run(&mut session, "pick assembly_view and name CA").is_err());
        assert!(session.recent_atoms.is_empty());
    }

    #[test]
    fn rigid_moves_preserve_instances_and_materialized_world_coordinates() {
        let mut session = fixture();
        run(&mut session, "biounit source, assembly_view").unwrap();
        let source_before = rmp_serde::to_vec_named(
            session
                .registry
                .get_molecule("assembly_view")
                .unwrap()
                .molecule(),
        )
        .unwrap();
        run(&mut session, "translate [2, 3, 0], assembly_view, camera=0").unwrap();
        run(
            &mut session,
            "rotate z, 90, assembly_view, camera=0, origin=[0,0,0]",
        )
        .unwrap();
        run(
            &mut session,
            "transform_selection assembly_view, [1,0,0,5, 0,1,0,0, 0,0,1,0, 0,0,0,1], homogenous=1",
        )
        .unwrap();
        let object = session.registry.get_molecule("assembly_view").unwrap();
        assert_eq!(
            rmp_serde::to_vec_named(object.molecule()).unwrap(),
            source_before
        );
        for (copy, y) in [(0, 3.0), (1, 13.0)] {
            let p = object
                .instance_world_coord(AtomIndex(0), Some(copy))
                .unwrap();
            assert!((p - Vec3::new(2.0, y, 0.0)).magnitude() < 1e-4);
        }
        run(
            &mut session,
            "translate [100, 200, 0], assembly_view, camera=0, center=1",
        )
        .unwrap();
        let object = session.registry.get_molecule("assembly_view").unwrap();
        let (min, max) = object.extent().unwrap();
        assert!(((min + max) * 0.5 - Vec3::new(100.0, 200.0, 0.0)).magnitude() < 1e-4);
        let before = [0, 1].map(|copy| {
            object
                .instance_world_coord(AtomIndex(0), Some(copy))
                .unwrap()
        });
        run(&mut session, "materialize assembly_view").unwrap();
        run(&mut session, "translate [7, 0, 0], assembly_view, camera=0").unwrap();
        for (atom, expected) in before.iter().enumerate() {
            let actual = session
                .registry
                .get_molecule("assembly_view")
                .unwrap()
                .instance_world_coord(AtomIndex(atom as u32), None)
                .unwrap();
            assert!(
                (actual - (*expected + Vec3::new(7.0, 0.0, 0.0))).magnitude() < 1e-4,
                "actual={actual:?}, before={expected:?}"
            );
        }
    }

    #[test]
    fn instance_movement_validates_batches_and_accepts_complete_named_selections() {
        let mut session = fixture();
        run(&mut session, "biounit source, assembly_view").unwrap();
        for command in [
            "rotate z, 45, source or (assembly_view and instance 1)",
            "translate [1,0,0], all, state=2, camera=0",
            "transform_selection all, [2,0,0,0, 0,1,0,0, 0,0,1,0, 0,0,0,1], homogenous=1",
            "transform_selection all, [1,0,0,0, 0,1,0,0, 0,0,1,0, 1,0,0,1], homogenous=1",
        ] {
            let before = rmp_serde::to_vec_named(&session).unwrap();
            assert!(run(&mut session, command).is_err(), "{command}");
            assert_eq!(
                rmp_serde::to_vec_named(&session).unwrap(),
                before,
                "{command}"
            );
        }
        run(
            &mut session,
            "select complete, assembly_view and (instance 1 or instance 2)",
        )
        .unwrap();
        run(&mut session, "translate [1,0,0], complete, camera=0").unwrap();
        run(&mut session, "translate [1,0,0], assembly_*, camera=0").unwrap();
        run(&mut session, "translate [1,0,0], all, camera=0").unwrap();
        let p = session
            .registry
            .get_molecule("assembly_view")
            .unwrap()
            .instance_world_coord(AtomIndex(0), Some(0))
            .unwrap();
        assert!((p.x - 4.0).abs() < 1e-4);
    }

    #[test]
    fn biounit_after_alignment_preserves_the_assembly_frame() {
        let mut session = fixture();
        let points = [
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(2.0, 1.0, 0.0),
            Vec3::new(1.0, 0.0, 2.0),
        ];
        let object = session.registry.get_molecule_mut("source").unwrap();
        let mol = object.molecule_mut();
        for residue in 2..=3 {
            mol.add_atom(
                AtomBuilder::new()
                    .name("CA")
                    .element(Element::Carbon)
                    .resn("GLY")
                    .resv(residue)
                    .chain("A")
                    .build(),
            );
        }
        mol.clear_coord_sets();
        mol.add_coord_set(CoordSet::from_vec3(&points));
        mol.classify_atoms();
        mol.assembly.chains.insert("A".into(), vec![0, 1, 2]);
        let mut target = mol.clone();
        target.name = "target".into();
        target.assembly = Default::default();
        target.clear_coord_sets();
        // A noncommuting rotation and translation exposes untransformed copy operators.
        let target_points = points.map(|p| Vec3::new(-p.y + 7.0, p.x + 11.0, p.z - 3.0));
        target.add_coord_set(CoordSet::from_vec3(&target_points));
        session.registry.add(MoleculeObject::from_raw(target));
        run(&mut session, "align source, target, cycles=0").unwrap();
        run(&mut session, "biounit source, assembly_view").unwrap();
        run(&mut session, "delete source").unwrap();
        let object = session.registry.get_molecule("assembly_view").unwrap();
        for (i, expected) in target_points.iter().enumerate() {
            for copy in 0..2 {
                let actual = object
                    .instance_world_coord(AtomIndex(i as u32), Some(copy))
                    .unwrap();
                assert!((actual - (*expected + Vec3::new(0.0, 10.0 * copy as f32, 0.0))).magnitude() < 1e-3, "atom={i}, copy={copy}, actual={actual:?}, expected={expected:?}, transform={:?}", object.state().transform);
            }
        }
        // Fitting one copy moves the complete assembly without editing its source.
        let source_before = rmp_serde::to_vec_named(object.molecule()).unwrap();
        let copies_before = object.state().instances.clone();
        for method in ["kabsch", "sequence"] {
            run(
                &mut session,
                "rotate x, 37, assembly_view, camera=0, origin=[0,0,0]",
            )
            .unwrap();
            run(&mut session, "translate [2,-3,4], assembly_view, camera=0").unwrap();
            run(
                &mut session,
                &format!("align assembly_view and instance 2, target, method={method}, cycles=0"),
            )
            .unwrap();
            let object = session.registry.get_molecule("assembly_view").unwrap();
            assert_eq!(
                rmp_serde::to_vec_named(object.molecule()).unwrap(),
                source_before
            );
            assert_eq!(object.state().instances, copies_before);
            for (i, expected) in target_points.iter().enumerate() {
                let actual = object
                    .instance_world_coord(AtomIndex(i as u32), Some(1))
                    .unwrap();
                assert!(
                    (actual - *expected).magnitude() < 1e-3,
                    "{method}: {actual:?} != {expected:?}"
                );
            }
        }
        run(&mut session, "translate [8,0,0], target, camera=0").unwrap();
        run(
            &mut session,
            "align target, assembly_view and instance 2, cycles=0",
        )
        .unwrap();
        for (i, expected) in target_points.iter().enumerate() {
            let actual = session
                .registry
                .get_molecule("target")
                .unwrap()
                .instance_world_coord(AtomIndex(i as u32), None)
                .unwrap();
            assert!((actual - *expected).magnitude() < 1e-3);
        }
    }

    #[test]
    fn transitive_copy_selections_keep_identity_after_materialization() {
        let mut session = fixture();
        run(&mut session, "biounit source, assembly_view").unwrap();
        run(
            &mut session,
            "select first_copy, instance 1 and assembly_view",
        )
        .unwrap();
        run(&mut session, "select copy_alias, first_copy").unwrap();
        run(&mut session, "select nested_alias, copy_alias").unwrap();
        run(
            &mut session,
            "select second_copy, instance 2 and assembly_view",
        )
        .unwrap();
        run(&mut session, "select whole_assembly, assembly_view").unwrap();
        run(&mut session, "select mixed_sources, source or nested_alias").unwrap();
        run(&mut session, "pick nested_alias and name CA").unwrap();
        assert_eq!(session.resolved_recent_atom_anchors()[0].instance, Some(0));
        run(
            &mut session,
            "distance alias_distance, nested_alias, second_copy",
        )
        .unwrap();
        assert!((distance(&session, "alias_distance") - 10.0).abs() < 1e-8);
        run(&mut session, "materialize assembly_view").unwrap();
        run(
            &mut session,
            "distance mapped_distance, nested_alias, second_copy",
        )
        .unwrap();
        assert!((distance(&session, "mapped_distance") - 10.0).abs() < 1e-8);
        run(&mut session, "unpick").unwrap();
        run(&mut session, "pick nested_alias").unwrap();
        assert_eq!(
            session.resolved_recent_atom_anchors()[0].atom_index,
            AtomIndex(0)
        );
        let mut needs_redraw = false;
        let adapter = SessionAdapter {
            session: &mut session,
            render_context: None,
            default_size: (64, 64),
            needs_redraw: &mut needs_redraw,
        };
        assert_eq!(
            crate::commands::selecting::evaluate_atom_anchors(&adapter, "mixed_sources")
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            crate::commands::selecting::evaluate_atom_anchors(&adapter, "whole_assembly")
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn copy_negation_is_evaluated_before_source_projection() {
        let mut session = fixture();
        run(&mut session, "biounit source, assembly_view").unwrap();
        run(
            &mut session,
            "select second_copy, assembly_view and not instance 1",
        )
        .unwrap();
        run(&mut session, "select second_alias, second_copy").unwrap();
        assert!(session.selections.get("second_copy").is_some());
        run(&mut session, "hide spheres, assembly_view").unwrap();
        run(&mut session, "show spheres, second_alias").unwrap();
        assert!(session
            .registry
            .get_molecule("assembly_view")
            .unwrap()
            .molecule()
            .get_atom(AtomIndex(0))
            .unwrap()
            .repr
            .visible_reps
            .is_visible(RepMask::SPHERES));
        run(&mut session, "pick assembly_view and instance 1").unwrap();
        run(&mut session, "pick assembly_view and instance 2").unwrap();
        run(&mut session, "select pick_difference, pk1 and not pk2").unwrap();
        run(&mut session, "select difference_alias, pick_difference").unwrap();
        let mut needs_redraw = false;
        let adapter = SessionAdapter {
            session: &mut session,
            render_context: None,
            default_size: (64, 64),
            needs_redraw: &mut needs_redraw,
        };
        for (expression, copy) in [
            ("assembly_view and not instance 1", 1),
            ("second_alias", 1),
            ("pk1 and not pk2", 0),
            ("pk2 and not pk1", 1),
            ("difference_alias", 0),
        ] {
            let (count, _) =
                crate::commands::selecting::select_with_context(&adapter, expression).unwrap();
            assert_eq!(count, 1, "{expression}");
            let anchors =
                crate::commands::selecting::evaluate_atom_anchors(&adapter, expression).unwrap();
            assert_eq!(anchors.len(), 1, "{expression}");
            assert_eq!(anchors[0].instance, Some(copy), "{expression}");
        }
    }

    #[test]
    fn rmsd_between_copies_uses_instance_world_coordinates() {
        let mut session = fixture();
        run(&mut session, "biounit source, assembly_view").unwrap();
        let before = rmp_serde::to_vec_named(&session).unwrap();
        let result = run(
            &mut session,
            "rms_cur assembly_view and instance 1, assembly_view and instance 2",
        )
        .unwrap();
        assert!(result
            .messages
            .iter()
            .any(|message| message.text.contains("10.000")));
        assert_eq!(rmp_serde::to_vec_named(&session).unwrap(), before);
    }

    #[test]
    fn definitions_are_required_and_default_names_do_not_replace_sources() {
        let mut session = fixture();
        run(&mut session, "biounit source").unwrap();
        run(&mut session, "biounit source").unwrap();
        assert!(session.registry.get("source").is_some());
        assert!(session.registry.get("source_biounit").is_some());
        assert!(session.registry.get("source_biounit_2").is_some());
        assert!(run(&mut session, "biounit source, source").is_err());
        assert!(run(&mut session, "biounit source, missing, assembly=404").is_err());
        session
            .registry
            .get_molecule_mut("source")
            .unwrap()
            .molecule_mut()
            .assembly
            .definitions
            .clear();
        assert!(run(&mut session, "biounit source, missing").is_err());
        assert!(session.registry.get("missing").is_none());
    }
}
