//! Typed objects events and their UI-thread handler.

use super::*;
use crate::app::events::{EventSender, GuiEvent};

#[derive(Debug)]
pub(crate) enum Event {
    GroupClicked(slint::SharedString, bool, bool),
    ObjectClicked(slint::SharedString, bool, bool),
    ObjectListBackgroundClicked,
    SubchainClicked(
        slint::SharedString,
        slint::SharedString,
        slint::SharedString,
        slint::SharedString,
        i32,
        slint::SharedString,
        bool,
        bool,
    ),
    ToggleGroupOpen(slint::SharedString),
    ToggleObjectExpand(slint::SharedString),
    SelectionClicked(slint::SharedString, bool, bool),
    SelectionListBackgroundClicked,
    RecentAtomClicked(slint::SharedString, bool, bool),
    RecentAtomsBackgroundClicked,
    RecentAtomRemoveClicked(slint::SharedString),
    RecentAtomsClear,
    ActionAddMeasurement,
    ActionAddMeasurementTo,
    MeasurementTargetConfirm(slint::SharedString),
    ActionAddLabel,
    LabelConfirm(
        slint::SharedString,
        slint::SharedString,
        slint::SharedString,
    ),
    RowBtnClicked(
        slint::SharedString,
        slint::SharedString,
        slint::SharedString,
        slint::SharedString,
        slint::SharedString,
        slint::SharedString,
        i32,
        slint::SharedString,
    ),
    PopoverClose,
    PopoverExecute {
        cmd: String,
        kind: String,
        target: String,
    },
    ActionZoom,
    ActionOrient,
    ActionCenter,
    ActionToggle,
    ActionOverflow,
    OverflowAction(slint::SharedString),
    NameInputConfirm {
        action: String,
        target: String,
        new_name: String,
    },
    ObjectRightClicked(slint::SharedString),
    SelectionRightClicked(slint::SharedString),
}

