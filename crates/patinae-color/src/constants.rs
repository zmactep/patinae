//! All color constants for the Patinae palette.

use crate::Color;

// --- Element colors ---------------------------------------------------------

pub const ELEM_UNKNOWN: Color = Color::new(0.655, 0.545, 0.980);
pub const ELEM_H: Color = Color::new(0.898, 0.898, 0.910);
pub const ELEM_C: Color = Color::new(0.435, 0.478, 0.541);
pub const ELEM_N: Color = Color::new(0.380, 0.494, 0.847);
pub const ELEM_O: Color = Color::new(0.878, 0.361, 0.424);
pub const ELEM_P: Color = Color::new(0.984, 0.616, 0.341);
pub const ELEM_S: Color = Color::new(0.902, 0.776, 0.251);
pub const ELEM_F: Color = Color::new(0.549, 0.831, 0.737);
pub const ELEM_CL: Color = Color::new(0.361, 0.729, 0.541);
pub const ELEM_BR: Color = Color::new(0.624, 0.341, 0.361);
pub const ELEM_I: Color = Color::new(0.439, 0.294, 0.525);
pub const ELEM_HE: Color = Color::new(0.831, 0.894, 0.914);
pub const ELEM_NE: Color = Color::new(0.784, 0.859, 0.890);
pub const ELEM_AR: Color = Color::new(0.737, 0.816, 0.867);
pub const ELEM_LI: Color = Color::new(0.800, 0.651, 0.925);
pub const ELEM_NA: Color = Color::new(0.667, 0.518, 0.847);
pub const ELEM_K: Color = Color::new(0.537, 0.416, 0.737);
pub const ELEM_BE: Color = Color::new(0.702, 0.863, 0.404);
#[expect(
    clippy::approx_constant,
    reason = "magnesium palette component, not FRAC_1_PI"
)]
pub const ELEM_MG: Color = Color::new(0.573, 0.784, 0.318);
pub const ELEM_CA: Color = Color::new(0.451, 0.706, 0.278);
pub const ELEM_B: Color = Color::new(0.933, 0.663, 0.663);
pub const ELEM_AL: Color = Color::new(0.663, 0.620, 0.627);
pub const ELEM_SI: Color = Color::new(0.824, 0.722, 0.588);
pub const ELEM_FE: Color = Color::new(0.824, 0.447, 0.231);
pub const ELEM_CU: Color = Color::new(0.725, 0.522, 0.298);
pub const ELEM_ZN: Color = Color::new(0.584, 0.655, 0.761);

// --- Secondary structure colors ---------------------------------------------

pub const SS_HELIX: Color = Color::new(0.957, 0.447, 0.714);
pub const SS_SHEET: Color = Color::new(0.992, 0.902, 0.541);
pub const SS_LOOP: Color = Color::new(0.557, 0.557, 0.596);
pub const SS_NUCLEIC: Color = Color::new(0.655, 0.545, 0.980);

// --- Residue type colors ----------------------------------------------------

pub const RES_HYDROPHOBIC: Color = Color::new(0.741, 0.690, 0.569);
pub const RES_POSITIVE: Color = Color::new(0.439, 0.569, 0.875);
pub const RES_NEGATIVE: Color = Color::new(0.906, 0.490, 0.537);
pub const RES_POLAR: Color = Color::new(0.498, 0.769, 0.812);
pub const RES_CYSTEINE: Color = Color::new(0.902, 0.776, 0.251);
pub const RES_GLYCINE: Color = Color::new(0.557, 0.557, 0.596);

// --- Named colors (canonical definitions) -----------------------------------
// These are the source of truth. Chain constants below reuse them.

// Basic
pub const WHITE: Color = Color::new(1.0, 1.0, 1.0);
pub const BLACK: Color = Color::new(0.0, 0.0, 0.0);
pub const RED: Color = Color::new(1.0, 0.0, 0.0);
pub const GREEN: Color = Color::new(0.0, 1.0, 0.0);
pub const BLUE: Color = Color::new(0.0, 0.0, 1.0);
pub const YELLOW: Color = Color::new(1.0, 1.0, 0.0);
pub const CYAN: Color = Color::from_rgb8(0x22, 0xD3, 0xEE);
pub const MAGENTA: Color = Color::new(1.0, 0.0, 1.0);

// Gray scale
pub const GRAY: Color = Color::new(0.5, 0.5, 0.5);
pub const GRAY10: Color = Color::new(0.1, 0.1, 0.1);
pub const GRAY20: Color = Color::new(0.2, 0.2, 0.2);
pub const GRAY30: Color = Color::new(0.3, 0.3, 0.3);
pub const GRAY40: Color = Color::new(0.4, 0.4, 0.4);
pub const GRAY50: Color = Color::new(0.5, 0.5, 0.5);
pub const GRAY60: Color = Color::new(0.6, 0.6, 0.6);
pub const GRAY70: Color = Color::new(0.7, 0.7, 0.7);
pub const GRAY80: Color = Color::new(0.8, 0.8, 0.8);
pub const GRAY90: Color = Color::new(0.9, 0.9, 0.9);

