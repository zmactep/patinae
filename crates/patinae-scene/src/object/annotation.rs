//! Shared semantic types for atom-anchored annotations.
//!
//! Measurement and label objects store only semantic anchors and presentation.
//! Coordinates and render primitives are resolved from these values at runtime.

use std::error::Error;
use std::fmt;

use lin_alg::f32::Vec3;
use patinae_color::ColorIndex;
use patinae_mol::{AtomIndex, AtomRemap};
use serde::{Deserialize, Deserializer, Serialize};

/// Reports an invalid annotation presentation value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnnotationPresentationError {
    /// Annotation colors cannot depend on an atom-specific color scheme.
    ColorMustBeNamed,
    /// Label sizes must be positive finite values.
    InvalidLabelSize,
}

impl fmt::Display for AnnotationPresentationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ColorMustBeNamed => formatter.write_str("annotation color must be a named color"),
            Self::InvalidLabelSize => {
                formatter.write_str("label size must be a positive finite value")
            }
        }
    }
}

impl Error for AnnotationPresentationError {}

/// References one atom in a named molecule.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AtomAnchor {
    /// Name of the source molecule object.
    pub object_name: String,
    /// Current index of the source atom.
    pub atom_index: AtomIndex,
    /// Prevents a deleted source from rebinding to a same-named object.
    #[serde(default)]
    orphaned: bool,
}

impl AtomAnchor {
    /// Creates a live atom anchor.
    pub fn new(object_name: impl Into<String>, atom_index: AtomIndex) -> Self {
        Self {
            object_name: object_name.into(),
            atom_index,
            orphaned: false,
        }
    }

    /// Returns whether the source object was permanently removed.
    pub const fn is_orphaned(&self) -> bool {
        self.orphaned
    }

    pub(crate) fn orphan_if_source(&mut self, object_name: &str) -> bool {
        if !self.orphaned && self.object_name == object_name {
            self.orphaned = true;
            true
        } else {
            false
        }
    }

    pub(crate) fn remap_if_source(&mut self, object_name: &str, remap: &AtomRemap) -> bool {
        if self.orphaned || self.object_name != object_name {
            return false;
        }

        let Some(atom_index) = remap.remap(self.atom_index) else {
            self.orphaned = true;
            return true;
        };
        if atom_index == self.atom_index {
            return false;
        }
        self.atom_index = atom_index;
        true
    }

    pub(crate) fn rename_source(&mut self, old_name: &str, new_name: &str) -> bool {
        if !self.orphaned && self.object_name == old_name {
            self.object_name = new_name.to_string();
            true
        } else {
            false
        }
    }
}

/// Selects how a semantic stroke path is patterned.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LinePattern {
    /// Draws the entire path continuously.
    Solid,
    /// Draws repeating dashes and restarts the phase for every path.
    #[default]
    Dashed,
    /// Draws repeating dots and restarts the phase for every path.
    Dotted,
}

/// Aligns text around its projected anchor point.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LabelAlignment {
    /// Aligns the bottom-left text corner to the anchor.
    #[default]
    BottomLeft,
    /// Centers the text bottom edge on the anchor.
    BottomCenter,
    /// Aligns the bottom-right text corner to the anchor.
    BottomRight,
    /// Centers the text left edge on the anchor.
    CenterLeft,
    /// Centers the text on the anchor.
    Center,
    /// Centers the text right edge on the anchor.
    CenterRight,
    /// Aligns the top-left text corner to the anchor.
    TopLeft,
    /// Centers the text top edge on the anchor.
    TopCenter,
    /// Aligns the top-right text corner to the anchor.
    TopRight,
}

impl LabelAlignment {
    /// Returns horizontal and vertical fractions used to place text around its anchor.
    pub const fn anchor_factors(self) -> (f32, f32) {
        match self {
            Self::BottomLeft => (0.0, 1.0),
            Self::BottomCenter => (0.5, 1.0),
            Self::BottomRight => (1.0, 1.0),
            Self::CenterLeft => (0.0, 0.5),
            Self::Center => (0.5, 0.5),
            Self::CenterRight => (1.0, 0.5),
            Self::TopLeft => (0.0, 0.0),
            Self::TopCenter => (0.5, 0.0),
            Self::TopRight => (1.0, 0.0),
        }
    }

    /// Returns the stable kebab-case host payload name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BottomLeft => "bottom-left",
            Self::BottomCenter => "bottom-center",
            Self::BottomRight => "bottom-right",
            Self::CenterLeft => "center-left",
            Self::Center => "center",
            Self::CenterRight => "center-right",
            Self::TopLeft => "top-left",
            Self::TopCenter => "top-center",
            Self::TopRight => "top-right",
        }
    }
}

