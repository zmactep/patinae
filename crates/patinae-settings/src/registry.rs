//! Setting descriptor registry — O(1) lookup by name.
//!
//! The registry is an index over descriptors generated from the typed settings
//! groups. It must not contain a per-setting list: adding a field to a
//! `groups/*.rs` declaration is enough to expose it through `set`/`get`,
//! completion, and legacy session import/export.

use std::sync::LazyLock;

use ahash::AHashMap;

use crate::macros::SettingDescriptor;

/// All setting descriptors, built once at startup from the root manifest.
static DESCRIPTORS: LazyLock<Vec<SettingDescriptor>> =
    LazyLock::new(crate::groups::build_setting_descriptors);

/// Name -> index into DESCRIPTORS.
static NAME_INDEX: LazyLock<AHashMap<&'static str, usize>> = LazyLock::new(|| {
    DESCRIPTORS
        .iter()
        .enumerate()
        .map(|(i, d)| (d.name, i))
        .collect()
});

/// Look up a descriptor by string name. O(1).
pub fn lookup_by_name(name: &str) -> Option<&'static SettingDescriptor> {
    NAME_INDEX.get(name).map(|&i| &DESCRIPTORS[i])
}

/// Iterator over all descriptors.
pub fn all_descriptors() -> &'static [SettingDescriptor] {
    &DESCRIPTORS
}

