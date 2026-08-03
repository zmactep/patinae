//! Pure resolution of semantic annotation objects into world-space primitives.

use std::f32::consts::{PI, TAU};
use std::hash::{DefaultHasher, Hash, Hasher};

use ahash::AHashSet;
use lin_alg::f32::Vec3;
use patinae_color::{Color as RgbColor, ColorIndex, NamedPalette};
use patinae_settings::{Color as SettingColor, Settings};

use super::{
    AtomAnchor, LabelAlignment, LinePattern, MeasurementEntity, MeasurementKind, MeasurementObject,
    MeasurementResolveOptions, Object, ObjectRegistry, RenderObjectId, ResolvedStrokePath,
    StrokePath, StrokeStyle,
};

const EPSILON: f32 = 1.0e-6;
const DEFAULT_LABEL_SIZE: f32 = 14.0;
const DIHEDRAL_WING_WIDTH_SCALE: f32 = 2.0;

struct AnnotationResolveContext<'a> {
    registry: &'a ObjectRegistry,
    settings: &'a Settings,
    named: &'a NamedPalette,
}

struct ResolvedBundleContents {
    paths: Vec<ResolvedStrokePath>,
    labels: Vec<ResolvedLabelPrimitive>,
    bounds: Option<(Vec3, Vec3)>,
    unresolved_count: usize,
}

/// Identifies the semantic object type that owns one annotation bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnnotationOwnerKind {
    /// A homogeneous measurement collection.
    Measurement(MeasurementKind),
    /// An atom-anchored text-label collection.
    Label,
}

/// A recoverable warning attached to one resolved annotation owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnnotationWarning {
    /// Some stored entities currently have no resolvable output.
    UnresolvedEntities {
        /// Number of unresolved entities.
        count: usize,
    },
}

/// One world-space label primitive shared by native and web overlays.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedLabelPrimitive {
    /// Stored or derived display text.
    pub text: String,
    /// World-space anchor used by host projection.
    pub position: Vec3,
    /// Resolved RGBA color.
    pub color: [f32; 4],
    /// Resolved font size in device-independent UI pixels.
    pub size: f32,
    /// Text alignment around the projected anchor.
    pub alignment: LabelAlignment,
    /// Stable identity of the semantic owner, not of this primitive.
    pub owner_id: RenderObjectId,
    /// Position of the semantic entity in its owner collection.
    pub insertion_ordinal: usize,
    /// Deterministic cross-owner paint order.
    pub display_order: u64,
}

/// Resolved RGB color state for an annotation owner's visible primitives.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AnnotationColorSummary {
    /// The owner currently emits no visible stroke or label primitive.
    Empty,
    /// Every visible primitive resolves to the same RGB color.
    Uniform([f32; 3]),
    /// Visible primitives resolve to more than one RGB color.
    Mixed,
}

/// One authoritative renderer-neutral payload for a semantic annotation owner.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedAnnotationBundle {
    /// Registry name of the semantic owner.
    pub owner_name: String,
    /// Stable registry render identity of the owner.
    pub owner_id: RenderObjectId,
    /// Position of the owner in registry render order.
    pub owner_order: usize,
    /// Owner type and, for measurements, homogeneous measurement kind.
    pub owner_kind: AnnotationOwnerKind,
    /// Ordered high-level paths. Every path restarts its own pattern phase.
    pub paths: Vec<ResolvedStrokePath>,
    /// Ordered visible text primitives.
    pub labels: Vec<ResolvedLabelPrimitive>,
    /// World bounds derived from resolvable semantic geometry, even when the
    /// owner is currently hidden.
    pub bounds: Option<(Vec3, Vec3)>,
    /// Stored entities that currently emit neither paths nor labels.
    pub unresolved_count: usize,
    /// Visible owner warning derived from `unresolved_count`.
    pub warning: Option<AnnotationWarning>,
    /// Token for path geometry, bounds, pattern, and resolvability changes.
    pub geometry_revision: u64,
    /// Token for stroke color, width, and cap changes.
    pub material_revision: u64,
    /// Token for label text, position, style, visibility, and order changes.
    pub label_revision: u64,
}