/// Describes one renderer-neutral annotation path in world space.
///
/// Paths remain semantic until the render bridge tessellates their line
/// pattern. In particular, an arc is not flattened into segments here.
#[derive(Debug, Clone, PartialEq)]
pub enum StrokePath {
    /// One straight path whose pattern phase starts at `start`.
    Segment {
        /// World-space start point.
        start: Vec3,
        /// World-space end point.
        end: Vec3,
    },
    /// One connected path whose pattern phase spans all consecutive points.
    Polyline {
        /// World-space vertices in path order.
        points: Vec<Vec3>,
    },
    /// One signed elliptical arc in a plane.
    Arc {
        /// World-space ellipse center.
        center: Vec3,
        /// Radius-scaled local x axis.
        x_axis: Vec3,
        /// Radius-scaled local y axis.
        y_axis: Vec3,
        /// Signed sweep angle in radians.
        sweep_radians: f32,
    },
}

/// Fully resolved stroke presentation used by all annotation owners.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrokeStyle {
    /// Resolved RGBA color.
    pub color: [f32; 4],
    /// Pattern applied independently to this path.
    pub pattern: LinePattern,
    /// Base screen-space width in pixels.
    pub width_px: f32,
    /// Per-path width multiplier, used by dihedral wings.
    pub width_scale: f32,
    /// Dash length in world units.
    pub dash_length: f32,
    /// Gap length in world units.
    pub dash_gap: f32,
    /// Whether the renderer should round stroke ends.
    pub round_ends: bool,
}

/// Associates one high-level path with its resolved presentation.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedStrokePath {
    /// Semantic world-space path.
    pub path: StrokePath,
    /// Presentation resolved through entity, owner, and global defaults.
    pub style: StrokeStyle,
}

/// Stores a validated named color for an annotation primitive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct AnnotationColor(ColorIndex);

impl AnnotationColor {
    /// Creates an annotation color from a named palette index.
    ///
    /// # Errors
    ///
    /// Returns [`AnnotationPresentationError::ColorMustBeNamed`] for
    /// atom-dependent color schemes.
    pub const fn new(color: ColorIndex) -> Result<Self, AnnotationPresentationError> {
        match color {
            ColorIndex::Named(_) => Ok(Self(color)),
            _ => Err(AnnotationPresentationError::ColorMustBeNamed),
        }
    }

    /// Returns the stored named color index.
    pub const fn color_index(self) -> ColorIndex {
        self.0
    }
}

impl TryFrom<ColorIndex> for AnnotationColor {
    type Error = AnnotationPresentationError;

    fn try_from(value: ColorIndex) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for AnnotationColor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let color = ColorIndex::deserialize(deserializer)?;
        Self::new(color).map_err(serde::de::Error::custom)
    }
}

pub(super) fn has_mixed_annotation_colors(
    inherited_color: ColorIndex,
    object_color: Option<ColorIndex>,
    entity_colors: impl IntoIterator<Item = Option<ColorIndex>>,
) -> Result<bool, AnnotationPresentationError> {
    let inherited_color = AnnotationColor::new(inherited_color)?.color_index();
    let object_color = object_color.unwrap_or(inherited_color);
    let mut colors = entity_colors
        .into_iter()
        .map(|color| color.unwrap_or(object_color));
    let Some(first) = colors.next() else {
        return Ok(false);
    };
    Ok(colors.any(|color| color != first))
}

/// Stores a validated label font size.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct LabelSize(f32);

impl LabelSize {
    /// Creates a positive finite label size.
    ///
    /// # Errors
    ///
    /// Returns [`AnnotationPresentationError::InvalidLabelSize`] when `size`
    /// is non-positive, NaN, or infinite.
    pub fn new(size: f32) -> Result<Self, AnnotationPresentationError> {
        if size.is_finite() && size > 0.0 {
            Ok(Self(size))
        } else {
            Err(AnnotationPresentationError::InvalidLabelSize)
        }
    }

    /// Returns the validated font size.
    pub const fn get(self) -> f32 {
        self.0
    }
}

impl TryFrom<f32> for LabelSize {
    type Error = AnnotationPresentationError;

    fn try_from(value: f32) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for LabelSize {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let size = f32::deserialize(deserializer)?;
        Self::new(size).map_err(serde::de::Error::custom)
    }
}

/// Stores sparse measurement defaults owned by one semantic object.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeasurementObjectPresentation {
    #[serde(default)]
    color: Option<AnnotationColor>,
    #[serde(default)]
    line_pattern: Option<LinePattern>,
    #[serde(default)]
    label_visible: Option<bool>,
}

