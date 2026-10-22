//! Unified command-facing setting access.
//!
//! This module bridges built-in settings from `patinae-settings` and dynamic
//! plugin settings registered with the command executor.

use patinae_scene::Object;
use patinae_settings::{
    registry, DynamicSettingDescriptor, SettingDescriptor, SettingType, SettingValue,
    SideEffectCategory,
};

use crate::command::{DynamicSettingEntry, DynamicSettingRegistry, ViewerLike};
use crate::error::{CmdError, CmdResult};

/// Origin of a resolved command setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingSource {
    /// Built-in setting from the static settings registry.
    BuiltIn,
    /// Dynamic setting registered by a plugin.
    Dynamic,
}

/// A command setting resolved by name.
///
/// Built-in settings take precedence over dynamic settings with the same name.
/// Dynamic settings clone cheaply because their stores are held behind shared
/// handles.
#[derive(Clone)]
pub enum ResolvedSetting {
    /// Built-in setting descriptor.
    BuiltIn(&'static SettingDescriptor),
    /// Dynamic plugin setting entry.
    Dynamic(DynamicSettingEntry),
}

impl ResolvedSetting {
    /// Resolve a setting by command name.
    #[must_use]
    pub fn lookup(name: &str, dynamic_settings: Option<&DynamicSettingRegistry>) -> Option<Self> {
        if let Some(desc) = registry::lookup_by_name(name) {
            return Some(Self::BuiltIn(desc));
        }
        dynamic_settings
            .and_then(|registry| registry.lookup(name).cloned())
            .map(Self::Dynamic)
    }

    /// Return the setting source.
    #[must_use]
    pub fn source(&self) -> SettingSource {
        match self {
            Self::BuiltIn(_) => SettingSource::BuiltIn,
            Self::Dynamic(_) => SettingSource::Dynamic,
        }
    }