impl ResolvedAnnotationBundle {
    /// Summarizes visible primitive colors for object-list swatches.
    pub fn color_summary(&self) -> AnnotationColorSummary {
        let colors = self
            .paths
            .iter()
            .map(|path| path.style.color)
            .chain(self.labels.iter().map(|label| label.color));
        let mut first = None;
        for rgba in colors {
            let color = [rgba[0], rgba[1], rgba[2]];
            if first.is_some_and(|first| first != color) {
                return AnnotationColorSummary::Mixed;
            }
            first = Some(color);
        }
        first.map_or(
            AnnotationColorSummary::Empty,
            AnnotationColorSummary::Uniform,
        )
    }
}

/// Resolves every measurement and label object in deterministic registry order.
pub fn resolve_annotation_bundles(
    registry: &ObjectRegistry,
    settings: &Settings,
    named: &NamedPalette,
) -> Vec<ResolvedAnnotationBundle> {
    let context = AnnotationResolveContext {
        registry,
        settings,
        named,
    };
    let mut bundles = Vec::new();
    let mut next_display_order = 0_u64;
    for (owner_order, owner_name) in registry.names().enumerate() {
        let Some(owner_id) = registry.render_id(owner_name) else {
            continue;
        };
        let owner_enabled = annotation_owner_enabled(registry, owner_name);
        let bundle = if let Some(measurement) = registry.get_measurement(owner_name) {
            Some(resolve_measurement_bundle(
                &context,
                owner_name,
                owner_id,
                owner_order,
                owner_enabled,
                measurement,
                &mut next_display_order,
            ))
        } else {
            registry.get_label(owner_name).map(|label| {
                let paths = Vec::new();
                let mut labels = Vec::new();
                let mut bounds = None;
                let mut unresolved_count = 0;
                let global_color = annotation_default_color(named);
                let global_size = valid_label_size(settings.measurement.label_size);
                for (ordinal, entity) in label.entities().iter().enumerate() {
                    let Some(position) = resolve_anchor(registry, entity.anchor()) else {
                        unresolved_count += 1;
                        continue;
                    };
                    let visible = entity
                        .presentation()
                        .visible()
                        .or(label.presentation().visible())
                        .unwrap_or(true);
                    if !visible {
                        continue;
                    }
                    include_point(&mut bounds, position);
                    if !owner_enabled {
                        continue;
                    }
                    let color = entity
                        .presentation()
                        .color()
                        .or(label.presentation().color())
                        .map_or(global_color, |color| {
                            resolve_named_color(color, named, global_color)
                        });
                    let size = valid_label_size(
                        entity
                            .presentation()
                            .size()
                            .or(label.presentation().size())
                            .unwrap_or(global_size),
                    );
                    labels.push(ResolvedLabelPrimitive {
                        text: entity.text().to_string(),
                        position,
                        color,
                        size,
                        alignment: label.presentation().alignment().unwrap_or_default(),
                        owner_id,
                        insertion_ordinal: ordinal,
                        display_order: take_display_order(&mut next_display_order),
                    });
                }
                finalize_bundle(
                    owner_name,
                    owner_id,
                    owner_order,
                    AnnotationOwnerKind::Label,
                    ResolvedBundleContents {
                        paths,
                        labels,
                        bounds,
                        unresolved_count,
                    },
                )
            })
        };
        if let Some(bundle) = bundle {
            bundles.push(bundle);
        }
    }
    bundles
}

