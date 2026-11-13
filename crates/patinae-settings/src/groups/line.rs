//! Line representation settings (object-overridable).

use crate::define_settings_group;
use crate::ColorValue;

define_settings_group! {
    /// Line representation parameters.
    group LineSettings / LineOverrides {
        // Screen-space width in physical pixels; the default matches PyMOL.
        width: f32 = 1.49,
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
    fn line_width_defaults_to_pymol_value() {
        assert_eq!(LineSettings::default().width, 1.49);
    }

    #[test]
    fn line_width_is_registered_and_object_overridable() {
        let desc = registry::lookup_by_name("line_width").expect("line_width registered");
        assert!(desc.is_object_overridable());
    }
}
