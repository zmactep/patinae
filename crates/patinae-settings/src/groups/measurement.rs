//! Measurement rendering settings.

use crate::{define_settings_group, Color};

define_settings_group! {
    /// Distance, angle, and dihedral display parameters.
    group_global MeasurementSettings {
        dash_length: f32 = 0.15,
            name = "dash_length",
            min = 0.0, max = 100.0,
            side_effects = [RepresentationRebuild];
        dash_gap: f32 = 0.45,
            name = "dash_gap",
            min = 0.0, max = 100.0,
            side_effects = [RepresentationRebuild];
        dash_width: f32 = 2.5,
            name = "dash_width",
            min = 0.0, max = 100.0,
            side_effects = [RepresentationRebuild];
        dash_round_ends: bool = true,
            name = "dash_round_ends",
            side_effects = [RepresentationRebuild];
        angle_size: f32 = 0.6666,
            name = "angle_size",
            min = 0.0, max = 100.0,
            side_effects = [RepresentationRebuild];
        angle_label_position: f32 = 0.5,
            name = "angle_label_position",
            side_effects = [ViewportUpdate];
        dihedral_size: f32 = 0.6666,
            name = "dihedral_size",
            min = 0.0, max = 100.0,
            side_effects = [RepresentationRebuild];
        dihedral_label_position: f32 = 1.2,
            name = "dihedral_label_position",
            side_effects = [ViewportUpdate];
        label_digits: i32 = 1,
            name = "label_digits",
            min = 0, max = 9,
            side_effects = [ViewportUpdate];
        label_distance_digits: i32 = -1,
            name = "label_distance_digits",
            min = -1, max = 9,
            side_effects = [ViewportUpdate];
        label_angle_digits: i32 = -1,
            name = "label_angle_digits",
            min = -1, max = 9,
            side_effects = [ViewportUpdate];
        label_dihedral_digits: i32 = -1,
            name = "label_dihedral_digits",
            min = -1, max = 9,
            side_effects = [ViewportUpdate];
        dash_color: Color = Color::UNSET,
            name = "dash_color",
            side_effects = [ColorRebuild];
        angle_color: Color = Color::UNSET,
            name = "angle_color",
            side_effects = [ColorRebuild];
        dihedral_color: Color = Color::UNSET,
            name = "dihedral_color",
            side_effects = [ColorRebuild];
        dash_transparency: f32 = 0.0,
            name = "dash_transparency",
            min = 0.0, max = 1.0,
            side_effects = [ColorRebuild];
        label_size: f32 = 14.0,
            name = "label_size",
            min = 0.01, max = 1000.0,
            side_effects = [ViewportUpdate];
    }
}

#[cfg(test)]
mod tests {
    use super::MeasurementSettings;

    #[test]
    fn defaults_match_measurement_settings() {
        let settings = MeasurementSettings::default();

        assert_eq!(settings.dash_length, 0.15);
        assert_eq!(settings.dash_gap, 0.45);
        assert_eq!(settings.dash_width, 2.5);
        assert_eq!(settings.angle_size, 0.6666);
        assert_eq!(settings.dihedral_size, 0.6666);
        assert_eq!(settings.label_digits, 1);
        assert_eq!(settings.label_size, 14.0);
        assert_eq!(settings.dash_transparency, 0.0);
    }
}
