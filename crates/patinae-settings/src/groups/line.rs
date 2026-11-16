//! Line representation settings (object-overridable).

use crate::define_settings_group;
use crate::ColorValue;

define_settings_group! {
    /// Line representation parameters.
    #[serde(default)]
    group LineSettings / LineOverrides {
        // Screen-space width in physical pixels; the default uses native one-pixel lines.
        width: f32 = 1.0,
            name = "line_width",
            min = 1.0, max = 20.0,
            side_effects = [RepresentationRebuild];
        @global {
            color: ColorValue = ColorValue::INHERIT,
                name = "line_color",
                side_effects = [ColorRebuild];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::LineSettings;
    use crate::registry;

    #[test]
    fn line_width_defaults_to_one_pixel() {
        assert_eq!(LineSettings::default().width, 1.0);
        assert_eq!(
            crate::get_setting(crate::id::line_width).unwrap().default,
            crate::SettingValue::Float(1.0)
        );
        let resolved = crate::ResolvedSettings::resolve(&crate::Settings::default(), None);
        assert_eq!(resolved.line.width, 1.0);
    }

    #[test]
    fn line_width_is_registered_and_object_overridable() {
        let desc = registry::lookup_by_name("line_width").expect("line_width registered");
        assert!(desc.is_object_overridable());
    }

    #[test]
    fn line_override_defaults_preserve_strict_field_validation() {
        use super::LineOverrides;

        for old in [serde_json::json!({}), serde_json::json!([])] {
            let overrides: LineOverrides = serde_json::from_value(old).unwrap();
            assert_eq!(overrides.width, None);
        }
        assert!(
            serde_json::from_value::<LineOverrides>(serde_json::json!({"widht": 3.0})).is_err()
        );
        assert!(
            serde_json::from_value::<LineOverrides>(serde_json::json!({"width": "invalid"}))
                .is_err()
        );
    }
}