pub(crate) fn resolve_annotation_bounds_with_options(
    registry: &ObjectRegistry,
    owner_name: &str,
    options: MeasurementResolveOptions,
) -> Option<(Vec3, Vec3)> {
    let mut settings = Settings::default();
    settings.measurement.dash_length = options.dash_length;
    settings.measurement.dash_gap = options.dash_gap;
    settings.measurement.angle_size = options.angle_size;
    settings.measurement.angle_label_position = options.angle_label_position;
    settings.measurement.dihedral_size = options.dihedral_size;
    settings.measurement.dihedral_label_position = options.dihedral_label_position;
    settings.measurement.label_distance_digits = options.distance_digits.min(9) as i32;
    settings.measurement.label_angle_digits = options.angle_digits.min(9) as i32;
    settings.measurement.label_dihedral_digits = options.dihedral_digits.min(9) as i32;
    resolve_annotation_bundles(registry, &settings, &NamedPalette::default())
        .into_iter()
        .find(|bundle| bundle.owner_name == owner_name)
        .and_then(|bundle| bundle.bounds)
}

/// Resolves one measurement entity through current atom coordinates and transforms.
///
/// This is the command-validation entry point. It uses the same geometry math as
/// the authoritative annotation bundle resolver without creating a second
/// render-facing measurement DTO.
pub fn resolve_measurement_entity_value(
    registry: &ObjectRegistry,
    kind: MeasurementKind,
    entity: &MeasurementEntity,
    options: MeasurementResolveOptions,
) -> Option<f64> {
    if entity.anchors.len() != kind.anchor_count() {
        return None;
    }
    let points = entity
        .anchors
        .iter()
        .map(|anchor| resolve_anchor(registry, anchor))
        .collect::<Option<Vec<_>>>()?;
    let style = StrokeStyle {
        color: [1.0; 4],
        pattern: LinePattern::Solid,
        width_px: 1.0,
        width_scale: 1.0,
        dash_length: options.dash_length,
        dash_gap: options.dash_gap,
        round_ends: true,
    };
    resolve_measurement_geometry(kind, &points, options, style).map(|geometry| geometry.value)
}

fn resolve_anchor(registry: &ObjectRegistry, anchor: &AtomAnchor) -> Option<Vec3> {
    if anchor.is_orphaned() {
        return None;
    }
    let molecule = registry.get_molecule(&anchor.object_name)?;
    let point = molecule.display_coord(anchor.atom_index)?;
    let transform = molecule.state().transform.clone();
    let transformed = transform * lin_alg::f32::Vec4::new(point.x, point.y, point.z, 1.0);
    Some(Vec3::new(transformed.x, transformed.y, transformed.z))
}

