// Rust guideline compliant 2026-02-21

use std::env;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use flate2::read::GzDecoder;
use patinae_scene::{ObjectState, Session};
use patinae_session::prs::{
    decode_prs_document, load_prs_document, save_prs, PRS_FORMAT_VERSION,
    PRS_LEGACY_FORMAT_VERSION, PRS_PRODUCER_VERSION,
};
use patinae_settings::groups::{CartoonOverrides, CartoonSettings, Settings};
use patinae_settings::ObjectOverrides;
use rmpv::Value;
use serde::de::DeserializeOwned;
use serde::Serialize;

const HELP: &str = "\
Upgrade legacy Patinae PRS sessions to the current format.

Usage:
  prs-upgrade <input.prs> <output.prs>

The output path must not exist. The input file is never modified.
Supported legacy producers: PyMOL-RS v0.3.3 and Patinae v0.4.0 through v0.4.2.
";

// Session stored `settings` as its seventh positional field before the PRS v2
// envelope was introduced. Changing this index would target unrelated data.
const SESSION_REGISTRY_INDEX: usize = 0;
const SESSION_SCENES_INDEX: usize = 3;
const SESSION_SETTINGS_INDEX: usize = 6;
const SESSION_NAMED_PALETTE_INDEX: usize = 7;
const SESSION_PALETTE_INDEX: usize = 8;
const SESSION_CLEAR_COLOR_INDEX: usize = 9;
const SESSION_CLEAR_COLOR_SET_INDEX: usize = 10;

// PyMOL-RS v0.3.3 stored ElementColors and the unit ChainColors separately
// before Patinae combined them into ThemedPalette.
const V033_SESSION_ELEMENT_COLORS_INDEX: usize = 8;
const V033_SESSION_CLEAR_COLOR_INDEX: usize = 10;

// v0.4.0-v0.4.2 Settings had 15 positional groups. v0.4.3 inserted renderer
// at index 3 and object at index 7, shifting all later fields.
const LEGACY_SETTINGS_TO_CURRENT: [usize; 15] =
    [0, 1, 2, 4, 5, 6, 9, 10, 11, 12, 13, 14, 15, 16, 17];
const LEGACY_SETTINGS_CARTOON_INDEX: usize = 6;

// PyMOL-RS v0.3.3 predates the FXAA, SSAO, renderer, object, and ellipsoid
// groups. The remaining groups retain these semantic destinations.
const V033_SETTINGS_TO_CURRENT: [usize; 12] = [0, 1, 2, 4, 9, 10, 11, 12, 13, 14, 15, 16];
const V033_SETTINGS_UI_INDEX: usize = 1;
const V033_SETTINGS_CARTOON_INDEX: usize = 4;
const V033_SETTINGS_STICK_INDEX: usize = 5;
const V033_SETTINGS_SPHERE_INDEX: usize = 6;
const V033_SETTINGS_SURFACE_INDEX: usize = 7;
const V033_SETTINGS_DOT_INDEX: usize = 10;
const V033_SETTINGS_MESH_INDEX: usize = 11;

const CURRENT_SETTINGS_UI_INDEX: usize = 1;
const CURRENT_SETTINGS_MEASUREMENT_INDEX: usize = 7;
const CURRENT_SETTINGS_OBJECT_INDEX: usize = 8;
const CURRENT_SETTINGS_CARTOON_INDEX: usize = 9;
const CURRENT_SETTINGS_STICK_INDEX: usize = 10;
const CURRENT_SETTINGS_SPHERE_INDEX: usize = 11;
const CURRENT_SETTINGS_SURFACE_INDEX: usize = 12;
const CURRENT_SETTINGS_DOT_INDEX: usize = 15;
const CURRENT_SETTINGS_MESH_INDEX: usize = 16;
const CURRENT_SETTINGS_ELLIPSOID_INDEX: usize = 17;

const V033_UI_TO_CURRENT: [Option<usize>; 10] = [
    Some(1),
    Some(2),
    Some(3),
    Some(4),
    Some(5),
    Some(6),
    Some(7),
    None,
    None,
    None,
];
const V033_CARTOON_TO_CURRENT: [Option<usize>; 15] = [
    Some(0),
    Some(1),
    Some(2),
    Some(3),
    Some(4),
    Some(5),
    Some(6),
    Some(7),
    Some(9),
    Some(10),
    Some(11),
    Some(12),
    Some(13),
    Some(14),
    Some(15),
];
const V033_STICK_TO_CURRENT: [Option<usize>; 5] = [Some(0), Some(1), Some(2), Some(3), None];
const V033_SPHERE_TO_CURRENT: [Option<usize>; 3] = [Some(0), None, Some(1)];
const V033_SURFACE_TO_CURRENT: [Option<usize>; 7] = [
    Some(0),
    Some(9),
    Some(2),
    Some(3),
    Some(6),
    Some(7),
    Some(8),
];
const V033_DOT_TO_CURRENT: [Option<usize>; 3] = [Some(1), Some(2), None];
const V033_MESH_TO_CURRENT: [Option<usize>; 2] = [Some(0), None];

// v0.4.5 inserted nucleic_ladder at index 8 and appended six dimensions to
// CartoonSettings. Existing values keep their original semantic positions.
const LEGACY_CARTOON_TO_CURRENT: [usize; 16] =
    [0, 1, 2, 3, 4, 5, 6, 7, 9, 10, 11, 12, 13, 14, 15, 16];

// v0.4.3 inserted ObjectSettingOverrides before the nine existing groups.
const LEGACY_OVERRIDES_TO_CURRENT: [usize; 9] = [1, 2, 3, 4, 5, 6, 7, 8, 9];
const LEGACY_OVERRIDES_CARTOON_INDEX: usize = 0;

// PyMOL-RS v0.3.3 had eight object-overridable groups. Patinae later added
// the object group at index 0 and ellipsoid at index 9.
const V033_OVERRIDES_TO_CURRENT: [usize; 8] = [1, 2, 3, 4, 5, 6, 7, 8];
const V033_OVERRIDES_CARTOON_INDEX: usize = 0;
const V033_OVERRIDES_STICK_INDEX: usize = 1;
const V033_OVERRIDES_SPHERE_INDEX: usize = 2;
const V033_OVERRIDES_SURFACE_INDEX: usize = 3;
const V033_OVERRIDES_DOT_INDEX: usize = 6;
const V033_OVERRIDES_MESH_INDEX: usize = 7;

const CURRENT_OVERRIDES_CARTOON_INDEX: usize = 1;
const CURRENT_OVERRIDES_STICK_INDEX: usize = 2;
const CURRENT_OVERRIDES_SPHERE_INDEX: usize = 3;
const CURRENT_OVERRIDES_SURFACE_INDEX: usize = 4;
const CURRENT_OVERRIDES_DOT_INDEX: usize = 7;
const CURRENT_OVERRIDES_MESH_INDEX: usize = 8;

// Patinae v0.4 added stable renderer ids to the PyMOL-RS v0.3.3 registry.
const V033_REGISTRY_TO_CURRENT: [usize; 7] = [0, 1, 2, 3, 4, 7, 8];
const REGISTRY_MOLECULES_INDEX: usize = 0;
const REGISTRY_GROUPS_INDEX: usize = 1;
const REGISTRY_MAPS_INDEX: usize = 2;
const REGISTRY_OBJECT_STATES_INDEX: usize = 4;
const MOLECULE_OVERRIDES_INDEX: usize = 3;
const MAP_OVERRIDES_INDEX: usize = 8;
const SNAPSHOT_STATE_INDEX: usize = 1;
const MOLECULE_DATA_INDEX: usize = 0;
const MOLECULE_ATOMS_INDEX: usize = 0;
const ATOM_REPRESENTATION_INDEX: usize = 20;

const GROUP_STATE_INDEX: usize = 1;
const SCENE_MANAGER_SCENES_INDEX: usize = 0;
const SCENE_OBJECT_DATA_INDEX: usize = 7;

#[derive(Debug)]
enum SourceFormat {
    CurrentRawSession,
    LegacyRawSession,
    LegacyNamedSession,
    LegacyDocument {
        format_version: Option<u64>,
        producer_version: Option<String>,
    },
}

impl fmt::Display for SourceFormat {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CurrentRawSession => formatter.write_str("current raw Session"),
            Self::LegacyRawSession => formatter.write_str("legacy raw positional Session"),
            Self::LegacyNamedSession => formatter.write_str("legacy raw named Session"),
            Self::LegacyDocument {
                format_version,
                producer_version,
            } => {
                write!(
                    formatter,
                    "legacy PRS document (format {}, producer {})",
                    format_version
                        .map(|version| format!("v{version}"))
                        .unwrap_or_else(|| "unknown".to_string()),
                    producer_version.as_deref().unwrap_or("unknown"),
                )
            }
        }
    }
}

