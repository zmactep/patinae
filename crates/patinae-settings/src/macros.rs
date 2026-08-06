//! Declarative macros for generating typed settings groups.
//!
//! The `define_settings_group!` macro is the single source of truth for each
//! settings group. It generates:
//! 1. The settings struct with concrete field types
//! 2. `Default` impl with specified default values
//! 3. (For `group`) An `*Overrides` struct with `Option<T>` fields
//! 4. (For `group`) A `Merge` impl
//! 5. Descriptor entries for the string-based `set`/`get` command registry

use crate::error::SettingError;
use crate::setting::{Color, SettingType, SettingValue};
use crate::side_effects::SideEffectCategory;

/// Trait that maps Rust types to `SettingType`/`SettingValue` conversions.
///
/// Implemented for `bool`, `i32`, `f32`, `[f32; 3]`, and any type implementing
/// `SettingEnum` (via blanket impl below).
pub trait SettingKind: Clone {
    fn setting_type() -> SettingType;
    fn wrap(&self) -> SettingValue;
    fn unwrap_value(v: SettingValue) -> Result<Self, SettingError>;
}

impl SettingKind for bool {
    fn setting_type() -> SettingType {
        SettingType::Bool
    }
    fn wrap(&self) -> SettingValue {
        SettingValue::Bool(*self)
    }
    fn unwrap_value(v: SettingValue) -> Result<Self, SettingError> {
        v.as_bool().ok_or(SettingError::type_mismatch("bool"))
    }
}

impl SettingKind for i32 {
    fn setting_type() -> SettingType {
        SettingType::Int
    }
    fn wrap(&self) -> SettingValue {
        SettingValue::Int(*self)
    }
    fn unwrap_value(v: SettingValue) -> Result<Self, SettingError> {
        v.as_int().ok_or(SettingError::type_mismatch("int"))
    }
}

impl SettingKind for f32 {
    fn setting_type() -> SettingType {
        SettingType::Float
    }
    fn wrap(&self) -> SettingValue {
        SettingValue::Float(*self)
    }
    fn unwrap_value(v: SettingValue) -> Result<Self, SettingError> {
        v.as_float().ok_or(SettingError::type_mismatch("float"))
    }
}

impl SettingKind for [f32; 3] {
    fn setting_type() -> SettingType {
        SettingType::Float3
    }
    fn wrap(&self) -> SettingValue {
        SettingValue::Float3(*self)
    }
    fn unwrap_value(v: SettingValue) -> Result<Self, SettingError> {
        v.as_float3().ok_or(SettingError::type_mismatch("float3"))
    }
}

impl SettingKind for Color {
    fn setting_type() -> SettingType {
        SettingType::Color
    }
    // Storage stays an int — PSE wire format uses `SettingValue::Int(i32)`.
    // The wrapper exists so the registry advertises `SettingType::Color`,
    // which drives parsing and CLI completion.
    fn wrap(&self) -> SettingValue {
        SettingValue::Int(self.0)
    }
    fn unwrap_value(v: SettingValue) -> Result<Self, SettingError> {
        v.as_int()
            .map(Color)
            .ok_or(SettingError::type_mismatch("color (int index)"))
    }
}

/// Blanket impl for any enum type implementing `SettingEnum` — stored as Int on the wire.
impl<T: crate::SettingEnum + Clone> SettingKind for T {
    fn setting_type() -> SettingType {
        SettingType::Int
    }
    fn wrap(&self) -> SettingValue {
        SettingValue::Int(self.to_i32())
    }
    fn unwrap_value(v: SettingValue) -> Result<Self, SettingError> {
        v.as_int()
            .map(T::from_i32)
            .ok_or(SettingError::type_mismatch("int (enum)"))
    }
}

type GroupGet<G> = Box<dyn Fn(&G) -> SettingValue + Send + Sync>;
type GroupSet<G> = Box<dyn Fn(&mut G, SettingValue) -> Result<(), SettingError> + Send + Sync>;
type OverrideGet<O> = Box<dyn Fn(&O) -> Option<SettingValue> + Send + Sync>;
type OverrideSet<O> = Box<dyn Fn(&mut O, SettingValue) -> Result<(), SettingError> + Send + Sync>;
type OverrideUnset<O> = Box<dyn Fn(&mut O) + Send + Sync>;