pub(crate) fn setup_callbacks(sender: EventSender, window: &AppWindow) {
    let state = window.global::<ObjectsState>();
    {
        let sender = sender.clone();
        state.on_group_clicked(move |arg0, arg1, arg2| {
            sender.send(GuiEvent::Objects(Event::GroupClicked(arg0, arg1, arg2)))
        });
    }
    {
        let sender = sender.clone();
        state.on_object_clicked(move |arg0, arg1, arg2| {
            sender.send(GuiEvent::Objects(Event::ObjectClicked(arg0, arg1, arg2)))
        });
    }
    {
        let sender = sender.clone();
        state.on_object_list_background_clicked(move || {
            sender.send(GuiEvent::Objects(Event::ObjectListBackgroundClicked))
        });
    }
    {
        let sender = sender.clone();
        state.on_subchain_clicked(move |arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7| {
            sender.send(GuiEvent::Objects(Event::SubchainClicked(
                arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7,
            )))
        });
    }
    {
        let sender = sender.clone();
        state.on_toggle_group_open(move |arg0| {
            sender.send(GuiEvent::Objects(Event::ToggleGroupOpen(arg0)))
        });
    }
    {
        let sender = sender.clone();
        state.on_toggle_object_expand(move |arg0| {
            sender.send(GuiEvent::Objects(Event::ToggleObjectExpand(arg0)))
        });
    }
    {
        let sender = sender.clone();
        state.on_selection_clicked(move |arg0, arg1, arg2| {
            sender.send(GuiEvent::Objects(Event::SelectionClicked(arg0, arg1, arg2)))
        });
    }
    {
        let sender = sender.clone();
        state.on_selection_list_background_clicked(move || {
            sender.send(GuiEvent::Objects(Event::SelectionListBackgroundClicked))
        });
    }
    {
        let sender = sender.clone();
        state.on_recent_atom_clicked(move |arg0, arg1, arg2| {
            sender.send(GuiEvent::Objects(Event::RecentAtomClicked(
                arg0, arg1, arg2,
            )))
        });
    }
    {
        let sender = sender.clone();
        state.on_recent_atoms_background_clicked(move || {
            sender.send(GuiEvent::Objects(Event::RecentAtomsBackgroundClicked))
        });
    }
    {
        let sender = sender.clone();
        state.on_recent_atom_remove_clicked(move |arg0| {
            sender.send(GuiEvent::Objects(Event::RecentAtomRemoveClicked(arg0)))
        });
    }
    {
        let sender = sender.clone();
        state
            .on_recent_atoms_clear(move || sender.send(GuiEvent::Objects(Event::RecentAtomsClear)));
    }
    {
        let sender = sender.clone();
        state.on_action_add_measurement(move || {
            sender.send(GuiEvent::Objects(Event::ActionAddMeasurement))
        });
    }
    {
        let sender = sender.clone();
        state.on_action_add_measurement_to(move || {
            sender.send(GuiEvent::Objects(Event::ActionAddMeasurementTo))
        });
    }
    {
        let sender = sender.clone();
        state.on_measurement_target_confirm(move |arg0| {
            sender.send(GuiEvent::Objects(Event::MeasurementTargetConfirm(arg0)))
        });
    }
    {
        let sender = sender.clone();
        state.on_action_add_label(move || sender.send(GuiEvent::Objects(Event::ActionAddLabel)));
    }
    {
        let sender = sender.clone();
        state.on_label_confirm(move |arg0, arg1, arg2| {
            sender.send(GuiEvent::Objects(Event::LabelConfirm(arg0, arg1, arg2)))
        });
    }
    {
        let sender = sender.clone();
        state.on_row_btn_clicked(move |arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7| {
            sender.send(GuiEvent::Objects(Event::RowBtnClicked(
                arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7,
            )))
        });
    }
    {
        let sender = sender.clone();
        state.on_popover_close(move || sender.send(GuiEvent::Objects(Event::PopoverClose)));
    }
    {
        let sender = sender.clone();
        let weak = window.as_weak();
        state.on_popover_execute(move || {
            if let Some(w) = weak.upgrade() {
                let os = w.global::<ObjectsState>();
                sender.send(GuiEvent::Objects(Event::PopoverExecute {
                    cmd: os.get_popover_cmd_preview().to_string(),
                    kind: os.get_popover_kind().to_string(),
                    target: os.get_popover_target().to_string(),
                }));
            }
        });
    }
    {
        let sender = sender.clone();
        state.on_action_zoom(move || sender.send(GuiEvent::Objects(Event::ActionZoom)));
    }
    {
        let sender = sender.clone();
        state.on_action_orient(move || sender.send(GuiEvent::Objects(Event::ActionOrient)));
    }
    {
        let sender = sender.clone();
        state.on_action_center(move || sender.send(GuiEvent::Objects(Event::ActionCenter)));
    }
    {
        let sender = sender.clone();
        state.on_action_toggle(move || sender.send(GuiEvent::Objects(Event::ActionToggle)));
    }
    {
        let sender = sender.clone();
        state.on_action_overflow(move || sender.send(GuiEvent::Objects(Event::ActionOverflow)));
    }
    {
        let sender = sender.clone();
        state.on_overflow_action(move |arg0| {
            sender.send(GuiEvent::Objects(Event::OverflowAction(arg0)))
        });
    }
    {
        let sender = sender.clone();
        let weak = window.as_weak();
        state.on_name_input_confirm(move || {
            if let Some(w) = weak.upgrade() {
                let os = w.global::<ObjectsState>();
                sender.send(GuiEvent::Objects(Event::NameInputConfirm {
                    action: os.get_name_input_action().to_string(),
                    target: os.get_name_input_target().to_string(),
                    new_name: os.get_name_input_text().to_string(),
                }));
            }
        });
    }
    {
        let sender = sender.clone();
        state.on_object_right_clicked(move |arg0| {
            sender.send(GuiEvent::Objects(Event::ObjectRightClicked(arg0)))
        });
    }
    {
        let sender = sender.clone();
        state.on_selection_right_clicked(move |arg0| {
            sender.send(GuiEvent::Objects(Event::SelectionRightClicked(arg0)))
        });
    }
    state.on_label_literal_valid(|literal| !literal.as_str().trim().is_empty());
}