    /// Return the setting name.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::BuiltIn(desc) => desc.name,
            Self::Dynamic(entry) => entry.descriptor.name.as_str(),
        }
    }

    /// Return the setting value type.
    #[must_use]
    pub fn setting_type(&self) -> SettingType {
        match self {
            Self::BuiltIn(desc) => desc.setting_type,
            Self::Dynamic(entry) => entry.descriptor.setting_type,
        }
    }

    /// Return the numeric minimum constraint.
    #[must_use]
    pub fn min(&self) -> Option<f32> {
        match self {
            Self::BuiltIn(desc) => desc.min,
            Self::Dynamic(entry) => entry.descriptor.min,
        }
    }

    /// Return the numeric maximum constraint.
    #[must_use]
    pub fn max(&self) -> Option<f32> {
        match self {
            Self::BuiltIn(desc) => desc.max,
            Self::Dynamic(entry) => entry.descriptor.max,
        }
    }

    /// Return side effects declared for this setting.
    #[must_use]
    pub fn side_effects(&self) -> &[SideEffectCategory] {
        match self {
            Self::BuiltIn(desc) => desc.side_effects,
            Self::Dynamic(entry) => &entry.descriptor.side_effects,
        }
    }

    /// Return the built-in descriptor, when this is a built-in setting.
    #[must_use]
    pub fn built_in_descriptor(&self) -> Option<&'static SettingDescriptor> {
        match self {
            Self::BuiltIn(desc) => Some(desc),
            Self::Dynamic(_) => None,
        }
    }

    /// Return the dynamic descriptor, when this is a plugin setting.
    #[must_use]
    pub fn dynamic_descriptor(&self) -> Option<&DynamicSettingDescriptor> {
        match self {
            Self::BuiltIn(_) => None,
            Self::Dynamic(entry) => Some(&entry.descriptor),
        }
    }

    /// Return whether this setting supports object overrides.
    #[must_use]
    pub fn is_object_overridable(&self) -> bool {
        match self {
            Self::BuiltIn(desc) => desc.is_object_overridable(),
            Self::Dynamic(entry) => entry.descriptor.object_overridable,
        }
    }

    /// Return whether this setting has named values.
    #[must_use]
    pub fn has_value_hints(&self) -> bool {
        match self {
            Self::BuiltIn(desc) => desc.has_value_hints(),
            Self::Dynamic(entry) => entry.descriptor.has_value_hints(),
        }
    }

    /// Resolve a named value hint.
    #[must_use]
    pub fn resolve_hint(&self, name: &str) -> Option<SettingValue> {
        match self {
            Self::BuiltIn(desc) => desc.resolve_hint(name).cloned(),
            Self::Dynamic(entry) => entry.descriptor.resolve_hint(name).cloned(),
        }
    }

    /// Return all named value hints.
    #[must_use]
    pub fn hint_names(&self) -> Vec<&str> {
        match self {
            Self::BuiltIn(desc) => desc.hint_names().collect(),
            Self::Dynamic(entry) => entry.descriptor.hint_names().collect(),
        }
    }

    /// Return the display name for a setting value.
    #[must_use]
    pub fn hint_name<'a>(&'a self, value: &SettingValue) -> Option<&'a str> {
        match self {
            Self::BuiltIn(desc) => desc.hint_name(value),
            Self::Dynamic(entry) => entry.descriptor.hint_name(value),
        }
    }

    /// Format a raw setting value for command output.
    #[must_use]
    pub fn format_value(value: &SettingValue, palette: &patinae_color::NamedPalette) -> String {
        match value {
            SettingValue::Bool(value) => {
                if *value {
                    "on".to_string()
                } else {
                    "off".to_string()
                }
            }
            SettingValue::Int(value) => value.to_string(),
            SettingValue::Float(value) => format!("{value:.6}"),
            SettingValue::Float3(value) => {
                format!("[{:.3}, {:.3}, {:.3}]", value[0], value[1], value[2])
            }
            SettingValue::Color(value) => palette.format_color(*value),
            SettingValue::String(value) => value.clone(),
        }
    }

    /// Format a value using value-hint names when available.
    #[must_use]
    pub fn format_display(
        &self,
        value: &SettingValue,
        palette: &patinae_color::NamedPalette,
    ) -> String {
        if let SettingValue::Color(color) = value {
            return palette.format_color(*color);
        }
        self.hint_name(value)
            .map(str::to_string)
            .unwrap_or_else(|| Self::format_value(value, palette))
    }

    /// Format a value with command-style type and hint metadata.
    #[must_use]
    pub fn format_report(
        &self,
        value: &SettingValue,
        palette: &patinae_color::NamedPalette,
    ) -> String {
        let raw = Self::format_value(value, palette);
        if let Some(hint_name) = self.hint_name(value) {
            format!("{} = {} ({hint_name})", self.name(), raw)
        } else {
            format!("{} ({}) = {}", self.name(), self.setting_type(), raw)
        }
    }

    /// Parse a command value for this setting.
    ///
    /// This applies value hints before falling back to type-based parsing.
    ///
    /// # Errors
    /// Returns an error when the input cannot be parsed for this setting.
    pub fn parse_value(&self, value: &str) -> CmdResult<SettingValue> {
        if self.has_value_hints() {
            if let Some(value) = self.resolve_hint(value) {
                return Ok(value);
            }
            if value.parse::<f64>().is_err() {
                let names = self.hint_names();
                return Err(CmdError::invalid_arg(
                    "value",
                    format!(
                        "Unknown value '{}' for {}. Use {}",
                        value,
                        self.name(),
                        names.join("/")
                    ),
                ));
            }
        }

        parse_setting_value(value, self.setting_type())
    }

    /// Return the descriptor default value.
    #[must_use]
    pub fn default_value(&self) -> SettingValue {
        match self {
            Self::BuiltIn(desc) => desc.get(&patinae_settings::Settings::default()),
            Self::Dynamic(entry) => entry.descriptor.default.clone(),
        }
    }

    /// Read the global value.
    ///
    /// Dynamic settings return an error when their shared store is poisoned.
    pub fn global_value<V: ViewerLike + ?Sized>(&self, viewer: &V) -> Result<SettingValue, String> {
        match self {
            Self::BuiltIn(desc) => Ok(desc.get(viewer.settings())),
            Self::Dynamic(entry) => {
                let store = entry.store.read().map_err(|e| e.to_string())?;
                Ok(store
                    .get(self.name())
                    .cloned()
                    .unwrap_or_else(|| entry.descriptor.default.clone()))
            }
        }
    }

    /// Read the global value for convenience APIs.
    ///
    /// If a dynamic setting store is poisoned, this returns the descriptor
    /// default instead of surfacing an error.
    #[must_use]
    pub fn global_value_lossy<V: ViewerLike + ?Sized>(&self, viewer: &V) -> SettingValue {
        match self {
            Self::BuiltIn(desc) => desc.get(viewer.settings()),
            Self::Dynamic(entry) => {
                let default = entry.descriptor.default.clone();
                let Ok(store) = entry.store.read() else {
                    return default;
                };
                store.get(self.name()).cloned().unwrap_or(default)
            }
        }
    }

    /// Read the global value from a read-only settings snapshot.
    ///
    /// If a dynamic setting store is poisoned, this returns the descriptor
    /// default instead of surfacing an error.
    #[must_use]
    pub fn global_value_from_settings(
        &self,
        settings: &patinae_settings::Settings,
    ) -> SettingValue {
        match self {
            Self::BuiltIn(desc) => desc.get(settings),
            Self::Dynamic(entry) => {
                let default = entry.descriptor.default.clone();
                let Ok(store) = entry.store.read() else {
                    return default;
                };
                store.get(self.name()).cloned().unwrap_or(default)
            }
        }
    }

    /// Write the global value.
    ///
    /// # Errors
    /// Returns an error when validation fails or a dynamic store is poisoned.
    pub fn set_global<V: ViewerLike + ?Sized>(
        &self,
        viewer: &mut V,
        value: SettingValue,
    ) -> Result<(), String> {
        match self {
            Self::BuiltIn(desc) => desc
                .set(viewer.settings_mut(), value)
                .map_err(|e| e.to_string()),
            Self::Dynamic(entry) => {
                entry
                    .store
                    .write()
                    .map_err(|e| e.to_string())?
                    .set(self.name(), value);
                Ok(())
            }
        }
    }

    /// Reset the global value to its descriptor default.
    ///
    /// # Errors
    /// Returns an error when validation fails or a dynamic store is poisoned.
    pub fn unset_global<V: ViewerLike + ?Sized>(
        &self,
        viewer: &mut V,
    ) -> Result<SettingValue, String> {
        let default = self.default_value();
        match self {
            Self::BuiltIn(desc) => desc
                .set(viewer.settings_mut(), default.clone())
                .map_err(|e| e.to_string())?,
            Self::Dynamic(entry) => {
                entry
                    .store
                    .write()
                    .map_err(|e| e.to_string())?
                    .remove(self.name());
            }
        }
        Ok(default)
    }

    /// Read a dynamic object override by object name.
    ///
    /// Built-in object overrides are stored on molecule objects and are
    /// therefore handled by command code that has a molecule reference.
    pub fn object_value(&self, object_name: &str) -> Result<Option<SettingValue>, String> {
        match self {
            Self::BuiltIn(_) => Ok(None),
            Self::Dynamic(entry) => {
                let store = entry.store.read().map_err(|e| e.to_string())?;
                Ok(store.get_object(object_name, self.name()).cloned())
            }
        }
    }

    /// Read a built-in object override.
    ///
    /// Dynamic object overrides are stored in the dynamic setting store.
    #[must_use]
    pub fn built_in_object_value(
        &self,
        object: &patinae_scene::MoleculeObject,
    ) -> Option<SettingValue> {
        match self {
            Self::BuiltIn(desc) => object
                .overrides()
                .and_then(|overrides| desc.get_override(overrides)),
            Self::Dynamic(_) => None,
        }
    }

    /// Write a dynamic object override by object name.
    ///
    /// Returns `false` for built-in settings because their object overrides are
    /// stored on molecule objects.
    pub fn set_object_value(&self, object_name: &str, value: SettingValue) -> Result<bool, String> {
        match self {
            Self::BuiltIn(_) => Ok(false),
            Self::Dynamic(entry) => {
                entry.store.write().map_err(|e| e.to_string())?.set_object(
                    object_name,
                    self.name(),
                    value,
                );
                Ok(true)
            }
        }
    }

    /// Remove a dynamic object override by object name.
    ///
    /// Returns `false` for built-in settings because their object overrides are
    /// stored on molecule objects.
    pub fn unset_object_value(&self, object_name: &str) -> Result<bool, String> {
        match self {
            Self::BuiltIn(_) => Ok(false),
            Self::Dynamic(entry) => {
                entry
                    .store
                    .write()
                    .map_err(|e| e.to_string())?
                    .remove_object(object_name, self.name());
                Ok(true)
            }
        }
    }

    /// Validate numeric range constraints.
    ///
    /// # Errors
    /// Returns an error message when `value` is outside the descriptor range.
    pub fn validate_range(&self, value: &SettingValue) -> Result<(), String> {
        let (Some(min), Some(max)) = (self.min(), self.max()) else {
            return Ok(());
        };
        let Some(fval) = value.as_float() else {
            return Ok(());
        };
        if fval < min || fval > max {
            return Err(format!(
                "Value {} out of range [{}, {}] for {}",
                fval,
                min,
                max,
                self.name()
            ));
        }
        Ok(())
    }
}