#[derive(Debug)]
struct UpgradeReport {
    source: SourceFormat,
    object_count: usize,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("prs-upgrade: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let Some((input, output)) = parse_arguments()? else {
        return Ok(());
    };

    let report = upgrade_path(&input, &output)?;
    println!("Upgraded {} -> {}", input.display(), output.display());
    println!("Source: {}", report.source);
    println!("Objects: {}", report.object_count);
    println!("Output: PRS format v{PRS_FORMAT_VERSION}, Patinae {PRS_PRODUCER_VERSION}");
    Ok(())
}

fn parse_arguments() -> Result<Option<(PathBuf, PathBuf)>> {
    let mut arguments = env::args_os();
    let _program = arguments.next();
    let Some(first) = arguments.next() else {
        print!("{HELP}");
        return Ok(None);
    };

    if first == "--help" || first == "-h" {
        print!("{HELP}");
        return Ok(None);
    }
    if first == "--version" || first == "-V" {
        println!("prs-upgrade {PRS_PRODUCER_VERSION}");
        return Ok(None);
    }

    let output = arguments
        .next()
        .context("missing output path; run with --help for usage")?;
    ensure!(
        arguments.next().is_none(),
        "too many arguments; run with --help for usage"
    );
    Ok(Some((PathBuf::from(first), PathBuf::from(output))))
}

fn upgrade_path(input: &Path, output: &Path) -> Result<UpgradeReport> {
    ensure!(
        input
            .extension()
            .is_some_and(|extension| extension == "prs"),
        "input path must have a .prs extension: {}",
        input.display()
    );
    ensure!(
        output
            .extension()
            .is_some_and(|extension| extension == "prs"),
        "output path must have a .prs extension: {}",
        output.display()
    );

    let payload = read_prs_payload(input)?;
    let current_document_error = match decode_prs_document(&payload) {
        Ok(document) => {
            let is_raw_session = document.prs_format_version == PRS_LEGACY_FORMAT_VERSION
                && document.producer.is_none()
                && document.producer_version.is_none();
            if !is_raw_session {
                bail!(
                    "{} already uses a PRS document readable by this Patinae version",
                    input.display()
                );
            }
            None
        }
        Err(error) => Some(error),
    };

    let (session, source) = match rmp_serde::from_slice::<Session>(&payload) {
        Ok(session) => (session, SourceFormat::CurrentRawSession),
        Err(current_error) => {
            let root = decode_value(&payload).with_context(|| {
                format!(
                    "{} is neither a current PRS document nor a supported legacy MessagePack value; current decoder: {current_error}",
                    input.display()
                )
            })?;
            if let Some(format_version) = document_format_version(&root) {
                if format_version >= u64::from(PRS_FORMAT_VERSION) {
                    let document_error = current_document_error
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_else(|| "unknown document error".to_string());
                    bail!(
                        "refusing to reinterpret PRS v{format_version} envelope as a legacy session; current decoder rejected it: {document_error}"
                    );
                }
            }
            let (mut session_value, source) = extract_session(root)?;
            migrate_session(&mut session_value)?;
            let migrated_payload = encode_value(&session_value)?;
            let session = match rmp_serde::from_slice::<Session>(&migrated_payload) {
                Ok(session) => session,
                Err(migrated_error) => {
                    let field_diagnostic = validate_positional_session_fields(&session_value)
                        .err()
                        .map(|error| format!("; migrated field validation: {error:#}"))
                        .unwrap_or_default();
                    bail!(
                        "legacy migration produced a Session that the current decoder rejected: {migrated_error}; original decoder: {current_error}{field_diagnostic}"
                    )
                }
            };
            (session, source)
        }
    };

    write_verified_session(&session, output)?;
    Ok(UpgradeReport {
        source,
        object_count: session.registry.len(),
    })
}

fn read_prs_payload(path: &Path) -> Result<Vec<u8>> {
    let file =
        File::open(path).with_context(|| format!("failed to open input PRS {}", path.display()))?;
    let mut decoder = GzDecoder::new(file);
    let mut payload = Vec::new();
    decoder
        .read_to_end(&mut payload)
        .with_context(|| format!("failed to decompress input PRS {}", path.display()))?;
    Ok(payload)
}