/// Descriptor for a field relative to one typed settings group.
///
/// Group files are the descriptor source of truth: every field declared through
/// `define_settings_group!` produces one `FieldDescriptor`. The root settings
/// manifest composes these relative descriptors into runtime
/// [`SettingDescriptor`] entries.
pub struct FieldDescriptor<G, O = ()> {
    /// String name for the `set`/`get` commands.
    pub name: &'static str,
    /// Value type.
    pub setting_type: SettingType,
    /// Min constraint (numeric).
    pub min: Option<f32>,
    /// Max constraint (numeric).
    pub max: Option<f32>,
    /// Named value variants (for enum settings).
    pub value_hints: &'static [(&'static str, SettingValue)],
    /// Side effects triggered when this setting changes.
    pub side_effects: &'static [SideEffectCategory],
    get: GroupGet<G>,
    set: GroupSet<G>,
    get_override: Option<OverrideGet<O>>,
    set_override: Option<OverrideSet<O>>,
    unset_override: Option<OverrideUnset<O>>,
}

impl<G: 'static> FieldDescriptor<G, ()> {
    /// Create a global-only descriptor for one field of `G`.
    pub fn global<T: SettingKind + 'static>(
        name: &'static str,
        get_field: fn(&G) -> &T,
        get_field_mut: fn(&mut G) -> &mut T,
        min: Option<f32>,
        max: Option<f32>,
        value_hints: &'static [(&'static str, SettingValue)],
        side_effects: &'static [SideEffectCategory],
    ) -> Self {
        Self {
            name,
            setting_type: T::setting_type(),
            min,
            max,
            value_hints,
            side_effects,
            get: Box::new(move |g| get_field(g).wrap()),
            set: Box::new(move |g, v| {
                *get_field_mut(g) = T::unwrap_value(v)?;
                Ok(())
            }),
            get_override: None,
            set_override: None,
            unset_override: None,
        }
    }
}

impl<G: 'static, O: 'static> FieldDescriptor<G, O> {
    /// Create an object-overridable descriptor for one field of `G`.
    #[expect(
        clippy::too_many_arguments,
        reason = "macro-generated descriptor constructor keeps each field accessor explicit"
    )]
    pub fn object<T: SettingKind + 'static>(
        name: &'static str,
        get_field: fn(&G) -> &T,
        get_field_mut: fn(&mut G) -> &mut T,
        get_override_field: fn(&O) -> &Option<T>,
        get_override_field_mut: fn(&mut O) -> &mut Option<T>,
        min: Option<f32>,
        max: Option<f32>,
        value_hints: &'static [(&'static str, SettingValue)],
        side_effects: &'static [SideEffectCategory],
    ) -> Self {
        Self {
            name,
            setting_type: T::setting_type(),
            min,
            max,
            value_hints,
            side_effects,
            get: Box::new(move |g| get_field(g).wrap()),
            set: Box::new(move |g, v| {
                *get_field_mut(g) = T::unwrap_value(v)?;
                Ok(())
            }),
            get_override: Some(Box::new(move |o| {
                get_override_field(o).as_ref().map(|v| v.wrap())
            })),
            set_override: Some(Box::new(move |o, v| {
                *get_override_field_mut(o) = Some(T::unwrap_value(v)?);
                Ok(())
            })),
            unset_override: Some(Box::new(move |o| {
                *get_override_field_mut(o) = None;
            })),
        }
    }

    /// Read a value from a concrete settings group.
    pub fn get_from_group(&self, group: &G) -> SettingValue {
        (self.get)(group)
    }

    /// Write a value into a concrete settings group.
    pub fn set_on_group(&self, group: &mut G, value: SettingValue) -> Result<(), SettingError> {
        (self.set)(group, value)
    }

    /// Read a value from a concrete overrides group.
    pub fn get_from_overrides(&self, overrides: &O) -> Option<SettingValue> {
        self.get_override.as_ref().and_then(|get| get(overrides))
    }

    /// Write a value into a concrete overrides group.
    pub fn set_on_overrides(
        &self,
        overrides: &mut O,
        value: SettingValue,
    ) -> Result<(), SettingError> {
        match &self.set_override {
            Some(set) => set(overrides, value),
            None => Ok(()),
        }
    }

    /// Clear a value from a concrete overrides group.
    pub fn unset_on_overrides(&self, overrides: &mut O) {
        if let Some(unset) = &self.unset_override {
            unset(overrides);
        }
    }

    /// Whether this field supports per-object overrides.
    pub fn is_object_overridable(&self) -> bool {
        self.set_override.is_some()
    }
}

