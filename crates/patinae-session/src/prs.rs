//! Native PRS session format: MessagePack + gzip.

use std::fs::File;
use std::io::{Cursor, Read, Write};
use std::path::Path;

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use patinae_scene::Session;
use rmpv::Value;
use serde::{Deserialize, Serialize};

/// Current native PRS document format version.
pub const PRS_FORMAT_VERSION: u32 = 3;

/// Format version assigned to legacy raw [`Session`] files.
pub const PRS_LEGACY_FORMAT_VERSION: u32 = 1;

/// Native PRS producer name.
pub const PRS_PRODUCER: &str = "patinae";

/// Native PRS producer crate version.
pub const PRS_PRODUCER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// A loaded PRS document plus its format metadata.
#[derive(Serialize, Deserialize)]
pub struct PrsDocument {
    /// PRS envelope format version.
    pub prs_format_version: u32,
    /// Producer name. Missing for legacy raw `Session` files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub producer: Option<String>,
    /// Producer version. Missing for legacy raw `Session` files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub producer_version: Option<String>,
    /// Scene session payload.
    pub session: Session,
}

impl std::fmt::Debug for PrsDocument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PrsDocument")
            .field("prs_format_version", &self.prs_format_version)
            .field("producer", &self.producer)
            .field("producer_version", &self.producer_version)
            .field("session", &"<Session>")
            .finish()
    }
}

impl PrsDocument {
    /// Return user-facing PRS compatibility warnings.
    pub fn warning_messages(&self) -> Vec<String> {
        prs_document_warning_messages(self)
    }

    fn legacy(session: Session) -> Self {
        Self {
            prs_format_version: PRS_LEGACY_FORMAT_VERSION,
            producer: None,
            producer_version: None,
            session,
        }
    }
}

#[derive(Serialize)]
struct PrsDocumentRef<'a> {
    prs_format_version: u32,
    producer: &'static str,
    producer_version: &'static str,
    session: &'a Session,
}

/// Errors for PRS save/load operations.
#[derive(Debug, thiserror::Error)]
pub enum PrsError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serialize(#[from] rmp_serde::encode::Error),

    #[error("deserialization error: {0}")]
    Deserialize(#[from] rmp_serde::decode::Error),

    #[error("invalid PRS data: {0}")]
    InvalidData(String),
}

/// Save a session to a `.prs` file (MessagePack + gzip).
pub fn save_prs(session: &Session, path: &Path) -> Result<(), PrsError> {
    let document = PrsDocumentRef {
        prs_format_version: PRS_FORMAT_VERSION,
        producer: PRS_PRODUCER,
        producer_version: PRS_PRODUCER_VERSION,
        session,
    };
    let data = rmp_serde::to_vec_named(&document)?;
    write_prs_bytes(path, &data)?;
    Ok(())
}

/// Load a session from a `.prs` file (MessagePack + gzip).
pub fn load_prs(path: &Path) -> Result<Session, PrsError> {
    let document = load_prs_document(path)?;
    log_prs_document_warnings(&document);
    Ok(document.session)
}

/// Load a `.prs` file together with PRS format and producer metadata.
pub fn load_prs_document(path: &Path) -> Result<PrsDocument, PrsError> {
    let bytes = read_prs_bytes(path)?;
    decode_prs_document(bytes)
}

/// Decode an uncompressed PRS document from MessagePack bytes.
///
/// Both current PRS envelopes and legacy raw [`Session`] payloads are accepted.
///
/// # Errors
///
/// Returns [`PrsError::InvalidData`] for malformed affected schemas and
/// [`PrsError::Deserialize`] when a validated payload cannot be decoded as a
/// current PRS document or legacy raw session.
pub fn decode_prs_document(bytes: impl AsRef<[u8]>) -> Result<PrsDocument, PrsError> {
    let staged = stage_prs_payload(bytes.as_ref())?;
    match rmp_serde::from_slice::<PrsDocument>(&staged) {
        Ok(document) => Ok(document),
        Err(envelope_error) => match rmp_serde::from_slice::<Session>(&staged) {
            Ok(session) => Ok(PrsDocument::legacy(session)),
            Err(_) => Err(PrsError::Deserialize(envelope_error)),
        },
    }
}

const REGISTRY_MOLECULES_INDEX: usize = 0;
const REGISTRY_MEASUREMENTS_INDEX: usize = 9;
const REGISTRY_LABELS_INDEX: usize = 10;
const MOLECULE_DATA_INDEX: usize = 0;
const INVALID_ATOM_INDEX: u64 = u32::MAX as u64;

fn stage_prs_payload(bytes: &[u8]) -> Result<Vec<u8>, PrsError> {
    let mut cursor = Cursor::new(bytes);
    let mut root = rmpv::decode::read_value(&mut cursor)
        .map_err(|error| PrsError::InvalidData(format!("invalid MessagePack: {error}")))?;
    if cursor.position() != bytes.len() as u64 {
        return Err(PrsError::InvalidData(format!(
            "MessagePack contains {} trailing bytes",
            bytes.len() as u64 - cursor.position()
        )));
    }

    migrate_positional_compatibility(&mut root)?;
    let (format_version, session) = session_value(&root)?;
    let registry = struct_field(session, "registry", 0, "Session")?;
    validate_registry(registry, format_version)?;

    let mut staged = Vec::new();
    rmpv::encode::write_value(&mut staged, &root)
        .map_err(|error| invalid(format!("failed to encode staged PRS data: {error}")))?;
    Ok(staged)
}

fn migrate_positional_compatibility(root: &mut Value) -> Result<(), PrsError> {
    let session = session_value_mut(root)?;
    let Value::Array(session_fields) = session else {
        return Ok(());
    };
    let settings = session_fields
        .get_mut(6)
        .ok_or_else(|| invalid("positional Session has no settings field"))?;
    migrate_positional_settings_layout(settings)?;
    let registry = session_fields
        .get_mut(0)
        .ok_or_else(|| invalid("positional Session has no registry field"))?;
    migrate_positional_override_layouts(registry)
}

fn migrate_positional_settings_layout(settings: &mut Value) -> Result<(), PrsError> {
    let Value::Array(fields) = settings else {
        return Ok(());
    };
    if fields.len() == 17 {
        fields.insert(7, default_measurement_settings_value()?);
        return Ok(());
    }
    if fields.len() != 18 {
        return Ok(());
    }
    let old_layout = fields
        .get(7)
        .and_then(Value::as_array)
        .is_some_and(|object| object.len() == 1);
    if !old_layout {
        return Ok(());
    }

    let old_measurement = fields
        .pop()
        .ok_or_else(|| invalid("old positional Settings omit measurement"))?;
    let old_measurement = value_array(&old_measurement, "legacy MeasurementSettings")?;
    if old_measurement.len() != 17 {
        return Err(invalid(format!(
            "legacy MeasurementSettings has {} fields; expected 17",
            old_measurement.len()
        )));
    }
    let Value::Array(mut current) = default_measurement_settings_value()? else {
        return Err(invalid("MeasurementSettings defaults are not positional"));
    };
    current[0] = old_measurement[0].clone();
    current[1] = old_measurement[1].clone();
    current[2] = old_measurement[2].clone();
    current[3..=15].clone_from_slice(&old_measurement[4..=16]);
    fields.insert(7, Value::Array(current));
    Ok(())
}

fn default_measurement_settings_value() -> Result<Value, PrsError> {
    let bytes = rmp_serde::to_vec(&patinae_settings::groups::MeasurementSettings::default())
        .map_err(PrsError::Serialize)?;
    let mut cursor = Cursor::new(bytes);
    rmpv::decode::read_value(&mut cursor)
        .map_err(|error| invalid(format!("invalid measurement defaults: {error}")))
}

fn migrate_positional_override_layouts(registry: &mut Value) -> Result<(), PrsError> {
    let Value::Array(fields) = registry else {
        return Ok(());
    };
    if let Some(molecules) = fields.get_mut(0) {
        migrate_positional_snapshot_overrides(molecules, 3, "molecule")?;
    }
    if let Some(maps) = fields.get_mut(2) {
        migrate_positional_snapshot_overrides(maps, 8, "map")?;
    }
    Ok(())
}

fn migrate_positional_snapshot_overrides(
    owners: &mut Value,
    overrides_index: usize,
    label: &str,
) -> Result<(), PrsError> {
    let Value::Array(owners) = owners else {
        return Err(invalid(format!(
            "positional {label} owners are not an array"
        )));
    };
    for (index, owner) in owners.iter_mut().enumerate() {
        let Value::Array(pair) = owner else {
            return Err(invalid(format!(
                "invalid {label} owner pair at index {index}"
            )));
        };
        let Some(Value::Array(snapshot)) = pair.get_mut(1) else {
            return Err(invalid(format!(
                "invalid positional {label} snapshot at index {index}"
            )));
        };
        let Some(overrides) = snapshot.get_mut(overrides_index) else {
            return Err(invalid(format!(
                "positional {label} snapshot {index} has no overrides field"
            )));
        };
        let Value::Array(overrides) = overrides else {
            continue;
        };
        if overrides.len() == 11 {
            overrides.pop();
        }
    }
    Ok(())
}

fn session_value(root: &Value) -> Result<(Option<u64>, &Value), PrsError> {
    match root {
        Value::Map(fields) => {
            if let Some(session) = map_value(fields, "session") {
                let format_version = map_value(fields, "prs_format_version")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| invalid("PRS document has no integer format version"))?;
                Ok((Some(format_version), session))
            } else if map_value(fields, "registry").is_some() {
                Ok((None, root))
            } else {
                Err(invalid("named MessagePack root has no Session payload"))
            }
        }
        Value::Array(fields) => {
            if fields.len() == 4 && fields.first().and_then(Value::as_u64).is_some() {
                Ok((fields[0].as_u64(), &fields[3]))
            } else {
                Ok((None, root))
            }
        }
        _ => Err(invalid(
            "MessagePack root is neither a document nor a Session",
        )),
    }
}

