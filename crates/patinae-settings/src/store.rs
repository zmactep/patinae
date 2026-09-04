//! Setting storage and retrieval with hierarchical inheritance

use serde::{Deserialize, Serialize};

use ahash::AHashMap;

use crate::definitions::{get_setting, SETTINGS, SETTING_COUNT};
use crate::error::SettingError;
use crate::setting::SettingValue;

// =============================================================================
// Global Settings Store
// =============================================================================

/// Global settings store - holds session-wide settings
/// Uses sparse HashMap storage for memory efficiency when most settings use defaults
#[derive(Debug, Serialize, Deserialize)]
pub struct GlobalSettings {
    /// Current values (only stores non-default values)
    values: AHashMap<u16, SettingValue>,
    /// Tracks which settings have been modified
    changed: AHashMap<u16, bool>,
}

impl GlobalSettings {
    /// Create a new global settings store with defaults
    pub fn new() -> Self {
        GlobalSettings {
            values: AHashMap::new(),
            changed: AHashMap::new(),
        }
    }

    /// Get a setting value, returning the default if not explicitly set
    pub fn get(&self, id: u16) -> Option<SettingValue> {
        if id as usize >= SETTING_COUNT {
            return None;
        }

        self.values
            .get(&id)
            .cloned()
            .or_else(|| get_setting(id).map(|s| s.default.clone()))
    }

    /// Check if a setting is explicitly defined (not using default)
    pub fn is_defined(&self, id: u16) -> bool {
        self.values.contains_key(&id)
    }

    /// Get only if explicitly defined (not default)
    pub fn get_if_defined(&self, id: u16) -> Option<&SettingValue> {
        self.values.get(&id)
    }

    /// Set a setting value
    pub fn set(&mut self, id: u16, value: SettingValue) -> Result<(), SettingError> {
        if id as usize >= SETTING_COUNT {
            return Err(SettingError::NotFound(format!("id:{}", id)));
        }

        let setting =
            get_setting(id).ok_or_else(|| SettingError::NotFound(format!("id:{}", id)))?;

        // Type check
        if !value.is_compatible_with(setting.setting_type) {
            return Err(SettingError::TypeMismatch {
                expected: setting.setting_type.to_string().leak(),
                actual: value.setting_type().to_string().leak(),
            });
        }

        // Range check for numeric types
        if let (Some(min), Some(max)) = (setting.min, setting.max) {
            if min != max {
                if let Some(v) = value.as_float() {
                    if v < min || v > max {
                        return Err(SettingError::InvalidValue {
                            name: setting.name.to_string(),
                            reason: format!("value {} is outside range [{}, {}]", v, min, max),
                        });
                    }
                }
            }
        }

        self.values.insert(id, value);
        self.changed.insert(id, true);
        Ok(())
    }

    /// Unset a setting (revert to default)
    pub fn unset(&mut self, id: u16) -> bool {
        let was_set = self.values.remove(&id).is_some();
        if was_set {
            self.changed.insert(id, true);
        }
        was_set
    }

    /// Reset a setting to its default value
    pub fn reset(&mut self, id: u16) -> Result<(), SettingError> {
        self.unset(id);
        Ok(())
    }

    /// Reset all settings to defaults
    pub fn reset_all(&mut self) {
        for id in self.values.keys().copied().collect::<Vec<_>>() {
            self.changed.insert(id, true);
        }
        self.values.clear();
    }

    /// Check if a setting has changed since last check
    pub fn check_changed(&mut self, id: u16) -> bool {
        self.changed.remove(&id).unwrap_or(false)
    }

    /// Get all changed setting IDs and clear changed flags
    pub fn get_changed_and_clear(&mut self) -> Vec<u16> {
        let changed: Vec<u16> = self.changed.keys().copied().collect();
        self.changed.clear();
        changed
    }

    /// Iterate over all defined (non-default) settings
    pub fn iter_defined(&self) -> impl Iterator<Item = (u16, &SettingValue)> {
        self.values.iter().map(|(&id, val)| (id, val))
    }

    // =========================================================================
    // Type-Safe Getters
    // =========================================================================

    /// Get a boolean setting
    pub fn get_bool(&self, id: u16) -> bool {
        self.get(id).and_then(|v| v.as_bool()).unwrap_or(false)
    }

    /// Get an integer setting
    pub fn get_int(&self, id: u16) -> i32 {
        self.get(id).and_then(|v| v.as_int()).unwrap_or(0)
    }

