use std::rc::Rc;

use slint::{Image, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel};

use patinae_framework::plugin_ui::{
    PanelButton as CoreButton, PanelControl as CoreControl, PanelControlNode as CoreControlNode,
    PanelOption as CoreOption,
};

use crate::{
    PluginButtonItem, PluginControl, PluginControlLeaf, PluginControlNode, PluginOption,
    PluginSubLeaf,
};

use super::text_highlights::{empty_highlight_lines_model, highlight_lines_model};

pub(crate) fn to_slint_control(control: &CoreControl) -> PluginControl {
    let mut row = empty_control();
    row.height = control_height(control);
    match control {
        CoreControl::Text { id, text } => {
            row.id = id.clone().into();
            row.kind = "text".into();
            row.text = text.clone().into();
        }
        CoreControl::Heading { id, text } => {
            row.id = id.clone().into();
            row.kind = "heading".into();
            row.text = text.clone().into();
        }
        CoreControl::TitleDesc { id, title, desc } => {
            row.id = id.clone().into();
            row.kind = "titledesc".into();
            row.label = title.clone().into();
            row.text = desc.clone().into();
        }
        CoreControl::Section { id, title, open } => {
            row.id = id.clone().into();
            row.kind = "section".into();
            row.label = title.clone().into();
            row.open = *open;
        }
        CoreControl::Button { id, label, primary } => {
            row.id = id.clone().into();
            row.kind = "button".into();
            row.label = label.clone().into();
            row.primary = *primary;
        }
        CoreControl::ButtonRow { id, buttons } => {
            row.id = id.clone().into();
            row.kind = "button-row".into();
            row.buttons = button_items_model(buttons);
        }
        CoreControl::Row(layout) => {
            row.id = layout.id.clone().into();
            row.kind = "row".into();
            row.gap = layout.gap;
            row.children = control_nodes_model(&layout.children);
        }
        CoreControl::Column(layout) => {
            row.id = layout.id.clone().into();
            row.kind = "column".into();
            row.gap = layout.gap;
            row.children = control_nodes_model(&layout.children);
        }
        CoreControl::Group(group) => {
            row.id = group.id.clone().into();
            row.kind = "group".into();
            row.label = group.title.clone().into();
            row.open = group.open;
            row.gap = group.gap;
            row.children = control_nodes_model(&group.children);
        }
        CoreControl::Toggle { id, label, value } => {
            row.id = id.clone().into();
            row.kind = "toggle".into();
            row.label = label.clone().into();
            row.bool_value = *value;
        }
        CoreControl::Slider {
            id,
            label,
            value,
            min,
            max,
            step,
        } => {
            row.id = id.clone().into();
            row.kind = "slider".into();
            row.label = label.clone().into();
            row.num_value = *value;
            row.min = *min;
            row.max = *max;
            row.step = *step;
        }
        CoreControl::Number {
            id,
            label,
            value,
            min,
            max,
            step,
        } => {
            row.id = id.clone().into();
            row.kind = "number".into();
            row.label = label.clone().into();
            row.value_text = format_number(*value).into();
            row.num_value = *value;
            row.min = *min;
            row.max = *max;
            row.step = *step;
        }
        CoreControl::Select {
            id,
            label,
            value,
            options,
        } => {
            row.id = id.clone().into();
            row.kind = "select".into();
            row.label = label.clone().into();
            row.value_text = value.clone().into();
            row.options = options_model(options);
        }
        CoreControl::TextInput {
            id,
            label,
            value,
            placeholder,
        } => {
            row.id = id.clone().into();
            row.kind = "text-input".into();
            row.label = label.clone().into();
            row.value_text = value.clone().into();
            row.text = placeholder.clone().into();
        }
        CoreControl::TextArea(area) => {
            row.id = area.id.clone().into();
            row.kind = "text-area".into();
            row.label = area.label.clone().into();
            row.value_text = area.value.clone().into();
            row.text = area.placeholder.clone().into();
            row.rows = area.rows as f32;
            row.read_only = area.read_only;
            row.highlight_lines = highlight_lines_model(&area.value, &area.highlights);
        }
        CoreControl::Image {
            id,
            width,
            height,
            rgba,
        } => {
            row.id = id.clone().into();
            row.kind = "image".into();
            row.image_width = *width as f32;
            row.image_height = *height as f32;
            row.image_value = rgba_to_image(rgba, *width, *height);
        }
        CoreControl::Spacer { id, height } => {
            row.id = id.clone().into();
            row.kind = "spacer".into();
            row.num_value = *height;
        }
    }
    row
}