fn session_value_mut(root: &mut Value) -> Result<&mut Value, PrsError> {
    enum Location {
        Root,
        Map(usize),
        Array(usize),
    }

    let location = match &*root {
        Value::Map(fields) => fields
            .iter()
            .position(|(key, _)| key.as_str() == Some("session"))
            .map(Location::Map)
            .or_else(|| {
                fields
                    .iter()
                    .any(|(key, _)| key.as_str() == Some("registry"))
                    .then_some(Location::Root)
            })
            .ok_or_else(|| invalid("named MessagePack root has no Session payload"))?,
        Value::Array(fields)
            if fields.len() == 4 && fields.first().and_then(Value::as_u64).is_some() =>
        {
            Location::Array(3)
        }
        Value::Array(_) => Location::Root,
        _ => {
            return Err(invalid(
                "MessagePack root is neither a document nor a Session",
            ))
        }
    };
    match location {
        Location::Root => Ok(root),
        Location::Map(index) => {
            let Value::Map(fields) = root else {
                unreachable!("location was derived from a map")
            };
            Ok(&mut fields[index].1)
        }
        Location::Array(index) => {
            let Value::Array(fields) = root else {
                unreachable!("location was derived from an array")
            };
            Ok(&mut fields[index])
        }
    }
}

fn validate_registry(registry: &Value, format_version: Option<u64>) -> Result<(), PrsError> {
    let field_count = struct_field_count(registry, "ObjectRegistrySnapshot")?;
    let valid_arity = match format_version {
        Some(3) => field_count == 11,
        Some(2) => matches!(field_count, 8 | 9),
        Some(1) => (7..=9).contains(&field_count),
        Some(version) => return Err(invalid(format!("unsupported PRS format version {version}"))),
        None => (7..=11).contains(&field_count),
    };
    if !valid_arity {
        return Err(invalid(format!(
            "unsupported ObjectRegistrySnapshot field count {field_count}"
        )));
    }
    validate_named_fields(
        registry,
        &[
            "molecules",
            "groups",
            "maps",
            "render_order",
            "object_states",
            "render_ids",
            "next_render_id",
            "next_id",
            "generation",
            "measurements",
            "labels",
        ],
        &[
            "molecules",
            "groups",
            "render_order",
            "object_states",
            "next_id",
            "generation",
        ],
        "ObjectRegistrySnapshot",
    )?;

    let molecules = struct_field(
        registry,
        "molecules",
        REGISTRY_MOLECULES_INDEX,
        "ObjectRegistrySnapshot",
    )?;
    validate_molecules(molecules)?;

    if let Some(measurements) =
        optional_struct_field(registry, "measurements", REGISTRY_MEASUREMENTS_INDEX)
    {
        validate_measurements(measurements)?;
    }
    if let Some(labels) = optional_struct_field(registry, "labels", REGISTRY_LABELS_INDEX) {
        validate_labels(labels)?;
    }
    Ok(())
}

