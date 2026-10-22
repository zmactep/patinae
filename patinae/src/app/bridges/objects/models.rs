//! Models for ObjectsBridge.

use super::*;

impl ObjectsBridge {
    pub(super) fn sync_dock(&mut self, window: &AppWindow) {
        let layout = window.global::<LayoutState>();
        let visible = [
            layout.get_objects_visible(),
            layout.get_selections_visible(),
            layout.get_recent_atoms_visible(),
        ];
        let visibility_changed = self.dock_visible != visible;
        if !self.dock_dirty.replace(false) && !visibility_changed {
            return;
        }
        self.dock_visible = visible;
        let rows = crate::bridges::dock::rows(
            self.top_level_model.as_ref(),
            self.selections_model.as_ref(),
            self.recent_atoms_model.as_ref(),
            visible,
        );
        let old = self.dock_model.iter().collect::<Vec<_>>();
        let os = window.global::<ObjectsState>();
        let metrics = window.global::<ComponentMetrics>();
        // A section toggle changes the list composition deliberately. Anchoring
        // to the old first row would scroll newly shown sections out of view.
        let scroll = if visibility_changed {
            0.0
        } else {
            crate::bridges::dock::restored_scroll(
                &old,
                &rows,
                os.get_dock_scroll_y(),
                metrics.get_section_header_h(),
                metrics.get_row_h(),
            )
        };
        // Preserve row instances (including keyboard focus) on selection-only updates.
        for (index, row) in rows.iter().enumerate() {
            if self
                .dock_model
                .row_data(index)
                .is_some_and(|old| old.key == row.key)
            {
                if self.dock_model.row_data(index).as_ref() != Some(row) {
                    self.dock_model.set_row_data(index, row.clone());
                }
            } else {
                self.dock_model.insert(index, row.clone());
            }
        }
        while self.dock_model.row_count() > rows.len() {
            self.dock_model.remove(rows.len());
        }
        os.set_dock_scroll_y(scroll);
    }

    pub(super) fn rebuild_top_level_model(&self, scene: &SceneModel) {
        self.dock_dirty.set(true);
        let mut rows: Vec<TopLevelRow> = Vec::new();

        for entry in &scene.entries {
            match entry {
                SceneEntry::Group(group) => {
                    let children: Vec<ObjectItem> = group
                        .children
                        .iter()
                        .map(|obj| self.build_object_item(obj))
                        .collect();

                    rows.push(TopLevelRow {
                        is_group: true,
                        group_name: group.name.clone().into(),
                        group_open: group.open,
                        group_enabled: group.enabled,
                        group_selected: self.scene_selection.selected_groups.contains(&group.name),
                        group_child_count: children.len() as i32,
                        group_objects: ModelRc::from(Rc::new(VecModel::from(children))),
                        object: ObjectItem::default(),
                    });
                }
                SceneEntry::Object(obj) => {
                    rows.push(TopLevelRow {
                        is_group: false,
                        group_name: Default::default(),
                        group_open: false,
                        group_enabled: false,
                        group_selected: false,
                        group_child_count: 0,
                        group_objects: ModelRc::default(),
                        object: self.build_object_item(obj),
                    });
                }
            }
        }

        replace_model(&self.top_level_model, rows);
    }