impl MeasurementObjectPresentation {
    /// Returns the object color override.
    pub const fn color(self) -> Option<ColorIndex> {
        match self.color {
            Some(color) => Some(color.color_index()),
            None => None,
        }
    }

    /// Sets the object color override.
    ///
    /// # Errors
    ///
    /// Returns an error when `color` is atom-dependent.
    pub fn set_color(&mut self, color: ColorIndex) -> Result<(), AnnotationPresentationError> {
        self.color = Some(AnnotationColor::new(color)?);
        Ok(())
    }

    /// Clears the object color override.
    pub fn clear_color(&mut self) {
        self.color = None;
    }

    /// Returns the object line-pattern override.
    pub const fn line_pattern(self) -> Option<LinePattern> {
        self.line_pattern
    }

    /// Sets the object line-pattern override.
    pub fn set_line_pattern(&mut self, line_pattern: LinePattern) {
        self.line_pattern = Some(line_pattern);
    }

    /// Clears the object line-pattern override.
    pub fn clear_line_pattern(&mut self) {
        self.line_pattern = None;
    }

    /// Returns the object measurement-label visibility override.
    pub const fn label_visible(self) -> Option<bool> {
        self.label_visible
    }

    /// Sets the object measurement-label visibility override.
    pub fn set_label_visible(&mut self, visible: bool) {
        self.label_visible = Some(visible);
    }

    /// Clears the object measurement-label visibility override.
    pub fn clear_label_visible(&mut self) {
        self.label_visible = None;
    }
}

/// Stores sparse presentation overrides for one measurement entity.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeasurementEntityPresentation {
    #[serde(default)]
    color: Option<AnnotationColor>,
    #[serde(default)]
    line_pattern: Option<LinePattern>,
    #[serde(default)]
    label_visible: Option<bool>,
}

impl MeasurementEntityPresentation {
    /// Returns the entity color override.
    pub const fn color(self) -> Option<ColorIndex> {
        match self.color {
            Some(color) => Some(color.color_index()),
            None => None,
        }
    }

    /// Sets the entity color override.
    ///
    /// # Errors
    ///
    /// Returns an error when `color` is atom-dependent.
    pub fn set_color(&mut self, color: ColorIndex) -> Result<(), AnnotationPresentationError> {
        self.color = Some(AnnotationColor::new(color)?);
        Ok(())
    }

    /// Clears the entity color override.
    pub fn clear_color(&mut self) {
        self.color = None;
    }

    /// Returns the entity line-pattern override.
    pub const fn line_pattern(self) -> Option<LinePattern> {
        self.line_pattern
    }

    /// Sets the entity line-pattern override.
    pub fn set_line_pattern(&mut self, line_pattern: LinePattern) {
        self.line_pattern = Some(line_pattern);
    }

    /// Clears the entity line-pattern override.
    pub fn clear_line_pattern(&mut self) {
        self.line_pattern = None;
    }

    /// Returns the entity measurement-label visibility override.
    pub const fn label_visible(self) -> Option<bool> {
        self.label_visible
    }

    /// Sets the entity measurement-label visibility override.
    pub fn set_label_visible(&mut self, visible: bool) {
        self.label_visible = Some(visible);
    }

    /// Clears the entity measurement-label visibility override.
    pub fn clear_label_visible(&mut self) {
        self.label_visible = None;
    }

    /// Resolves entity, object, and global presentation precedence.
    pub fn resolve(
        self,
        object: &MeasurementObjectPresentation,
        global: MeasurementPresentation,
    ) -> MeasurementPresentation {
        MeasurementPresentation {
            color: self.color.or(object.color).unwrap_or(global.color),
            line_pattern: self
                .line_pattern
                .or(object.line_pattern)
                .unwrap_or(global.line_pattern),
            label_visible: self
                .label_visible
                .or(object.label_visible)
                .unwrap_or(global.label_visible),
        }
    }
}

/// Contains fully resolved measurement presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeasurementPresentation {
    color: AnnotationColor,
    line_pattern: LinePattern,
    label_visible: bool,
}

impl MeasurementPresentation {
    /// Creates resolved measurement presentation.
    ///
    /// # Errors
    ///
    /// Returns an error when `color` is atom-dependent.
    pub fn new(
        color: ColorIndex,
        line_pattern: LinePattern,
        label_visible: bool,
    ) -> Result<Self, AnnotationPresentationError> {
        Ok(Self {
            color: AnnotationColor::new(color)?,
            line_pattern,
            label_visible,
        })
    }

