//! Adapter from shared annotation bundles to generic renderer strokes.

use std::hash::{DefaultHasher, Hash, Hasher};

use patinae_color::NamedPalette;
use patinae_render::{ObjectId, RenderStrokeInput, StrokeSegment};
use patinae_settings::Settings;

use crate::{
    LinePattern, Object, ObjectRegistry, ResolvedAnnotationBundle, ResolvedStrokePath, StrokePath,
    StrokeStyle,
};

use super::ResolvedSceneAnnotations;

const EPSILON: f32 = 1.0e-6;
const MAX_PATTERN_SEGMENTS_PER_PATH: usize = 16_384;

#[derive(Debug)]
struct ResolvedStrokeRenderObject {
    object_id: ObjectId,
    segments: Vec<StrokeSegment>,
    bounds: Option<([f32; 3], [f32; 3])>,
    geometry_revision: u64,
    material_revision: u64,
}

impl ResolvedStrokeRenderObject {
    fn render_input(&self) -> RenderStrokeInput<'_> {
        RenderStrokeInput {
            object_id: self.object_id,
            segments: &self.segments,
            bounds: self.bounds,
            geometry_revision: self.geometry_revision,
            material_revision: self.material_revision,
        }
    }
}

/// Cached renderer strokes tessellated from the shared annotation bundles.
#[derive(Debug, Default)]
pub struct ResolvedSceneStrokes {
    annotations: ResolvedSceneAnnotations,
    objects: Vec<ResolvedStrokeRenderObject>,
    source_fingerprint: Option<u64>,
    #[cfg(test)]
    rebuild_count: usize,
}

impl ResolvedSceneStrokes {
    /// Returns whether semantic bundles or tessellated strokes are stale.
    pub fn needs_rebuild(
        &self,
        registry: &ObjectRegistry,
        settings: &Settings,
        named: &NamedPalette,
    ) -> bool {
        self.source_fingerprint != Some(annotation_source_fingerprint(registry, settings, named))
            || registry.has_any_dirty_molecule()
            || registry.has_any_dirty_measurement()
            || registry.has_any_dirty_label()
    }

    /// Rebuilds shared bundles once, then adapts their paths to renderer strokes.
    pub fn rebuild(
        &mut self,
        registry: &ObjectRegistry,
        settings: &Settings,
        named: &NamedPalette,
    ) {
        self.annotations.rebuild(registry, settings, named);
        self.objects.clear();

        for bundle in self.annotations.bundles() {
            let mut segments = Vec::new();
            for path in &bundle.paths {
                segments.extend(tessellate_path(path));
            }
            self.objects.push(ResolvedStrokeRenderObject {
                object_id: ObjectId(bundle.owner_id.get()),
                segments,
                bounds: bundle
                    .bounds
                    .map(|(min, max)| ([min.x, min.y, min.z], [max.x, max.y, max.z])),
                geometry_revision: bundle.geometry_revision,
                material_revision: bundle.material_revision,
            });
        }
        self.source_fingerprint = Some(annotation_source_fingerprint(registry, settings, named));
        #[cfg(test)]
        {
            self.rebuild_count += 1;
        }
    }

    /// Returns short-lived renderer inputs borrowing the compatibility payload.
    pub fn render_inputs(&self) -> Vec<RenderStrokeInput<'_>> {
        self.objects
            .iter()
            .map(ResolvedStrokeRenderObject::render_input)
            .collect()
    }

    /// Returns all authoritative measurement and standalone-label bundles.
    pub fn annotation_bundles(&self) -> &[ResolvedAnnotationBundle] {
        self.annotations.bundles()
    }

    #[cfg(test)]
    pub(super) const fn rebuild_count(&self) -> usize {
        self.rebuild_count
    }
}

