//! Compact draw rows reuse source geometry, colors, and topology.

use super::{ObjectEntry, SceneStore};
use crate::picking::ObjectId;
use crate::render_input::IDENTITY_TRANSFORM;

impl SceneStore {
    /// Dynamic uniform offsets for the object's displayed copies.
    pub fn draw_offsets(&self, object: ObjectId) -> &[u32] {
        self.draw_offsets.get(&object.0).map_or(&[], Vec::as_slice)
    }

    /// Transformed objects must draw uncropped source geometry.
    pub fn needs_raw_draw(&self, object: ObjectId) -> bool {
        self.geometry_aliases.contains_key(&object.0)
            || self.instance_tables.contains_key(&object.0)
            || self
                .slot(object)
                .is_some_and(|slot| self.object_entry(*slot).model_matrix != IDENTITY_TRANSFORM)
    }

    pub(super) fn rebuild_instance_entries(&mut self) {
        self.draw_offsets.clear();
        self.geometry_aliases.clear();
        let mut rows = Vec::new();
        // Source visibility occupies the prefix. Subset masks occur once per
        // group, irrespective of the number of copies using that group.
        let mut mask_tail = Vec::<u32>::new();
        let source_words = (self.next_atom_offset as usize).div_ceil(32);
        for (&id, &slot) in &self.slots {
            let source = self.object_entry(slot);
            let Some(table) = self.instance_tables.get(&id) else {
                self.draw_offsets.insert(id, vec![slot.dynamic_offset()]);
                continue;
            };
            let canonical = crate::render_input::canonical_instance_groups(table);
            let mut groups = Vec::with_capacity(table.groups.len());
            for (group_index, group) in table.groups.iter().enumerate() {
                if canonical[group_index] != group_index as u32 {
                    groups.push(groups[canonical[group_index] as usize]);
                    continue;
                }
                if group.indices.is_empty() {
                    groups.push([0, 0]);
                    continue;
                }
                let start = mask_tail.len();
                let words = (slot.atom_count as usize).div_ceil(32);
                mask_tail.resize(start + words, 0);
                for &index in &group.indices {
                    if index < slot.atom_count {
                        mask_tail[start + index as usize / 32] |= 1 << (index % 32);
                    }
                }
                groups.push([(source_words + start) as u32, 1]);
            }
            let mut offsets = Vec::with_capacity(table.copies.len());
            for (index, copy) in table.copies.iter().enumerate() {
                let Some(&subset) = groups.get(copy.group as usize) else {
                    continue;
                };
                // Public instance tables are validated by the scene. Defend the
                // packed ID boundary for standalone renderer callers as well.
                if index >= 65_535 {
                    break;
                }
                let table_index = self.next_table_index + rows.len() as u32;
                let offset = table_index * ObjectEntry::STRIDE as u32;
                offsets.push(offset);
                let alias = crate::render_input::subset_geometry_id(
                    ObjectId(id),
                    canonical[copy.group as usize],
                );
                self.draw_offsets.entry(alias.0).or_default().push(offset);
                self.geometry_aliases
                    .entry(alias.0)
                    .or_insert(super::ObjectSlot {
                        table_index,
                        ..slot
                    });

                rows.push(ObjectEntry {
                    flags: index as u32 + 1,
                    _pad0: subset,
                    model_matrix: multiply(source.model_matrix, copy.transform),
                    ..source
                });
            }
            self.draw_offsets.insert(id, offsets);
        }
        self.mask_lut.resize(source_words + mask_tail.len(), 0);
        for (index, mask) in mask_tail.into_iter().enumerate() {
            self.mask_lut.set(source_words + index, mask);
        }
        // Retain source rows, reclaim old copy rows when copy counts shrink.
        let entries = self.next_table_index as usize + rows.len();
        self.grow_obj_table(entries);
        for (index, row) in rows.into_iter().enumerate() {
            self.write_obj_entry(self.next_table_index + index as u32, row);
        }
        // Old copy rows are no longer live. GPU capacity may remain reserved,
        // while logical allocation and pending upload bounds follow current rows.
        let bytes = entries * ObjectEntry::STRIDE as usize;
        self.obj_table_cpu.truncate(bytes);
        self.obj_table_dirty = self
            .obj_table_dirty
            .and_then(|(lo, hi)| (lo < bytes).then_some((lo, hi.min(bytes))));
    }
}