fn write_verified_session(session: &Session, output: &Path) -> Result<()> {
    let claim = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .with_context(|| {
            format!(
                "output already exists or cannot be created: {}",
                output.display()
            )
        })?;
    drop(claim);

    let result = (|| -> Result<()> {
        save_prs(session, output)
            .with_context(|| format!("failed to write upgraded PRS {}", output.display()))?;
        let document = load_prs_document(output)
            .with_context(|| format!("failed to verify upgraded PRS {}", output.display()))?;
        ensure!(
            document.prs_format_version == PRS_FORMAT_VERSION,
            "verification returned PRS format v{}, expected v{}",
            document.prs_format_version,
            PRS_FORMAT_VERSION
        );
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(output);
    }
    result
}

fn extract_session(root: Value) -> Result<(Value, SourceFormat)> {
    match root {
        Value::Array(_) => Ok((root, SourceFormat::LegacyRawSession)),
        Value::Map(mut fields) => {
            if let Some(session) = remove_map_value(&mut fields, "session") {
                let format_version =
                    map_value(&fields, "prs_format_version").and_then(Value::as_u64);
                let producer_version = map_value(&fields, "producer_version")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                Ok((
                    session,
                    SourceFormat::LegacyDocument {
                        format_version,
                        producer_version,
                    },
                ))
            } else if map_value(&fields, "registry").is_some()
                && map_value(&fields, "settings").is_some()
            {
                Ok((Value::Map(fields), SourceFormat::LegacyNamedSession))
            } else {
                bail!("unsupported named MessagePack root: no Session payload")
            }
        }
        other => bail!(
            "unsupported MessagePack root {}; expected a Session array or map",
            value_kind(&other)
        ),
    }
}

fn document_format_version(root: &Value) -> Option<u64> {
    match root {
        Value::Map(fields) if map_value(fields, "session").is_some() => {
            map_value(fields, "prs_format_version").and_then(Value::as_u64)
        }
        Value::Array(fields) if fields.len() == 4 => fields.first().and_then(Value::as_u64),
        _ => None,
    }
}

fn migrate_session(session: &mut Value) -> Result<()> {
    match session {
        Value::Array(_) => migrate_positional_session(session),
        Value::Map(_) => migrate_named_session(session),
        other => bail!(
            "unsupported Session encoding {}; expected an array or map",
            value_kind(other)
        ),
    }
}

fn migrate_positional_session(session: &mut Value) -> Result<()> {
    let fields = array_mut(session, "Session")?;
    ensure!(
        fields.len() == 10 || fields.len() == 11,
        "unsupported positional Session field count {}; expected 10 or 11",
        fields.len()
    );
    let registry_field_count = fields
        .get(SESSION_REGISTRY_INDEX)
        .and_then(Value::as_array)
        .map(Vec::len)
        .context("legacy Session registry is not an array")?;
    let is_v033 = registry_field_count == V033_REGISTRY_TO_CURRENT.len();

    migrate_positional_registry(
        fields
            .get_mut(SESSION_REGISTRY_INDEX)
            .context("legacy Session has no registry field")?,
    )?;
    migrate_positional_settings(
        fields
            .get_mut(SESSION_SETTINGS_INDEX)
            .context("legacy Session has no settings field")?,
    )?;

    if is_v033 {
        migrate_v033_scenes(
            fields
                .get_mut(SESSION_SCENES_INDEX)
                .context("PyMOL-RS v0.3.3 Session has no scenes field")?,
        )?;
        migrate_v033_session_colors(fields)?;
    }
    Ok(())
}

fn migrate_positional_settings(settings: &mut Value) -> Result<()> {
    let mut legacy = take_array(settings, "legacy Settings")?;
    match legacy.len() {
        count if count == LEGACY_SETTINGS_TO_CURRENT.len() => {
            migrate_positional_cartoon(
                legacy
                    .get_mut(LEGACY_SETTINGS_CARTOON_INDEX)
                    .context("legacy Settings has no cartoon group")?,
                false,
            )?;
            *settings = merge_positional(
                legacy,
                positional_value(&Settings::default())?,
                &LEGACY_SETTINGS_TO_CURRENT,
                "Settings",
            )?;
        }
        count if count == V033_SETTINGS_TO_CURRENT.len() => {
            migrate_v033_positional_settings(&mut legacy)?;
            *settings = merge_positional(
                legacy,
                positional_value(&Settings::default())?,
                &V033_SETTINGS_TO_CURRENT,
                "PyMOL-RS v0.3.3 Settings",
            )?;
        }
        count => {
            bail!(
                "unsupported legacy Settings field count {count}; expected {} for PyMOL-RS v0.3.3 or {} for Patinae v0.4.0-v0.4.2",
                V033_SETTINGS_TO_CURRENT.len(),
                LEGACY_SETTINGS_TO_CURRENT.len()
            )
        }
    }
    Ok(())
}

fn migrate_positional_object_overrides(overrides: &mut Value) -> Result<()> {
    let mut legacy = take_array(overrides, "legacy ObjectOverrides")?;
    match legacy.len() {
        count if count == LEGACY_OVERRIDES_TO_CURRENT.len() => {
            migrate_positional_cartoon(
                legacy
                    .get_mut(LEGACY_OVERRIDES_CARTOON_INDEX)
                    .context("legacy ObjectOverrides has no cartoon group")?,
                true,
            )?;
            *overrides = merge_positional(
                legacy,
                positional_value(&ObjectOverrides::default())?,
                &LEGACY_OVERRIDES_TO_CURRENT,
                "ObjectOverrides",
            )?;
        }
        count if count == V033_OVERRIDES_TO_CURRENT.len() => {
            migrate_v033_positional_overrides(&mut legacy)?;
            *overrides = merge_positional(
                legacy,
                positional_value(&ObjectOverrides::default())?,
                &V033_OVERRIDES_TO_CURRENT,
                "PyMOL-RS v0.3.3 ObjectOverrides",
            )?;
        }
        count => {
            bail!(
                "unsupported legacy ObjectOverrides field count {count}; expected {} for PyMOL-RS v0.3.3 or {} for Patinae v0.4.0-v0.4.2",
                V033_OVERRIDES_TO_CURRENT.len(),
                LEGACY_OVERRIDES_TO_CURRENT.len()
            )
        }
    }
    Ok(())
}

fn migrate_v033_positional_settings(legacy: &mut [Value]) -> Result<()> {
    let defaults = take_array(
        &mut positional_value(&Settings::default())?,
        "current Settings defaults",
    )?;
    migrate_v033_group(
        legacy,
        V033_SETTINGS_UI_INDEX,
        &defaults,
        CURRENT_SETTINGS_UI_INDEX,
        &V033_UI_TO_CURRENT,
        "UI settings",
    )?;
    migrate_v033_group(
        legacy,
        V033_SETTINGS_CARTOON_INDEX,
        &defaults,
        CURRENT_SETTINGS_CARTOON_INDEX,
        &V033_CARTOON_TO_CURRENT,
        "cartoon settings",
    )?;
    migrate_v033_group(
        legacy,
        V033_SETTINGS_STICK_INDEX,
        &defaults,
        CURRENT_SETTINGS_STICK_INDEX,
        &V033_STICK_TO_CURRENT,
        "stick settings",
    )?;
    migrate_v033_group(
        legacy,
        V033_SETTINGS_SPHERE_INDEX,
        &defaults,
        CURRENT_SETTINGS_SPHERE_INDEX,
        &V033_SPHERE_TO_CURRENT,
        "sphere settings",
    )?;
    migrate_v033_group(
        legacy,
        V033_SETTINGS_SURFACE_INDEX,
        &defaults,
        CURRENT_SETTINGS_SURFACE_INDEX,
        &V033_SURFACE_TO_CURRENT,
        "surface settings",
    )?;
    migrate_v033_group(
        legacy,
        V033_SETTINGS_DOT_INDEX,
        &defaults,
        CURRENT_SETTINGS_DOT_INDEX,
        &V033_DOT_TO_CURRENT,
        "dot settings",
    )?;
    migrate_v033_group(
        legacy,
        V033_SETTINGS_MESH_INDEX,
        &defaults,
        CURRENT_SETTINGS_MESH_INDEX,
        &V033_MESH_TO_CURRENT,
        "mesh settings",
    )?;
    Ok(())
}

fn migrate_v033_positional_overrides(legacy: &mut [Value]) -> Result<()> {
    let defaults = take_array(
        &mut positional_value(&ObjectOverrides::default())?,
        "current ObjectOverrides defaults",
    )?;
    migrate_v033_group(
        legacy,
        V033_OVERRIDES_CARTOON_INDEX,
        &defaults,
        CURRENT_OVERRIDES_CARTOON_INDEX,
        &V033_CARTOON_TO_CURRENT,
        "cartoon overrides",
    )?;
    migrate_v033_group(
        legacy,
        V033_OVERRIDES_STICK_INDEX,
        &defaults,
        CURRENT_OVERRIDES_STICK_INDEX,
        &V033_STICK_TO_CURRENT,
        "stick overrides",
    )?;
    migrate_v033_group(
        legacy,
        V033_OVERRIDES_SPHERE_INDEX,
        &defaults,
        CURRENT_OVERRIDES_SPHERE_INDEX,
        &V033_SPHERE_TO_CURRENT,
        "sphere overrides",
    )?;
    migrate_v033_group(
        legacy,
        V033_OVERRIDES_SURFACE_INDEX,
        &defaults,
        CURRENT_OVERRIDES_SURFACE_INDEX,
        &V033_SURFACE_TO_CURRENT,
        "surface overrides",
    )?;
    migrate_v033_group(
        legacy,
        V033_OVERRIDES_DOT_INDEX,
        &defaults,
        CURRENT_OVERRIDES_DOT_INDEX,
        &V033_DOT_TO_CURRENT,
        "dot overrides",
    )?;
    migrate_v033_group(
        legacy,
        V033_OVERRIDES_MESH_INDEX,
        &defaults,
        CURRENT_OVERRIDES_MESH_INDEX,
        &V033_MESH_TO_CURRENT,
        "mesh overrides",
    )?;
    Ok(())
}

fn migrate_v033_group(
    legacy_groups: &mut [Value],
    legacy_index: usize,
    current_defaults: &[Value],
    current_index: usize,
    legacy_to_current: &[Option<usize>],
    label: &str,
) -> Result<()> {
    let group = legacy_groups
        .get_mut(legacy_index)
        .with_context(|| format!("PyMOL-RS v0.3.3 Settings has no {label}"))?;
    let legacy = take_array(group, &format!("PyMOL-RS v0.3.3 {label}"))?;
    let defaults = current_defaults
        .get(current_index)
        .with_context(|| format!("current Settings defaults have no {label}"))?
        .clone();
    *group = merge_selected_positional(legacy, defaults, legacy_to_current, label)?;
    Ok(())
}

fn migrate_positional_cartoon(cartoon: &mut Value, overrides: bool) -> Result<()> {
    let legacy = take_array(cartoon, "legacy cartoon settings")?;
    ensure!(
        legacy.len() == LEGACY_CARTOON_TO_CURRENT.len(),
        "unsupported legacy cartoon field count {}; expected {}",
        legacy.len(),
        LEGACY_CARTOON_TO_CURRENT.len()
    );
    let defaults = if overrides {
        positional_value(&CartoonOverrides::default())?
    } else {
        positional_value(&CartoonSettings::default())?
    };
    *cartoon = merge_positional(
        legacy,
        defaults,
        &LEGACY_CARTOON_TO_CURRENT,
        "cartoon settings",
    )?;
    Ok(())
}

fn migrate_positional_registry(registry: &mut Value) -> Result<()> {
    let field_count = registry
        .as_array()
        .map(Vec::len)
        .context("ObjectRegistrySnapshot is not an array")?;
    ensure!(
        field_count == 11
            || field_count == 9
            || field_count == V033_REGISTRY_TO_CURRENT.len(),
        "unsupported ObjectRegistrySnapshot field count {field_count}; expected {} for PyMOL-RS v0.3.3, 9 for Patinae v0.4.0-v0.4.2, or 11 for a current raw Session",
        V033_REGISTRY_TO_CURRENT.len()
    );
    let is_v033 = field_count == V033_REGISTRY_TO_CURRENT.len();
    let fields = array_mut(registry, "ObjectRegistrySnapshot")?;
    migrate_positional_snapshots(
        fields
            .get_mut(REGISTRY_MOLECULES_INDEX)
            .context("legacy registry has no molecules field")?,
        MOLECULE_OVERRIDES_INDEX,
        "molecule",
        is_v033,
    )?;
    migrate_positional_snapshots(
        fields
            .get_mut(REGISTRY_MAPS_INDEX)
            .context("legacy registry has no maps field")?,
        MAP_OVERRIDES_INDEX,
        "map",
        is_v033,
    )?;

    if is_v033 {
        migrate_v033_groups(
            fields
                .get_mut(REGISTRY_GROUPS_INDEX)
                .context("PyMOL-RS v0.3.3 registry has no groups field")?,
        )?;
        migrate_v033_named_object_states(
            fields
                .get_mut(REGISTRY_OBJECT_STATES_INDEX)
                .context("PyMOL-RS v0.3.3 registry has no object states field")?,
        )?;
        let legacy = take_array(registry, "PyMOL-RS v0.3.3 ObjectRegistrySnapshot")?;
        *registry = merge_positional(
            legacy,
            current_registry_defaults()?,
            &V033_REGISTRY_TO_CURRENT,
            "ObjectRegistrySnapshot",
        )?;
    }
    Ok(())
}

fn migrate_positional_snapshots(
    snapshots: &mut Value,
    overrides_index: usize,
    label: &str,
    migrate_v033_state: bool,
) -> Result<()> {
    for (index, entry) in array_mut(snapshots, "registry snapshot collection")?
        .iter_mut()
        .enumerate()
    {
        let pair = array_mut(entry, "named object tuple")
            .with_context(|| format!("invalid {label} entry at index {index}"))?;
        ensure!(
            pair.len() == 2,
            "invalid {label} entry at index {index}: expected a name/data pair"
        );
        let snapshot = array_mut(&mut pair[1], "object snapshot")
            .with_context(|| format!("invalid {label} snapshot at index {index}"))?;
        if migrate_v033_state {
            if label == "molecule" {
                migrate_v033_molecule(snapshot.get_mut(MOLECULE_DATA_INDEX).with_context(
                    || format!("molecule snapshot at index {index} has no molecule field"),
                )?)
                .with_context(|| format!("failed to migrate molecule data at index {index}"))?;
            }
            migrate_v033_object_state(snapshot.get_mut(SNAPSHOT_STATE_INDEX).with_context(
                || format!("{label} snapshot at index {index} has no state field"),
            )?)
            .with_context(|| format!("failed to migrate {label} state at index {index}"))?;
        }
        let overrides = snapshot
            .get_mut(overrides_index)
            .with_context(|| format!("{label} snapshot at index {index} has no overrides field"))?;
        if !overrides.is_nil() {
            migrate_positional_object_overrides(overrides)
                .with_context(|| format!("failed to migrate {label} overrides at index {index}"))?;
        }
    }
    Ok(())
}

fn migrate_v033_molecule(molecule: &mut Value) -> Result<()> {
    let molecule_fields = array_mut(molecule, "PyMOL-RS v0.3.3 ObjectMolecule")?;
    let atoms = molecule_fields
        .get_mut(MOLECULE_ATOMS_INDEX)
        .context("PyMOL-RS v0.3.3 ObjectMolecule has no atoms field")?;
    for (index, atom) in array_mut(atoms, "PyMOL-RS v0.3.3 atom collection")?
        .iter_mut()
        .enumerate()
    {
        let atom_fields = array_mut(atom, "PyMOL-RS v0.3.3 Atom")
            .with_context(|| format!("invalid atom at index {index}"))?;
        let representation = atom_fields
            .get_mut(ATOM_REPRESENTATION_INDEX)
            .with_context(|| format!("atom at index {index} has no representation field"))?;
        migrate_v033_atom_representation(representation)
            .with_context(|| format!("failed to migrate atom representation at index {index}"))?;
    }
    Ok(())
}

fn migrate_v033_atom_representation(representation: &mut Value) -> Result<()> {
    let legacy = take_array(representation, "PyMOL-RS v0.3.3 AtomRepresentation")?;
    ensure!(
        legacy.len() == 9,
        "PyMOL-RS v0.3.3 AtomRepresentation has {} fields; expected 9",
        legacy.len()
    );
    let [
        colors,
        sphere_scale,
        visible_reps,
        cartoon,
        text_type,
        label,
        masked,
        unique_id,
        has_setting,
    ]: [Value; 9] = legacy.try_into().map_err(|values: Vec<Value>| {
        anyhow::anyhow!(
            "PyMOL-RS v0.3.3 AtomRepresentation has {} fields; expected 9",
            values.len()
        )
    })?;
    let mut colors = match colors {
        Value::Array(fields) => fields,
        other => bail!(
            "PyMOL-RS v0.3.3 AtomColors is {}, expected an array",
            value_kind(&other)
        ),
    };
    ensure!(
        colors.len() == 8,
        "PyMOL-RS v0.3.3 AtomColors has {} fields; expected 8",
        colors.len()
    );
    colors.push(Value::from(i32::MIN));
    colors.push(Value::from(i32::MIN));
    *representation = Value::Array(vec![
        Value::Array(colors),
        sphere_scale,
        Value::Nil,
        Value::Nil,
        Value::Nil,
        Value::Nil,
        Value::Nil,
        visible_reps,
        cartoon,
        text_type,
        label,
        masked,
        unique_id,
        has_setting,
    ]);
    Ok(())
}

fn current_registry_defaults() -> Result<Value> {
    let mut session = take_array(
        &mut positional_value(&Session::new())?,
        "current Session defaults",
    )?;
    session
        .get_mut(SESSION_REGISTRY_INDEX)
        .map(|registry| std::mem::replace(registry, Value::Nil))
        .context("current Session defaults have no registry field")
}

fn migrate_v033_groups(groups: &mut Value) -> Result<()> {
    for (index, entry) in array_mut(groups, "registry group collection")?
        .iter_mut()
        .enumerate()
    {
        let pair = array_mut(entry, "named group tuple")
            .with_context(|| format!("invalid group entry at index {index}"))?;
        ensure!(
            pair.len() == 2,
            "invalid group entry at index {index}: expected a name/data pair"
        );
        let group = array_mut(&mut pair[1], "GroupObject")
            .with_context(|| format!("invalid group snapshot at index {index}"))?;
        migrate_v033_object_state(
            group
                .get_mut(GROUP_STATE_INDEX)
                .with_context(|| format!("group snapshot at index {index} has no state field"))?,
        )
        .with_context(|| format!("failed to migrate group state at index {index}"))?;
    }
    Ok(())
}

fn migrate_v033_named_object_states(states: &mut Value) -> Result<()> {
    for (index, entry) in array_mut(states, "registry object state collection")?
        .iter_mut()
        .enumerate()
    {
        let pair = array_mut(entry, "named object state tuple")
            .with_context(|| format!("invalid object state entry at index {index}"))?;
        ensure!(
            pair.len() == 2,
            "invalid object state entry at index {index}: expected a name/state pair"
        );
        migrate_v033_object_state(&mut pair[1])
            .with_context(|| format!("failed to migrate object state at index {index}"))?;
    }
    Ok(())
}

fn migrate_v033_object_state(state: &mut Value) -> Result<()> {
    let legacy = take_array(state, "PyMOL-RS v0.3.3 ObjectState")?;
    ensure!(
        legacy.len() == 4,
        "unsupported PyMOL-RS v0.3.3 ObjectState field count {}; expected 4",
        legacy.len()
    );
    let visible_reps = legacy
        .get(2)
        .context("PyMOL-RS v0.3.3 ObjectState has no visible_reps field")?
        .clone();
    let mut current = match merge_positional(
        legacy,
        positional_value(&ObjectState::default())?,
        &[0, 1, 2, 4],
        "ObjectState",
    )? {
        Value::Array(fields) => fields,
        _ => unreachable!("merge_positional always returns an array"),
    };
    current[3] = visible_reps;
    *state = Value::Array(current);
    Ok(())
}

fn migrate_v033_scenes(scenes: &mut Value) -> Result<()> {
    let manager = array_mut(scenes, "PyMOL-RS v0.3.3 SceneManager")?;
    let scene_map = manager
        .get_mut(SCENE_MANAGER_SCENES_INDEX)
        .context("PyMOL-RS v0.3.3 SceneManager has no scenes map")?;
    let Value::Map(entries) = scene_map else {
        bail!(
            "PyMOL-RS v0.3.3 SceneManager scenes are {}, expected a map",
            value_kind(scene_map)
        )
    };
    for (scene_name, scene) in entries {
        let scene_label = scene_name.as_str().unwrap_or("<unnamed>");
        let scene_fields = array_mut(scene, "PyMOL-RS v0.3.3 Scene")
            .with_context(|| format!("invalid scene {scene_label}"))?;
        let object_data = scene_fields
            .get_mut(SCENE_OBJECT_DATA_INDEX)
            .with_context(|| format!("scene {scene_label} has no object_data field"))?;
        let Value::Map(objects) = object_data else {
            bail!(
                "scene {scene_label} object_data is {}, expected a map",
                value_kind(object_data)
            )
        };
        for (object_name, data) in objects {
            let object_label = object_name.as_str().unwrap_or("<unnamed>");
            let legacy = take_array(data, "PyMOL-RS v0.3.3 SceneObjectData")
                .with_context(|| format!("invalid scene object {object_label}"))?;
            ensure!(
                legacy.len() == 5,
                "scene object {object_label} has {} fields; expected 5",
                legacy.len()
            );
            let draw_reps = legacy
                .get(2)
                .context("PyMOL-RS v0.3.3 SceneObjectData has no visible_reps field")?
                .clone();
            let [enabled, color, visible_reps, current_state, per_atom_data]: [Value; 5] =
                legacy.try_into().map_err(|values: Vec<Value>| {
                    anyhow::anyhow!(
                        "scene object {object_label} has {} fields; expected 5",
                        values.len()
                    )
                })?;
            *data = Value::Array(vec![
                enabled,
                color,
                visible_reps,
                draw_reps,
                Value::Nil,
                current_state,
                per_atom_data,
            ]);
        }
    }
    Ok(())
}

fn migrate_v033_session_colors(fields: &mut [Value]) -> Result<()> {
    ensure!(
        fields.len() == 11,
        "PyMOL-RS v0.3.3 Session has {} fields; expected 11",
        fields.len()
    );
    let element_colors = std::mem::replace(
        fields
            .get_mut(V033_SESSION_ELEMENT_COLORS_INDEX)
            .context("PyMOL-RS v0.3.3 Session has no element colors field")?,
        Value::Nil,
    );
    let clear_color = std::mem::replace(
        fields
            .get_mut(V033_SESSION_CLEAR_COLOR_INDEX)
            .context("PyMOL-RS v0.3.3 Session has no clear color field")?,
        Value::Nil,
    );

    let mut defaults = take_array(
        &mut positional_value(&Session::new())?,
        "current Session defaults",
    )?;
    let mut palette = std::mem::replace(
        defaults
            .get_mut(SESSION_PALETTE_INDEX)
            .context("current Session defaults have no themed palette field")?,
        Value::Nil,
    );
    let palette_fields = array_mut(&mut palette, "current ThemedPalette defaults")?;
    let element_palette = palette_fields
        .get_mut(0)
        .context("current ThemedPalette defaults have no element palette")?;
    *element_palette = element_colors;

    fields[SESSION_PALETTE_INDEX] = palette;
    fields[SESSION_CLEAR_COLOR_INDEX] = clear_color;
    fields[SESSION_CLEAR_COLOR_SET_INDEX] = Value::Boolean(true);
    ensure!(
        !fields[SESSION_NAMED_PALETTE_INDEX].is_nil(),
        "PyMOL-RS v0.3.3 Session has no named colors"
    );
    Ok(())
}

fn migrate_named_session(session: &mut Value) -> Result<()> {
    {
        let registry =
            map_value_mut(session, "registry").context("legacy Session has no registry field")?;
        migrate_named_registry(registry)?;
    }
    let settings =
        map_value_mut(session, "settings").context("legacy Session has no settings field")?;
    migrate_named_settings(settings)
}

fn migrate_named_settings(settings: &mut Value) -> Result<()> {
    if let Some(cartoon) = optional_map_value_mut(settings, "cartoon")? {
        migrate_named_cartoon(cartoon, false)?;
    }
    merge_named(settings, named_value(&Settings::default())?, "Settings")
}

fn migrate_named_object_overrides(overrides: &mut Value) -> Result<()> {
    if let Some(cartoon) = optional_map_value_mut(overrides, "cartoon")? {
        migrate_named_cartoon(cartoon, true)?;
    }
    merge_named(
        overrides,
        named_value(&ObjectOverrides::default())?,
        "ObjectOverrides",
    )
}

fn migrate_named_cartoon(cartoon: &mut Value, overrides: bool) -> Result<()> {
    let defaults = if overrides {
        named_value(&CartoonOverrides::default())?
    } else {
        named_value(&CartoonSettings::default())?
    };
    merge_named(cartoon, defaults, "cartoon settings")
}

fn migrate_named_registry(registry: &mut Value) -> Result<()> {
    if let Some(molecules) = optional_map_value_mut(registry, "molecules")? {
        migrate_named_snapshots(molecules, "molecule")?;
    }
    if let Some(maps) = optional_map_value_mut(registry, "maps")? {
        migrate_named_snapshots(maps, "map")?;
    }
    Ok(())
}

fn migrate_named_snapshots(snapshots: &mut Value, label: &str) -> Result<()> {
    for (index, entry) in array_mut(snapshots, "registry snapshot collection")?
        .iter_mut()
        .enumerate()
    {
        let pair = array_mut(entry, "named object tuple")
            .with_context(|| format!("invalid {label} entry at index {index}"))?;
        ensure!(
            pair.len() == 2,
            "invalid {label} entry at index {index}: expected a name/data pair"
        );
        if let Some(overrides) = optional_map_value_mut(&mut pair[1], "overrides")
            .with_context(|| format!("invalid {label} snapshot at index {index}"))?
        {
            if !overrides.is_nil() {
                migrate_named_object_overrides(overrides).with_context(|| {
                    format!("failed to migrate {label} overrides at index {index}")
                })?;
            }
        }
    }
    Ok(())
}

fn merge_positional(
    legacy: Vec<Value>,
    current_defaults: Value,
    legacy_to_current: &[usize],
    label: &str,
) -> Result<Value> {
    ensure!(
        legacy.len() == legacy_to_current.len(),
        "{label} migration mapping has {} entries for {} legacy fields",
        legacy_to_current.len(),
        legacy.len()
    );
    let mut current = match current_defaults {
        Value::Array(fields) => fields,
        other => bail!(
            "current {label} defaults encoded as {}, expected an array",
            value_kind(&other)
        ),
    };
    for (legacy_value, current_index) in legacy.into_iter().zip(legacy_to_current) {
        let field = current.get_mut(*current_index).with_context(|| {
            format!("{label} migration targets missing current field {current_index}")
        })?;
        *field = legacy_value;
    }
    Ok(Value::Array(current))
}

fn merge_selected_positional(
    legacy: Vec<Value>,
    current_defaults: Value,
    legacy_to_current: &[Option<usize>],
    label: &str,
) -> Result<Value> {
    ensure!(
        legacy.len() == legacy_to_current.len(),
        "{label} migration mapping has {} entries for {} legacy fields",
        legacy_to_current.len(),
        legacy.len()
    );
    let mut current = match current_defaults {
        Value::Array(fields) => fields,
        other => bail!(
            "current {label} defaults encoded as {}, expected an array",
            value_kind(&other)
        ),
    };
    for (legacy_value, current_index) in legacy.into_iter().zip(legacy_to_current) {
        if let Some(current_index) = current_index {
            let field = current.get_mut(*current_index).with_context(|| {
                format!("{label} migration targets missing current field {current_index}")
            })?;
            *field = legacy_value;
        }
    }
    Ok(Value::Array(current))
}

fn merge_named(target: &mut Value, current_defaults: Value, label: &str) -> Result<()> {
    let legacy = take_map(target, &format!("legacy {label}"))?;
    let mut current = match current_defaults {
        Value::Map(fields) => fields,
        other => bail!(
            "current {label} defaults encoded as {}, expected a map",
            value_kind(&other)
        ),
    };
    for (key, value) in legacy {
        if let Some((_, current_value)) =
            current.iter_mut().find(|(candidate, _)| candidate == &key)
        {
            *current_value = value;
        } else {
            current.push((key, value));
        }
    }
    *target = Value::Map(current);
    Ok(())
}

fn positional_value<T: Serialize>(value: &T) -> Result<Value> {
    let bytes = rmp_serde::to_vec(value).context("failed to encode current positional defaults")?;
    decode_value(&bytes)
}

fn named_value<T: Serialize>(value: &T) -> Result<Value> {
    let bytes =
        rmp_serde::to_vec_named(value).context("failed to encode current named defaults")?;
    decode_value(&bytes)
}

fn decode_value(bytes: &[u8]) -> Result<Value> {
    let mut cursor = Cursor::new(bytes);
    let value = rmpv::decode::read_value(&mut cursor).context("invalid MessagePack")?;
    ensure!(
        cursor.position() == bytes.len() as u64,
        "MessagePack contains {} trailing bytes",
        bytes.len() as u64 - cursor.position()
    );
    Ok(value)
}

fn encode_value(value: &Value) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    rmpv::encode::write_value(&mut bytes, value)
        .context("failed to encode migrated MessagePack")?;
    Ok(bytes)
}

