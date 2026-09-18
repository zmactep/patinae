//! Reversible plugin overrides of host registrations.

use ahash::AHashMap;

/// Identifies one installation within a command executor's lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PluginInstanceId(pub(crate) u64);

#[derive(Clone)]
pub(crate) struct RegistrationLayers<T> {
    entries: AHashMap<String, Vec<(Option<PluginInstanceId>, T)>>,
    active: AHashMap<String, T>,
}

impl<T> Default for RegistrationLayers<T> {
    fn default() -> Self {
        Self {
            entries: AHashMap::new(),
            active: AHashMap::new(),
        }
    }
}

impl<T: Clone> RegistrationLayers<T> {
    pub(crate) fn active(&self) -> &AHashMap<String, T> {
        &self.active
    }

    /// Replace a key outright, discarding its previous ownership layers.
    pub(crate) fn replace(&mut self, key: String, value: T) {
        self.entries.remove(&key);
        self.active.insert(key, value);
    }

    pub(crate) fn remove(&mut self, key: &str) -> Option<T> {
        self.entries.remove(key);
        self.active.remove(key)
    }

    pub(crate) fn insert(&mut self, key: String, owner: PluginInstanceId, value: T) {
        let layers = self.entries.entry(key.clone()).or_insert_with(|| {
            self.active
                .get(&key)
                .cloned()
                .map(|value| vec![(None, value)])
                .unwrap_or_default()
        });
        layers.retain(|(id, _)| *id != Some(owner));
        layers.push((Some(owner), value.clone()));
        self.active.insert(key, value);
    }

    pub(crate) fn remove_owner(&mut self, owner: PluginInstanceId) {
        self.remove_where(|_, id, _| id == Some(owner));
    }

    pub(crate) fn remove_where(
        &mut self,
        matches: impl Fn(&str, Option<PluginInstanceId>, &T) -> bool,
    ) {
        self.entries.retain(|key, layers| {
            let before = layers.len();
            layers.retain(|(id, value)| !matches(key, *id, value));
            if layers.len() == before {
                return true;
            }
            if let Some((_, value)) = layers.last() {
                self.active.insert(key.clone(), value.clone());
            } else {
                self.active.remove(key);
            }
            layers.iter().any(|(id, _)| id.is_some())
        });
    }
}
