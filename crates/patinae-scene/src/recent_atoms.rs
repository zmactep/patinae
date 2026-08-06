//! Ordered recent atom paths with transient runtime identities.

use std::collections::HashSet;

use patinae_settings::groups::RecentPickLimit;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Opaque identity for one runtime recent-atom row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RecentAtomId(u64);

impl RecentAtomId {
    #[cfg(test)]
    fn raw_for_tests(self) -> u64 {
        self.0
    }
}

/// One recent atom row with a transient identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentAtomRow {
    id: RecentAtomId,
    path: String,
}

impl RecentAtomRow {
    /// Returns this row's transient runtime identity.
    pub fn id(&self) -> RecentAtomId {
        self.id
    }

    /// Returns this row's canonical slash path.
    pub fn path(&self) -> &str {
        &self.path
    }
}

/// Ordered unique canonical atom paths owned by a session.
///
/// Serialization contains only paths. Row identities and the generation token
/// are reconstructed whenever the collection is deserialized.
#[derive(Debug, Clone)]
pub struct RecentAtoms {
    rows: Vec<RecentAtomRow>,
    next_id: u64,
    generation: u64,
    incarnation: u64,
}

impl Default for RecentAtoms {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            next_id: 1,
            generation: 0,
            incarnation: 0,
        }
    }
}

impl RecentAtoms {
    /// Creates an empty recent atom collection.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the generation incremented by structural mutations.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Returns the transient collection identity changed by session replacement.
    pub fn incarnation(&self) -> u64 {
        self.incarnation
    }

    /// Returns all rows in insertion order.
    pub fn rows(&self) -> &[RecentAtomRow] {
        &self.rows
    }

    /// Iterates canonical paths in insertion order.
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.rows.iter().map(RecentAtomRow::path)
    }

    /// Returns the identity currently associated with `path`.
    pub fn row_id(&self, path: &str) -> Option<RecentAtomId> {
        self.rows
            .iter()
            .find(|row| row.path == path)
            .map(|row| row.id)
    }

    /// Returns the number of stored paths.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Returns whether no paths are stored.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Inserts a path if it is not present and enforces `limit` after the append.
    pub fn insert(&mut self, path: impl Into<String>, limit: RecentPickLimit) -> bool {
        let path = path.into();
        if self.rows.iter().any(|row| row.path == path) || limit == RecentPickLimit::Bounded(0) {
            return false;
        }

        let id = self.allocate_id();
        self.rows.push(RecentAtomRow { id, path });
        self.remove_excess(limit);
        self.finish(true)
    }

    /// Removes one row by durable path identity.
    pub fn remove_path(&mut self, path: &str) -> bool {
        let Some(index) = self.rows.iter().position(|row| row.path == path) else {
            return false;
        };
        self.rows.remove(index);
        self.finish(true)
    }

    /// Removes every row in insertion order.
    pub fn clear(&mut self) -> bool {
        if self.rows.is_empty() {
            return false;
        }
        self.rows.clear();
        self.finish(true)
    }

    /// Rewrites retained paths and removes rejected paths in one stable pass.
    pub(crate) fn reconcile_paths(
        &mut self,
        mut reconcile: impl FnMut(&str) -> Option<String>,
    ) -> bool {
        let mut changed = false;
        self.rows.retain_mut(|row| {
            let Some(path) = reconcile(&row.path) else {
                changed = true;
                return false;
            };
            if path != row.path {
                row.path = path;
                changed = true;
            }
            true
        });
        changed |= self.deduplicate_rows();
        self.finish(changed)
    }

    /// Applies `limit`, removing oldest rows first.
    pub(crate) fn enforce_limit(&mut self, limit: RecentPickLimit) -> bool {
        let changed = self.remove_excess(limit);
        self.finish(changed)
    }

    /// Invalidates generation-based observers without changing durable rows.
    pub fn invalidate(&mut self) {
        self.generation = self
            .generation
            .checked_add(1)
            .expect("recent atom generation exhausted");
    }

    pub(crate) fn mark_replaced_after(&mut self, previous_incarnation: u64) {
        self.incarnation = previous_incarnation
            .checked_add(1)
            .expect("recent atom incarnation exhausted");
    }

    fn allocate_id(&mut self) -> RecentAtomId {
        let id = RecentAtomId(self.next_id);
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("recent atom row identities exhausted");
        id
    }

    fn deduplicate_rows(&mut self) -> bool {
        let mut seen = HashSet::with_capacity(self.rows.len());
        let previous_len = self.rows.len();
        self.rows.retain(|row| seen.insert(row.path.clone()));
        self.rows.len() != previous_len
    }

    fn remove_excess(&mut self, limit: RecentPickLimit) -> bool {
        let RecentPickLimit::Bounded(limit) = limit else {
            return false;
        };
        let excess = self.rows.len().saturating_sub(limit);
        self.rows.drain(..excess);
        excess != 0
    }

    fn finish(&mut self, changed: bool) -> bool {
        if changed {
            self.generation = self
                .generation
                .checked_add(1)
                .expect("recent atom generation exhausted");
        }
        changed
    }

    fn from_paths(paths: Vec<String>) -> Self {
        let mut recent = Self::new();
        let mut seen = HashSet::with_capacity(paths.len());
        for path in paths {
            if seen.insert(path.clone()) {
                let id = recent.allocate_id();
                recent.rows.push(RecentAtomRow { id, path });
            }
        }
        recent.generation = 0;
        recent
    }
}

impl Serialize for RecentAtoms {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_seq(self.paths())
    }
}

