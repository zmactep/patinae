//! Measurement commands: distance, angle, dihedral
//!
//! Create measurement objects that display dashed lines between atoms
//! with labels showing the measured value.

use ahash::AHashSet;

use crate::args::ParsedCommand;
use crate::command::{ArgHint, Command, CommandContext, CommandRegistry, ViewerLike};
use crate::command_help;
use crate::commands::selecting::{evaluate_selection, select_with_context};
use crate::error::{CmdError, CmdResult};

use patinae_color::ColorIndex;
use patinae_scene::{
    resolve_measurement_entity_value, MeasurementAnchor, MeasurementEntry, MeasurementKind,
    MeasurementObject, MeasurementResolveOptions, Object,
};

pub fn register(registry: &mut CommandRegistry) {
    registry.register(DistanceCommand);
    registry.register(AngleCommand);
    registry.register(DihedralCommand);
}

/// Selects whether a typed measurement creates or appends an object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MeasurementTarget {
    /// Create a new object with a registry-allocated kind-specific name.
    New,
    /// Append to an existing measurement object.
    Existing(String),
}

/// Requests one measurement from ordered singleton atom paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeasurementRequest {
    /// Ordered atom selection paths shown in the native operand queue.
    pub operands: Vec<String>,
    /// Destination for the new measurement entity.
    pub target: MeasurementTarget,
}

impl MeasurementRequest {
    /// Creates a typed measurement request.
    pub fn new<I, S>(operands: I, target: MeasurementTarget) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            operands: operands.into_iter().map(Into::into).collect(),
            target,
        }
    }

    /// Infers the measurement kind from the operand count.
    ///
    /// # Errors
    ///
    /// Returns an error unless exactly two, three, or four operands exist.
    pub fn inferred_kind(&self) -> CmdResult<MeasurementKind> {
        measurement_kind_for_count(self.operands.len())
    }
}

/// Describes one successfully applied typed measurement request.
#[derive(Debug, Clone, PartialEq)]
pub struct MeasurementOutcome {
    /// Object created or appended by the request.
    pub object_name: String,
    /// Inferred measurement kind.
    pub kind: MeasurementKind,
    /// Current value of the added entity.
    pub value: f64,
}

