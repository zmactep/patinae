//! Named selection management
//!
//! This module provides storage and evaluation of named selections,
//! (named atom selection management).

use ahash::AHashMap;
use patinae_select::{EvalContext, SelectionOptions, SelectionResult};
use serde::{Deserialize, Serialize};

use crate::object::ObjectRegistry;

/// Entry for a named selection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectionEntry {
    /// The selection expression
    pub expression: String,
    /// Whether the selection indicators are visible
    pub visible: bool,
    /// Cached per-object evaluation results (object_name → SelectionResult).
    /// Populated at define time, used by build_eval_context to avoid re-evaluation.
    #[serde(skip)]
    pub cached_results: AHashMap<String, SelectionResult>,
}

impl SelectionEntry {
    /// Create a new selection entry with visibility enabled by default
    pub fn new(expression: String) -> Self {
        Self {
            expression,
            visible: true,
            cached_results: AHashMap::new(),
        }
    }

    /// Create a new selection entry with cached evaluation results
    pub fn with_results(expression: String, results: Vec<(String, SelectionResult)>) -> Self {
        Self {
            expression,
            visible: true,
            cached_results: results.into_iter().collect(),
        }
    }
}

/// Manager for named selections
///
/// Handles storage, retrieval, and evaluation of named selection expressions.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct SelectionManager {
    /// Named selections (name -> entry with expression and visibility)
    selections: AHashMap<String, SelectionEntry>,
    /// Generation counter, incremented on structural changes (add/remove/rename)
    #[serde(skip)]
    generation: u64,
}

impl SelectionManager {
    /// Create a new empty selection manager
    pub fn new() -> Self {
        Self::default()
    }

    /// Get the generation counter (incremented on structural changes)
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Force observers that track `generation()` to resync.
    pub fn invalidate(&mut self) {
        self.generation += 1;
    }

    /// Get a selection entry by name
    pub fn get(&self, name: &str) -> Option<&SelectionEntry> {
        self.selections.get(name)
    }

    /// Get a mutable selection entry by name
    pub fn get_mut(&mut self, name: &str) -> Option<&mut SelectionEntry> {
        self.selections.get_mut(name)
    }

    /// Get a selection expression by name
    pub fn get_expression(&self, name: &str) -> Option<&str> {
        self.selections.get(name).map(|e| e.expression.as_str())
    }

    /// Define (store) a named selection expression.
    ///
    /// If a selection with this name already exists, it will be replaced.
    /// The cache will be lazily populated on the next `rebuild_caches` /
    /// `evaluate_visible` call.
    pub fn define(&mut self, name: &str, expression: &str) {
        self.selections.insert(
            name.to_string(),
            SelectionEntry::new(expression.to_string()),
        );
        self.generation += 1;
    }

    /// Define a named selection with pre-computed evaluation results.
    ///
    /// The `results` contain per-object SelectionResult bitvecs evaluated at
    /// define time, so subsequent lookups avoid re-parsing and re-evaluating.
    pub fn define_with_results(
        &mut self,
        name: &str,
        expression: &str,
        results: Vec<(String, SelectionResult)>,
    ) {
        self.selections.insert(
            name.to_string(),
            SelectionEntry::with_results(expression.to_string(), results),
        );
        self.generation += 1;
    }

    /// Remove a named selection
    ///
    /// Returns true if the selection existed and was removed.
    pub fn remove(&mut self, name: &str) -> bool {
        let removed = self.selections.remove(name).is_some();
        if removed {
            self.generation += 1;
        }
        removed
    }

    /// Get all selection names
    pub fn names(&self) -> Vec<String> {
        self.selections.keys().cloned().collect()
    }

    /// Rename a named selection
    ///
    /// Returns true if the selection existed and was renamed.
    pub fn rename(&mut self, old_name: &str, new_name: &str) -> bool {
        if let Some(entry) = self.selections.remove(old_name) {
            self.selections.insert(new_name.to_string(), entry);
            self.generation += 1;
            true
        } else {
            false
        }
    }

    /// Check if a selection exists
    pub fn contains(&self, name: &str) -> bool {
        self.selections.contains_key(name)
    }

    /// Get the number of selections
    pub fn len(&self) -> usize {
        self.selections.len()
    }

    /// Check if there are no selections
    pub fn is_empty(&self) -> bool {
        self.selections.is_empty()
    }

    /// Set the visibility of a named selection's indicators
    pub fn set_visible(&mut self, name: &str, visible: bool) {
        if let Some(entry) = self.selections.get_mut(name) {
            if entry.visible != visible {
                entry.visible = visible;
                self.generation += 1;
            }
        }
    }