/// Parse a command string into a setting value.
fn parse_setting_value(value: &str, setting_type: SettingType) -> CmdResult<SettingValue> {
    match setting_type {
        SettingType::Bool => match value.to_lowercase().as_str() {
            "1" | "on" | "true" | "yes" => Ok(SettingValue::Bool(true)),
            "0" | "off" | "false" | "no" => Ok(SettingValue::Bool(false)),
            _ => Err(CmdError::invalid_arg(
                "value",
                format!(
                    "Invalid boolean value '{}'. Use 1/0, on/off, true/false, or yes/no",
                    value
                ),
            )),
        },
        SettingType::Int => {
            if let Ok(parsed) = value.parse::<i32>() {
                return Ok(SettingValue::Int(parsed));
            }
            match value.to_lowercase().as_str() {
                "on" | "true" | "yes" => Ok(SettingValue::Int(1)),
                "off" | "false" | "no" => Ok(SettingValue::Int(0)),
                _ => Err(CmdError::invalid_arg(
                    "value",
                    format!(
                        "Invalid integer value '{}'. Expected a number or on/off",
                        value
                    ),
                )),
            }
        }
        SettingType::Float => {
            if let Ok(parsed) = value.parse::<f32>() {
                return Ok(SettingValue::Float(parsed));
            }
            match value.to_lowercase().as_str() {
                "on" | "true" | "yes" => Ok(SettingValue::Float(1.0)),
                "off" | "false" | "no" => Ok(SettingValue::Float(0.0)),
                _ => Err(CmdError::invalid_arg(
                    "value",
                    format!(
                        "Invalid float value '{}'. Expected a number or on/off",
                        value
                    ),
                )),
            }
        }
        SettingType::Float3 => {
            let cleaned = value.trim_start_matches('[').trim_end_matches(']');
            let parts: Vec<&str> = cleaned.split(',').map(str::trim).collect();
            if parts.len() != 3 {
                return Err(CmdError::invalid_arg(
                    "value",
                    format!(
                        "Invalid float3 value '{}'. Expected [x, y, z] or x,y,z format",
                        value
                    ),
                ));
            }
            let x = parts[0]
                .parse::<f32>()
                .map_err(|_| CmdError::invalid_arg("value", "Invalid x component"))?;
            let y = parts[1]
                .parse::<f32>()
                .map_err(|_| CmdError::invalid_arg("value", "Invalid y component"))?;
            let z = parts[2]
                .parse::<f32>()
                .map_err(|_| CmdError::invalid_arg("value", "Invalid z component"))?;
            Ok(SettingValue::Float3([x, y, z]))
        }
        SettingType::Color => value
            .parse::<patinae_color::ColorValue>()
            .map(SettingValue::Color)
            .map_err(|error| CmdError::invalid_arg("value", error)),
        SettingType::String => Ok(SettingValue::String(value.to_string())),
        SettingType::Blank => Err(CmdError::invalid_arg(
            "value",
            "Cannot set a blank/unused setting",
        )),
    }
}