fn annotation_source_fingerprint(
    registry: &ObjectRegistry,
    settings: &Settings,
    named: &NamedPalette,
) -> u64 {
    let mut hasher = DefaultHasher::new();
    registry.generation().hash(&mut hasher);

    for name in registry.names() {
        name.hash(&mut hasher);
        if let Some(molecule) = registry.get_molecule(name) {
            molecule.display_state().hash(&mut hasher);
            molecule.molecule().atom_count().hash(&mut hasher);
            for value in molecule.state().transform.data {
                value.to_bits().hash(&mut hasher);
            }
        } else if let Some(measurement) = registry.get_measurement(name) {
            measurement.is_enabled().hash(&mut hasher);
            measurement.revisions().geometry.hash(&mut hasher);
            measurement.revisions().material.hash(&mut hasher);
            measurement.revisions().labels.hash(&mut hasher);
        } else if let Some(label) = registry.get_label(name) {
            label.is_enabled().hash(&mut hasher);
            label.revisions().geometry.hash(&mut hasher);
            label.revisions().material.hash(&mut hasher);
            label.revisions().labels.hash(&mut hasher);
        }
    }

    let measurement = &settings.measurement;
    for value in [
        measurement.dash_length,
        measurement.dash_gap,
        measurement.dash_width,
        measurement.angle_size,
        measurement.angle_label_position,
        measurement.dihedral_size,
        measurement.dihedral_label_position,
        measurement.dash_transparency,
        measurement.label_size,
    ] {
        value.to_bits().hash(&mut hasher);
    }
    measurement.dash_round_ends.hash(&mut hasher);
    measurement.label_digits.hash(&mut hasher);
    measurement.label_distance_digits.hash(&mut hasher);
    measurement.label_angle_digits.hash(&mut hasher);
    measurement.label_dihedral_digits.hash(&mut hasher);
    measurement.dash_color.hash(&mut hasher);
    measurement.angle_color.hash(&mut hasher);
    measurement.dihedral_color.hash(&mut hasher);

    named.len().hash(&mut hasher);
    for (name, color) in named.iter() {
        name.hash(&mut hasher);
        color.r.to_bits().hash(&mut hasher);
        color.g.to_bits().hash(&mut hasher);
        color.b.to_bits().hash(&mut hasher);
    }
    hasher.finish()
}

fn tessellate_path(path: &ResolvedStrokePath) -> Vec<StrokeSegment> {
    match &path.path {
        StrokePath::Segment { start, end } => tessellate_straight_path(&[*start, *end], path.style),
        StrokePath::Polyline { points } => tessellate_straight_path(points, path.style),
        StrokePath::Arc {
            center,
            x_axis,
            y_axis,
            sweep_radians,
        } => tessellate_arc(*center, *x_axis, *y_axis, *sweep_radians, path.style),
    }
}

fn tessellate_straight_path(
    points: &[lin_alg::f32::Vec3],
    style: StrokeStyle,
) -> Vec<StrokeSegment> {
    if points.len() < 2 {
        return Vec::new();
    }
    if style.pattern == LinePattern::Solid {
        return points
            .windows(2)
            .filter(|pair| (pair[1] - pair[0]).magnitude() > EPSILON)
            .map(|pair| segment(pair[0], pair[1], style))
            .collect();
    }
    let lengths = points
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).magnitude())
        .collect::<Vec<_>>();
    let total_length = lengths.iter().copied().sum::<f32>();
    let Some((draw_length, stride, count)) = bounded_pattern(
        total_length,
        style.pattern,
        style.dash_length,
        style.dash_gap,
    ) else {
        return Vec::new();
    };
    let mut result = Vec::with_capacity(count);
    for index in 0..count {
        let start = index as f32 * stride;
        if start >= total_length {
            break;
        }
        let end = (start + draw_length).min(total_length);
        append_polyline_interval(points, &lengths, start, end, style, &mut result);
    }
    result
}

