mod controls;
mod events;
mod text_highlights;

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, Model, ModelRc, VecModel};

use patinae_framework::plugin_ui::{PanelEvent, PanelPlacement};
use patinae_plugin_host::PanelFrame;

use crate::{
    AppWindow, LayoutState, PluginControl, PluginPanelTab, PluginState, PluginToolbarItem,
};

use controls::to_slint_control;
use events::panel_event_payload;

pub struct PluginBridge {
    toolbar_model: Rc<VecModel<PluginToolbarItem>>,
    right_tab_model: Rc<VecModel<PluginPanelTab>>,
    bottom_tab_model: Rc<VecModel<PluginPanelTab>>,
    right_control_model: Rc<VecModel<PluginControl>>,
    bottom_control_model: Rc<VecModel<PluginControl>>,
    toolbar_signature: RefCell<Vec<String>>,
    right_tab_signature: RefCell<Vec<String>>,
    bottom_tab_signature: RefCell<Vec<String>>,
}

impl PluginBridge {
    pub fn new() -> Self {
        Self {
            toolbar_model: Rc::new(VecModel::default()),
            right_tab_model: Rc::new(VecModel::default()),
            bottom_tab_model: Rc::new(VecModel::default()),
            right_control_model: Rc::new(VecModel::default()),
            bottom_control_model: Rc::new(VecModel::default()),
            toolbar_signature: RefCell::new(Vec::new()),
            right_tab_signature: RefCell::new(Vec::new()),
            bottom_tab_signature: RefCell::new(Vec::new()),
        }
    }

    pub fn attach(&self, window: &AppWindow) {
        let ps = window.global::<PluginState>();
        ps.set_toolbar_items(ModelRc::from(self.toolbar_model.clone()));
        ps.set_right_tabs(ModelRc::from(self.right_tab_model.clone()));
        ps.set_bottom_tabs(ModelRc::from(self.bottom_tab_model.clone()));
        ps.set_active_right_controls(ModelRc::from(self.right_control_model.clone()));
        ps.set_active_bottom_controls(ModelRc::from(self.bottom_control_model.clone()));
    }

    pub fn sync(&self, frames: Vec<PanelFrame>, window: &AppWindow) {
        let ps = window.global::<PluginState>();
        let ls = window.global::<LayoutState>();

        let toolbar: Vec<PluginToolbarItem> = frames
            .iter()
            .map(|frame| {
                let desc = &frame.status.descriptor;
                PluginToolbarItem {
                    id: desc.id.clone().into(),
                    title: desc.title.clone().into(),
                    icon: desc.icon.clone().into(),
                    placement: placement_name(desc.placement).into(),
                    visible: frame.status.visible,
                    active: frame.status.visible,
                }
            })
            .collect();

        let right_tabs: Vec<PluginPanelTab> = frames
            .iter()
            .filter(|frame| {
                frame.status.visible && frame.status.descriptor.placement == PanelPlacement::Right
            })
            .map(to_tab)
            .collect();
        let bottom_tabs: Vec<PluginPanelTab> = frames
            .iter()
            .filter(|frame| {
                frame.status.visible && frame.status.descriptor.placement == PanelPlacement::Bottom
            })
            .map(to_tab)
            .collect();

        let active_right = frames.iter().find(|frame| {
            frame.status.visible
                && frame.status.active
                && frame.status.descriptor.placement == PanelPlacement::Right
        });
        let active_bottom = frames.iter().find(|frame| {
            frame.status.visible
                && frame.status.active
                && frame.status.descriptor.placement == PanelPlacement::Bottom
        });

        replace_model_if_changed(
            &self.toolbar_model,
            toolbar,
            &self.toolbar_signature,
            toolbar_signature,
        );
        replace_model_if_changed(
            &self.right_tab_model,
            right_tabs,
            &self.right_tab_signature,
            tabs_signature,
        );
        replace_model_if_changed(
            &self.bottom_tab_model,
            bottom_tabs,
            &self.bottom_tab_signature,
            tabs_signature,
        );

        if let Some(frame) = active_right {
            ps.set_active_right_panel_id(frame.status.descriptor.id.clone().into());
            ps.set_active_right_title(frame.status.descriptor.title.clone().into());
            replace_control_model(
                &self.right_control_model,
                frame
                    .snapshot
                    .controls
                    .iter()
                    .map(to_slint_control)
                    .collect(),
            );
        } else {
            ps.set_active_right_panel_id("".into());
            ps.set_active_right_title("".into());
            replace_model(&self.right_control_model, Vec::new());
        }

        if let Some(frame) = active_bottom {
            ps.set_active_bottom_panel_id(frame.status.descriptor.id.clone().into());
            ps.set_active_bottom_title(frame.status.descriptor.title.clone().into());
            replace_control_model(
                &self.bottom_control_model,
                frame
                    .snapshot
                    .controls
                    .iter()
                    .map(to_slint_control)
                    .collect(),
            );
        } else {
            ps.set_active_bottom_panel_id("".into());
            ps.set_active_bottom_title("".into());
            replace_model(&self.bottom_control_model, Vec::new());
        }

        let right_visible = active_right.is_some();
        let bottom_visible = active_bottom.is_some();
        ps.set_right_visible(right_visible);
        ps.set_bottom_visible(bottom_visible);
        ls.set_plugin_right_visible(right_visible);
        ls.set_plugin_bottom_visible(bottom_visible);
    }
}