fn validate_positional_session_fields(session: &Value) -> Result<()> {
    let fields = session
        .as_array()
        .context("migrated Session is not positional")?;
    ensure!(
        fields.len() == 11,
        "migrated positional Session has {} fields; expected 11",
        fields.len()
    );
    validate_field::<patinae_scene::ObjectRegistrySnapshot>(fields, 0, "registry")?;
    validate_field::<patinae_scene::prelude::Camera>(fields, 1, "camera")?;
    validate_field::<patinae_scene::SelectionManager>(fields, 2, "selections")?;
    validate_field::<patinae_scene::SceneManager>(fields, 3, "scenes")?;
    validate_field::<patinae_scene::ViewManager>(fields, 4, "views")?;
    validate_field::<patinae_scene::Movie>(fields, 5, "movie")?;
    validate_positional_settings_fields(
        fields
            .get(6)
            .context("migrated Session has no settings field")?,
    )?;
    validate_field::<Settings>(fields, 6, "settings")?;
    validate_field::<patinae_scene::NamedPalette>(fields, 7, "named_palette")?;
    validate_field::<patinae_scene::ThemedPalette>(fields, 8, "palette")?;
    validate_field::<[f32; 3]>(fields, 9, "clear_color")?;
    validate_field::<bool>(fields, 10, "clear_color_set")
}