fn resolve_measurement_bundle(
    context: &AnnotationResolveContext<'_>,
    owner_name: &str,
    owner_id: RenderObjectId,
    owner_order: usize,
    owner_enabled: bool,
    measurement: &MeasurementObject,
    next_display_order: &mut u64,
) -> ResolvedAnnotationBundle {
    let measurement_settings = &context.settings.measurement;
    let options = MeasurementResolveOptions::from_settings(measurement_settings);
    let fallback_color =
        measurement_kind_color(measurement.kind(), measurement_settings, context.named);
    let mut paths = Vec::new();
    let mut labels = Vec::new();
    let mut bounds = None;
    let mut unresolved_count = 0;

    for (ordinal, entity) in measurement.entries().iter().enumerate() {
        let points = entity
            .anchors
            .iter()
            .map(|anchor| resolve_anchor(context.registry, anchor))
            .collect::<Option<Vec<_>>>();
        let Some(points) = points else {
            unresolved_count += 1;
            continue;
        };
        let color = entity
            .presentation()
            .color()
            .or(measurement.presentation().color())
            .map_or(fallback_color, |color| {
                with_alpha(
                    resolve_named_color(color, context.named, fallback_color),
                    1.0 - measurement_settings.dash_transparency.clamp(0.0, 1.0),
                )
            });
        let pattern = entity
            .presentation()
            .line_pattern()
            .or(measurement.presentation().line_pattern())
            .unwrap_or_default();
        let style = StrokeStyle {
            color,
            pattern,
            width_px: measurement_settings.dash_width.max(0.0),
            width_scale: 1.0,
            dash_length: measurement_settings.dash_length.max(0.0),
            dash_gap: measurement_settings.dash_gap.max(0.0),
            round_ends: measurement_settings.dash_round_ends,
        };
        let Some(geometry) =
            resolve_measurement_geometry(measurement.kind(), &points, options, style)
        else {
            unresolved_count += 1;
            continue;
        };
        for path in &geometry.paths {
            include_path_bounds(&mut bounds, &path.path);
        }
        include_point(&mut bounds, geometry.label_position);
        if !owner_enabled {
            continue;
        }
        paths.extend(geometry.paths);
        let label_visible = entity
            .presentation()
            .label_visible()
            .or(measurement.presentation().label_visible())
            .unwrap_or(true);
        if label_visible {
            labels.push(ResolvedLabelPrimitive {
                text: format_measurement_label(measurement.kind(), geometry.value, options),
                position: geometry.label_position,
                color,
                size: valid_label_size(measurement_settings.label_size),
                alignment: LabelAlignment::Center,
                owner_id,
                insertion_ordinal: ordinal,
                display_order: take_display_order(next_display_order),
            });
        }
    }

    finalize_bundle(
        owner_name,
        owner_id,
        owner_order,
        AnnotationOwnerKind::Measurement(measurement.kind()),
        ResolvedBundleContents {
            paths,
            labels,
            bounds,
            unresolved_count,
        },
    )
}

fn finalize_bundle(
    owner_name: &str,
    owner_id: RenderObjectId,
    owner_order: usize,
    owner_kind: AnnotationOwnerKind,
    contents: ResolvedBundleContents,
) -> ResolvedAnnotationBundle {
    let ResolvedBundleContents {
        paths,
        labels,
        bounds,
        unresolved_count,
    } = contents;
    let warning = (unresolved_count != 0).then_some(AnnotationWarning::UnresolvedEntities {
        count: unresolved_count,
    });
    let geometry_revision = geometry_revision(&paths, bounds, unresolved_count);
    let material_revision = material_revision(&paths);
    let label_revision = label_revision(&labels);
    ResolvedAnnotationBundle {
        owner_name: owner_name.to_string(),
        owner_id,
        owner_order,
        owner_kind,
        paths,
        labels,
        bounds,
        unresolved_count,
        warning,
        geometry_revision,
        material_revision,
        label_revision,
    }
}

fn annotation_owner_enabled(registry: &ObjectRegistry, owner_name: &str) -> bool {
    if !registry.get(owner_name).is_some_and(Object::is_enabled) {
        return false;
    }
    let mut current = owner_name;
    let mut visited = AHashSet::new();
    while let Some(parent) = registry.parent_group(current) {
        if !visited.insert(parent) || !registry.get(parent).is_some_and(Object::is_enabled) {
            return false;
        }
        current = parent;
    }
    true
}

fn take_display_order(next: &mut u64) -> u64 {
    let current = *next;
    *next = next.saturating_add(1);
    current
}

fn valid_label_size(size: f32) -> f32 {
    if size.is_finite() && size > 0.0 {
        size
    } else {
        DEFAULT_LABEL_SIZE
    }
}

fn measurement_kind_color(
    kind: MeasurementKind,
    settings: &patinae_settings::groups::MeasurementSettings,
    named: &NamedPalette,
) -> [f32; 4] {
    let cyan = annotation_default_color(named);
    let setting = match kind {
        MeasurementKind::Distance => settings.dash_color,
        MeasurementKind::Angle => settings.angle_color,
        MeasurementKind::Dihedral => settings.dihedral_color,
    };
    with_alpha(
        resolve_setting_color(setting, named, cyan),
        1.0 - settings.dash_transparency.clamp(0.0, 1.0),
    )
}

