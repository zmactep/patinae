//! Recent for ObjectsBridge.

use super::*;

impl ObjectsBridge {
    pub(super) fn sync_recent_atoms(&mut self, recent_atoms: &RecentAtoms) -> bool {
        self.dock_dirty.set(true);
        let observation = (recent_atoms.incarnation(), recent_atoms.generation());
        if self.recent.last_recent_observation == Some(observation) {
            return false;
        }

        let first_observation = self.recent.last_recent_observation.is_none();
        let previous_operands = self.recent.recent_operands.clone();
        let previous_anchor = self.recent.recent_anchor;
        let replaced = self
            .recent
            .last_recent_observation
            .is_some_and(|(incarnation, _)| incarnation != observation.0);
        let appended_one =
            self.recent
                .last_recent_observation
                .is_some_and(|(incarnation, generation)| {
                    incarnation == observation.0
                        && generation.checked_add(1) == Some(observation.1)
                        && self.recent.recent_bindings.len().checked_add(1)
                            == Some(recent_atoms.len())
                });
        if replaced {
            self.recent.recent_operands.clear();
            self.recent.recent_anchor = None;
        }

        self.recent.last_recent_observation = Some(observation);
        if first_observation || replaced {
            self.recent.recent_bindings.clear();
            for row in recent_atoms.rows() {
                let key = self.allocate_recent_key();
                self.recent.recent_bindings.push(RecentAtomBinding {
                    key,
                    id: row.id(),
                    path: row.path().to_string(),
                });
            }
            self.reset_recent_atoms_model();
        } else if appended_one {
            let row = recent_atoms
                .rows()
                .last()
                .expect("single recent atom append has a final row");
            let binding = RecentAtomBinding {
                key: self.allocate_recent_key(),
                id: row.id(),
                path: row.path().to_string(),
            };
            let model_row = recent_atom_item(&binding, &self.recent.recent_operands);
            self.recent.recent_bindings.push(binding);
            self.recent_atoms_model.push(model_row);
        } else {
            let live_ids = recent_atoms
                .rows()
                .iter()
                .map(|row| row.id())
                .collect::<HashSet<_>>();
            self.recent
                .recent_operands
                .retain(|id| live_ids.contains(id));
            if self
                .recent
                .recent_anchor
                .is_some_and(|id| !live_ids.contains(&id))
            {
                self.recent.recent_anchor = None;
            }
            self.reconcile_recent_atoms_model(recent_atoms, &previous_operands, &live_ids);
        }

        self.recent.recent_operands != previous_operands
            || self.recent.recent_anchor != previous_anchor
    }

    pub(super) fn allocate_recent_key(&mut self) -> String {
        let key = format!("recent-atom-{}", self.recent.next_recent_key);
        self.recent.next_recent_key = self
            .recent
            .next_recent_key
            .checked_add(1)
            .expect("recent atom desktop row keys exhausted");
        key
    }

    pub(super) fn reset_recent_atoms_model(&self) {
        let rows = self
            .recent
            .recent_bindings
            .iter()
            .map(|binding| recent_atom_item(binding, &self.recent.recent_operands))
            .collect();
        replace_model(&self.recent_atoms_model, rows);
    }

    pub(super) fn reconcile_recent_atoms_model(
        &mut self,
        recent_atoms: &RecentAtoms,
        previous_operands: &[RecentAtomId],
        live_ids: &HashSet<RecentAtomId>,
    ) {
        for index in (0..self.recent.recent_bindings.len()).rev() {
            if !live_ids.contains(&self.recent.recent_bindings[index].id) {
                self.recent.recent_bindings.remove(index);
                self.recent_atoms_model.remove(index);
            }
        }

        let survivor_count = self.recent.recent_bindings.len();
        assert!(
            survivor_count <= recent_atoms.len()
                && self
                    .recent
                    .recent_bindings
                    .iter()
                    .zip(recent_atoms.rows())
                    .all(|(binding, row)| binding.id == row.id()),
            "recent atom mutations must preserve survivor order"
        );

        for index in 0..survivor_count {
            let next_row = &recent_atoms.rows()[index];
            let path_changed = self.recent.recent_bindings[index].path != next_row.path();
            let operand_changed =
                recent_operand_position(self.recent.recent_bindings[index].id, previous_operands)
                    != recent_operand_position(
                        self.recent.recent_bindings[index].id,
                        &self.recent.recent_operands,
                    );
            if path_changed {
                self.recent.recent_bindings[index].path = next_row.path().to_string();
            }
            if path_changed || operand_changed {
                self.update_recent_atom_model_row(index);
            }
        }

        for next_row in &recent_atoms.rows()[survivor_count..] {
            let binding = RecentAtomBinding {
                key: self.allocate_recent_key(),
                id: next_row.id(),
                path: next_row.path().to_string(),
            };
            let row = recent_atom_item(&binding, &self.recent.recent_operands);
            self.recent.recent_bindings.push(binding);
            self.recent_atoms_model.push(row);
        }
    }