impl Default for PluginBridge {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) mod gui_events;
pub(crate) use gui_events::setup_callbacks;

fn placement_name(placement: PanelPlacement) -> &'static str {
    match placement {
        PanelPlacement::Right => "right",
        PanelPlacement::Bottom => "bottom",
    }
}

fn to_tab(frame: &PanelFrame) -> PluginPanelTab {
    PluginPanelTab {
        id: frame.status.descriptor.id.clone().into(),
        title: frame.status.descriptor.title.clone().into(),
        active: frame.status.active,
    }
}

fn replace_model<T: Clone + 'static>(model: &VecModel<T>, rows: Vec<T>) {
    while model.row_count() > rows.len() {
        model.remove(model.row_count() - 1);
    }
    for (idx, row) in rows.into_iter().enumerate() {
        if idx < model.row_count() {
            model.set_row_data(idx, row);
        } else {
            model.push(row);
        }
    }
}

fn replace_control_model(model: &VecModel<PluginControl>, rows: Vec<PluginControl>) {
    while model.row_count() > rows.len() {
        model.remove(model.row_count() - 1);
    }
    for (idx, row) in rows.into_iter().enumerate() {
        if idx < model.row_count() {
            let row = match model.row_data(idx) {
                Some(existing) => reconcile_control_row(&existing, row),
                None => row,
            };
            model.set_row_data(idx, row);
        } else {
            model.push(row);
        }
    }
}

fn reconcile_control_row(existing: &PluginControl, mut next: PluginControl) -> PluginControl {
    if is_stable_container(existing, &next) {
        let rows = collect_model_rows(&next.children);
        let rows = reconcile_node_rows(&existing.children, rows);
        if replace_model_rc(&existing.children, rows) {
            next.children = existing.children.clone();
        }
    }
    next
}

fn is_stable_container(existing: &PluginControl, next: &PluginControl) -> bool {
    existing.id == next.id
        && existing.kind == next.kind
        && matches!(next.kind.as_str(), "row" | "column" | "group")
}

fn reconcile_node_rows(
    existing_model: &ModelRc<crate::PluginControlNode>,
    rows: Vec<crate::PluginControlNode>,
) -> Vec<crate::PluginControlNode> {
    rows.into_iter()
        .enumerate()
        .map(|(idx, row)| match existing_model.row_data(idx) {
            Some(existing) => reconcile_control_node(&existing, row),
            None => row,
        })
        .collect()
}

fn reconcile_control_node(
    existing: &crate::PluginControlNode,
    mut next: crate::PluginControlNode,
) -> crate::PluginControlNode {
    if is_stable_node_container(existing, &next) {
        let rows = collect_model_rows(&next.leaf_children);
        if replace_model_rc(&existing.leaf_children, rows) {
            next.leaf_children = existing.leaf_children.clone();
        }
    }
    next
}

fn is_stable_node_container(
    existing: &crate::PluginControlNode,
    next: &crate::PluginControlNode,
) -> bool {
    existing.id == next.id
        && existing.kind == next.kind
        && matches!(next.kind.as_str(), "row" | "column")
}

fn collect_model_rows<T: Clone + 'static>(model: &ModelRc<T>) -> Vec<T> {
    (0..model.row_count())
        .filter_map(|idx| model.row_data(idx))
        .collect()
}