    /// Returns the resolved measurement color.
    pub const fn color(self) -> ColorIndex {
        self.color.color_index()
    }

    /// Returns the resolved line pattern.
    pub const fn line_pattern(self) -> LinePattern {
        self.line_pattern
    }

    /// Returns whether the measurement label is visible.
    pub const fn label_visible(self) -> bool {
        self.label_visible
    }
}

/// Stores sparse label defaults owned by one semantic object.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct LabelObjectPresentation {
    #[serde(default)]
    color: Option<AnnotationColor>,
    #[serde(default)]
    size: Option<LabelSize>,
    #[serde(default)]
    visible: Option<bool>,
    #[serde(default)]
    alignment: Option<LabelAlignment>,
}

impl LabelObjectPresentation {
    /// Returns the object color override.
    pub const fn color(self) -> Option<ColorIndex> {
        match self.color {
            Some(color) => Some(color.color_index()),
            None => None,
        }
    }

    /// Sets the object color override.
    ///
    /// # Errors
    ///
    /// Returns an error when `color` is atom-dependent.
    pub fn set_color(&mut self, color: ColorIndex) -> Result<(), AnnotationPresentationError> {
        self.color = Some(AnnotationColor::new(color)?);
        Ok(())
    }

    /// Clears the object color override.
    pub fn clear_color(&mut self) {
        self.color = None;
    }

    /// Returns the object size override.
    pub const fn size(self) -> Option<f32> {
        match self.size {
            Some(size) => Some(size.get()),
            None => None,
        }
    }

    /// Sets the object size override.
    ///
    /// # Errors
    ///
    /// Returns an error for non-positive or non-finite values.
    pub fn set_size(&mut self, size: f32) -> Result<(), AnnotationPresentationError> {
        self.size = Some(LabelSize::new(size)?);
        Ok(())
    }

    /// Clears the object size override.
    pub fn clear_size(&mut self) {
        self.size = None;
    }

    /// Returns the object label-visibility override.
    pub const fn visible(self) -> Option<bool> {
        self.visible
    }

    /// Sets the object label-visibility override.
    pub fn set_visible(&mut self, visible: bool) {
        self.visible = Some(visible);
    }

    /// Clears the object label-visibility override.
    pub fn clear_visible(&mut self) {
        self.visible = None;
    }

    /// Returns the object label-alignment override.
    pub const fn alignment(self) -> Option<LabelAlignment> {
        self.alignment
    }

    /// Sets the object label-alignment override.
    pub fn set_alignment(&mut self, alignment: LabelAlignment) {
        self.alignment = Some(alignment);
    }

    /// Clears the object label-alignment override.
    pub fn clear_alignment(&mut self) {
        self.alignment = None;
    }
}

/// Stores sparse presentation overrides for one label entity.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct LabelEntityPresentation {
    #[serde(default)]
    color: Option<AnnotationColor>,
    #[serde(default)]
    size: Option<LabelSize>,
    #[serde(default)]
    visible: Option<bool>,
}

impl LabelEntityPresentation {
    /// Returns the entity color override.
    pub const fn color(self) -> Option<ColorIndex> {
        match self.color {
            Some(color) => Some(color.color_index()),
            None => None,
        }
    }

    /// Sets the entity color override.
    ///
    /// # Errors
    ///
    /// Returns an error when `color` is atom-dependent.
    pub fn set_color(&mut self, color: ColorIndex) -> Result<(), AnnotationPresentationError> {
        self.color = Some(AnnotationColor::new(color)?);
        Ok(())
    }

    /// Clears the entity color override.
    pub fn clear_color(&mut self) {
        self.color = None;
    }

    /// Returns the entity size override.
    pub const fn size(self) -> Option<f32> {
        match self.size {
            Some(size) => Some(size.get()),
            None => None,
        }
    }

    /// Sets the entity size override.
    ///
    /// # Errors
    ///
    /// Returns an error for non-positive or non-finite values.
    pub fn set_size(&mut self, size: f32) -> Result<(), AnnotationPresentationError> {
        self.size = Some(LabelSize::new(size)?);
        Ok(())
    }

    /// Clears the entity size override.
    pub fn clear_size(&mut self) {
        self.size = None;
    }

    /// Returns the entity visibility override.
    pub const fn visible(self) -> Option<bool> {
        self.visible
    }

    /// Sets the entity visibility override.
    pub fn set_visible(&mut self, visible: bool) {
        self.visible = Some(visible);
    }

    /// Clears the entity visibility override.
    pub fn clear_visible(&mut self) {
        self.visible = None;
    }