    pub(super) fn build_object_item(
        &self,
        obj: &patinae_framework::model::scene::SceneObject,
    ) -> ObjectItem {
        let subchains: Vec<SubchainItem> = obj
            .subchains
            .iter()
            .map(|sub| {
                let label = sub.display_label().to_string();
                let selected = self.scene_selection.selected_subchains.iter().any(|k| {
                    k.obj_name == obj.name
                        && k.chain_id == sub.chain_id
                        && k.label == label
                        && k.kind == sub.kind.as_str()
                });
                SubchainItem {
                    chain_id: sub.chain_id.clone().into(),
                    label: label.into(),
                    kind: sub.kind.as_str().into(),
                    atom_count: sub.atom_count as i32,
                    color: sidebar_color_to_slint(sub.color),
                    multicolor: matches!(sub.color, SidebarColor::Multicolor),
                    selected,
                    entry_index: sub.entry_index as i32,
                    selector_clause: sub.selector_clause.clone().into(),
                }
            })
            .collect();

        const MAX_VISIBLE_SUBCHAINS: usize = 100;
        let overflow_count = subchains.len().saturating_sub(MAX_VISIBLE_SUBCHAINS) as i32;
        let subchains: Vec<SubchainItem> =
            subchains.into_iter().take(MAX_VISIBLE_SUBCHAINS).collect();

        ObjectItem {
            name: obj.name.clone().into(),
            object_type: match obj.kind {
                SceneObjectKind::Molecule => "molecule".into(),
                SceneObjectKind::Map => "map".into(),
                SceneObjectKind::Measurement => "measurement".into(),
                SceneObjectKind::Label => "label".into(),
            },
            object_icon_kind: object_icon_kind(obj).into(),
            measurement_kind: measurement_kind_name(obj.measurement_kind).into(),
            entity_count: i32::try_from(obj.entity_count).unwrap_or(i32::MAX),
            has_unresolved_entities: obj.has_unresolved_entities,
            focus_disabled_reason: obj
                .focus_disabled_reason
                .as_deref()
                .unwrap_or_default()
                .into(),
            enabled: obj.enabled,
            expanded: obj.expanded,
            selected: self.scene_selection.selected_objects.contains(&obj.name),
            atom_count: i32::try_from(obj.displayed_atom_count).unwrap_or(i32::MAX),
            stored_atom_count: i32::try_from(obj.stored_atom_count).unwrap_or(i32::MAX),
            instanced: obj.storage_mode == patinae_mol::StorageMode::Instanced,
            color: sidebar_color_to_slint(obj.color),
            multicolor: matches!(obj.color, SidebarColor::Multicolor),
            has_representations: obj.capabilities.representations,
            can_color: obj.capabilities.color,
            can_copy: obj.capabilities.copy,
            can_extract: obj.capabilities.extract,
            can_align: obj.capabilities.align,
            can_orient: obj.capabilities.orient,
            can_remove_atoms: obj.capabilities.remove_atoms,
            can_delete: obj.capabilities.delete,
            can_rename: obj.capabilities.rename,
            can_group: obj.capabilities.grouping,
            can_focus: obj.capabilities.focus,
            can_toggle: obj.capabilities.visibility,
            subchains: ModelRc::from(Rc::new(VecModel::from(subchains))),
            overflow_count,
        }
    }

    pub(super) fn rebuild_selections(&mut self, kernel: &AppKernel) -> bool {
        self.dock_dirty.set(true);
        let sel_mgr = &kernel.session.selections;
        let mut rows: Vec<SelectionRow> = Vec::new();
        let mut surviving_names: Vec<String> = Vec::new();
        let before_selected = self.named_selection.selected_selections.clone();
        let before_anchor = self.named_selection.selection_anchor.clone();

        for name in sel_mgr.names() {
            let expr = sel_mgr.get_expression(&name).unwrap_or("").to_string();
            let entry = sel_mgr.get(&name);

            let atom_count = if let Some(entry) = entry {
                entry
                    .cached_results
                    .iter()
                    .filter_map(|(obj_name, result)| {
                        let mol = kernel.session.registry.get_molecule(obj_name)?;
                        (result.atom_count() == mol.molecule().atom_count()).then(|| result.count())
                    })
                    .sum::<usize>() as i32
            } else {
                0
            };

            let enabled = entry.map(|e| e.visible).unwrap_or(false);
            let selected = self.named_selection.selected_selections.contains(&name);

            if selected {
                surviving_names.push(name.clone());
            }

            rows.push(SelectionRow {
                name: name.into(),
                expression: expr.into(),
                atom_count,
                residue_count: 0,
                enabled,
                selected,
            });
        }

        // Prune stale selections (deleted while selected)
        self.named_selection
            .selected_selections
            .retain(|n| surviving_names.contains(n));
        if self.named_selection.selected_selections.is_empty() {
            self.named_selection.selection_anchor = None;
        }

        replace_model(&self.selections_model, rows);

        self.named_selection.selected_selections != before_selected
            || self.named_selection.selection_anchor != before_anchor
    }
}
pub(super) fn replace_model<T: Clone + 'static>(model: &Rc<VecModel<T>>, rows: Vec<T>) {
    model.set_vec(rows);
}

/// Map semantic SidebarColor to a concrete Slint color.
pub(super) fn sidebar_color_to_slint(sc: SidebarColor) -> slint::Color {
    match sc {
        SidebarColor::Solvent => slint::Color::from_rgb_u8(102, 153, 255),
        SidebarColor::Other => slint::Color::from_rgb_u8(128, 128, 128),
        SidebarColor::Multicolor => slint::Color::from_rgb_u8(128, 128, 128),
        SidebarColor::Color(c) => slint::Color::from_rgb_u8(
            (c.r * 255.0) as u8,
            (c.g * 255.0) as u8,
            (c.b * 255.0) as u8,
        ),
    }
}