fn append_polyline_interval(
    points: &[lin_alg::f32::Vec3],
    lengths: &[f32],
    start: f32,
    end: f32,
    style: StrokeStyle,
    output: &mut Vec<StrokeSegment>,
) {
    let mut offset = 0.0;
    for (index, length) in lengths.iter().copied().enumerate() {
        let edge_end = offset + length;
        let local_start = start.max(offset);
        let local_end = end.min(edge_end);
        if length > EPSILON && local_end > local_start {
            let direction = (points[index + 1] - points[index]) / length;
            output.push(segment(
                points[index] + direction * (local_start - offset),
                points[index] + direction * (local_end - offset),
                style,
            ));
        }
        if edge_end >= end {
            break;
        }
        offset = edge_end;
    }
}

fn tessellate_arc(
    center: lin_alg::f32::Vec3,
    x_axis: lin_alg::f32::Vec3,
    y_axis: lin_alg::f32::Vec3,
    sweep: f32,
    style: StrokeStyle,
) -> Vec<StrokeSegment> {
    let radius = x_axis.magnitude();
    let arc_length = radius * sweep.abs();
    if radius <= EPSILON || arc_length <= EPSILON {
        return Vec::new();
    }
    if style.pattern == LinePattern::Solid {
        let count = ((arc_length / 0.1).ceil() as usize).clamp(1, MAX_PATTERN_SEGMENTS_PER_PATH);
        return (0..count)
            .map(|index| {
                let start = sweep * index as f32 / count as f32;
                let end = sweep * (index + 1) as f32 / count as f32;
                segment(
                    arc_point(center, x_axis, y_axis, start),
                    arc_point(center, x_axis, y_axis, end),
                    style,
                )
            })
            .collect();
    }
    let Some((draw_length, stride, count)) =
        bounded_pattern(arc_length, style.pattern, style.dash_length, style.dash_gap)
    else {
        return Vec::new();
    };
    let sign = sweep.signum();
    (0..count)
        .filter_map(|index| {
            let start = index as f32 * stride;
            (start < arc_length).then(|| {
                let end = (start + draw_length).min(arc_length);
                segment(
                    arc_point(center, x_axis, y_axis, start / radius * sign),
                    arc_point(center, x_axis, y_axis, end / radius * sign),
                    style,
                )
            })
        })
        .collect()
}

fn bounded_pattern(
    total_length: f32,
    pattern: LinePattern,
    dash_length: f32,
    gap: f32,
) -> Option<(f32, f32, usize)> {
    if !total_length.is_finite()
        || !dash_length.is_finite()
        || !gap.is_finite()
        || total_length <= EPSILON
        || dash_length <= EPSILON
    {
        return None;
    }
    let draw_length = if pattern == LinePattern::Dotted {
        dash_length.min(gap.max(EPSILON)) * 0.2
    } else {
        dash_length
    };
    let stride = draw_length + gap.max(0.0);
    let scale = (total_length / (MAX_PATTERN_SEGMENTS_PER_PATH as f32 * stride)).max(1.0);
    let draw_length = draw_length * scale;
    let stride = stride * scale;
    let count = (total_length / stride)
        .ceil()
        .clamp(1.0, MAX_PATTERN_SEGMENTS_PER_PATH as f32) as usize;
    Some((draw_length, stride, count))
}

fn arc_point(
    center: lin_alg::f32::Vec3,
    x_axis: lin_alg::f32::Vec3,
    y_axis: lin_alg::f32::Vec3,
    angle: f32,
) -> lin_alg::f32::Vec3 {
    center + x_axis * angle.cos() + y_axis * angle.sin()
}

