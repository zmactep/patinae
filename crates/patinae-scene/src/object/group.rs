//! Group object for hierarchical object organization
//!
//! Groups allow logical grouping of objects without geometric data.
//! They propagate visibility and transforms to their children.

use lin_alg::f32::Vec3;
use serde::{Deserialize, Serialize};

use super::{Object, ObjectState, ObjectType};

/// A group object for hierarchical organization
///
/// Groups are container objects that can hold references to other objects.
/// They have no geometry of their own but can:
/// - Enable/disable all children recursively
/// - Compute the combined bounding box of children
/// - Be expanded/collapsed in the UI
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupObject {
    /// Group name
    name: String,
    /// Visual state (enabled, color, etc.)
    state: ObjectState,
    /// Names of child objects (can include other groups)
    children: Vec<String>,
    /// Whether the group is expanded in the UI
    open: bool,
}

impl GroupObject {
    /// Create a new empty group
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            state: ObjectState::default(),
            children: Vec::new(),
            open: true,
        }
    }

    /// Create a group with initial children
    pub fn with_children(name: &str, children: Vec<String>) -> Self {
        Self {
            name: name.to_string(),
            state: ObjectState::default(),
            children,
            open: true,
        }
    }

    /// Get the list of child object names
    pub fn children(&self) -> &[String] {
        &self.children
    }

    /// Get mutable access to the children list
    pub fn children_mut(&mut self) -> &mut Vec<String> {
        &mut self.children
    }

    /// Add a child to this group
    pub fn add_child(&mut self, name: String) {
        if !self.children.contains(&name) {
            self.children.push(name);
        }
    }

    /// Remove a child from this group
    pub fn remove_child(&mut self, name: &str) -> bool {
        if let Some(pos) = self.children.iter().position(|n| n == name) {
            self.children.remove(pos);
            true
        } else {
            false
        }
    }

    /// Check if this group contains a specific child
    pub fn contains_child(&self, name: &str) -> bool {
        self.children.iter().any(|n| n == name)
    }

    /// Check if the group is empty
    pub fn is_empty(&self) -> bool {
        self.children.is_empty()
    }

    /// Get the number of children
    pub fn len(&self) -> usize {
        self.children.len()
    }

    /// Check if the group is open (expanded in UI)
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Set whether the group is open (expanded in UI)
    pub fn set_open(&mut self, open: bool) {
        self.open = open;
    }

    /// Toggle the open/closed state
    pub fn toggle_open(&mut self) {
        self.open = !self.open;
    }

    /// Clear all children from this group
    pub fn clear(&mut self) {
        self.children.clear();
    }
}

impl Object for GroupObject {
    fn name(&self) -> &str {
        &self.name
    }

    fn object_type(&self) -> ObjectType {
        ObjectType::Group
    }

    fn state(&self) -> &ObjectState {
        &self.state
    }

    fn state_mut(&mut self) -> &mut ObjectState {
        &mut self.state
    }

    /// Groups have no geometry, so extent returns None
    /// Use `ObjectRegistry::group_extent()` to get combined child extent
    fn extent(&self) -> Option<(Vec3, Vec3)> {
        None
    }

    fn n_states(&self) -> usize {
        1
    }

    fn current_state(&self) -> usize {
        0
    }

    fn set_current_state(&mut self, _state: usize) -> bool {
        false
    }

    fn set_name(&mut self, name: String) {
        self.name = name;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_group_creation() {
        let group = GroupObject::new("my_group");
        assert_eq!(group.name(), "my_group");
        assert!(group.is_empty());
        assert!(group.is_open());
        assert!(group.is_enabled());
        assert_eq!(group.object_type(), ObjectType::Group);
        assert!(group.extent().is_none());
    }

    #[test]
    fn test_group_with_children() {
        let group =
            GroupObject::with_children("my_group", vec!["obj1".to_string(), "obj2".to_string()]);
        assert_eq!(group.len(), 2);
        assert!(group.contains_child("obj1"));
        assert!(group.contains_child("obj2"));
    }

    #[test]
    fn test_add_remove_children() {
        let mut group = GroupObject::new("my_group");

        group.add_child("obj1".to_string());
        group.add_child("obj2".to_string());
        assert_eq!(group.len(), 2);

        // Adding duplicate should be ignored
        group.add_child("obj1".to_string());
        assert_eq!(group.len(), 2);

        assert!(group.remove_child("obj1"));
        assert_eq!(group.len(), 1);
        assert!(!group.contains_child("obj1"));

        // Removing non-existent child
        assert!(!group.remove_child("nonexistent"));
    }

    #[test]
    fn test_group_open_state() {
        let mut group = GroupObject::new("my_group");
        assert!(group.is_open());

        group.set_open(false);
        assert!(!group.is_open());

        group.toggle_open();
        assert!(group.is_open());
    }
}
