//! Themed palette — one instance per visual theme.

use serde::{Deserialize, Serialize};

use crate::constants::*;
use crate::gradient::Gradient;
use crate::scheme::{ChainPalette, ElementPalette, ResiduePalette, SsPalette};
use crate::Color;

/// A complete color palette for one visual theme.
///
/// Each instance is a fully self-contained theme.
/// To switch themes, replace the palette with a different one.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemedPalette {
    pub element: ElementPalette,
    pub chains: ChainPalette,
    pub ss: SsPalette,
    pub residue: ResiduePalette,
    pub spectrum: Gradient,
    pub b_factor: Gradient,
    pub viewport_bg: Color,
    pub viewport_ao: Color,
}

impl ThemedPalette {
    /// Dark theme.
    pub fn dark() -> Self {
        ThemedPalette {
            element: ElementPalette::new(),
            chains: ChainPalette::new(CHAIN_DARK.to_vec()),
            ss: SsPalette::new(),
            residue: ResiduePalette::new(),
            spectrum: Gradient::rainbow(),
            b_factor: Gradient::blue_white_red(),
            viewport_bg: VIEWPORT_BG_DARK,
            viewport_ao: VIEWPORT_AO_DARK,
        }
    }

    /// Look up a theme by name. Falls back to dark for unknown names.
    pub fn by_name(name: &str) -> Self {
        match name.to_lowercase().as_str() {
            "light" => Self::light(),
            _ => Self::dark(),
        }
    }

    /// Light theme.
    pub fn light() -> Self {
        ThemedPalette {
            element: ElementPalette::new(),
            chains: ChainPalette::new(CHAIN_LIGHT.to_vec()),
            ss: SsPalette::new(),
            residue: ResiduePalette::new(),
            spectrum: Gradient::rainbow(),
            b_factor: Gradient::blue_white_red(),
            viewport_bg: VIEWPORT_BG_LIGHT,
            viewport_ao: VIEWPORT_AO_LIGHT,
        }
    }
}

impl Default for ThemedPalette {
    fn default() -> Self {
        Self::dark()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_lookup_preserves_names_case_and_unknown_fallback() {
        for (name, expected) in [
            ("dark", ThemedPalette::dark()),
            ("light", ThemedPalette::light()),
            ("Light", ThemedPalette::light()),
            ("solarized", ThemedPalette::dark()),
        ] {
            let actual = ThemedPalette::by_name(name);
            assert_eq!(actual.viewport_bg, expected.viewport_bg, "{name}");
            assert_eq!(actual.chains.colors(), expected.chains.colors(), "{name}");
        }
    }

    #[test]
    fn test_dark_light_differ() {
        let dark = ThemedPalette::dark();
        let light = ThemedPalette::light();
        assert_ne!(dark.chains.get("A"), light.chains.get("A"));
        assert_ne!(dark.viewport_bg, light.viewport_bg);
    }

    #[test]
    fn test_chain_wrapping() {
        let p = ThemedPalette::dark();
        // G wraps to index 6 % 6 = 0 → same as A
        assert_eq!(p.chains.get("G"), p.chains.get("A"));
    }

    #[test]
    fn test_default_chain_palettes_keep_theme_pair_order() {
        let dark = ThemedPalette::dark();
        let light = ThemedPalette::light();

        let expected = [
            ("A", CHAIN_DARK_A, CHAIN_LIGHT_A),
            ("B", CHAIN_DARK_B, CHAIN_LIGHT_B),
            ("C", CHAIN_DARK_C, CHAIN_LIGHT_C),
            ("D", CHAIN_DARK_D, CHAIN_LIGHT_D),
            ("E", CHAIN_DARK_E, CHAIN_LIGHT_E),
            ("F", CHAIN_DARK_F, CHAIN_LIGHT_F),
        ];

        for (chain, dark_color, light_color) in expected {
            assert_eq!(dark.chains.get(chain), dark_color);
            assert_eq!(light.chains.get(chain), light_color);
        }
    }

    #[test]
    fn default_chain_colors_are_unique_and_contrast_with_their_background() {
        fn luminance(color: Color) -> f32 {
            let [r, g, b] = color.to_array().map(|channel| {
                if channel <= 0.04045 {
                    channel / 12.92
                } else {
                    ((channel + 0.055) / 1.055).powf(2.4)
                }
            });
            0.2126 * r + 0.7152 * g + 0.0722 * b
        }

        for palette in [ThemedPalette::dark(), ThemedPalette::light()] {
            let colors = palette.chains.colors();
            let background = luminance(palette.viewport_bg);
            for (index, &color) in colors.iter().enumerate() {
                assert!(!colors[..index].contains(&color), "duplicate chain color");
                let foreground = luminance(color);
                let contrast =
                    (foreground.max(background) + 0.05) / (foreground.min(background) + 0.05);
                // Base swatches must remain visible before renderer lighting.
                assert!(contrast >= 3.0, "chain {index}: contrast {contrast}");
            }
        }
    }
}