impl<'de> Deserialize<'de> for RecentAtoms {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Vec::<String>::deserialize(deserializer).map(Self::from_paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use patinae_settings::groups::RecentPickLimit;

    fn paths(recent: &RecentAtoms) -> Vec<&str> {
        recent.paths().collect()
    }

    #[test]
    fn insert_duplicate_remove_and_clear_preserve_stable_row_ids() {
        let mut recent = RecentAtoms::new();

        assert!(recent.insert("/first", RecentPickLimit::Unlimited));
        let first_id = recent.row_id("/first").unwrap();
        assert!(recent.insert("/second", RecentPickLimit::Unlimited));
        let second_id = recent.row_id("/second").unwrap();

        assert_eq!(paths(&recent), ["/first", "/second"]);
        assert!(first_id != second_id);

        assert!(!recent.insert("/first", RecentPickLimit::Unlimited));
        assert_eq!(paths(&recent), ["/first", "/second"]);

        assert!(recent.remove_path("/first"));
        assert!(recent.remove_path("/second"));
        assert!(recent.is_empty());

        recent.insert("/third", RecentPickLimit::Unlimited);
        recent.insert("/fourth", RecentPickLimit::Unlimited);
        assert!(recent.clear());
        assert!(recent.is_empty());
    }

    #[test]
    fn deserialize_stably_deduplicates_paths_and_reconstructs_transient_ids() {
        let restored: RecentAtoms =
            serde_json::from_str(r#"["/first","/second","/first"]"#).unwrap();

        assert_eq!(paths(&restored), ["/first", "/second"]);
        assert_eq!(restored.generation(), 0);
        assert_eq!(restored.rows()[0].id().raw_for_tests(), 1);
        assert_eq!(restored.rows()[1].id().raw_for_tests(), 2);

        let mut original = RecentAtoms::new();
        original.insert("/discarded", RecentPickLimit::Unlimited);
        original.remove_path("/discarded");
        original.insert("/first", RecentPickLimit::Unlimited);
        original.insert("/second", RecentPickLimit::Unlimited);
        let json = serde_json::to_string(&original).unwrap();
        let reconstructed: RecentAtoms = serde_json::from_str(&json).unwrap();

        assert_eq!(json, r#"["/first","/second"]"#);
        assert_ne!(original.rows()[0].id(), reconstructed.rows()[0].id());
        assert_eq!(paths(&reconstructed), ["/first", "/second"]);
    }

    #[test]
    fn reconciliation_rewrites_prunes_and_deduplicates_stably() {
        let mut recent = RecentAtoms::new();
        recent.insert("/first", RecentPickLimit::Unlimited);
        recent.insert("/second", RecentPickLimit::Unlimited);
        recent.insert("/third", RecentPickLimit::Unlimited);
        let second = recent.row_id("/second").unwrap();
        let third = recent.row_id("/third").unwrap();

        assert!(recent.reconcile_paths(|path| match path {
            "/first" => None,
            "/second" => Some("/renamed".to_string()),
            _ => Some(path.to_string()),
        }));
        assert_eq!(paths(&recent), ["/renamed", "/third"]);
        assert_eq!(recent.row_id("/renamed"), Some(second));

        let duplicate_id = recent.allocate_id();
        recent.rows.push(RecentAtomRow {
            id: duplicate_id,
            path: "/renamed".to_string(),
        });
        assert!(recent.reconcile_paths(|path| Some(path.to_string())));
        assert_eq!(paths(&recent), ["/renamed", "/third"]);
        assert_eq!(recent.row_id("/third"), Some(third));
    }

    #[test]
    fn generation_changes_once_per_structural_operation() {
        let mut recent = RecentAtoms::new();
        assert_eq!(recent.generation(), 0);

        recent.insert("/first", RecentPickLimit::Unlimited);
        assert_eq!(recent.generation(), 1);
        recent.insert("/first", RecentPickLimit::Unlimited);
        assert_eq!(recent.generation(), 1);
        recent.remove_path("/first");
        assert_eq!(recent.generation(), 2);
        recent.clear();
        assert_eq!(recent.generation(), 2);
    }

    #[test]
    fn limits_keep_newest_paths_after_deduplication() {
        let mut recent = RecentAtoms::new();
        recent.insert("/one", RecentPickLimit::Unlimited);
        recent.insert("/two", RecentPickLimit::Unlimited);
        recent.insert("/three", RecentPickLimit::Unlimited);

        assert!(recent.enforce_limit(RecentPickLimit::Bounded(2)));
        assert_eq!(paths(&recent), ["/two", "/three"]);

        assert!(!recent.enforce_limit(RecentPickLimit::Bounded(4)));
        recent.insert("/four", RecentPickLimit::Bounded(2));
        assert_eq!(paths(&recent), ["/three", "/four"]);

        assert!(recent.enforce_limit(RecentPickLimit::Bounded(0)));
        assert!(recent.is_empty());
        assert!(!recent.insert("/five", RecentPickLimit::Bounded(0)));
        assert!(recent.is_empty());

        recent.insert("/six", RecentPickLimit::Unlimited);
        recent.insert("/seven", RecentPickLimit::Unlimited);
        assert_eq!(paths(&recent), ["/six", "/seven"]);
    }

    #[test]
    fn singleton_reconciliation_prunes_malformed_zero_and_multiple_matches() {
        let mut recent = RecentAtoms::new();
        recent.insert("malformed", RecentPickLimit::Unlimited);
        recent.insert("/valid", RecentPickLimit::Unlimited);
        recent.insert("/zero", RecentPickLimit::Unlimited);
        recent.insert("/multiple", RecentPickLimit::Unlimited);
        let valid = recent.row_id("/valid").unwrap();

        assert!(
            recent.reconcile_paths(|path| { matches!(path, "/valid").then(|| path.to_string()) })
        );

        assert_eq!(recent.rows()[0].id(), valid);
        assert_eq!(paths(&recent), ["/valid"]);
    }
}