fn annotation_default_color(named: &NamedPalette) -> [f32; 4] {
    named
        .get_by_name("cyan")
        .map(|(_, color)| color.to_rgba(1.0))
        .unwrap_or([
            0x22 as f32 / 255.0,
            0xD3 as f32 / 255.0,
            0xEE as f32 / 255.0,
            1.0,
        ])
}

fn resolve_setting_color(
    setting: SettingColor,
    named: &NamedPalette,
    fallback: [f32; 4],
) -> [f32; 4] {
    if setting.0 < 0 {
        return fallback;
    }
    named
        .get_by_index(setting.0 as u32)
        .unwrap_or_else(|| RgbColor::from_packed_rgb(setting.0))
        .to_rgba(fallback[3])
}

fn resolve_named_color(color: ColorIndex, named: &NamedPalette, fallback: [f32; 4]) -> [f32; 4] {
    match color {
        ColorIndex::Named(index) => named
            .get_by_index(index)
            .map(|color| color.to_rgba(fallback[3]))
            .unwrap_or(fallback),
        _ => fallback,
    }
}

fn with_alpha(mut color: [f32; 4], alpha: f32) -> [f32; 4] {
    color[3] = alpha;
    color
}

#[derive(Debug)]
pub(crate) struct ResolvedMeasurementGeometry {
    pub(crate) value: f64,
    pub(crate) label_position: Vec3,
    pub(crate) paths: Vec<ResolvedStrokePath>,
}

pub(crate) fn resolve_measurement_geometry(
    kind: MeasurementKind,
    points: &[Vec3],
    options: MeasurementResolveOptions,
    style: StrokeStyle,
) -> Option<ResolvedMeasurementGeometry> {
    if points.len() != kind.anchor_count() {
        return None;
    }
    let mut paths = Vec::with_capacity(match kind {
        MeasurementKind::Distance => 1,
        MeasurementKind::Angle => 3,
        MeasurementKind::Dihedral => 6,
    });
    let (value, label_position) = match kind {
        MeasurementKind::Distance => {
            let value = distance_value(points[0], points[1])?;
            paths.push(segment(points[0], points[1], style));
            (value, (points[0] + points[1]) * 0.5)
        }
        MeasurementKind::Angle => {
            let value = angle_value(points[0], points[1], points[2])?;
            let arc =
                ArcFrame::for_angle(points[0], points[1], points[2], value, options.angle_size)?;
            paths.push(segment(points[0], points[1], style));
            paths.push(segment(points[1], points[2], style));
            paths.push(arc.path(style));
            (value, arc.label_position(options.angle_label_position))
        }
        MeasurementKind::Dihedral => {
            let value = dihedral_value(points[0], points[1], points[2], points[3])?;
            let arc = ArcFrame::for_dihedral(
                points[0],
                points[1],
                points[2],
                points[3],
                value,
                options.dihedral_size,
            )?;
            paths.push(segment(points[0], points[1], style));
            paths.push(segment(points[1], points[2], style));
            paths.push(segment(points[2], points[3], style));
            paths.push(arc.path(style));
            let mut wing_style = style;
            wing_style.pattern = LinePattern::Solid;
            wing_style.width_scale = DIHEDRAL_WING_WIDTH_SCALE;
            paths.extend(arc.wings(wing_style));
            (value, arc.label_position(options.dihedral_label_position))
        }
    };
    Some(ResolvedMeasurementGeometry {
        value,
        label_position,
        paths,
    })
}

fn segment(start: Vec3, end: Vec3, style: StrokeStyle) -> ResolvedStrokePath {
    ResolvedStrokePath {
        path: StrokePath::Segment { start, end },
        style,
    }
}

fn distance_value(first: Vec3, second: Vec3) -> Option<f64> {
    let value = (second - first).magnitude();
    (value > EPSILON && value.is_finite()).then_some(f64::from(value))
}