fn replace_model_rc<T: Clone + 'static>(model: &ModelRc<T>, rows: Vec<T>) -> bool {
    if let Some(vec_model) = model.as_any().downcast_ref::<VecModel<T>>() {
        replace_model(vec_model, rows);
        return true;
    }

    if model.row_count() != rows.len() {
        return false;
    }

    for (idx, row) in rows.into_iter().enumerate() {
        model.set_row_data(idx, row);
    }
    true
}

fn replace_model_if_changed<T: Clone + 'static>(
    model: &Rc<VecModel<T>>,
    rows: Vec<T>,
    signature_store: &RefCell<Vec<String>>,
    signature: impl Fn(&[T]) -> Vec<String>,
) {
    let next_signature = signature(&rows);
    if *signature_store.borrow() == next_signature {
        return;
    }
    *signature_store.borrow_mut() = next_signature;
    replace_model(model, rows);
}

fn toolbar_signature(rows: &[PluginToolbarItem]) -> Vec<String> {
    rows.iter()
        .map(|row| {
            format!(
                "{}:{}:{}:{}",
                row.id, row.placement, row.visible, row.active
            )
        })
        .collect()
}

fn tabs_signature(rows: &[PluginPanelTab]) -> Vec<String> {
    rows.iter()
        .map(|row| format!("{}:{}:{}", row.id, row.title, row.active))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use patinae_framework::plugin_ui::{
        PanelColumn, PanelControl as CoreControl, PanelControlNode as CoreControlNode, PanelRow,
        PanelTextArea, PanelTextHighlight, PanelTextStyle,
    };
    use slint::Model;

    fn text_area(value: &str) -> CoreControl {
        CoreControl::TextArea(
            PanelTextArea::new("script", "", value, "", 5, false).with_highlights(vec![
                PanelTextHighlight::new(0, value.len().min(5), PanelTextStyle::Function),
            ]),
        )
    }

    fn row_control(id: &str, value: &str) -> PluginControl {
        to_slint_control(&CoreControl::Row(PanelRow::new(
            id,
            vec![CoreControlNode::new(text_area(value)).grow(1.0)],
        )))
    }

    fn column_control(id: &str, value: &str) -> PluginControl {
        to_slint_control(&CoreControl::Column(PanelColumn::new(
            id,
            vec![CoreControlNode::new(text_area(value)).grow(1.0)],
        )))
    }

    #[test]
    fn stable_row_reuses_child_model_and_updates_text_area() {
        let model = Rc::new(VecModel::default());
        replace_control_model(&model, vec![row_control("script_output", "print")]);
        let original_children = model.row_data(0).unwrap().children;

        replace_control_model(&model, vec![row_control("script_output", "print('hello')")]);
        let updated = model.row_data(0).unwrap();
        let child = updated.children.row_data(0).unwrap();
        let highlight_line = child.highlight_lines.row_data(0).unwrap();
        let highlight_run = highlight_line.runs.row_data(0).unwrap();

        assert_eq!(updated.children, original_children);
        assert_eq!(child.value_text.as_str(), "print('hello')");
        assert_eq!(highlight_run.text.as_str(), "print");
        assert_eq!(highlight_run.style.as_str(), "function");
    }

    #[test]
    fn stable_column_reuses_child_model() {
        let model = Rc::new(VecModel::default());
        replace_control_model(&model, vec![column_control("stack", "alpha")]);
        let original_children = model.row_data(0).unwrap().children;

        replace_control_model(&model, vec![column_control("stack", "beta")]);
        let updated = model.row_data(0).unwrap();

        assert_eq!(updated.children, original_children);
        assert_eq!(
            updated.children.row_data(0).unwrap().value_text.as_str(),
            "beta"
        );
    }

    #[test]
    fn changed_container_identity_does_not_reuse_children() {
        let model = Rc::new(VecModel::default());
        replace_control_model(&model, vec![row_control("container", "alpha")]);
        let original_children = model.row_data(0).unwrap().children;

        replace_control_model(&model, vec![column_control("container", "beta")]);
        let changed_kind_children = model.row_data(0).unwrap().children;
        assert_ne!(changed_kind_children, original_children);

        replace_control_model(&model, vec![row_control("other", "gamma")]);
        let changed_id_children = model.row_data(0).unwrap().children;
        assert_ne!(changed_id_children, changed_kind_children);
    }
}