    /// Get a float setting (with automatic int-to-float coercion)
    pub fn get_float(&self, id: u16) -> f32 {
        self.get(id).and_then(|v| v.as_float()).unwrap_or(0.0)
    }

    /// Get a float3 setting
    pub fn get_float3(&self, id: u16) -> [f32; 3] {
        self.get(id)
            .and_then(|v| v.as_float3())
            .unwrap_or([0.0, 0.0, 0.0])
    }

    /// Get a color index setting
    pub fn get_color(&self, id: u16) -> i32 {
        self.get(id).and_then(|v| v.as_color()).unwrap_or(-1)
    }

    /// Get a string setting
    pub fn get_string(&self, id: u16) -> String {
        self.get(id)
            .and_then(|v| v.as_string().map(|s| s.to_string()))
            .unwrap_or_default()
    }

    // =========================================================================
    // Type-Safe Setters
    // =========================================================================

    /// Set a boolean setting
    pub fn set_bool(&mut self, id: u16, value: bool) -> Result<(), SettingError> {
        self.set(id, SettingValue::Bool(value))
    }

    /// Set an integer setting
    pub fn set_int(&mut self, id: u16, value: i32) -> Result<(), SettingError> {
        self.set(id, SettingValue::Int(value))
    }

    /// Set a float setting
    pub fn set_float(&mut self, id: u16, value: f32) -> Result<(), SettingError> {
        self.set(id, SettingValue::Float(value))
    }

    /// Set a float3 setting
    pub fn set_float3(&mut self, id: u16, value: [f32; 3]) -> Result<(), SettingError> {
        self.set(id, SettingValue::Float3(value))
    }

    /// Set a color setting
    pub fn set_color(&mut self, id: u16, value: i32) -> Result<(), SettingError> {
        self.set(id, SettingValue::Color(value))
    }

    /// Set a string setting
    pub fn set_string(&mut self, id: u16, value: impl Into<String>) -> Result<(), SettingError> {
        self.set(id, SettingValue::String(value.into()))
    }
}

impl Default for GlobalSettings {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for GlobalSettings {
    fn clone(&self) -> Self {
        GlobalSettings {
            values: self.values.clone(),
            changed: AHashMap::new(), // Don't clone changed state
        }
    }
}

// =============================================================================
// Serialization Support
// =============================================================================

/// Settings that should not be saved in sessions (system-dependent)
const SESSION_BLACKLIST: &[u16] = &[
    // Add system-dependent settings here that shouldn't persist
    // e.g., internal_gui_width, window positions, etc.
];

/// Check if a setting should be excluded from sessions
fn is_session_blacklisted(id: u16) -> bool {
    SESSION_BLACKLIST.contains(&id)
}

/// Serialized setting entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedSetting {
    pub id: u16,
    pub value: SettingValue,
}

impl GlobalSettings {
    /// Serialize settings to a list for session saving
    /// Only includes explicitly set (non-default) settings
    pub fn to_session_list(&self) -> Vec<SerializedSetting> {
        self.values
            .iter()
            .filter_map(|(&id, value)| {
                if is_session_blacklisted(id) {
                    return None;
                }
                Some(SerializedSetting {
                    id,
                    value: value.clone(),
                })
            })
            .collect()
    }

    /// Restore settings from a session list
    pub fn from_session_list(&mut self, list: &[SerializedSetting]) {
        for entry in list {
            if !is_session_blacklisted(entry.id) && (entry.id as usize) < SETTING_COUNT {
                self.values.insert(entry.id, entry.value.clone());
                self.changed.insert(entry.id, true);
            }
        }
    }

    /// Create a new GlobalSettings from a session list
    pub fn new_from_session_list(list: &[SerializedSetting]) -> Self {
        let mut settings = Self::new();
        settings.from_session_list(list);
        settings
    }

    /// Export all settings (including defaults) for debugging
    pub fn export_all(&self) -> Vec<(u16, String, SettingValue)> {
        SETTINGS
            .iter()
            .map(|s| {
                let value = self.get(s.id).unwrap_or_else(|| s.default.clone());
                (s.id, s.name.to_string(), value)
            })
            .collect()
    }
}

impl UniqueSettings {
    /// Serialize unique settings to a list
    pub fn to_session_list(&self) -> Vec<(UniqueId, u16, SettingValue)> {
        let mut result = Vec::new();
        for (&unique_id, &first_offset) in &self.id_to_offset {
            let mut offset = first_offset;
            loop {
                if let Some(entry) = &self.entries[offset] {
                    if !is_session_blacklisted(entry.setting_id) {
                        result.push((unique_id, entry.setting_id, entry.value.clone()));
                    }
                }
                offset = self.next[offset];
                if offset == 0 {
                    break;
                }
            }
        }
        result
    }

