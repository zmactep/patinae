//! Setting definitions and value types

use serde::{Deserialize, Serialize};
use std::fmt;

/// Type of a setting value
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum SettingType {
    /// Unused/placeholder setting
    Blank = 0,
    /// Boolean value
    Bool = 1,
    /// Integer value
    Int = 2,
    /// Single float value
    Float = 3,
    /// Three-component float vector (e.g., positions, light directions)
    Float3 = 4,
    /// Tagged color value (resolved via patinae-color)
    Color = 5,
    /// String value
    String = 6,
}

impl fmt::Display for SettingType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SettingType::Blank => write!(f, "blank"),
            SettingType::Bool => write!(f, "bool"),
            SettingType::Int => write!(f, "int"),
            SettingType::Float => write!(f, "float"),
            SettingType::Float3 => write!(f, "float3"),
            SettingType::Color => write!(f, "color"),
            SettingType::String => write!(f, "string"),
        }
    }
}

/// Setting level - determines where a setting can be applied
///
/// Settings follow a hierarchical inheritance model:
/// - `global` < `object` < `object-state`
/// - `object-state` < `atom` < `atom-state`
/// - `object-state` < `bond` < `bond-state`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum SettingLevel {
    /// Deprecated/unused settings
    Unused = 0,
    /// Global settings (affect entire session)
    Global = 1,
    /// Per-object settings
    Object = 2,
    /// Per-object-state settings
    ObjectState = 3,
    /// Per-atom settings
    Atom = 4,
    /// Per-atom-state settings
    AtomState = 5,
    /// Per-bond settings
    Bond = 6,
    /// Per-bond-state settings
    BondState = 7,
}

impl SettingLevel {
    /// Get the display name for this level
    pub fn name(&self) -> &'static str {
        match self {
            SettingLevel::Unused => "unused",
            SettingLevel::Global => "global",
            SettingLevel::Object => "object",
            SettingLevel::ObjectState => "object-state",
            SettingLevel::Atom => "atom",
            SettingLevel::AtomState => "atom-state",
            SettingLevel::Bond => "bond",
            SettingLevel::BondState => "bond-state",
        }
    }

    /// Get the bitmask for valid sub-levels
    /// Used to check if a setting can be applied at a given level
    pub fn mask(&self) -> u8 {
        match self {
            SettingLevel::Unused => 0x00,
            SettingLevel::Global => 0x00,
            SettingLevel::Object => 0x01,
            SettingLevel::ObjectState => 0x03,
            SettingLevel::Atom => 0x07,
            SettingLevel::AtomState => 0x0F,
            SettingLevel::Bond => 0x13,
            SettingLevel::BondState => 0x33,
        }
    }

    /// Check if this level is valid within the given mask
    pub fn check_mask(&self, mask: u8) -> bool {
        (mask & !self.mask()) == 0
    }
}

impl fmt::Display for SettingLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// A setting value
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SettingValue {
    /// Boolean value
    Bool(bool),
    /// Integer value
    Int(i32),
    /// Single float value
    Float(f32),
    /// Three-component float vector
    Float3([f32; 3]),
    /// Tagged color value: RGB, palette, scheme, or inheritance.
    Color(ColorValue),
    /// String value
    String(String),
}

impl SettingValue {
    /// Get the type of this value
    pub fn setting_type(&self) -> SettingType {
        match self {
            SettingValue::Bool(_) => SettingType::Bool,
            SettingValue::Int(_) => SettingType::Int,
            SettingValue::Float(_) => SettingType::Float,
            SettingValue::Float3(_) => SettingType::Float3,
            SettingValue::Color(_) => SettingType::Color,
            SettingValue::String(_) => SettingType::String,
        }
    }

    /// Try to get as bool
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            SettingValue::Bool(v) => Some(*v),
            SettingValue::Int(v) => Some(*v != 0),
            SettingValue::Float(v) => Some(*v != 0.0),
            _ => None,
        }
    }

    /// Try to get as int
    pub fn as_int(&self) -> Option<i32> {
        match self {
            SettingValue::Int(v) => Some(*v),
            SettingValue::Bool(v) => Some(if *v { 1 } else { 0 }),
            SettingValue::Float(v) => Some(*v as i32),
            _ => None,
        }
    }

    /// Try to get as float
    pub fn as_float(&self) -> Option<f32> {
        match self {
            SettingValue::Float(v) => Some(*v),
            SettingValue::Int(v) => Some(*v as f32),
            SettingValue::Bool(v) => Some(if *v { 1.0 } else { 0.0 }),
            _ => None,
        }
    }

    /// Try to get as float3
    pub fn as_float3(&self) -> Option<[f32; 3]> {
        match self {
            SettingValue::Float3(v) => Some(*v),
            _ => None,
        }
    }

    /// Try to get a tagged color value
    pub fn as_color(&self) -> Option<ColorValue> {
        match self {
            SettingValue::Color(v) => Some(*v),
            _ => None,
        }
    }

    /// Try to get as string
    pub fn as_string(&self) -> Option<&str> {
        match self {
            SettingValue::String(v) => Some(v),
            _ => None,
        }
    }

    /// Check if this value can be converted to the target type
    pub fn is_compatible_with(&self, target_type: SettingType) -> bool {
        match (self, target_type) {
            // Exact matches
            (SettingValue::Bool(_), SettingType::Bool) => true,
            (SettingValue::Int(_), SettingType::Int) => true,
            (SettingValue::Float(_), SettingType::Float) => true,
            (SettingValue::Float3(_), SettingType::Float3) => true,
            (SettingValue::Color(_), SettingType::Color) => true,
            (SettingValue::String(_), SettingType::String) => true,
            // Int-compatible types
            (SettingValue::Bool(_), SettingType::Int) => true,
            (SettingValue::Int(_), SettingType::Bool) => true,
            // Float-compatible types
            (SettingValue::Int(_), SettingType::Float) => true,
            (SettingValue::Bool(_), SettingType::Float) => true,
            (SettingValue::Float(_), SettingType::Int) => true,
            _ => false,
        }
    }
}