fn segment(
    start: lin_alg::f32::Vec3,
    end: lin_alg::f32::Vec3,
    style: StrokeStyle,
) -> StrokeSegment {
    StrokeSegment::new(
        [start.x, start.y, start.z],
        [end.x, end.y, end.z],
        style.color,
        style.width_px * style.width_scale,
        style.round_ends,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use lin_alg::f32::Vec3;
    use patinae_mol::{Atom, AtomIndex, CoordSet, Element, ObjectMolecule};

    use crate::{AtomAnchor, LabelEntity, LabelObject, MoleculeObject};

    #[test]
    fn every_straight_path_restarts_its_pattern_phase() {
        let style = crate::StrokeStyle {
            color: [0.0, 1.0, 1.0, 1.0],
            pattern: LinePattern::Dashed,
            width_px: 2.5,
            width_scale: 1.0,
            dash_length: 0.25,
            dash_gap: 0.25,
            round_ends: true,
        };
        let first = ResolvedStrokePath {
            path: StrokePath::Segment {
                start: lin_alg::f32::Vec3::new(0.0, 0.0, 0.0),
                end: lin_alg::f32::Vec3::new(1.0, 0.0, 0.0),
            },
            style,
        };
        let second = ResolvedStrokePath {
            path: StrokePath::Segment {
                start: lin_alg::f32::Vec3::new(1.0, 0.0, 0.0),
                end: lin_alg::f32::Vec3::new(2.0, 0.0, 0.0),
            },
            style,
        };

        assert_eq!(tessellate_path(&first)[0].start, [0.0, 0.0, 0.0]);
        assert_eq!(tessellate_path(&second)[0].start, [1.0, 0.0, 0.0]);
    }

    #[test]
    fn tessellation_keeps_material_on_each_segment() {
        let first_style = crate::StrokeStyle {
            color: [1.0, 0.0, 0.0, 0.25],
            pattern: LinePattern::Solid,
            width_px: 2.0,
            width_scale: 1.0,
            dash_length: 0.25,
            dash_gap: 0.25,
            round_ends: true,
        };
        let second_style = crate::StrokeStyle {
            color: [0.0, 1.0, 0.0, 0.75],
            width_scale: 2.0,
            round_ends: false,
            ..first_style
        };
        let first = ResolvedStrokePath {
            path: StrokePath::Segment {
                start: lin_alg::f32::Vec3::new(0.0, 0.0, 0.0),
                end: lin_alg::f32::Vec3::new(1.0, 0.0, 0.0),
            },
            style: first_style,
        };
        let second = ResolvedStrokePath {
            path: StrokePath::Segment {
                start: lin_alg::f32::Vec3::new(1.0, 0.0, 0.0),
                end: lin_alg::f32::Vec3::new(2.0, 0.0, 0.0),
            },
            style: second_style,
        };

        let first = tessellate_path(&first)[0];
        let second = tessellate_path(&second)[0];
        assert_eq!(first.color, first_style.color);
        assert_eq!(first.width_px, 2.0);
        assert_eq!(first.round_ends, 1);
        assert_eq!(second.color, second_style.color);
        assert_eq!(second.width_px, 4.0);
        assert_eq!(second.round_ends, 0);
    }

    #[test]
    fn pathological_dash_settings_are_bounded() {
        let pattern = bounded_pattern(100.0, LinePattern::Dashed, EPSILON * 2.0, 0.0)
            .expect("positive dash length");

        assert!(pattern.0 > 0.0);
        assert!(pattern.2 <= MAX_PATTERN_SEGMENTS_PER_PATH);
        assert!(bounded_pattern(100.0, LinePattern::Dashed, 0.0, 0.0).is_none());
    }

    #[test]
    fn label_only_owner_keeps_bounds_without_a_drawable_gpu_payload() {
        let mut source = ObjectMolecule::new("source");
        source.add_atom(Atom::new("CA", Element::Carbon));
        source.add_coord_set(CoordSet::from_vec3(&[Vec3::new(1.0, 2.0, 3.0)]));
        let mut registry = ObjectRegistry::new();
        registry.add(MoleculeObject::with_name(source, "source"));
        registry.add(LabelObject::with_entities(
            "labels",
            vec![LabelEntity::new(
                AtomAnchor::new("source", AtomIndex(0)),
                "CA",
            )],
        ));

        let mut cache = ResolvedSceneStrokes::default();
        cache.rebuild(&registry, &Settings::default(), &NamedPalette::default());
        let inputs = cache.render_inputs();

        assert_eq!(inputs.len(), 1);
        assert!(inputs[0].segments.is_empty());
        assert_eq!(inputs[0].bounds, Some(([1.0, 2.0, 3.0], [1.0, 2.0, 3.0])));
    }
}