    /// Restore unique settings from a session list
    pub fn from_session_list(&mut self, list: &[(UniqueId, u16, SettingValue)]) {
        for (unique_id, setting_id, value) in list {
            if !is_session_blacklisted(*setting_id) {
                // We use a simplified set that ignores errors
                let _ = self.set(*unique_id, *setting_id, value.clone());
            }
        }
    }
}

// =============================================================================
// Unique Settings (Per-atom/bond/object-state)
// =============================================================================

/// Unique ID for per-atom/bond settings
pub type UniqueId = i32;

/// Entry in the unique settings storage
#[derive(Debug, Clone, Serialize, Deserialize)]
struct UniqueEntry {
    setting_id: u16,
    value: SettingValue,
}

/// Storage for per-atom/bond settings using a sparse linked-list
/// representation for memory efficiency.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct UniqueSettings {
    /// Map from unique_id to first entry index
    id_to_offset: AHashMap<UniqueId, usize>,
    /// Linked list of entries
    entries: Vec<Option<UniqueEntry>>,
    /// Next links for entries (0 = end of chain)
    next: Vec<usize>,
    /// Free list head
    next_free: usize,
}

impl UniqueSettings {
    /// Create a new unique settings store
    pub fn new() -> Self {
        let initial_capacity = 64;
        let mut settings = UniqueSettings {
            id_to_offset: AHashMap::new(),
            entries: Vec::with_capacity(initial_capacity),
            next: Vec::with_capacity(initial_capacity),
            next_free: 0,
        };
        // Initialize with some free entries
        for i in 0..initial_capacity {
            settings.entries.push(None);
            settings
                .next
                .push(if i + 1 < initial_capacity { i + 1 } else { 0 });
        }
        settings.next_free = 0;
        settings
    }

    /// Expand the storage pool
    fn expand(&mut self) {
        let old_size = self.entries.len();
        let new_size = old_size + old_size / 2 + 16;

        for i in old_size..new_size {
            self.entries.push(None);
            self.next.push(if i + 1 < new_size { i + 1 } else { 0 });
        }
        self.next_free = old_size;
    }

    /// Allocate a new entry
    fn alloc(&mut self) -> usize {
        if self.next_free == 0 && self.entries.iter().all(|e| e.is_some()) {
            self.expand();
        }

        // Find a free slot
        if self.next_free != 0 || self.entries[0].is_none() {
            let offset = self.next_free;
            self.next_free = self.next[offset];
            offset
        } else {
            // Expand and try again
            self.expand();
            let offset = self.next_free;
            self.next_free = self.next[offset];
            offset
        }
    }

    /// Free an entry
    fn free(&mut self, offset: usize) {
        self.entries[offset] = None;
        self.next[offset] = self.next_free;
        self.next_free = offset;
    }

    /// Get a setting value for a unique ID
    pub fn get(&self, unique_id: UniqueId, setting_id: u16) -> Option<&SettingValue> {
        let mut offset = *self.id_to_offset.get(&unique_id)?;

        while offset != 0 || self.entries[offset].is_some() {
            if let Some(entry) = &self.entries[offset] {
                if entry.setting_id == setting_id {
                    return Some(&entry.value);
                }
            }
            offset = self.next[offset];
            if offset == 0 {
                break;
            }
        }
        None
    }

    /// Check if a setting is defined for a unique ID
    pub fn is_defined(&self, unique_id: UniqueId, setting_id: u16) -> bool {
        self.get(unique_id, setting_id).is_some()
    }

    /// Set a setting value for a unique ID
    pub fn set(
        &mut self,
        unique_id: UniqueId,
        setting_id: u16,
        value: SettingValue,
    ) -> Result<(), SettingError> {
        // Validate the setting
        let setting = get_setting(setting_id)
            .ok_or_else(|| SettingError::NotFound(format!("id:{}", setting_id)))?;

        if !value.is_compatible_with(setting.setting_type) {
            return Err(SettingError::TypeMismatch {
                expected: setting.setting_type.to_string().leak(),
                actual: value.setting_type().to_string().leak(),
            });
        }

        // Check if we already have this setting for this unique_id
        if let Some(&first_offset) = self.id_to_offset.get(&unique_id) {
            let mut offset = first_offset;
            loop {
                if let Some(entry) = &mut self.entries[offset] {
                    if entry.setting_id == setting_id {
                        entry.value = value;
                        return Ok(());
                    }
                }
                let next = self.next[offset];
                if next == 0 {
                    // Append to end of chain
                    let new_offset = self.alloc();
                    self.entries[new_offset] = Some(UniqueEntry { setting_id, value });
                    self.next[new_offset] = 0;
                    self.next[offset] = new_offset;
                    return Ok(());
                }
                offset = next;
            }
        } else {
            // New unique_id
            let offset = self.alloc();
            self.entries[offset] = Some(UniqueEntry { setting_id, value });
            self.next[offset] = 0;
            self.id_to_offset.insert(unique_id, offset);
            Ok(())
        }
    }

