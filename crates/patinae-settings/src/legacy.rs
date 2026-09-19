//! Legacy bridge for PSE session import/export.
//!
//! Maps between the PSE session format's flat `(id, SettingValue)` representation
//! and the typed `Settings` / `ObjectOverrides` structs. The mapping from legacy
//! numeric IDs to setting names lives exclusively here — the rest of the codebase
//! uses string-based lookup via the registry.

use std::sync::LazyLock;

use ahash::AHashMap;

use crate::groups::Settings;
use crate::overrides::ObjectOverrides;
use crate::registry;
use crate::store::SerializedSetting;

/// PSE legacy ID ↔ setting name mapping.
///
/// Only legacy settings with stable IDs are included. Workspace-specific
/// settings such as shading and postprocess controls have no PSE equivalent
/// and are excluded from import/export.
static LEGACY_MAP: &[(&str, u16)] = &[
    // Behavior
    ("bonding_vdw_cutoff", 0),
    // Dot
    ("dot_density", 2),
    // Shading — Classic
    ("ambient", 7),
    ("direct", 8),
    ("reflect", 9),
    ("light", 10),
    // UI
    ("antialias", 12),
    // Ribbon
    ("ribbon_power", 17),
    ("ribbon_power_b", 18),
    ("ribbon_sampling", 19),
    ("ribbon_radius", 20),
    // Stick
    ("stick_radius", 21),
    // UI
    ("orthoscopic", 23),
    // Surface
    ("surface_quality", 38),
    // Stick
    ("valence", 64),
    // Dot
    ("dot_width", 77),
    // UI
    ("selection_width", 80),
    // Shading — Common
    ("depth_cue", 84),
    // Shading — Classic
    ("specular", 85),
    ("shininess", 86),
    // Shading — Common
    ("fog", 88),
    // Cartoon
    ("cartoon_sampling", 91),
    ("cartoon_power", 94),
    ("cartoon_power_b", 95),
    ("cartoon_round_helices", 111),
    ("cartoon_refine_normals", 112),
    ("cartoon_smooth_loops", 114),
    // Ribbon
    ("ribbon_throw", 121),
    // Cartoon
    ("cartoon_throw", 122),
    ("cartoon_refine", 123),
    // Surface
    ("surface_transparency", 138),
    ("surface_color", 144),
    // Mesh
    ("mesh_color", 146),
    // Sphere
    ("sphere_scale", 155),
    // Sphere
    ("sphere_color", 173),
    // Shading — Common
    ("fog_start", 192),
    // Surface
    ("transparency_mode", 213),
    // Ribbon
    ("ribbon_color", 235),
    // Cartoon
    ("cartoon_color", 236),
    // Cartoon
    ("cartoon_smooth_first", 257),
    ("cartoon_smooth_last", 258),
    ("cartoon_smooth_cycles", 259),
    ("cartoon_flat_cycles", 260),
    // Movie
    ("movie_loop", 299),
    // Behavior
    ("auto_dss", 323),
    // UI
    ("mouse_selection_mode", 354),
    // Stick
    ("stick_color", 376),
    // Behavior
    ("ignore_case", 414),
    // UI
    ("opaque_background", 435),
    // Cartoon
    ("cartoon_nucleic_acid_color", 451),
    // Shading — Classic
    ("spec_direct", 454),
    ("light_count", 455),
    ("light2", 456),
    ("light3", 457),
    ("light4", 463),
    ("light5", 464),
    ("light6", 465),
    ("light7", 466),
    // Shading — Classic
    ("spec_direct_power", 488),
    ("light8", 489),
    ("light9", 490),
    ("spec_count", 492),
    // Stick
    ("stick_valence_scale", 512),
    // UI
    ("mouse_wheel_scale", 523),
    // Line
    ("line_color", 526),
    // Movie
    ("movie_fps", 550),
    // Movie
    ("movie_auto_interpolate", 621),
    // Cartoon
    ("cartoon_gap_cutoff", 750),
    // Behavior
    ("ignore_case_chain", 751),
];

/// ID → setting name.
static ID_TO_NAME: LazyLock<AHashMap<u16, &'static str>> =
    LazyLock::new(|| LEGACY_MAP.iter().map(|&(name, id)| (id, name)).collect());

/// Setting name → PSE legacy ID.
static NAME_TO_ID: LazyLock<AHashMap<&'static str, u16>> =
    LazyLock::new(|| LEGACY_MAP.iter().map(|&(name, id)| (name, id)).collect());

/// Look up a setting name by PSE legacy ID.
pub fn name_for_id(id: u16) -> Option<&'static str> {
    ID_TO_NAME.get(&id).copied()
}

/// Look up a PSE legacy ID by setting name.
pub fn id_for_name(name: &str) -> Option<u16> {
    NAME_TO_ID.get(name).copied()
}

/// Import settings from a PSE session list into typed `Settings`.
///
/// Unknown or patinae-specific IDs (those not in the legacy map) are silently
/// skipped. The `Settings` struct is initialized to defaults first.
pub fn import_session(list: &[SerializedSetting]) -> Settings {
    let mut settings = Settings::default();
    apply_session_list(&mut settings, list);
    settings
}