/// Compose nested global-only descriptors into a parent settings group.
pub fn append_nested_global_descriptors<P: 'static, G: 'static>(
    out: &mut Vec<FieldDescriptor<P, ()>>,
    fields: Vec<FieldDescriptor<G, ()>>,
    get_group: fn(&P) -> &G,
    get_group_mut: fn(&mut P) -> &mut G,
) {
    out.extend(fields.into_iter().map(|field| {
        let FieldDescriptor {
            name,
            setting_type,
            min,
            max,
            value_hints,
            side_effects,
            get,
            set,
            ..
        } = field;
        FieldDescriptor {
            name,
            setting_type,
            min,
            max,
            value_hints,
            side_effects,
            get: Box::new(move |parent| get(get_group(parent))),
            set: Box::new(move |parent, value| set(get_group_mut(parent), value)),
            get_override: None,
            set_override: None,
            unset_override: None,
        }
    }));
}

/// Runtime descriptor composed from the root `Settings` / `ObjectOverrides`
/// types. Accessors are private; callers use methods so the registry can be
/// assembled from generated field descriptors instead of per-setting code.
pub struct SettingDescriptor {
    /// String name for the `set`/`get` commands.
    pub name: &'static str,
    /// Value type.
    pub setting_type: SettingType,
    /// Min constraint (numeric).
    pub min: Option<f32>,
    /// Max constraint (numeric).
    pub max: Option<f32>,
    /// Named value variants (for enum settings).
    pub value_hints: &'static [(&'static str, SettingValue)],
    /// Side effects triggered when this setting changes.
    pub side_effects: &'static [SideEffectCategory],
    get: GroupGet<crate::groups::Settings>,
    set: GroupSet<crate::groups::Settings>,
    get_override: Option<OverrideGet<crate::overrides::ObjectOverrides>>,
    set_override: Option<OverrideSet<crate::overrides::ObjectOverrides>>,
    unset_override: Option<OverrideUnset<crate::overrides::ObjectOverrides>>,
}

