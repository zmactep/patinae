//! Per-object override containers and resolved settings.
//!
//! `ObjectOverrides` holds `Option<T>` wrappers for every object-overridable
//! group. `ResolvedSettings` is the fully-merged result of global + overrides,
//! built once per object per rebuild.

use crate::groups::{
    CartoonOverrides, CartoonSettings, DotOverrides, DotSettings, EllipsoidOverrides,
    EllipsoidSettings, LineOverrides, LineSettings, MeshOverrides, MeshSettings,
    ObjectSettingOverrides, ObjectSettings, RibbonOverrides, RibbonSettings, SphereOverrides,
    SphereSettings, StickOverrides, StickSettings, SurfaceOverrides, SurfaceSettings,
};
use crate::macros::Merge;

macro_rules! define_overrides_from_manifest {
    (
        global { $( $global_field:ident : $global_ty:ty, )* }
        object { $( $object_field:ident : $object_ty:ty => $object_overrides_ty:ty, )* }
    ) => {
        /// Per-object overrides — only object-overridable groups appear here.
        ///
        /// Global-only groups are absent, enforcing at the type level that
        /// they cannot be overridden per-object.
        #[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
        pub struct ObjectOverrides {
            $( #[serde(default)] pub $object_field: $object_overrides_ty, )*
        }

        /// Fully resolved settings for a single object (global + overrides merged).
        #[derive(Debug, Clone)]
        pub struct ResolvedSettings {
            $( pub $object_field: $object_ty, )*
        }

        impl ResolvedSettings {
            /// Build resolved settings from global defaults + per-object overrides.
            pub fn resolve(
                global: &crate::groups::Settings,
                overrides: Option<&ObjectOverrides>,
            ) -> Self {
                match overrides {
                    Some(o) => Self {
                        $(
                            $object_field: global
                                .$object_field
                                .with_overrides(&o.$object_field),
                        )*
                    },
                    None => Self {
                        $( $object_field: global.$object_field.clone(), )*
                    },
                }
            }
        }
    };
}

crate::__patinae_settings_root_manifest!(define_overrides_from_manifest);