fn validate_molecules(molecules: &Value) -> Result<(), PrsError> {
    for (index, owner) in value_array(molecules, "registry molecules")?
        .iter()
        .enumerate()
    {
        let pair = value_array(owner, "molecule owner pair")?;
        if pair.len() != 2 || pair[0].as_str().is_none() {
            return Err(invalid(format!(
                "invalid molecule owner pair at index {index}"
            )));
        }
        validate_struct_arity(&pair[1], &[5], "MoleculeObjectSnapshot")?;
        validate_named_fields(
            &pair[1],
            &[
                "molecule",
                "state",
                "display_state",
                "overrides",
                "surface_quality",
            ],
            &[
                "molecule",
                "state",
                "display_state",
                "overrides",
                "surface_quality",
            ],
            "MoleculeObjectSnapshot",
        )?;
        let molecule = struct_field(
            &pair[1],
            "molecule",
            MOLECULE_DATA_INDEX,
            "MoleculeObjectSnapshot",
        )?;
        validate_struct_arity(molecule, &[10], "ObjectMolecule")?;
        validate_named_fields(
            molecule,
            &[
                "atoms",
                "bonds",
                "coord_sets",
                "name",
                "title",
                "current_state",
                "discrete",
                "settings",
                "unique_settings",
                "symmetry",
            ],
            &[
                "atoms",
                "bonds",
                "coord_sets",
                "name",
                "title",
                "current_state",
                "discrete",
                "settings",
                "unique_settings",
                "symmetry",
            ],
            "ObjectMolecule",
        )?;
    }
    Ok(())
}

fn validate_measurements(measurements: &Value) -> Result<(), PrsError> {
    for (owner_index, owner) in value_array(measurements, "measurement owners")?
        .iter()
        .enumerate()
    {
        let pair = owner_pair(owner, "measurement", owner_index)?;
        let snapshot = &pair[1];
        validate_struct_arity(snapshot, &[5, 6], "MeasurementObjectSnapshot")?;
        validate_named_fields(
            snapshot,
            &[
                "kind",
                "entries",
                "state",
                "color_explicit",
                "revisions",
                "presentation",
            ],
            &["kind", "entries", "state"],
            "MeasurementObjectSnapshot",
        )?;
        let entries = struct_field(snapshot, "entries", 1, "MeasurementObjectSnapshot")?;
        for (entity_index, entity) in value_array(entries, "measurement entities")?
            .iter()
            .enumerate()
        {
            validate_struct_arity(entity, &[1, 2], "MeasurementEntity")?;
            validate_named_fields(
                entity,
                &["anchors", "presentation"],
                &["anchors"],
                "MeasurementEntity",
            )?;
            let anchors = struct_field(entity, "anchors", 0, "MeasurementEntity")?;
            for (anchor_index, anchor) in value_array(anchors, "measurement anchors")?
                .iter()
                .enumerate()
            {
                validate_anchor(
                    anchor,
                    &format!("measurement {owner_index}/{entity_index}/{anchor_index}"),
                )?;
            }
        }
    }
    Ok(())
}

fn validate_labels(labels: &Value) -> Result<(), PrsError> {
    for (owner_index, owner) in value_array(labels, "label owners")?.iter().enumerate() {
        let pair = owner_pair(owner, "label", owner_index)?;
        let snapshot = &pair[1];
        validate_struct_arity(snapshot, &[4], "LabelObjectSnapshot")?;
        validate_named_fields(
            snapshot,
            &["state", "entities", "presentation", "revisions"],
            &["state", "entities"],
            "LabelObjectSnapshot",
        )?;
        let entities = struct_field(snapshot, "entities", 1, "LabelObjectSnapshot")?;
        for (entity_index, entity) in value_array(entities, "label entities")?.iter().enumerate() {
            validate_struct_arity(entity, &[3], "LabelEntity")?;
            validate_named_fields(
                entity,
                &["anchor", "text", "presentation"],
                &["anchor", "text"],
                "LabelEntity",
            )?;
            let anchor = struct_field(entity, "anchor", 0, "LabelEntity")?;
            validate_anchor(anchor, &format!("label {owner_index}/{entity_index}"))?;
        }
    }
    Ok(())
}

fn validate_anchor(anchor: &Value, label: &str) -> Result<(), PrsError> {
    validate_struct_arity(anchor, &[2, 3], "AtomAnchor")?;
    validate_named_fields(
        anchor,
        &["object_name", "atom_index", "orphaned"],
        &["object_name", "atom_index"],
        "AtomAnchor",
    )?;
    let object_name = struct_field(anchor, "object_name", 0, "AtomAnchor")?;
    if object_name.as_str().is_none_or(str::is_empty) {
        return Err(invalid(format!("{label} has an empty source object name")));
    }
    let atom_index = struct_field(anchor, "atom_index", 1, "AtomAnchor")?
        .as_u64()
        .ok_or_else(|| invalid(format!("{label} has a non-integer atom index")))?;
    if atom_index >= INVALID_ATOM_INDEX {
        return Err(invalid(format!(
            "{label} has invalid atom index {atom_index}"
        )));
    }
    Ok(())
}

fn owner_pair<'a>(owner: &'a Value, label: &str, index: usize) -> Result<&'a [Value], PrsError> {
    let pair = value_array(owner, "annotation owner pair")?;
    if pair.len() != 2 || pair[0].as_str().is_none_or(str::is_empty) {
        return Err(invalid(format!(
            "invalid {label} owner pair at index {index}"
        )));
    }
    Ok(pair)
}