fn validate_positional_settings_fields(settings: &Value) -> Result<()> {
    let fields = settings
        .as_array()
        .context("migrated Settings is not positional")?;
    validate_field::<patinae_settings::groups::ShadingSettings>(fields, 0, "settings.shading")?;
    validate_field::<patinae_settings::groups::UiSettings>(fields, 1, "settings.ui")?;
    validate_field::<patinae_settings::groups::MovieSettings>(fields, 2, "settings.movie")?;
    validate_field::<patinae_settings::groups::RendererSettings>(fields, 3, "settings.renderer")?;
    validate_field::<patinae_settings::groups::BehaviorSettings>(fields, 4, "settings.behavior")?;
    validate_field::<patinae_settings::groups::SsaoSettings>(fields, 5, "settings.ssao")?;
    validate_field::<patinae_settings::groups::FxaaSettings>(fields, 6, "settings.fxaa")?;
    validate_field::<patinae_settings::groups::MeasurementSettings>(
        fields,
        CURRENT_SETTINGS_MEASUREMENT_INDEX,
        "settings.measurement",
    )?;
    validate_field::<patinae_settings::groups::ObjectSettings>(
        fields,
        CURRENT_SETTINGS_OBJECT_INDEX,
        "settings.object",
    )?;
    validate_field::<CartoonSettings>(fields, CURRENT_SETTINGS_CARTOON_INDEX, "settings.cartoon")?;
    validate_field::<patinae_settings::groups::StickSettings>(
        fields,
        CURRENT_SETTINGS_STICK_INDEX,
        "settings.stick",
    )?;
    validate_field::<patinae_settings::groups::SphereSettings>(
        fields,
        CURRENT_SETTINGS_SPHERE_INDEX,
        "settings.sphere",
    )?;
    validate_field::<patinae_settings::groups::SurfaceSettings>(
        fields,
        CURRENT_SETTINGS_SURFACE_INDEX,
        "settings.surface",
    )?;
    validate_field::<patinae_settings::groups::RibbonSettings>(fields, 13, "settings.ribbon")?;
    validate_field::<patinae_settings::groups::LineSettings>(fields, 14, "settings.line")?;
    validate_field::<patinae_settings::groups::DotSettings>(
        fields,
        CURRENT_SETTINGS_DOT_INDEX,
        "settings.dot",
    )?;
    validate_field::<patinae_settings::groups::MeshSettings>(
        fields,
        CURRENT_SETTINGS_MESH_INDEX,
        "settings.mesh",
    )?;
    validate_field::<patinae_settings::groups::EllipsoidSettings>(
        fields,
        CURRENT_SETTINGS_ELLIPSOID_INDEX,
        "settings.ellipsoid",
    )
}

