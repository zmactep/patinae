//! Viewport bridge: FPS tracking, texture push, frame dimensions.

use patinae_scene::manipulation::{label_target, ViewportBounds, ViewportTarget};
use patinae_scene::{ObjectRegistry, RenderObjectId};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;
use std::time::Instant;

use slint::{ComponentHandle, Model, ModelRc, VecModel};

use crate::{AppWindow, ViewportLabel, ViewportState};

type LabelSizes = HashMap<(String, i32), [f32; 2]>;

/// Tracks frame dimensions and FPS, pushes rendered images to Slint.
pub struct ViewportBridge {
    pub width: u32,
    pub height: u32,
    frame_count: u64,
    last_fps_time: Instant,
    labels: Rc<VecModel<ViewportLabel>>,
    labels_fingerprint: Option<u64>,
    label_sizes: Rc<RefCell<LabelSizes>>,
}

impl ViewportBridge {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            frame_count: 0,
            last_fps_time: Instant::now(),
            labels: Rc::new(VecModel::default()),
            labels_fingerprint: None,
            label_sizes: Rc::default(),
        }
    }

    /// Attaches the persistent annotation label model to Slint.
    pub fn attach(&self, window: &AppWindow) {
        window
            .global::<ViewportState>()
            .set_labels(ModelRc::from(self.labels.clone()));
    }

    pub(crate) fn set_label_size(&mut self, owner: String, entity: i32, size: [f32; 2]) {
        self.label_sizes.borrow_mut().insert((owner, entity), size);
    }

    /// Uses the same text extents Slint rendered, without highlight padding.
    pub fn manipulation_targets(&self, registry: &ObjectRegistry) -> Vec<ViewportTarget> {
        let sizes = self.label_sizes.borrow();
        self.labels
            .iter()
            .filter_map(|label| {
                let [width, height] =
                    *sizes.get(&(label.owner_id.to_string(), label.insertion_ordinal))?;
                let owner = RenderObjectId::new(label.owner_id.parse().ok()?)?;
                label_target(
                    registry,
                    owner,
                    label.insertion_ordinal.try_into().ok()?,
                    ViewportBounds {
                        x: label.x - width * label.anchor_x,
                        y: label.y - height * label.anchor_y,
                        width,
                        height,
                    },
                    label.display_order.parse().ok()?,
                )
            })
            .collect()
    }

    /// Update viewport dimensions.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width.max(1);
        self.height = height.max(1);
    }

    /// Push a rendered frame image and update FPS display in Slint.
    pub fn push_frame(&mut self, image: Option<slint::Image>, vp: &ViewportState) {
        if let Some(image) = image {
            vp.set_viewport_texture(image);
        }

        self.frame_count += 1;
        let elapsed = self.last_fps_time.elapsed();
        if elapsed.as_secs() >= 1 {
            let fps = self.frame_count as f64 / elapsed.as_secs_f64();
            vp.set_fps_text(format!("{:.0} fps", fps).into());
            vp.set_fps_value(fps as f32);
            self.frame_count = 0;
            self.last_fps_time = Instant::now();
        }
    }

    /// Replaces projected annotation labels shown above the viewport image.
    pub fn set_labels(&mut self, labels: Vec<ViewportLabel>) {
        let fingerprint = label_fingerprint(&labels);
        if self.labels_fingerprint == Some(fingerprint) {
            return;
        }
        let visible: HashSet<_> = labels
            .iter()
            .map(|label| (label.owner_id.as_str(), label.insertion_ordinal))
            .collect();
        self.label_sizes
            .borrow_mut()
            .retain(|(owner, entity), _| visible.contains(&(owner.as_str(), *entity)));
        self.labels_fingerprint = Some(fingerprint);
        update_label_rows(&self.labels, labels);
    }
}

fn update_label_rows(model: &VecModel<ViewportLabel>, labels: Vec<ViewportLabel>) {
    let previous_count = model.row_count();
    let next_count = labels.len();

    for (row, label) in labels.into_iter().enumerate() {
        if row < previous_count {
            model.set_row_data(row, label);
        } else {
            model.push(label);
        }
    }

    // Removing from the tail preserves the surviving Slint repeater instances.
    for row in (next_count..previous_count).rev() {
        model.remove(row);
    }
}

fn label_fingerprint(labels: &[ViewportLabel]) -> u64 {
    let mut hasher = DefaultHasher::new();
    labels.len().hash(&mut hasher);
    for label in labels {
        label.x.to_bits().hash(&mut hasher);
        label.y.to_bits().hash(&mut hasher);
        label.text.as_str().hash(&mut hasher);
        label.color.red().hash(&mut hasher);
        label.color.green().hash(&mut hasher);
        label.color.blue().hash(&mut hasher);
        label.color.alpha().hash(&mut hasher);
        label.size.to_bits().hash(&mut hasher);
        label.anchor_x.to_bits().hash(&mut hasher);
        label.anchor_y.to_bits().hash(&mut hasher);
        label.alignment.as_str().hash(&mut hasher);
        label.owner_id.as_str().hash(&mut hasher);
        label.insertion_ordinal.hash(&mut hasher);
        label.display_order.as_str().hash(&mut hasher);
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use slint::{Color, Model, SharedString, VecModel};

    use super::*;
    fn label(x: f32, text: &str) -> ViewportLabel {
        ViewportLabel {
            x,
            y: 20.0,
            text: SharedString::from(text),
            color: Color::from_rgb_u8(34, 211, 238),
            size: 14.0,
            anchor_x: 0.5,
            anchor_y: 0.5,
            alignment: SharedString::from("center"),
            owner_id: SharedString::from("1"),
            insertion_ordinal: 0,
            display_order: SharedString::from("0"),
        }
    }

    #[test]
    fn label_rows_are_updated_without_replacing_the_model() {
        let model = VecModel::from(vec![label(10.0, "old"), label(20.0, "remove")]);

        update_label_rows(&model, vec![label(30.0, "updated")]);

        assert_eq!(model.row_count(), 1);
        let updated = model.row_data(0).expect("updated label row");
        assert_eq!(updated.x, 30.0);
        assert_eq!(updated.text.as_str(), "updated");

        update_label_rows(&model, vec![label(40.0, "first"), label(50.0, "added")]);

        assert_eq!(model.row_count(), 2);
        assert_eq!(model.row_data(0).unwrap().text.as_str(), "first");
        assert_eq!(model.row_data(1).unwrap().text.as_str(), "added");
    }
}