/// Total number of registered settings.
pub fn descriptor_count() -> usize {
    DESCRIPTORS.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::groups::Settings;
    use crate::overrides::ObjectOverrides;
    use crate::setting::{SettingType, SettingValue};

    #[test]
    fn test_registry_matches_manifest_builder() {
        let expected = crate::groups::build_setting_descriptors();
        let actual = all_descriptors();

        assert_eq!(actual.len(), expected.len());
        for (actual, expected) in actual.iter().zip(expected.iter()) {
            assert_eq!(actual.name, expected.name);
            assert_eq!(actual.setting_type, expected.setting_type);
            assert_eq!(actual.min, expected.min);
            assert_eq!(actual.max, expected.max);
            assert_eq!(actual.value_hints, expected.value_hints);
            assert_eq!(actual.side_effects, expected.side_effects);
            assert_eq!(
                actual.is_object_overridable(),
                expected.is_object_overridable()
            );
        }
    }

    #[test]
    fn test_lookup_by_name() {
        let d = lookup_by_name("ambient").expect("ambient not found");
        assert_eq!(d.setting_type, SettingType::Float);
    }

    #[test]
    fn test_get_set_roundtrip() {
        let mut settings = Settings::default();
        let d = lookup_by_name("ambient").unwrap();

        let val = d.get(&settings);
        assert_eq!(val, SettingValue::Float(0.14));

        d.set(&mut settings, SettingValue::Float(0.3)).unwrap();
        let val = d.get(&settings);
        assert_eq!(val, SettingValue::Float(0.3));
    }

    #[test]
    fn test_override_get_set() {
        let mut overrides = ObjectOverrides::default();
        let d = lookup_by_name("stick_radius").unwrap();

        assert!(d.is_object_overridable());
        assert!(d.get_override(&overrides).is_none());

        assert!(d
            .set_override(&mut overrides, SettingValue::Float(0.5))
            .unwrap());
        assert_eq!(d.get_override(&overrides), Some(SettingValue::Float(0.5)));

        assert!(d.unset_override(&mut overrides));
        assert!(d.get_override(&overrides).is_none());
    }

    #[test]
    fn test_global_only_has_no_override() {
        let mut overrides = ObjectOverrides::default();
        let d = lookup_by_name("ambient").unwrap();

        assert!(!d.is_object_overridable());
        assert!(d.get_override(&overrides).is_none());
        assert!(!d
            .set_override(&mut overrides, SettingValue::Float(0.5))
            .unwrap());
        assert!(!d.unset_override(&mut overrides));
    }

    #[test]
    fn test_enum_hints() {
        let d = lookup_by_name("shading_mode").unwrap();
        assert_eq!(d.value_hints.len(), 3);
        assert_eq!(d.value_hints[0].0, "classic");

        let d = lookup_by_name("mouse_selection_mode").unwrap();
        assert_eq!(d.value_hints.len(), 7);
    }

    #[test]
    fn test_enum_get_set() {
        let mut settings = Settings::default();
        let d = lookup_by_name("shading_mode").unwrap();

        let val = d.get(&settings);
        assert_eq!(val, SettingValue::Int(0)); // Classic

        d.set(&mut settings, SettingValue::Int(1)).unwrap(); // Skripkin
        assert_eq!(settings.shading.mode, crate::ShadingMode::Skripkin);
    }

    #[test]
    fn test_state_is_regular_object_setting() {
        let mut settings = Settings::default();
        let d = lookup_by_name("state").expect("state not found");

        assert!(d.is_object_overridable());
        assert_eq!(d.setting_type, SettingType::Int);

        d.set(&mut settings, SettingValue::Int(2)).unwrap();
        assert_eq!(d.get(&settings), SettingValue::Int(2));
    }

    #[test]
    fn test_render_memory_settings_are_registered() {
        let mut settings = Settings::default();
        let profile = lookup_by_name("render_memory_profile").expect("profile not found");
        let budget = lookup_by_name("render_memory_budget").expect("budget not found");

        assert_eq!(profile.setting_type, SettingType::Int);
        assert_eq!(budget.setting_type, SettingType::Int);
        assert!(!profile.is_object_overridable());
        assert!(!budget.is_object_overridable());

        profile.set(&mut settings, SettingValue::Int(3)).unwrap();
        budget.set(&mut settings, SettingValue::Int(1024)).unwrap();

        assert_eq!(profile.get(&settings), SettingValue::Int(3));
        assert_eq!(budget.get(&settings), SettingValue::Int(1024));
    }

    #[test]
    fn test_no_duplicate_names() {
        let names: Vec<&str> = DESCRIPTORS.iter().map(|d| d.name).collect();
        let mut seen = std::collections::HashSet::new();
        for name in &names {
            assert!(seen.insert(name), "duplicate descriptor name: {name}");
        }
    }

    #[test]
    fn test_light_float3() {
        let mut settings = Settings::default();
        let d = lookup_by_name("light").unwrap();
        assert_eq!(d.setting_type, SettingType::Float3);

        let val = d.get(&settings);
        assert_eq!(val, SettingValue::Float3([-0.4, -0.4, -1.0]));

        d.set(&mut settings, SettingValue::Float3([1.0, 0.0, 0.0]))
            .unwrap();
        assert_eq!(settings.shading.classic.light, [1.0, 0.0, 0.0]);
    }

    #[test]
    fn test_originally_missing_group_settings_are_registered() {
        let mut settings = Settings::default();
        let setting_names = crate::setting_names();

        for (name, value) in [
            ("cartoon_smooth_loops", SettingValue::Bool(true)),
            ("fxaa_enabled", SettingValue::Bool(false)),
            ("ssao_enabled", SettingValue::Bool(true)),
            ("ssao_radius", SettingValue::Float(1.5)),
            ("ssao_intensity", SettingValue::Float(2.0)),
            ("ssao_bias", SettingValue::Float(0.05)),
            ("surface_solvent", SettingValue::Bool(true)),
            ("mesh_quality", SettingValue::Int(2)),
            ("mesh_solvent", SettingValue::Bool(true)),
            ("mesh_solvent_radius", SettingValue::Float(2.4)),
            ("mesh_transparency", SettingValue::Float(0.35)),
            ("bonding_vdw_cutoff", SettingValue::Float(0.6)),
            ("antialias", SettingValue::Int(0)),
            ("opaque_background", SettingValue::Bool(true)),
            ("orthoscopic", SettingValue::Bool(true)),
            (
                "silhouette_color",
                SettingValue::Color(crate::ColorValue::palette(7)),
            ),
            (
                "cartoon_nucleic_acid_color",
                SettingValue::Color(crate::ColorValue::palette(8)),
            ),
            ("ribbon_sampling", SettingValue::Int(3)),
            ("ribbon_power", SettingValue::Float(3.0)),
            ("ribbon_power_b", SettingValue::Float(0.9)),
            ("ribbon_throw", SettingValue::Float(2.0)),
            ("surface_mode", SettingValue::Int(4)),
            ("surface_individual_chains", SettingValue::Bool(true)),
            ("dot_density", SettingValue::Int(4)),
            ("dot_width", SettingValue::Float(5.0)),
        ] {
            let d = lookup_by_name(name).unwrap_or_else(|| panic!("{name} missing from registry"));
            d.set(&mut settings, value.clone()).unwrap();
            assert_eq!(d.get(&settings), value);
            assert!(
                setting_names.contains(&name),
                "{name} missing from setting_names/completion source"
            );
        }

        for removed in [
            "sphere_quality",
            "valence_size",
            "mesh_lighting",
            "ellipsoid_quality",
            "dot_lighting",
        ] {
            assert!(
                lookup_by_name(removed).is_none(),
                "{removed} should not be an active runtime setting"
            );
            assert!(
                !setting_names.contains(&removed),
                "{removed} should not appear in completion source"
            );
        }
    }
}
