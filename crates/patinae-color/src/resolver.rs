//! Color resolution without molecular context.
use crate::{Color, ColorValue, NamedPalette};

/// Resolve a literal or palette color without molecular context.
pub fn resolve_literal_color(
    color: ColorValue,
    named: &NamedPalette,
    fallback: [f32; 4],
) -> [f32; 4] {
    if let Some([r, g, b]) = color.as_rgb8() {
        Color::from_rgb8(r, g, b).to_rgba(fallback[3])
    } else if let Some(id) = color.palette_index() {
        named
            .get_by_index(id)
            .map_or(fallback, |color| color.to_rgba(fallback[3]))
    } else {
        fallback
    }
}
