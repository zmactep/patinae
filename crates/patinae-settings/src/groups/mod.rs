//! Typed settings groups.
//!
//! Each group is defined via [`define_settings_group!`](crate::define_settings_group) and contains a plain
//! data struct with concrete Rust types. Groups marked `group` are
//! object-overridable; `group_global` groups are global-only.

pub mod behavior;
pub mod cartoon;
pub mod dot;
pub mod ellipsoid;
pub mod fxaa;
pub mod line;
pub mod measurement;
pub mod mesh;
pub mod movie;
pub mod object;
pub mod renderer;
pub mod ribbon;
pub mod shading;
pub mod sphere;
pub mod ssao;
pub mod stick;
pub mod surface;
pub mod ui;

pub use behavior::{BehaviorSettings, RecentPickLimit};
pub use cartoon::{CartoonOverrides, CartoonSettings};
pub use dot::{DotOverrides, DotSettings};
pub use ellipsoid::{EllipsoidOverrides, EllipsoidSettings};
pub use fxaa::FxaaSettings;
pub use line::{LineOverrides, LineSettings};
pub use measurement::MeasurementSettings;
pub use mesh::{MeshOverrides, MeshSettings};
pub use movie::MovieSettings;
pub use object::{ObjectSettingOverrides, ObjectSettings};
pub use renderer::RendererSettings;
pub use ribbon::{RibbonOverrides, RibbonSettings};
pub use shading::{
    ClassicShadingSettings, CommonShadingSettings, FullShadingSettings, ShadingSettings,
    SkripkinShadingSettings,
};
pub use sphere::{SphereOverrides, SphereSettings};
pub use ssao::SsaoSettings;
pub use stick::{StickOverrides, StickSettings};
pub use surface::{SurfaceOverrides, SurfaceSettings};
pub use ui::UiSettings;

use crate::macros::{FieldDescriptor, SettingDescriptor};

macro_rules! define_settings_from_manifest {
    (
        global { $( $global_field:ident : $global_ty:ty, )* }
        object { $( $object_field:ident : $object_ty:ty => $object_overrides_ty:ty, )* }
    ) => {
        /// Top-level settings container — holds all typed groups.
        ///
        /// Global-only groups live directly on `Settings`. Object-overridable
        /// groups also live here as the global defaults.
        #[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
        pub struct Settings {
            $( #[serde(default)] pub $global_field: $global_ty, )*
            $( #[serde(default)] pub $object_field: $object_ty, )*
        }

        /// Build runtime descriptors from the root settings manifest and each
        /// group's generated field descriptors.
        pub(crate) fn build_setting_descriptors() -> Vec<SettingDescriptor> {
            let mut descs = Vec::new();
            $(
                append_global_group(
                    &mut descs,
                    <$global_ty>::field_descriptors(),
                    |s| &s.$global_field,
                    |s| &mut s.$global_field,
                );
            )*
            $(
                append_object_group(
                    &mut descs,
                    <$object_ty>::field_descriptors(),
                    |s| &s.$object_field,
                    |s| &mut s.$object_field,
                    |o| &o.$object_field,
                    |o| &mut o.$object_field,
                );
            )*
            descs
        }
    };
}

crate::__patinae_settings_root_manifest!(define_settings_from_manifest);

fn append_global_group<G: 'static>(
    out: &mut Vec<SettingDescriptor>,
    fields: Vec<FieldDescriptor<G, ()>>,
    get_group: fn(&Settings) -> &G,
    get_group_mut: fn(&mut Settings) -> &mut G,
) {
    out.extend(
        fields
            .into_iter()
            .map(|field| SettingDescriptor::from_global_field(field, get_group, get_group_mut)),
    );
}

fn append_object_group<G: 'static, O: 'static>(
    out: &mut Vec<SettingDescriptor>,
    fields: Vec<FieldDescriptor<G, O>>,
    get_group: fn(&Settings) -> &G,
    get_group_mut: fn(&mut Settings) -> &mut G,
    get_overrides: fn(&crate::overrides::ObjectOverrides) -> &O,
    get_overrides_mut: fn(&mut crate::overrides::ObjectOverrides) -> &mut O,
) {
    out.extend(fields.into_iter().map(|field| {
        SettingDescriptor::from_object_field(
            field,
            get_group,
            get_group_mut,
            get_overrides,
            get_overrides_mut,
        )
    }));
}

#[cfg(test)]
mod tests {
    use super::Settings;

    #[test]
    fn settings_without_measurement_group_use_defaults() {
        let mut value = serde_json::to_value(Settings::default()).expect("serialize settings");
        value
            .as_object_mut()
            .expect("settings serialize as a map")
            .remove("measurement");

        let restored: Settings = serde_json::from_value(value).expect("deserialize old settings");

        assert_eq!(restored.measurement.dash_length, 0.15);
        assert_eq!(restored.measurement.label_digits, 1);
    }
}
