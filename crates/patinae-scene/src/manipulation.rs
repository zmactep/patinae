//! Shared viewport target selection, capture, and label manipulation.
//!
//! Frontends supply rendered bounds in logical pixels. The controller reserves
//! the complete gesture even on a miss or after modifiers are released. New
//! target kinds can extend `ViewportTargetId` without changing pointer routing.

use crate::{Modifiers, ObjectRegistry, RenderObjectId};

/// Additional acquisition distance around rendered text, in logical pixels.
pub const ACQUISITION_MARGIN: f32 = 12.0;
/// Matches the normal viewport click threshold in both frontends.
pub const DRAG_THRESHOLD: f32 = 5.0;

/// Semantic identity of an independently manipulable viewport entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewportTargetId {
    /// A standalone or measurement label within one annotation owner.
    Label {
        owner: RenderObjectId,
        entity: usize,
    },
}

/// Rendered bounds in viewport-local logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewportBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl ViewportBounds {
    fn contains(self, point: [f32; 2]) -> bool {
        self.width > 0.0
            && self.height > 0.0
            && point[0] >= self.x - ACQUISITION_MARGIN
            && point[0] <= self.x + self.width + ACQUISITION_MARGIN
            && point[1] >= self.y - ACQUISITION_MARGIN
            && point[1] <= self.y + self.height + ACQUISITION_MARGIN
    }
}

/// One visible target; later paint order wins overlapping acquisition areas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewportTarget {
    pub id: ViewportTargetId,
    pub bounds: ViewportBounds,
    pub display_order: u64,
    /// Invalidates capture when a collection is changed or replaced.
    pub revision: [u64; 2],
}

/// Transient affordance, independent of label layout and semantic presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetHighlight {
    None,
    Available,
    Active,
}

/// Pointer intent; platform adapters translate it to their native cursor type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewportCursor {
    Default,
    Grab,
    Grabbing,
}

/// A single hit-test result shared by every overlay in one UI update.
#[derive(Debug, Clone, Copy)]
pub struct ManipulationFeedback {
    pub active_target: Option<ViewportTargetId>,
    pub highlights_visible: bool,
    pub cursor: ViewportCursor,
}

impl ManipulationFeedback {
    /// Classifies a visible label without repeating target acquisition.
    pub fn highlight(self, id: ViewportTargetId) -> TargetHighlight {
        if !self.highlights_visible {
            TargetHighlight::None
        } else if self.active_target == Some(id) {
            TargetHighlight::Active
        } else {
            TargetHighlight::Available
        }
    }
}

#[derive(Debug, Clone)]
struct Capture {
    target: ViewportTarget,
    start: [f32; 2],
    offset: [f32; 2],
    dragging: bool,
}

/// Routes a reserved pointer gesture before camera controls or atom picking.
#[derive(Debug, Default)]
pub struct ViewportManipulation {
    targets: Vec<ViewportTarget>,
    chord: bool,
    pointer: Option<[f32; 2]>,
    reserved: bool,
    pressed: bool,
    cancelled_click: bool,
    capture: Option<Capture>,
}

impl ViewportManipulation {
    /// Updates the visible targets and cancels a vanished or replaced capture.
    pub fn set_targets(&mut self, targets: Vec<ViewportTarget>) {
        if self.capture.as_ref().is_some_and(|capture| {
            !targets.iter().any(|target| {
                target.id == capture.target.id && target.revision == capture.target.revision
            })
        }) {
            self.capture = None;
        }
        self.targets = targets;
    }

    /// Updates modifiers; a started gesture stays reserved until pointer release.
    pub fn set_modifiers(&mut self, modifiers: Modifiers, macos: bool) {
        self.chord = modifiers.manipulation_chord(macos);
        self.reserved |= self.chord && self.pressed;
        if !self.chord {
            self.capture = None;
        }
    }

    /// Whether camera input and ordinary picking must be suppressed.
    pub fn is_reserved(&self) -> bool {
        self.chord || self.reserved
    }

    /// Returns the captured target, or the topmost target under the pointer.
    pub fn active_target(&self) -> Option<ViewportTargetId> {
        if !self.chord {
            return None;
        }
        self.capture
            .as_ref()
            .map(|capture| capture.target.id)
            .or_else(|| {
                let point = self.pointer?;
                self.targets
                    .iter()
                    .filter(|target| target.bounds.contains(point))
                    .max_by_key(|target| target.display_order)
                    .map(|target| target.id)
            })
    }

