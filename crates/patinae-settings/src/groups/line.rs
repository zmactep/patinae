//! Line representation settings (object-overridable).

use crate::define_settings_group;
use crate::ColorValue;

define_settings_group! {
    /// Line representation parameters.
    group LineSettings / LineOverrides {
        @global {
            color: ColorValue = ColorValue::INHERIT,
                name = "line_color",
                side_effects = [ColorRebuild];
        }
    }
}