    /// Check if a named selection's indicators are visible
    pub fn is_visible(&self, name: &str) -> bool {
        self.selections
            .get(name)
            .map(|e| e.visible)
            .unwrap_or(false)
    }

    /// Set the "indicate" selection (shows pink indicators in the 3D view)
    ///
    /// This creates or updates the special "indicate" selection.
    pub fn indicate(&mut self, selection: &str) {
        self.selections.insert(
            "indicate".to_string(),
            SelectionEntry::new(selection.to_string()),
        );
        self.generation += 1;
    }

    /// Clear all selection indicators (hide all visible selections)
    pub fn clear_indication(&mut self) {
        let had_visible = self.selections.values().any(|e| e.visible);
        for entry in self.selections.values_mut() {
            entry.visible = false;
        }
        if had_visible {
            self.generation += 1;
        }
    }

    /// Get the currently indicated selection expression
    ///
    /// Returns the first visible selection expression for backwards compatibility.
    pub fn indicated_selection(&self) -> Option<&str> {
        self.selections
            .values()
            .find(|e| e.visible)
            .map(|e| e.expression.as_str())
    }

    /// Get an iterator over all selections
    pub fn iter(&self) -> impl Iterator<Item = (&String, &SelectionEntry)> {
        self.selections.iter()
    }

    /// Get selection names matching a wildcard pattern.
    ///
    /// Supports `*` (any characters) and `?` (single character) via
    /// `patinae_select::Pattern::Wildcard`. Pattern `*` or `all` matches everything.
    pub fn matching(&self, pattern: &str) -> Vec<&str> {
        if pattern == "*" || pattern == "all" {
            return self.selections.keys().map(|s| s.as_str()).collect();
        }
        let pat = patinae_select::Pattern::Wildcard(pattern.to_string());
        self.selections
            .keys()
            .filter(|name| pat.matches(name, false))
            .map(|s| s.as_str())
            .collect()
    }

    /// Get an iterator over visible selections
    pub fn visible_selections(&self) -> impl Iterator<Item = (&str, &str)> {
        self.selections
            .iter()
            .filter(|(_, entry)| entry.visible)
            .map(|(name, entry)| (name.as_str(), entry.expression.as_str()))
    }

    /// Clear all selections
    pub fn clear(&mut self) {
        if !self.selections.is_empty() {
            self.selections.clear();
            self.generation += 1;
        }
    }

    /// Evaluate all visible selections for all molecules.
    ///
    /// Rebuilds stale caches first, then combines cached results with OR.
    ///
    /// Returns a vector of (object_name, SelectionResult) pairs where the
    /// SelectionResult is the union of all visible selections.
    pub fn evaluate_visible(
        &mut self,
        registry: &ObjectRegistry,
        options: SelectionOptions,
        palette: &patinae_color::NamedPalette,
    ) -> Vec<(String, SelectionResult)> {
        let has_visible = self.selections.values().any(|e| e.visible);
        if !has_visible {
            return Vec::new();
        }

        // Ensure all caches are up to date
        self.rebuild_caches(registry, options, palette);

        let object_names: Vec<String> = registry.names().map(|s| s.to_string()).collect();
        let mut results: Vec<(String, SelectionResult)> = Vec::new();

        for obj_name in &object_names {
            let Some(mol_obj) = registry.get_molecule(obj_name) else {
                continue;
            };
            let atom_count = mol_obj.molecule().atom_count();

            // Combine all visible selections with OR using cached results
            let mut combined: Option<SelectionResult> = None;
            for (_, entry) in self.selections.iter().filter(|(_, e)| e.visible) {
                if let Some(cached) = entry.cached_results.get(obj_name) {
                    if cached.atom_count() == atom_count {
                        combined = Some(match combined.take() {
                            Some(existing) => existing.union(cached),
                            None => cached.clone(),
                        });
                    }
                }
            }

            if let Some(result) = combined {
                if result.any() {
                    results.push((obj_name.clone(), result));
                }
            }
        }

        results
    }

    /// Rebuild stale caches for all selections across all molecules in the registry.
    pub fn rebuild_caches(
        &mut self,
        registry: &ObjectRegistry,
        options: SelectionOptions,
        palette: &patinae_color::NamedPalette,
    ) {
        if self.selections.is_empty() {
            return;
        }
        let object_names: Vec<String> = registry.names().map(|s| s.to_string()).collect();
        for obj_name in &object_names {
            let Some(mol_obj) = registry.get_molecule(obj_name) else {
                continue;
            };
            let (_, resolved) = resolve_selections(
                mol_obj.molecule(),
                mol_obj.display_state(),
                obj_name,
                &object_names,
                options,
                &self.selections,
                palette,
            );
            // Write newly resolved results back into caches
            for (sel_name, result) in resolved {
                if let Some(entry) = self.selections.get_mut(&sel_name) {
                    entry.cached_results.insert(obj_name.to_string(), result);
                }
            }
        }
    }

