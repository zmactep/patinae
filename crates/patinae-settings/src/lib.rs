//! Settings system.
//!
//! This crate provides configuration and settings management.
//! It supports global settings, per-object settings, per-atom/bond settings,
//! and hierarchical setting inheritance.
//!
//! # Architecture
//!
//! The settings system has 813 settings organized into a hierarchical inheritance
//! model (indices stable for PSE format compatibility):
//!
//! ```text
//! global < object < object-state
//!                   object-state < atom < atom-state
//!                   object-state < bond < bond-state
//! ```
//!
//! # Setting Types
//!
//! - `Bool` - Boolean values
//! - `Int` - Integer values
//! - `Float` - Single-precision floats
//! - `Float3` - Three-component float vectors (positions, directions)
//! - `ColorValue` - Tagged RGB, palette, scheme, and inheritance values
//! - `String` - String values
//!
//! # Example
//!
//! ```rust
//! use patinae_settings::{GlobalSettings, SettingValue, id};
//!
//! let mut settings = GlobalSettings::new();
//!
//! // Set a float setting
//! settings.set_float(id::ambient, 0.2).unwrap();
//!
//! // Get with type-safe getter
//! let ambient = settings.get_float(id::ambient);
//! assert_eq!(ambient, 0.2);
//! ```

// enums must come first so that `impl_setting_enum!` is available to later modules
pub mod enums;

// macros must come before groups (groups use define_settings_group!)
pub mod macros;

#[cfg(not(target_arch = "wasm32"))]
pub mod config;
pub mod groups;
pub mod legacy;
pub mod overrides;
pub mod paths;
pub mod recent_files;
pub mod registry;

mod definitions;
pub mod dynamic;
mod error;
mod setting;
pub mod shading_mode;
mod side_effects;
mod store;

// Re-export main types
pub use definitions::{get_setting, get_setting_id, setting_names, SETTINGS, SETTING_COUNT};
pub use enums::{
    DssAlgorithm, MouseSelectionMode, RenderMemoryProfileSetting, SettingEnum, SurfaceMode,
    ThemeMode,
};
pub use error::SettingError;
pub use setting::{ColorValue, Setting, SettingLevel, SettingType, SettingValue};
pub use shading_mode::ShadingMode;
pub use side_effects::SideEffectCategory;
pub use store::{GlobalSettings, SerializedSetting, UniqueId, UniqueSettings};

// New typed settings system re-exports
pub use dynamic::{DynamicSettingDescriptor, DynamicSettingStore, SharedSettingStore};
pub use groups::Settings;
pub use macros::{FieldDescriptor, Merge, SettingDescriptor, SettingKind};
pub use overrides::{ObjectOverrides, ResolvedSettings};

/// Setting ID constants
pub mod id {
    pub use crate::definitions::id::*;
}

/// Re-export commonly used types for convenience
pub mod prelude {
    pub use crate::definitions::id;
    pub use crate::{GlobalSettings, SettingError, SettingType, SettingValue, UniqueSettings};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_usage() {
        let mut settings = GlobalSettings::new();

        // Test setting and getting values
        settings.set_float(id::ambient, 0.25).unwrap();
        assert_eq!(settings.get_float(id::ambient), 0.25);

        // Test reset
        settings.reset(id::ambient).unwrap();
        assert_eq!(settings.get_float(id::ambient), 0.14); // default
    }

    #[test]
    fn test_serialization_roundtrip() {
        let mut settings = GlobalSettings::new();
        settings.set_float(id::ambient, 0.3).unwrap();
        settings.set_bool(id::orthoscopic, true).unwrap();

        let list = settings.to_session_list();
        assert!(!list.is_empty());

        let mut restored = GlobalSettings::new();
        restored.from_session_list(&list);

        assert_eq!(restored.get_float(id::ambient), 0.3);
        assert!(restored.get_bool(id::orthoscopic));
    }
}
