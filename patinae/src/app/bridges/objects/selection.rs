//! Selection for ObjectsBridge.

use super::*;

impl ObjectsBridge {
    pub(super) fn clear_scene_selection(&mut self) {
        self.scene_selection.selected_groups.clear();
        self.scene_selection.selected_objects.clear();
        self.scene_selection.selected_subchains.clear();
        self.scene_selection.selection_level = SelectionLevel::None;
        self.scene_selection.anchor = None;
    }

    pub(super) fn clear_named_selection(&mut self) {
        self.named_selection.selected_selections.clear();
        self.named_selection.selection_anchor = None;
    }

    pub(super) fn prune_scene_selection_state(&mut self, scene: &SceneModel) -> bool {
        let before_groups = self.scene_selection.selected_groups.clone();
        let before_objects = self.scene_selection.selected_objects.clone();
        let before_subchains = self.scene_selection.selected_subchains.clone();
        let before_level = self.scene_selection.selection_level;
        let before_anchor = self.scene_selection.anchor.clone();

        self.scene_selection
            .selected_groups
            .retain(|name| scene.get_group(name).is_some());
        self.scene_selection
            .selected_objects
            .retain(|name| scene.get(name).is_some());
        self.scene_selection
            .selected_subchains
            .retain(|key| scene_has_subchain(scene, key));

        let active_level_empty = match self.scene_selection.selection_level {
            SelectionLevel::None => false,
            SelectionLevel::Groups => self.scene_selection.selected_groups.is_empty(),
            SelectionLevel::Objects => self.scene_selection.selected_objects.is_empty(),
            SelectionLevel::Subchains => self.scene_selection.selected_subchains.is_empty(),
        };
        if active_level_empty {
            self.scene_selection.selection_level = SelectionLevel::None;
            self.scene_selection.anchor = None;
        } else if self.scene_selection.selected_groups != before_groups
            || self.scene_selection.selected_objects != before_objects
            || self.scene_selection.selected_subchains != before_subchains
        {
            self.scene_selection.anchor = None;
        }

        self.scene_selection.selected_groups != before_groups
            || self.scene_selection.selected_objects != before_objects
            || self.scene_selection.selected_subchains != before_subchains
            || self.scene_selection.selection_level != before_level
            || self.scene_selection.anchor != before_anchor
    }

    pub(super) fn switch_level(&mut self, level: SelectionLevel) {
        if self.scene_selection.selection_level != level {
            self.scene_selection.selected_groups.clear();
            self.scene_selection.selected_objects.clear();
            self.scene_selection.selected_subchains.clear();
            self.scene_selection.anchor = None;
            self.scene_selection.selection_level = level;
        }
    }

    pub(super) fn selected_count(&self) -> i32 {
        let level_count = match self.scene_selection.selection_level {
            SelectionLevel::None => 0,
            SelectionLevel::Groups => self.scene_selection.selected_groups.len() as i32,
            SelectionLevel::Objects => self.scene_selection.selected_objects.len() as i32,
            SelectionLevel::Subchains => self.scene_selection.selected_subchains.len() as i32,
        };
        level_count + self.named_selection.selected_selections.len() as i32
    }

    /// Return the single selected object that can receive an object movie
    /// keyframe. Named selections, groups, and subchains are intentionally
    /// excluded because `mview store, object=...` needs a concrete object.
    pub(crate) fn single_movie_keyframe_object(&self) -> Option<String> {
        if self.scene_selection.selection_level == SelectionLevel::Objects
            && self.scene_selection.selected_objects.len() == 1
            && self.named_selection.selected_selections.is_empty()
        {
            Some(self.scene_selection.selected_objects[0].clone())
        } else {
            None
        }
    }