    /// Build an `EvalContext` for a single molecule within the registry.
    ///
    /// The context includes:
    /// - Implicit object-name selections (all for the target, none for others)
    /// - Cached named selections resolved via multi-pass fixed-point
    /// - The provided `SelectionOptions` for case-sensitivity control
    ///
    /// This method is deliberately `&self` (read-only). It resolves uncached
    /// selections on the fly but does not persist them. The render path should
    /// use `evaluate_visible` (which calls `rebuild_caches`) for persistent
    /// cache updates.
    pub fn build_eval_context<'a>(
        &self,
        mol: &'a patinae_mol::ObjectMolecule,
        display_state: usize,
        obj_name: &str,
        all_object_names: &[String],
        options: SelectionOptions,
        palette: &'a patinae_color::NamedPalette,
    ) -> EvalContext<'a> {
        let (ctx, _) = resolve_selections(
            mol,
            display_state,
            obj_name,
            all_object_names,
            options,
            &self.selections,
            palette,
        );
        ctx
    }
}

/// Build an `EvalContext` seeded with object-name and cached selections, then
/// resolve any uncached/stale entries via multi-pass fixed-point evaluation.
///
/// Returns `(ctx, resolved)` where `resolved` contains the newly evaluated
/// `(selection_name, SelectionResult)` pairs that were not found in cache.
fn resolve_selections<'a>(
    mol: &'a patinae_mol::ObjectMolecule,
    display_state: usize,
    obj_name: &str,
    all_object_names: &[String],
    options: SelectionOptions,
    selections: &AHashMap<String, SelectionEntry>,
    palette: &'a patinae_color::NamedPalette,
) -> (EvalContext<'a>, Vec<(String, SelectionResult)>) {
    let atom_count = mol.atom_count();
    let mut ctx = EvalContext::single(mol);
    ctx.options = options;
    ctx.named_palette = Some(palette);
    ctx.set_state(Some(display_state.min(mol.state_count().saturating_sub(1))));

    // Implicit object-name selections
    for other in all_object_names {
        if other == obj_name {
            ctx.add_selection(other.clone(), SelectionResult::all(atom_count));
        } else {
            ctx.add_selection(other.clone(), SelectionResult::none(atom_count));
        }
    }

    // Seed from cached results; collect uncached entries for resolution
    let mut uncached: Vec<(&String, &SelectionEntry)> = Vec::new();
    for (sel_name, entry) in selections {
        if let Some(cached) = entry.cached_results.get(obj_name) {
            if cached.atom_count() == atom_count {
                ctx.add_selection(sel_name.clone(), cached.clone());
                continue;
            }
        }
        uncached.push((sel_name, entry));
    }

    // Multi-pass: keep resolving until no progress (handles inter-selection deps)
    let mut resolved: Vec<(String, SelectionResult)> = Vec::new();
    if !uncached.is_empty() {
        let mut made_progress = true;
        while made_progress && !uncached.is_empty() {
            made_progress = false;
            uncached.retain(|(sel_name, entry)| {
                if let Ok(ast) = patinae_select::parse(&entry.expression) {
                    if let Ok(result) = patinae_select::evaluate(&ast, &ctx) {
                        ctx.add_selection((*sel_name).clone(), result.clone());
                        resolved.push(((*sel_name).clone(), result));
                        made_progress = true;
                        return false;
                    }
                }
                true
            });
        }
    }

    (ctx, resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_selection_entry() {
        let entry = SelectionEntry::new("chain A".to_string());
        assert_eq!(entry.expression, "chain A");
        assert!(entry.visible);
    }

    #[test]
    fn test_selection_manager_basic() {
        let mut manager = SelectionManager::new();

        manager.define("sel1", "chain A");
        assert!(manager.contains("sel1"));
        assert_eq!(manager.get_expression("sel1"), Some("chain A"));

        manager.define("sel2", "resn ALA");
        assert_eq!(manager.len(), 2);

        assert!(manager.remove("sel1"));
        assert!(!manager.contains("sel1"));
        assert_eq!(manager.len(), 1);
    }

    #[test]
    fn test_selection_visibility() {
        let mut manager = SelectionManager::new();

        manager.define("sel1", "chain A");
        assert!(manager.is_visible("sel1"));

        manager.set_visible("sel1", false);
        assert!(!manager.is_visible("sel1"));

        manager.set_visible("sel1", true);
        assert!(manager.is_visible("sel1"));
    }

    #[test]
    fn test_indicate_selection() {
        let mut manager = SelectionManager::new();

        manager.indicate("chain B");
        assert!(manager.contains("indicate"));
        assert_eq!(manager.get_expression("indicate"), Some("chain B"));
        assert_eq!(manager.indicated_selection(), Some("chain B"));
    }

    #[test]
    fn test_clear_indication() {
        let mut manager = SelectionManager::new();

        manager.define("sel1", "chain A");
        manager.define("sel2", "chain B");

        manager.clear_indication();

        assert!(!manager.is_visible("sel1"));
        assert!(!manager.is_visible("sel2"));
        assert!(manager.indicated_selection().is_none());
    }

    #[test]
    fn test_build_eval_context_basic_chain_selection() {
        use lin_alg::f32::Vec3;
        use patinae_mol::{Atom, AtomResidue, CoordSet, Element, ObjectMolecule};
        use std::sync::Arc;

        // Create a molecule with two chains (H and L)
        let mut mol = ObjectMolecule::new("1fdl");
        let chain_h = Arc::new(AtomResidue::from_parts("H", "ALA", 1, ' ', ""));
        let chain_l = Arc::new(AtomResidue::from_parts("L", "ALA", 1, ' ', ""));

        for (residue, name) in [(chain_h.clone(), "CA"), (chain_l.clone(), "CA")] {
            let mut atom = Atom::new(name, Element::Carbon);
            atom.residue = residue.clone();
            mol.add_atom(atom);
        }
        mol.add_coord_set(CoordSet::from_vec3(&[
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(5.0, 0.0, 0.0),
        ]));

        // Empty selection manager (no named selections)
        let manager = SelectionManager::new();
        let object_names = vec!["1fdl".to_string()];
        let options = SelectionOptions::default();

        let palette = patinae_color::NamedPalette::default();
        let ctx = manager.build_eval_context(&mol, 0, "1fdl", &object_names, options, &palette);

        // Evaluate "chain H or chain L"
        let expr = patinae_select::parse("chain H or chain L").unwrap();
        let result = patinae_select::evaluate(&expr, &ctx).unwrap();
        assert_eq!(result.count(), 2, "chain H or chain L should match 2 atoms");

        // Evaluate "chain H" alone
        let expr = patinae_select::parse("chain H").unwrap();
        let result = patinae_select::evaluate(&expr, &ctx).unwrap();
        assert_eq!(result.count(), 1, "chain H should match 1 atom");
    }

    #[test]
    fn test_cached_results_used_in_build_eval_context() {
        use lin_alg::f32::Vec3;
        use patinae_mol::{Atom, AtomResidue, CoordSet, Element, ObjectMolecule};
        use std::sync::Arc;

        let mut mol = ObjectMolecule::new("test");
        let chain_h = Arc::new(AtomResidue::from_parts("H", "ALA", 1, ' ', ""));
        let chain_l = Arc::new(AtomResidue::from_parts("L", "ALA", 1, ' ', ""));

        for residue in &[chain_h.clone(), chain_l.clone()] {
            let mut atom = Atom::new("CA", Element::Carbon);
            atom.residue = residue.clone();
            mol.add_atom(atom);
        }
        mol.add_coord_set(CoordSet::from_vec3(&[
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(5.0, 0.0, 0.0),
        ]));

        let mut manager = SelectionManager::new();
        let object_names = vec!["test".to_string()];
        let options = SelectionOptions::default();

        // Define "ab" = "chain H" with cached results
        let palette = patinae_color::NamedPalette::default();
        let ctx = manager.build_eval_context(&mol, 0, "test", &object_names, options, &palette);
        let expr = patinae_select::parse("chain H").unwrap();
        let result = patinae_select::evaluate(&expr, &ctx).unwrap();
        assert_eq!(result.count(), 1);

        manager.define_with_results("ab", "chain H", vec![("test".to_string(), result)]);

        // Now build context again — "ab" should be resolvable from cache
        let palette = patinae_color::NamedPalette::default();
        let ctx = manager.build_eval_context(&mol, 0, "test", &object_names, options, &palette);
        assert!(
            ctx.has_selection("ab"),
            "named selection 'ab' should be in context"
        );

        // "ab or chain L" should work
        let expr = patinae_select::parse("ab or chain L").unwrap();
        let result = patinae_select::evaluate(&expr, &ctx).unwrap();
        assert_eq!(result.count(), 2, "ab or chain L should match 2 atoms");
    }
}
