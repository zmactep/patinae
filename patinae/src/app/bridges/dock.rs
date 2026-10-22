//! Derived, virtualized rows for the built-in dock; domain state stays in ObjectsBridge.

use slint::Model;

use crate::{DockRow, ObjectItem, RecentAtomItem, SelectionRow, TopLevelRow};

pub(super) fn rows(
    objects: &impl Model<Data = TopLevelRow>,
    selections: &impl Model<Data = SelectionRow>,
    recent: &impl Model<Data = RecentAtomItem>,
    visible: [bool; 3],
) -> Vec<DockRow> {
    let mut rows = Vec::new();
    if visible[0] {
        let count = objects
            .iter()
            .map(|r| if r.is_group { r.group_child_count } else { 1 })
            .sum();
        header(&mut rows, "objects", "OBJECTS", count);
        for group in objects.iter() {
            if group.is_group {
                rows.push(DockRow {
                    kind: "group".into(),
                    key: format!("group:{}", group.group_name).into(),
                    group: group.clone(),
                    ..Default::default()
                });
                if group.group_open {
                    for object in group.group_objects.iter() {
                        append_object(&mut rows, object, 1);
                    }
                }
            } else {
                append_object(&mut rows, group.object, 0);
            }
        }
        if count == 0 {
            empty(&mut rows, "objects", "no objects yet");
        }
    }
    if visible[1] {
        header(
            &mut rows,
            "selections",
            "SELECTIONS",
            selections.row_count() as i32,
        );
        for selection in selections.iter() {
            rows.push(DockRow {
                kind: "selection".into(),
                key: format!("selection:{}", selection.name).into(),
                selection,
                ..Default::default()
            });
        }
        if selections.row_count() == 0 {
            empty(&mut rows, "selections", "no selections yet");
        }
    }
    if visible[2] {
        header(
            &mut rows,
            "recent",
            "RECENT ATOMS",
            recent.row_count() as i32,
        );
        for recent in recent.iter() {
            rows.push(DockRow {
                kind: "recent".into(),
                key: recent.key.clone(),
                recent,
                ..Default::default()
            });
        }
        if recent.row_count() == 0 {
            empty(&mut rows, "recent", "no recent atoms yet");
        }
    }
    rows
}

fn header(rows: &mut Vec<DockRow>, key: &str, title: &str, count: i32) {
    rows.push(DockRow {
        kind: "header".into(),
        key: key.into(),
        title: title.into(),
        count,
        ..Default::default()
    });
}

fn empty(rows: &mut Vec<DockRow>, key: &str, title: &str) {
    rows.push(DockRow {
        kind: "empty".into(),
        key: format!("empty:{key}").into(),
        title: title.into(),
        ..Default::default()
    });
}

fn append_object(rows: &mut Vec<DockRow>, object: ObjectItem, depth: i32) {
    rows.push(DockRow {
        kind: "object".into(),
        key: format!("object:{}", object.name).into(),
        depth,
        object: object.clone(),
        ..Default::default()
    });
    if object.expanded && object.has_representations {
        for subchain in object.subchains.iter() {
            rows.push(DockRow {
                kind: "subchain".into(),
                key: format!("subchain:{}:{}", object.name, subchain.entry_index).into(),
                depth: depth + 1,
                object: object.clone(),
                subchain,
                ..Default::default()
            });
        }
        if object.overflow_count > 0 {
            empty(
                rows,
                &format!("overflow:{}", object.name),
                &format!("{} more subchains not shown", object.overflow_count),
            );
        }
    }
}

/// Retain the top visible row and its intra-row offset across model changes.
pub(super) fn restored_scroll(
    old: &[DockRow],
    new: &[DockRow],
    scroll: f32,
    header: f32,
    row: f32,
) -> f32 {
    let height = |r: &DockRow| if r.kind == "header" { header } else { row };
    let mut top = 0.0;
    for anchor in old {
        let h = height(anchor);
        if top + h > -scroll {
            let mut new_top = 0.0;
            for candidate in new {
                if candidate.key == anchor.key {
                    return -(new_top + (-scroll - top).max(0.0));
                }
                new_top += height(candidate);
            }
            break;
        }
        top += h;
    }
    scroll.min(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::{ModelRc, VecModel};
    use std::rc::Rc;

    #[test]
    fn empty_and_hidden_sections_have_only_their_own_rows() {
        let objects = VecModel::<TopLevelRow>::default();
        let selections = VecModel::<SelectionRow>::default();
        let recent = VecModel::<RecentAtomItem>::default();
        let result = rows(&objects, &selections, &recent, [true, false, true]);
        assert_eq!(
            result.iter().map(|r| r.key.as_str()).collect::<Vec<_>>(),
            ["objects", "empty:objects", "recent", "empty:recent"]
        );
        assert!(rows(&objects, &selections, &recent, [false; 3]).is_empty());
    }

    #[test]
    fn expanded_tree_is_flat_and_keeps_stable_keys() {
        let object = ObjectItem {
            name: "mol".into(),
            expanded: true,
            has_representations: true,
            subchains: ModelRc::from(Rc::new(VecModel::from(vec![crate::SubchainItem {
                entry_index: 3,
                ..Default::default()
            }]))),
            ..Default::default()
        };
        let objects = VecModel::from(vec![TopLevelRow {
            object,
            ..Default::default()
        }]);
        let result = rows(
            &objects,
            &VecModel::default(),
            &VecModel::default(),
            [true, false, false],
        );
        assert_eq!(
            result.iter().map(|r| r.key.as_str()).collect::<Vec<_>>(),
            ["objects", "object:mol", "subchain:mol:3"]
        );
        assert_eq!(result[2].depth, 1);
        let mut changed = result.clone();
        changed.insert(
            1,
            DockRow {
                key: "inserted".into(),
                ..Default::default()
            },
        );
        assert_eq!(restored_scroll(&result, &changed, -31.0, 28.0, 30.0), -61.0);
    }
}