fn validate_field<T: DeserializeOwned>(fields: &[Value], index: usize, label: &str) -> Result<()> {
    let value = fields
        .get(index)
        .with_context(|| format!("migrated Session has no {label} field"))?;
    let bytes = encode_value(value)?;
    rmp_serde::from_slice::<T>(&bytes)
        .map(|_| ())
        .with_context(|| format!("migrated Session field {label} is invalid"))
}

fn array_mut<'a>(value: &'a mut Value, label: &str) -> Result<&'a mut Vec<Value>> {
    match value {
        Value::Array(fields) => Ok(fields),
        other => bail!("{label} is {}, expected an array", value_kind(other)),
    }
}

fn take_array(value: &mut Value, label: &str) -> Result<Vec<Value>> {
    match std::mem::replace(value, Value::Nil) {
        Value::Array(fields) => Ok(fields),
        other => bail!("{label} is {}, expected an array", value_kind(&other)),
    }
}

fn take_map(value: &mut Value, label: &str) -> Result<Vec<(Value, Value)>> {
    match std::mem::replace(value, Value::Nil) {
        Value::Map(fields) => Ok(fields),
        other => bail!("{label} is {}, expected a map", value_kind(&other)),
    }
}

fn map_value<'a>(fields: &'a [(Value, Value)], key: &str) -> Option<&'a Value> {
    fields
        .iter()
        .find(|(candidate, _)| candidate.as_str() == Some(key))
        .map(|(_, value)| value)
}

fn remove_map_value(fields: &mut Vec<(Value, Value)>, key: &str) -> Option<Value> {
    let index = fields
        .iter()
        .position(|(candidate, _)| candidate.as_str() == Some(key))?;
    Some(fields.remove(index).1)
}

fn map_value_mut<'a>(container: &'a mut Value, key: &str) -> Option<&'a mut Value> {
    let Value::Map(fields) = container else {
        return None;
    };
    fields
        .iter_mut()
        .find(|(candidate, _)| candidate.as_str() == Some(key))
        .map(|(_, value)| value)
}

fn optional_map_value_mut<'a>(
    container: &'a mut Value,
    key: &str,
) -> Result<Option<&'a mut Value>> {
    ensure!(
        matches!(container, Value::Map(_)),
        "named structure is {}, expected a map",
        value_kind(container)
    );
    Ok(map_value_mut(container, key))
}