    /// Unset a setting for a unique ID
    pub fn unset(&mut self, unique_id: UniqueId, setting_id: u16) -> bool {
        let first_offset = match self.id_to_offset.get(&unique_id) {
            Some(&o) => o,
            None => return false,
        };

        let mut prev = 0;
        let mut offset = first_offset;

        loop {
            if let Some(entry) = &self.entries[offset] {
                if entry.setting_id == setting_id {
                    let next = self.next[offset];

                    if prev == 0 {
                        // First entry in chain
                        if next == 0 {
                            // Only entry - remove the whole chain
                            self.id_to_offset.remove(&unique_id);
                        } else {
                            // Update chain head
                            self.id_to_offset.insert(unique_id, next);
                        }
                    } else {
                        // Middle or end of chain
                        self.next[prev] = next;
                    }

                    self.free(offset);
                    return true;
                }
            }

            let next = self.next[offset];
            if next == 0 {
                break;
            }
            prev = offset;
            offset = next;
        }

        false
    }

    /// Remove all settings for a unique ID
    pub fn remove_all(&mut self, unique_id: UniqueId) {
        let first_offset = match self.id_to_offset.remove(&unique_id) {
            Some(o) => o,
            None => return,
        };

        let mut offset = first_offset;
        loop {
            let next = self.next[offset];
            self.free(offset);
            if next == 0 {
                break;
            }
            offset = next;
        }
    }

    /// Get all settings for a unique ID
    pub fn get_all(&self, unique_id: UniqueId) -> Vec<(u16, &SettingValue)> {
        let mut result = Vec::new();
        let first_offset = match self.id_to_offset.get(&unique_id) {
            Some(&o) => o,
            None => return result,
        };

        let mut offset = first_offset;
        loop {
            if let Some(entry) = &self.entries[offset] {
                result.push((entry.setting_id, &entry.value));
            }
            let next = self.next[offset];
            if next == 0 {
                break;
            }
            offset = next;
        }
        result
    }

    /// Clear all unique settings
    pub fn clear(&mut self) {
        self.id_to_offset.clear();
        self.entries.fill(None);
        // Rebuild free list
        for (i, next) in self.next.iter_mut().enumerate() {
            *next = if i + 1 < self.entries.len() { i + 1 } else { 0 };
        }
        self.next_free = 0;
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_global_settings_default() {
        let settings = GlobalSettings::new();
        let value = settings.get(crate::definitions::id::sphere_quality);
        assert!(value.is_some());
    }

    #[test]
    fn test_global_settings_set_get() {
        let mut settings = GlobalSettings::new();
        settings
            .set(crate::definitions::id::ambient, SettingValue::Float(0.5))
            .unwrap();
        let value = settings.get(crate::definitions::id::ambient).unwrap();
        assert_eq!(value.as_float(), Some(0.5));
    }

    #[test]
    fn test_global_settings_reset() {
        let mut settings = GlobalSettings::new();
        settings
            .set(crate::definitions::id::ambient, SettingValue::Float(0.5))
            .unwrap();
        settings.reset(crate::definitions::id::ambient).unwrap();
        let value = settings.get(crate::definitions::id::ambient).unwrap();
        // Should be back to default (0.14)
        assert_eq!(value.as_float(), Some(0.14));
    }

    #[test]
    fn test_unique_settings() {
        let mut unique = UniqueSettings::new();
        unique
            .set(
                1,
                crate::definitions::id::sphere_quality,
                SettingValue::Int(3),
            )
            .unwrap();

        let value = unique.get(1, crate::definitions::id::sphere_quality);
        assert!(value.is_some());
        assert_eq!(value.unwrap().as_int(), Some(3));

        // Different unique_id should not have this setting
        assert!(unique
            .get(2, crate::definitions::id::sphere_quality)
            .is_none());
    }
}