fn empty_control() -> PluginControl {
    PluginControl {
        id: "".into(),
        kind: "".into(),
        label: "".into(),
        text: "".into(),
        value_text: "".into(),
        bool_value: false,
        num_value: 0.0,
        min: 0.0,
        max: 1.0,
        step: 1.0,
        primary: false,
        open: false,
        read_only: false,
        rows: 1.0,
        grow: 0.0,
        gap: 0.0,
        height: 4.0,
        highlight_lines: empty_highlight_lines_model(),
        options: ModelRc::from(Rc::new(VecModel::default())),
        buttons: ModelRc::from(Rc::new(VecModel::default())),
        children: ModelRc::from(Rc::new(VecModel::default())),
        image_value: Image::default(),
        image_width: 0.0,
        image_height: 0.0,
    }
}

fn control_nodes_model(nodes: &[CoreControlNode]) -> ModelRc<PluginControlNode> {
    let rows: Vec<PluginControlNode> = nodes
        .iter()
        .map(|node| {
            let control = to_slint_control(&node.control);
            PluginControlNode {
                id: control.id,
                kind: control.kind,
                label: control.label,
                text: control.text,
                value_text: control.value_text,
                bool_value: control.bool_value,
                num_value: control.num_value,
                min: control.min,
                max: control.max,
                step: control.step,
                primary: control.primary,
                open: control.open,
                read_only: control.read_only,
                rows: control.rows,
                grow: node.grow,
                gap: control.gap,
                height: control.height,
                highlight_lines: control.highlight_lines,
                options: control.options,
                buttons: control.buttons,
                leaf_children: leaf_children_model(&node.control),
                image_value: control.image_value,
                image_width: control.image_width,
                image_height: control.image_height,
            }
        })
        .collect();
    ModelRc::from(Rc::new(VecModel::from(rows)))
}

fn leaf_children_model(control: &CoreControl) -> ModelRc<PluginControlLeaf> {
    match control {
        CoreControl::Row(layout) => control_leaf_model(&layout.children),
        CoreControl::Column(layout) => control_leaf_model(&layout.children),
        _ => ModelRc::from(Rc::new(VecModel::default())),
    }
}

fn control_leaf_model(nodes: &[CoreControlNode]) -> ModelRc<PluginControlLeaf> {
    let rows: Vec<PluginControlLeaf> = nodes
        .iter()
        .map(|node| {
            let control = to_slint_control(&node.control);
            PluginControlLeaf {
                id: control.id,
                kind: control.kind,
                label: control.label,
                text: control.text,
                value_text: control.value_text,
                bool_value: control.bool_value,
                num_value: control.num_value,
                min: control.min,
                max: control.max,
                step: control.step,
                primary: control.primary,
                open: control.open,
                read_only: control.read_only,
                rows: control.rows,
                grow: node.grow,
                gap: control.gap,
                height: control.height,
                highlight_lines: control.highlight_lines,
                options: control.options,
                buttons: control.buttons,
                sub_children: sub_leaf_model(&node.control),
                image_value: control.image_value,
                image_width: control.image_width,
                image_height: control.image_height,
            }
        })
        .collect();
    ModelRc::from(Rc::new(VecModel::from(rows)))
}

fn sub_leaf_model(control: &CoreControl) -> ModelRc<PluginSubLeaf> {
    match control {
        CoreControl::Row(layout) => sub_leaf_rows(&layout.children),
        CoreControl::Column(layout) => sub_leaf_rows(&layout.children),
        _ => ModelRc::from(Rc::new(VecModel::default())),
    }
}

fn sub_leaf_rows(nodes: &[CoreControlNode]) -> ModelRc<PluginSubLeaf> {
    let rows: Vec<PluginSubLeaf> = nodes
        .iter()
        .map(|node| {
            let control = to_slint_control(&node.control);
            PluginSubLeaf {
                id: control.id,
                kind: control.kind,
                label: control.label,
                text: control.text,
                value_text: control.value_text,
                bool_value: control.bool_value,
                num_value: control.num_value,
                min: control.min,
                max: control.max,
                step: control.step,
                primary: control.primary,
                read_only: control.read_only,
                rows: control.rows,
                grow: node.grow,
                gap: control.gap,
                height: control.height,
                highlight_lines: control.highlight_lines,
                options: control.options,
                buttons: control.buttons,
            }
        })
        .collect();
    ModelRc::from(Rc::new(VecModel::from(rows)))
}

fn options_model(options: &[CoreOption]) -> ModelRc<PluginOption> {
    let rows: Vec<PluginOption> = options
        .iter()
        .map(|option| PluginOption {
            label: option.label.clone().into(),
            value: option.value.clone().into(),
        })
        .collect();
    ModelRc::from(Rc::new(VecModel::from(rows)))
}

