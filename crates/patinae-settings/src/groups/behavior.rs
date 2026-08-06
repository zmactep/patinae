//! Behavioral / algorithmic settings (global-only).

use crate::define_settings_group;

/// Normalized capacity for the recent atom collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecentPickLimit {
    /// Keep every collected atom.
    Unlimited,
    /// Keep at most the newest number of atoms.
    Bounded(usize),
}

define_settings_group! {
    /// Behavioral and algorithmic defaults.
    #[serde(default)]
    group_global BehaviorSettings {
        ignore_case: bool = true,
            name = "ignore_case";
        ignore_case_chain: bool = false,
            name = "ignore_case_chain";
        auto_dss: bool = true,
            name = "auto_dss";
        dss_algorithm: crate::DssAlgorithm = crate::DssAlgorithm::PyMol,
            name = "dss_algorithm",
            hints = crate::DssAlgorithm;
        bonding_vdw_cutoff: f32 = 0.45,
            name = "bonding_vdw_cutoff",
            min = 0.0, max = 1.0;
        /// Maximum recent atom count; `-1` means unlimited.
        max_recent_picks: i32 = -1,
            name = "max_recent_picks",
            min = -1, max = i32::MAX;
    }
}

impl BehaviorSettings {
    /// Returns the normalized recent atom capacity.
    pub fn recent_pick_limit(&self) -> RecentPickLimit {
        if self.max_recent_picks < 0 {
            RecentPickLimit::Unlimited
        } else {
            RecentPickLimit::Bounded(self.max_recent_picks as usize)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::lookup_by_name;
    use crate::{SettingValue, Settings};

    #[test]
    fn recent_pick_limit_defaults_and_descriptor_contract() {
        let behavior = BehaviorSettings::default();
        assert_eq!(behavior.max_recent_picks, -1);
        assert_eq!(behavior.recent_pick_limit(), RecentPickLimit::Unlimited);

        let descriptor = lookup_by_name("max_recent_picks").unwrap();
        assert_eq!(descriptor.get(&Settings::default()), SettingValue::Int(-1));
        assert_eq!(descriptor.min, Some(-1.0));

        let mut settings = Settings::default();
        assert!(descriptor
            .set(&mut settings, SettingValue::Int(-2))
            .is_err());
        descriptor.set(&mut settings, SettingValue::Int(3)).unwrap();
        assert_eq!(
            settings.behavior.recent_pick_limit(),
            RecentPickLimit::Bounded(3)
        );
    }

    #[test]
    fn settings_roundtrip_preserves_recent_pick_limit() {
        let mut settings = Settings::default();
        settings.behavior.max_recent_picks = 7;

        let json = serde_json::to_string(&settings).unwrap();
        let restored: Settings = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.behavior.max_recent_picks, 7);
    }

    #[test]
    fn legacy_named_and_positional_behavior_default_recent_pick_limit() {
        #[derive(serde::Serialize)]
        struct LegacyBehavior {
            ignore_case: bool,
            ignore_case_chain: bool,
            auto_dss: bool,
            dss_algorithm: crate::DssAlgorithm,
            bonding_vdw_cutoff: f32,
        }

        let legacy = LegacyBehavior {
            ignore_case: false,
            ignore_case_chain: true,
            auto_dss: false,
            dss_algorithm: crate::DssAlgorithm::PyMol,
            bonding_vdw_cutoff: 0.7,
        };
        let named = serde_json::to_value(&legacy).unwrap();
        let named: BehaviorSettings = serde_json::from_value(named).unwrap();
        assert_eq!(named.max_recent_picks, -1);

        let positional = serde_json::json!([false, true, false, "PyMol", 0.7]);
        let positional: BehaviorSettings = serde_json::from_value(positional).unwrap();
        assert_eq!(positional.max_recent_picks, -1);
    }
}