    pub(super) fn update_recent_atom_model_row(&self, index: usize) {
        let Some(binding) = self.recent.recent_bindings.get(index) else {
            return;
        };
        let next = recent_atom_item(binding, &self.recent.recent_operands);
        if self.recent_atoms_model.row_data(index) != Some(next.clone()) {
            self.recent_atoms_model.set_row_data(index, next);
        }
    }

    pub(super) fn update_recent_operand_rows(&self, previous_operands: &[RecentAtomId]) {
        let affected: HashSet<RecentAtomId> = previous_operands
            .iter()
            .chain(&self.recent.recent_operands)
            .copied()
            .collect();
        for (index, binding) in self.recent.recent_bindings.iter().enumerate() {
            if affected.contains(&binding.id) {
                self.update_recent_atom_model_row(index);
            }
        }
    }

    pub(super) fn click_recent_atom(
        &mut self,
        target: RecentAtomId,
        shift: bool,
        meta: bool,
    ) -> bool {
        debug_assert!(self
            .recent
            .recent_bindings
            .iter()
            .any(|binding| binding.id == target));
        let previous_operands = self.recent.recent_operands.clone();
        if shift {
            let anchor_index = self.recent.recent_anchor.and_then(|anchor| {
                self.recent
                    .recent_bindings
                    .iter()
                    .position(|binding| binding.id == anchor)
            });
            let target_index = self
                .recent
                .recent_bindings
                .iter()
                .position(|binding| binding.id == target);
            match (anchor_index, target_index) {
                (Some(anchor), Some(target)) => {
                    let (lo, hi) = if anchor <= target {
                        (anchor, target)
                    } else {
                        (target, anchor)
                    };
                    let start = (hi + 1).saturating_sub(MAX_RECENT_OPERANDS).max(lo);
                    self.recent.recent_operands = self.recent.recent_bindings[start..=hi]
                        .iter()
                        .map(|binding| binding.id)
                        .collect();
                }
                _ => {
                    self.recent.recent_operands = vec![target];
                    self.recent.recent_anchor = Some(target);
                }
            }
        } else {
            handle_click(
                &mut self.recent.recent_operands,
                target,
                target,
                |_| None,
                &[],
                false,
                meta,
                &mut self.recent.recent_anchor,
            );
        }
        retain_newest_operands(&mut self.recent.recent_operands);
        if self.recent.recent_operands == previous_operands {
            return false;
        }
        self.update_recent_operand_rows(&previous_operands);
        true
    }

    pub(super) fn clear_recent_operands(&mut self) -> bool {
        let previous_operands = std::mem::take(&mut self.recent.recent_operands);
        self.recent.recent_anchor = None;
        if previous_operands.is_empty() {
            return false;
        }
        self.update_recent_operand_rows(&previous_operands);
        true
    }

    pub(super) fn recent_id_for_key(&self, key: &str) -> Option<RecentAtomId> {
        self.recent
            .recent_bindings
            .iter()
            .find(|binding| binding.key == key)
            .map(|binding| binding.id)
    }

    pub(super) fn recent_path_for_key(&self, key: &str) -> Option<&str> {
        self.recent
            .recent_bindings
            .iter()
            .find(|binding| binding.key == key)
            .map(|binding| binding.path.as_str())
    }