/// Apply a PSE session list on top of existing settings (additive).
pub fn apply_session_list(settings: &mut Settings, list: &[SerializedSetting]) {
    for entry in list {
        if let Some(name) = name_for_id(entry.id) {
            if let Some(desc) = registry::lookup_by_name(name) {
                // Best-effort: skip type mismatches rather than failing the whole import
                let _ = desc.set(settings, entry.value.clone());
            }
        }
    }
}

/// Export typed `Settings` to a PSE session list.
///
/// Only includes settings that have PSE legacy IDs (present in LEGACY_MAP).
/// Workspace-specific settings are excluded because they have no legacy
/// session-format equivalent.
pub fn export_session(settings: &Settings) -> Vec<SerializedSetting> {
    LEGACY_MAP
        .iter()
        .filter_map(|&(name, id)| {
            let desc = registry::lookup_by_name(name)?;
            Some(SerializedSetting {
                id,
                value: desc.get(settings),
            })
        })
        .collect()
}

/// Import per-object overrides from a PSE session list.
///
/// Only object-overridable settings (those with `set_override` in the registry)
/// are applied. Global-only settings in the list are silently skipped.
pub fn import_object_overrides(list: &[SerializedSetting]) -> ObjectOverrides {
    let mut overrides = ObjectOverrides::default();
    for entry in list {
        if let Some(name) = name_for_id(entry.id) {
            if let Some(desc) = registry::lookup_by_name(name) {
                if desc.is_object_overridable() {
                    let _ = desc.set_override(&mut overrides, entry.value.clone());
                }
            }
        }
    }
    overrides
}

/// Export per-object overrides to a PSE session list.
///
/// Only includes overrides that are actually set (Non-None) and have PSE legacy IDs.
pub fn export_object_overrides(overrides: &ObjectOverrides) -> Vec<SerializedSetting> {
    LEGACY_MAP
        .iter()
        .filter_map(|&(name, id)| {
            let desc = registry::lookup_by_name(name)?;
            let value = desc.get_override(overrides)?;
            Some(SerializedSetting { id, value })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::setting::SettingValue;

    #[test]
    fn test_import_export_roundtrip() {
        let mut settings = Settings::default();
        settings.shading.classic.ambient = 0.3;
        settings.stick.radius = 0.5;

        let list = export_session(&settings);
        let restored = import_session(&list);

        assert_eq!(restored.shading.classic.ambient, 0.3);
        assert_eq!(restored.stick.radius, 0.5);
    }

    #[test]
    fn test_export_excludes_patinae_settings() {
        let settings = Settings::default();
        let list = export_session(&settings);

        // patinae-specific settings should not appear in PSE export
        assert!(!list.iter().any(|s| s.id >= 798));
    }

    #[test]
    fn test_import_unknown_ids_skipped() {
        let list = vec![
            SerializedSetting {
                id: 7,
                value: SettingValue::Float(0.5),
            }, // ambient
            SerializedSetting {
                id: 9999,
                value: SettingValue::Int(42),
            }, // unknown
        ];
        let settings = import_session(&list);
        assert_eq!(settings.shading.classic.ambient, 0.5);
    }

    #[test]
    fn test_object_override_roundtrip() {
        let mut overrides = ObjectOverrides::default();
        overrides.stick.radius = Some(0.8);
        overrides.cartoon.color = Some(crate::Color(5));

        let list = export_object_overrides(&overrides);
        let restored = import_object_overrides(&list);

        assert_eq!(restored.stick.radius, Some(0.8));
        assert_eq!(restored.cartoon.color, Some(crate::Color(5)));
    }

    #[test]
    fn object_import_retains_object_values_and_skips_global_values() {
        let stick_radius_id = id_for_name("stick_radius").unwrap();
        let overrides = import_object_overrides(&[
            SerializedSetting {
                id: 7,
                value: SettingValue::Float(0.5),
            },
            SerializedSetting {
                id: stick_radius_id,
                value: SettingValue::Float(0.8),
            },
        ]);
        assert_eq!(overrides.stick.radius, Some(0.8));
        let exported = export_object_overrides(&overrides);
        assert_eq!(exported.len(), 1);
        assert_eq!(exported[0].id, stick_radius_id);
        assert_eq!(exported[0].value, SettingValue::Float(0.8));
    }

    #[test]
    fn test_legacy_map_consistency() {
        // Every name in LEGACY_MAP should exist in the registry
        for &(name, _id) in LEGACY_MAP {
            assert!(
                registry::lookup_by_name(name).is_some(),
                "LEGACY_MAP entry '{name}' not found in registry"
            );
        }
    }

    #[test]
    fn test_no_duplicate_ids_in_legacy_map() {
        let mut seen = std::collections::HashSet::new();
        for &(_name, id) in LEGACY_MAP {
            assert!(seen.insert(id), "duplicate legacy ID: {id}");
        }
    }

    #[test]
    fn test_name_id_roundtrip() {
        for &(name, id) in LEGACY_MAP {
            assert_eq!(name_for_id(id), Some(name));
            assert_eq!(id_for_name(name), Some(id));
        }
    }
}