impl Default for SettingValue {
    fn default() -> Self {
        SettingValue::Int(0)
    }
}

impl From<bool> for SettingValue {
    fn from(v: bool) -> Self {
        SettingValue::Bool(v)
    }
}

impl From<i32> for SettingValue {
    fn from(v: i32) -> Self {
        SettingValue::Int(v)
    }
}

impl From<f32> for SettingValue {
    fn from(v: f32) -> Self {
        SettingValue::Float(v)
    }
}

impl From<[f32; 3]> for SettingValue {
    fn from(v: [f32; 3]) -> Self {
        SettingValue::Float3(v)
    }
}

impl From<String> for SettingValue {
    fn from(v: String) -> Self {
        SettingValue::String(v)
    }
}

impl From<&str> for SettingValue {
    fn from(v: &str) -> Self {
        SettingValue::String(v.to_string())
    }
}

/// Shared tagged color value used by settings and molecular atoms.
pub use patinae_color::ColorValue;

/// Named value variants for settings with a fixed set of allowed values.
/// Each pair is (display_name, setting_value).
pub type ValueHints = &'static [(&'static str, SettingValue)];

fn empty_hints() -> ValueHints {
    &[]
}

/// Metadata for a setting definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Setting {
    /// Unique identifier for the setting (stable across versions for session compatibility)
    pub id: u16,
    /// Name of the setting (e.g., "sphere_scale")
    pub name: &'static str,
    /// Type of the setting
    pub setting_type: SettingType,
    /// Level at which this setting can be applied
    pub level: SettingLevel,
    /// Default value
    pub default: SettingValue,
    /// Minimum value (for numeric types)
    pub min: Option<f32>,
    /// Maximum value (for numeric types)
    pub max: Option<f32>,
    /// Named string aliases for integer values (e.g., "classic" = 0, "skripkin" = 1).
    /// Empty slice for settings without named variants.
    #[serde(skip, default = "empty_hints")]
    pub value_hints: ValueHints,
}

impl Setting {
    /// Create a new blank/unused setting
    pub const fn new_blank(id: u16, name: &'static str) -> Self {
        Setting {
            id,
            name,
            setting_type: SettingType::Blank,
            level: SettingLevel::Unused,
            default: SettingValue::Int(0),
            min: None,
            max: None,
            value_hints: &[],
        }
    }

    /// Create a new boolean setting
    pub const fn new_bool(id: u16, name: &'static str, level: SettingLevel, default: bool) -> Self {
        Setting {
            id,
            name,
            setting_type: SettingType::Bool,
            level,
            default: SettingValue::Bool(default),
            min: Some(0.0),
            max: Some(1.0),
            value_hints: &[],
        }
    }

    /// Create a new integer setting
    pub const fn new_int(
        id: u16,
        name: &'static str,
        level: SettingLevel,
        default: i32,
        min: Option<i32>,
        max: Option<i32>,
    ) -> Self {
        Setting {
            id,
            name,
            setting_type: SettingType::Int,
            level,
            default: SettingValue::Int(default),
            min: match min {
                Some(v) => Some(v as f32),
                None => None,
            },
            max: match max {
                Some(v) => Some(v as f32),
                None => None,
            },
            value_hints: &[],
        }
    }

    /// Create a new float setting
    pub const fn new_float(
        id: u16,
        name: &'static str,
        level: SettingLevel,
        default: f32,
        min: Option<f32>,
        max: Option<f32>,
    ) -> Self {
        Setting {
            id,
            name,
            setting_type: SettingType::Float,
            level,
            default: SettingValue::Float(default),
            min,
            max,
            value_hints: &[],
        }
    }