fn button_items_model(buttons: &[CoreButton]) -> ModelRc<PluginButtonItem> {
    let rows: Vec<PluginButtonItem> = buttons
        .iter()
        .map(|button| PluginButtonItem {
            id: button.id.clone().into(),
            label: button.label.clone().into(),
            icon: button.icon.clone().into(),
            primary: button.primary,
            enabled: button.enabled,
        })
        .collect();
    ModelRc::from(Rc::new(VecModel::from(rows)))
}

fn control_height(control: &CoreControl) -> f32 {
    match control {
        CoreControl::Heading { .. } => 24.0,
        CoreControl::Text { .. } => 20.0,
        CoreControl::TitleDesc { .. } => 46.0,
        CoreControl::Section { .. } => 36.0,
        // Tool-rail rows are compact; the prominent "run" action button is taller.
        CoreControl::Button { id, .. } if id == "run" => 30.0,
        CoreControl::Button { .. } => 26.0,
        CoreControl::ButtonRow { .. } => 24.0,
        CoreControl::Toggle { .. } => 28.0,
        CoreControl::Slider { .. } => 38.0,
        CoreControl::Number { .. } | CoreControl::TextInput { .. } => 46.0,
        CoreControl::Select { .. } => 52.0,
        CoreControl::TextArea(area) => (area.rows as f32 * 18.0 + 22.0).max(72.0),
        CoreControl::Image { height, .. } => *height as f32,
        CoreControl::Spacer { height, .. } => (*height).max(4.0),
        CoreControl::Row(layout) => layout
            .children
            .iter()
            .map(|node| control_height(&node.control))
            .fold(4.0, f32::max),
        CoreControl::Column(layout) => {
            let child_count = layout.children.len();
            let children_height = layout
                .children
                .iter()
                .map(|node| control_height(&node.control))
                .sum::<f32>();
            let gaps = child_count.saturating_sub(1) as f32 * layout.gap;
            // +24: the renderer wraps a column-in-row in a padded card.
            (children_height + gaps + 24.0).max(4.0)
        }
        CoreControl::Group(group) => {
            let child_count = if group.open { group.children.len() } else { 0 };
            let children_height = if group.open {
                group
                    .children
                    .iter()
                    .map(|node| control_height(&node.control))
                    .sum::<f32>()
            } else {
                0.0
            };
            let child_gaps = if group.open {
                child_count.saturating_sub(1) as f32 * group.gap
            } else {
                0.0
            };
            let has_title = !group.title.is_empty();
            let title_height = if has_title { 24.0 } else { 0.0 };
            let title_gap = if has_title && child_count > 0 {
                8.0
            } else {
                0.0
            };
            let vertical_padding = 16.0;
            (title_height + title_gap + children_height + child_gaps + vertical_padding).max(36.0)
        }
    }
}

fn rgba_to_image(rgba: &[u8], w: u32, h: u32) -> Image {
    if w == 0 || h == 0 || rgba.len() != (w as usize * h as usize * 4) {
        return Image::default();
    }
    let buf = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(rgba, w, h);
    Image::from_rgba8(buf)
}