    /// Resolves entity, object, and global presentation precedence.
    pub fn resolve(
        self,
        object: &LabelObjectPresentation,
        global: LabelPresentation,
    ) -> LabelPresentation {
        LabelPresentation {
            color: self.color.or(object.color).unwrap_or(global.color),
            size: self.size.or(object.size).unwrap_or(global.size),
            visible: self.visible.or(object.visible).unwrap_or(global.visible),
            alignment: object.alignment.unwrap_or(global.alignment),
        }
    }
}

/// Contains fully resolved standalone-label presentation.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LabelPresentation {
    color: AnnotationColor,
    size: LabelSize,
    visible: bool,
    alignment: LabelAlignment,
}

impl LabelPresentation {
    /// Creates resolved standalone-label presentation.
    ///
    /// # Errors
    ///
    /// Returns an error for an atom-dependent color or invalid size.
    pub fn new(
        color: ColorIndex,
        size: f32,
        visible: bool,
        alignment: LabelAlignment,
    ) -> Result<Self, AnnotationPresentationError> {
        Ok(Self {
            color: AnnotationColor::new(color)?,
            size: LabelSize::new(size)?,
            visible,
            alignment,
        })
    }

    /// Returns the resolved label color.
    pub const fn color(self) -> ColorIndex {
        self.color.color_index()
    }

    /// Returns the resolved label size.
    pub const fn size(self) -> f32 {
        self.size.get()
    }

    /// Returns whether the label is visible.
    pub const fn visible(self) -> bool {
        self.visible
    }

    /// Returns the resolved label alignment.
    pub const fn alignment(self) -> LabelAlignment {
        self.alignment
    }
}

#[cfg(test)]
mod tests {
    use patinae_color::ColorIndex;

    use super::*;

    #[test]
    fn presentation_uses_entity_object_global_precedence() {
        let global = MeasurementPresentation::new(ColorIndex::Named(1), LinePattern::Solid, true)
            .expect("global presentation");
        let mut object = MeasurementObjectPresentation::default();
        object
            .set_color(ColorIndex::Named(2))
            .expect("object color");
        object.set_line_pattern(LinePattern::Dotted);
        let mut entity = MeasurementEntityPresentation::default();
        entity
            .set_color(ColorIndex::Named(3))
            .expect("entity color");
        entity.set_label_visible(false);

        let resolved = entity.resolve(&object, global);

        assert_eq!(resolved.color(), ColorIndex::Named(3));
        assert_eq!(resolved.line_pattern(), LinePattern::Dotted);
        assert!(!resolved.label_visible());
    }

    #[test]
    fn annotation_color_rejects_atom_dependent_schemes() {
        let error = AnnotationColor::new(ColorIndex::ByChain).expect_err("scheme is invalid");

        assert_eq!(error.to_string(), "annotation color must be a named color");
    }

    #[test]
    fn label_size_rejects_non_positive_and_non_finite_values() {
        for value in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert!(LabelSize::new(value).is_err(), "accepted {value:?}");
        }
    }

    #[test]
    fn default_line_pattern_is_dashed() {
        assert_eq!(LinePattern::default(), LinePattern::Dashed);
    }

    #[test]
    fn deserialization_rejects_invalid_color_and_size() {
        let color_json = serde_json::to_string(&ColorIndex::ByChain).expect("serialize color");

        let color_error = serde_json::from_str::<AnnotationColor>(&color_json)
            .expect_err("atom-dependent color must fail");
        let size_error = serde_json::from_str::<LabelSize>("0.0").expect_err("zero size must fail");

        assert!(color_error
            .to_string()
            .contains("annotation color must be a named color"));
        assert!(size_error
            .to_string()
            .contains("label size must be a positive finite value"));
    }

    #[test]
    fn label_presentation_uses_entity_object_global_precedence() {
        let global =
            LabelPresentation::new(ColorIndex::Named(1), 10.0, true, LabelAlignment::Center)
                .expect("global presentation");
        let mut object = LabelObjectPresentation::default();
        object.set_size(12.0).expect("object size");
        object.set_alignment(LabelAlignment::TopCenter);
        let mut entity = LabelEntityPresentation::default();
        entity
            .set_color(ColorIndex::Named(3))
            .expect("entity color");
        entity.set_visible(false);

        let resolved = entity.resolve(&object, global);

        assert_eq!(resolved.color(), ColorIndex::Named(3));
        assert_eq!(resolved.size(), 12.0);
        assert!(!resolved.visible());
        assert_eq!(resolved.alignment(), LabelAlignment::TopCenter);
    }
}