impl SettingDescriptor {
    /// Compose a root-level descriptor for a global-only settings group.
    pub fn from_global_field<G: 'static>(
        field: FieldDescriptor<G, ()>,
        get_group: fn(&crate::groups::Settings) -> &G,
        get_group_mut: fn(&mut crate::groups::Settings) -> &mut G,
    ) -> Self {
        let FieldDescriptor {
            name,
            setting_type,
            min,
            max,
            value_hints,
            side_effects,
            get,
            set,
            ..
        } = field;
        Self {
            name,
            setting_type,
            min,
            max,
            value_hints,
            side_effects,
            get: Box::new(move |settings| get(get_group(settings))),
            set: Box::new(move |settings, value| set(get_group_mut(settings), value)),
            get_override: None,
            set_override: None,
            unset_override: None,
        }
    }

    /// Compose a root-level descriptor for an object-overridable settings group.
    pub fn from_object_field<G: 'static, O: 'static>(
        field: FieldDescriptor<G, O>,
        get_group: fn(&crate::groups::Settings) -> &G,
        get_group_mut: fn(&mut crate::groups::Settings) -> &mut G,
        get_overrides: fn(&crate::overrides::ObjectOverrides) -> &O,
        get_overrides_mut: fn(&mut crate::overrides::ObjectOverrides) -> &mut O,
    ) -> Self {
        let FieldDescriptor {
            name,
            setting_type,
            min,
            max,
            value_hints,
            side_effects,
            get,
            set,
            get_override,
            set_override,
            unset_override,
        } = field;
        Self {
            name,
            setting_type,
            min,
            max,
            value_hints,
            side_effects,
            get: Box::new(move |settings| get(get_group(settings))),
            set: Box::new(move |settings, value| set(get_group_mut(settings), value)),
            get_override: get_override.map(|get| {
                Box::new(move |overrides: &crate::overrides::ObjectOverrides| {
                    get(get_overrides(overrides))
                }) as OverrideGet<crate::overrides::ObjectOverrides>
            }),
            set_override: set_override.map(|set| {
                Box::new(
                    move |overrides: &mut crate::overrides::ObjectOverrides, value| {
                        set(get_overrides_mut(overrides), value)
                    },
                ) as OverrideSet<crate::overrides::ObjectOverrides>
            }),
            unset_override: unset_override.map(|unset| {
                Box::new(move |overrides: &mut crate::overrides::ObjectOverrides| {
                    unset(get_overrides_mut(overrides));
                }) as OverrideUnset<crate::overrides::ObjectOverrides>
            }),
        }
    }

    /// Read the current value from global `Settings`.
    pub fn get(&self, settings: &crate::groups::Settings) -> SettingValue {
        (self.get)(settings)
    }

    /// Write a value to global `Settings`.
    pub fn set(
        &self,
        settings: &mut crate::groups::Settings,
        value: SettingValue,
    ) -> Result<(), SettingError> {
        self.validate_range(&value)?;
        (self.set)(settings, value)
    }

    /// Read an object override value.
    pub fn get_override(
        &self,
        overrides: &crate::overrides::ObjectOverrides,
    ) -> Option<SettingValue> {
        self.get_override.as_ref().and_then(|get| get(overrides))
    }

    /// Write an object override value. Returns `false` when global-only.
    pub fn set_override(
        &self,
        overrides: &mut crate::overrides::ObjectOverrides,
        value: SettingValue,
    ) -> Result<bool, SettingError> {
        match &self.set_override {
            Some(set) => {
                self.validate_range(&value)?;
                set(overrides, value)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// Clear an object override value. Returns `false` when global-only.
    pub fn unset_override(&self, overrides: &mut crate::overrides::ObjectOverrides) -> bool {
        match &self.unset_override {
            Some(unset) => {
                unset(overrides);
                true
            }
            None => false,
        }
    }

    /// Whether this setting has named value variants.
    pub fn has_value_hints(&self) -> bool {
        !self.value_hints.is_empty()
    }

    /// Look up a setting value from a string alias (case-insensitive).
    pub fn resolve_hint(&self, name: &str) -> Option<&'static SettingValue> {
        let name_lower = name.to_lowercase();
        self.value_hints
            .iter()
            .find(|(n, _)| n.to_lowercase() == name_lower)
            .map(|(_, v)| v)
    }

    /// Look up a display name from a setting value.
    pub fn hint_name(&self, value: &SettingValue) -> Option<&'static str> {
        self.value_hints
            .iter()
            .find(|(_, v)| v == value)
            .map(|(n, _)| *n)
    }

    /// Get all hint names (for autocomplete).
    pub fn hint_names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.value_hints.iter().map(|(n, _)| *n)
    }

    /// Whether this setting supports per-object overrides.
    pub fn is_object_overridable(&self) -> bool {
        self.set_override.is_some()
    }

    fn validate_range(&self, value: &SettingValue) -> Result<(), SettingError> {
        let Some(value) = value.as_float() else {
            return Ok(());
        };
        let below_minimum = self.min.is_some_and(|minimum| value < minimum);
        let above_maximum = self.max.is_some_and(|maximum| value > maximum);
        if below_minimum || above_maximum {
            return Err(SettingError::InvalidValue {
                name: self.name.to_string(),
                reason: format!(
                    "value {value} is outside range [{}, {}]",
                    self.min.map_or("unbounded".to_string(), |v| v.to_string()),
                    self.max.map_or("unbounded".to_string(), |v| v.to_string())
                ),
            });
        }
        Ok(())
    }
}

#[doc(hidden)]
#[macro_export]
macro_rules! __patinae_settings_root_manifest {
    ($callback:ident) => {
        $callback! {
            global {
                shading: ShadingSettings,
                ui: UiSettings,
                movie: MovieSettings,
                renderer: RendererSettings,
                behavior: BehaviorSettings,
                ssao: SsaoSettings,
                fxaa: FxaaSettings,
                measurement: MeasurementSettings,
            }
            object {
                object: ObjectSettings => ObjectSettingOverrides,
                cartoon: CartoonSettings => CartoonOverrides,
                stick: StickSettings => StickOverrides,
                sphere: SphereSettings => SphereOverrides,
                surface: SurfaceSettings => SurfaceOverrides,
                ribbon: RibbonSettings => RibbonOverrides,
                line: LineSettings => LineOverrides,
                dot: DotSettings => DotOverrides,
                mesh: MeshSettings => MeshOverrides,
                ellipsoid: EllipsoidSettings => EllipsoidOverrides,
            }
        }
    };
}

/// Trait for merging a base settings struct with an overrides struct.
pub trait Merge: Clone {
    type Overrides: Default;
    /// Return a copy of `self` with non-None overrides applied.
    fn with_overrides(&self, overrides: &Self::Overrides) -> Self;
}