    /// Computes overlay and cursor semantics once, independently of UI styling.
    pub fn feedback(&self) -> ManipulationFeedback {
        let active_target = self.active_target();
        let cursor = if self
            .capture
            .as_ref()
            .is_some_and(|capture| capture.dragging)
        {
            ViewportCursor::Grabbing
        } else if active_target.is_some() {
            ViewportCursor::Grab
        } else {
            ViewportCursor::Default
        };
        ManipulationFeedback {
            active_target,
            highlights_visible: self.chord && self.pointer.is_some() && !self.targets.is_empty(),
            cursor,
        }
    }

    /// Captures a label on left press, reserving misses as well.
    pub fn press(&mut self, point: [f32; 2], registry: &ObjectRegistry) -> bool {
        self.pointer = Some(point);
        self.cancelled_click = false;
        self.pressed = true;
        self.reserved = self.chord;
        self.capture = None;
        if !self.reserved {
            return false;
        }
        if let Some(id) = self.active_target() {
            if let (Some(target), Some(offset)) = (
                self.targets.iter().find(|target| target.id == id),
                label_offset(registry, id),
            ) {
                self.capture = Some(Capture {
                    target: *target,
                    start: point,
                    offset,
                    dragging: false,
                });
            }
        }
        true
    }

    /// Moves only the captured entity once the logical drag threshold is crossed.
    pub fn motion(&mut self, point: [f32; 2], registry: &mut ObjectRegistry) -> bool {
        self.pointer = Some(point);
        let Some(capture) = &mut self.capture else {
            return false;
        };
        if target_revision(registry, capture.target.id) != Some(capture.target.revision) {
            self.capture = None;
            return false;
        }
        let delta = [point[0] - capture.start[0], point[1] - capture.start[1]];
        capture.dragging |= delta[0].hypot(delta[1]) >= DRAG_THRESHOLD;
        if !capture.dragging {
            return false;
        }
        set_label_offset(
            registry,
            capture.target.id,
            [capture.offset[0] + delta[0], capture.offset[1] + delta[1]],
        )
    }

    /// Finishes capture and reports whether the release must suppress a click.
    pub fn release(&mut self) -> bool {
        let consumed = self.is_reserved() || self.cancelled_click;
        self.cancelled_click = false;
        self.capture = None;
        self.reserved = false;
        self.pressed = false;
        consumed
    }

    /// Clears hover while retaining an in-progress captured drag.
    pub fn leave(&mut self) {
        self.pointer = None;
    }

    /// Terminates capture on cancellation, focus loss, or lost pointer capture.
    pub fn cancel(&mut self) {
        // A delayed native pointer-up may still publish a click after cancellation.
        self.cancelled_click = true;
        self.capture = None;
        self.reserved = false;
        self.pressed = false;
        self.pointer = None;
        self.chord = false;
    }
}

fn owner_name(registry: &ObjectRegistry, id: ViewportTargetId) -> Option<&str> {
    let ViewportTargetId::Label { owner, .. } = id;
    registry
        .names()
        .find(|name| registry.render_id(name) == Some(owner))
}

fn target_revision(registry: &ObjectRegistry, id: ViewportTargetId) -> Option<[u64; 2]> {
    label_state(registry, id).map(|(_, revision)| revision)
}

fn label_offset(registry: &ObjectRegistry, id: ViewportTargetId) -> Option<[f32; 2]> {
    label_state(registry, id).map(|(offset, _)| offset)
}

fn label_state(registry: &ObjectRegistry, id: ViewportTargetId) -> Option<([f32; 2], [u64; 2])> {
    let ViewportTargetId::Label { entity, .. } = id;
    let name = owner_name(registry, id)?;
    let (offset, revision) = if let Some(object) = registry.get_label(name) {
        (
            object.entities().get(entity)?.label_offset(),
            object.revisions().geometry,
        )
    } else {
        let object = registry.get_measurement(name)?;
        (
            object.entries().get(entity)?.label_offset(),
            object.revisions().geometry,
        )
    };
    Some((offset, [registry.generation(), revision]))
}