fn value_kind(value: &Value) -> &'static str {
    match value {
        Value::Nil => "nil",
        Value::Boolean(_) => "a boolean",
        Value::Integer(_) => "an integer",
        Value::F32(_) | Value::F64(_) => "a float",
        Value::String(_) => "a string",
        Value::Binary(_) => "binary data",
        Value::Array(_) => "an array",
        Value::Map(_) => "a map",
        Value::Ext(_, _) => "an extension",
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    use flate2::write::GzEncoder;
    use flate2::Compression;

    use super::*;

    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

    const NEW_CARTOON_FIELDS: [&str; 7] = [
        "nucleic_ladder",
        "oval_width",
        "oval_length",
        "rect_width",
        "rect_length",
        "loop_radius",
        "arrow_tip_scale",
    ];

    #[test]
    fn positional_v042_session_upgrades_and_preserves_settings() {
        let paths = TestPaths::new("positional");
        let mut session = Session::new();
        session.settings.cartoon.power = 3.25;
        let mut value = positional_value(&session).unwrap();
        downgrade_positional_session(&mut value).unwrap();
        write_gzip_value(&paths.input, &value).unwrap();

        let report = upgrade_path(&paths.input, &paths.output).unwrap();
        let upgraded = load_prs_document(&paths.output).unwrap();

        assert!(matches!(report.source, SourceFormat::LegacyRawSession));
        assert_eq!(upgraded.prs_format_version, PRS_FORMAT_VERSION);
        assert_eq!(upgraded.session.settings.cartoon.power, 3.25);
        assert!(upgraded.session.settings.cartoon.nucleic_ladder);
        assert_eq!(upgraded.session.settings.cartoon.oval_width, 0.0);
    }

    #[test]
    fn positional_v033_session_upgrades_settings_and_palettes() {
        let paths = TestPaths::new("positional_v033");
        let mut session = Session::new();
        session.settings.cartoon.power = 5.75;
        session.clear_color = [0.2, 0.3, 0.4];
        let expected_carbon = session.palette.element.get(6);
        let mut value = positional_value(&session).unwrap();
        downgrade_v033_positional_session(&mut value).unwrap();
        write_gzip_value(&paths.input, &value).unwrap();

        let report = upgrade_path(&paths.input, &paths.output).unwrap();
        let upgraded = load_prs_document(&paths.output).unwrap();

        assert!(matches!(report.source, SourceFormat::LegacyRawSession));
        assert_eq!(upgraded.session.settings.cartoon.power, 5.75);
        assert_eq!(upgraded.session.clear_color, [0.2, 0.3, 0.4]);
        assert!(upgraded.session.clear_color_set);
        assert_eq!(upgraded.session.palette.element.get(6), expected_carbon);
    }

    #[test]
    fn positional_v033_object_state_restores_draw_mask() {
        let mut current = take_array(
            &mut positional_value(&ObjectState::default()).unwrap(),
            "current ObjectState",
        )
        .unwrap();
        let mut legacy =
            select_legacy_fields(&mut current, &[0, 1, 2, 4], "current ObjectState").unwrap();

        migrate_v033_object_state(&mut legacy).unwrap();
        let upgraded: ObjectState = rmp_serde::from_slice(&encode_value(&legacy).unwrap()).unwrap();

        assert_eq!(upgraded.draw_reps, upgraded.visible_reps);
    }

    #[test]
    fn positional_v033_atom_representation_inserts_transparency_fields() {
        let colors = Value::Array((0_i32..8).map(Value::from).collect());
        let mut representation = Value::Array(vec![
            colors,
            Value::Nil,
            Value::from(128_u32),
            Value::from(2_i64),
            Value::from(""),
            Value::from("label"),
            Value::Boolean(false),
            Value::Nil,
            Value::Boolean(false),
        ]);

        migrate_v033_atom_representation(&mut representation).unwrap();
        let fields = representation.as_array().unwrap();

        assert_eq!(fields.len(), 14);
        assert!(fields[2..7].iter().all(Value::is_nil));
        assert_eq!(fields[7].as_u64(), Some(128));
        assert_eq!(fields[9].as_str(), Some(""));
        assert_eq!(fields[10].as_str(), Some("label"));
        assert_eq!(fields[0].as_array().unwrap().len(), 10);
    }

    #[test]
    fn named_v042_document_upgrades_and_fills_new_fields() {
        let paths = TestPaths::new("named");
        let mut session = Session::new();
        session.settings.cartoon.power = 4.5;
        let mut session_value = named_value(&session).unwrap();
        downgrade_named_session(&mut session_value).unwrap();
        let document = Value::Map(vec![
            (Value::from("prs_format_version"), Value::from(2_u64)),
            (Value::from("producer"), Value::from("patinae")),
            (Value::from("producer_version"), Value::from("0.4.2")),
            (Value::from("session"), session_value),
        ]);
        write_gzip_value(&paths.input, &document).unwrap();

        let report = upgrade_path(&paths.input, &paths.output).unwrap();
        let upgraded = load_prs_document(&paths.output).unwrap();

        assert!(matches!(
            report.source,
            SourceFormat::LegacyDocument {
                format_version: Some(2),
                producer_version: Some(ref version),
            } if version == "0.4.2"
        ));
        assert_eq!(upgraded.session.settings.cartoon.power, 4.5);
        assert!(upgraded.session.settings.cartoon.nucleic_ladder);
        assert_eq!(upgraded.session.settings.object.state, 1);
    }

    #[test]
    fn positional_object_overrides_keep_existing_values() {
        let mut overrides = ObjectOverrides::default();
        overrides.cartoon.power = Some(6.0);
        overrides.stick.radius = Some(0.75);
        let mut value = positional_value(&overrides).unwrap();
        downgrade_positional_object_overrides(&mut value).unwrap();

        migrate_positional_object_overrides(&mut value).unwrap();
        let bytes = encode_value(&value).unwrap();
        let upgraded: ObjectOverrides = rmp_serde::from_slice(&bytes).unwrap();

        assert_eq!(upgraded.cartoon.power, Some(6.0));
        assert_eq!(upgraded.stick.radius, Some(0.75));
        assert_eq!(upgraded.object.state, None);
        assert_eq!(upgraded.cartoon.nucleic_ladder, None);
        assert_eq!(upgraded.cartoon.oval_width, None);
    }

    #[test]
    fn existing_output_is_never_overwritten() {
        let paths = TestPaths::new("existing_output");
        let mut value = positional_value(&Session::new()).unwrap();
        downgrade_positional_session(&mut value).unwrap();
        write_gzip_value(&paths.input, &value).unwrap();
        fs::write(&paths.output, b"keep me").unwrap();

        let error = upgrade_path(&paths.input, &paths.output).unwrap_err();

        assert!(error.to_string().contains("output already exists"));
        assert_eq!(fs::read(&paths.output).unwrap(), b"keep me");
    }

    #[test]
    fn current_document_is_rejected_without_creating_output() {
        let paths = TestPaths::new("current");
        save_prs(&Session::new(), &paths.input).unwrap();

        let error = upgrade_path(&paths.input, &paths.output).unwrap_err();

        assert!(error.to_string().contains("already uses a PRS document"));
        assert!(!paths.output.exists());
    }

    #[test]
    fn current_raw_session_upgrades_through_raw_session_path() {
        let paths = TestPaths::new("current_raw");
        let session = Session::new();
        write_gzip_value(&paths.input, &positional_value(&session).unwrap()).unwrap();

        let report = upgrade_path(&paths.input, &paths.output).unwrap();
        let upgraded = load_prs_document(&paths.output).unwrap();

        assert!(matches!(report.source, SourceFormat::CurrentRawSession));
        assert_eq!(upgraded.prs_format_version, PRS_FORMAT_VERSION);
        assert!(paths.output.exists());
    }

    #[test]
    fn malformed_v3_document_is_rejected_without_legacy_fallback() {
        let paths = TestPaths::new("malformed_v3");
        let mut session = positional_value(&Session::new()).unwrap();
        let registry = array_mut(&mut session, "Session")
            .unwrap()
            .get_mut(SESSION_REGISTRY_INDEX)
            .unwrap();
        assert_eq!(array_mut(registry, "registry").unwrap().len(), 11);
        array_mut(registry, "registry").unwrap().pop();
        let document = Value::Array(vec![
            Value::from(3_u64),
            Value::from("patinae"),
            Value::from(PRS_PRODUCER_VERSION),
            session,
        ]);
        write_gzip_value(&paths.input, &document).unwrap();

        let error = upgrade_path(&paths.input, &paths.output).unwrap_err();

        assert!(error
            .to_string()
            .contains("refusing to reinterpret PRS v3 envelope"));
        assert!(!paths.output.exists());
    }

    #[test]
    fn future_document_is_rejected_without_legacy_fallback() {
        let paths = TestPaths::new("future_v4");
        let document = Value::Map(vec![
            (Value::from("prs_format_version"), Value::from(4_u64)),
            (Value::from("producer"), Value::from("patinae")),
            (
                Value::from("producer_version"),
                Value::from(PRS_PRODUCER_VERSION),
            ),
            (
                Value::from("session"),
                named_value(&Session::new()).unwrap(),
            ),
        ]);
        write_gzip_value(&paths.input, &document).unwrap();

        let error = upgrade_path(&paths.input, &paths.output).unwrap_err();

        assert!(error
            .to_string()
            .contains("refusing to reinterpret PRS v4 envelope"));
        assert!(!paths.output.exists());
    }

    #[test]
    fn registry_migration_accepts_current_raw_arity_and_rejects_transition_arity() {
        let session = Session::new();
        let current_session = positional_value(&session).unwrap();
        let current_registry = current_session
            .as_array()
            .unwrap()
            .get(SESSION_REGISTRY_INDEX)
            .unwrap()
            .clone();
        assert_eq!(current_registry.as_array().unwrap().len(), 11);

        let mut transition_registry = current_registry.clone();
        array_mut(&mut transition_registry, "transition registry")
            .unwrap()
            .pop();

        let error = migrate_positional_registry(&mut transition_registry).unwrap_err();
        assert!(error
            .to_string()
            .contains("unsupported ObjectRegistrySnapshot field count 10"));
        let mut current_registry = current_registry;
        migrate_positional_registry(&mut current_registry).unwrap();
    }

    #[test]
    fn registry_migration_rejects_unknown_v3_arity() {
        let session = Session::new();
        let mut session_value = positional_value(&session).unwrap();
        let registry = array_mut(&mut session_value, "Session").unwrap()[SESSION_REGISTRY_INDEX]
            .as_array()
            .unwrap()
            .clone();
        let mut unsupported = Value::Array(registry);
        array_mut(&mut unsupported, "registry")
            .unwrap()
            .push(Value::Nil);

        let error = migrate_positional_registry(&mut unsupported).unwrap_err();

        assert!(error
            .to_string()
            .contains("unsupported ObjectRegistrySnapshot field count 12"));
    }

    fn downgrade_positional_session(session: &mut Value) -> Result<()> {
        let fields = array_mut(session, "current Session")?;
        remove_current_recent_atoms(fields)?;
        downgrade_positional_settings(
            fields
                .get_mut(SESSION_SETTINGS_INDEX)
                .context("current Session has no settings field")?,
        )
    }

    fn downgrade_v033_positional_session(session: &mut Value) -> Result<()> {
        let fields = array_mut(session, "current Session")?;
        remove_current_recent_atoms(fields)?;
        downgrade_v033_positional_registry(
            fields
                .get_mut(SESSION_REGISTRY_INDEX)
                .context("current Session has no registry field")?,
        )?;
        downgrade_v033_positional_settings(
            fields
                .get_mut(SESSION_SETTINGS_INDEX)
                .context("current Session has no settings field")?,
        )?;

        let clear_color = std::mem::replace(
            fields
                .get_mut(SESSION_CLEAR_COLOR_INDEX)
                .context("current Session has no clear color field")?,
            Value::Nil,
        );
        let mut palette = take_array(
            fields
                .get_mut(SESSION_PALETTE_INDEX)
                .context("current Session has no themed palette field")?,
            "current ThemedPalette",
        )?;
        let element_colors = std::mem::replace(
            palette
                .get_mut(0)
                .context("current ThemedPalette has no element palette")?,
            Value::Nil,
        );
        fields[V033_SESSION_ELEMENT_COLORS_INDEX] = element_colors;
        fields[SESSION_CLEAR_COLOR_INDEX] = Value::Nil;
        fields[V033_SESSION_CLEAR_COLOR_INDEX] = clear_color;
        Ok(())
    }

    fn remove_current_recent_atoms(fields: &mut Vec<Value>) -> Result<()> {
        ensure!(
            fields.len() == 12,
            "current Session has {} fields; expected 12",
            fields.len()
        );
        fields.pop();
        Ok(())
    }

    fn downgrade_v033_positional_registry(registry: &mut Value) -> Result<()> {
        let mut current = take_array(registry, "current ObjectRegistrySnapshot")?;
        *registry = select_legacy_fields(
            &mut current,
            &V033_REGISTRY_TO_CURRENT,
            "current ObjectRegistrySnapshot",
        )?;
        Ok(())
    }

    fn downgrade_v033_positional_settings(settings: &mut Value) -> Result<()> {
        const CARTOON_FIELDS: [usize; 15] = [0, 1, 2, 3, 4, 5, 6, 7, 9, 10, 11, 12, 13, 14, 15];

        let mut current = take_array(settings, "current Settings")?;

        let mut ui = select_legacy_values(
            array_mut(
                current
                    .get_mut(CURRENT_SETTINGS_UI_INDEX)
                    .context("current Settings has no UI group")?,
                "current UI settings",
            )?,
            &[1, 2, 3, 4, 5, 6, 7],
            "current UI settings",
        )?;
        ui.extend([
            Value::from(0_i64),
            Value::from(0x00004D_i64),
            Value::from(0x333380_i64),
        ]);
        current[CURRENT_SETTINGS_UI_INDEX] = Value::Array(ui);

        let cartoon = select_legacy_values(
            array_mut(
                current
                    .get_mut(CURRENT_SETTINGS_CARTOON_INDEX)
                    .context("current Settings has no cartoon group")?,
                "current cartoon settings",
            )?,
            &CARTOON_FIELDS,
            "current cartoon settings",
        )?;
        current[CURRENT_SETTINGS_CARTOON_INDEX] = Value::Array(cartoon);

        let mut stick = select_legacy_values(
            array_mut(
                current
                    .get_mut(CURRENT_SETTINGS_STICK_INDEX)
                    .context("current Settings has no stick group")?,
                "current stick settings",
            )?,
            &[0, 1, 2, 3],
            "current stick settings",
        )?;
        stick.push(Value::F32(0.06));
        current[CURRENT_SETTINGS_STICK_INDEX] = Value::Array(stick);

        let sphere = array_mut(
            current
                .get_mut(CURRENT_SETTINGS_SPHERE_INDEX)
                .context("current Settings has no sphere group")?,
            "current sphere settings",
        )?;
        let mut sphere_legacy = select_legacy_values(sphere, &[0, 1], "current sphere settings")?;
        sphere_legacy.insert(1, Value::from(1_i64));
        current[CURRENT_SETTINGS_SPHERE_INDEX] = Value::Array(sphere_legacy);

        let surface = select_legacy_values(
            array_mut(
                current
                    .get_mut(CURRENT_SETTINGS_SURFACE_INDEX)
                    .context("current Settings has no surface group")?,
                "current surface settings",
            )?,
            &[0, 9, 2, 3, 6, 7, 8],
            "current surface settings",
        )?;
        current[CURRENT_SETTINGS_SURFACE_INDEX] = Value::Array(surface);

        let mut dot = select_legacy_values(
            array_mut(
                current
                    .get_mut(CURRENT_SETTINGS_DOT_INDEX)
                    .context("current Settings has no dot group")?,
                "current dot settings",
            )?,
            &[1, 2],
            "current dot settings",
        )?;
        dot.push(Value::Boolean(true));
        current[CURRENT_SETTINGS_DOT_INDEX] = Value::Array(dot);

        let mut mesh = select_legacy_values(
            array_mut(
                current
                    .get_mut(CURRENT_SETTINGS_MESH_INDEX)
                    .context("current Settings has no mesh group")?,
                "current mesh settings",
            )?,
            &[0],
            "current mesh settings",
        )?;
        mesh.push(Value::Boolean(true));
        current[CURRENT_SETTINGS_MESH_INDEX] = Value::Array(mesh);

        *settings =
            select_legacy_fields(&mut current, &V033_SETTINGS_TO_CURRENT, "current Settings")?;
        Ok(())
    }

    fn downgrade_positional_settings(settings: &mut Value) -> Result<()> {
        let mut current = take_array(settings, "current Settings")?;
        downgrade_positional_cartoon(
            current
                .get_mut(LEGACY_SETTINGS_TO_CURRENT[LEGACY_SETTINGS_CARTOON_INDEX])
                .context("current Settings has no cartoon group")?,
        )?;
        *settings = select_legacy_fields(
            &mut current,
            &LEGACY_SETTINGS_TO_CURRENT,
            "current Settings",
        )?;
        Ok(())
    }

    fn downgrade_positional_object_overrides(overrides: &mut Value) -> Result<()> {
        let mut current = take_array(overrides, "current ObjectOverrides")?;
        downgrade_positional_cartoon(
            current
                .get_mut(LEGACY_OVERRIDES_TO_CURRENT[LEGACY_OVERRIDES_CARTOON_INDEX])
                .context("current ObjectOverrides has no cartoon group")?,
        )?;
        *overrides = select_legacy_fields(
            &mut current,
            &LEGACY_OVERRIDES_TO_CURRENT,
            "current ObjectOverrides",
        )?;
        Ok(())
    }

    fn downgrade_positional_cartoon(cartoon: &mut Value) -> Result<()> {
        let mut current = take_array(cartoon, "current cartoon settings")?;
        *cartoon = select_legacy_fields(
            &mut current,
            &LEGACY_CARTOON_TO_CURRENT,
            "current cartoon settings",
        )?;
        Ok(())
    }

    fn select_legacy_fields(
        current: &mut [Value],
        legacy_to_current: &[usize],
        label: &str,
    ) -> Result<Value> {
        Ok(Value::Array(select_legacy_values(
            current,
            legacy_to_current,
            label,
        )?))
    }

    fn select_legacy_values(
        current: &mut [Value],
        legacy_to_current: &[usize],
        label: &str,
    ) -> Result<Vec<Value>> {
        let mut legacy = Vec::with_capacity(legacy_to_current.len());
        for current_index in legacy_to_current {
            let field = current.get_mut(*current_index).with_context(|| {
                format!("{label} has no field at current index {current_index}")
            })?;
            legacy.push(std::mem::replace(field, Value::Nil));
        }
        Ok(legacy)
    }

    fn downgrade_named_session(session: &mut Value) -> Result<()> {
        let settings =
            map_value_mut(session, "settings").context("current Session has no settings field")?;
        let cartoon =
            map_value_mut(settings, "cartoon").context("current Settings has no cartoon group")?;
        remove_named_fields(cartoon, &NEW_CARTOON_FIELDS)?;
        remove_named_fields(settings, &["renderer", "object"])
    }

    fn remove_named_fields(value: &mut Value, names: &[&str]) -> Result<()> {
        let fields = match value {
            Value::Map(fields) => fields,
            other => bail!(
                "current named structure is {}, expected a map",
                value_kind(other)
            ),
        };
        fields.retain(|(key, _)| {
            key.as_str()
                .is_none_or(|candidate| !names.contains(&candidate))
        });
        Ok(())
    }

    fn write_gzip_value(path: &Path, value: &Value) -> Result<()> {
        let payload = encode_value(value)?;
        let file = File::create(path)
            .with_context(|| format!("failed to create test PRS {}", path.display()))?;
        let mut encoder = GzEncoder::new(file, Compression::default());
        encoder.write_all(&payload)?;
        encoder.finish()?;
        Ok(())
    }

    struct TestPaths {
        dir: PathBuf,
        input: PathBuf,
        output: PathBuf,
    }

    impl TestPaths {
        fn new(name: &str) -> Self {
            let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
            let dir = env::temp_dir().join(format!(
                "patinae_prs_upgrade_{name}_{}_{}",
                std::process::id(),
                id
            ));
            fs::create_dir_all(&dir).unwrap();
            Self {
                input: dir.join("legacy.prs"),
                output: dir.join("upgraded.prs"),
                dir,
            }
        }
    }

    impl Drop for TestPaths {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }
}