pub(in crate::app) fn dispatch(
    event: Event,
    objects: &mut ObjectsBridge,
    kernel: &mut AppKernel,
    annotations: &mut std::collections::VecDeque<AnnotationRequest>,
    window: &AppWindow,
) {
    let weak = window.as_weak();
    match event {
        Event::GroupClicked(name, shift, meta) => {
            let Some(w) = weak.upgrade() else { return };
            let os = w.global::<ObjectsState>();
            let name = name.to_string();

            let order = visible_order_groups(&kernel.scene);
            objects.click_groups(name, &order, shift, meta);

            objects.update_slint_selection(&os, &kernel.scene, &kernel.session.registry);
        }
        Event::ObjectClicked(name, shift, meta) => {
            let Some(w) = weak.upgrade() else { return };
            let os = w.global::<ObjectsState>();
            let name = name.to_string();

            let order = visible_order_objects(&kernel.scene);
            objects.click_objects(name, &order, shift, meta);

            objects.update_slint_selection(&os, &kernel.scene, &kernel.session.registry);
        }
        Event::ObjectListBackgroundClicked => {
            let Some(w) = weak.upgrade() else { return };
            let os = w.global::<ObjectsState>();
            objects.clear_scene_selection();
            objects.update_slint_selection(&os, &kernel.scene, &kernel.session.registry);
        }
        Event::SubchainClicked(
            obj_name,
            chain_id,
            label,
            kind,
            entry_index,
            selector_clause,
            shift,
            meta,
        ) => {
            let Some(w) = weak.upgrade() else { return };
            let os = w.global::<ObjectsState>();
            let obj_name = obj_name.to_string();
            let chain_id = chain_id.to_string();
            let label = label.to_string();
            let kind = kind.to_string();
            let key = subchain_key(&obj_name, &chain_id, &label);
            let target = SubchainKey {
                obj_name: obj_name.clone(),
                chain_id: chain_id.clone(),
                label: label.clone(),
                kind: kind.clone(),
                entry_index: entry_index.max(0) as u32,
                selector_clause: selector_clause.to_string(),
            };

            let order = visible_order_subchains(&kernel.scene);
            objects.click_subchains(target, key, &order, shift, meta);

            objects.update_slint_selection(&os, &kernel.scene, &kernel.session.registry);
        }
        Event::ToggleGroupOpen(name) => {
            let name = name.to_string();
            kernel.scene.toggle_group_open(&name);
            kernel.scene.invalidate();
        }
        Event::ToggleObjectExpand(name) => {
            let name = name.to_string();
            kernel.scene.toggle_expanded(&name);
            kernel.scene.invalidate();
        }
        Event::SelectionClicked(name, shift, meta) => {
            let Some(w) = weak.upgrade() else { return };
            let os = w.global::<ObjectsState>();
            let name = name.to_string();

            let order = visible_order_selections(objects);
            objects.click_selections(name, &order, shift, meta);

            objects.update_slint_selection(&os, &kernel.scene, &kernel.session.registry);
        }
        Event::SelectionListBackgroundClicked => {
            let Some(w) = weak.upgrade() else { return };
            let os = w.global::<ObjectsState>();
            objects.clear_named_selection();
            objects.update_slint_selection(&os, &kernel.scene, &kernel.session.registry);
        }
        Event::RecentAtomClicked(key, shift, meta) => {
            let Some(w) = weak.upgrade() else { return };
            let os = w.global::<ObjectsState>();
            let Some(target) = objects.recent_id_for_key(key.as_str()) else {
                return;
            };
            if objects.click_recent_atom(target, shift, meta) {
                objects.update_slint_selection(&os, &kernel.scene, &kernel.session.registry);
            }
        }
        Event::RecentAtomsBackgroundClicked => {
            let Some(w) = weak.upgrade() else { return };
            let os = w.global::<ObjectsState>();
            if objects.clear_recent_operands() {
                objects.update_slint_selection(&os, &kernel.scene, &kernel.session.registry);
            }
        }
        Event::RecentAtomRemoveClicked(key) => {
            let Some(path) = objects.recent_path_for_key(key.as_str()) else {
                return;
            };
            let command = format!("unpick {}", quote_command_arg(path));
            kernel.bus.execute_command(command);
        }
        Event::RecentAtomsClear => {
            kernel.bus.execute_command("unpick");
        }
        Event::ActionAddMeasurement => {
            let Some(w) = weak.upgrade() else { return };

            objects.sync_recent_atoms(&kernel.session.recent_atoms);
            match objects
                .measurement_request(MeasurementTarget::New)
                .map(AnnotationRequest::Measurement)
            {
                Some(request) => annotations.push_back(request),
                None => kernel
                    .output
                    .print_error("Recent atom operands changed; select them again"),
            }
            w.window().request_redraw();
        }
        Event::ActionAddMeasurementTo => {
            let Some(w) = weak.upgrade() else { return };
            let os = w.global::<ObjectsState>();
            if os.get_popover_kind() == "AM" {
                os.set_popover_kind("".into());
                return;
            }

            objects.sync_recent_atoms(&kernel.session.recent_atoms);
            let targets = objects.compatible_measurement_targets(&kernel.session.registry);
            if targets.is_empty() {
                os.set_popover_kind("".into());
                return;
            }
            os.set_measurement_targets(annotation_target_model(targets));
            os.set_popover_source("pill".into());
            os.set_popover_kind("AM".into());
        }
        Event::MeasurementTargetConfirm(target) => {
            let Some(w) = weak.upgrade() else { return };
            let os = w.global::<ObjectsState>();
            let target = target.to_string();

            objects.sync_recent_atoms(&kernel.session.recent_atoms);
            match objects.measurement_request(MeasurementTarget::Existing(target)) {
                Some(request) => {
                    annotations.push_back(AnnotationRequest::Measurement(request));
                    w.window().request_redraw();
                }
                None => kernel
                    .output
                    .print_error("Recent atom operands changed; select them again"),
            }
            os.set_popover_kind("".into());
        }
        Event::ActionAddLabel => {
            let Some(w) = weak.upgrade() else { return };
            let os = w.global::<ObjectsState>();
            if os.get_popover_kind() == "AL" {
                os.set_popover_kind("".into());
                return;
            }

            objects.sync_recent_atoms(&kernel.session.recent_atoms);
            if !(1..=4).contains(&objects.recent.recent_operands.len()) {
                kernel
                    .output
                    .print_error("Recent atom operands changed; select them again");
                os.set_popover_kind("".into());
                w.window().request_redraw();
                return;
            }
            let targets = objects.label_targets(&kernel.session.registry);
            os.set_label_targets(annotation_target_model(targets));
            os.set_popover_source("pill".into());
            os.set_popover_kind("AL".into());
        }
        Event::LabelConfirm(expression, literal, target) => {
            let Some(w) = weak.upgrade() else { return };
            let os = w.global::<ObjectsState>();
            if expression.as_str() == "literal" && literal.as_str().trim().is_empty() {
                kernel
                    .output
                    .print_error("Literal label text cannot be empty");
                return;
            }
            let Some(expression) = label_expression_from_key(expression.as_str(), literal.as_str())
            else {
                kernel.output.print_error("Unknown label expression");
                os.set_popover_kind("".into());
                return;
            };
            let target = if target.is_empty() {
                LabelTarget::New
            } else {
                LabelTarget::Existing(target.to_string())
            };

            objects.sync_recent_atoms(&kernel.session.recent_atoms);
            match objects.label_request(expression, target) {
                Some(request) => {
                    annotations.push_back(AnnotationRequest::Label(request));
                    w.window().request_redraw();
                }
                None => kernel
                    .output
                    .print_error("Recent atom operands changed; select them again"),
            }
            os.set_popover_kind("".into());
        }
        Event::RowBtnClicked(
            kind,
            source,
            obj_name,
            chain_id,
            subchain_label,
            subchain_kind,
            entry_index,
            selector_clause,
        ) => {
            let Some(w) = weak.upgrade() else { return };
            let os = w.global::<ObjectsState>();
            let kind = kind.to_string();
            let source = source.to_string();
            let obj_name = obj_name.to_string();
            let chain_id = chain_id.to_string();
            let subchain_label = subchain_label.to_string();
            let subchain_kind = subchain_kind.to_string();
            let selector_clause = selector_clause.to_string();

            // Toggle: if same popover already open, close it
            if os.get_popover_kind() == kind.as_str()
                && os.get_popover_source() == source.as_str()
                && os.get_popover_obj_name() == obj_name.as_str()
                && os.get_popover_chain_id() == chain_id.as_str()
                && os.get_popover_subchain_label() == subchain_label.as_str()
            {
                os.set_popover_kind("".into());
                return;
            }

            if source == "selection" {
                os.set_popover_solid_only(false);
                // Selection-sourced popover: target is the selection name
                if kind == "R" {
                    set_active_reps_for_selection(
                        &os,
                        &kernel.session.registry,
                        &kernel.session.settings,
                        &kernel.session.selections,
                        &obj_name,
                    );
                }

                os.set_popover_kind(kind.into());
                os.set_popover_source("selection".into());
                os.set_popover_obj_name(obj_name.clone().into());
                os.set_popover_chain_id("".into());
                os.set_popover_subchain_label("".into());
                os.set_popover_subchain_kind("".into());
                os.set_popover_entry_index(-1);
                os.set_popover_selector_clause("".into());
                os.set_popover_target(obj_name.clone().into());
                os.set_popover_display_label(obj_name.into());
                os.set_popover_cmd_preview("".into());
            } else {
                // Object/subchain-sourced popover
                let target = build_popover_target(&obj_name, &selector_clause);
                let label = build_popover_label(&obj_name, &chain_id, &subchain_label);
                let solid_only = { uses_solid_color_popover(&kernel.scene, &obj_name) };
                os.set_popover_solid_only(solid_only);

                // Reset scope to "all" when landing on a disabled scope button.
                let current_scope = os.get_popover_scope().to_string();
                let is_bio = subchain_kind.is_empty() || subchain_kind == "biopolymer";
                let is_bio_or_organic = is_bio || subchain_kind == "organic";
                if solid_only
                    || (!is_bio && current_scope == "cartoon")
                    || (!is_bio_or_organic && current_scope == "all-c")
                {
                    os.set_popover_scope("all".into());
                }

                if kind == "R" {
                    set_active_reps(
                        &os,
                        &kernel.session.registry,
                        &kernel.session.settings,
                        &obj_name,
                        entry_index,
                    );
                }

                os.set_popover_kind(kind.into());
                os.set_popover_source("object".into());
                os.set_popover_obj_name(obj_name.into());
                os.set_popover_chain_id(chain_id.into());
                os.set_popover_subchain_label(subchain_label.into());
                os.set_popover_subchain_kind(subchain_kind.into());
                os.set_popover_entry_index(entry_index);
                os.set_popover_selector_clause(selector_clause.into());
                os.set_popover_target(target.into());
                os.set_popover_display_label(label.into());
                os.set_popover_cmd_preview("".into());
            }
            if os.get_popover_kind() == "C" {
                sync_color_assignments(&os, kernel);
            }
        }
        Event::PopoverClose => {
            let Some(w) = weak.upgrade() else { return };
            let os = w.global::<ObjectsState>();
            os.set_popover_kind("".into());
        }
        Event::PopoverExecute { cmd, kind, target } => {
            let Some(w) = weak.upgrade() else { return };
            let os = w.global::<ObjectsState>();
            if !cmd.is_empty() {
                kernel.bus.execute_command(&cmd);
            }

            // Optimistic update: the command is queued (async), so we can't
            // recompute from atoms yet. Instead, parse the command to flip
            // the corresponding boolean immediately.
            if kind == "R"
                && !cmd.is_empty()
                && os.get_popover_kind() == kind.as_str()
                && os.get_popover_target() == target.as_str()
            {
                let showing = cmd.starts_with("show ");
                let is_show_or_hide = showing || cmd.starts_with("hide ");
                if is_show_or_hide {
                    // Detect preset commands by the suffix BEYOND the known
                    // target. The target itself may contain "and polymer" etc.
                    // for bio chains, so `cmd.contains(...)` would false-match.
                    let selection = cmd.split_once(',').map(|(_, s)| s.trim()).unwrap_or("");
                    let suffix = selection
                        .strip_prefix(target.as_str())
                        .unwrap_or(selection)
                        .trim();

                    // `ends_with` (with leading space) avoids the substring
                    // collision where "inorganic" matches the "organic"
                    // branch. `inorganic` is checked before `organic` for
                    // belt-and-braces.
                    if suffix.contains("(sidechain") {
                        os.set_popover_preset_sidechain(showing);
                    } else if suffix.ends_with(" backbone") {
                        os.set_popover_preset_backbone(showing);
                    } else if suffix.ends_with(" inorganic") {
                        os.set_popover_preset_inorganic(showing);
                    } else if suffix.ends_with(" organic") {
                        os.set_popover_preset_organic(showing);
                    } else if suffix.ends_with(" solvent") {
                        os.set_popover_preset_solvent(showing);
                    } else if suffix.ends_with(" polymer") {
                        os.set_popover_preset_polymer(showing);
                    } else {
                        // Plain rep toggle: "show/hide <rep>, <target>"
                        let rep = cmd[5..].split(',').next().unwrap_or("").trim();
                        match rep {
                            "lines" => os.set_popover_rep_lines(showing),
                            "sticks" => os.set_popover_rep_sticks(showing),
                            "spheres" => os.set_popover_rep_spheres(showing),
                            "cartoon" => os.set_popover_rep_cartoon(showing),
                            "ribbon" => os.set_popover_rep_ribbon(showing),
                            "surface" => os.set_popover_rep_surface(showing),
                            "mesh" => os.set_popover_rep_mesh(showing),
                            "dots" => os.set_popover_rep_dots(showing),
                            _ => {}
                        }
                    }
                }
            }
        }
        Event::ActionZoom => {
            let can_focus = objects
                .selected_capabilities(&kernel.scene)
                .is_some_and(|capabilities| capabilities.focus);
            if can_focus {
                let Some(target) = objects.collect_selected_target() else {
                    return;
                };
                kernel.bus.execute_command(format!("zoom {}", target));
            }
        }
        Event::ActionOrient => {
            let can_orient = objects
                .selected_capabilities(&kernel.scene)
                .is_some_and(|capabilities| capabilities.orient);
            if can_orient {
                let Some(target) = objects.collect_selected_target() else {
                    return;
                };
                kernel.bus.execute_command(format!("orient {}", target));
            }
        }
        Event::ActionCenter => {
            let can_focus = objects
                .selected_capabilities(&kernel.scene)
                .is_some_and(|capabilities| capabilities.focus);
            if can_focus {
                let Some(target) = objects.collect_selected_target() else {
                    return;
                };
                kernel.bus.execute_command(format!("center {}", target));
            }
        }
        Event::ActionToggle => {
            let can_toggle = objects
                .selected_capabilities(&kernel.scene)
                .is_some_and(|capabilities| capabilities.visibility);
            if !can_toggle {
                return;
            }

            // Toggle level-based selections (groups/objects)
            match objects.scene_selection.selection_level {
                SelectionLevel::Groups => {
                    let groups = objects.scene_selection.selected_groups.clone();
                    for group_name in groups {
                        if let Some(group) = kernel.scene.get_group(&group_name) {
                            let children: Vec<String> =
                                group.children.iter().map(|o| o.name.clone()).collect();
                            for child in children {
                                kernel.bus.execute_command(format!("toggle {}", child));
                            }
                        }
                    }
                }
                SelectionLevel::Objects => {
                    let objects = objects.scene_selection.selected_objects.clone();
                    for name in objects {
                        kernel.bus.execute_command(format!("toggle {}", name));
                    }
                }
                _ => {} // Subchains can't be toggled
            }

            // Toggle selected named selections' visibility indicators
            let sel_names = objects.named_selection.selected_selections.clone();
            for name in sel_names {
                let sel_mgr = &mut kernel.session.selections;
                let visible = sel_mgr.is_visible(&name);
                sel_mgr.set_visible(&name, !visible);
            }
        }
        Event::ActionOverflow => {
            let Some(w) = weak.upgrade() else { return };
            let os = w.global::<ObjectsState>();

            // Toggle: close if already open
            if os.get_popover_kind() == "M" {
                os.set_popover_kind("".into());
                return;
            }

            let items = objects.compute_overflow_menu(&kernel.scene);
            let model: Rc<VecModel<OverflowMenuItem>> = Rc::new(VecModel::from(items));
            os.set_overflow_menu_items(ModelRc::from(model));
            os.set_popover_kind("M".into());
        }
        Event::OverflowAction(action) => {
            let Some(w) = weak.upgrade() else { return };
            let os = w.global::<ObjectsState>();
            let action = action.to_string();
            let Some(capabilities) = objects.selected_capabilities(&kernel.scene) else {
                return;
            };
            let Some(target) = objects.collect_selected_target() else {
                return;
            };
            let short = ObjectsBridge::truncate_name(&target, 28);

            match action.as_str() {
                "materialize"
                    if objects.scene_selection.selection_level == SelectionLevel::Objects
                        && objects.scene_selection.selected_objects.len() == 1 =>
                {
                    let name = objects.scene_selection.selected_objects[0].clone();
                    if kernel
                        .scene
                        .get(&name)
                        .is_some_and(|object| object.can_materialize)
                    {
                        kernel
                            .bus
                            .execute_command(format!("materialize {}", quote_command_arg(&name)));
                        os.set_popover_kind("".into());
                    }
                }
                "rename" if capabilities.rename => open_name_popup(&os, "rename", &target, &short),
                "copy" if capabilities.copy => open_name_popup(&os, "copy", &target, &short),
                "extract" if capabilities.extract => {
                    open_name_popup(&os, "extract", &target, &short)
                }
                "remove" if capabilities.remove_atoms => {
                    let cmd = format!("remove {}", target);
                    kernel.bus.execute_command(&cmd);
                    os.set_popover_kind("".into());
                }
                "delete" if capabilities.all_annotations && capabilities.delete => {
                    let targets: Vec<String> = objects
                        .scene_selection
                        .selected_groups
                        .iter()
                        .chain(&objects.scene_selection.selected_objects)
                        .cloned()
                        .collect();
                    for name in targets {
                        kernel.bus.execute_command(format!("delete {}", name));
                    }
                    os.set_popover_kind("".into());
                }
                "align" if capabilities.align => {
                    if let Some((mobile, fixed)) = objects.collect_align_targets() {
                        let cmd = format!("align {}, {}", mobile, fixed);
                        kernel.bus.execute_command(&cmd);
                    }
                    os.set_popover_kind("".into());
                }
                "color" if capabilities.color => {
                    if os.get_popover_kind() == "C" {
                        os.set_popover_kind("".into());
                        return;
                    }
                    os.set_popover_scope("all".into());
                    os.set_popover_solid_only(!capabilities.representations);
                    open_pill_popover(&os, "C", &target);
                    sync_color_assignments(&os, kernel);
                }
                "representation" if capabilities.representations => {
                    if os.get_popover_kind() == "R" {
                        os.set_popover_kind("".into());
                        return;
                    }
                    os.set_popover_solid_only(false);
                    set_active_reps_for_multi(&os, kernel, objects);
                    open_pill_popover(&os, "R", &target);
                }
                _ => {}
            }
        }
        Event::NameInputConfirm {
            action,
            target,
            new_name,
        } => {
            let Some(w) = weak.upgrade() else { return };
            let os = w.global::<ObjectsState>();

            if new_name.trim().is_empty() {
                return;
            }
            if action.is_empty() || target.is_empty() {
                os.set_popover_kind("".into());
                return;
            }

            let cmd = match action.as_str() {
                "rename" => format!(
                    "set_name {}, {}",
                    quote_command_arg(&target),
                    quote_command_arg(&new_name)
                ),
                "copy" => format!("copy {}, {}", new_name, target),
                "extract" => format!("extract {}, {}", new_name, target),
                _ => return,
            };
            kernel.bus.execute_command(&cmd);

            os.set_name_input_action("".into());
            if action == "rename" {
                objects.scene_selection.selected_groups.clear();
                objects.scene_selection.selected_objects.clear();
                objects.scene_selection.selected_subchains.clear();
                objects.named_selection.selected_selections.clear();
                objects.scene_selection.selection_level = SelectionLevel::None;
                objects.scene_selection.anchor = None;
                objects.named_selection.selection_anchor = None;
                objects.update_slint_selection(&os, &kernel.scene, &kernel.session.registry);
            }

            os.set_popover_kind("".into());
        }
        Event::ObjectRightClicked(name) => {
            kernel.bus.execute_command(format!("toggle {}", name));
        }
        Event::SelectionRightClicked(name) => {
            let sel_mgr = &mut kernel.session.selections;
            let visible = sel_mgr.is_visible(&name);
            sel_mgr.set_visible(&name, !visible);
        }
    }
}