/// Infers a measurement kind from its ordered operand count.
///
/// # Errors
///
/// Returns an error unless `count` is two, three, or four.
pub fn measurement_kind_for_count(count: usize) -> CmdResult<MeasurementKind> {
    match count {
        2 => Ok(MeasurementKind::Distance),
        3 => Ok(MeasurementKind::Angle),
        4 => Ok(MeasurementKind::Dihedral),
        _ => Err(CmdError::invalid_arg(
            "operands",
            "measurement requires exactly 2, 3, or 4 atoms",
        )),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DistanceMode {
    Broadcast,
    Cartesian,
}

impl DistanceMode {
    fn parse(value: &str) -> CmdResult<Self> {
        match value.to_ascii_lowercase().as_str() {
            "broadcast" => Ok(Self::Broadcast),
            "cartesian" => Ok(Self::Cartesian),
            _ => Err(CmdError::invalid_arg(
                "mode",
                format!("unknown distance mode '{value}'; expected 'broadcast' or 'cartesian'"),
            )),
        }
    }
}

/// Resolve every atom anchor from a selection expression in stable order.
fn resolve_atom_anchors(
    viewer: &dyn ViewerLike,
    selection: &str,
) -> CmdResult<Vec<MeasurementAnchor>> {
    let results = evaluate_selection(viewer, selection)?;
    let mut anchors = Vec::new();
    for (obj_name, selected) in &results {
        if viewer.objects().get_molecule(obj_name).is_some() {
            for idx in selected.indices() {
                anchors.push(MeasurementAnchor::new(obj_name, idx));
            }
        }
    }
    if anchors.is_empty() {
        Err(CmdError::selection(format!(
            "no atoms found in selection '{}'",
            selection
        )))
    } else {
        Ok(anchors)
    }
}

/// Resolve the first atom for commands that require singleton selections.
fn resolve_atom_anchor(viewer: &dyn ViewerLike, selection: &str) -> CmdResult<MeasurementAnchor> {
    let results = evaluate_selection(viewer, selection)?;
    for (object_name, selected) in results {
        if viewer.objects().get_molecule(&object_name).is_none() {
            continue;
        }
        if let Some(atom_index) = selected.indices().next() {
            return Ok(MeasurementAnchor::new(object_name, atom_index));
        }
    }
    Err(CmdError::selection(format!(
        "no atoms found in selection '{selection}'"
    )))
}

fn resolve_singleton_atom_anchor(
    viewer: &dyn ViewerLike,
    selection: &str,
) -> CmdResult<MeasurementAnchor> {
    let (total_count, results) = select_with_context(viewer, selection)?;
    if total_count != 1 {
        return Err(CmdError::selection(format!(
            "operand '{selection}' does not resolve to exactly one atom"
        )));
    }
    for (object_name, selected) in results {
        if viewer.objects().get_molecule(&object_name).is_none() {
            continue;
        }
        if let Some(atom_index) = selected.indices().next() {
            return Ok(MeasurementAnchor::new(object_name, atom_index));
        }
    }
    Err(CmdError::selection(format!(
        "operand '{selection}' does not resolve to exactly one atom"
    )))
}

fn validate_measurement_target(
    viewer: &dyn ViewerLike,
    name: &str,
    kind: MeasurementKind,
    require_existing: bool,
) -> CmdResult {
    let Some(existing) = viewer.objects().get(name) else {
        return if require_existing {
            Err(CmdError::object_not_found(name))
        } else {
            Ok(())
        };
    };
    let Some(measurement) = viewer.objects().get_measurement(name) else {
        return Err(CmdError::invalid_arg(
            "name",
            format!(
                "object '{}' is {}, not a measurement",
                name,
                existing.object_type()
            ),
        ));
    };
    if measurement.kind() != kind {
        return Err(CmdError::invalid_arg(
            "name",
            format!(
                "measurement '{}' is {:?}, not {:?}",
                name,
                measurement.kind(),
                kind
            ),
        ));
    }
    Ok(())
}

/// Validate and add homogeneous measurement entries as one mutation.
fn add_measurements_to_scene(
    viewer: &mut dyn ViewerLike,
    name: &str,
    kind: MeasurementKind,
    entries: Vec<MeasurementEntry>,
) -> CmdResult<Vec<f64>> {
    validate_measurement_target(viewer, name, kind, false)?;

    if entries.is_empty() {
        return Err(CmdError::invalid_arg(
            "selection",
            "measurement geometry is undefined",
        ));
    }

    let options = MeasurementResolveOptions::from_settings(&viewer.settings().measurement);
    let mut values = Vec::with_capacity(entries.len());
    for entry in &entries {
        let value = resolve_measurement_entity_value(viewer.objects(), kind, entry, options)
            .ok_or_else(|| {
                CmdError::invalid_arg("selection", "measurement geometry is undefined")
            })?;
        values.push(value);
    }

    if let Some(measurement) = viewer.objects_mut().get_measurement_mut(name) {
        measurement
            .add_entries(entries)
            .map_err(|error| CmdError::invalid_arg("selection", error.to_string()))?;
    } else {
        let mut candidate = MeasurementObject::with_entities(name, kind, entries)
            .map_err(|error| CmdError::invalid_arg("selection", error.to_string()))?;
        let cyan = viewer.color_index("cyan").ok_or_else(|| {
            CmdError::invalid_arg("color", "default measurement color 'cyan' is unavailable")
        })?;
        candidate.state_mut().color = ColorIndex::Named(cyan);
        candidate.invalidate_material();
        viewer.objects_mut().add(candidate);
    }
    viewer.request_redraw();
    Ok(values)
}

/// Validates and applies one typed measurement request transactionally.
///
/// All operands and geometry are resolved before the registry is mutated.
/// Automatic object names are allocated only after that validation.
///
/// # Errors
///
/// Returns an error for invalid cardinality, stale or non-singleton operands,
/// undefined geometry, missing targets, and targets of another object kind.
pub fn execute_measurement_request(
    viewer: &mut dyn ViewerLike,
    request: &MeasurementRequest,
) -> CmdResult<MeasurementOutcome> {
    let kind = request.inferred_kind()?;
    if let MeasurementTarget::Existing(name) = &request.target {
        validate_measurement_target(viewer, name, kind, true)?;
    }
    let anchors = request
        .operands
        .iter()
        .map(|operand| resolve_singleton_atom_anchor(viewer, operand))
        .collect::<CmdResult<Vec<_>>>()?;
    let entry = MeasurementEntry::new(anchors);
    let object_name = match &request.target {
        MeasurementTarget::New => viewer.objects().first_free_measurement_name(kind),
        MeasurementTarget::Existing(name) => name.clone(),
    };
    let values = add_measurements_to_scene(viewer, &object_name, kind, vec![entry])?;
    let value = values
        .into_iter()
        .next()
        .ok_or_else(|| CmdError::invalid_arg("operands", "measurement geometry is undefined"))?;
    Ok(MeasurementOutcome {
        object_name,
        kind,
        value,
    })
}

fn distance_entries(
    viewer: &dyn ViewerLike,
    selection1: &str,
    selection2: &str,
    mode: DistanceMode,
) -> CmdResult<Vec<MeasurementEntry>> {
    let anchors1 = resolve_atom_anchors(viewer, selection1)?;
    let anchors2 = resolve_atom_anchors(viewer, selection2)?;
    match mode {
        DistanceMode::Broadcast => broadcast_distance_entries(&anchors1, &anchors2),
        DistanceMode::Cartesian => cartesian_distance_entries(&anchors1, &anchors2),
    }
}

fn broadcast_distance_entries(
    anchors1: &[MeasurementAnchor],
    anchors2: &[MeasurementAnchor],
) -> CmdResult<Vec<MeasurementEntry>> {
    let (singleton, targets, singleton_is_first) = match (anchors1, anchors2) {
        ([source], targets) => (source, targets, true),
        (sources, [target]) => (target, sources, false),
        _ => {
            return Err(CmdError::invalid_arg(
                "selection",
                "distance requires at least one selection to contain exactly one atom",
            ))
        }
    };
    let mut entries = Vec::with_capacity(targets.len());
    for target in targets {
        if singleton.object_name == target.object_name && singleton.atom_index == target.atom_index
        {
            continue;
        }
        let anchors = if singleton_is_first {
            vec![singleton.clone(), target.clone()]
        } else {
            vec![target.clone(), singleton.clone()]
        };
        entries.push(MeasurementEntry::new(anchors));
    }
    Ok(entries)
}

fn cartesian_distance_entries(
    anchors1: &[MeasurementAnchor],
    anchors2: &[MeasurementAnchor],
) -> CmdResult<Vec<MeasurementEntry>> {
    let candidate_count = anchors1.len().checked_mul(anchors2.len()).ok_or_else(|| {
        CmdError::invalid_arg("selection", "cartesian distance pair count is too large")
    })?;
    let mut entries = Vec::new();
    entries.try_reserve(candidate_count).map_err(|_| {
        CmdError::invalid_arg("selection", "cartesian distance pair count is too large")
    })?;
    let mut seen = AHashSet::new();
    seen.try_reserve(candidate_count).map_err(|_| {
        CmdError::invalid_arg("selection", "cartesian distance pair count is too large")
    })?;

    for anchor1 in anchors1 {
        for anchor2 in anchors2 {
            let id1 = (anchor1.object_name.as_str(), anchor1.atom_index);
            let id2 = (anchor2.object_name.as_str(), anchor2.atom_index);
            if id1 == id2 {
                continue;
            }
            let pair = if id1 < id2 { (id1, id2) } else { (id2, id1) };
            if seen.insert(pair) {
                entries.push(MeasurementEntry::new(vec![
                    anchor1.clone(),
                    anchor2.clone(),
                ]));
            }
        }
    }
    Ok(entries)
}

// ============================================================================
// distance command
// ============================================================================

struct DistanceCommand;

impl Command for DistanceCommand {
    fn name(&self) -> &str {
        "distance"
    }

    fn aliases(&self) -> &[&str] {
        &["dist"]
    }

    command_help! {
        CMD "distance"
        DESCRIPTION [
            "creates a distance measurement between two atom selections.",
            "Without arguments, uses pk1 and pk2 and allocates a new object name.",
            "The default broadcast mode requires one singleton selection.",
            "Cartesian mode creates every unique pair across both selections.",
        ]
        USAGE [
            "distance",
            "distance name, selection1, selection2 [, mode]",
        ]
        REQUIRED [
            { "name", "string", "name for the measurement object" },
            { "selection1", "string", "first atom selection" },
            { "selection2", "string", "second atom selection" },
        ]
        OPTIONAL [
            { "mode", "string", "broadcast or cartesian pairing", "broadcast" },
        ]
        EXAMPLES [
            "distance",
            "distance dist1, /1hpx///A/1/CA, /1hpx///A/10/CA",
            "distance d1, chain A and name CA and resi 1, chain A and name CA and resi 10",
            "distance contacts, chain A and name CA, chain B and name CA, cartesian",
            "distance contacts, chain A and name CA, chain B and name CA, mode=cartesian",
        ]
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[
            ArgHint::None,
            ArgHint::Selection,
            ArgHint::Selection,
            ArgHint::Keywords(&["broadcast", "cartesian"]),
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        if args.arg_count() == 0 {
            let request = MeasurementRequest::new(["pk1", "pk2"], MeasurementTarget::New);
            let outcome = execute_measurement_request(ctx.viewer, &request)?;
            ctx.print(&format!(
                " distance: {:.3} Angstroms (1 measurements)",
                outcome.value
            ));
            return Ok(());
        }

        let name = args
            .str_arg(0, "name")
            .ok_or_else(|| CmdError::missing_argument("name"))?;
        let sel1 = args
            .str_arg(1, "selection1")
            .ok_or_else(|| CmdError::missing_argument("selection1"))?;
        let sel2 = args
            .str_arg(2, "selection2")
            .ok_or_else(|| CmdError::missing_argument("selection2"))?;
        let mode = DistanceMode::parse(args.str_arg_or(3, "mode", "broadcast"))?;

        let entries = distance_entries(ctx.viewer, sel1, sel2, mode)?;
        let values =
            add_measurements_to_scene(ctx.viewer, name, MeasurementKind::Distance, entries)?;
        let average = values.iter().sum::<f64>() / values.len() as f64;
        ctx.print(&format!(
            " distance: {:.3} Angstroms ({} measurements)",
            average,
            values.len()
        ));

        Ok(())
    }
}

// ============================================================================
// angle command
// ============================================================================

struct AngleCommand;

impl Command for AngleCommand {
    fn name(&self) -> &str {
        "angle"
    }

    command_help! {
        CMD "angle"
        DESCRIPTION [
            "creates an angle measurement between three atom selections.",
            "Without arguments, uses pk1 through pk3 and allocates a new object name.",
            "The angle is measured at the second atom (vertex).",
        ]
        USAGE [
            "angle",
            "angle name, selection1, selection2, selection3",
        ]
        REQUIRED [
            { "name", "string", "name for the measurement object" },
            { "selection1", "string", "first atom selection" },
            { "selection2", "string", "second atom (vertex) selection" },
            { "selection3", "string", "third atom selection" },
        ]
        OPTIONAL []
        EXAMPLES [
            "angle",
            "angle ang1, /1hpx///A/1/CA, /1hpx///A/5/CA, /1hpx///A/10/CA",
        ]
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[
            ArgHint::None,
            ArgHint::Selection,
            ArgHint::Selection,
            ArgHint::Selection,
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        if args.arg_count() == 0 {
            let request = MeasurementRequest::new(["pk1", "pk2", "pk3"], MeasurementTarget::New);
            let outcome = execute_measurement_request(ctx.viewer, &request)?;
            ctx.print(&format!(" angle: {:.1} degrees", outcome.value));
            return Ok(());
        }

        let name = args
            .get_str(0)
            .ok_or_else(|| CmdError::missing_argument("name"))?;
        let sel1 = args
            .get_str(1)
            .ok_or_else(|| CmdError::missing_argument("selection1"))?;
        let sel2 = args
            .get_str(2)
            .ok_or_else(|| CmdError::missing_argument("selection2"))?;
        let sel3 = args
            .get_str(3)
            .ok_or_else(|| CmdError::missing_argument("selection3"))?;

        let entry = MeasurementEntry::new(vec![
            resolve_atom_anchor(ctx.viewer, sel1)?,
            resolve_atom_anchor(ctx.viewer, sel2)?,
            resolve_atom_anchor(ctx.viewer, sel3)?,
        ]);
        let value =
            add_measurements_to_scene(ctx.viewer, name, MeasurementKind::Angle, vec![entry])?[0];
        ctx.print(&format!(" angle: {:.1} degrees", value));

        Ok(())
    }
}

// ============================================================================
// dihedral command
// ============================================================================

struct DihedralCommand;

impl Command for DihedralCommand {
    fn name(&self) -> &str {
        "dihedral"
    }

    command_help! {
        CMD "dihedral"
        DESCRIPTION [
            "creates a dihedral angle measurement between four atom selections.",
            "Without arguments, uses pk1 through pk4 and allocates a new object name.",
            "The dihedral is measured around the bond between atoms 2 and 3.",
        ]
        USAGE [
            "dihedral",
            "dihedral name, selection1, selection2, selection3, selection4",
        ]
        REQUIRED [
            { "name", "string", "name for the measurement object" },
            { "selection1", "string", "first atom selection" },
            { "selection2", "string", "second atom selection" },
            { "selection3", "string", "third atom selection" },
            { "selection4", "string", "fourth atom selection" },
        ]
        OPTIONAL []
        EXAMPLES [
            "dihedral",
            "dihedral dih1, /1hpx///A/1/N, /1hpx///A/1/CA, /1hpx///A/1/C, /1hpx///A/2/N",
        ]
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[
            ArgHint::None,
            ArgHint::Selection,
            ArgHint::Selection,
            ArgHint::Selection,
            ArgHint::Selection,
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        if args.arg_count() == 0 {
            let request =
                MeasurementRequest::new(["pk1", "pk2", "pk3", "pk4"], MeasurementTarget::New);
            let outcome = execute_measurement_request(ctx.viewer, &request)?;
            ctx.print(&format!(" dihedral: {:.1} degrees", outcome.value));
            return Ok(());
        }

        let name = args
            .get_str(0)
            .ok_or_else(|| CmdError::missing_argument("name"))?;
        let sel1 = args
            .get_str(1)
            .ok_or_else(|| CmdError::missing_argument("selection1"))?;
        let sel2 = args
            .get_str(2)
            .ok_or_else(|| CmdError::missing_argument("selection2"))?;
        let sel3 = args
            .get_str(3)
            .ok_or_else(|| CmdError::missing_argument("selection3"))?;
        let sel4 = args
            .get_str(4)
            .ok_or_else(|| CmdError::missing_argument("selection4"))?;

        let entry = MeasurementEntry::new(vec![
            resolve_atom_anchor(ctx.viewer, sel1)?,
            resolve_atom_anchor(ctx.viewer, sel2)?,
            resolve_atom_anchor(ctx.viewer, sel3)?,
            resolve_atom_anchor(ctx.viewer, sel4)?,
        ]);
        let value =
            add_measurements_to_scene(ctx.viewer, name, MeasurementKind::Dihedral, vec![entry])?[0];
        ctx.print(&format!(" dihedral: {:.1} degrees", value));

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lin_alg::f32::Vec3;
    use patinae_mol::{Atom, AtomIndex, CoordSet, Element, ObjectMolecule};
    use patinae_scene::{
        canonical_atom_path_for_hit, LabelObject, MoleculeObject, ObjectType, PickHit, Session,
        SessionAdapter,
    };
    use patinae_settings::groups::RecentPickLimit;

    use crate::CommandExecutor;

    fn measurement_session() -> Session {
        let mut molecule = ObjectMolecule::new("source");
        for name in ["A", "B", "C", "D"] {
            molecule.add_atom(Atom::new(name, Element::Carbon));
        }
        molecule.add_coord_set(CoordSet::from_vec3(&[
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, 1.0, 1.0),
        ]));
        let mut session = Session::new();
        session
            .registry
            .add(MoleculeObject::with_name(molecule, "source"));
        session
    }

    fn execute(session: &mut Session, command: &str) -> CmdResult {
        let mut needs_redraw = false;
        let mut adapter = SessionAdapter {
            session,
            render_context: None,
            default_size: (64, 64),
            needs_redraw: &mut needs_redraw,
            async_fetch_fn: None,
        };
        CommandExecutor::new().do_(&mut adapter, command)
    }

    fn execute_typed(
        session: &mut Session,
        request: &MeasurementRequest,
    ) -> CmdResult<MeasurementOutcome> {
        let mut needs_redraw = false;
        let mut adapter = SessionAdapter {
            session,
            render_context: None,
            default_size: (64, 64),
            needs_redraw: &mut needs_redraw,
            async_fetch_fn: None,
        };
        execute_measurement_request(&mut adapter, request)
    }

    fn canonical_path(session: &Session, atom_index: usize) -> String {
        let molecule = session.registry.get_molecule("source").unwrap();
        canonical_atom_path_for_hit(
            &PickHit {
                object_name: "source".to_string(),
                object_type: ObjectType::Molecule,
                atom_index: Some(AtomIndex::from(atom_index)),
                position: Vec3::new(0.0, 0.0, 0.0),
                distance: 0.0,
            },
            molecule.molecule(),
        )
        .unwrap()
    }

    fn set_recent_atoms(session: &mut Session, atom_indices: &[usize]) {
        for &atom_index in atom_indices {
            let path = canonical_path(session, atom_index);
            assert!(session
                .recent_atoms
                .insert(path, RecentPickLimit::Unlimited));
        }
    }

    fn measurement_anchor_indices(session: &Session, name: &str) -> Vec<AtomIndex> {
        session.registry.get_measurement(name).unwrap().entries()[0]
            .anchors
            .iter()
            .map(|anchor| anchor.atom_index)
            .collect()
    }

    #[test]
    fn argument_free_measurements_use_first_recent_atoms_and_alias() {
        let mut session = measurement_session();
        set_recent_atoms(&mut session, &[0, 1, 2, 3]);

        execute(&mut session, "distance").unwrap();
        execute(&mut session, "angle").unwrap();
        execute(&mut session, "dihedral").unwrap();
        execute(&mut session, "dist").unwrap();

        assert_eq!(
            measurement_anchor_indices(&session, "distance01"),
            [AtomIndex(0), AtomIndex(1)]
        );
        assert_eq!(
            measurement_anchor_indices(&session, "angle01"),
            [AtomIndex(0), AtomIndex(1), AtomIndex(2)]
        );
        assert_eq!(
            measurement_anchor_indices(&session, "dihedral01"),
            [AtomIndex(0), AtomIndex(1), AtomIndex(2), AtomIndex(3)]
        );
        assert_eq!(
            measurement_anchor_indices(&session, "distance02"),
            [AtomIndex(0), AtomIndex(1)]
        );
    }

    #[test]
    fn argument_free_measurements_use_first_free_kind_names() {
        let mut session = measurement_session();
        set_recent_atoms(&mut session, &[0, 1, 2, 3]);
        for name in ["distance01", "angle01", "dihedral01"] {
            session.registry.add(LabelObject::new(name));
        }

        execute(&mut session, "distance").unwrap();
        execute(&mut session, "angle").unwrap();
        execute(&mut session, "dihedral").unwrap();

        for name in ["distance01", "angle01", "dihedral01"] {
            assert!(session.registry.get_label(name).is_some());
        }
        assert!(session.registry.get_measurement("distance02").is_some());
        assert!(session.registry.get_measurement("angle02").is_some());
        assert!(session.registry.get_measurement("dihedral02").is_some());
    }

    #[test]
    fn argument_free_measurements_fail_transactionally() {
        for (command, atom_indices, missing_alias, default_name) in [
            ("distance", &[0][..], "pk2", "distance01"),
            ("angle", &[0, 1][..], "pk3", "angle01"),
            ("dihedral", &[0, 1, 2][..], "pk4", "dihedral01"),
        ] {
            let mut missing = measurement_session();
            set_recent_atoms(&mut missing, atom_indices);
            let before = missing.registry.len();

            let error = execute(&mut missing, command).unwrap_err();

            assert!(error.to_string().contains(missing_alias));
            assert_eq!(missing.registry.len(), before);
            assert!(missing.registry.get_measurement(default_name).is_none());
        }

        let mut undefined = measurement_session();
        undefined
            .registry
            .get_molecule_mut("source")
            .unwrap()
            .molecule_mut()
            .set_coord(AtomIndex(1), 0, Vec3::new(1.0, 0.0, 0.0));
        set_recent_atoms(&mut undefined, &[0, 1]);
        let before = undefined.registry.len();

        let error = execute(&mut undefined, "distance").unwrap_err();

        assert!(error.to_string().contains("undefined"));
        assert_eq!(undefined.registry.len(), before);
        assert!(undefined.registry.get_measurement("distance01").is_none());
    }

    #[test]
    fn measurement_help_documents_argument_free_forms() {
        let registry = CommandRegistry::with_builtins();
        for (command, usage, operands) in [
            ("distance", "    distance\n", "uses pk1 and pk2"),
            ("angle", "    angle\n", "uses pk1 through pk3"),
            ("dihedral", "    dihedral\n", "uses pk1 through pk4"),
        ] {
            let registered = registry.get(command).unwrap();
            let help = registered.help();
            assert!(help.contains(usage), "{command}: {help}");
            assert!(help.contains(operands), "{command}: {help}");
        }
    }

    #[test]
    fn partial_measurement_commands_keep_missing_argument_errors() {
        let mut session = measurement_session();

        for command in ["distance named", "angle named", "dihedral named"] {
            let error = execute(&mut session, command).unwrap_err();
            assert!(error.is_missing_argument(), "{command}: {error}");
            assert_eq!(error.argument_name(), Some("selection1"));
        }
    }

    #[test]
    fn typed_measurements_preserve_operand_order_and_allocate_at_execution() {
        let mut session = measurement_session();
        let paths = (0..4)
            .map(|index| canonical_path(&session, index))
            .collect::<Vec<_>>();
        let requests = [
            MeasurementRequest::new(paths[..2].iter().cloned(), MeasurementTarget::New),
            MeasurementRequest::new(paths[2..].iter().cloned(), MeasurementTarget::New),
        ];

        let first = execute_typed(&mut session, &requests[0]).unwrap();
        let second = execute_typed(&mut session, &requests[1]).unwrap();

        assert_eq!(first.object_name, "distance01");
        assert_eq!(second.object_name, "distance02");
        assert_eq!(first.kind, MeasurementKind::Distance);
        let entry = &session
            .registry
            .get_measurement("distance01")
            .unwrap()
            .entries()[0];
        assert_eq!(
            entry
                .anchors
                .iter()
                .map(|anchor| anchor.atom_index)
                .collect::<Vec<_>>(),
            [AtomIndex(0), AtomIndex(1)]
        );
    }

    #[test]
    fn typed_measurements_infer_all_kinds_and_filter_append_targets() {
        let mut session = measurement_session();
        for (operands, expected_kind, expected_name) in [
            (
                vec!["name A", "name B"],
                MeasurementKind::Distance,
                "distance01",
            ),
            (
                vec!["name A", "name B", "name C"],
                MeasurementKind::Angle,
                "angle01",
            ),
            (
                vec!["name A", "name B", "name C", "name D"],
                MeasurementKind::Dihedral,
                "dihedral01",
            ),
        ] {
            let request = MeasurementRequest::new(operands, MeasurementTarget::New);
            let outcome = execute_typed(&mut session, &request).unwrap();
            assert_eq!(outcome.kind, expected_kind);
            assert_eq!(outcome.object_name, expected_name);
        }

        let before = session
            .registry
            .get_measurement("distance01")
            .unwrap()
            .len();
        let wrong_kind = MeasurementRequest::new(
            ["name A", "name B", "name C"],
            MeasurementTarget::Existing("distance01".to_string()),
        );
        let error = execute_typed(&mut session, &wrong_kind).unwrap_err();
        assert!(error.to_string().contains("not Angle"));
        assert_eq!(
            session
                .registry
                .get_measurement("distance01")
                .unwrap()
                .len(),
            before
        );

        let stale_target = MeasurementRequest::new(
            ["name A", "name B"],
            MeasurementTarget::Existing("missing".to_string()),
        );
        assert!(execute_typed(&mut session, &stale_target).is_err());
        assert!(session.registry.get_measurement("missing").is_none());

        let wrong_object = MeasurementRequest::new(
            ["name A", "name B"],
            MeasurementTarget::Existing("source".to_string()),
        );
        assert!(execute_typed(&mut session, &wrong_object).is_err());
        assert!(session.registry.get_molecule("source").is_some());
    }

    #[test]
    fn typed_measurement_rejects_stale_non_singleton_and_undefined_without_mutation() {
        let mut session = measurement_session();
        for request in [
            MeasurementRequest::new(["name missing", "name B"], MeasurementTarget::New),
            MeasurementRequest::new(["all", "name B"], MeasurementTarget::New),
            MeasurementRequest::new(["name B", "name A", "name A"], MeasurementTarget::New),
        ] {
            assert!(execute_typed(&mut session, &request).is_err());
        }
        assert!(session.registry.get_measurement("distance01").is_none());
        assert!(session.registry.get_measurement("angle01").is_none());
    }

    #[test]
    fn distance_creates_cyan_object_and_same_kind_appends() {
        let mut session = measurement_session();

        execute(&mut session, "distance d, name A, name B").unwrap();
        execute(&mut session, "distance d, name C, name D").unwrap();

        let measurement = session.registry.get_measurement("d").unwrap();
        let cyan = session.named_palette.get_by_name("cyan").unwrap().0;
        assert_eq!(measurement.kind(), MeasurementKind::Distance);
        assert_eq!(measurement.len(), 2);
        assert_eq!(measurement.state().color, ColorIndex::Named(cyan));
    }

    #[test]
    fn measurement_kind_conflict_does_not_mutate_object() {
        let mut session = measurement_session();
        execute(&mut session, "distance measure, name A, name B").unwrap();
        let revisions = session
            .registry
            .get_measurement("measure")
            .unwrap()
            .revisions();

        let error = execute(&mut session, "angle measure, name A, name B, name C").unwrap_err();

        assert!(error.to_string().contains("not Angle"));
        let measurement = session.registry.get_measurement("measure").unwrap();
        assert_eq!(measurement.kind(), MeasurementKind::Distance);
        assert_eq!(measurement.len(), 1);
        assert_eq!(measurement.revisions(), revisions);
    }

    #[test]
    fn angle_and_dihedral_create_distinct_measurement_kinds() {
        let mut session = measurement_session();

        execute(&mut session, "angle a, name A, name B, name C").unwrap();
        execute(&mut session, "dihedral phi, name A, name B, name C, name D").unwrap();

        let angle = session.registry.get_measurement("a").unwrap();
        let dihedral = session.registry.get_measurement("phi").unwrap();
        assert_eq!(angle.kind(), MeasurementKind::Angle);
        assert_eq!(angle.len(), 1);
        assert_eq!(dihedral.kind(), MeasurementKind::Dihedral);
        assert_eq!(dihedral.len(), 1);
    }

    #[test]
    fn non_measurement_name_conflict_does_not_replace_object() {
        let mut session = measurement_session();

        let error = execute(&mut session, "distance source, name A, name B").unwrap_err();

        assert!(error.to_string().contains("not a measurement"));
        assert!(session.registry.get_molecule("source").is_some());
        assert!(session.registry.get_measurement("source").is_none());
    }

    #[test]
    fn label_name_conflict_does_not_replace_or_mutate_object() {
        let mut session = measurement_session();
        session.registry.add(LabelObject::new("labels"));
        let revisions = session.registry.get_label("labels").unwrap().revisions();

        let error = execute(&mut session, "distance labels, name A, name B").unwrap_err();

        assert!(error.to_string().contains("not a measurement"));
        let labels = session.registry.get_label("labels").unwrap();
        assert!(labels.is_empty());
        assert_eq!(labels.revisions(), revisions);
        assert!(session.registry.get_measurement("labels").is_none());
    }

    #[test]
    fn distance_selection_broadcasts_one_source_to_all_targets() {
        let mut session = measurement_session();
        let expected_targets = [AtomIndex(1), AtomIndex(2), AtomIndex(3)];

        execute(&mut session, "distance d, name A, all").unwrap();

        let measurement = session.registry.get_measurement("d").unwrap();
        assert_eq!(measurement.len(), 3);
        for (entry, target) in measurement.entries().iter().zip(expected_targets) {
            assert_eq!(entry.anchors[0].atom_index, AtomIndex(0));
            assert_eq!(entry.anchors[1].atom_index, target);
        }
    }

    #[test]
    fn distance_selection_broadcasts_all_sources_to_one_target_in_argument_order() {
        let mut session = measurement_session();

        execute(&mut session, "distance d, name A+B+C, name D").unwrap();

        let measurement = session.registry.get_measurement("d").unwrap();
        assert_eq!(measurement.len(), 3);
        for (entry, source) in
            measurement
                .entries()
                .iter()
                .zip([AtomIndex(0), AtomIndex(1), AtomIndex(2)])
        {
            assert_eq!(entry.anchors[0].atom_index, source);
            assert_eq!(entry.anchors[1].atom_index, AtomIndex(3));
        }
    }

    #[test]
    fn distance_rejects_two_multi_atom_selections_without_mutation() {
        let mut session = measurement_session();

        let error = execute(&mut session, "distance d, name A+B, name C+D").unwrap_err();

        assert!(error.to_string().contains("exactly one atom"));
        assert!(session.registry.get_measurement("d").is_none());
    }

    #[test]
    fn distance_cartesian_mode_builds_unique_cross_product_in_stable_order() {
        let mut session = measurement_session();

        execute(&mut session, "distance d, name A+B, name C+D, cartesian").unwrap();

        let measurement = session.registry.get_measurement("d").unwrap();
        let pairs = measurement
            .entries()
            .iter()
            .map(|entry| (entry.anchors[0].atom_index, entry.anchors[1].atom_index))
            .collect::<Vec<_>>();
        assert_eq!(
            pairs,
            [
                (AtomIndex(0), AtomIndex(2)),
                (AtomIndex(0), AtomIndex(3)),
                (AtomIndex(1), AtomIndex(2)),
                (AtomIndex(1), AtomIndex(3)),
            ]
        );
    }

    #[test]
    fn distance_cartesian_named_mode_deduplicates_self_and_mirrored_pairs() {
        let mut session = measurement_session();

        execute(
            &mut session,
            "distance d, name A+B, name B+A, mode=cartesian",
        )
        .unwrap();

        let measurement = session.registry.get_measurement("d").unwrap();
        assert_eq!(measurement.len(), 1);
        assert_eq!(measurement.entries()[0].anchors[0].atom_index, AtomIndex(0));
        assert_eq!(measurement.entries()[0].anchors[1].atom_index, AtomIndex(1));
    }

    #[test]
    fn distance_rejects_unknown_mode_without_mutation() {
        let mut session = measurement_session();

        let error = execute(&mut session, "distance d, name A, name B, dense").unwrap_err();

        assert!(error.to_string().contains("unknown distance mode 'dense'"));
        assert!(session.registry.get_measurement("d").is_none());
    }

    #[test]
    fn undefined_geometry_is_rejected_without_registry_mutation() {
        let mut session = measurement_session();
        let before = session.registry.len();

        let error = execute(&mut session, "distance bad, name A, name A").unwrap_err();

        assert!(error.to_string().contains("undefined"));
        assert_eq!(session.registry.len(), before);
        assert!(!session.registry.contains("bad"));
    }

    #[test]
    fn mixed_valid_and_undefined_broadcast_does_not_partially_append() {
        let mut session = measurement_session();
        execute(&mut session, "distance d, name A, name B").unwrap();
        session
            .registry
            .get_molecule_mut("source")
            .unwrap()
            .molecule_mut()
            .set_coord(AtomIndex(3), 0, Vec3::new(1.0, 0.0, 0.0));
        let before = session.registry.get_measurement("d").unwrap();
        let before_entries = before.entries().to_vec();
        let before_revisions = before.revisions();

        let error = execute(&mut session, "distance d, name A, name C+D").unwrap_err();

        assert!(error.to_string().contains("undefined"));
        let after = session.registry.get_measurement("d").unwrap();
        assert_eq!(after.entries(), before_entries);
        assert_eq!(after.revisions(), before_revisions);
    }
}