    pub(super) fn update_slint_selection(
        &self,
        os: &ObjectsState,
        scene: &SceneModel,
        registry: &ObjectRegistry,
    ) {
        os.set_selected_count(self.selected_count());
        os.set_recent_operand_count(self.recent.recent_operands.len() as i32);
        os.set_recent_can_add_measurement((2..=4).contains(&self.recent.recent_operands.len()));
        os.set_recent_can_add_label((1..=4).contains(&self.recent.recent_operands.len()));
        let measurement_kind =
            patinae_cmd::measurement_kind_for_count(self.recent.recent_operands.len())
                .ok()
                .map(|kind| measurement_kind_name(Some(kind)))
                .unwrap_or("");
        os.set_recent_measurement_kind(measurement_kind.into());
        os.set_recent_has_measurement_targets(
            !self.compatible_measurement_targets(registry).is_empty(),
        );
        os.set_selection_level(self.scene_selection.selection_level.as_str().into());
        os.set_selected_selection_count(self.named_selection.selected_selections.len() as i32);
        let capabilities = self.selected_capabilities(scene);
        os.set_action_can_focus(capabilities.is_some_and(|value| value.focus));
        os.set_action_can_orient(capabilities.is_some_and(|value| value.orient));
        os.set_action_can_center(capabilities.is_some_and(|value| value.focus));
        os.set_action_can_toggle(capabilities.is_some_and(|value| value.visibility));
        os.set_action_can_overflow(capabilities.is_some());
        // Rebuild model to reflect selection flags
        // (we need to update selected flags in the existing model)
        self.update_selection_flags_in_model();
        self.update_selection_row_flags();
    }

    pub(super) fn update_selection_flags_in_model(&self) {
        self.dock_dirty.set(true);
        let count = self.top_level_model.row_count();
        for i in 0..count {
            let Some(mut row) = self.top_level_model.row_data(i) else {
                continue;
            };
            let mut changed = false;

            if row.is_group {
                let want = self
                    .scene_selection
                    .selected_groups
                    .contains(&row.group_name.to_string());
                if row.group_selected != want {
                    row.group_selected = want;
                    changed = true;
                }
                // Update children selection flags
                let children = row.group_objects.clone();
                let child_count = children.row_count();
                for j in 0..child_count {
                    let Some(mut obj) = children.row_data(j) else {
                        continue;
                    };
                    let obj_name = obj.name.to_string();
                    let obj_want = self.scene_selection.selected_objects.contains(&obj_name);
                    if obj.selected != obj_want {
                        obj.selected = obj_want;
                        self.update_subchain_selection(&mut obj);
                        children.set_row_data(j, obj);
                    } else {
                        let sub_changed = self.update_subchain_selection(&mut obj);
                        if sub_changed {
                            children.set_row_data(j, obj);
                        }
                    }
                }
            } else {
                let obj_name = row.object.name.to_string();
                let want = self.scene_selection.selected_objects.contains(&obj_name);
                if row.object.selected != want {
                    row.object.selected = want;
                    changed = true;
                }
                let sub_changed = self.update_subchain_selection(&mut row.object);
                changed = changed || sub_changed;
            }

            if changed {
                self.top_level_model.set_row_data(i, row);
            }
        }
    }

    /// Update subchain selection flags on an ObjectItem. Returns true if any changed.
    pub(super) fn update_subchain_selection(&self, obj: &mut ObjectItem) -> bool {
        let subchains = obj.subchains.clone();
        let count = subchains.row_count();
        let obj_name = obj.name.to_string();
        let mut any_changed = false;
        for k in 0..count {
            let Some(mut sub) = subchains.row_data(k) else {
                continue;
            };
            let want = self.scene_selection.selected_subchains.iter().any(|key| {
                key.obj_name == obj_name
                    && key.chain_id == sub.chain_id.as_str()
                    && key.label == sub.label.as_str()
                    && key.kind == sub.kind.as_str()
            });
            if sub.selected != want {
                sub.selected = want;
                subchains.set_row_data(k, sub);
                any_changed = true;
            }
        }
        any_changed
    }