    /// Create a new float3 setting
    pub const fn new_float3(
        id: u16,
        name: &'static str,
        level: SettingLevel,
        default: [f32; 3],
    ) -> Self {
        Setting {
            id,
            name,
            setting_type: SettingType::Float3,
            level,
            default: SettingValue::Float3(default),
            min: None,
            max: None,
            value_hints: &[],
        }
    }

    /// Create a new color setting
    pub const fn new_color(
        id: u16,
        name: &'static str,
        level: SettingLevel,
        default: ColorValue,
    ) -> Self {
        Setting {
            id,
            name,
            setting_type: SettingType::Color,
            level,
            default: SettingValue::Color(default),
            min: None,
            max: None,
            value_hints: &[],
        }
    }

    /// Create a new string setting
    /// Note: Due to const fn limitations, use `new_string_with_default` for runtime initialization
    pub const fn new_string(
        id: u16,
        name: &'static str,
        level: SettingLevel,
        _default: &'static str, // Unused in const context, see new_string_with_default
    ) -> Self {
        Setting {
            id,
            name,
            setting_type: SettingType::String,
            level,
            // Note: We can't use String in const fn, so we use a workaround
            // The actual default will be set during initialization
            default: SettingValue::Int(0), // Placeholder, actual default handled specially
            min: None,
            max: None,
            value_hints: &[],
        }
    }

    /// Create a new string setting with an actual default value (non-const)
    pub fn new_string_with_default(
        id: u16,
        name: &'static str,
        level: SettingLevel,
        default: &str,
    ) -> Self {
        Setting {
            id,
            name,
            setting_type: SettingType::String,
            level,
            default: SettingValue::String(default.to_string()),
            min: None,
            max: None,
            value_hints: &[],
        }
    }

    /// Check if this setting has min/max constraints
    pub fn has_range(&self) -> bool {
        self.min.is_some() && self.max.is_some() && self.min != self.max
    }

    /// Check if a value is within the valid range for this setting
    pub fn is_in_range(&self, value: f32) -> bool {
        match (self.min, self.max) {
            (Some(min), Some(max)) if min != max => value >= min && value <= max,
            _ => true,
        }
    }

    /// Clamp a value to the valid range for this setting
    pub fn clamp(&self, value: f32) -> f32 {
        match (self.min, self.max) {
            (Some(min), Some(max)) if min != max => value.clamp(min, max),
            _ => value,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn obsolete_object_color_fields_are_rejected() {
        assert!(serde_json::from_str::<crate::groups::StickOverrides>(
            r#"{"radius":null,"transparency":null,"valence":null,"color":33554435}"#
        )
        .is_err());
    }

    #[test]
    fn tagged_color_roundtrips_without_palette_or_scheme_collisions() {
        for color in [
            ColorValue::rgb(0, 0, 2),
            ColorValue::palette(2),
            ColorValue::scheme(2),
            ColorValue::INHERIT,
            ColorValue::palette(0xffffff),
            ColorValue::scheme(0xffffff),
            ColorValue::from_bits(0xfe123456),
            ColorValue::from_bits(0xffffffff),
            ColorValue::from_bits(0x80000000),
        ] {
            let value = SettingValue::Color(color);
            let json = serde_json::to_string(&value).unwrap();
            assert_eq!(serde_json::from_str::<SettingValue>(&json).unwrap(), value);
        }
        for old in ["-1", "2", r#"{"Named":2}"#, r#""BySS""#] {
            assert!(serde_json::from_str::<ColorValue>(old).is_err());
        }
    }

    #[test]
    fn test_setting_type_display() {
        assert_eq!(format!("{}", SettingType::Bool), "bool");
        assert_eq!(format!("{}", SettingType::Float3), "float3");
    }

    #[test]
    fn test_setting_level_mask() {
        assert!(SettingLevel::Object.check_mask(0x01));
        assert!(!SettingLevel::Object.check_mask(0x02));
    }

    #[test]
    fn test_value_coercion() {
        let int_val = SettingValue::Int(42);
        assert_eq!(int_val.as_float(), Some(42.0));
        assert_eq!(int_val.as_bool(), Some(true));

        let bool_val = SettingValue::Bool(false);
        assert_eq!(bool_val.as_int(), Some(0));
    }

    #[test]
    fn test_type_compatibility() {
        let int_val = SettingValue::Int(42);
        assert!(int_val.is_compatible_with(SettingType::Int));
        assert!(int_val.is_compatible_with(SettingType::Float));
        assert!(int_val.is_compatible_with(SettingType::Bool));
        assert!(!int_val.is_compatible_with(SettingType::String));
    }

    #[test]
    fn test_setting_range() {
        let setting = Setting::new_int(1, "test", SettingLevel::Global, 5, Some(0), Some(10));
        assert!(setting.has_range());
        assert!(setting.is_in_range(5.0));
        assert!(!setting.is_in_range(15.0));
        assert_eq!(setting.clamp(15.0), 10.0);
    }
}