fn angle_value(first: Vec3, vertex: Vec3, third: Vec3) -> Option<f64> {
    let first_leg = first - vertex;
    let second_leg = third - vertex;
    let first_length = first_leg.magnitude();
    let second_length = second_leg.magnitude();
    if first_length <= EPSILON || second_length <= EPSILON {
        return None;
    }
    let cosine = (first_leg.dot(second_leg) / (first_length * second_length)).clamp(-1.0, 1.0);
    let radians = cosine.acos();
    radians
        .is_finite()
        .then_some(f64::from(radians.to_degrees()))
}

fn dihedral_value(first: Vec3, second: Vec3, third: Vec3, fourth: Vec3) -> Option<f64> {
    let first_bond = second - first;
    let central_bond = third - second;
    let third_bond = fourth - third;
    let central_length = central_bond.magnitude();
    if central_length <= EPSILON {
        return None;
    }
    let first_normal = first_bond.cross(central_bond);
    let second_normal = central_bond.cross(third_bond);
    let first_normal_length = first_normal.magnitude();
    let second_normal_length = second_normal.magnitude();
    if first_normal_length <= EPSILON || second_normal_length <= EPSILON {
        return None;
    }
    let cosine = (first_normal.dot(second_normal) / (first_normal_length * second_normal_length))
        .clamp(-1.0, 1.0);
    let mut angle = f64::from(cosine.acos().to_degrees());
    if second_normal.dot(central_bond.cross(first_normal)) < 0.0 {
        angle = -angle;
    }
    angle.is_finite().then_some(angle)
}