    /// Update `selected` flags on each `SelectionRow` in the selections model.
    pub(super) fn update_selection_row_flags(&self) {
        self.dock_dirty.set(true);
        let count = self.selections_model.row_count();
        for i in 0..count {
            let Some(mut row) = self.selections_model.row_data(i) else {
                continue;
            };
            let name = row.name.to_string();
            let want = self.named_selection.selected_selections.contains(&name);
            if row.selected != want {
                row.selected = want;
                self.selections_model.set_row_data(i, row);
            }
        }
    }

    pub(super) fn click_selections(
        &mut self,
        target: String,
        order: &[String],
        shift: bool,
        meta: bool,
    ) {
        handle_click(
            &mut self.named_selection.selected_selections,
            target.clone(),
            target,
            |s| Some(s.to_string()),
            order,
            shift,
            meta,
            &mut self.named_selection.selection_anchor,
        );
    }

    pub(super) fn click_groups(
        &mut self,
        target: String,
        order: &[String],
        shift: bool,
        meta: bool,
    ) {
        self.switch_level(SelectionLevel::Groups);
        let cleared = handle_click(
            &mut self.scene_selection.selected_groups,
            target.clone(),
            target,
            |s| Some(s.to_string()),
            order,
            shift,
            meta,
            &mut self.scene_selection.anchor,
        );
        if cleared {
            self.scene_selection.selection_level = SelectionLevel::None;
        }
    }

    pub(super) fn click_objects(
        &mut self,
        target: String,
        order: &[String],
        shift: bool,
        meta: bool,
    ) {
        self.switch_level(SelectionLevel::Objects);
        let cleared = handle_click(
            &mut self.scene_selection.selected_objects,
            target.clone(),
            target,
            |s| Some(s.to_string()),
            order,
            shift,
            meta,
            &mut self.scene_selection.anchor,
        );
        if cleared {
            self.scene_selection.selection_level = SelectionLevel::None;
        }
    }

    pub(super) fn click_subchains(
        &mut self,
        target: SubchainKey,
        key: String,
        order: &[SubchainKey],
        shift: bool,
        meta: bool,
    ) {
        self.switch_level(SelectionLevel::Subchains);
        let cleared = handle_click(
            &mut self.scene_selection.selected_subchains,
            target,
            key,
            |s| {
                let parts: Vec<&str> = s.split('\0').collect();
                if parts.len() == 3 {
                    // Anchor key carries (obj, chain, label) — find the
                    // matching SubchainKey in the visible order so we can
                    // resolve full identity (entry_index/selector_clause).
                    order
                        .iter()
                        .find(|k| {
                            k.obj_name == parts[0] && k.chain_id == parts[1] && k.label == parts[2]
                        })
                        .cloned()
                } else {
                    None
                }
            },
            order,
            shift,
            meta,
            &mut self.scene_selection.anchor,
        );
        if cleared {
            self.scene_selection.selection_level = SelectionLevel::None;
        }
    }

    pub(super) fn collect_selected_target(&self) -> Option<String> {
        let level_expr = match self.scene_selection.selection_level {
            SelectionLevel::None => None,
            SelectionLevel::Groups => join_or(&self.scene_selection.selected_groups),
            SelectionLevel::Objects => join_or(&self.scene_selection.selected_objects),
            SelectionLevel::Subchains => {
                build_subchains_expr(&self.scene_selection.selected_subchains)
            }
        };

        let sel_expr = join_or(&self.named_selection.selected_selections);

        match (level_expr, sel_expr) {
            (Some(l), Some(s)) => Some(format!("{} or {}", l, s)),
            (Some(l), None) => Some(l),
            (None, Some(s)) => Some(s),
            (None, None) => None,
        }
    }