fn validate_struct_arity(value: &Value, accepted: &[usize], label: &str) -> Result<(), PrsError> {
    if let Value::Array(fields) = value {
        if !accepted.contains(&fields.len()) {
            return Err(invalid(format!(
                "unsupported {label} field count {}; expected one of {accepted:?}",
                fields.len()
            )));
        }
    }
    Ok(())
}

fn validate_named_fields(
    value: &Value,
    allowed: &[&str],
    required: &[&str],
    label: &str,
) -> Result<(), PrsError> {
    let Value::Map(fields) = value else {
        return Ok(());
    };
    let mut seen = std::collections::HashSet::with_capacity(fields.len());
    for (key, _) in fields {
        let key = key
            .as_str()
            .ok_or_else(|| invalid(format!("{label} contains a non-string field name")))?;
        if !allowed.contains(&key) {
            return Err(invalid(format!("{label} contains unknown field {key}")));
        }
        if !seen.insert(key) {
            return Err(invalid(format!("{label} repeats field {key}")));
        }
    }
    for required in required {
        if !seen.contains(required) {
            return Err(invalid(format!("{label} omits required field {required}")));
        }
    }
    Ok(())
}

fn struct_field_count(value: &Value, label: &str) -> Result<usize, PrsError> {
    match value {
        Value::Array(fields) => Ok(fields.len()),
        Value::Map(fields) => Ok(fields.len()),
        _ => Err(invalid(format!("{label} is neither an array nor a map"))),
    }
}

fn struct_field<'a>(
    value: &'a Value,
    name: &str,
    index: usize,
    label: &str,
) -> Result<&'a Value, PrsError> {
    match value {
        Value::Array(fields) => fields
            .get(index)
            .ok_or_else(|| invalid(format!("{label} has no field {name}"))),
        Value::Map(fields) => {
            map_value(fields, name).ok_or_else(|| invalid(format!("{label} has no field {name}")))
        }
        _ => Err(invalid(format!("{label} is neither an array nor a map"))),
    }
}

fn optional_struct_field<'a>(value: &'a Value, name: &str, index: usize) -> Option<&'a Value> {
    match value {
        Value::Array(fields) => fields.get(index),
        Value::Map(fields) => map_value(fields, name),
        _ => None,
    }
}

fn value_array<'a>(value: &'a Value, label: &str) -> Result<&'a [Value], PrsError> {
    value
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| invalid(format!("{label} is not an array")))
}

fn map_value<'a>(fields: &'a [(Value, Value)], key: &str) -> Option<&'a Value> {
    fields
        .iter()
        .find(|(candidate, _)| candidate.as_str() == Some(key))
        .map(|(_, value)| value)
}

fn invalid(message: impl Into<String>) -> PrsError {
    PrsError::InvalidData(message.into())
}

fn log_prs_document_warnings(document: &PrsDocument) {
    for warning in document.warning_messages() {
        log::warn!("{}", warning);
    }
}

fn prs_document_warning_messages(document: &PrsDocument) -> Vec<String> {
    let mut warnings = Vec::new();

    if document.prs_format_version == PRS_LEGACY_FORMAT_VERSION
        && document.producer.is_none()
        && document.producer_version.is_none()
    {
        warnings.push(format!(
            "Loaded legacy PRS session (format v{}). Re-save it with this Patinae version to upgrade PRS metadata.",
            PRS_LEGACY_FORMAT_VERSION,
        ));
    }

    let Some(producer_version) = document.producer_version.as_deref() else {
        return warnings;
    };
    if document.producer.as_deref() == Some(PRS_PRODUCER)
        && version_is_newer(producer_version, PRS_PRODUCER_VERSION)
    {
        warnings.push(format!(
            "Loaded PRS session produced by newer Patinae version {}. Current Patinae version is {}; some session data may not be interpreted exactly as saved.",
            producer_version,
            PRS_PRODUCER_VERSION,
        ));
    }

    warnings
}

fn version_is_newer(candidate: &str, current: &str) -> bool {
    match (semver_core(candidate), semver_core(current)) {
        (Some(candidate), Some(current)) => candidate > current,
        _ => false,
    }
}

fn semver_core(version: &str) -> Option<[u64; 3]> {
    let core = version
        .split_once(['-', '+'])
        .map_or(version, |(core, _)| core);
    let mut parsed = [0; 3];
    let mut parts = core.split('.');

    for slot in &mut parsed {
        let Some(part) = parts.next() else {
            break;
        };
        if part.is_empty() {
            return None;
        }
        *slot = part.parse().ok()?;
    }

    if parts.next().is_some() {
        return None;
    }

    Some(parsed)
}

fn write_prs_bytes(path: &Path, data: &[u8]) -> Result<(), PrsError> {
    let file = File::create(path)?;
    let mut encoder = GzEncoder::new(file, Compression::default());
    encoder.write_all(data)?;
    encoder.finish()?;
    Ok(())
}