fn set_label_offset(registry: &mut ObjectRegistry, id: ViewportTargetId, offset: [f32; 2]) -> bool {
    let ViewportTargetId::Label { entity, .. } = id;
    let Some(name) = owner_name(registry, id).map(str::to_owned) else {
        return false;
    };
    if let Some(object) = registry.get_label_mut(&name) {
        object.set_label_offset(entity, offset)
    } else if let Some(object) = registry.get_measurement_mut(&name) {
        object.set_label_offset(entity, offset)
    } else {
        false
    }
}

/// Builds a target using current semantic identity and frontend text measurements.
pub fn label_target(
    registry: &ObjectRegistry,
    owner: RenderObjectId,
    entity: usize,
    bounds: ViewportBounds,
    display_order: u64,
) -> Option<ViewportTarget> {
    let id = ViewportTargetId::Label { owner, entity };
    let (_, revision) = label_state(registry, id)?;
    Some(ViewportTarget {
        id,
        bounds,
        display_order,
        revision,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AtomAnchor, LabelEntity, LabelObject, MeasurementEntity, MeasurementKind, MeasurementObject,
    };
    use patinae_mol::AtomIndex;

    fn fixture() -> (ObjectRegistry, Vec<ViewportTarget>) {
        let mut registry = ObjectRegistry::new();
        registry.add(LabelObject::with_entities(
            "labels",
            vec![
                LabelEntity::new(AtomAnchor::new("mol", AtomIndex(0)), "first"),
                LabelEntity::new(AtomAnchor::new("mol", AtomIndex(1)), "second"),
            ],
        ));
        let owner = registry.render_id("labels").unwrap();
        let targets = (0..2)
            .map(|entity| {
                label_target(
                    &registry,
                    owner,
                    entity,
                    ViewportBounds {
                        x: 100.0,
                        y: 100.0,
                        width: 30.0,
                        height: 14.0,
                    },
                    entity as u64,
                )
                .unwrap()
            })
            .collect();
        (registry, targets)
    }

    fn controller(targets: Vec<ViewportTarget>) -> ViewportManipulation {
        let mut controller = ViewportManipulation::default();
        controller.set_targets(targets);
        controller.set_modifiers(
            Modifiers {
                shift: true,
                ctrl: true,
                ..Modifiers::EMPTY
            },
            false,
        );
        controller
    }

    #[test]
    fn cancelled_pointer_up_cannot_turn_into_an_atom_click() {
        let (registry, targets) = fixture();
        let mut state = controller(targets);
        state.press([100.0, 100.0], &registry);
        state.cancel();
        assert!(!state.is_reserved());
        assert!(state.release());
        assert!(!state.release());
        assert!(!state.press([100.0, 100.0], &registry));
        assert!(!state.release());
    }

    #[test]
    fn chord_entered_mid_drag_reserves_release_without_acquiring_a_new_target() {
        let (mut registry, targets) = fixture();
        let id = targets[1].id;
        let mut state = ViewportManipulation::default();
        state.set_targets(targets);
        assert!(!state.press([100.0, 100.0], &registry));
        state.set_modifiers(
            Modifiers {
                shift: true,
                ctrl: true,
                ..Modifiers::EMPTY
            },
            false,
        );
        assert!(!state.motion([110.0, 100.0], &mut registry));
        state.set_modifiers(Modifiers::EMPTY, false);
        assert!(state.release());
        assert_eq!(label_offset(&registry, id), Some([0.0; 2]));
    }

    #[test]
    fn highlights_all_labels_and_acquires_topmost_with_margin() {
        let (mut registry, mut targets) = fixture();
        let top = targets[1].id;
        let back = targets[0].id;
        targets.reverse(); // Selection uses paint order, not registration order.
        let mut state = controller(targets);
        state.motion([88.0, 100.0], &mut registry);
        assert_eq!(state.active_target(), Some(top));
        let feedback = state.feedback();
        assert_eq!(feedback.highlight(top), TargetHighlight::Active);
        assert_eq!(feedback.highlight(back), TargetHighlight::Available);
        assert_eq!(feedback.cursor, ViewportCursor::Grab);
        state.motion([87.0, 100.0], &mut registry);
        assert_eq!(state.active_target(), None);
        state.leave();
        assert_eq!(state.feedback().highlight(back), TargetHighlight::None);
        state.set_modifiers(Modifiers::EMPTY, false);
        assert_eq!(state.feedback().highlight(top), TargetHighlight::None);
    }

    #[test]
    fn threshold_capture_and_release_move_only_one_entity() {
        let (mut registry, targets) = fixture();
        let top = targets[1].id;
        let mut state = controller(targets.clone());
        assert!(state.press([100.0, 100.0], &registry));
        assert!(!state.motion([102.0, 102.0], &mut registry));
        assert_eq!(label_offset(&registry, top), Some([0.0; 2]));
        assert!(state.motion([106.0, 100.0], &mut registry));
        assert!(state.motion([400.0, 350.0], &mut registry));
        assert_eq!(state.active_target(), Some(top));
        assert_eq!(state.feedback().cursor, ViewportCursor::Grabbing);
        assert_eq!(label_offset(&registry, top), Some([300.0, 250.0]));
        assert_eq!(label_offset(&registry, targets[0].id), Some([0.0; 2]));
        assert!(state.release());
        assert!(!state.motion([500.0, 500.0], &mut registry));
        assert_eq!(label_offset(&registry, top), Some([300.0, 250.0]));
    }

    #[test]
    fn miss_and_modifier_release_remain_reserved_until_mouse_up() {
        let (mut registry, targets) = fixture();
        let mut state = controller(targets);
        assert!(state.press([0.0, 0.0], &registry));
        state.set_modifiers(Modifiers::EMPTY, false);
        assert!(state.is_reserved());
        assert!(!state.motion([105.0, 105.0], &mut registry));
        assert!(state.release());
        assert!(!state.is_reserved());
    }

    #[test]
    fn cancellation_and_hidden_or_replaced_targets_end_capture() {
        for reason in 0..4 {
            let (mut registry, targets) = fixture();
            let id = targets[1].id;
            let mut state = controller(targets);
            state.press([100.0, 100.0], &registry);
            state.motion([110.0, 100.0], &mut registry);
            match reason {
                0 => state.cancel(),                // Pointer cancel, lost capture, focus loss.
                1 => state.set_targets(Vec::new()), // Hidden, unresolved, offscreen.
                2 => registry.get_label_mut("labels").unwrap().invalidate(),
                _ => state.set_modifiers(Modifiers::EMPTY, false),
            }
            assert!(!state.motion([180.0, 100.0], &mut registry));
            assert_eq!(label_offset(&registry, id), Some([10.0, 0.0]));
            assert_ne!(state.feedback().cursor, ViewportCursor::Grabbing);
        }
    }

    #[test]
    fn every_measurement_kind_has_independent_offsets_without_geometry_changes() {
        for kind in [
            MeasurementKind::Distance,
            MeasurementKind::Angle,
            MeasurementKind::Dihedral,
        ] {
            let mut registry = ObjectRegistry::new();
            let anchors = (0..kind.anchor_count())
                .map(|i| AtomAnchor::new("mol", AtomIndex(i as u32)))
                .collect::<Vec<_>>();
            let mut object = MeasurementObject::new("measurement", kind);
            object
                .add_entries(vec![
                    MeasurementEntity::new(anchors.clone()),
                    MeasurementEntity::new(anchors.clone()),
                ])
                .unwrap();
            let before = object.revisions();
            registry.add(object);
            let target = label_target(
                &registry,
                registry.render_id("measurement").unwrap(),
                1,
                ViewportBounds {
                    x: 0.0,
                    y: 0.0,
                    width: 30.0,
                    height: 14.0,
                },
                0,
            )
            .unwrap();
            let mut state = controller(vec![target]);
            state.press([0.0, 0.0], &registry);
            state.motion([25.0, -15.0], &mut registry);
            let object = registry.get_measurement("measurement").unwrap();
            assert_eq!(object.entries()[0].label_offset(), [0.0; 2]);
            assert_eq!(object.entries()[1].label_offset(), [25.0, -15.0]);
            assert_eq!(object.entries()[1].anchors, anchors);
            assert_eq!(object.revisions().geometry, before.geometry);
            assert_eq!(object.revisions().material, before.material);
        }
    }
}