    /// Build (mobile, fixed) pair for align: mobile = all but last OR-joined,
    /// fixed = the very last selected item.
    pub(super) fn collect_align_targets(&self) -> Option<(String, String)> {
        let mut parts: Vec<String> = Vec::new();

        match self.scene_selection.selection_level {
            SelectionLevel::Groups => {
                parts.extend(self.scene_selection.selected_groups.iter().cloned());
            }
            SelectionLevel::Objects => {
                parts.extend(self.scene_selection.selected_objects.iter().cloned());
            }
            SelectionLevel::Subchains => {
                // Build individual subchain expressions
                for key in &self.scene_selection.selected_subchains {
                    parts.push(build_single_subchain_expr(key));
                }
            }
            SelectionLevel::None => {}
        }

        parts.extend(self.named_selection.selected_selections.iter().cloned());

        if parts.len() < 2 {
            return None;
        }

        let fixed = parts.pop().unwrap();
        let mobile = parts.join(" or ");

        Some((mobile, fixed))
    }
}
/// Join names with ` or `, returning `None` if empty.
pub(super) fn join_or(names: &[String]) -> Option<String> {
    if names.is_empty() {
        None
    } else {
        Some(names.join(" or "))
    }
}

/// Build a single selection expression from multiple selected
/// subchains. Each `SubchainKey` carries its own pre-baked
/// `selector_clause` (synthesized in the scene model from typed
/// `SubchainKind`/`SubchainLabel`); this function only groups by
/// `obj_name` and joins.
pub(super) fn build_subchains_expr(subchains: &[SubchainKey]) -> Option<String> {
    if subchains.is_empty() {
        return None;
    }

    // Group by object name, preserving insertion order; collect distinct
    // selector clauses per object (empty clause = whole-object scope).
    let mut obj_groups: Vec<(&str, Vec<&str>)> = Vec::new();
    for key in subchains {
        let clause = key.selector_clause.as_str();
        if let Some(entry) = obj_groups
            .iter_mut()
            .find(|(k, _)| *k == key.obj_name.as_str())
        {
            if !entry.1.contains(&clause) {
                entry.1.push(clause);
            }
        } else {
            obj_groups.push((key.obj_name.as_str(), vec![clause]));
        }
    }

    let mut obj_exprs: Vec<String> = Vec::new();
    for (obj, clauses) in &obj_groups {
        // Whole-object selection wins: if any selected row covers the
        // whole object, all other clauses for that object are redundant.
        if clauses.iter().any(|c| c.is_empty()) {
            obj_exprs.push((*obj).to_string());
            continue;
        }
        let expr = if clauses.len() == 1 {
            format!("{} and {}", obj, clauses[0])
        } else {
            let joined = clauses.to_vec().join(" or ");
            format!("{} and ({})", obj, joined)
        };
        obj_exprs.push(expr);
    }

    if obj_exprs.len() == 1 {
        Some(obj_exprs.into_iter().next().unwrap())
    } else {
        let parts: Vec<String> = obj_exprs.into_iter().map(|e| format!("({})", e)).collect();
        Some(parts.join(" or "))
    }
}

/// Build a selection expression for a single subchain.
pub(super) fn build_single_subchain_expr(key: &SubchainKey) -> String {
    if key.selector_clause.is_empty() {
        key.obj_name.clone()
    } else {
        format!("{} and {}", key.obj_name, key.selector_clause)
    }
}

pub(super) fn visible_order_groups(scene: &SceneModel) -> Vec<String> {
    scene
        .entries
        .iter()
        .filter_map(|e| match e {
            SceneEntry::Group(g) => Some(g.name.clone()),
            _ => None,
        })
        .collect()
}

pub(super) fn visible_order_objects(scene: &SceneModel) -> Vec<String> {
    let mut order = Vec::new();
    for entry in &scene.entries {
        match entry {
            SceneEntry::Object(obj) => order.push(obj.name.clone()),
            SceneEntry::Group(g) if g.open => {
                for child in &g.children {
                    order.push(child.name.clone());
                }
            }
            _ => {}
        }
    }
    order
}

