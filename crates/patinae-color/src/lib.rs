//! Color system.
//!
//! Color management for molecular visualization:
//! - Named colors (runtime registry)
//! - Gradients for continuous coloring (spectrum, b-factor)
//! - Per-scheme palettes (element, chain, SS, residue type)
//! - Themed palettes composing all schemes into one swappable unit

mod color;
mod constants;
mod error;
mod gradient;
mod named;
mod resolver;
mod scheme;
mod theme;
mod value;

pub use color::Color;
pub use constants::*;
pub use error::ColorError;
pub use gradient::Gradient;
pub use named::NamedPalette;
pub use resolver::resolve_literal_color;
pub use scheme::{
    ids as scheme_id, ChainPalette, ElementPalette, ResiduePalette, SchemeDescriptor, SsPalette,
    SCHEMES, SCHEME_NAMES,
};
pub use theme::ThemedPalette;
pub use value::{ColorKind, ColorValue, MAX_COLOR_ID};