fn read_prs_bytes(path: &Path) -> Result<Vec<u8>, PrsError> {
    let file = File::open(path)?;
    let mut decoder = GzDecoder::new(file);
    let mut bytes = Vec::new();
    decoder.read_to_end(&mut bytes)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lin_alg::f32::{Mat4, Vec3};
    use patinae_color::ColorIndex;
    use patinae_mol::{Atom, CoordSet, Element, ObjectMolecule, RepMask};
    use patinae_scene::{
        AtomAnchor, Camera, GroupObject, LabelEntity, LabelObject, MeasurementAnchor,
        MeasurementEntry, MeasurementKind, MeasurementObject, MoleculeObject, Object, SceneManager,
        SelectionManager, ViewManager,
    };
    use patinae_settings::{ObjectOverrides, Settings};
    use serde::Serialize;
    use std::fs;
    use std::path::PathBuf;

    fn temp_prs_path(name: &str) -> (PathBuf, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("patinae_prs_test_{}_{}", name, std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.prs");
        (dir, path)
    }

    fn cartoon_session_with_restore() -> Session {
        let mut session = Session::new();
        let mut mol = ObjectMolecule::new("mol");
        mol.add_atom(Atom::new("CA", Element::Carbon));
        mol.add_coord_set(CoordSet::from_vec3(&[Vec3::new(0.0, 0.0, 0.0)]));
        let mut obj = MoleculeObject::with_name(mol, "mol");
        obj.state_mut().hide_draw_rep(RepMask::CARTOON);
        session.registry.add(obj);
        session
    }

    #[test]
    fn test_prs_round_trip() {
        let session = cartoon_session_with_restore();
        let (dir, path) = temp_prs_path("roundtrip");

        save_prs(&session, &path).unwrap();
        assert!(path.exists());

        let document = load_prs_document(&path).unwrap();
        assert_eq!(document.prs_format_version, PRS_FORMAT_VERSION);
        assert_eq!(document.producer.as_deref(), Some(PRS_PRODUCER));
        assert_eq!(
            document.producer_version.as_deref(),
            Some(PRS_PRODUCER_VERSION)
        );
        assert!(document
            .session
            .registry
            .get_molecule("mol")
            .unwrap()
            .draw_mask_restorable_reps()
            .is_visible(RepMask::CARTOON));

        let loaded = load_prs(&path).unwrap();
        assert_eq!(loaded.clear_color, session.clear_color);
        assert!(loaded
            .registry
            .get_molecule("mol")
            .unwrap()
            .draw_mask_restorable_reps()
            .is_visible(RepMask::CARTOON));

        // Cleanup
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn measurement_objects_round_trip_with_lifecycle_data() {
        let mut session = cartoon_session_with_restore();
        let atom_index = patinae_mol::AtomIndex(0);
        let mut measurement = MeasurementObject::new("distance", MeasurementKind::Distance);
        measurement
            .add_entry(MeasurementEntry::new(vec![
                MeasurementAnchor::new("mol", atom_index),
                MeasurementAnchor::new("mol", atom_index),
            ]))
            .unwrap();
        measurement.set_color(ColorIndex::Named(6));
        session.registry.add(measurement);
        assert!(session.registry.add_to_group("measurements", "distance"));
        let render_id = session.registry.render_id("distance");
        let revisions = session
            .registry
            .get_measurement("distance")
            .unwrap()
            .revisions();
        let (dir, path) = temp_prs_path("measurement_roundtrip");

        save_prs(&session, &path).unwrap();
        let loaded = load_prs(&path).unwrap();

        let measurement = loaded.registry.get_measurement("distance").unwrap();
        assert_eq!(measurement.kind(), MeasurementKind::Distance);
        assert_eq!(measurement.len(), 1);
        assert_eq!(measurement.state().color, ColorIndex::Named(6));
        assert!(measurement.has_explicit_color());
        assert_eq!(measurement.revisions(), revisions);
        assert_eq!(loaded.registry.render_id("distance"), render_id);
        assert_eq!(
            loaded.registry.parent_group("distance"),
            Some("measurements")
        );
        assert_eq!(measurement.entries()[0].anchors[0].object_name, "mol");
        assert_eq!(measurement.entries()[0].anchors[0].atom_index, atom_index);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_raw_session_loads_as_format_v1_without_producer_metadata() {
        let session = Session::new();
        let (dir, path) = temp_prs_path("legacy_empty");
        let data = rmp_serde::to_vec(&session).unwrap();
        write_prs_bytes(&path, &data).unwrap();

        let document = load_prs_document(&path).unwrap();

        assert_eq!(document.prs_format_version, PRS_LEGACY_FORMAT_VERSION);
        assert_eq!(document.producer, None);
        assert_eq!(document.producer_version, None);
        assert_eq!(document.session.clear_color, session.clear_color);
        assert!(document
            .warning_messages()
            .iter()
            .any(|warning| warning.contains("legacy PRS session")));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn current_document_decodes_from_uncompressed_bytes() {
        let session = cartoon_session_with_restore();
        let document = PrsDocumentRef {
            prs_format_version: PRS_FORMAT_VERSION,
            producer: PRS_PRODUCER,
            producer_version: PRS_PRODUCER_VERSION,
            session: &session,
        };
        let data = rmp_serde::to_vec_named(&document).unwrap();

        let decoded = decode_prs_document(data).unwrap();

        assert_eq!(decoded.prs_format_version, PRS_FORMAT_VERSION);
        assert_eq!(decoded.producer.as_deref(), Some(PRS_PRODUCER));
        assert!(decoded.session.registry.get_molecule("mol").is_some());
    }

    #[test]
    fn label_objects_round_trip_without_reviving_orphaned_anchors() {
        let mut session = cartoon_session_with_restore();
        let atom_index = patinae_mol::AtomIndex(0);
        session.registry.add(LabelObject::with_entities(
            "labels",
            vec![
                LabelEntity::new(AtomAnchor::new("mol", atom_index), "first"),
                LabelEntity::new(AtomAnchor::new("mol", atom_index), "duplicate"),
            ],
        ));
        let mut measurement = MeasurementObject::new("distance", MeasurementKind::Distance);
        measurement
            .add_entry(MeasurementEntry::new(vec![
                MeasurementAnchor::new("mol", atom_index),
                MeasurementAnchor::new("mol", atom_index),
            ]))
            .unwrap();
        session.registry.add(measurement);
        session.registry.remove("mol");
        let mut replacement = ObjectMolecule::new("mol");
        replacement.add_atom(Atom::new("CA", Element::Carbon));
        session.registry.add(MoleculeObject::from_raw(replacement));
        let bytes = rmp_serde::to_vec_named(&PrsDocumentRef {
            prs_format_version: PRS_FORMAT_VERSION,
            producer: PRS_PRODUCER,
            producer_version: PRS_PRODUCER_VERSION,
            session: &session,
        })
        .unwrap();

        let decoded = decode_prs_document(bytes).unwrap();
        let labels = decoded.session.registry.get_label("labels").unwrap();

        assert_eq!(labels.len(), 2);
        assert!(labels
            .entities()
            .iter()
            .all(|entity| entity.anchor().is_orphaned()));
        assert!(decoded
            .session
            .registry
            .get_measurement("distance")
            .unwrap()
            .entries()[0]
            .anchors
            .iter()
            .all(AtomAnchor::is_orphaned));
    }

    #[test]
    fn legacy_atom_labels_migrate_once_in_registry_and_atom_order() {
        let mut session = Session::new();
        session.registry.add(GroupObject::new("label01"));
        session.registry.add(LabelObject::new("label02"));
        for source_name in ["first", "second"] {
            let mut molecule = ObjectMolecule::new(source_name);
            let first = molecule.add_atom(Atom::new("CA", Element::Carbon));
            let second = molecule.add_atom(Atom::new("CB", Element::Carbon));
            molecule.get_atom_mut(first).unwrap().repr.label = format!("{source_name}-visible");
            molecule
                .get_atom_mut(first)
                .unwrap()
                .repr
                .visible_reps
                .set_visible(RepMask::LABELS);
            molecule.get_atom_mut(second).unwrap().repr.label = format!("{source_name}-hidden");
            let mut object = MoleculeObject::from_raw(molecule);
            object.state_mut().visible_reps.set_visible(RepMask::LABELS);
            object.state_mut().draw_reps.set_visible(RepMask::LABELS);
            session.registry.add(object);
        }
        let document = PrsDocumentRef {
            prs_format_version: PRS_FORMAT_VERSION,
            producer: PRS_PRODUCER,
            producer_version: PRS_PRODUCER_VERSION,
            session: &session,
        };
        let bytes = rmp_serde::to_vec_named(&document).unwrap();

        let decoded = decode_prs_document(bytes).unwrap();

        let names = decoded.session.registry.names().collect::<Vec<_>>();
        assert_eq!(names[names.len() - 2..], ["label03", "label04"]);
        for (label_name, source_name) in [("label03", "first"), ("label04", "second")] {
            let labels = decoded.session.registry.get_label(label_name).unwrap();
            assert_eq!(
                labels
                    .entities()
                    .iter()
                    .map(LabelEntity::text)
                    .collect::<Vec<_>>(),
                [
                    format!("{source_name}-visible"),
                    format!("{source_name}-hidden")
                ]
            );
            assert_eq!(labels.entities()[0].presentation().visible(), Some(true));
            assert_eq!(labels.entities()[1].presentation().visible(), Some(false));
            let molecule = decoded.session.registry.get_molecule(source_name).unwrap();
            assert!(molecule
                .molecule()
                .atoms()
                .all(|atom| atom.repr.label.is_empty()));
        }

        let saved_again = rmp_serde::to_vec_named(&PrsDocumentRef {
            prs_format_version: PRS_FORMAT_VERSION,
            producer: PRS_PRODUCER,
            producer_version: PRS_PRODUCER_VERSION,
            session: &decoded.session,
        })
        .unwrap();
        let loaded_again = decode_prs_document(saved_again).unwrap();
        assert_eq!(
            loaded_again
                .session
                .registry
                .names()
                .filter(|name| loaded_again.session.registry.get_label(name).is_some())
                .collect::<Vec<_>>(),
            ["label02", "label03", "label04"]
        );
    }

    #[test]
    fn historical_positional_v2_settings_insert_measurement_defaults() {
        let mut session = Session::new();
        session.settings.cartoon.power = 4.25;
        let mut value = positional_test_document(&session);
        let document = value.as_array_mut_for_test();
        document[0] = Value::from(2_u64);
        let session = document[3].as_array_mut_for_test();
        let registry = session[0].as_array_mut_for_test();
        assert_eq!(registry.len(), 11);
        registry.pop();
        registry.pop();
        assert_eq!(registry.len(), 9);
        let settings = session[6].as_array_mut_for_test();
        assert_eq!(settings.len(), 18);
        settings.remove(7);
        assert_eq!(settings.len(), 17);

        let decoded = decode_prs_document(encode_test_value(&value)).unwrap();

        assert_eq!(decoded.prs_format_version, 2);
        assert_eq!(decoded.session.settings.cartoon.power, 4.25);
        assert_eq!(decoded.session.settings.measurement.dash_length, 0.15);
        assert_eq!(decoded.session.settings.measurement.label_size, 14.0);
    }

    #[test]
    fn current_v3_rejects_unknown_registry_and_invalid_annotation_data() {
        let session = cartoon_session_with_restore();
        let mut unknown_registry = named_test_document(&session);
        let Value::Map(fields) = test_registry_value_mut(&mut unknown_registry) else {
            panic!("registry must be named");
        };
        fields.push((Value::from("unexpected"), Value::Nil));
        assert!(matches!(
            decode_prs_document(encode_test_value(&unknown_registry)),
            Err(PrsError::InvalidData(_))
        ));

        let mut session = cartoon_session_with_restore();
        session.registry.add(LabelObject::with_entities(
            "labels",
            vec![LabelEntity::new(
                AtomAnchor::new("mol", patinae_mol::AtomIndex(0)),
                "CA",
            )],
        ));
        let positional = positional_test_document(&session);
        let mut invalid_anchor = positional.clone();
        first_test_label_anchor_mut(&mut invalid_anchor)[1] = Value::from(u32::MAX as u64);
        assert!(matches!(
            decode_prs_document(encode_test_value(&invalid_anchor)),
            Err(PrsError::InvalidData(_))
        ));

        let mut non_integer_anchor = positional.clone();
        first_test_label_anchor_mut(&mut non_integer_anchor)[1] = Value::F32(1.5);
        assert!(matches!(
            decode_prs_document(encode_test_value(&non_integer_anchor)),
            Err(PrsError::InvalidData(_))
        ));

        let mut truncated = positional;
        let document = truncated.as_array_mut_for_test();
        let session = document[3].as_array_mut_for_test();
        let registry = session[0].as_array_mut_for_test();
        let owners = registry[REGISTRY_LABELS_INDEX].as_array_mut_for_test();
        let owner = owners[0].as_array_mut_for_test();
        owner[1].as_array_mut_for_test().pop();

        assert!(matches!(
            decode_prs_document(encode_test_value(&truncated)),
            Err(PrsError::InvalidData(_))
        ));
    }

    #[test]
    fn corrupt_current_file_is_not_modified_when_staging_fails() {
        let mut session = cartoon_session_with_restore();
        session.registry.add(LabelObject::with_entities(
            "labels",
            vec![LabelEntity::new(
                AtomAnchor::new("mol", patinae_mol::AtomIndex(0)),
                "CA",
            )],
        ));
        let mut value = positional_test_document(&session);
        first_test_label_anchor_mut(&mut value)[1] = Value::from(u32::MAX as u64);
        let payload = encode_test_value(&value);
        let (dir, path) = temp_prs_path("corrupt_unchanged");
        write_prs_bytes(&path, &payload).unwrap();
        let before = fs::read(&path).unwrap();

        assert!(load_prs_document(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);

        let _ = fs::remove_dir_all(&dir);
    }

    fn named_test_document(session: &Session) -> Value {
        let bytes = rmp_serde::to_vec_named(&PrsDocumentRef {
            prs_format_version: PRS_FORMAT_VERSION,
            producer: PRS_PRODUCER,
            producer_version: PRS_PRODUCER_VERSION,
            session,
        })
        .unwrap();
        let mut cursor = Cursor::new(bytes);
        rmpv::decode::read_value(&mut cursor).unwrap()
    }

    fn positional_test_document(session: &Session) -> Value {
        let bytes = rmp_serde::to_vec(&PrsDocumentRef {
            prs_format_version: PRS_FORMAT_VERSION,
            producer: PRS_PRODUCER,
            producer_version: PRS_PRODUCER_VERSION,
            session,
        })
        .unwrap();
        let mut cursor = Cursor::new(bytes);
        rmpv::decode::read_value(&mut cursor).unwrap()
    }

    fn encode_test_value(value: &Value) -> Vec<u8> {
        let mut bytes = Vec::new();
        rmpv::encode::write_value(&mut bytes, value).unwrap();
        bytes
    }

    fn test_registry_value_mut(document: &mut Value) -> &mut Value {
        let session = test_map_field_mut(document, "session");
        test_map_field_mut(session, "registry")
    }

    fn first_test_label_anchor_mut(document: &mut Value) -> &mut Vec<Value> {
        let document = document.as_array_mut_for_test();
        let session = document[3].as_array_mut_for_test();
        let registry = session[0].as_array_mut_for_test();
        let owners = registry[REGISTRY_LABELS_INDEX].as_array_mut_for_test();
        let owner = owners[0].as_array_mut_for_test();
        let snapshot = owner[1].as_array_mut_for_test();
        let entities = snapshot[1].as_array_mut_for_test();
        let entity = entities[0].as_array_mut_for_test();
        entity[0].as_array_mut_for_test()
    }

    fn test_map_field_mut<'a>(value: &'a mut Value, name: &str) -> &'a mut Value {
        let Value::Map(fields) = value else {
            panic!("value must be a map");
        };
        fields
            .iter_mut()
            .find(|(key, _)| key.as_str() == Some(name))
            .map(|(_, value)| value)
            .unwrap()
    }

    trait TestValueExt {
        fn as_array_mut_for_test(&mut self) -> &mut Vec<Value>;
    }

    impl TestValueExt for Value {
        fn as_array_mut_for_test(&mut self) -> &mut Vec<Value> {
            let Value::Array(fields) = self else {
                panic!("value must be an array");
            };
            fields
        }
    }

    #[test]
    fn producer_version_warning_detects_newer_semver_core() {
        assert!(version_is_newer("0.10.0", "0.9.9"));
        assert!(version_is_newer("1.0.0-alpha.1", "0.9.9"));
        assert!(!version_is_newer("0.4.1", "0.4.1"));
        assert!(!version_is_newer("0.4.1+local", "0.4.1"));
        assert!(!version_is_newer("0.4.0", "0.4.1"));
        assert!(!version_is_newer("not-a-version", "0.4.1"));
    }

    #[test]
    fn warning_messages_detect_newer_producer_version() {
        let document = PrsDocument {
            prs_format_version: PRS_FORMAT_VERSION,
            producer: Some(PRS_PRODUCER.to_string()),
            producer_version: Some("999.0.0".to_string()),
            session: Session::new(),
        };

        assert!(document.warning_messages().iter().any(|warning| {
            warning.contains("newer Patinae version 999.0.0")
                && warning.contains(PRS_PRODUCER_VERSION)
        }));
    }

    #[derive(Serialize)]
    struct LegacyObjectState {
        enabled: bool,
        color: ColorIndex,
        visible_reps: RepMask,
        draw_reps: Option<RepMask>,
        #[serde(with = "patinae_scene::serde_helpers::mat4_serde")]
        transform: Mat4,
    }

    #[derive(Serialize)]
    struct LegacyMoleculeObjectSnapshot {
        molecule: ObjectMolecule,
        state: LegacyObjectState,
        display_state: usize,
        overrides: Option<ObjectOverrides>,
        surface_quality: i32,
    }

    #[derive(Serialize)]
    struct LegacyObjectRegistrySnapshot {
        molecules: Vec<(String, LegacyMoleculeObjectSnapshot)>,
        groups: Vec<(String, GroupObject)>,
        render_order: Vec<String>,
        object_states: Vec<(String, LegacyObjectState)>,
        render_ids: Vec<(String, u32)>,
        next_render_id: u32,
        next_id: u32,
        generation: u64,
    }

    /// PRS v2 positional registry layout. `measurements` must remain a
    /// trailing field in the current snapshot so this nine-field sequence
    /// continues to decode with an empty default.
    #[derive(Serialize)]
    struct LegacyV2ObjectRegistrySnapshot {
        molecules: Vec<(String, LegacyMoleculeObjectSnapshot)>,
        groups: Vec<(String, GroupObject)>,
        maps: Vec<(String, ())>,
        render_order: Vec<String>,
        object_states: Vec<(String, LegacyObjectState)>,
        render_ids: Vec<(String, u32)>,
        next_render_id: u32,
        next_id: u32,
        generation: u64,
    }

    #[derive(Serialize)]
    struct LegacySessionRef<'a> {
        registry: LegacyObjectRegistrySnapshot,
        camera: &'a Camera,
        selections: &'a SelectionManager,
        scenes: &'a SceneManager,
        views: &'a ViewManager,
        movie: &'a patinae_scene::Movie,
        settings: &'a Settings,
        named_palette: &'a patinae_scene::NamedPalette,
        palette: &'a patinae_scene::ThemedPalette,
        clear_color: [f32; 3],
        clear_color_set: bool,
    }

    #[derive(Serialize)]
    struct VersionTwoDocumentRef<'a> {
        prs_format_version: u32,
        producer: &'static str,
        producer_version: &'static str,
        session: LegacySessionRef<'a>,
    }

    #[derive(Serialize)]
    struct LegacyV2SessionRef<'a> {
        registry: LegacyV2ObjectRegistrySnapshot,
        camera: &'a Camera,
        selections: &'a SelectionManager,
        scenes: &'a SceneManager,
        views: &'a ViewManager,
        movie: &'a patinae_scene::Movie,
        settings: &'a Settings,
        named_palette: &'a patinae_scene::NamedPalette,
        palette: &'a patinae_scene::ThemedPalette,
        clear_color: [f32; 3],
        clear_color_set: bool,
    }

    #[derive(Serialize)]
    struct PositionalVersionTwoDocumentRef<'a> {
        prs_format_version: u32,
        producer: &'static str,
        producer_version: &'static str,
        session: LegacyV2SessionRef<'a>,
    }

    fn legacy_cartoon_state() -> LegacyObjectState {
        LegacyObjectState {
            enabled: true,
            color: ColorIndex::default(),
            visible_reps: RepMask::CARTOON,
            draw_reps: Some(RepMask::NONE),
            transform: Mat4::new_identity(),
        }
    }

    #[test]
    fn legacy_object_state_sequence_defaults_restore_mask_to_none() {
        let data = rmp_serde::to_vec(&legacy_cartoon_state()).unwrap();
        let state: patinae_scene::ObjectState = rmp_serde::from_slice(&data).unwrap();

        assert!(state.visible_reps.is_visible(RepMask::CARTOON));
        assert!(!state.draw_reps.is_visible(RepMask::CARTOON));
        assert_eq!(state.draw_mask_restorable_reps, RepMask::NONE);
    }

    #[test]
    fn legacy_raw_session_object_state_defaults_restore_mask_to_none() {
        let mut mol = ObjectMolecule::new("legacy");
        mol.add_atom(Atom::new("CA", Element::Carbon));
        mol.add_coord_set(CoordSet::from_vec3(&[Vec3::new(0.0, 0.0, 0.0)]));
        let legacy_registry = LegacyObjectRegistrySnapshot {
            molecules: vec![(
                "legacy".to_string(),
                LegacyMoleculeObjectSnapshot {
                    molecule: mol,
                    state: legacy_cartoon_state(),
                    display_state: 0,
                    overrides: None,
                    surface_quality: 0,
                },
            )],
            groups: Vec::new(),
            render_order: vec!["legacy".to_string()],
            object_states: vec![("legacy".to_string(), legacy_cartoon_state())],
            render_ids: Vec::new(),
            next_render_id: 1,
            next_id: 1,
            generation: 0,
        };
        let base = Session::new();
        let legacy_session = LegacySessionRef {
            registry: legacy_registry,
            camera: &base.camera,
            selections: &base.selections,
            scenes: &base.scenes,
            views: &base.views,
            movie: &base.movie,
            settings: &base.settings,
            named_palette: &base.named_palette,
            palette: &base.palette,
            clear_color: base.clear_color,
            clear_color_set: base.clear_color_set,
        };
        let (dir, path) = temp_prs_path("legacy_restore_flag");
        let data = rmp_serde::to_vec_named(&legacy_session).unwrap();
        write_prs_bytes(&path, &data).unwrap();

        let document = load_prs_document(&path).unwrap();
        let obj = document.session.registry.get_molecule("legacy").unwrap();

        assert_eq!(document.prs_format_version, PRS_LEGACY_FORMAT_VERSION);
        assert!(obj.visible_reps().is_visible(RepMask::CARTOON));
        assert!(!obj.draw_reps().is_visible(RepMask::CARTOON));
        assert_eq!(obj.draw_mask_restorable_reps(), RepMask::NONE);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn version_two_document_defaults_measurements_to_empty() {
        let base = Session::new();
        let legacy_session = LegacySessionRef {
            registry: LegacyObjectRegistrySnapshot {
                molecules: Vec::new(),
                groups: Vec::new(),
                render_order: Vec::new(),
                object_states: Vec::new(),
                render_ids: Vec::new(),
                next_render_id: 1,
                next_id: 1,
                generation: 0,
            },
            camera: &base.camera,
            selections: &base.selections,
            scenes: &base.scenes,
            views: &base.views,
            movie: &base.movie,
            settings: &base.settings,
            named_palette: &base.named_palette,
            palette: &base.palette,
            clear_color: base.clear_color,
            clear_color_set: base.clear_color_set,
        };
        let document = VersionTwoDocumentRef {
            prs_format_version: 2,
            producer: PRS_PRODUCER,
            producer_version: PRS_PRODUCER_VERSION,
            session: legacy_session,
        };
        let data = rmp_serde::to_vec_named(&document).unwrap();

        let decoded = decode_prs_document(data).unwrap();

        assert_eq!(decoded.prs_format_version, 2);
        assert!(decoded.session.registry.is_empty());
    }

    #[test]
    fn positional_version_two_registry_defaults_measurements_to_empty() {
        let base = Session::new();
        let legacy_session = LegacyV2SessionRef {
            registry: LegacyV2ObjectRegistrySnapshot {
                molecules: Vec::new(),
                groups: Vec::new(),
                maps: Vec::new(),
                render_order: Vec::new(),
                object_states: Vec::new(),
                render_ids: Vec::new(),
                next_render_id: 1,
                next_id: 1,
                generation: 0,
            },
            camera: &base.camera,
            selections: &base.selections,
            scenes: &base.scenes,
            views: &base.views,
            movie: &base.movie,
            settings: &base.settings,
            named_palette: &base.named_palette,
            palette: &base.palette,
            clear_color: base.clear_color,
            clear_color_set: base.clear_color_set,
        };
        let document = PositionalVersionTwoDocumentRef {
            prs_format_version: 2,
            producer: PRS_PRODUCER,
            producer_version: PRS_PRODUCER_VERSION,
            session: legacy_session,
        };
        let data = rmp_serde::to_vec(&document).unwrap();

        let decoded = decode_prs_document(data).unwrap();

        assert_eq!(decoded.prs_format_version, 2);
        assert!(decoded.session.registry.is_empty());
    }
}
