//! Reversible plugin overrides of host registrations.

use ahash::AHashMap;

#[derive(Clone)]
pub(crate) struct RegistrationLayers<T> {
    entries: AHashMap<String, Vec<(Option<u64>, T)>>,
}

impl<T> Default for RegistrationLayers<T> {
    fn default() -> Self {
        Self {
            entries: AHashMap::new(),
        }
    }
}

impl<T: Clone> RegistrationLayers<T> {
    pub(crate) fn insert(
        &mut self,
        active: &mut AHashMap<String, T>,
        key: String,
        owner: u64,
        value: T,
    ) {
        let layers = self.entries.entry(key.clone()).or_insert_with(|| {
            active
                .get(&key)
                .cloned()
                .map(|value| vec![(None, value)])
                .unwrap_or_default()
        });
        layers.retain(|(id, _)| *id != Some(owner));
        layers.push((Some(owner), value.clone()));
        active.insert(key, value);
    }

    pub(crate) fn remove_owner(&mut self, active: &mut AHashMap<String, T>, owner: u64) {
        self.remove_where(active, |_, id, _| id == Some(owner));
    }

    pub(crate) fn remove_where(
        &mut self,
        active: &mut AHashMap<String, T>,
        matches: impl Fn(&str, Option<u64>, &T) -> bool,
    ) {
        self.entries.retain(|key, layers| {
            let before = layers.len();
            layers.retain(|(id, value)| !matches(key, *id, value));
            if layers.len() == before {
                return true;
            }
            if let Some((_, value)) = layers.last() {
                active.insert(key.clone(), value.clone());
            } else {
                active.remove(key);
            }
            layers.iter().any(|(id, _)| id.is_some())
        });
    }

    pub(crate) fn forget(&mut self, key: &str) {
        self.entries.remove(key);
    }
}