fn format_number(v: f32) -> String {
    format!("{v:.4}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::Model;

    #[test]
    fn row_layout_converts_children_and_grow() {
        let control = CoreControl::Row(patinae_framework::plugin_ui::PanelRow::new(
            "pair",
            vec![
                CoreControlNode::new(CoreControl::TextArea(
                    patinae_framework::plugin_ui::PanelTextArea::new("left", "", "a", "", 5, false),
                ))
                .grow(1.0),
                CoreControlNode::new(CoreControl::TextArea(
                    patinae_framework::plugin_ui::PanelTextArea::new("right", "", "b", "", 5, true),
                ))
                .grow(1.0),
            ],
        ));

        let row = to_slint_control(&control);

        assert_eq!(row.kind.as_str(), "row");
        assert_eq!(row.children.row_count(), 2);
        assert_eq!(row.children.row_data(0).unwrap().grow, 1.0);
        assert_eq!(row.children.row_data(1).unwrap().kind.as_str(), "text-area");
        assert_eq!(row.height, 112.0);
    }

    #[test]
    fn column_layout_height_includes_child_heights_and_gaps() {
        let control = CoreControl::Column(
            patinae_framework::plugin_ui::PanelColumn::new(
                "stack",
                vec![
                    CoreControlNode::new(CoreControl::Button {
                        id: "a".into(),
                        label: "A".into(),
                        primary: false,
                    }),
                    CoreControlNode::new(CoreControl::Button {
                        id: "b".into(),
                        label: "B".into(),
                        primary: false,
                    }),
                ],
            )
            .gap(3.0),
        );

        let column = to_slint_control(&control);

        assert_eq!(column.kind.as_str(), "column");
        assert_eq!(column.children.row_count(), 2);
        // two 26px rail buttons + 3px gap + 24px card padding
        assert_eq!(column.height, 79.0);
    }

    #[test]
    fn row_layout_preserves_practical_nested_control_kinds() {
        let control = CoreControl::Row(patinae_framework::plugin_ui::PanelRow::new(
            "mixed",
            vec![
                CoreControlNode::new(CoreControl::Number {
                    id: "width".into(),
                    label: "Width".into(),
                    value: 1920.0,
                    min: 64.0,
                    max: 7680.0,
                    step: 1.0,
                })
                .grow(1.0),
                CoreControlNode::new(CoreControl::Select {
                    id: "mode".into(),
                    label: "Mode".into(),
                    value: "0".into(),
                    options: vec![CoreOption {
                        label: "Normal".into(),
                        value: "0".into(),
                    }],
                })
                .grow(1.0),
                CoreControlNode::new(CoreControl::Slider {
                    id: "fog".into(),
                    label: "Fog".into(),
                    value: 0.0,
                    min: -1.0,
                    max: 1.0,
                    step: 0.01,
                })
                .grow(1.0),
                CoreControlNode::new(CoreControl::Toggle {
                    id: "shadow".into(),
                    label: "Shadows".into(),
                    value: true,
                })
                .grow(1.0),
            ],
        ));

        let row = to_slint_control(&control);
        let kinds: Vec<_> = (0..row.children.row_count())
            .map(|idx| row.children.row_data(idx).unwrap().kind.to_string())
            .collect();

        assert_eq!(kinds, ["number", "select", "slider", "toggle"]);
    }

    #[test]
    fn group_layout_converts_title_children_and_height() {
        let control = CoreControl::Group(patinae_framework::plugin_ui::PanelGroup::new(
            "output_group",
            "Output",
            vec![
                CoreControlNode::new(CoreControl::Select {
                    id: "preset".into(),
                    label: "Resolution".into(),
                    value: "1080".into(),
                    options: vec![CoreOption {
                        label: "1080p".into(),
                        value: "1080".into(),
                    }],
                }),
                CoreControlNode::new(CoreControl::Slider {
                    id: "antialias".into(),
                    label: "Antialias".into(),
                    value: 2.0,
                    min: 1.0,
                    max: 4.0,
                    step: 1.0,
                }),
            ],
        ));

        let group = to_slint_control(&control);

        assert_eq!(group.kind.as_str(), "group");
        assert_eq!(group.label.as_str(), "Output");
        assert_eq!(group.children.row_count(), 2);
        assert_eq!(group.height, 146.0);
    }

    #[test]
    fn closed_group_height_excludes_children() {
        let control = CoreControl::Group(
            patinae_framework::plugin_ui::PanelGroup::new(
                "output_group",
                "Output",
                vec![CoreControlNode::new(CoreControl::Slider {
                    id: "antialias".into(),
                    label: "Antialias".into(),
                    value: 2.0,
                    min: 1.0,
                    max: 4.0,
                    step: 1.0,
                })],
            )
            .open(false),
        );

        let group = to_slint_control(&control);

        assert_eq!(group.kind.as_str(), "group");
        assert!(!group.open);
        assert_eq!(group.height, 40.0);
    }

    #[test]
    fn group_layout_preserves_nested_row_children() {
        let control = CoreControl::Group(patinae_framework::plugin_ui::PanelGroup::new(
            "output_group",
            "Output",
            vec![CoreControlNode::new(CoreControl::Row(
                patinae_framework::plugin_ui::PanelRow::new(
                    "dimensions",
                    vec![
                        CoreControlNode::new(CoreControl::Number {
                            id: "width".into(),
                            label: "Width".into(),
                            value: 1920.0,
                            min: 64.0,
                            max: 7680.0,
                            step: 1.0,
                        }),
                        CoreControlNode::new(CoreControl::Number {
                            id: "height".into(),
                            label: "Height".into(),
                            value: 1080.0,
                            min: 64.0,
                            max: 4320.0,
                            step: 1.0,
                        }),
                    ],
                ),
            ))],
        ));

        let group = to_slint_control(&control);
        let dimensions = group.children.row_data(0).unwrap();

        assert_eq!(dimensions.kind.as_str(), "row");
        assert_eq!(dimensions.leaf_children.row_count(), 2);
        assert_eq!(
            dimensions.leaf_children.row_data(1).unwrap().label.as_str(),
            "Height"
        );
    }
}