// Extended palette
pub const ORANGE: Color = Color::from_rgb8(0xFB, 0x92, 0x3C);
pub const PINK: Color = Color::from_rgb8(0xF4, 0x72, 0xB6);
pub const PURPLE: Color = Color::new(0.75, 0.0, 0.75);
pub const BROWN: Color = Color::new(0.65, 0.32, 0.17);
pub const SALMON: Color = Color::new(1.0, 0.6, 0.6);
pub const LIME: Color = Color::new(0.5, 1.0, 0.5);
pub const SLATE: Color = Color::new(0.5, 0.5, 1.0);
pub const HOTPINK: Color = Color::new(1.0, 0.0, 0.5);
pub const TEAL: Color = Color::from_rgb8(0x0E, 0x74, 0x90);
pub const OLIVE: Color = Color::new(0.77, 0.7, 0.0);
pub const MARINE: Color = Color::new(0.0, 0.5, 1.0);
pub const FOREST: Color = Color::from_rgb8(0x04, 0x78, 0x57);
pub const FIREBRICK: Color = Color::new(0.7, 0.13, 0.13);
pub const CHOCOLATE: Color = Color::new(0.55, 0.27, 0.07);
pub const WHEAT: Color = Color::new(0.99, 0.82, 0.65);
pub const VIOLET: Color = Color::from_rgb8(0xA7, 0x8B, 0xFA);
pub const LIGHT_BLUE: Color = Color::new(0.75, 0.75, 1.0);
pub const LIGHT_GREEN: Color = Color::new(0.75, 1.0, 0.75);
pub const LIGHT_ORANGE: Color = Color::new(1.0, 0.8, 0.5);
pub const LIGHT_PINK: Color = Color::new(1.0, 0.75, 0.87);
pub const PALE_CYAN: Color = Color::new(0.8, 1.0, 1.0);
pub const PALE_YELLOW: Color = Color::new(1.0, 1.0, 0.8);
pub const PALE_GREEN: Color = Color::new(0.65, 0.9, 0.65);
pub const AQUAMARINE: Color = Color::new(0.5, 1.0, 0.83);
pub const DEEP_TEAL: Color = Color::new(0.1, 0.6, 0.6);
pub const DEEP_PURPLE: Color = Color::new(0.6, 0.1, 0.6);
pub const DEEP_OLIVE: Color = Color::new(0.6, 0.6, 0.1);
pub const DEEP_SALMON: Color = Color::new(1.0, 0.42, 0.42);
pub const AMBER: Color = Color::from_rgb8(0xFD, 0xE6, 0x8A);
pub const EMERALD: Color = Color::from_rgb8(0x34, 0xD3, 0x99);
pub const BURGUNDY: Color = Color::from_rgb8(0xBE, 0x18, 0x5D);
pub const DEEP_VIOLET: Color = Color::from_rgb8(0x6D, 0x28, 0xD9);
pub const BURNT_ORANGE: Color = Color::from_rgb8(0xD9, 0x77, 0x06);
pub const TERRACOTTA: Color = Color::from_rgb8(0x9A, 0x34, 0x12);

// TV colors
pub const TV_RED: Color = Color::new(1.0, 0.2, 0.2);
pub const TV_GREEN: Color = Color::new(0.2, 1.0, 0.2);
pub const TV_BLUE: Color = Color::new(0.3, 0.3, 1.0);
pub const TV_YELLOW: Color = Color::new(1.0, 1.0, 0.2);
pub const TV_ORANGE: Color = Color::new(1.0, 0.55, 0.15);

// --- Chain colors (dark palette) --------------------------------------------
// Figma PATINAE re / UI Colors / chains/cA..cF, Dark (50:1).
// Keep the same hue order in Light (52:0); colors are rounded to 8-bit RGB.

pub const CHAIN_DARK_A: Color = Color::from_rgb8(0x22, 0xD3, 0xEE);
pub const CHAIN_DARK_B: Color = Color::from_rgb8(0xF4, 0x72, 0xB6);
pub const CHAIN_DARK_C: Color = Color::from_rgb8(0xA7, 0x8B, 0xFA);
pub const CHAIN_DARK_D: Color = Color::from_rgb8(0xF7, 0xE0, 0x6E);
pub const CHAIN_DARK_E: Color = Color::from_rgb8(0x34, 0xD3, 0x99);
pub const CHAIN_DARK_F: Color = Color::from_rgb8(0xFB, 0x92, 0x3C);

pub const CHAIN_DARK: [Color; 6] = [
    CHAIN_DARK_A,
    CHAIN_DARK_B,
    CHAIN_DARK_C,
    CHAIN_DARK_D,
    CHAIN_DARK_E,
    CHAIN_DARK_F,
];

// --- Chain colors (light palette) -------------------------------------------

pub const CHAIN_LIGHT_A: Color = Color::from_rgb8(0x0E, 0x74, 0x90);
pub const CHAIN_LIGHT_B: Color = Color::from_rgb8(0xBE, 0x18, 0x5D);
pub const CHAIN_LIGHT_C: Color = Color::from_rgb8(0x6D, 0x28, 0xD9);
pub const CHAIN_LIGHT_D: Color = Color::from_rgb8(0x91, 0x4D, 0x08);
pub const CHAIN_LIGHT_E: Color = Color::from_rgb8(0x04, 0x78, 0x57);
pub const CHAIN_LIGHT_F: Color = Color::from_rgb8(0x8E, 0x2C, 0x0B);

pub const CHAIN_LIGHT: [Color; 6] = [
    CHAIN_LIGHT_A,
    CHAIN_LIGHT_B,
    CHAIN_LIGHT_C,
    CHAIN_LIGHT_D,
    CHAIN_LIGHT_E,
    CHAIN_LIGHT_F,
];

// --- Viewport colors --------------------------------------------------------

// Match UI Foundations defaults, including the intentionally brighter Light viewport.
pub const VIEWPORT_BG_DARK: Color = Color::from_rgb8(0x0B, 0x0B, 0x0B);
pub const VIEWPORT_AO_DARK: Color = Color::from_rgb8(0x06, 0x06, 0x0A);
pub const VIEWPORT_BG_LIGHT: Color = Color::from_rgb8(0xFA, 0xF9, 0xF5);
pub const VIEWPORT_AO_LIGHT: Color = Color::from_rgb8(0xD8, 0xD4, 0xC4);