fn multiply(left: [[f32; 4]; 4], right: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    std::array::from_fn(|column| {
        std::array::from_fn(|row| (0..4).map(|k| left[k][row] * right[column][k]).sum())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use patinae_mol::instancing::{InstanceGroup, InstanceTable, ObjectInstance};

    #[test]
    fn copy_rows_share_source_storage_and_preserve_composed_transforms() {
        for count in [1, 10, 100] {
            let mut store = SceneStore::new();
            let slot = store.ensure_slot(ObjectId(7), 64, 3);
            let mut object_transform = IDENTITY_TRANSFORM;
            object_transform[3][1] = 20.0;
            store.write_obj_entry(
                slot.table_index,
                ObjectEntry {
                    atom_offset: slot.atom_offset,
                    atom_count: slot.atom_count,
                    bond_offset: slot.bond_offset,
                    bond_count: slot.bond_count,
                    object_id: 7,
                    flags: 0,
                    _pad0: [0, 0],
                    model_matrix: object_transform,
                },
            );
            store.instance_tables.insert(
                7,
                InstanceTable {
                    groups: vec![InstanceGroup {
                        indices: vec![0, 33],
                    }],
                    copies: (0..count)
                        .map(|index| {
                            let mut transform = IDENTITY_TRANSFORM;
                            transform[3][0] = index as f32 * 5.0;
                            ObjectInstance {
                                group: 0,
                                transform,
                            }
                        })
                        .collect(),
                },
            );
            store.rebuild_instance_entries();
            assert_eq!(store.atoms.cpu().len(), 64);
            assert_eq!(store.coords.cpu().len(), 64);
            assert_eq!(store.color_lut.cpu().len(), 64);
            assert_eq!(store.bonds.cpu().len(), 3);
            // Two source-mask words and two subset words, independent of copies.
            assert_eq!(store.mask_lut.cpu().len(), 4);
            assert_eq!(&store.mask_lut.cpu()[2..], &[1, 2]);
            assert_eq!(store.draw_offsets(ObjectId(7)).len(), count);
            for (index, &offset) in store.draw_offsets(ObjectId(7)).iter().enumerate() {
                let row = store.object_entry(super::super::ObjectSlot {
                    table_index: offset / ObjectEntry::STRIDE as u32,
                    ..slot
                });
                assert_eq!(row.atom_offset, slot.atom_offset);
                assert_eq!(row.bond_offset, slot.bond_offset);
                assert_eq!(row.flags, index as u32 + 1);
                assert_eq!(row.model_matrix[3], [index as f32 * 5.0, 20.0, 0.0, 1.0]);
            }
        }
    }

    #[test]
    fn equivalent_groups_share_masks_and_geometry_aliases() {
        let mut store = SceneStore::new();
        store.ensure_slot(ObjectId(1), 64, 0);
        store.instance_tables.insert(
            1,
            InstanceTable {
                groups: vec![
                    InstanceGroup {
                        indices: vec![0, 33]
                    };
                    100
                ],
                copies: (0..100)
                    .map(|group| ObjectInstance {
                        group,
                        transform: IDENTITY_TRANSFORM,
                    })
                    .collect(),
            },
        );
        store.rebuild_instance_entries();
        assert_eq!(store.mask_lut.cpu().len(), 4);
        assert_eq!(store.geometry_aliases.len(), 1);
        let alias = crate::render_input::subset_geometry_id(ObjectId(1), 0);
        assert_eq!(store.draw_offsets(alias).len(), 100);
    }

    #[test]
    fn explicit_and_empty_instanced_objects_have_different_draw_counts() {
        let mut store = SceneStore::new();
        store.ensure_slot(ObjectId(1), 1, 0);
        store.rebuild_instance_entries();
        assert_eq!(store.draw_offsets(ObjectId(1)).len(), 1);
        store.instance_tables.insert(1, InstanceTable::default());
        store.rebuild_instance_entries();
        assert!(store.draw_offsets(ObjectId(1)).is_empty());
    }
}