pub(crate) fn format_measurement_label(
    kind: MeasurementKind,
    value: f64,
    options: MeasurementResolveOptions,
) -> String {
    match kind {
        MeasurementKind::Distance => {
            format!("{value:.precision$} Å", precision = options.distance_digits)
        }
        MeasurementKind::Angle => {
            format!("{value:.precision$}°", precision = options.angle_digits)
        }
        MeasurementKind::Dihedral => {
            format!("{value:+.precision$}°", precision = options.dihedral_digits)
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ArcFrame {
    center: Vec3,
    x_axis: Vec3,
    y_axis: Vec3,
    sweep_radians: f32,
}

impl ArcFrame {
    fn for_angle(
        first: Vec3,
        vertex: Vec3,
        third: Vec3,
        value_degrees: f64,
        size: f32,
    ) -> Option<Self> {
        let first_leg = first - vertex;
        let second_leg = third - vertex;
        let first_length = first_leg.magnitude();
        let second_length = second_leg.magnitude();
        if first_length <= EPSILON || second_length <= EPSILON || !size.is_finite() {
            return None;
        }
        let radius = first_length.min(second_length) * size.max(EPSILON);
        let (x_axis, y_axis) =
            perpendicular_frame(first_leg / first_length, second_leg / second_length, radius)?;
        Some(Self {
            center: vertex,
            x_axis,
            y_axis,
            sweep_radians: (value_degrees as f32).to_radians(),
        })
    }

    fn for_dihedral(
        first: Vec3,
        second: Vec3,
        third: Vec3,
        fourth: Vec3,
        value_degrees: f64,
        size: f32,
    ) -> Option<Self> {
        let central_bond = third - second;
        let central_length = central_bond.magnitude();
        if central_length <= EPSILON || !size.is_finite() {
            return None;
        }
        let axis = central_bond / central_length;
        let first_projection = project_perpendicular(first - second, axis);
        let second_projection = project_perpendicular(fourth - third, axis);
        let first_length = first_projection.magnitude();
        let second_length = second_projection.magnitude();
        if first_length <= EPSILON || second_length <= EPSILON {
            return None;
        }
        let radius = first_length.min(second_length) * size.max(EPSILON);
        let (x_axis, mut y_axis) = perpendicular_frame(
            first_projection / first_length,
            second_projection / second_length,
            radius,
        )?;
        if value_degrees < 0.0 {
            y_axis *= -1.0;
        }
        Some(Self {
            center: (second + third) * 0.5,
            x_axis,
            y_axis,
            sweep_radians: (value_degrees as f32).to_radians(),
        })
    }

    fn path(self, style: StrokeStyle) -> ResolvedStrokePath {
        ResolvedStrokePath {
            path: StrokePath::Arc {
                center: self.center,
                x_axis: self.x_axis,
                y_axis: self.y_axis,
                sweep_radians: self.sweep_radians,
            },
            style,
        }
    }

    fn label_position(self, scale: f32) -> Vec3 {
        let scale = if scale.is_finite() { scale } else { 1.0 };
        let half_angle = self.sweep_radians * 0.5;
        self.center + (self.x_axis * half_angle.cos() + self.y_axis * half_angle.sin()) * scale
    }

    fn wings(self, style: StrokeStyle) -> [ResolvedStrokePath; 2] {
        const WING_EXTENSION: f32 = 1.2;
        let first_end = self.center + self.x_axis * WING_EXTENSION;
        let second_offset =
            self.x_axis * self.sweep_radians.cos() + self.y_axis * self.sweep_radians.sin();
        let second_end = self.center + second_offset * WING_EXTENSION;
        [
            segment(self.center, first_end, style),
            segment(self.center, second_end, style),
        ]
    }
}

fn project_perpendicular(vector: Vec3, axis: Vec3) -> Vec3 {
    vector - axis * vector.dot(axis)
}

fn perpendicular_frame(first: Vec3, second: Vec3, radius: f32) -> Option<(Vec3, Vec3)> {
    let perpendicular = second - first * second.dot(first);
    let length = perpendicular.magnitude();
    let y_axis = if length > EPSILON {
        perpendicular / length
    } else {
        deterministic_perpendicular(first)?
    };
    Some((first * radius, y_axis * radius))
}

fn deterministic_perpendicular(direction: Vec3) -> Option<Vec3> {
    let direction_length = direction.magnitude();
    if direction_length <= EPSILON {
        return None;
    }
    let direction = direction / direction_length;
    let basis = if direction.x.abs() <= direction.y.abs() && direction.x.abs() <= direction.z.abs()
    {
        Vec3::new(1.0, 0.0, 0.0)
    } else if direction.y.abs() <= direction.z.abs() {
        Vec3::new(0.0, 1.0, 0.0)
    } else {
        Vec3::new(0.0, 0.0, 1.0)
    };
    let perpendicular = direction.cross(basis);
    let length = perpendicular.magnitude();
    (length > EPSILON).then_some(perpendicular / length)
}

fn include_path_bounds(bounds: &mut Option<(Vec3, Vec3)>, path: &StrokePath) {
    match path {
        StrokePath::Segment { start, end } => {
            include_point(bounds, *start);
            include_point(bounds, *end);
        }
        StrokePath::Polyline { points } => {
            for point in points {
                include_point(bounds, *point);
            }
        }
        StrokePath::Arc {
            center,
            x_axis,
            y_axis,
            sweep_radians,
        } => {
            include_point(bounds, arc_point(*center, *x_axis, *y_axis, 0.0));
            include_point(bounds, arc_point(*center, *x_axis, *y_axis, *sweep_radians));
            for component in 0..3 {
                let x = component_value(*x_axis, component);
                let y = component_value(*y_axis, component);
                let extremum = y.atan2(x);
                for angle in [extremum, extremum + PI] {
                    if angle_on_sweep(angle, *sweep_radians) {
                        include_point(bounds, arc_point(*center, *x_axis, *y_axis, angle));
                    }
                }
            }
        }
    }
}

fn arc_point(center: Vec3, x_axis: Vec3, y_axis: Vec3, angle: f32) -> Vec3 {
    center + x_axis * angle.cos() + y_axis * angle.sin()
}

fn component_value(vector: Vec3, component: usize) -> f32 {
    match component {
        0 => vector.x,
        1 => vector.y,
        _ => vector.z,
    }
}

fn angle_on_sweep(angle: f32, sweep: f32) -> bool {
    let directed = if sweep >= 0.0 { angle } else { -angle };
    directed.rem_euclid(TAU) <= sweep.abs() + EPSILON
}

fn include_point(bounds: &mut Option<(Vec3, Vec3)>, point: Vec3) {
    if !point.x.is_finite() || !point.y.is_finite() || !point.z.is_finite() {
        return;
    }
    match bounds {
        None => *bounds = Some((point, point)),
        Some((min, max)) => {
            min.x = min.x.min(point.x);
            min.y = min.y.min(point.y);
            min.z = min.z.min(point.z);
            max.x = max.x.max(point.x);
            max.y = max.y.max(point.y);
            max.z = max.z.max(point.z);
        }
    }
}

fn geometry_revision(
    paths: &[ResolvedStrokePath],
    bounds: Option<(Vec3, Vec3)>,
    unresolved_count: usize,
) -> u64 {
    let mut hasher = DefaultHasher::new();
    paths.len().hash(&mut hasher);
    unresolved_count.hash(&mut hasher);
    if let Some((min, max)) = bounds {
        hash_vec3(min, &mut hasher);
        hash_vec3(max, &mut hasher);
    }
    for path in paths {
        hash_path(&path.path, &mut hasher);
        path.style.pattern.hash(&mut hasher);
        hash_f32(path.style.dash_length, &mut hasher);
        hash_f32(path.style.dash_gap, &mut hasher);
    }
    hasher.finish().max(1)
}

fn material_revision(paths: &[ResolvedStrokePath]) -> u64 {
    let mut hasher = DefaultHasher::new();
    paths.len().hash(&mut hasher);
    for path in paths {
        hash_f32_slice(&path.style.color, &mut hasher);
        hash_f32(path.style.width_px, &mut hasher);
        hash_f32(path.style.width_scale, &mut hasher);
        path.style.round_ends.hash(&mut hasher);
    }
    hasher.finish().max(1)
}

fn label_revision(labels: &[ResolvedLabelPrimitive]) -> u64 {
    let mut hasher = DefaultHasher::new();
    labels.len().hash(&mut hasher);
    for label in labels {
        label.text.hash(&mut hasher);
        hash_vec3(label.position, &mut hasher);
        hash_f32_slice(&label.color, &mut hasher);
        hash_f32(label.size, &mut hasher);
        label.alignment.hash(&mut hasher);
        label.owner_id.get().hash(&mut hasher);
        label.insertion_ordinal.hash(&mut hasher);
        label.display_order.hash(&mut hasher);
    }
    hasher.finish().max(1)
}

fn hash_path(path: &StrokePath, hasher: &mut impl Hasher) {
    match path {
        StrokePath::Segment { start, end } => {
            0_u8.hash(hasher);
            hash_vec3(*start, hasher);
            hash_vec3(*end, hasher);
        }
        StrokePath::Polyline { points } => {
            1_u8.hash(hasher);
            points.len().hash(hasher);
            for point in points {
                hash_vec3(*point, hasher);
            }
        }
        StrokePath::Arc {
            center,
            x_axis,
            y_axis,
            sweep_radians,
        } => {
            2_u8.hash(hasher);
            hash_vec3(*center, hasher);
            hash_vec3(*x_axis, hasher);
            hash_vec3(*y_axis, hasher);
            hash_f32(*sweep_radians, hasher);
        }
    }
}

fn hash_vec3(value: Vec3, hasher: &mut impl Hasher) {
    hash_f32_slice(&[value.x, value.y, value.z], hasher);
}

fn hash_f32_slice(values: &[f32], hasher: &mut impl Hasher) {
    for value in values {
        hash_f32(*value, hasher);
    }
}

fn hash_f32(value: f32, hasher: &mut impl Hasher) {
    value.to_bits().hash(hasher);
}