    pub(super) fn recent_operand_paths(&self) -> Option<Vec<String>> {
        self.recent
            .recent_operands
            .iter()
            .map(|id| {
                self.recent
                    .recent_bindings
                    .iter()
                    .find(|binding| binding.id == *id)
                    .map(|binding| binding.path.clone())
            })
            .collect()
    }

    pub(super) fn measurement_request(
        &self,
        target: MeasurementTarget,
    ) -> Option<MeasurementRequest> {
        let operands = self.recent_operand_paths()?;
        (2..=4)
            .contains(&operands.len())
            .then(|| MeasurementRequest::new(operands, target))
    }

    pub(super) fn label_request(
        &self,
        expression: LabelExpression,
        target: LabelTarget,
    ) -> Option<LabelRequest> {
        let operands = self.recent_operand_paths()?;
        (1..=4)
            .contains(&operands.len())
            .then(|| LabelRequest::new(operands, expression, target))
    }

    pub(super) fn compatible_measurement_targets(&self, registry: &ObjectRegistry) -> Vec<String> {
        let Ok(kind) = patinae_cmd::measurement_kind_for_count(self.recent.recent_operands.len())
        else {
            return Vec::new();
        };
        registry
            .names()
            .filter(|name| {
                registry
                    .get_measurement(name)
                    .is_some_and(|measurement| measurement.kind() == kind)
            })
            .map(str::to_string)
            .collect()
    }

    pub(super) fn label_targets(&self, registry: &ObjectRegistry) -> Vec<String> {
        registry
            .names()
            .filter(|name| registry.get_label(name).is_some())
            .map(str::to_string)
            .collect()
    }
}
pub(super) fn recent_atom_item(
    binding: &RecentAtomBinding,
    recent_operands: &[RecentAtomId],
) -> RecentAtomItem {
    let operand_position = recent_operand_position(binding.id, recent_operands);
    RecentAtomItem {
        key: binding.key.clone().into(),
        path: binding.path.clone().into(),
        display_path: display_atom_path(&binding.path).into(),
        selected: operand_position > 0,
        operand_position,
    }
}

pub(super) fn recent_operand_position(id: RecentAtomId, recent_operands: &[RecentAtomId]) -> i32 {
    recent_operands
        .iter()
        .position(|operand| operand == &id)
        .map_or(0, |position| position as i32 + 1)
}

pub(super) fn retain_newest_operands(operands: &mut Vec<RecentAtomId>) {
    let excess = operands.len().saturating_sub(MAX_RECENT_OPERANDS);
    operands.drain(..excess);
}

pub(super) fn label_expression_from_key(key: &str, literal: &str) -> Option<LabelExpression> {
    if key == "literal" {
        let literal = literal.trim();
        return (!literal.is_empty()).then(|| LabelExpression::Literal(literal.to_string()));
    }
    LabelExpression::from_builtin_key(key)
}

pub(super) fn label_popover_targets_invalidated(
    shown_targets: &ModelRc<AnnotationTargetItem>,
    live_targets: &[String],
) -> bool {
    if shown_targets.row_count() != live_targets.len() {
        return true;
    }

    let shown_names = (0..shown_targets.row_count())
        .filter_map(|index| shown_targets.row_data(index))
        .map(|target| target.name.to_string())
        .collect::<HashSet<_>>();
    let live_names = live_targets.iter().cloned().collect::<HashSet<_>>();
    shown_names != live_names
}

pub(super) fn annotation_target_model(names: Vec<String>) -> ModelRc<AnnotationTargetItem> {
    let rows = names
        .into_iter()
        .map(|name| AnnotationTargetItem { name: name.into() })
        .collect::<Vec<_>>();
    ModelRc::from(Rc::new(VecModel::from(rows)))
}

/// State of the recent domain.
pub(super) struct RecentSelection {
    pub(super) recent_bindings: Vec<RecentAtomBinding>,
    pub(super) recent_operands: Vec<RecentAtomId>,
    pub(super) recent_anchor: Option<RecentAtomId>,
    pub(super) last_recent_observation: Option<(u64, u64)>,
    pub(super) next_recent_key: u64,
}