pub(super) fn visible_order_subchains(scene: &SceneModel) -> Vec<SubchainKey> {
    fn push_obj(order: &mut Vec<SubchainKey>, obj: &patinae_framework::model::scene::SceneObject) {
        for sub in &obj.subchains {
            order.push(SubchainKey {
                obj_name: obj.name.clone(),
                chain_id: sub.chain_id.clone(),
                label: sub.display_label().to_string(),
                kind: sub.kind.as_str().to_string(),
                entry_index: sub.entry_index,
                selector_clause: sub.selector_clause.clone(),
            });
        }
    }

    let mut order = Vec::new();
    for entry in &scene.entries {
        match entry {
            SceneEntry::Object(obj) if obj.expanded => push_obj(&mut order, obj),
            SceneEntry::Group(g) if g.open => {
                for child in &g.children {
                    if child.expanded {
                        push_obj(&mut order, child);
                    }
                }
            }
            _ => {}
        }
    }
    order
}

pub(super) fn visible_order_selections(bridge: &ObjectsBridge) -> Vec<String> {
    let count = bridge.selections_model.row_count();
    let mut order = Vec::with_capacity(count);
    for i in 0..count {
        if let Some(row) = bridge.selections_model.row_data(i) {
            order.push(row.name.to_string());
        }
    }
    order
}

/// Select range between two items in an ordered list.
pub(super) fn select_range<T: PartialEq + Clone>(order: &[T], anchor: &T, target: &T) -> Vec<T> {
    let a_pos = order.iter().position(|x| x == anchor);
    let t_pos = order.iter().position(|x| x == target);
    match (a_pos, t_pos) {
        (Some(a), Some(t)) => {
            let (lo, hi) = if a <= t { (a, t) } else { (t, a) };
            order[lo..=hi].to_vec()
        }
        _ => vec![target.clone()],
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "UI selection helper keeps each click input explicit at call sites"
)]
pub(super) fn handle_click<T: PartialEq + Clone, A: Clone>(
    selected: &mut Vec<T>,
    target: T,
    anchor_key: A,
    resolve_anchor: impl FnOnce(&A) -> Option<T>,
    order: &[T],
    shift: bool,
    meta: bool,
    anchor: &mut Option<A>,
) -> bool {
    if shift {
        if let Some(anchor_item) = anchor.as_ref().and_then(resolve_anchor) {
            *selected = select_range(order, &anchor_item, &target);
        } else {
            *selected = vec![target];
            *anchor = Some(anchor_key);
        }
    } else if meta {
        if let Some(pos) = selected.iter().position(|x| x == &target) {
            selected.remove(pos);
        } else {
            selected.push(target);
        }
        *anchor = Some(anchor_key);
    } else {
        *selected = vec![target];
        *anchor = Some(anchor_key);
    }
    selected.is_empty()
}

pub(super) fn scene_has_subchain(scene: &SceneModel, key: &SubchainKey) -> bool {
    scene.get(&key.obj_name).is_some_and(|obj| {
        obj.subchains.iter().any(|sub| {
            sub.chain_id.as_str() == key.chain_id.as_str()
                && sub.display_label() == key.label.as_str()
                && sub.kind.as_str() == key.kind.as_str()
                && sub.entry_index == key.entry_index
                && sub.selector_clause.as_str() == key.selector_clause.as_str()
        })
    })
}

/// Encode a subchain selection key for anchor tracking.
pub(super) fn subchain_key(obj: &str, chain: &str, label: &str) -> String {
    format!("{}\0{}\0{}", obj, chain, label)
}

/// State of the scene selection domain.
pub(super) struct SceneSelection {
    pub(super) selection_level: SelectionLevel,
    pub(super) selected_groups: Vec<String>,
    pub(super) selected_objects: Vec<String>,
    pub(super) selected_subchains: Vec<SubchainKey>,
    pub(super) anchor: Option<String>,
}

/// State of the named selection domain.
pub(super) struct NamedSelection {
    pub(super) selected_selections: Vec<String>,
    pub(super) selection_anchor: Option<String>,
    pub(super) last_selection_generation: u64,
}