// =============================================================================
// Core macro: define_settings_group!
// =============================================================================

/// Define a typed settings group.
///
/// # Forms
///
/// **`group` — object-overridable** (generates struct + Overrides + Merge):
/// ```ignore
/// define_settings_group! {
///     group StickSettings / StickOverrides {
///         radius: f32 = 0.25, name = "stick_radius",
///             min = 0.01, max = 5.0,
///             side_effects = [RepresentationRebuild];
///         color: i32 = -1, name = "stick_color",
///             side_effects = [ColorRebuild];
///     }
/// }
/// ```
///
/// **`group_global` — global-only** (generates struct only, no Overrides):
/// ```ignore
/// define_settings_group! {
///     group_global RayTraceSettings {
///         max_passes: i32 = 25, name = "ray_max_passes";
///     }
/// }
/// ```
#[macro_export]
macro_rules! define_settings_group {
    // =========================================================================
    // group (object-overridable): generates struct + Overrides + Merge
    // =========================================================================
    (
        $(#[$meta:meta])*
        group $Name:ident / $Overrides:ident {
            $(
                $(#[$field_meta:meta])*
                $field:ident : $ty:ty = $default:expr,
                    name = $sname:expr
                    $(, min = $min:expr, max = $max:expr)?
                    $(, hints = $hints_ty:ty)?
                    $(, side_effects = [$($se:ident),* $(,)?])?
                    ;
            )*
        }
    ) => {
        #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
        $(#[$meta])*
        pub struct $Name {
            $(
                $(#[$field_meta])*
                pub $field: $ty,
            )*
        }

        impl Default for $Name {
            fn default() -> Self {
                Self {
                    $( $field: $default, )*
                }
            }
        }

        #[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
        pub struct $Overrides {
            $(
                pub $field: Option<$ty>,
            )*
        }

        impl $crate::macros::Merge for $Name {
            type Overrides = $Overrides;

            fn with_overrides(&self, o: &$Overrides) -> Self {
                Self {
                    $( $field: o.$field.unwrap_or(self.$field.clone()), )*
                }
            }
        }

        impl $Name {
            /// Return descriptor entries for all fields in this group.
            #[allow(unused_imports)]
            pub fn field_descriptors() -> Vec<$crate::macros::FieldDescriptor<Self, $Overrides>> {
                use $crate::SettingEnum as _;
                vec![
                    $(
                        $crate::macros::FieldDescriptor::<Self, $Overrides>::object::<$ty>(
                            $sname,
                            |s| &s.$field,
                            |s| &mut s.$field,
                            |o| &o.$field,
                            |o| &mut o.$field,
                            $crate::define_settings_group!(@opt_f32 $($min)?),
                            $crate::define_settings_group!(@opt_f32 $($max)?),
                            $crate::define_settings_group!(@hints $($hints_ty)?),
                            &[$($($crate::SideEffectCategory::$se),*)?],
                        ),
                    )*
                ]
            }
        }
    };

    // =========================================================================
    // group_global: generates struct only, no Overrides
    // =========================================================================
    (
        $(#[$meta:meta])*
        group_global $Name:ident {
            $(
                $(#[$field_meta:meta])*
                $field:ident : $ty:ty = $default:expr,
                    name = $sname:expr
                    $(, min = $min:expr, max = $max:expr)?
                    $(, hints = $hints_ty:ty)?
                    $(, side_effects = [$($se:ident),* $(,)?])?
                    ;
            )*
        }
    ) => {
        #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
        $(#[$meta])*
        pub struct $Name {
            $(
                $(#[$field_meta])*
                pub $field: $ty,
            )*
        }

        impl Default for $Name {
            fn default() -> Self {
                Self {
                    $( $field: $default, )*
                }
            }
        }

        impl $Name {
            /// Return descriptor entries for all fields in this group.
            #[allow(unused_imports)]
            pub fn field_descriptors() -> Vec<$crate::macros::FieldDescriptor<Self, ()>> {
                use $crate::SettingEnum as _;
                vec![
                    $(
                        $crate::macros::FieldDescriptor::<Self, ()>::global::<$ty>(
                            $sname,
                            |s| &s.$field,
                            |s| &mut s.$field,
                            $crate::define_settings_group!(@opt_f32 $($min)?),
                            $crate::define_settings_group!(@opt_f32 $($max)?),
                            $crate::define_settings_group!(@hints $($hints_ty)?),
                            &[$($($crate::SideEffectCategory::$se),*)?],
                        ),
                    )*
                ]
            }
        }
    };

    // =========================================================================
    // Internal helpers
    // =========================================================================
    (@opt_f32) => { None };
    (@opt_f32 $val:expr) => { Some($val as f32) };

    (@hints) => { &[] };
    (@hints $ty:ty) => { <$ty as $crate::SettingEnum>::value_hints() };
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test the `group` form (object-overridable): generates struct + Overrides + Merge
    crate::define_settings_group! {
        /// Test stick settings
        group TestStick / TestStickOverrides {
            radius: f32 = 0.25,
                name = "stick_radius",
                min = 0.01, max = 5.0,
                side_effects = [RepresentationRebuild];
            color: i32 = -1,
                name = "stick_color",
                side_effects = [ColorRebuild];
        }
    }

    // Test the `group_global` form (global-only): generates struct only
    crate::define_settings_group! {
        group_global TestGlobal {
            max_passes: i32 = 25,
                name = "ray_max_passes";
            enabled: bool = true,
                name = "test_enabled",
                side_effects = [SceneInvalidate];
        }
    }

    // Test with enum type + hints
    crate::define_settings_group! {
        group_global TestWithEnum {
            mode: crate::ShadingMode = crate::ShadingMode::Classic,
                name = "shading_mode",
                hints = crate::ShadingMode,
                side_effects = [ShaderReload];
        }
    }

    #[test]
    fn test_group_defaults() {
        let s = TestStick::default();
        assert_eq!(s.radius, 0.25);
        assert_eq!(s.color, -1);
    }

    #[test]
    fn test_overrides_default_is_none() {
        let o = TestStickOverrides::default();
        assert!(o.radius.is_none());
        assert!(o.color.is_none());
    }

    #[test]
    fn test_merge_no_overrides() {
        let base = TestStick::default();
        let o = TestStickOverrides::default();
        let merged = base.with_overrides(&o);
        assert_eq!(merged.radius, 0.25);
        assert_eq!(merged.color, -1);
    }

    #[test]
    fn test_merge_with_override() {
        let base = TestStick::default();
        let o = TestStickOverrides {
            radius: Some(0.5),
            color: None,
        };
        let merged = base.with_overrides(&o);
        assert_eq!(merged.radius, 0.5);
        assert_eq!(merged.color, -1);
    }

    #[test]
    fn test_global_defaults() {
        let s = TestGlobal::default();
        assert_eq!(s.max_passes, 25);
        assert!(s.enabled);
    }

    #[test]
    fn test_descriptors_group() {
        let descs = TestStick::field_descriptors();
        assert_eq!(descs.len(), 2);
        assert_eq!(descs[0].name, "stick_radius");
        assert_eq!(descs[0].setting_type, SettingType::Float);
        assert_eq!(descs[0].min, Some(0.01));
        assert_eq!(descs[0].max, Some(5.0));
        assert_eq!(descs[0].side_effects.len(), 1);

        assert_eq!(descs[1].name, "stick_color");
        assert_eq!(descs[1].setting_type, SettingType::Int);
        assert!(descs[1].min.is_none());
    }

    #[test]
    fn test_descriptors_global() {
        let descs = TestGlobal::field_descriptors();
        assert_eq!(descs.len(), 2);
        assert_eq!(descs[0].name, "ray_max_passes");
        assert!(!descs[0].is_object_overridable());
    }

    #[test]
    fn test_color_setting_kind() {
        use crate::setting::Color;
        assert_eq!(<Color as SettingKind>::setting_type(), SettingType::Color);
        // Storage roundtrips via SettingValue::Int (PSE wire-compatible).
        assert_eq!(Color(7).wrap(), SettingValue::Int(7));
        assert_eq!(
            <Color as SettingKind>::unwrap_value(SettingValue::Int(7)).unwrap(),
            Color(7),
        );
        // Default == UNSET
        assert_eq!(Color::default(), Color::UNSET);
        assert_eq!(Color::UNSET.0, -1);
    }

    #[test]
    fn test_enum_hints() {
        let descs = TestWithEnum::field_descriptors();
        assert_eq!(descs.len(), 1);
        assert_eq!(descs[0].name, "shading_mode");
        assert_eq!(descs[0].setting_type, SettingType::Int); // enum stored as Int
        assert_eq!(descs[0].value_hints.len(), 3); // classic, skripkin, full
        assert_eq!(descs[0].value_hints[0].0, "classic");
    }
}
